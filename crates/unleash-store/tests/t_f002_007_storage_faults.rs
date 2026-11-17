// T-F002-007 / AC-F002-007 / REQ-PERSIST-007
//! Injection `SQLITE_BUSY`, espace bas, `SQLITE_FULL`, `SQLITE_IOERR` et `SQLITE_CORRUPT`.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Map;
use unleash_domain::{ActorId, CommandId, RequestId, Revision, Sha256Digest, TimestampMs};
use unleash_protocol::{
    CommandEnvelope, ErrorCode, ProtocolVersion, ReasonCode, ReceiptState, RetryClass,
};
use unleash_store::{
    AggregateKind, CommandMutation, CommitCommandRequest, FaultTarget, FixedDiskSpaceProbe,
    InjectedStorageFault, MonotonicClockPort, ObjectState, PrepareObjectDeletionRequest,
    PrepareObjectRequest, PublishObjectReferenceRequest, StorageFaultInjector, StoreConfig,
    StoreError, UnleashStore, WriteCriticality,
};
use unleash_testkit::{DeterministicIdGenerator, ManualClock, local_test_dir};

struct ClockAdapter(Arc<ManualClock>);
impl MonotonicClockPort for ClockAdapter {
    fn monotonic_now(&self) -> Duration {
        self.0.monotonic_now()
    }
}

struct QueueFaultInjector {
    faults: Mutex<VecDeque<(FaultTarget, InjectedStorageFault)>>,
}

impl QueueFaultInjector {
    fn new() -> Self {
        Self {
            faults: Mutex::new(VecDeque::new()),
        }
    }

    fn push(&self, target: FaultTarget, fault: InjectedStorageFault) {
        self.faults
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push_back((target, fault));
    }
}

impl StorageFaultInjector for QueueFaultInjector {
    fn next_fault(&self, target: FaultTarget) -> Option<InjectedStorageFault> {
        let mut guard = self
            .faults
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if guard.front().is_some_and(|(t, _)| *t == target) {
            guard.pop_front().map(|(_, f)| f)
        } else {
            None
        }
    }
}

fn noop_cmd(
    authority: unleash_domain::AuthorityGeneration,
    cmd_id: CommandId,
    expected_rev: Revision,
) -> Result<CommitCommandRequest, StoreError> {
    Ok(CommitCommandRequest {
        actor_id: ActorId::new("user:louis")?,
        envelope: CommandEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_u128(1),
            command_id: cmd_id,
            authority_generation: authority,
            actor_ref: Some("user:louis".to_owned()),
            project_id: None,
            aggregate_id: authority.as_uuid(),
            expected_revision: expected_rev,
            command_type: "NoopCommand".to_owned(),
            payload: Map::new(),
        },
        target_aggregate: AggregateKind::Daemon,
        operation_id: None,
        receipt_state: ReceiptState::Completed,
        result_ref: None,
        retention_days_override: None,
        fence_assertion: None,
        mutation: CommandMutation::NoopDaemonBump,
        events: vec![],
        capacity_reservations: vec![],
        budget_reservations: vec![],
    })
}

#[test]
fn test_t_f002_007_busy_low_space_full_ioerr_and_corrupt() {
    let dir = local_test_dir("f002-007").unwrap();
    let clock = Arc::new(ManualClock::default());
    let ids = Arc::new(DeterministicIdGenerator::new(0xF002_0007));
    let disk_probe = Arc::new(FixedDiskSpaceProbe::new(10 * 1024 * 1024 * 1024));
    let injector = Arc::new(QueueFaultInjector::new());

    let mut cfg = StoreConfig::new(dir.path());
    cfg.busy_timeout_ms = 20;
    cfg.clock = clock.clone();
    cfg.monotonic_clock = Arc::new(ClockAdapter(Arc::clone(&clock)));
    cfg.id_generator = ids.clone();
    cfg.disk_space_probe = disk_probe.clone();
    cfg.fault_injector = injector.clone();

    let store = UnleashStore::open(cfg.clone()).unwrap();
    let authority = store.active_authority_generation().unwrap();

    // 1. Vraie contention SQLITE_BUSY via verrou EXCLUSIVE externe
    let ext_conn = rusqlite::Connection::open(&cfg.db_path).unwrap();
    ext_conn.execute_batch("BEGIN EXCLUSIVE;").unwrap();
    let cmd_busy = CommandId::from_u128(701);
    let err_busy = store
        .commit_command(noop_cmd(authority, cmd_busy, Revision::INITIAL).unwrap())
        .unwrap_err();
    assert!(matches!(err_busy, StoreError::StorageBusy(_)));
    assert_eq!(err_busy.error_code(), ErrorCode::ResourceBusy);
    assert_eq!(err_busy.reason_code(), Some(ReasonCode::StorageBusy));
    ext_conn.execute_batch("ROLLBACK;").unwrap();
    drop(ext_conn);

    // Vérifier qu'aucun faux receipt n'a été écrit lors de SQLITE_BUSY
    assert!(
        store
            .inspect_command_receipt(&ActorId::new("user:louis").unwrap(), cmd_busy)
            .is_err()
    );

    // 2. Espace disque < 1 GiB -> ResourceBusy/STORAGE_LOW_SPACE (after_human_decision)
    disk_probe.set_bytes(512 * 1024 * 1024);
    let cmd_low = CommandId::from_u128(702);
    let err_low = store
        .commit_command(noop_cmd(authority, cmd_low, Revision::INITIAL).unwrap())
        .unwrap_err();
    assert!(matches!(err_low, StoreError::StorageLowSpace { .. }));
    assert_eq!(err_low.error_code(), ErrorCode::ResourceBusy);
    assert_eq!(err_low.reason_code(), Some(ReasonCode::StorageLowSpace));
    assert_eq!(err_low.retry_class(), RetryClass::AfterHumanDecision);
    disk_probe.set_bytes(10 * 1024 * 1024 * 1024);

    // 3. Vrai SQLITE_FULL via PRAGMA max_page_count sur base isolée + injection SQLITE_FULL
    let dir_full = local_test_dir("f002-007-full").unwrap();
    let full_store = UnleashStore::open(StoreConfig::new(dir_full.path())).unwrap();
    let err_real_full = full_store
        .execute_in_writer_tx(WriteCriticality::Normal, |tx, _| {
            let page_count: i64 = tx.pragma_query_value(None, "page_count", |r| r.get(0))?;
            tx.pragma_update(None, "max_page_count", page_count)?;
            tx.execute("CREATE TABLE _fill(b TEXT)", [])?;
            tx.execute("INSERT INTO _fill(b) VALUES (?1)", ["Z".repeat(200_000)])?;
            Ok(())
        })
        .unwrap_err();
    assert!(matches!(err_real_full, StoreError::StorageFull(_)));
    assert_eq!(err_real_full.error_code(), ErrorCode::InternalFailure);
    assert_eq!(err_real_full.reason_code(), Some(ReasonCode::StorageFull));
    assert_eq!(err_real_full.retry_class(), RetryClass::AfterHumanDecision);
    full_store.shutdown().unwrap();

    // 4. SQLITE_IOERR injecté -> InternalFailure/STORAGE_UNAVAILABLE, aucun faux receipt
    injector.push(FaultTarget::CommandWrite, InjectedStorageFault::IoErr);
    let cmd_io = CommandId::from_u128(703);
    let err_io = store
        .commit_command(noop_cmd(authority, cmd_io, Revision::INITIAL).unwrap())
        .unwrap_err();
    assert!(matches!(err_io, StoreError::StorageUnavailable(_)));
    assert_eq!(err_io.error_code(), ErrorCode::InternalFailure);
    assert_eq!(err_io.reason_code(), Some(ReasonCode::StorageUnavailable));
    assert!(
        store
            .inspect_command_receipt(&ActorId::new("user:louis").unwrap(), cmd_io)
            .is_err()
    );
    store.shutdown().unwrap();

    // 5. Vraie corruption SQLITE_CORRUPT / SQLITE_NOTADB sur copie isolée ; WAL conservé
    let dir_corrupt = local_test_dir("f002-007-corrupt").unwrap();
    let s_corrupt = UnleashStore::open(StoreConfig::new(dir_corrupt.path())).unwrap();
    let auth_c = s_corrupt.active_authority_generation().unwrap();
    s_corrupt
        .commit_command(noop_cmd(auth_c, CommandId::from_u128(704), Revision::INITIAL).unwrap())
        .unwrap();
    s_corrupt.shutdown().unwrap();

    let db_file = dir_corrupt.path().join("unleash-authority.db");
    let mut bytes = std::fs::read(&db_file).unwrap();
    for b in bytes.iter_mut().take(32) {
        *b = 0xFF;
    }
    std::fs::write(&db_file, bytes).unwrap();
    let err_corrupt = UnleashStore::open(StoreConfig::new(dir_corrupt.path())).unwrap_err();
    assert!(matches!(err_corrupt, StoreError::StorageCorrupt(_)));
    assert_eq!(err_corrupt.error_code(), ErrorCode::InternalFailure);
    assert_eq!(err_corrupt.reason_code(), Some(ReasonCode::StorageCorrupt));
    assert_eq!(err_corrupt.retry_class(), RetryClass::Never);
    assert!(db_file.exists(), "les fichiers suspects sont conservés");
}

#[test]
fn test_t_f002_007_maintenance_and_object_unlink_faults() {
    let dir = local_test_dir("f002-007-maint-unlink").unwrap();
    let clock = Arc::new(ManualClock::default());
    let ids = Arc::new(DeterministicIdGenerator::new(0xF002_0702));
    let injector = Arc::new(QueueFaultInjector::new());

    let mut cfg = StoreConfig::new(dir.path());
    cfg.clock = clock.clone();
    cfg.monotonic_clock = Arc::new(ClockAdapter(clock));
    cfg.id_generator = ids;
    cfg.fault_injector = injector.clone();

    let store = UnleashStore::open(cfg.clone()).unwrap();

    // FaultTarget::Maintenance sur tick WAL et PurgeHistory
    injector.push(FaultTarget::Maintenance, InjectedStorageFault::Busy);
    let err_maint = store.run_wal_maintenance_tick().unwrap_err();
    assert!(matches!(err_maint, StoreError::StorageBusy(_)));
    injector.push(FaultTarget::Maintenance, InjectedStorageFault::IoErr);
    let err_purge = store.purge_history(TimestampMs::new(1000)).unwrap_err();
    assert!(matches!(err_purge, StoreError::StorageUnavailable(_)));

    // FaultTarget::ObjectUnlink avant unlink -> deleting/prepared + fichier intacts jusqu'à réconciliation
    let bytes_u = b"unlink-fault-blob".to_vec();
    let hash_u = Sha256Digest::from_bytes(&bytes_u);
    let hu = store
        .prepare_object(PrepareObjectRequest {
            object_hash: hash_u,
            byte_length: u64::try_from(bytes_u.len()).unwrap(),
            media_type: "application/octet-stream".to_owned(),
            owner_kind: "evidence".to_owned(),
            owner_id: "ev-fault-unlink".to_owned(),
        })
        .unwrap();
    store.write_prepared_object_bytes(&hu, &bytes_u).unwrap();
    store
        .publish_object_reference(PublishObjectReferenceRequest {
            pin_id: hu.pin_id,
            object_hash: hash_u,
            object_generation: hu.object_generation,
            owner_kind: "evidence".to_owned(),
            owner_id: "ev-fault-unlink".to_owned(),
            source_field: "object_hash".to_owned(),
        })
        .unwrap();
    store
        .tombstone_object_references(hash_u, "cleanup")
        .unwrap();
    let del_u = store
        .prepare_object_deletion(PrepareObjectDeletionRequest {
            object_hash: hash_u,
            expected_generation: 1,
            reason: "fault_test".to_owned(),
            decision_receipt_id: None,
        })
        .unwrap();
    injector.push(FaultTarget::ObjectUnlink, InjectedStorageFault::IoErr);
    let err_unlink = store
        .execute_and_finalize_object_deletion(&del_u)
        .unwrap_err();
    assert!(matches!(err_unlink, StoreError::StorageUnavailable(_)));
    assert_eq!(
        store.object_record(hash_u).unwrap().state,
        ObjectState::Deleting
    );
    assert!(cfg.artifacts_dir.join(&del_u.relative_path).exists());

    // Après levée de la panne, réconciliation termine unlink + tombstone
    let rec_u = store.reconcile_objects_and_deletions().unwrap();
    assert_eq!(rec_u.deletions_finalized, 1);
    assert_eq!(
        store.object_record(hash_u).unwrap().state,
        ObjectState::Tombstone
    );
    assert!(!cfg.artifacts_dir.join(&del_u.relative_path).exists());
    store.shutdown().unwrap();
}
