//! T-F017-013 / AC-F017-007 / REQ-SEC-007 : sauvegarde interne liée à la politique engagée.
//!
//! Le courtier réservé à `RequestBackup` est lié à la révision de politique que la demande a
//! engagée. Une politique validée ne la refuse pas. Une politique qui change avant le dispatch
//! laisse la demande dans l'état terminal `blocked`, avec un motif fermé attesté par le magasin,
//! un événement publié, aucun fichier de sauvegarde et aucun dispatch.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::too_many_lines
)]

use serde_json::{Map, Value, json};
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;
use unleash_client::{ClientConfig, EventStreamReader, UnleashClient};
use unleash_daemon::{DaemonConfig, DaemonHandle, DaemonStartOutcome, UnleashDaemon};
use unleash_domain::{CommandId, RequestId, Revision};
use unleash_protocol::{
    ClientKind, CommandEnvelope, ProtocolVersion, QueryEnvelope, ReceiptState,
    SubscribeEventsFrame, SubscribeEventsRequest,
};
use unleash_store::{
    AggregateKind, CallerAuthority, CallerRole, CommandMutation, CommitCommandRequest,
    CutpointObserver, PolicyIsolationMode, PolicyScopeKind, StoreConfig, StoreCutpoint, StoreError,
    UnleashStore,
};
use unleash_testkit::{local_test_dir, private_test_dir};

#[tokio::test]
async fn standard_policy_does_not_deny_internal_backup() {
    use unleash_domain::ActorId;
    let rdv = private_test_dir("f017-007-rdv").unwrap();
    let data = local_test_dir("f017-007-data").unwrap();
    let data_dir = data.path().to_path_buf();
    let seed_dir = data_dir.clone();
    tokio::task::spawn_blocking(move || {
        let store = UnleashStore::open(StoreConfig::new(seed_dir)).unwrap();
        let generation = store.daemon_state().unwrap().authority_generation;
        let actor = ActorId::new("fixture-admin").unwrap();
        store
            .commit_command(CommitCommandRequest {
                actor_id: actor.clone(),
                envelope: CommandEnvelope {
                    protocol_version: ProtocolVersion,
                    request_id: RequestId::from_u128(701),
                    command_id: CommandId::from_u128(702),
                    authority_generation: generation,
                    actor_ref: None,
                    project_id: None,
                    aggregate_id: generation.as_uuid(),
                    expected_revision: Revision::INITIAL,
                    command_type: "SetPolicy".into(),
                    payload: Map::new(),
                },
                target_aggregate: AggregateKind::SecurityPolicy,
                operation_id: None,
                receipt_state: ReceiptState::Completed,
                result_ref: None,
                retention_days_override: None,
                fence_assertion: None,
                mutation: CommandMutation::SetPolicy {
                    scope_kind: PolicyScopeKind::Global,
                    scope_id: "global".into(),
                    isolation_mode: PolicyIsolationMode::Standard,
                    policy_version: "test-standard-v1".into(),
                    reason: "test fixture".into(),
                    caller: CallerAuthority {
                        actor_id: actor.clone(),
                        role: CallerRole::PolicyAdmin,
                    },
                },
                events: Vec::new(),
                capacity_reservations: Vec::new(),
                budget_reservations: Vec::new(),
            })
            .unwrap();
    })
    .await
    .unwrap();
    let DaemonStartOutcome::Started(handle) = UnleashDaemon::start(DaemonConfig {
        data_root: Some(data_dir),
        rendezvous_dir: Some(rdv.path().to_path_buf()),
        ..DaemonConfig::default()
    })
    .await
    .unwrap() else {
        panic!("daemon attendu")
    };
    let mut client = UnleashClient::connect(ClientConfig::new(
        handle.socket_path().to_path_buf(),
        ClientKind::Cli,
        "policy-backup-test",
    ))
    .await
    .unwrap();
    let backup_id = uuid::Uuid::from_u128(703);
    let command = CommandEnvelope {
        protocol_version: ProtocolVersion,
        request_id: RequestId::from_u128(704),
        command_id: CommandId::from_u128(705),
        authority_generation: client.hello().authority_generation,
        actor_ref: None,
        project_id: None,
        aggregate_id: backup_id,
        expected_revision: Revision::INITIAL,
        command_type: "RequestBackup".into(),
        payload: Map::new(),
    };
    let receipt = client.send_command(&command).await.unwrap();
    assert_eq!(receipt.state, ReceiptState::Accepted);

    let store_cfg = StoreConfig::new(data.path());
    let db_path = store_cfg.db_path;
    let mut last = ("aucune ligne".to_owned(), "aucune ligne".to_owned());
    let completed = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let db_path = db_path.clone();
            let observed = tokio::task::spawn_blocking(move || {
                let conn = rusqlite::Connection::open_with_flags(
                    db_path,
                    rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
                )
                .unwrap();
                conn.query_row(
                    "SELECT b.state, a.state FROM backup_requests r JOIN backup_jobs b USING(backup_id) JOIN actions a ON a.action_id=r.action_id WHERE r.backup_id=?1",
                    [backup_id.to_string()],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                )
                .ok()
            })
            .await
            .unwrap();
            if let Some((job, action)) = observed.as_ref() {
                if job == "complete" && action == "observed_success" {
                    return;
                }
                last = (job.clone(), action.clone());
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await;
    assert!(
        completed.is_ok(),
        "une politique standard validée ne doit pas refuser la sauvegarde ; derniers états observés : job='{}', action='{}'",
        last.0,
        last.1
    );

    let query = QueryEnvelope {
        protocol_version: ProtocolVersion,
        request_id: RequestId::from_u128(706),
        query_type: "GetBackupManifest".into(),
        project_id: None,
        payload: json!({"backup_id": backup_id}).as_object().unwrap().clone(),
    };
    let result = client.query(&query).await.unwrap();
    assert_eq!(result.result["state"], "completed");
    handle.shutdown_graceful("test").await.unwrap();
}

/// Fige le harnais à un point de contrôle, hors du thread writer, ou y injecte une panne.
struct HarnessGate {
    checkpoint: StoreCutpoint,
    injected: bool,
    entered: tokio::sync::Notify,
    release: Mutex<bool>,
    condvar: Condvar,
}

impl HarnessGate {
    fn new(checkpoint: StoreCutpoint, injected: bool) -> Self {
        Self {
            checkpoint,
            injected,
            entered: tokio::sync::Notify::new(),
            release: Mutex::new(false),
            condvar: Condvar::new(),
        }
    }

    fn resume(&self) {
        *self.release.lock().unwrap() = true;
        self.condvar.notify_one();
    }
}

impl CutpointObserver for HarnessGate {
    fn hit(&self, point: StoreCutpoint) -> Result<(), StoreError> {
        if point != self.checkpoint {
            return Ok(());
        }
        self.entered.notify_one();
        if self.injected {
            return Err(StoreError::CutpointInjected {
                cutpoint: "C11".to_owned(),
                message: "panne du harnais injectée avant dispatch".to_owned(),
            });
        }
        let mut release = self.release.lock().unwrap();
        while !*release {
            release = self.condvar.wait(release).unwrap();
        }
        Ok(())
    }
}

/// Démon sans politique posée : la demande engage la révision initiale, puis attend au point de contrôle.
struct BackupFixture {
    handle: DaemonHandle,
    client: UnleashClient,
    data_dir: PathBuf,
    backup_id: uuid::Uuid,
    next_request: u128,
    _rendezvous: tempfile::TempDir,
    _data: tempfile::TempDir,
}

const BACKUP_ID: u128 = 0xF017_1300;
const FIRST_REQUEST: u128 = 0xF017_1310;
const OBSERVATION_DEADLINE: Duration = Duration::from_secs(10);

fn backup_command(
    client: &UnleashClient,
    backup_id: uuid::Uuid,
    request: u128,
    command: u128,
) -> CommandEnvelope {
    CommandEnvelope {
        protocol_version: ProtocolVersion,
        request_id: RequestId::from_u128(request),
        command_id: CommandId::from_u128(command),
        authority_generation: client.hello().authority_generation,
        actor_ref: None,
        project_id: None,
        aggregate_id: backup_id,
        expected_revision: Revision::INITIAL,
        command_type: "RequestBackup".into(),
        payload: Map::new(),
    }
}

/// Politique standard posée par la socket IPC : la révision effective devient 1.
fn standard_policy_command(
    client: &UnleashClient,
    request: u128,
    command: u128,
) -> CommandEnvelope {
    CommandEnvelope {
        protocol_version: ProtocolVersion,
        request_id: RequestId::from_u128(request),
        command_id: CommandId::from_u128(command),
        authority_generation: client.hello().authority_generation,
        actor_ref: None,
        project_id: None,
        aggregate_id: client.hello().authority_generation.as_uuid(),
        expected_revision: Revision::INITIAL,
        command_type: "SetPolicy".into(),
        payload: json!({
            "scope_kind": "global",
            "scope_id": "global",
            "isolation_mode": "standard",
            "policy_version": "f017-013-changed-policy",
            "reason": "test fixture"
        })
        .as_object()
        .unwrap()
        .clone(),
    }
}

fn manifest_query(backup_id: uuid::Uuid, request: u128) -> QueryEnvelope {
    QueryEnvelope {
        protocol_version: ProtocolVersion,
        request_id: RequestId::from_u128(request),
        query_type: "GetBackupManifest".into(),
        project_id: None,
        payload: json!({"backup_id": backup_id}).as_object().unwrap().clone(),
    }
}

/// Attend l'événement `BackupRequestBlocked` de la sauvegarde et rend son motif et son état d'exécution.
async fn blocked_event(events: &mut EventStreamReader, backup_id: uuid::Uuid) -> (Value, Value) {
    tokio::time::timeout(OBSERVATION_DEADLINE, async {
        loop {
            match events.next_frame().await.unwrap() {
                Some(SubscribeEventsFrame::Events { events, .. }) => {
                    let event = events.iter().find(|event| {
                        event.event_type == "BackupRequestBlocked" && event.aggregate_id == backup_id
                    });
                    if let Some(event) = event {
                        return (
                            event.payload.get("reason").cloned().unwrap_or(Value::Null),
                            event
                                .payload
                                .get("execution_state")
                                .cloned()
                                .unwrap_or(Value::Null),
                        );
                    }
                }
                Some(_) => {}
                None => panic!("flux d'événements fermé avant le blocage"),
            }
        }
    })
    .await
    .expect("événement BackupRequestBlocked publié")
}

impl BackupFixture {
    /// Démarre le démon sans politique posée, observé au point de contrôle du harnais.
    async fn started(gate: Arc<HarnessGate>, label: &str) -> Self {
        let rendezvous = private_test_dir(&format!("f017-013-{label}-rdv")).unwrap();
        let data = local_test_dir(&format!("f017-013-{label}-data")).unwrap();
        let data_dir = data.path().to_path_buf();
        let DaemonStartOutcome::Started(handle) = UnleashDaemon::start(DaemonConfig {
            data_root: Some(data_dir.clone()),
            rendezvous_dir: Some(rendezvous.path().to_path_buf()),
            store_overrides: unleash_daemon::StoreOverrides {
                cutpoints: Some(gate),
                ..Default::default()
            },
            ..DaemonConfig::default()
        })
        .await
        .unwrap() else {
            panic!("daemon attendu")
        };
        let client = UnleashClient::connect(ClientConfig::new(
            handle.socket_path().to_path_buf(),
            ClientKind::Cli,
            "policy-backup-test",
        ))
        .await
        .unwrap();
        Self {
            handle,
            client,
            data_dir,
            backup_id: uuid::Uuid::from_u128(BACKUP_ID),
            next_request: FIRST_REQUEST,
            _rendezvous: rendezvous,
            _data: data,
        }
    }

    /// Demande une sauvegarde par la socket IPC.
    async fn request_backup(&mut self) {
        let command = backup_command(&self.client, self.backup_id, 0xF017_1301, 0xF017_1302);
        let accepted = self.client.send_command(&command).await.unwrap();
        assert_eq!(accepted.state, ReceiptState::Accepted);
    }

    fn next_request(&mut self) -> u128 {
        self.next_request += 1;
        self.next_request
    }

    async fn manifest(&mut self) -> Value {
        let query = manifest_query(self.backup_id, self.next_request());
        self.client.query(&query).await.unwrap().result
    }

    /// Tant que la demande attend, son issue reste observable : réclamée, sans motif ni manifeste.
    async fn assert_waiting_claim(&mut self) {
        let waiting = self.manifest().await;
        assert_eq!(waiting["state"], "claimed");
        assert!(waiting["blocked_reason"].is_null());
        assert!(waiting["manifest"].is_null());
    }

    /// Suit les événements publiés pour la sauvegarde à partir de maintenant.
    async fn subscribe(&mut self) -> EventStreamReader {
        let request = self.next_request();
        let (_, cursor) = self
            .client
            .get_state_snapshot(RequestId::from_u128(request), None, Map::new())
            .await
            .unwrap();
        self.client
            .subscribe_events(&SubscribeEventsRequest {
                stream_id: cursor.stream_id,
                after_sequence: cursor.sequence,
                project_id: None,
                aggregate_id: Some(self.backup_id),
                batch_limit: 32,
            })
            .await
            .unwrap()
    }

    async fn wait_for_blocked(&mut self) -> Value {
        let mut last = String::from("aucune observation");
        {
            let last = &mut last;
            tokio::time::timeout(OBSERVATION_DEADLINE, async {
                loop {
                    let manifest = self.manifest().await;
                    if manifest["state"] == "blocked" {
                        return manifest;
                    }
                    *last = manifest["state"].to_string();
                    tokio::time::sleep(Duration::from_millis(25)).await;
                }
            })
            .await
        }
        .unwrap_or_else(|_| {
            panic!(
                "aucun état terminal `blocked` sous {OBSERVATION_DEADLINE:?} ; dernier état observé : {last}"
            )
        })
    }

    /// Aucun effet : pas de répertoire de sauvegarde, aucun dispatch, job et action aux états donnés.
    async fn assert_no_backup_effect(&self, job_state: &str, action_state: Option<&str>) {
        let store = StoreConfig::new(&self.data_dir);
        assert!(!store.backups_dir.join(self.backup_id.to_string()).exists());
        let db_path = store.db_path;
        let backup_id = self.backup_id;
        let observed = tokio::task::spawn_blocking(move || {
            let conn = rusqlite::Connection::open_with_flags(
                db_path,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            )
            .unwrap();
            conn.query_row(
                "SELECT j.state, a.state, (SELECT COUNT(*) FROM action_dispatches d WHERE d.action_id=r.action_id) FROM backup_requests r JOIN backup_jobs j USING(backup_id) LEFT JOIN actions a ON a.action_id=r.action_id WHERE r.backup_id=?1",
                [backup_id.to_string()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
            .unwrap()
        })
        .await
        .unwrap();
        assert_eq!(
            observed,
            (job_state.to_owned(), action_state.map(str::to_owned), 0)
        );
    }
}

/// Demande réclamée et figée à un point de contrôle du harnais, ses événements suivis depuis avant la demande.
async fn claim_waiting_at(
    checkpoint: StoreCutpoint,
    label: &str,
) -> (Arc<HarnessGate>, BackupFixture, EventStreamReader) {
    let gate = Arc::new(HarnessGate::new(checkpoint, false));
    let mut fixture = BackupFixture::started(Arc::clone(&gate), label).await;
    let events = fixture.subscribe().await;
    fixture.request_backup().await;
    tokio::time::timeout(OBSERVATION_DEADLINE, gate.entered.notified())
        .await
        .expect("la demande est réclamée et attend à son point de contrôle");
    fixture.assert_waiting_claim().await;
    (gate, fixture, events)
}

/// La politique change par la socket IPC pendant que la demande attend, puis la demande reprend.
async fn change_policy_while_waiting(fixture: &mut BackupFixture, request: u128, gate: &HarnessGate) {
    let command = standard_policy_command(&fixture.client, request, request + 1);
    let policy = fixture.client.send_command(&command).await.unwrap();
    assert_eq!(policy.new_revision, Revision::new(1));
    gate.resume();
}

async fn assert_blocked_before_dispatch(
    fixture: &mut BackupFixture,
    events: &mut EventStreamReader,
    action_state: Option<&str>,
) {
    let blocked = fixture.wait_for_blocked().await;
    assert_eq!(blocked["blocked_reason"], "policy_changed");
    assert!(blocked["manifest"].is_null());
    assert_eq!(
        blocked["action_state"],
        action_state.map_or(Value::Null, |state| json!(state))
    );
    fixture.assert_no_backup_effect("aborted", action_state).await;
    assert_eq!(
        blocked_event(events, fixture.backup_id).await,
        (json!("policy_changed"), json!("not_dispatched"))
    );
}

// T-F017-013 / AC-F017-007 / REQ-SEC-007 : la politique change entre le claim et la revalidation.
#[tokio::test]
async fn policy_changed_after_claim_leaves_the_backup_blocked_before_dispatch() {
    let (gate, mut fixture, mut events) =
        claim_waiting_at(StoreCutpoint::C11hAfterBackupClaim, "after-claim").await;
    change_policy_while_waiting(&mut fixture, 0xF017_1303, &gate).await;
    assert_blocked_before_dispatch(&mut fixture, &mut events, None).await;
    fixture.handle.shutdown_graceful("test").await.unwrap();
}

// T-F017-013 / AC-F017-007 / REQ-SEC-007 : la politique change après la revalidation, avant la phase `Authorizing`.
#[tokio::test]
async fn policy_changed_after_revalidation_leaves_the_backup_blocked_before_dispatch() {
    let (gate, mut fixture, mut events) =
        claim_waiting_at(StoreCutpoint::C11iAfterBackupRevalidation, "after-revalidation").await;
    change_policy_while_waiting(&mut fixture, 0xF017_1323, &gate).await;
    assert_blocked_before_dispatch(&mut fixture, &mut events, None).await;
    fixture.handle.shutdown_graceful("test").await.unwrap();
}

// T-F017-013 / AC-F017-007 / REQ-SEC-007 : décision allow, puis la politique change avant le dispatch.
#[tokio::test]
async fn policy_changed_after_authorization_never_dispatches_the_backup() {
    let (gate, mut fixture, mut events) =
        claim_waiting_at(StoreCutpoint::C11jAfterBackupAuthorization, "after-authorization").await;
    change_policy_while_waiting(&mut fixture, 0xF017_1333, &gate).await;
    assert_blocked_before_dispatch(&mut fixture, &mut events, Some("authorized")).await;
    fixture.handle.shutdown_graceful("test").await.unwrap();
}

// T-F017-013 / AC-F017-007 / REQ-SEC-007 : contrôle interne du harnais en échec avant tout marqueur d'effet.
#[tokio::test]
async fn harness_failure_before_dispatch_leaves_the_backup_blocked() {
    let gate = Arc::new(HarnessGate::new(StoreCutpoint::C11hAfterBackupClaim, true));
    let mut fixture = BackupFixture::started(Arc::clone(&gate), "harness-failure").await;
    let mut events = fixture.subscribe().await;
    fixture.request_backup().await;
    tokio::time::timeout(OBSERVATION_DEADLINE, gate.entered.notified())
        .await
        .expect("la panne est injectée au point de contrôle");
    let blocked = fixture.wait_for_blocked().await;
    assert_eq!(blocked["blocked_reason"], "preflight_failed");
    assert!(blocked["manifest"].is_null());
    assert!(blocked["action_state"].is_null());
    fixture.assert_no_backup_effect("aborted", None).await;
    assert_eq!(
        blocked_event(&mut events, fixture.backup_id).await,
        (json!("preflight_failed"), json!("not_dispatched"))
    );
    fixture.handle.shutdown_graceful("test").await.unwrap();
}
