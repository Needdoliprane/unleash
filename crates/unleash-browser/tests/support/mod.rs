//! Harnais local de test d'intégration pour `unleash-browser` :
//! - Site marchand synthétique HTTP local (`127.0.0.1:0`) avec oracle indépendant.
//! - Serveurs canari modèle et tiers non autorisé avec compteurs atomiques indépendants.
//! - Serveur `WebDriver` W3C fidèle en Rust routant ses navigations et clics via `EgressProxy`.

#![allow(
    clippy::missing_errors_doc,
    clippy::too_many_lines,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing
)]

use std::collections::{BTreeMap, HashMap};
use std::convert::Infallible;
use std::io::Write;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use http::{Method, Request, Response, StatusCode, Uri};
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Empty, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use serde_json::{Value, json};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use unleash_browser::{BrowserEngineKind, PinnedBinary, PinnedBrowserRuntime, TargetPlatform};
use unleash_domain::Sha256Digest;
use url::Url;

type BoxedBody = BoxBody<Bytes, hyper::Error>;

fn full_body(s: impl Into<String>, content_type: &str) -> Response<BoxedBody> {
    let body = Full::new(Bytes::from(s.into()))
        .map_err(|never| match never {})
        .boxed();
    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", content_type)
        .body(body)
        .unwrap()
}

fn empty_proxy_body() -> BoxedBody {
    Empty::<Bytes>::new()
        .map_err(|never| match never {})
        .boxed()
}

/// Comportement injectable du site marchand synthétique local.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MerchantMode {
    Normal,
    AmbiguousDom,
    DriftedSkuDom,
    ReorderedValidDom,
    WrongQuantity,
    WrongPrice,
    WrongCurrency,
}

#[derive(Debug, Clone)]
pub(crate) struct CartEntry {
    pub(crate) sku: String,
    pub(crate) quantity: u64,
    pub(crate) unit_price_minor: u64,
    pub(crate) currency: String,
}

#[derive(Debug)]
struct MerchantState {
    mode: MerchantMode,
    cart: BTreeMap<String, CartEntry>,
    evil_origin: Option<String>,
    canary_origin: Option<String>,
}

/// Site marchand synthétique HTTP local servant du vrai HTML et maintenant un oracle serveur indépendant.
pub(crate) struct SyntheticMerchantServer {
    addr: SocketAddr,
    state: Arc<Mutex<MerchantState>>,
    mutation_count: Arc<AtomicU64>,
    shutdown_tx: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<()>>,
}

impl SyntheticMerchantServer {
    pub(crate) async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let state = Arc::new(Mutex::new(MerchantState {
            mode: MerchantMode::Normal,
            cart: BTreeMap::new(),
            evil_origin: None,
            canary_origin: None,
        }));
        let mutation_count = Arc::new(AtomicU64::new(0));
        let (shutdown_tx, mut shutdown_rx) = oneshot::channel::<()>();

        let st = Arc::clone(&state);
        let mc = Arc::clone(&mutation_count);
        let task = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut shutdown_rx => break,
                    accepted = listener.accept() => {
                        let Ok((stream, _)) = accepted else { break; };
                        let st_conn = Arc::clone(&st);
                        let mc_conn = Arc::clone(&mc);
                        tokio::spawn(async move {
                            let io = TokioIo::new(stream);
                            let svc = service_fn(move |req| {
                                std::future::ready(Ok::<_, Infallible>(handle_merchant_req(
                                    &st_conn, &mc_conn, &req,
                                )))
                            });
                            let _ = hyper::server::conn::http1::Builder::new()
                                .serve_connection(io, svc)
                                .await;
                        });
                    }
                }
            }
        });

        Self {
            addr,
            state,
            mutation_count,
            shutdown_tx: Some(shutdown_tx),
            task: Some(task),
        }
    }

    pub(crate) fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    pub(crate) fn set_mode(&self, mode: MerchantMode) {
        self.state.lock().unwrap().mode = mode;
    }

    pub(crate) fn reset_for_mode(&self, mode: MerchantMode) {
        let mut guard = self.state.lock().unwrap();
        guard.mode = mode;
        guard.cart.clear();
        self.mutation_count.store(0, Ordering::SeqCst);
    }

    pub(crate) fn set_redirect_targets(&self, evil_origin: &str, canary_origin: &str) {
        let mut guard = self.state.lock().unwrap();
        guard.evil_origin = Some(evil_origin.to_owned());
        guard.canary_origin = Some(canary_origin.to_owned());
    }

    pub(crate) fn mutation_count(&self) -> u64 {
        self.mutation_count.load(Ordering::SeqCst)
    }

    pub(crate) fn cart_snapshot(&self) -> BTreeMap<String, CartEntry> {
        self.state.lock().unwrap().cart.clone()
    }

    pub(crate) async fn shutdown(mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        if let Some(t) = self.task.take() {
            let _ = t.await;
        }
    }
}

impl Drop for SyntheticMerchantServer {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        if let Some(t) = self.task.take() {
            t.abort();
        }
    }
}

fn handle_merchant_req(
    state: &Arc<Mutex<MerchantState>>,
    mutation_count: &Arc<AtomicU64>,
    req: &Request<Incoming>,
) -> Response<BoxedBody> {
    let path = req.uri().path().to_owned();
    let query = req.uri().query().unwrap_or("").to_owned();
    let method = req.method().clone();

    if method == Method::GET && path == "/catalog" {
        let mode = state.lock().unwrap().mode;
        let html = match mode {
            MerchantMode::Normal
            | MerchantMode::WrongQuantity
            | MerchantMode::WrongPrice
            | MerchantMode::WrongCurrency => {
                r#"<!doctype html><html><head><title>Synthetic Shop - Catalog</title></head><body>
                <label for="qty-input">Quantity</label>
                <input id="qty-input" name="qty" data-testid="qty-input" value="2" />
                <button id="btn-add-apple" data-testid="add-apple" class="add-to-cart" data-sku="SYNTHETIC-APPLE-1KG" data-price-minor="350" data-currency="EUR" data-action="/cart/add?sku=SYNTHETIC-APPLE-1KG&qty=2" role="button" aria-label="Add Apple">Add Apple</button>
                <a data-testid="goto-cart" href="/cart">View Cart</a>
                <a data-testid="leak-evil" href="/redirect-evil">Promo</a>
                <a data-testid="leak-canary" href="/redirect-canary">Help</a>
                </body></html>"#
            }
            MerchantMode::AmbiguousDom => {
                r#"<!doctype html><html><head><title>Synthetic Shop - Catalog</title></head><body>
                <button id="btn-add-apple-1" data-testid="add-apple" class="add-to-cart" data-sku="SYNTHETIC-APPLE-1KG" data-price-minor="350" data-currency="EUR" data-action="/cart/add?sku=SYNTHETIC-APPLE-1KG&qty=2" role="button" aria-label="Add Apple">Add Apple</button>
                <button id="btn-add-apple-2" data-testid="add-apple" class="add-to-cart" data-sku="SYNTHETIC-APPLE-1KG" data-price-minor="350" data-currency="EUR" data-action="/cart/add?sku=SYNTHETIC-APPLE-1KG&qty=2" role="button" aria-label="Add Apple">Add Apple</button>
                </body></html>"#
            }
            MerchantMode::DriftedSkuDom => {
                r#"<!doctype html><html><head><title>Synthetic Shop - Catalog</title></head><body>
                <button id="btn-add-other" data-testid="add-apple" class="add-to-cart" data-sku="SYNTHETIC-OTHER-ITEM" data-price-minor="350" data-currency="EUR" data-action="/cart/add?sku=SYNTHETIC-OTHER-ITEM&qty=2" role="button" aria-label="Add Apple">Add Apple</button>
                </body></html>"#
            }
            MerchantMode::ReorderedValidDom => {
                r#"<!doctype html><html><head><title>Synthetic Shop - Catalog</title></head><body>
                <div class="banner">Reordered Promo</div>
                <button id="btn-add-rice" data-testid="add-rice" class="add-to-cart" data-sku="SYNTHETIC-RICE-1KG" data-price-minor="280" data-currency="EUR" data-action="/cart/add?sku=SYNTHETIC-RICE-1KG&qty=1" role="button" aria-label="Add Rice">Add Rice</button>
                <button id="btn-add-apple-moved" data-testid="add-apple" class="add-to-cart-primary" data-sku="SYNTHETIC-APPLE-1KG" data-price-minor="350" data-currency="EUR" data-action="/cart/add?sku=SYNTHETIC-APPLE-1KG&qty=2" role="button" aria-label="Add Apple">Add Apple</button>
                </body></html>"#
            }
        };
        return full_body(html, "text/html; charset=utf-8");
    }

    if method == Method::POST && path == "/cart/add" {
        let mut sku = "SYNTHETIC-APPLE-1KG".to_owned();
        let mut qty = 2_u64;
        for part in query.split('&') {
            if let Some((k, v)) = part.split_once('=') {
                if k == "sku" {
                    v.clone_into(&mut sku);
                } else if k == "qty"
                    && let Ok(parsed) = v.parse::<u64>()
                {
                    qty = parsed;
                }
            }
        }
        mutation_count.fetch_add(1, Ordering::SeqCst);
        let mut guard = state.lock().unwrap();
        let (effective_qty, effective_price, effective_currency) = match guard.mode {
            MerchantMode::WrongQuantity => (99, 350, "EUR".to_owned()),
            MerchantMode::WrongPrice => (qty, 999, "EUR".to_owned()),
            MerchantMode::WrongCurrency => (qty, 350, "USD".to_owned()),
            _ => (qty, 350, "EUR".to_owned()),
        };
        guard.cart.insert(
            sku.clone(),
            CartEntry {
                sku,
                quantity: effective_qty,
                unit_price_minor: effective_price,
                currency: effective_currency,
            },
        );
        return Response::builder()
            .status(StatusCode::SEE_OTHER)
            .header("location", "/cart")
            .body(empty_proxy_body())
            .unwrap();
    }

    if method == Method::GET && path == "/cart" {
        let guard = state.lock().unwrap();
        let entry = guard.cart.values().next().cloned().unwrap_or(CartEntry {
            sku: "EMPTY".into(),
            quantity: 0,
            unit_price_minor: 0,
            currency: "EUR".into(),
        });
        let total = entry.quantity * entry.unit_price_minor;
        let html = format!(
            r#"<!doctype html><html><head><title>Synthetic Cart</title></head><body>
            <span data-testid="cart-sku">{}</span>
            <span data-testid="cart-qty">{}</span>
            <span data-testid="cart-unit-price">{}</span>
            <span data-testid="cart-currency">{}</span>
            <span data-testid="cart-total-minor">{}</span>
            </body></html>"#,
            entry.sku, entry.quantity, entry.unit_price_minor, entry.currency, total
        );
        return full_body(html, "text/html; charset=utf-8");
    }

    if method == Method::GET && path == "/redirect-evil" {
        let target = state
            .lock()
            .unwrap()
            .evil_origin
            .clone()
            .unwrap_or_else(|| "http://127.0.0.1:9".into());
        return Response::builder()
            .status(StatusCode::FOUND)
            .header("location", format!("{target}/exfiltrate"))
            .body(empty_proxy_body())
            .unwrap();
    }

    if method == Method::GET && path == "/redirect-canary" {
        let target = state
            .lock()
            .unwrap()
            .canary_origin
            .clone()
            .unwrap_or_else(|| "http://127.0.0.1:9".into());
        return Response::builder()
            .status(StatusCode::FOUND)
            .header("location", format!("{target}/v1/chat/completions"))
            .body(empty_proxy_body())
            .unwrap();
    }

    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .body(empty_proxy_body())
        .unwrap()
}

/// Serveur HTTP minimal comptant chaque requête reçue (pour canari modèle ou origine interdite).
pub(crate) struct HitCounterServer {
    addr: SocketAddr,
    hits: Arc<AtomicU64>,
    shutdown_tx: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<()>>,
}

impl HitCounterServer {
    pub(crate) async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let hits = Arc::new(AtomicU64::new(0));
        let (shutdown_tx, mut shutdown_rx) = oneshot::channel::<()>();
        let hits_clone = Arc::clone(&hits);

        let task = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut shutdown_rx => break,
                    accepted = listener.accept() => {
                        let Ok((stream, _)) = accepted else { break; };
                        let h = Arc::clone(&hits_clone);
                        tokio::spawn(async move {
                            let io = TokioIo::new(stream);
                            let svc = service_fn(move |_req: Request<Incoming>| {
                                let h_inner = Arc::clone(&h);
                                async move {
                                    h_inner.fetch_add(1, Ordering::SeqCst);
                                    Ok::<_, Infallible>(full_body(r#"{"ok":true}"#, "application/json"))
                                }
                            });
                            let _ = hyper::server::conn::http1::Builder::new()
                                .serve_connection(io, svc)
                                .await;
                        });
                    }
                }
            }
        });

        Self {
            addr,
            hits,
            shutdown_tx: Some(shutdown_tx),
            task: Some(task),
        }
    }

    pub(crate) fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    pub(crate) fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub(crate) fn hits(&self) -> u64 {
        self.hits.load(Ordering::SeqCst)
    }

    pub(crate) fn reset_hits(&self) {
        self.hits.store(0, Ordering::SeqCst);
    }

    pub(crate) async fn shutdown(mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        if let Some(t) = self.task.take() {
            let _ = t.await;
        }
    }
}

impl Drop for HitCounterServer {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        if let Some(t) = self.task.take() {
            t.abort();
        }
    }
}

#[derive(Debug, Clone)]
struct DomNode {
    element_id: String,
    tag: String,
    attrs: BTreeMap<String, String>,
    text: String,
}

#[derive(Debug, Clone)]
struct WdSession {
    proxy_host_port: Option<String>,
    current_url: String,
    title: String,
    source: String,
    nodes: Vec<DomNode>,
    next_el_seq: u64,
}

#[derive(Debug)]
struct WdServerState {
    sessions: HashMap<String, WdSession>,
    next_session_seq: u64,
    drop_next_click_connection: Arc<AtomicBool>,
}

/// Serveur `WebDriver` W3C fidèle en Rust gérant sessions, DOM réel, rôles/labels calculés,
/// passage par le proxy HTTP déclaré et injection de coupure TCP après dispatch de clic.
pub(crate) struct FaithfulW3cWebDriverServer {
    addr: SocketAddr,
    drop_next_click_connection: Arc<AtomicBool>,
    shutdown_tx: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<()>>,
}

impl FaithfulW3cWebDriverServer {
    pub(crate) async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let drop_next_click_connection = Arc::new(AtomicBool::new(false));
        let state = Arc::new(tokio::sync::Mutex::new(WdServerState {
            sessions: HashMap::new(),
            next_session_seq: 1,
            drop_next_click_connection: Arc::clone(&drop_next_click_connection),
        }));
        let (shutdown_tx, mut shutdown_rx) = oneshot::channel::<()>();

        let st = Arc::clone(&state);
        let task = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut shutdown_rx => break,
                    accepted = listener.accept() => {
                        let Ok((stream, _)) = accepted else { break; };
                        let st_conn = Arc::clone(&st);
                        tokio::spawn(async move {
                            let io = TokioIo::new(stream);
                            let svc = service_fn(move |req| {
                                handle_wd_req(Arc::clone(&st_conn), req)
                            });
                            let _ = hyper::server::conn::http1::Builder::new()
                                .serve_connection(io, svc)
                                .await;
                        });
                    }
                }
            }
        });

        Self {
            addr,
            drop_next_click_connection,
            shutdown_tx: Some(shutdown_tx),
            task: Some(task),
        }
    }

    pub(crate) fn endpoint_url(&self) -> Url {
        Url::parse(&format!("http://{}", self.addr)).unwrap()
    }

    pub(crate) fn arm_drop_after_next_click(&self) {
        self.drop_next_click_connection
            .store(true, Ordering::SeqCst);
    }

    pub(crate) async fn shutdown(mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        if let Some(t) = self.task.take() {
            let _ = t.await;
        }
    }
}

impl Drop for FaithfulW3cWebDriverServer {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        if let Some(t) = self.task.take() {
            t.abort();
        }
    }
}

fn w3c_ok(val: &Value) -> Response<BoxedBody> {
    let payload = json!({ "value": val }).to_string();
    full_body(payload, "application/json; charset=utf-8")
}

fn w3c_err(status: StatusCode, error: &str, message: &str) -> Response<BoxedBody> {
    let payload = json!({
        "value": {
            "error": error,
            "message": message,
            "stacktrace": ""
        }
    })
    .to_string();
    let body = Full::new(Bytes::from(payload))
        .map_err(|never| match never {})
        .boxed();
    Response::builder()
        .status(status)
        .header("content-type", "application/json; charset=utf-8")
        .body(body)
        .unwrap()
}

async fn read_json_body(req: Request<Incoming>) -> Result<Value, std::io::Error> {
    let collected = req
        .into_body()
        .collect()
        .await
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let bytes = collected.to_bytes();
    if bytes.is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_slice(&bytes)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

async fn handle_wd_req(
    state: Arc<tokio::sync::Mutex<WdServerState>>,
    req: Request<Incoming>,
) -> Result<Response<BoxedBody>, std::io::Error> {
    let method = req.method().clone();
    let path = req.uri().path().trim_start_matches('/').to_owned();
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();

    if method == Method::GET && segments.as_slice() == ["status"] {
        return Ok(w3c_ok(
            &json!({ "ready": true, "message": "unleash wd fixture" }),
        ));
    }

    if method == Method::POST && segments.as_slice() == ["session"] {
        let body = read_json_body(req).await?;
        let proxy_host_port = body
            .pointer("/capabilities/alwaysMatch/proxy/httpProxy")
            .or_else(|| body.pointer("/desiredCapabilities/proxy/httpProxy"))
            .and_then(Value::as_str)
            .map(str::to_owned);

        let mut guard = state.lock().await;
        let sid = format!("wd-session-{}", guard.next_session_seq);
        guard.next_session_seq += 1;
        guard.sessions.insert(
            sid.clone(),
            WdSession {
                proxy_host_port,
                current_url: "about:blank".into(),
                title: String::new(),
                source: String::new(),
                nodes: Vec::new(),
                next_el_seq: 1,
            },
        );
        return Ok(w3c_ok(&json!({
            "sessionId": sid,
            "capabilities": {
                "browserName": "chrome",
                "platformName": "mac"
            }
        })));
    }

    if segments.first() != Some(&"session") || segments.len() < 2 {
        return Ok(w3c_err(StatusCode::NOT_FOUND, "unknown command", &path));
    }
    let sid = segments[1].to_owned();
    let sub = &segments[2..];

    if method == Method::DELETE && sub.is_empty() {
        state.lock().await.sessions.remove(&sid);
        return Ok(w3c_ok(&Value::Null));
    }

    if method == Method::POST && sub == ["timeouts"] {
        let _ = read_json_body(req).await?;
        return Ok(w3c_ok(&Value::Null));
    }

    if method == Method::GET && sub == ["url"] {
        let guard = state.lock().await;
        let Some(sess) = guard.sessions.get(&sid) else {
            return Ok(w3c_err(StatusCode::NOT_FOUND, "invalid session id", &sid));
        };
        return Ok(w3c_ok(&Value::String(sess.current_url.clone())));
    }

    if method == Method::POST && sub == ["url"] {
        let body = read_json_body(req).await?;
        let target_url = body
            .get("url")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let proxy_hp = {
            let guard = state.lock().await;
            let Some(sess) = guard.sessions.get(&sid) else {
                return Ok(w3c_err(StatusCode::NOT_FOUND, "invalid session id", &sid));
            };
            sess.proxy_host_port.clone()
        };

        let fetched = fetch_through_proxy(Method::GET, &target_url, proxy_hp.as_deref()).await;
        let mut guard = state.lock().await;
        let Some(sess) = guard.sessions.get_mut(&sid) else {
            return Ok(w3c_err(StatusCode::NOT_FOUND, "invalid session id", &sid));
        };
        apply_fetched_page(sess, &fetched);
        return Ok(w3c_ok(&Value::Null));
    }

    if method == Method::GET && sub == ["title"] {
        let guard = state.lock().await;
        let Some(sess) = guard.sessions.get(&sid) else {
            return Ok(w3c_err(StatusCode::NOT_FOUND, "invalid session id", &sid));
        };
        return Ok(w3c_ok(&Value::String(sess.title.clone())));
    }

    if method == Method::GET && sub == ["source"] {
        let guard = state.lock().await;
        let Some(sess) = guard.sessions.get(&sid) else {
            return Ok(w3c_err(StatusCode::NOT_FOUND, "invalid session id", &sid));
        };
        return Ok(w3c_ok(&Value::String(sess.source.clone())));
    }

    if method == Method::GET && sub == ["screenshot"] {
        return Ok(w3c_ok(&Value::String("UE5HX1RFU1RfQllURVM=".into())));
    }

    if method == Method::POST && (sub == ["element"] || sub == ["elements"]) {
        let body = read_json_body(req).await?;
        let using = body.get("using").and_then(Value::as_str).unwrap_or("");
        let selector = body.get("value").and_then(Value::as_str).unwrap_or("");
        if using != "css selector" {
            return Ok(w3c_err(
                StatusCode::BAD_REQUEST,
                "invalid argument",
                "only css selector supported",
            ));
        }
        let guard = state.lock().await;
        let Some(sess) = guard.sessions.get(&sid) else {
            return Ok(w3c_err(StatusCode::NOT_FOUND, "invalid session id", &sid));
        };
        let matched: Vec<Value> = sess
            .nodes
            .iter()
            .filter(|n| node_matches_css(n, selector))
            .map(|n| json!({ "element-6066-11e4-a52e-4f735466cecf": n.element_id }))
            .collect();
        if sub == ["element"] {
            if let Some(first) = matched.into_iter().next() {
                return Ok(w3c_ok(&first));
            }
            return Ok(w3c_err(StatusCode::NOT_FOUND, "no such element", selector));
        }
        return Ok(w3c_ok(&Value::Array(matched)));
    }

    if sub.len() >= 3 && sub[0] == "element" {
        let eid = sub[1].to_owned();
        let action_sub = &sub[2..];

        if method == Method::GET && action_sub == ["text"] {
            let guard = state.lock().await;
            let Some(node) = find_node(&guard, &sid, &eid) else {
                return Ok(w3c_err(StatusCode::NOT_FOUND, "no such element", &eid));
            };
            return Ok(w3c_ok(&Value::String(node.text.clone())));
        }

        if method == Method::GET && action_sub.len() == 2 && action_sub[0] == "attribute" {
            let attr = action_sub[1];
            let guard = state.lock().await;
            let Some(node) = find_node(&guard, &sid, &eid) else {
                return Ok(w3c_err(StatusCode::NOT_FOUND, "no such element", &eid));
            };
            let val = node
                .attrs
                .get(attr)
                .map_or(Value::Null, |v| Value::String(v.clone()));
            return Ok(w3c_ok(&val));
        }

        if method == Method::GET && action_sub.len() == 2 && action_sub[0] == "property" {
            let prop = action_sub[1];
            let guard = state.lock().await;
            let Some(node) = find_node(&guard, &sid, &eid) else {
                return Ok(w3c_err(StatusCode::NOT_FOUND, "no such element", &eid));
            };
            let val = node
                .attrs
                .get(prop)
                .map_or(Value::Null, |v| Value::String(v.clone()));
            return Ok(w3c_ok(&val));
        }

        if method == Method::GET && action_sub == ["computedrole"] {
            let guard = state.lock().await;
            let Some(node) = find_node(&guard, &sid, &eid) else {
                return Ok(w3c_err(StatusCode::NOT_FOUND, "no such element", &eid));
            };
            return Ok(w3c_ok(&Value::String(compute_role(node))));
        }

        if method == Method::GET && action_sub == ["computedlabel"] {
            let guard = state.lock().await;
            let Some(sess) = guard.sessions.get(&sid) else {
                return Ok(w3c_err(StatusCode::NOT_FOUND, "invalid session id", &sid));
            };
            let Some(node) = sess.nodes.iter().find(|n| n.element_id == eid) else {
                return Ok(w3c_err(StatusCode::NOT_FOUND, "no such element", &eid));
            };
            return Ok(w3c_ok(&Value::String(compute_label(node, &sess.nodes))));
        }

        if method == Method::POST && action_sub == ["clear"] {
            let mut guard = state.lock().await;
            if let Some(sess) = guard.sessions.get_mut(&sid)
                && let Some(node) = sess.nodes.iter_mut().find(|n| n.element_id == eid)
            {
                node.attrs.insert("value".into(), String::new());
                return Ok(w3c_ok(&Value::Null));
            }
            return Ok(w3c_err(StatusCode::NOT_FOUND, "no such element", &eid));
        }

        if method == Method::POST && action_sub == ["value"] {
            let body = read_json_body(req).await?;
            let text = body
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            let mut guard = state.lock().await;
            if let Some(sess) = guard.sessions.get_mut(&sid)
                && let Some(node) = sess.nodes.iter_mut().find(|n| n.element_id == eid)
            {
                node.attrs.insert("value".into(), text);
                return Ok(w3c_ok(&Value::Null));
            }
            return Ok(w3c_err(StatusCode::NOT_FOUND, "no such element", &eid));
        }

        if method == Method::POST && action_sub == ["click"] {
            let _ = read_json_body(req).await?;
            let (action_url, action_method, proxy_hp, drop_flag) = {
                let guard = state.lock().await;
                let Some(sess) = guard.sessions.get(&sid) else {
                    return Ok(w3c_err(StatusCode::NOT_FOUND, "invalid session id", &sid));
                };
                let Some(node) = sess.nodes.iter().find(|n| n.element_id == eid) else {
                    return Ok(w3c_err(StatusCode::NOT_FOUND, "no such element", &eid));
                };
                let base = Url::parse(&sess.current_url).ok();
                let target_rel = node
                    .attrs
                    .get("data-action")
                    .or_else(|| node.attrs.get("href"))
                    .cloned();
                let resolved = match (base, target_rel) {
                    (Some(b), Some(rel)) => b.join(&rel).map(|u| u.to_string()).ok(),
                    _ => None,
                };
                let m = if node.attrs.contains_key("data-action") {
                    Method::POST
                } else {
                    Method::GET
                };
                (
                    resolved,
                    m,
                    sess.proxy_host_port.clone(),
                    Arc::clone(&guard.drop_next_click_connection),
                )
            };

            if let Some(target_url) = action_url {
                let fetched =
                    fetch_through_proxy(action_method, &target_url, proxy_hp.as_deref()).await;
                if drop_flag.swap(false, Ordering::SeqCst) {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::ConnectionReset,
                        "simulated post-dispatch connection drop",
                    ));
                }
                let mut guard = state.lock().await;
                if let Some(sess) = guard.sessions.get_mut(&sid) {
                    apply_fetched_page(sess, &fetched);
                }
            }
            return Ok(w3c_ok(&Value::Null));
        }
    }

    Ok(w3c_err(StatusCode::NOT_FOUND, "unknown command", &path))
}

fn find_node<'a>(guard: &'a WdServerState, sid: &str, eid: &str) -> Option<&'a DomNode> {
    guard
        .sessions
        .get(sid)
        .and_then(|s| s.nodes.iter().find(|n| n.element_id == eid))
}

struct FetchedPage {
    final_url: String,
    status: StatusCode,
    body: String,
}

async fn fetch_through_proxy(
    initial_method: Method,
    initial_url: &str,
    proxy_host_port: Option<&str>,
) -> FetchedPage {
    let mut current_url = initial_url.to_owned();
    let mut method = initial_method;

    for _hop in 0..5 {
        let resp = send_one_http_request(&method, &current_url, proxy_host_port).await;
        match resp {
            Ok((status, location, body)) => {
                if status.is_redirection()
                    && let Some(loc) = location
                    && let Ok(base) = Url::parse(&current_url)
                    && let Ok(next_url) = base.join(&loc)
                {
                    current_url = next_url.to_string();
                    method = Method::GET;
                    continue;
                }
                return FetchedPage {
                    final_url: current_url,
                    status,
                    body,
                };
            }
            Err(err_msg) => {
                return FetchedPage {
                    final_url: current_url,
                    status: StatusCode::BAD_GATEWAY,
                    body: format!(
                        "<html><title>Transport Error</title><body>{err_msg}</body></html>"
                    ),
                };
            }
        }
    }

    FetchedPage {
        final_url: current_url,
        status: StatusCode::LOOP_DETECTED,
        body: "<html><title>Too Many Redirects</title></html>".into(),
    }
}

async fn send_one_http_request(
    method: &Method,
    target_url: &str,
    proxy_host_port: Option<&str>,
) -> Result<(StatusCode, Option<String>, String), String> {
    let parsed = Url::parse(target_url).map_err(|e| e.to_string())?;
    let host_port = format!(
        "{}:{}",
        parsed.host_str().unwrap_or("127.0.0.1"),
        parsed.port_or_known_default().unwrap_or(80)
    );
    let connect_addr = proxy_host_port.unwrap_or(&host_port);
    let stream = TcpStream::connect(connect_addr)
        .await
        .map_err(|e| e.to_string())?;
    let io = TokioIo::new(stream);
    let (mut sender, conn) = hyper::client::conn::http1::handshake(io)
        .await
        .map_err(|e| e.to_string())?;
    tokio::spawn(async move {
        let _ = conn.await;
    });

    let uri_str = if proxy_host_port.is_some() {
        target_url.to_owned()
    } else {
        let mut path = parsed.path().to_owned();
        if let Some(q) = parsed.query() {
            path.push('?');
            path.push_str(q);
        }
        path
    };
    let uri: Uri = uri_str
        .parse()
        .map_err(|e: http::uri::InvalidUri| e.to_string())?;
    let req = Request::builder()
        .method(method.clone())
        .uri(uri)
        .header(http::header::HOST, &host_port)
        .body(empty_proxy_body())
        .map_err(|e| e.to_string())?;

    let resp = sender.send_request(req).await.map_err(|e| e.to_string())?;
    let status = resp.status();
    let location = resp
        .headers()
        .get(http::header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let bytes = resp
        .into_body()
        .collect()
        .await
        .map_err(|e| e.to_string())?
        .to_bytes();
    let body = String::from_utf8_lossy(&bytes).into_owned();
    Ok((status, location, body))
}

fn apply_fetched_page(sess: &mut WdSession, fetched: &FetchedPage) {
    if fetched.status == StatusCode::FORBIDDEN {
        sess.title = "403 Blocked by EgressProxy".into();
        sess.source.clone_from(&fetched.body);
        sess.nodes.clear();
        return;
    }
    sess.current_url.clone_from(&fetched.final_url);
    sess.source.clone_from(&fetched.body);
    sess.title = extract_tag_text(&fetched.body, "title").unwrap_or_default();
    sess.nodes = parse_html_nodes(&fetched.body, &mut sess.next_el_seq);
}

fn extract_tag_text(html: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = html.find(&open)? + open.len();
    let end = html[start..].find(&close)? + start;
    Some(html[start..end].trim().to_owned())
}

fn parse_html_nodes(html: &str, next_seq: &mut u64) -> Vec<DomNode> {
    let mut nodes = Vec::new();
    let mut pos = 0;
    let bytes = html.as_bytes();

    while pos < bytes.len() {
        let Some(lt_rel) = html[pos..].find('<') else {
            break;
        };
        let tag_start = pos + lt_rel + 1;
        let Some(gt_rel) = html[tag_start..].find('>') else {
            break;
        };
        let tag_end = tag_start + gt_rel;
        let inside = html[tag_start..tag_end].trim();

        if inside.starts_with('/') || inside.starts_with('!') {
            pos = tag_end + 1;
            continue;
        }

        let self_closing = inside.ends_with('/');
        let clean = inside.trim_end_matches('/').trim();
        let (tag_name, attr_raw) = match clean.split_once(char::is_whitespace) {
            Some((t, rest)) => (t.to_ascii_lowercase(), rest),
            None => (clean.to_ascii_lowercase(), ""),
        };

        let attrs = parse_attrs(attr_raw);
        let mut text = String::new();
        let after_open = tag_end + 1;
        if !self_closing
            && matches!(
                tag_name.as_str(),
                "button" | "a" | "span" | "label" | "option" | "div" | "h1"
            )
        {
            let close_pat = format!("</{tag_name}>");
            if let Some(close_rel) = html[after_open..].find(&close_pat) {
                let inner = &html[after_open..after_open + close_rel];
                if !inner.contains('<') {
                    inner.trim().clone_into(&mut text);
                }
            }
        }

        if matches!(
            tag_name.as_str(),
            "button"
                | "a"
                | "input"
                | "select"
                | "textarea"
                | "span"
                | "label"
                | "div"
                | "option"
                | "h1"
        ) {
            let eid = format!("el-{}", *next_seq);
            *next_seq += 1;
            nodes.push(DomNode {
                element_id: eid,
                tag: tag_name,
                attrs,
                text,
            });
        }
        pos = after_open;
    }

    nodes
}

fn parse_attrs(raw: &str) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    let mut rest = raw.trim();
    while !rest.is_empty() {
        let Some(eq_idx) = rest.find('=') else {
            break;
        };
        let name = rest[..eq_idx].trim().to_ascii_lowercase();
        rest = rest[eq_idx + 1..].trim_start();
        if let Some(after_q) = rest.strip_prefix('"') {
            if let Some(end_q) = after_q.find('"') {
                map.insert(name, after_q[..end_q].to_owned());
                rest = after_q[end_q + 1..].trim_start();
            } else {
                break;
            }
        } else if let Some(after_q) = rest.strip_prefix('\'') {
            if let Some(end_q) = after_q.find('\'') {
                map.insert(name, after_q[..end_q].to_owned());
                rest = after_q[end_q + 1..].trim_start();
            } else {
                break;
            }
        } else {
            let end_tok = rest.find(char::is_whitespace).unwrap_or(rest.len());
            map.insert(name, rest[..end_tok].to_owned());
            rest = rest[end_tok..].trim_start();
        }
    }
    map
}

fn node_matches_css(node: &DomNode, selector_list: &str) -> bool {
    selector_list
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .any(|sel| node_matches_single_css(node, sel))
}

fn node_matches_single_css(node: &DomNode, sel: &str) -> bool {
    let mut s = sel.trim();
    if let Some(tag_end) = s.find(['.', '#', '[']) {
        let tag = &s[..tag_end];
        if !tag.is_empty() && !node.tag.eq_ignore_ascii_case(tag) {
            return false;
        }
        s = &s[tag_end..];
    } else {
        return node.tag.eq_ignore_ascii_case(s);
    }

    while !s.is_empty() {
        if let Some(rest) = s.strip_prefix('#') {
            let end = rest.find(['.', '#', '[']).unwrap_or(rest.len());
            let id = &rest[..end];
            if node.attrs.get("id").map(String::as_str) != Some(id) {
                return false;
            }
            s = &rest[end..];
        } else if let Some(rest) = s.strip_prefix('.') {
            let end = rest.find(['.', '#', '[']).unwrap_or(rest.len());
            let cls = &rest[..end];
            let has_cls = node
                .attrs
                .get("class")
                .is_some_and(|c| c.split_whitespace().any(|w| w == cls));
            if !has_cls {
                return false;
            }
            s = &rest[end..];
        } else if let Some(rest) = s.strip_prefix('[') {
            let Some(close) = rest.find(']') else {
                return false;
            };
            let inside = &rest[..close];
            if let Some((attr, raw_val)) = inside.split_once('=') {
                let val = raw_val
                    .trim()
                    .trim_matches('"')
                    .trim_matches('\'')
                    .replace("\\\"", "\"")
                    .replace("\\\\", "\\");
                if node.attrs.get(attr.trim()).map(String::as_str) != Some(val.as_str()) {
                    return false;
                }
            } else if !node.attrs.contains_key(inside.trim()) {
                return false;
            }
            s = &rest[close + 1..];
        } else {
            return false;
        }
    }
    true
}

fn compute_role(node: &DomNode) -> String {
    if let Some(r) = node.attrs.get("role") {
        return r.clone();
    }
    match node.tag.as_str() {
        "button" => "button".into(),
        "a" if node.attrs.contains_key("href") => "link".into(),
        "input" => "textbox".into(),
        "select" => "combobox".into(),
        "h1" => "heading".into(),
        _ => "generic".into(),
    }
}

fn compute_label(node: &DomNode, all_nodes: &[DomNode]) -> String {
    if let Some(aria) = node.attrs.get("aria-label") {
        return aria.clone();
    }
    if let Some(id) = node.attrs.get("id") {
        for other in all_nodes {
            if other.tag == "label"
                && other.attrs.get("for").map(String::as_str) == Some(id.as_str())
            {
                return other.text.clone();
            }
        }
    }
    node.text.clone()
}

/// Crée un couple factice `chrome` + `chromedriver` dans `dir` et retourne un [`PinnedBrowserRuntime`].
pub(crate) fn create_pinned_dummy_runtime(dir: &Path, version: &str) -> PinnedBrowserRuntime {
    let chrome_path: PathBuf = dir.join("chrome-for-testing");
    let driver_path: PathBuf = dir.join("chromedriver");
    let mut cf = std::fs::File::create(&chrome_path).unwrap();
    cf.write_all(b"dummy-chrome-for-testing-binary").unwrap();
    let mut df = std::fs::File::create(&driver_path).unwrap();
    df.write_all(b"dummy-chromedriver-binary").unwrap();

    PinnedBrowserRuntime {
        engine: BrowserEngineKind::ChromeForTesting,
        platform: TargetPlatform::MacOsArm64,
        browser: PinnedBinary {
            path: chrome_path,
            expected_version: version.to_owned(),
            expected_sha256: Sha256Digest::from_bytes(b"dummy-chrome-for-testing-binary"),
        },
        driver: PinnedBinary {
            path: driver_path,
            expected_version: version.to_owned(),
            expected_sha256: Sha256Digest::from_bytes(b"dummy-chromedriver-binary"),
        },
        mapping_version: "unleash.browser.v1".into(),
    }
}
