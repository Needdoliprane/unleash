// T-F002-004 / AC-F002-004 / REQ-PERSIST-004
//! Lignée d'autorité, digest canonique v2, tombstones et fenêtre de conservation.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{Map, Value};
use unleash_domain::{
    ActorId, AuthorityGeneration, CommandId, CompletionPolicy, IdGeneratorPort, MissionId,
    MissionState, ProjectId, RequestId, Revision,
};
use unleash_protocol::{CommandEnvelope, ProtocolVersion, ReasonCode, ReceiptState};
use unleash_store::{
    AggregateKind, BackupRetentionTier, CommandMutation, CommitCommandRequest, CommitOutcome,
    EmptyExecutorNeutralizer, MonotonicClockPort, PrepareBackupRequest, ReceiptInspectionOutcome,
    ReceiptPurgeReport, RestoreBackupRequest, StoreConfig, StoreError, UnleashStore,
};
use unleash_testkit::{DeterministicIdGenerator, ManualClock, local_test_dir};

struct ClockAdapter(Arc<ManualClock>);
impl MonotonicClockPort for ClockAdapter {
    fn monotonic_now(&self) -> Duration {
        self.0.monotonic_now()
    }
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "Scénario complet T-F002-004 : rejet v1, digest RFC 8785, purge tombstone et rejet lignée avant lookup"
)]
fn test_t_f002_004_v1_reject_canonical_digest_tombstone_and_stale_lineage_before_lookup() {
    let dir = local_test_dir("f002-004").unwrap();
    let clock = Arc::new(ManualClock::default());
    let ids = Arc::new(DeterministicIdGenerator::new(0xF002_0004));
    let mut cfg = StoreConfig::new(dir.path());
    cfg.clock = clock.clone();
    cfg.monotonic_clock = Arc::new(ClockAdapter(Arc::clone(&clock)));
    cfg.id_generator = ids.clone();
    cfg.default_receipt_retention_days = 7;

    // 1. Rejeter unleash.ipc.v1 et autorité manquante/malformée
    let v1_json = r#"{
        "protocol_version": "unleash.ipc.v1",
        "request_id": "00000000-0000-4000-8000-000000000001",
        "command_id": "00000000-0000-4000-8000-000000000002",
        "authority_generation": "00000000-0000-4000-8000-000000000100",
        "actor_ref": null,
        "project_id": null,
        "aggregate_id": "00000000-0000-4000-8000-000000000004",
        "expected_revision": 0,
        "command_type": "PauseMission",
        "payload": {}
    }"#;
    let err_v1 = UnleashStore::decode_command_envelope_json(v1_json).unwrap_err();
    assert!(matches!(err_v1, StoreError::ProtocolIncompatible { .. }));

    let missing_auth = r#"{
        "protocol_version": "unleash.ipc.v2",
        "request_id": "00000000-0000-4000-8000-000000000001",
        "command_id": "00000000-0000-4000-8000-000000000002",
        "actor_ref": null,
        "project_id": null,
        "aggregate_id": "00000000-0000-4000-8000-000000000004",
        "expected_revision": 0,
        "command_type": "PauseMission",
        "payload": {}
    }"#;
    assert!(UnleashStore::decode_command_envelope_json(missing_auth).is_err());

    let store = UnleashStore::open(cfg.clone()).unwrap();
    store
        .register_principal("p1", "user", "user:louis")
        .unwrap();
    let authority_1 = store.active_authority_generation().unwrap();
    let project_id = ProjectId::from_uuid(ids.new_uuid());
    let mission_id = MissionId::from_uuid(ids.new_uuid());

    // Créer projet + backup à révision 0 de la mission
    store
        .commit_command(CommitCommandRequest {
            actor_id: ActorId::new("user:louis").unwrap(),
            envelope: CommandEnvelope {
                protocol_version: ProtocolVersion,
                request_id: RequestId::from_uuid(ids.new_uuid()),
                command_id: ids.new_command_id(),
                authority_generation: authority_1,
                actor_ref: Some("user:louis".to_owned()),
                project_id: Some(project_id),
                aggregate_id: project_id.as_uuid(),
                expected_revision: Revision::INITIAL,
                command_type: "CreateProject".to_owned(),
                payload: Map::new(),
            },
            target_aggregate: AggregateKind::Project,
            operation_id: None,
            receipt_state: ReceiptState::Completed,
            result_ref: None,
            retention_days_override: None,
            fence_assertion: None,
            mutation: CommandMutation::CreateProject {
                project_id,
                name: "p".to_owned(),
                weight: 1,
            },
            events: vec![],
            capacity_reservations: vec![],
            budget_reservations: vec![],
        })
        .unwrap();

    // Backup avant création mission
    let backup_dir = dir.path().join("backups/b-pre-mission");
    store
        .prepare_backup(PrepareBackupRequest {
            destination_dir: backup_dir.clone(),
            retention_tier: BackupRetentionTier::Manual,
            coverage: "db_and_objects".to_owned(),
        })
        .unwrap();

    // Commande v2 avec clés JSON ordonnées d'une façon, réponse "perdue"
    let cmd_k = CommandId::from_u128(777);
    let mut payload_a = Map::new();
    payload_a.insert("b".into(), Value::from(2));
    payload_a.insert("a".into(), Value::from(1));

    let req_k = CommitCommandRequest {
        actor_id: ActorId::new("user:louis").unwrap(),
        envelope: CommandEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_u128(1),
            command_id: cmd_k,
            authority_generation: authority_1,
            actor_ref: Some("user:louis".to_owned()),
            project_id: Some(project_id),
            aggregate_id: mission_id.as_uuid(),
            expected_revision: Revision::INITIAL,
            command_type: "CreateMission".to_owned(),
            payload: payload_a.clone(),
        },
        target_aggregate: AggregateKind::Mission,
        operation_id: None,
        receipt_state: ReceiptState::Accepted,
        result_ref: None,
        retention_days_override: Some(7),
        fence_assertion: None,
        mutation: CommandMutation::CreateMission {
            mission_id,
            goal_id: None,
            project_id,
            owner_principal_id: Some("p1".to_owned()),
            owner_agent_id: None,
            completion_policy: CompletionPolicy::Mechanical,
            state: MissionState::Ready,
        },
        events: vec![],
        capacity_reservations: vec![],
        budget_reservations: vec![],
    };
    let original_receipt = match store.commit_command(req_k.clone()).unwrap() {
        CommitOutcome::Committed(r) => r,
        CommitOutcome::IdempotentReplay(_) => panic!("premier commit attendu"),
    };

    // Redémarrage ordinaire (même lignée) + clés réordonnées + autre request_id -> même receipt
    let store = store.restart_same_lineage().unwrap();
    let mut payload_reordered = Map::new();
    payload_reordered.insert("a".into(), Value::from(1));
    payload_reordered.insert("b".into(), Value::from(2));
    let mut req_retry = req_k.clone();
    req_retry.envelope.request_id = RequestId::from_u128(2);
    req_retry.envelope.payload = payload_reordered;
    match store.commit_command(req_retry).unwrap() {
        CommitOutcome::IdempotentReplay(r) => assert_eq!(r, original_receipt),
        CommitOutcome::Committed(_) => panic!("rejeu idempotent attendu"),
    }

    // R7 : même clé/payload mais autre expected_revision OU autre aggregate_id -> IdempotencyConflict
    let mut req_diff_rev = req_k.clone();
    req_diff_rev.envelope.expected_revision = Revision::new(1);
    assert!(matches!(
        store.commit_command(req_diff_rev),
        Err(StoreError::IdempotencyConflict { .. })
    ));
    let mut req_diff_agg = req_k.clone();
    req_diff_agg.envelope.aggregate_id = uuid::Uuid::from_u128(9999);
    assert!(matches!(
        store.commit_command(req_diff_agg),
        Err(StoreError::IdempotencyConflict { .. })
    ));

    // Horloge incertaine suspend la purge même après 7 jours ; baisser la config de durée ne raccourcit pas la promesse
    clock.advance(Duration::from_secs(6 * 86_400));
    // Même si un réglage futur baissait la config à 1 jour, le reçu promis 7 j reste full
    store
        .config()
        .clock_uncertain
        .store(true, std::sync::atomic::Ordering::Release);
    clock.advance(Duration::from_secs(2 * 86_400)); // total 8 j
    assert_eq!(
        store.purge_expired_command_receipts().unwrap(),
        ReceiptPurgeReport::SuspendedClockUncertain
    );

    // Une fois l'horloge sûre, la purge transforme en tombstone
    store
        .config()
        .clock_uncertain
        .store(false, std::sync::atomic::Ordering::Release);
    assert_eq!(
        store.purge_expired_command_receipts().unwrap(),
        ReceiptPurgeReport::Purged {
            tombstoned_count: 2
        }
    );
    let insp = store
        .inspect_command_receipt(&ActorId::new("user:louis").unwrap(), cmd_k)
        .unwrap();
    assert!(matches!(insp, ReceiptInspectionOutcome::Tombstone { .. }));

    // Retry identique sur tombstone -> IDEMPOTENCY_WINDOW_EXPIRED (retry_class=never)
    let err_exp = store.commit_command(req_k.clone()).unwrap_err();
    assert!(matches!(
        err_exp,
        StoreError::IdempotencyWindowExpired { .. }
    ));
    assert_eq!(
        err_exp.reason_code(),
        Some(ReasonCode::IdempotencyWindowExpired)
    );
    assert_eq!(err_exp.retry_class(), unleash_protocol::RetryClass::Never);

    // Restore vers backup_dir (avant mission) : expected_revision=0 est de nouveau valide,
    // MAIS l'ancienne commande portant authority_1 reçoit StaleFence/STALE_AUTHORITY_GENERATION AVANT lookup !
    let restore_root = local_test_dir("f002-004-restored").unwrap();
    let mut restore_cfg = StoreConfig::new(restore_root.path());
    restore_cfg.clock = clock.clone();
    restore_cfg.monotonic_clock = Arc::new(ClockAdapter(Arc::clone(&clock)));
    restore_cfg.id_generator = ids.clone();

    let act_res = UnleashStore::restore_backup(
        RestoreBackupRequest {
            backup_dir,
            target_data_root: restore_root.path().to_path_buf(),
            command_id: ids.new_command_id(),
            actor_id: "user:louis".to_owned(),
            command_digest: unleash_protocol::canonical_sha256(&"restore").unwrap(),
            source_authority_generation: authority_1,
            source_db_unreadable: false,
            source_db_path: Some(dir.path().join("unleash-authority.db")),
            source_artifacts_dir: Some(dir.path().join("artifacts")),
            neutralizer: Arc::new(EmptyExecutorNeutralizer),
        },
        restore_cfg.clone(),
    )
    .unwrap();
    assert_ne!(act_res.target_authority_generation, authority_1);
    store.shutdown().unwrap();

    let restored_store = UnleashStore::open(restore_cfg).unwrap();
    let stale_err = restored_store.commit_command(req_k).unwrap_err();
    assert!(matches!(
        stale_err,
        StoreError::StaleAuthorityGeneration { .. }
    ));
    assert_eq!(
        stale_err.reason_code(),
        Some(ReasonCode::StaleAuthorityGeneration)
    );
    assert_eq!(stale_err.retry_class(), unleash_protocol::RetryClass::Never);

    // Même avec un autre authority_generation inconnu -> refus avant lookup
    let mut req_other = CommitCommandRequest {
        actor_id: ActorId::new("user:louis").unwrap(),
        envelope: CommandEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_u128(999),
            command_id: CommandId::from_u128(888),
            authority_generation: AuthorityGeneration::from_u128(0xDEAD),
            actor_ref: Some("user:louis".to_owned()),
            project_id: Some(project_id),
            aggregate_id: mission_id.as_uuid(),
            expected_revision: Revision::INITIAL,
            command_type: "CreateMission".to_owned(),
            payload: Map::new(),
        },
        target_aggregate: AggregateKind::Mission,
        operation_id: None,
        receipt_state: ReceiptState::Accepted,
        result_ref: None,
        retention_days_override: None,
        fence_assertion: None,
        mutation: CommandMutation::NoopDaemonBump,
        events: vec![],
        capacity_reservations: vec![],
        budget_reservations: vec![],
    };
    req_other.envelope.authority_generation = authority_1;
    assert!(matches!(
        restored_store.commit_command(req_other),
        Err(StoreError::StaleAuthorityGeneration { .. })
    ));
}
