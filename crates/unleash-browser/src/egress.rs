//! Proxy d'egress HTTP local lié au mandat d'exécution (`EgressProxy`).
//!
//! Filtre chaque requête HTTP et `CONNECT` selon la liste d'origines autorisées
//! par le manifeste ([`EgressPolicy`]), bloque les redirections `3xx` sortant du
//! périmètre autorisé et bloque toute destination identifiée comme canari modèle
//! sous `ModelPolicy::Forbidden`.

use std::collections::BTreeSet;
use std::convert::Infallible;
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use http::{HeaderValue, Method, Request, Response, StatusCode, Uri};
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Empty, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::client::legacy::Client as HyperClient;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::{TokioExecutor, TokioIo};
use serde::{Deserialize, Serialize};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, watch};
use tokio::task::{JoinHandle, JoinSet};
use unleash_domain::{LeaseId, ModelPolicy, RunId};
use url::Url;

use crate::error::BrowserError;

type ProxyBody = BoxBody<Bytes, hyper::Error>;
type TunnelFuture = Pin<Box<dyn Future<Output = ()> + Send>>;

const MAX_CONCURRENT_PROXY_TASKS: usize = 128;
const TUNNEL_QUEUE_CAPACITY: usize = 64;
const PROXY_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const PROXY_IO_TIMEOUT: Duration = Duration::from_secs(300);

/// Politique d'egress autorisée par le manifeste du run navigateur.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct EgressPolicy {
    /// Ensemble des origines normalisées (`scheme://host:port`) autorisées par le mandat.
    pub allowed_origins: BTreeSet<String>,
    /// Ensemble optionnel d'origines canaris de modèles à refuser explicitement sous `Forbidden`.
    pub model_canary_origins: BTreeSet<String>,
    /// Bloque toute redirection HTTP `3xx` pointant hors de `allowed_origins`.
    pub block_redirects_outside_mandate: bool,
}

impl EgressPolicy {
    /// Construit une politique autorisant uniquement les origines fournies (avec filtrage des redirections activé).
    ///
    /// # Errors
    /// - [`BrowserError::InvalidConfiguration`] si une URL fournie n'est pas une origine HTTP/HTTPS valide.
    pub fn from_urls(allowed: &[&str]) -> Result<Self, BrowserError> {
        let mut allowed_origins = BTreeSet::new();
        for raw in allowed {
            let parsed = Url::parse(raw).map_err(|err| BrowserError::InvalidConfiguration {
                reason: format!("URL d'origine autorisée invalide '{raw}': {err}"),
            })?;
            allowed_origins.insert(normalize_origin(&parsed)?);
        }
        Ok(Self {
            allowed_origins,
            model_canary_origins: BTreeSet::new(),
            block_redirects_outside_mandate: true,
        })
    }

    /// Ajoute une origine canari de modèle à surveiller et bloquer sous `ModelPolicy::Forbidden`.
    ///
    /// # Errors
    /// - [`BrowserError::InvalidConfiguration`] si l'URL est invalide.
    pub fn add_model_canary_url(&mut self, raw: &str) -> Result<(), BrowserError> {
        let parsed = Url::parse(raw).map_err(|err| BrowserError::InvalidConfiguration {
            reason: format!("URL canari invalide '{raw}': {err}"),
        })?;
        self.model_canary_origins.insert(normalize_origin(&parsed)?);
        Ok(())
    }

    /// Vérifie si une origine normalisée est admise par ce mandat et cette politique modèle.
    ///
    /// # Errors
    /// - [`BrowserError::EgressBlocked`] si l'origine vise un canari modèle sous `Forbidden` ou
    ///   est absente de `allowed_origins`.
    pub fn check_origin(
        &self,
        origin: &str,
        model_policy: ModelPolicy,
    ) -> Result<(), BrowserError> {
        if model_policy == ModelPolicy::Forbidden && self.model_canary_origins.contains(origin) {
            return Err(BrowserError::EgressBlocked {
                origin: origin.to_owned(),
                reason: "model_policy_forbidden".into(),
            });
        }
        if !self.allowed_origins.contains(origin) {
            return Err(BrowserError::EgressBlocked {
                origin: origin.to_owned(),
                reason: "origin_not_in_mandate".into(),
            });
        }
        Ok(())
    }
}

/// Événement d'audit enregistré par le proxy d'egress pour chaque requête ou redirection filtrée.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EgressAuditEvent {
    /// Numéro séquentiel de l'événement au sein du proxy.
    pub sequence: u64,
    /// Méthode HTTP observée (`GET`, `POST`, `CONNECT`, etc.).
    pub method: String,
    /// Cible demandée.
    pub target_url: String,
    /// Origine normalisée évaluée.
    pub resolved_origin: String,
    /// `true` si autorisée, `false` si bloquée.
    pub allowed: bool,
    /// Motif de décision (`allowed`, `origin_not_in_mandate`, `redirect_origin_denied`, etc.).
    pub reason: String,
    /// Origine de destination en cas de redirection `3xx` inspectée.
    pub redirect_target_origin: Option<String>,
}

/// Normalise une URL vers sa forme canonique d'origine `scheme://host:port`.
///
/// Refuse tout schéma autre que `http`/`https` (`file:`, `javascript:`, `data:`) ainsi que
/// les identifiants intégrés dans l'URL (`user:pass@`).
///
/// # Errors
/// - [`BrowserError::EgressBlocked`] si le schéma est refusé ou que des identifiants sont présents.
/// - [`BrowserError::InvalidConfiguration`] si l'hôte est absent.
pub fn normalize_origin(url: &Url) -> Result<String, BrowserError> {
    let scheme = url.scheme().to_ascii_lowercase();
    if scheme != "http" && scheme != "https" {
        return Err(BrowserError::EgressBlocked {
            origin: url.as_str().to_owned(),
            reason: format!("unsupported_scheme:{scheme}"),
        });
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(BrowserError::EgressBlocked {
            origin: url.as_str().to_owned(),
            reason: "userinfo_in_url_forbidden".into(),
        });
    }
    let host = url
        .host_str()
        .ok_or_else(|| BrowserError::InvalidConfiguration {
            reason: format!("URL sans hôte : '{url}'"),
        })?
        .to_ascii_lowercase();
    let port = url
        .port_or_known_default()
        .ok_or_else(|| BrowserError::InvalidConfiguration {
            reason: format!("URL sans port résoluble : '{url}'"),
        })?;
    Ok(format!("{scheme}://{host}:{port}"))
}

#[derive(Debug)]
struct ProxyState {
    run_id: RunId,
    lease_id: LeaseId,
    model_policy: ModelPolicy,
    policy: EgressPolicy,
    revoked: AtomicBool,
    next_seq: AtomicU64,
    blocked_count: AtomicU64,
    events: Mutex<Vec<EgressAuditEvent>>,
    client: HyperClient<HttpConnector, ProxyBody>,
}

impl ProxyState {
    fn record_event(
        &self,
        method: &Method,
        target_url: &str,
        resolved_origin: &str,
        allowed: bool,
        reason: &str,
        redirect_target_origin: Option<String>,
    ) {
        if !allowed {
            self.blocked_count.fetch_add(1, Ordering::SeqCst);
            tracing::warn!(
                run_id = %self.run_id,
                lease_id = %self.lease_id,
                %method,
                target_url,
                resolved_origin,
                reason,
                ?redirect_target_origin,
                "EgressProxy a bloqué une sortie ou redirection hors mandat"
            );
        }
        let seq = self.next_seq.fetch_add(1, Ordering::SeqCst) + 1;
        if let Ok(mut guard) = self.events.lock() {
            guard.push(EgressAuditEvent {
                sequence: seq,
                method: method.as_str().to_owned(),
                target_url: target_url.to_owned(),
                resolved_origin: resolved_origin.to_owned(),
                allowed,
                reason: reason.to_owned(),
                redirect_target_origin,
            });
        }
    }
}

/// Serveur proxy HTTP/1.1 loopback lié à un mandat d'exécution navigateur.
#[derive(Debug)]
pub struct EgressProxy {
    addr: SocketAddr,
    state: Arc<ProxyState>,
    cancel_tx: watch::Sender<bool>,
    task: Option<JoinHandle<()>>,
}

impl EgressProxy {
    /// Démarre un [`EgressProxy`] sur `127.0.0.1:0` pour le mandat indiqué.
    ///
    /// # Errors
    /// - [`BrowserError::InvalidConfiguration`] si le bind loopback échoue.
    pub async fn start(
        run_id: RunId,
        lease_id: LeaseId,
        model_policy: ModelPolicy,
        policy: EgressPolicy,
    ) -> Result<Self, BrowserError> {
        let listener = TcpListener::bind("127.0.0.1:0").await.map_err(|err| {
            BrowserError::InvalidConfiguration {
                reason: format!("Impossible de binder EgressProxy sur 127.0.0.1:0 : {err}"),
            }
        })?;
        let addr = listener
            .local_addr()
            .map_err(|err| BrowserError::InvalidConfiguration {
                reason: format!("Impossible de lire l'adresse locale d'EgressProxy : {err}"),
            })?;

        let client = HyperClient::builder(TokioExecutor::new()).build_http::<ProxyBody>();
        let state = Arc::new(ProxyState {
            run_id,
            lease_id,
            model_policy,
            policy,
            revoked: AtomicBool::new(false),
            next_seq: AtomicU64::new(0),
            blocked_count: AtomicU64::new(0),
            events: Mutex::new(Vec::new()),
            client,
        });

        let (cancel_tx, mut accept_cancel_rx) = watch::channel(false);
        let loop_cancel_rx = cancel_tx.subscribe();
        let accept_state = Arc::clone(&state);

        let task = tokio::spawn(async move {
            let (tunnel_tx, mut tunnel_rx) = mpsc::channel::<TunnelFuture>(TUNNEL_QUEUE_CAPACITY);
            let mut child_tasks: JoinSet<()> = JoinSet::new();

            loop {
                tokio::select! {
                    () = wait_for_cancel(&mut accept_cancel_rx) => {
                        break;
                    }
                    Some(_) = child_tasks.join_next(), if !child_tasks.is_empty() => {}
                    Some(tunnel_fut) = tunnel_rx.recv(), if child_tasks.len() < MAX_CONCURRENT_PROXY_TASKS => {
                        child_tasks.spawn(tunnel_fut);
                    }
                    accepted = listener.accept(), if child_tasks.len() < MAX_CONCURRENT_PROXY_TASKS / 2 => {
                        match accepted {
                            Ok((stream, _peer)) => {
                                let conn_state = Arc::clone(&accept_state);
                                let conn_tunnel_tx = tunnel_tx.clone();
                                let mut conn_cancel = loop_cancel_rx.clone();
                                let svc_cancel = loop_cancel_rx.clone();
                                child_tasks.spawn(async move {
                                    let io = TokioIo::new(stream);
                                    let svc = service_fn(move |req| {
                                        handle_proxy_request(
                                            Arc::clone(&conn_state),
                                            conn_tunnel_tx.clone(),
                                            svc_cancel.clone(),
                                            req,
                                        )
                                    });
                                    let conn_fut = hyper::server::conn::http1::Builder::new()
                                        .preserve_header_case(true)
                                        .title_case_headers(true)
                                        .serve_connection(io, svc)
                                        .with_upgrades();
                                    tokio::select! {
                                        () = wait_for_cancel(&mut conn_cancel) => {}
                                        _ = tokio::time::timeout(PROXY_IO_TIMEOUT, conn_fut) => {}
                                    }
                                });
                            }
                            Err(_) => break,
                        }
                    }
                }
            }

            tunnel_rx.close();
            drop(tunnel_tx);
            child_tasks.shutdown().await;
        });

        Ok(Self {
            addr,
            state,
            cancel_tx,
            task: Some(task),
        })
    }

    /// Adresse socket loopback sur laquelle le proxy écoute.
    #[must_use]
    pub const fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// URL HTTP du proxy à passer aux capacités `WebDriver` (`http://127.0.0.1:<port>`).
    #[must_use]
    pub fn proxy_url_string(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// Drapeaux de ligne de commande `Chrome for Testing` forçant le trafic (y compris loopback)
    /// via le proxy d'egress et désactivant QUIC / UDP WebRTC non proxifié.
    #[must_use]
    pub fn chrome_proxy_flags(&self) -> Vec<String> {
        vec![
            format!("--proxy-server={}", self.proxy_url_string()),
            "--proxy-bypass-list=<-loopback>".into(),
            "--disable-quic".into(),
            "--webrtc-ip-handling-policy=disable_non_proxied_udp".into(),
            "--force-webrtc-ip-handling-policy".into(),
        ]
    }

    /// Révoque immédiatement le mandat : toute requête suivante est bloquée (`403 Forbidden`).
    pub fn revoke(&self) {
        self.state.revoked.store(true, Ordering::SeqCst);
    }

    /// Nombre total de requêtes ou redirections bloquées depuis le démarrage.
    #[must_use]
    pub fn blocked_count(&self) -> u64 {
        self.state.blocked_count.load(Ordering::SeqCst)
    }

    /// Retourne une copie des événements d'audit capturés.
    #[must_use]
    pub fn audit_events(&self) -> Vec<EgressAuditEvent> {
        self.state
            .events
            .lock()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }

    /// Arrête proprement le serveur proxy ainsi que toutes les connexions et tunnels actifs.
    pub async fn shutdown(mut self) {
        self.revoke();
        let _ = self.cancel_tx.send(true);
        if let Some(handle) = self.task.take() {
            let abort_handle = handle.abort_handle();
            if tokio::time::timeout(PROXY_CONNECT_TIMEOUT, handle)
                .await
                .is_err()
            {
                abort_handle.abort();
            }
        }
    }
}

impl Drop for EgressProxy {
    fn drop(&mut self) {
        self.state.revoked.store(true, Ordering::SeqCst);
        let _ = self.cancel_tx.send(true);
        if let Some(handle) = self.task.take() {
            handle.abort();
        }
    }
}

async fn wait_for_cancel(cancel_rx: &mut watch::Receiver<bool>) {
    if *cancel_rx.borrow() {
        return;
    }
    let _ = cancel_rx.wait_for(|cancelled| *cancelled).await;
}

async fn handle_proxy_request(
    state: Arc<ProxyState>,
    tunnel_tx: mpsc::Sender<TunnelFuture>,
    cancel_rx: watch::Receiver<bool>,
    req: Request<Incoming>,
) -> Result<Response<ProxyBody>, Infallible> {
    let method = req.method().clone();
    let raw_target = req.uri().to_string();

    if state.revoked.load(Ordering::SeqCst) {
        state.record_event(&method, &raw_target, "", false, "mandate_revoked", None);
        return Ok(forbidden_response(
            "mandate_revoked",
            "Mandat d'egress révoqué",
        ));
    }

    if method == Method::CONNECT {
        return Ok(handle_connect_request(&state, &tunnel_tx, &cancel_rx, req).await);
    }

    let Ok(target_url) = extract_http_target_url(&req) else {
        state.record_event(&method, &raw_target, "", false, "invalid_target_url", None);
        return Ok(forbidden_response(
            "invalid_target_url",
            "URL cible invalide",
        ));
    };

    let origin = match normalize_origin(&target_url) {
        Ok(o) => o,
        Err(BrowserError::EgressBlocked { reason, .. }) => {
            state.record_event(&method, target_url.as_str(), "", false, &reason, None);
            return Ok(forbidden_response(
                &reason,
                "Origine refusée par EgressProxy",
            ));
        }
        Err(_) => {
            state.record_event(
                &method,
                target_url.as_str(),
                "",
                false,
                "invalid_origin",
                None,
            );
            return Ok(forbidden_response("invalid_origin", "Origine invalide"));
        }
    };

    if let Err(BrowserError::EgressBlocked { reason, .. }) =
        state.policy.check_origin(&origin, state.model_policy)
    {
        state.record_event(&method, target_url.as_str(), &origin, false, &reason, None);
        return Ok(forbidden_response(&reason, "Origine hors mandat"));
    }

    forward_http_with_redirect_filter(state, req, target_url, origin).await
}

async fn handle_connect_request(
    state: &Arc<ProxyState>,
    tunnel_tx: &mpsc::Sender<TunnelFuture>,
    cancel_rx: &watch::Receiver<bool>,
    req: Request<Incoming>,
) -> Response<ProxyBody> {
    let method = req.method().clone();
    let authority_str = req
        .uri()
        .authority()
        .map_or("", http::uri::Authority::as_str)
        .to_owned();

    let candidate_url = format!("https://{authority_str}");
    let Ok(parsed) = Url::parse(&candidate_url) else {
        state.record_event(
            &method,
            &authority_str,
            "",
            false,
            "invalid_connect_target",
            None,
        );
        return forbidden_response("invalid_connect_target", "Cible CONNECT invalide");
    };

    let Ok(origin) = normalize_origin(&parsed) else {
        state.record_event(
            &method,
            &authority_str,
            "",
            false,
            "invalid_connect_origin",
            None,
        );
        return forbidden_response("invalid_connect_origin", "Origine CONNECT invalide");
    };

    if let Err(BrowserError::EgressBlocked { reason, .. }) =
        state.policy.check_origin(&origin, state.model_policy)
    {
        state.record_event(&method, &authority_str, &origin, false, &reason, None);
        return forbidden_response(&reason, "Tunnel CONNECT hors mandat refusé");
    }

    let mut tunnel_cancel = cancel_rx.clone();
    let tunnel_authority = authority_str.clone();
    let tunnel_fut: TunnelFuture = Box::pin(async move {
        tokio::select! {
            () = wait_for_cancel(&mut tunnel_cancel) => {}
            () = run_connect_tunnel(req, &tunnel_authority) => {}
        }
    });

    if !matches!(
        tokio::time::timeout(PROXY_CONNECT_TIMEOUT, tunnel_tx.send(tunnel_fut)).await,
        Ok(Ok(()))
    ) {
        state.record_event(
            &method,
            &authority_str,
            &origin,
            false,
            "proxy_tunnel_queue_unavailable",
            None,
        );
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "proxy_tunnel_queue_unavailable",
            "File de tunnels CONNECT indisponible",
        );
    }

    state.record_event(
        &method,
        &authority_str,
        &origin,
        true,
        "allowed_connect",
        None,
    );

    Response::builder()
        .status(StatusCode::OK)
        .body(empty_body())
        .unwrap_or_else(|_| Response::new(empty_body()))
}

async fn run_connect_tunnel(req: Request<Incoming>, authority: &str) {
    let Ok(Ok(upgraded)) =
        tokio::time::timeout(PROXY_CONNECT_TIMEOUT, hyper::upgrade::on(req)).await
    else {
        return;
    };
    let Ok(Ok(mut server_stream)) =
        tokio::time::timeout(PROXY_CONNECT_TIMEOUT, TcpStream::connect(authority)).await
    else {
        return;
    };
    let mut client_io = TokioIo::new(upgraded);
    let _ = tokio::time::timeout(
        PROXY_IO_TIMEOUT,
        tokio::io::copy_bidirectional(&mut client_io, &mut server_stream),
    )
    .await;
}

async fn forward_http_with_redirect_filter(
    state: Arc<ProxyState>,
    req: Request<Incoming>,
    target_url: Url,
    origin: String,
) -> Result<Response<ProxyBody>, Infallible> {
    let method = req.method().clone();
    let Ok(upstream_uri) = target_url.as_str().parse::<Uri>() else {
        state.record_event(
            &method,
            target_url.as_str(),
            &origin,
            false,
            "invalid_upstream_uri",
            None,
        );
        return Ok(forbidden_response(
            "invalid_upstream_uri",
            "URI amont invalide",
        ));
    };

    let (mut parts, body) = req.into_parts();
    parts.uri = upstream_uri;
    parts.headers.remove("proxy-connection");
    let outbound = Request::from_parts(parts, body.boxed());

    let upstream_resp = match state.client.request(outbound).await {
        Ok(resp) => resp,
        Err(err) => {
            state.record_event(
                &method,
                target_url.as_str(),
                &origin,
                false,
                "upstream_error",
                None,
            );
            return Ok(error_response(
                StatusCode::BAD_GATEWAY,
                "upstream_error",
                &format!("Échec amont EgressProxy : {err}"),
            ));
        }
    };

    if state.policy.block_redirects_outside_mandate
        && upstream_resp.status().is_redirection()
        && let Some(blocked) =
            check_redirect_response(&state, &method, &target_url, &origin, &upstream_resp)
    {
        return Ok(blocked);
    }

    state.record_event(&method, target_url.as_str(), &origin, true, "allowed", None);
    Ok(upstream_resp.map(BodyExt::boxed))
}

fn check_redirect_response(
    state: &ProxyState,
    method: &Method,
    target_url: &Url,
    origin: &str,
    upstream_resp: &Response<Incoming>,
) -> Option<Response<ProxyBody>> {
    let Some(loc_str) = upstream_resp
        .headers()
        .get(http::header::LOCATION)
        .and_then(|v| v.to_str().ok())
    else {
        state.record_event(
            method,
            target_url.as_str(),
            origin,
            false,
            "redirect_missing_location",
            None,
        );
        return Some(forbidden_response(
            "redirect_origin_denied",
            "Redirection sans cible vérifiable bloquée",
        ));
    };

    let Ok(resolved_redir) = target_url.join(loc_str) else {
        state.record_event(
            method,
            target_url.as_str(),
            origin,
            false,
            "redirect_unparseable",
            None,
        );
        return Some(forbidden_response(
            "redirect_origin_denied",
            "En-tête Location illisible bloqué",
        ));
    };

    let redir_origin = match normalize_origin(&resolved_redir) {
        Ok(o) => o,
        Err(BrowserError::EgressBlocked { reason, .. }) => {
            state.record_event(
                method,
                target_url.as_str(),
                origin,
                false,
                &format!("redirect_{reason}"),
                Some(resolved_redir.to_string()),
            );
            return Some(forbidden_response(
                "redirect_origin_denied",
                "Schéma de redirection interdit",
            ));
        }
        Err(_) => {
            state.record_event(
                method,
                target_url.as_str(),
                origin,
                false,
                "redirect_invalid_origin",
                None,
            );
            return Some(forbidden_response(
                "redirect_origin_denied",
                "Redirection invalide bloquée",
            ));
        }
    };

    if let Err(BrowserError::EgressBlocked { reason, .. }) =
        state.policy.check_origin(&redir_origin, state.model_policy)
    {
        state.record_event(
            method,
            target_url.as_str(),
            origin,
            false,
            &format!("redirect_{reason}"),
            Some(redir_origin),
        );
        return Some(forbidden_response(
            "redirect_origin_denied",
            "Redirection hors mandat bloquée par EgressProxy",
        ));
    }

    None
}

fn extract_http_target_url(req: &Request<Incoming>) -> Result<Url, ()> {
    let uri = req.uri();
    if uri.scheme().is_some() {
        return Url::parse(&uri.to_string()).map_err(|_| ());
    }
    let host = req
        .headers()
        .get(http::header::HOST)
        .and_then(|v| v.to_str().ok())
        .ok_or(())?;
    let path_and_query = uri
        .path_and_query()
        .map_or("/", http::uri::PathAndQuery::as_str);
    Url::parse(&format!("http://{host}{path_and_query}")).map_err(|_| ())
}

fn empty_body() -> ProxyBody {
    Empty::<Bytes>::new()
        .map_err(|never| match never {})
        .boxed()
}

fn full_body(msg: &str) -> ProxyBody {
    Full::new(Bytes::from(msg.to_owned()))
        .map_err(|never| match never {})
        .boxed()
}

fn forbidden_response(code: &str, msg: &str) -> Response<ProxyBody> {
    error_response(StatusCode::FORBIDDEN, code, msg)
}

fn error_response(status: StatusCode, code: &str, msg: &str) -> Response<ProxyBody> {
    let mut resp = Response::new(full_body(msg));
    *resp.status_mut() = status;
    if let Ok(val) = HeaderValue::from_str(code) {
        resp.headers_mut().insert("x-unleash-egress-blocked", val);
    }
    resp
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origin_normalization_rejects_non_http_and_userinfo() {
        let plain_url = Url::parse("http://EXAMPLE.com/cart").unwrap();
        assert_eq!(
            normalize_origin(&plain_url).unwrap(),
            "http://example.com:80"
        );

        let tls_url = Url::parse("https://127.0.0.1:8443/pay").unwrap();
        assert_eq!(
            normalize_origin(&tls_url).unwrap(),
            "https://127.0.0.1:8443"
        );

        let file_url = Url::parse("file:///etc/passwd").unwrap();
        assert!(matches!(
            normalize_origin(&file_url),
            Err(BrowserError::EgressBlocked { .. })
        ));

        let userinfo = Url::parse("http://user:secret@example.com/").unwrap();
        assert!(matches!(
            normalize_origin(&userinfo),
            Err(BrowserError::EgressBlocked { .. })
        ));
    }

    #[tokio::test]
    async fn proxy_connect_tunnel_runs_and_shuts_down_cleanly() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let upstream = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let upstream_addr = upstream.local_addr().unwrap();
        let mut upstream_set = JoinSet::new();
        upstream_set.spawn(async move {
            if let Ok((mut sock, _)) = upstream.accept().await {
                let mut buf = [0_u8; 4];
                if sock.read_exact(&mut buf).await.is_ok() {
                    let _ = sock.write_all(&buf).await;
                }
            }
        });

        let origin_url = format!("https://{upstream_addr}");
        let policy = EgressPolicy::from_urls(&[&origin_url]).unwrap();
        let proxy = EgressProxy::start(
            RunId::from_u128(99),
            LeaseId::from_u128(199),
            ModelPolicy::Forbidden,
            policy,
        )
        .await
        .unwrap();

        let stream = TcpStream::connect(proxy.addr()).await.unwrap();
        let io = TokioIo::new(stream);
        let (mut sender, conn) = hyper::client::conn::http1::handshake(io).await.unwrap();
        let mut conn_set = JoinSet::new();
        conn_set.spawn(async move {
            let _ = conn.with_upgrades().await;
        });

        let req = Request::builder()
            .method(Method::CONNECT)
            .uri(upstream_addr.to_string())
            .body(Empty::<Bytes>::new())
            .unwrap();
        let resp = sender.send_request(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let upgraded = hyper::upgrade::on(resp).await.unwrap();
        let mut tunnel_io = TokioIo::new(upgraded);
        tunnel_io.write_all(b"ping").await.unwrap();
        let mut echoed = [0_u8; 4];
        tunnel_io.read_exact(&mut echoed).await.unwrap();
        assert_eq!(&echoed, b"ping");
        drop(tunnel_io);
        drop(sender);

        assert!(
            proxy
                .audit_events()
                .iter()
                .any(|ev| ev.allowed && ev.reason == "allowed_connect")
        );
        proxy.shutdown().await;
        upstream_set.shutdown().await;
        conn_set.shutdown().await;
    }

    #[test]
    fn policy_blocks_canary_under_forbidden() {
        let mut policy = EgressPolicy::from_urls(&["http://127.0.0.1:8080"]).unwrap();
        policy
            .add_model_canary_url("http://127.0.0.1:9090/v1/chat")
            .unwrap();

        assert!(
            policy
                .check_origin("http://127.0.0.1:8080", ModelPolicy::Forbidden)
                .is_ok()
        );
        assert!(matches!(
            policy.check_origin("http://127.0.0.1:9090", ModelPolicy::Forbidden),
            Err(BrowserError::EgressBlocked { reason, .. }) if reason == "model_policy_forbidden"
        ));
    }
}
