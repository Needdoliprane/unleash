// SPDX-License-Identifier: Apache-2.0
//! Tests d'intégration de `unleash-client` sur serveur socket Unix de test sous `/tmp` :
//! handshake `Hello` v2, rejet v1 / trame > 8 MiB / JSON dupliqué, corrélation idempotente
//! persistable, refus `STALE_AUTHORITY_GENERATION` sans réécriture, `SubscribeEvents` +
//! rechargement atomique snapshot+watermark sur `ResyncRequired`, et upload segmenté `ClientUploadSegment`.

#![allow(clippy::unwrap_used, clippy::panic, clippy::too_many_lines)]

use std::collections::HashMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde_json::{Map, Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use unleash_client::{
    ClientConfig, ClientError, CommandCorrelationQueue, CorrelationState, EventFollowerConfig,
    EventSyncUpdate, EventSynchronizer, UnleashClient,
};
use unleash_domain::{
    AuthorityGeneration, CommandId, DaemonEpoch, DaemonInstanceId, OperationId, ProjectId,
    ReceiptId, RequestId, Revision, Sequence, Sha256Digest, StreamId,
};
use unleash_platform::{bind_private_unix_listener, verify_same_uid_peer};
use unleash_protocol::{
    ClientKind, ClientUploadSegment, CommandEnvelope, CommandReceipt, CommandReceiptInspection,
    ErrorCode, EventCausation, EventCursor, EventEnvelope, HelloRequest, HelloResponse, IpcError,
    LineageStatus, MAX_CLIENT_UPLOAD_BYTES, MAX_FRAME_BYTES, ProtocolError, ProtocolVersion,
    QueryEnvelope, QueryResponse, ReasonCode, ReceiptState, RetryClass, Sensitivity,
    SubscribeEventsFrame, SubscribeEventsRequest, compute_command_digest_v2, from_json_str_strict,
};
use unleash_testkit::local_test_dir;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OneShotFault {
    V1Hello,
    OversizedFrame,
    DuplicateJsonKey,
    DropAfterCommit,
    MissingWatermark,
}

#[derive(Default)]
struct FaultScript {
    next_fault: Option<OneShotFault>,
    inject_duplicate_first_event: bool,
    emit_event_on_snapshot_read: Option<EventEnvelope>,
}

impl FaultScript {
    fn take_if(&mut self, expected: OneShotFault) -> bool {
        if self.next_fault == Some(expected) {
            self.next_fault = None;
            true
        } else {
            false
        }
    }
}

struct TestServerState {
    authority_generation: AuthorityGeneration,
    daemon_epoch: DaemonEpoch,
    daemon_instance_id: DaemonInstanceId,
    stream_id: StreamId,
    suggested_segment_bytes: u32,
    mutation_count: u64,
    snapshot_state: Value,
    events: Vec<EventEnvelope>,
    oldest_retained_seq: u64,
    receipts: HashMap<(String, CommandId), (Sha256Digest, AuthorityGeneration, CommandReceipt)>,
    faults: FaultScript,
}

impl TestServerState {
    fn new(gen_u128: u128, stream_u128: u128) -> Self {
        Self {
            authority_generation: AuthorityGeneration::from_u128(gen_u128),
            daemon_epoch: DaemonEpoch::new(1),
            daemon_instance_id: DaemonInstanceId::from_u128(0xD000_0001),
            stream_id: StreamId::from_u128(stream_u128),
            suggested_segment_bytes: 32 * 1024,
            mutation_count: 0,
            snapshot_state: json!({ "missions": 0, "status": "ready" }),
            events: Vec::new(),
            oldest_retained_seq: 0,
            receipts: HashMap::new(),
            faults: FaultScript::default(),
        }
    }

    fn current_watermark(&self) -> EventCursor {
        let last_seq = self.events.last().map_or(0, |ev| ev.sequence.as_u64());
        EventCursor::new(self.stream_id, Sequence::new(last_seq))
    }

    fn push_event(&mut self, seq: u64, event_type: &str, payload: Value) -> EventEnvelope {
        let mut map = Map::new();
        if let Value::Object(obj) = payload {
            map = obj;
        }
        let ts: DateTime<Utc> = "2026-11-16T12:00:00Z".parse().unwrap();
        let ev = EventEnvelope {
            event_id: Uuid::from_u128(
                0xE000_0000 + u128::from(seq) + self.stream_id.as_uuid().as_u128(),
            ),
            stream_id: self.stream_id,
            sequence: Sequence::new(seq),
            event_type: event_type.to_owned(),
            schema_version: 1,
            occurred_at: ts,
            recorded_at: ts,
            project_id: None,
            aggregate_id: Uuid::from_u128(0xA000_0001),
            aggregate_revision: Revision::new(seq),
            actor_ref: String::from("client:local-user"),
            correlation_id: Uuid::from_u128(0xC000_0001),
            causation: EventCausation::from_command_receipt(ReceiptId::from_u128(
                0xB000_0000 + u128::from(seq),
            )),
            command_id: None,
            payload_hash: Sha256Digest::from_bytes(format!("{event_type}:{seq}").as_bytes()),
            payload: map,
            sensitivity: Sensitivity::Project,
        };
        self.events.push(ev.clone());
        ev
    }
}

struct TestDaemon {
    socket_path: PathBuf,
    state: Arc<Mutex<TestServerState>>,
    notify: Arc<Notify>,
    accept_task: JoinHandle<()>,
    _sandbox: tempfile::TempDir,
}

impl TestDaemon {
    fn start(state: TestServerState) -> Self {
        let sandbox = local_test_dir("ipc").unwrap();
        let rdv = sandbox.path().join("rdv");
        unleash_platform::ensure_private_rendezvous_dir(&rdv).unwrap();
        let socket_path = rdv.join("daemon.sock");
        let std_listener = bind_private_unix_listener(&socket_path).unwrap();
        std_listener.set_nonblocking(true).unwrap();
        let listener = UnixListener::from_std(std_listener).unwrap();

        let state = Arc::new(Mutex::new(state));
        let notify = Arc::new(Notify::new());
        let state_bg = Arc::clone(&state);
        let notify_bg = Arc::clone(&notify);

        let accept_task = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let st = Arc::clone(&state_bg);
                let nf = Arc::clone(&notify_bg);
                tokio::spawn(async move {
                    let _ = handle_connection(stream, st, nf).await;
                });
            }
        });

        Self {
            socket_path,
            state,
            notify,
            accept_task,
            _sandbox: sandbox,
        }
    }

    fn client_config(&self) -> ClientConfig {
        ClientConfig::new(self.socket_path.clone(), ClientKind::Cli, "0.1.0")
            .with_timeouts(Duration::from_secs(3), Duration::from_secs(3))
    }
}

impl Drop for TestDaemon {
    fn drop(&mut self) {
        self.accept_task.abort();
    }
}

async fn read_server_frame(stream: &mut UnixStream) -> std::io::Result<Option<String>> {
    let mut header = [0u8; 4];
    match stream.read_exact(&mut header).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let len = u32::from_be_bytes(header) as usize;
    assert!(len <= MAX_FRAME_BYTES);
    let mut body = vec![0u8; len];
    stream.read_exact(&mut body).await?;
    Ok(Some(String::from_utf8(body).unwrap()))
}

async fn write_server_bytes(stream: &mut UnixStream, bytes: &[u8]) -> std::io::Result<()> {
    let len = u32::try_from(bytes.len()).unwrap();
    stream.write_all(&len.to_be_bytes()).await?;
    stream.write_all(bytes).await?;
    stream.flush().await
}

async fn write_server_json<T: serde::Serialize>(
    stream: &mut UnixStream,
    val: &T,
) -> std::io::Result<()> {
    let bytes = serde_json::to_vec(val).unwrap();
    write_server_bytes(stream, &bytes).await
}

async fn handle_connection(
    mut stream: UnixStream,
    state: Arc<Mutex<TestServerState>>,
    notify: Arc<Notify>,
) -> std::io::Result<()> {
    verify_same_uid_peer(&stream)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::PermissionDenied, e))?;
    let actor_ref = format!("uid:{}", unleash_platform::effective_uid());

    let Some(hello_raw) = read_server_frame(&mut stream).await? else {
        return Ok(());
    };
    let _hello_req: HelloRequest = from_json_str_strict(&hello_raw)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

    let (hello_resp_bytes, force_v1) = {
        let mut st = state.lock().unwrap();
        if st.faults.take_if(OneShotFault::V1Hello) {
            (Vec::new(), true)
        } else {
            let resp = HelloResponse {
                protocol_version: ProtocolVersion,
                authority_generation: st.authority_generation,
                daemon_epoch: st.daemon_epoch,
                daemon_instance_id: st.daemon_instance_id,
                stream_id: st.stream_id,
                max_frame_bytes: u32::try_from(MAX_FRAME_BYTES).unwrap(),
                suggested_segment_bytes: st.suggested_segment_bytes,
                idempotency_retention_seconds: 604_800,
                capabilities: vec![String::from("core_v2"), String::from("events")],
            };
            (serde_json::to_vec(&resp).unwrap(), false)
        }
    };

    if force_v1 {
        let err = IpcError::new(
            ErrorCode::ProtocolIncompatible,
            None,
            "Version v1 refusée",
            RetryClass::Never,
        );
        return write_server_json(&mut stream, &err).await;
    }
    write_server_bytes(&mut stream, &hello_resp_bytes).await?;

    let mut uploads: HashMap<CommandId, Vec<ClientUploadSegment>> = HashMap::new();

    while let Some(raw) = read_server_frame(&mut stream).await? {
        let (send_oversized, send_dup) = {
            let mut st = state.lock().unwrap();
            let oversized = st.faults.take_if(OneShotFault::OversizedFrame);
            let dup = !oversized && st.faults.take_if(OneShotFault::DuplicateJsonKey);
            (oversized, dup)
        };
        if send_oversized {
            let bad_len = u32::try_from(MAX_FRAME_BYTES + 1).unwrap();
            stream.write_all(&bad_len.to_be_bytes()).await?;
            stream.flush().await?;
            return Ok(());
        }
        if send_dup {
            let dup = br#"{"request_id":"00000000-0000-4000-8000-000000000001","result":{},"result":{"dup":true}}"#;
            return write_server_bytes(&mut stream, dup).await;
        }

        if let Ok(seg) = from_json_str_strict::<ClientUploadSegment>(&raw) {
            seg.validate()
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
            uploads.entry(seg.command_id).or_default().push(seg);
            continue;
        }

        if let Ok(cmd) = from_json_str_strict::<CommandEnvelope>(&raw) {
            let reply = process_command(&cmd, &actor_ref, &state, &notify, &mut uploads);
            match reply {
                CommandAction::DropConnection => return Ok(()),
                CommandAction::ReplyReceipt(r) => write_server_json(&mut stream, &r).await?,
                CommandAction::ReplyError(e) => write_server_json(&mut stream, &e).await?,
            }
            continue;
        }

        if let Ok(query) = from_json_str_strict::<QueryEnvelope>(&raw) {
            let resp = process_query(&query, &state);
            write_server_json(&mut stream, &resp).await?;
            continue;
        }

        if let Ok(sub_req) = from_json_str_strict::<SubscribeEventsRequest>(&raw) {
            run_subscription_loop(&mut stream, sub_req, &state, &notify).await?;
            return Ok(());
        }

        let err = IpcError::new(
            ErrorCode::InvalidArgument,
            None,
            "Trame inconnue du serveur de test",
            RetryClass::Never,
        );
        write_server_json(&mut stream, &err).await?;
    }

    Ok(())
}

enum CommandAction {
    DropConnection,
    ReplyReceipt(CommandReceipt),
    ReplyError(IpcError),
}

fn process_command(
    cmd: &CommandEnvelope,
    actor_ref: &str,
    state: &Arc<Mutex<TestServerState>>,
    notify: &Arc<Notify>,
    uploads: &mut HashMap<CommandId, Vec<ClientUploadSegment>>,
) -> CommandAction {
    let mut st = state.lock().unwrap();
    // Contrôle autorité AVANT lookup idempotence (ipc-and-apis.md)
    if cmd.authority_generation != st.authority_generation {
        return CommandAction::ReplyError(
            IpcError::from_reason(
                ReasonCode::StaleAuthorityGeneration,
                "Lignée d'autorité différente de la lignée active",
            )
            .with_command_id(cmd.command_id),
        );
    }

    let digest = compute_command_digest_v2(cmd).unwrap();
    let key = (actor_ref.to_owned(), cmd.command_id);
    if let Some((stored_digest, stored_gen, stored_receipt)) = st.receipts.get(&key)
        && *stored_gen == st.authority_generation
    {
        if *stored_digest == digest {
            uploads.remove(&cmd.command_id);
            return CommandAction::ReplyReceipt(stored_receipt.clone());
        }
        return CommandAction::ReplyError(
            IpcError::new(
                ErrorCode::IdempotencyConflict,
                None,
                "Même command_id avec un digest canonique différent",
                RetryClass::Never,
            )
            .with_command_id(cmd.command_id),
        );
    }

    if cmd.command_type == "ImportAutomationData"
        && let Some(source) = cmd.payload.get("source").and_then(Value::as_object)
        && source.get("kind").and_then(Value::as_str) == Some("client_upload")
    {
        let expected_len = source.get("byte_length").and_then(Value::as_u64);
        let expected_sha = source.get("sha256").and_then(Value::as_str);
        let segs = uploads.remove(&cmd.command_id).unwrap_or_default();
        if !verify_reassembled_upload(&segs, expected_len, expected_sha) {
            return CommandAction::ReplyError(
                IpcError::from_reason(
                    ReasonCode::ClientUploadMismatch,
                    "Segments client_upload incomplets ou digest différent",
                )
                .with_command_id(cmd.command_id),
            );
        }
    }

    st.mutation_count += 1;
    let next_seq = st.events.last().map_or(1, |e| e.sequence.as_u64() + 1);
    st.push_event(
        next_seq,
        "CommandCommitted",
        json!({ "cmd": cmd.command_type }),
    );

    let receipt = CommandReceipt {
        receipt_id: ReceiptId::from_u128(0x9000_0000 + u128::from(st.mutation_count)),
        command_id: cmd.command_id,
        accepted_at: "2026-11-16T12:00:00Z".parse().unwrap(),
        aggregate_id: cmd.aggregate_id,
        new_revision: cmd.expected_revision.next(),
        operation_id: Some(OperationId::from_u128(0x8000_0001)),
        state: ReceiptState::Accepted,
        event_cursor: EventCursor::new(st.stream_id, Sequence::new(next_seq)),
        result_ref: None,
    };
    let cur_gen = st.authority_generation;
    st.receipts.insert(key, (digest, cur_gen, receipt.clone()));
    notify.notify_waiters();

    if st.faults.take_if(OneShotFault::DropAfterCommit) {
        return CommandAction::DropConnection;
    }

    CommandAction::ReplyReceipt(receipt)
}

fn verify_reassembled_upload(
    segs: &[ClientUploadSegment],
    expected_len: Option<u64>,
    expected_sha: Option<&str>,
) -> bool {
    let Some(expected_len) = expected_len else {
        return false;
    };
    let Some(expected_sha) = expected_sha else {
        return false;
    };
    let Some(first) = segs.first() else {
        return false;
    };
    let total = first.total_segments as usize;
    if segs.len() != total {
        return false;
    }
    let mut ordered = segs.to_vec();
    ordered.sort_by_key(|s| s.segment_index);
    let mut assembled = Vec::new();
    for (idx, seg) in ordered.iter().enumerate() {
        if seg.segment_index as usize != idx || seg.total_segments as usize != total {
            return false;
        }
        if seg.byte_offset != assembled.len() as u64 {
            return false;
        }
        let Ok(chunk) = seg.decode_and_validate() else {
            return false;
        };
        assembled.extend_from_slice(&chunk);
    }
    if assembled.len() as u64 != expected_len {
        return false;
    }
    Sha256Digest::from_bytes(&assembled).as_prefixed_str() == expected_sha
}

fn process_query(query: &QueryEnvelope, state: &Arc<Mutex<TestServerState>>) -> QueryResponse {
    let mut st = state.lock().unwrap();
    if query.query_type == "GetStateSnapshot" {
        if let Some(ev) = st.faults.emit_event_on_snapshot_read.take() {
            st.events.push(ev);
        }
        let watermark = if st.faults.take_if(OneShotFault::MissingWatermark) {
            None
        } else {
            Some(st.current_watermark())
        };
        return QueryResponse {
            request_id: query.request_id,
            watermark,
            result: st.snapshot_state.clone(),
        };
    }

    if query.query_type == "GetCommandReceipt"
        && let Some(cmd_str) = query.payload.get("command_id").and_then(Value::as_str)
        && let Ok(cmd_id) = cmd_str.parse::<CommandId>()
        && let Some((_, stored_gen, receipt)) = st
            .receipts
            .iter()
            .find(|((_, c), _)| *c == cmd_id)
            .map(|(_, v)| v)
    {
        let lineage_status = if *stored_gen == st.authority_generation {
            LineageStatus::Current
        } else {
            LineageStatus::Historical
        };
        let inspection = CommandReceiptInspection {
            receipt: receipt.clone(),
            authority_generation: *stored_gen,
            lineage_status,
            provenance_ref: Some(String::from("test-journal")),
        };
        return QueryResponse {
            request_id: query.request_id,
            watermark: Some(st.current_watermark()),
            result: serde_json::to_value(&inspection).unwrap(),
        };
    }

    QueryResponse {
        request_id: query.request_id,
        watermark: Some(st.current_watermark()),
        result: json!({ "ok": true }),
    }
}

async fn run_subscription_loop(
    stream: &mut UnixStream,
    sub_req: SubscribeEventsRequest,
    state: &Arc<Mutex<TestServerState>>,
    notify: &Arc<Notify>,
) -> std::io::Result<()> {
    let mut sent_after = sub_req.after_sequence.as_u64();

    loop {
        let frame_to_send = {
            let mut st = state.lock().unwrap();
            if sub_req.stream_id != st.stream_id || sent_after < st.oldest_retained_seq {
                Some(SubscribeEventsFrame::ResyncRequired {
                    stream_id: st.stream_id,
                    current_watermark: st.current_watermark(),
                    reason: String::from("stream_id restauré ou curseur expiré"),
                })
            } else {
                let mut batch: Vec<EventEnvelope> = st
                    .events
                    .iter()
                    .filter(|ev| ev.sequence.as_u64() > sent_after)
                    .take(sub_req.batch_limit as usize)
                    .cloned()
                    .collect();
                if batch.is_empty() {
                    None
                } else {
                    if st.faults.inject_duplicate_first_event {
                        st.faults.inject_duplicate_first_event = false;
                        if let Some(first) = batch.first().cloned() {
                            batch.insert(1, first);
                        }
                    }
                    let max_seq = batch
                        .iter()
                        .map(|e| e.sequence.as_u64())
                        .max()
                        .unwrap_or(sent_after);
                    sent_after = max_seq;
                    Some(SubscribeEventsFrame::Events {
                        events: batch,
                        cursor: EventCursor::new(st.stream_id, Sequence::new(max_seq)),
                    })
                }
            }
        };

        if let Some(frame) = frame_to_send {
            let is_resync = matches!(frame, SubscribeEventsFrame::ResyncRequired { .. });
            write_server_json(stream, &frame).await?;
            if is_resync {
                return Ok(());
            }
            continue;
        }

        notify.notified().await;
    }
}

fn make_command_envelope(
    generation: AuthorityGeneration,
    cmd_id: CommandId,
    req_id: RequestId,
    cmd_type: &str,
    payload: Value,
) -> CommandEnvelope {
    let mut map = Map::new();
    if let Value::Object(obj) = payload {
        map = obj;
    }
    CommandEnvelope {
        protocol_version: ProtocolVersion,
        request_id: req_id,
        command_id: cmd_id,
        authority_generation: generation,
        actor_ref: None,
        project_id: Some(ProjectId::from_u128(0xF000_0001_u128)),
        aggregate_id: Uuid::from_u128(0xA000_0001),
        expected_revision: Revision::new(0),
        command_type: cmd_type.to_owned(),
        payload: map,
    }
}

// T-F001-003 / AC-F001-003 / REQ-CORE-003
#[tokio::test]
async fn t_f001_003_hello_negotiation_and_strict_rejections() {
    let daemon = TestDaemon::start(TestServerState::new(0x100, 0x200));

    // 1. Handshake nominal unleash.ipc.v2
    let mut client = UnleashClient::connect(daemon.client_config())
        .await
        .unwrap();
    assert_eq!(client.hello().protocol_version, ProtocolVersion);
    assert_eq!(
        client.hello().authority_generation,
        AuthorityGeneration::from_u128(0x100)
    );
    assert_eq!(client.hello().stream_id, StreamId::from_u128(0x200));

    // 2. Rejet trame > 8 MiB avant allocation
    {
        daemon.state.lock().unwrap().faults.next_fault = Some(OneShotFault::OversizedFrame);
    }
    let query = QueryEnvelope {
        protocol_version: ProtocolVersion,
        request_id: RequestId::from_u128(1),
        query_type: String::from("GetDaemonStatus"),
        project_id: None,
        payload: Map::new(),
    };
    let oversized_err = client.query(&query).await.unwrap_err();
    assert!(matches!(
        oversized_err,
        ClientError::Protocol(ProtocolError::FrameTooLarge { .. })
    ));

    // 3. Rejet clé JSON dupliquée après reconnexion
    client.reconnect().await.unwrap();
    {
        daemon.state.lock().unwrap().faults.next_fault = Some(OneShotFault::DuplicateJsonKey);
    }
    let dup_err = client.query(&query).await.unwrap_err();
    assert!(matches!(
        dup_err,
        ClientError::Protocol(ProtocolError::DuplicateKey { .. })
    ));

    // 4. Rejet ProtocolIncompatible au handshake
    {
        daemon.state.lock().unwrap().faults.next_fault = Some(OneShotFault::V1Hello);
    }
    let v1_err = UnleashClient::connect(daemon.client_config())
        .await
        .unwrap_err();
    assert!(matches!(
        v1_err,
        ClientError::Remote(ref e) if e.code == ErrorCode::ProtocolIncompatible
    ));

    // 5. Rejet connexion si permissions du socket dégradées à 0644
    fs::set_permissions(&daemon.socket_path, PermissionsExt::from_mode(0o644)).unwrap();
    let perm_err = UnleashClient::connect(daemon.client_config())
        .await
        .unwrap_err();
    assert!(matches!(perm_err, ClientError::Platform(_)));
}

// T-F001-005 / AC-F001-005 / REQ-CORE-005
#[tokio::test]
async fn t_f001_005_correlated_retry_and_stale_authority_generation() {
    let daemon = TestDaemon::start(TestServerState::new(0x111, 0x222));
    let queue_dir = local_test_dir("corrq").unwrap();
    let queue_path = queue_dir.path().join("correlation-queue.json");

    // Garde .claude/ sur la file de corrélation
    assert!(matches!(
        CommandCorrelationQueue::open_or_create(PathBuf::from("/tmp/evil/.claude/queue.json")),
        Err(ClientError::Platform(_))
    ));

    let mut client = UnleashClient::connect(daemon.client_config())
        .await
        .unwrap();
    let g1 = client.hello().authority_generation;

    // Cas 1 : Coupure après commit serveur -> rechargement depuis disque -> retry idempotent
    let cmd_1 = CommandId::from_u128(0xC001);
    let req_1 = RequestId::from_u128(0xA001);
    let env_1 = make_command_envelope(
        g1,
        cmd_1,
        req_1,
        "PauseMission",
        json!({ "reason": "test" }),
    );

    {
        let mut queue = CommandCorrelationQueue::open_or_create(queue_path.clone()).unwrap();
        queue.enqueue(env_1.clone(), 1000).unwrap();

        daemon.state.lock().unwrap().faults.next_fault = Some(OneShotFault::DropAfterCommit);
        let first_err = client
            .submit_correlated(&mut queue, cmd_1, RequestId::from_u128(0xA002), 1001)
            .await
            .unwrap_err();
        assert!(matches!(first_err, ClientError::Disconnected { .. }));
        assert!(matches!(
            queue.get(cmd_1).unwrap().state,
            CorrelationState::Uncertain { attempts: 1, .. }
        ));
    }

    // Recharger la file depuis disque (simule redémarrage client) et reconnecter
    let mut reloaded_queue = CommandCorrelationQueue::open_or_create(queue_path.clone()).unwrap();
    client.reconnect().await.unwrap();

    let fresh_req_2 = RequestId::from_u128(0xA003);
    let receipt = client
        .submit_correlated(&mut reloaded_queue, cmd_1, fresh_req_2, 1002)
        .await
        .unwrap();

    assert_eq!(receipt.command_id, cmd_1);
    assert_eq!(
        daemon.state.lock().unwrap().mutation_count,
        1,
        "Aucune double mutation autorisée"
    );
    let confirmed_entry = reloaded_queue.get(cmd_1).unwrap();
    assert_eq!(confirmed_entry.envelope.authority_generation, g1);
    assert_eq!(confirmed_entry.envelope.request_id, fresh_req_2);
    assert!(matches!(
        confirmed_entry.state,
        CorrelationState::Confirmed { .. }
    ));

    // Inspection GetCommandReceipt dans la lignée active => lineage_status = Current
    let inspected_cur = client
        .get_command_receipt(RequestId::from_u128(0xA099), cmd_1)
        .await
        .unwrap();
    assert_eq!(inspected_cur.lineage_status, LineageStatus::Current);
    assert_eq!(inspected_cur.authority_generation, g1);

    // Cas 1b : Même command_id avec payload différent => IdempotencyConflict
    let divergent = make_command_envelope(
        g1,
        cmd_1,
        RequestId::from_u128(0xA004),
        "PauseMission",
        json!({ "reason": "autre" }),
    );
    let conflict_err = client.send_command(&divergent).await.unwrap_err();
    assert!(matches!(
        conflict_err,
        ClientError::Remote(ref e) if e.code == ErrorCode::IdempotencyConflict
    ));

    // Cas 2 : Restauration daemon vers G2 -> aucune réécriture silencieuse après STALE_AUTHORITY_GENERATION
    let cmd_2 = CommandId::from_u128(0xC002);
    let env_2 = make_command_envelope(
        g1,
        cmd_2,
        RequestId::from_u128(0xB001),
        "ResumeMission",
        json!({}),
    );
    reloaded_queue.enqueue(env_2, 1005).unwrap();

    let g2 = AuthorityGeneration::from_u128(0x222);
    {
        let mut st = daemon.state.lock().unwrap();
        st.authority_generation = g2;
        st.stream_id = StreamId::from_u128(0x333);
    }

    client.reconnect().await.unwrap();
    assert_eq!(client.hello().authority_generation, g2);

    // Inspection GetCommandReceipt de cmd_1 après restauration vers G2 => Historical
    let inspected_hist = client
        .get_command_receipt(RequestId::from_u128(0xB099), cmd_1)
        .await
        .unwrap();
    assert_eq!(inspected_hist.lineage_status, LineageStatus::Historical);
    assert_eq!(inspected_hist.authority_generation, g1);

    let stale_err = client
        .submit_correlated(
            &mut reloaded_queue,
            cmd_2,
            RequestId::from_u128(0xB002),
            1010,
        )
        .await
        .unwrap_err();
    assert!(matches!(
        stale_err,
        ClientError::Remote(ref e)
            if e.code == ErrorCode::StaleFence
            && e.reason_code == Some(ReasonCode::StaleAuthorityGeneration)
            && e.retry_class == RetryClass::Never
    ));

    // Vérifier que la file persistée sur disque conserve G1 et cmd_2 intacts et bloque tout nouveau retry
    let disk_queue = CommandCorrelationQueue::open_or_create(queue_path).unwrap();
    let stale_entry = disk_queue.get(cmd_2).unwrap();
    assert_eq!(stale_entry.envelope.authority_generation, g1);
    assert_eq!(stale_entry.envelope.command_id, cmd_2);
    assert!(matches!(
        stale_entry.state,
        CorrelationState::StaleLineageRejected {
            observed_daemon_generation: Some(obs),
            ..
        } if obs == g2
    ));

    let mut locked_queue = disk_queue;
    let locked_err = client
        .submit_correlated(&mut locked_queue, cmd_2, RequestId::from_u128(0xB003), 1020)
        .await
        .unwrap_err();
    assert!(matches!(
        locked_err,
        ClientError::StaleLineageLocked { command_id, pinned_generation }
            if command_id == cmd_2 && pinned_generation == g1
    ));

    // Intention explicite de récupération sous nouvelle lignée avec lien causal distinct
    let recovery_cmd_id = CommandId::from_u128(0xC003);
    let (recovery_env, supersedes) = locked_queue
        .create_explicit_recovery_intention(
            cmd_2,
            recovery_cmd_id,
            RequestId::from_u128(0xB004),
            g2,
        )
        .unwrap();
    assert_eq!(supersedes, cmd_2);
    assert_eq!(recovery_env.authority_generation, g2);
    assert_eq!(recovery_env.command_id, recovery_cmd_id);
    locked_queue
        .enqueue_with_causal_link(recovery_env, 1025, Some(supersedes))
        .unwrap();
    let rec_receipt = client
        .submit_correlated(
            &mut locked_queue,
            recovery_cmd_id,
            RequestId::from_u128(0xB005),
            1026,
        )
        .await
        .unwrap();
    assert_eq!(rec_receipt.command_id, recovery_cmd_id);
}

// T-F001-010 / AC-F001-010 / REQ-CORE-010
#[tokio::test]
async fn t_f001_010_subscribe_events_and_atomic_resync_watermark() {
    let mut init = TestServerState::new(0x1000, 0x51_u128);
    init.push_event(1, "MissionCreated", json!({ "n": 1 }));
    init.push_event(2, "MissionStarted", json!({ "n": 2 }));
    init.faults.inject_duplicate_first_event = true;
    let daemon = TestDaemon::start(init);

    let mut query_client = UnleashClient::connect(daemon.client_config())
        .await
        .unwrap();
    let mut sync = EventSynchronizer::new(
        daemon.client_config(),
        EventFollowerConfig {
            project_id: None,
            aggregate_id: None,
            batch_limit: 50,
            snapshot_query_payload: Map::new(),
        },
    );

    // 1. Chargement initial snapshot + watermark puis abonnement
    let (init_update, mut stream) = sync
        .sync_initial(query_client.connection_mut(), RequestId::from_u128(1))
        .await
        .unwrap();
    assert!(matches!(
        init_update,
        EventSyncUpdate::SnapshotLoaded { watermark, .. } if watermark.sequence == Sequence::new(2)
    ));

    // Pousser événement 3 et tester déduplication d'un doublon injecté
    {
        let mut st = daemon.state.lock().unwrap();
        st.faults.inject_duplicate_first_event = true;
        st.push_event(3, "StepAccepted", json!({ "n": 3 }));
    }
    daemon.notify.notify_waiters();

    let (ev_update, maybe_new) = sync
        .poll_next(&mut stream, query_client.connection_mut(), || {
            RequestId::from_u128(2)
        })
        .await
        .unwrap();
    assert!(maybe_new.is_none());
    match ev_update {
        EventSyncUpdate::Events { events, cursor } => {
            assert_eq!(
                events.len(),
                1,
                "Le doublon at-least-once doit être dédupliqué"
            );
            assert_eq!(events.first().unwrap().sequence, Sequence::new(3));
            assert_eq!(cursor.sequence, Sequence::new(3));
        }
        other => panic!("Attendu Events, obtenu : {other:?}"),
    }

    // 2. Restauration vers un nouveau stream S2 (seq=10) + événement 11 committé pendant GetStateSnapshot
    let s2 = StreamId::from_u128(0x52_u128);
    {
        let mut st = daemon.state.lock().unwrap();
        st.stream_id = s2;
        st.events.clear();
        st.push_event(10, "SnapshotRestored", json!({ "restored": true }));
        st.snapshot_state = json!({ "restored_at": 11 });
        let mut ev11 = st.push_event(11, "PostRestoreCommitted", json!({ "n": 11 }));
        // Retirer ev11 temporairement pour l'émettre atomiquement lors de GetStateSnapshot
        st.events.pop();
        ev11.stream_id = s2;
        st.faults.emit_event_on_snapshot_read = Some(ev11);
    }
    daemon.notify.notify_waiters();

    let (resync_update, maybe_stream2) = sync
        .poll_next(&mut stream, query_client.connection_mut(), || {
            RequestId::from_u128(3)
        })
        .await
        .unwrap();

    let mut stream2 = maybe_stream2.expect("ResyncRequired doit ouvrir un nouveau flux abonné");
    match resync_update {
        EventSyncUpdate::SnapshotLoaded {
            watermark,
            snapshot,
            resync_reason,
        } => {
            assert_eq!(watermark.stream_id, s2);
            assert_eq!(watermark.sequence, Sequence::new(11));
            assert_eq!(snapshot.get("restored_at"), Some(&json!(11)));
            assert!(resync_reason.is_some());
        }
        other => panic!("Attendu SnapshotLoaded après ResyncRequired, obtenu : {other:?}"),
    }

    // Émettre événement 12 sur S2 et vérifier reprise continue sans trou
    {
        let mut st = daemon.state.lock().unwrap();
        st.push_event(12, "PostSnapshotEvent", json!({ "n": 12 }));
    }
    daemon.notify.notify_waiters();

    let (post_update, maybe_stream3) = sync
        .poll_next(&mut stream2, query_client.connection_mut(), || {
            RequestId::from_u128(4)
        })
        .await
        .unwrap();
    assert!(maybe_stream3.is_none());
    match post_update {
        EventSyncUpdate::Events { events, cursor } => {
            assert_eq!(events.len(), 1);
            assert_eq!(events.first().unwrap().stream_id, s2);
            assert_eq!(events.first().unwrap().sequence, Sequence::new(12));
            assert_eq!(cursor, EventCursor::new(s2, Sequence::new(12)));
        }
        other => panic!("Attendu Events(12), obtenu : {other:?}"),
    }

    // Cas négatif : GetStateSnapshot sans watermark refusé explicitement
    daemon.state.lock().unwrap().faults.next_fault = Some(OneShotFault::MissingWatermark);
    let missing_wm_err = query_client
        .get_state_snapshot(RequestId::from_u128(5), None, Map::new())
        .await
        .unwrap_err();
    assert!(matches!(
        missing_wm_err,
        ClientError::MissingSnapshotWatermark
    ));
}

#[tokio::test]
async fn test_client_upload_segmented_and_mismatch_rejection() {
    let daemon = TestDaemon::start(TestServerState::new(0x700, 0x800));
    let mut client = UnleashClient::connect(daemon.client_config())
        .await
        .unwrap();
    let g1 = client.hello().authority_generation;

    // 1. Upload multi-segments (150 KiB découpés selon suggested_segment_bytes = 32 KiB)
    let raw_bytes: Vec<u8> = (0..150_000_usize)
        .map(|i| u8::try_from(i % 251).unwrap())
        .collect();
    let cmd_id = CommandId::from_u128(0xD001);
    let digest = Sha256Digest::from_bytes(&raw_bytes);

    let mut queue = CommandCorrelationQueue::in_memory();
    let env = make_command_envelope(
        g1,
        cmd_id,
        RequestId::from_u128(0xE001),
        "ImportAutomationData",
        json!({
            "source": {
                "kind": "client_upload",
                "byte_length": raw_bytes.len(),
                "sha256": digest.as_prefixed_str()
            }
        }),
    );
    queue.enqueue(env, 2000).unwrap();

    let (summary, receipt) = client
        .upload_and_submit(
            &mut queue,
            cmd_id,
            RequestId::from_u128(0xE002),
            &raw_bytes,
            2001,
        )
        .await
        .unwrap();
    assert_eq!(summary.command_id, cmd_id);
    assert_eq!(summary.total_segments, 5);
    assert_eq!(summary.byte_length, raw_bytes.len() as u64);
    assert_eq!(summary.sha256, digest);
    assert_eq!(receipt.command_id, cmd_id);

    // 2. Cas limite : upload 0 octet (1 segment vide)
    let empty_cmd = CommandId::from_u128(0xD002);
    let empty_digest = Sha256Digest::from_bytes(b"");
    let empty_env = make_command_envelope(
        g1,
        empty_cmd,
        RequestId::from_u128(0xE003),
        "ImportAutomationData",
        json!({
            "source": {
                "kind": "client_upload",
                "byte_length": 0,
                "sha256": empty_digest.as_prefixed_str()
            }
        }),
    );
    queue.enqueue(empty_env, 2002).unwrap();
    let (empty_summary, _) = client
        .upload_and_submit(
            &mut queue,
            empty_cmd,
            RequestId::from_u128(0xE004),
            b"",
            2003,
        )
        .await
        .unwrap();
    assert_eq!(empty_summary.total_segments, 1);

    // 3. Rejet côté client si dépassement 10 MiB (MAX_CLIENT_UPLOAD_BYTES)
    let oversized = vec![0u8; usize::try_from(MAX_CLIENT_UPLOAD_BYTES).unwrap() + 1];
    let too_big_cmd = CommandId::from_u128(0xD003);
    let too_big_err = client
        .upload_and_submit(
            &mut queue,
            too_big_cmd,
            RequestId::from_u128(0xE005),
            &oversized,
            2004,
        )
        .await
        .unwrap_err();
    assert!(matches!(too_big_err, ClientError::UploadTooLarge { .. }));

    // 4. Rejet serveur InvalidArgument/CLIENT_UPLOAD_MISMATCH si digest ne correspond pas
    let bad_cmd = CommandId::from_u128(0xD004);
    let bad_env = make_command_envelope(
        g1,
        bad_cmd,
        RequestId::from_u128(0xE006),
        "ImportAutomationData",
        json!({
            "source": {
                "kind": "client_upload",
                "byte_length": 4,
                "sha256": Sha256Digest::from_bytes(b"other").as_prefixed_str()
            }
        }),
    );
    queue.enqueue(bad_env, 2005).unwrap();
    let mismatch_err = client
        .upload_and_submit(
            &mut queue,
            bad_cmd,
            RequestId::from_u128(0xE007),
            b"test",
            2006,
        )
        .await
        .unwrap_err();
    assert!(matches!(
        mismatch_err,
        ClientError::Remote(ref e)
            if e.code == ErrorCode::InvalidArgument
            && e.reason_code == Some(ReasonCode::ClientUploadMismatch)
    ));
}

#[tokio::test]
async fn test_event_sync_preserves_cursor_when_stream_open_fails() {
    let mut init = TestServerState::new(0x1000, 0x61_u128);
    init.push_event(1, "MissionCreated", json!({ "n": 1 }));
    init.push_event(2, "MissionStarted", json!({ "n": 2 }));
    let daemon = TestDaemon::start(init);

    let mut query_client = UnleashClient::connect(daemon.client_config())
        .await
        .unwrap();
    let mut sync = EventSynchronizer::new(daemon.client_config(), EventFollowerConfig::default());

    // 1. Si open_stream_at échoue après GetStateSnapshot dans sync_initial, le curseur reste None
    daemon.state.lock().unwrap().faults.next_fault = Some(OneShotFault::V1Hello);
    let init_err = sync
        .sync_initial(query_client.connection_mut(), RequestId::from_u128(0xF001))
        .await
        .unwrap_err();
    assert!(matches!(
        init_err,
        ClientError::Remote(ref e) if e.code == ErrorCode::ProtocolIncompatible
    ));
    assert_eq!(sync.current_cursor(), None);

    // Reprise saine sans défaut : établit le curseur initial sur S1
    let (_, mut stream) = sync
        .sync_initial(query_client.connection_mut(), RequestId::from_u128(0xF002))
        .await
        .unwrap();
    let s1 = StreamId::from_u128(0x61_u128);
    let s1_cursor = EventCursor::new(s1, Sequence::new(2));
    assert_eq!(sync.current_cursor(), Some(s1_cursor));

    // 2. Sur ResyncRequired -> perform_atomic_resync : si open_stream_at échoue après le nouveau
    // GetStateSnapshot sur S2, le curseur courant reste figé sur S1 au lieu d'avancer à S2
    let s2 = StreamId::from_u128(0x62_u128);
    {
        let mut st = daemon.state.lock().unwrap();
        st.stream_id = s2;
        st.events.clear();
        st.push_event(10, "SnapshotRestored", json!({ "restored": true }));
        st.faults.next_fault = Some(OneShotFault::V1Hello);
    }
    daemon.notify.notify_waiters();

    let resync_err = sync
        .poll_next(&mut stream, query_client.connection_mut(), || {
            RequestId::from_u128(0xF003)
        })
        .await
        .unwrap_err();
    assert!(matches!(
        resync_err,
        ClientError::Remote(ref e) if e.code == ErrorCode::ProtocolIncompatible
    ));
    assert_eq!(sync.current_cursor(), Some(s1_cursor));

    // Reprise ultérieure sur S2 rétablit normalement le nouveau watermark
    let (recovered, _stream2) = sync
        .sync_initial(query_client.connection_mut(), RequestId::from_u128(0xF004))
        .await
        .unwrap();
    assert!(matches!(
        recovered,
        EventSyncUpdate::SnapshotLoaded { watermark, .. }
            if watermark == EventCursor::new(s2, Sequence::new(10))
    ));
    assert_eq!(
        sync.current_cursor(),
        Some(EventCursor::new(s2, Sequence::new(10)))
    );
}
