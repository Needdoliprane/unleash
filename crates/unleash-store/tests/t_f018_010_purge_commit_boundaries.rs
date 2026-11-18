//! T-F018-010: aucun unlink avant commit et reprise durable des purges Q12.
use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
};
use std::time::Duration;
use unleash_domain::{ActorId, IdGeneratorPort, RequestId, Revision, Sha256Digest};
use unleash_protocol::{CommandEnvelope, ProtocolVersion, ReceiptState};
use unleash_store::{
    AggregateKind, CommandMutation, CommitCommandRequest, CutpointObserver, ObjectState,
    PrepareObjectRequest, PublishObjectReferenceRequest, RetentionClass, RetentionPurgeRequest,
    StoreConfig, StoreCutpoint, StoreError, UnleashStore,
};
use unleash_testkit::{DeterministicIdGenerator, ManualClock, local_test_dir};

struct ArmedCutpoint(AtomicU8);
impl CutpointObserver for ArmedCutpoint {
    fn hit(&self, point: StoreCutpoint) -> Result<(), StoreError> {
        let index = match point {
            StoreCutpoint::C1BeforeCommandCommit => 1,
            StoreCutpoint::C2AfterCommandCommit => 2,
            StoreCutpoint::C10cAfterDeletingIntent => 3,
            StoreCutpoint::C10dAfterUnlinkSyncBeforeTombstone => 4,
            _ => 0,
        };
        if index != 0 && self.0.load(Ordering::SeqCst) == index {
            Err(StoreError::CutpointInjected {
                cutpoint: format!("Q12-{index}"),
                message: "injected".to_owned(),
            })
        } else {
            Ok(())
        }
    }
}

#[test]
fn retention_command_crash_boundaries_preserve_durable_deletion() {
    for cutpoint in 1..=4 {
        purge_case(cutpoint);
    }
}

#[tokio::test]
async fn retention_async_command_finishes_deletion_and_checkpoint() {
    // Le chemin async doit utiliser le même post-traitement, sans bloquer le runtime courant.
    let fixture = tokio::task::spawn_blocking(fixture).await.unwrap();
    let request = purge_command(&fixture.0, &fixture.2);
    let result = fixture.0.commit_command_async(request).await;
    let state = fixture.0.object_record(fixture.3).unwrap().state;
    let audited: i64 = fixture
        .0
        .read_snapshot(|conn, _| {
            conn.query_row(
                "SELECT COALESCE(MAX(wal_checkpoint_truncated), 0) FROM retention_purges",
                [],
                |r| r.get(0),
            )
            .map_err(StoreError::from_rusqlite)
        })
        .unwrap();
    tokio::task::spawn_blocking(move || drop(fixture))
        .await
        .unwrap();
    assert_eq!(result.unwrap().receipt().state, ReceiptState::Accepted);
    assert_eq!(state, ObjectState::Tombstone);
    assert_eq!(audited, 1);
}

#[expect(
    clippy::unwrap_used,
    reason = "Helper d’assertion et fixture de panne synthétique"
)]
fn fixture() -> (
    UnleashStore,
    Arc<ArmedCutpoint>,
    Arc<DeterministicIdGenerator>,
    Sha256Digest,
    impl std::fmt::Debug,
    StoreConfig,
) {
    let directory = local_test_dir("f018-010-commit-boundary").unwrap();
    let clock = Arc::new(ManualClock::default());
    let ids = Arc::new(DeterministicIdGenerator::new(0xF018_1010));
    let control = Arc::new(ArmedCutpoint(AtomicU8::new(0)));
    let mut config = StoreConfig::new(directory.path());
    config.clock = clock.clone();
    config.id_generator = ids.clone();
    config.cutpoints = control.clone();
    let store = UnleashStore::open(config.clone()).unwrap();
    let hash = Sha256Digest::from_bytes(b"expired-q12-object");
    let object = store
        .prepare_object(PrepareObjectRequest {
            object_hash: hash,
            byte_length: 18,
            media_type: "text/plain".to_owned(),
            owner_kind: "diagnostic".to_owned(),
            owner_id: "fixture".to_owned(),
        })
        .unwrap();
    store
        .write_prepared_object_bytes(&object, b"expired-q12-object")
        .unwrap();
    store
        .publish_object_reference(PublishObjectReferenceRequest {
            pin_id: object.pin_id,
            object_hash: hash,
            object_generation: object.object_generation,
            owner_kind: "diagnostic".to_owned(),
            owner_id: "fixture".to_owned(),
            source_field: "payload".to_owned(),
        })
        .unwrap();
    store
        .tag_object_retention(hash, RetentionClass::LogsTelemetry, false, None)
        .unwrap();
    clock.advance(Duration::from_secs(40 * 86_400));
    (store, control, ids, hash, directory, config)
}

#[expect(
    clippy::unwrap_used,
    reason = "Helper d’assertion et fixture de panne synthétique"
)]
fn purge_command(store: &UnleashStore, ids: &DeterministicIdGenerator) -> CommitCommandRequest {
    CommitCommandRequest {
        actor_id: ActorId::new("user:louis").unwrap(),
        envelope: CommandEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_uuid(ids.new_uuid()),
            command_id: ids.new_command_id(),
            authority_generation: store.active_authority_generation().unwrap(),
            actor_ref: Some("user:louis".to_owned()),
            project_id: None,
            aggregate_id: ids.new_uuid(),
            expected_revision: Revision::INITIAL,
            command_type: "PurgeHistory".to_owned(),
            payload: serde_json::Map::new(),
        },
        target_aggregate: AggregateKind::RetentionPurge,
        operation_id: None,
        receipt_state: ReceiptState::Completed,
        result_ref: None,
        retention_days_override: None,
        fence_assertion: None,
        mutation: CommandMutation::PurgeHistory {
            request: RetentionPurgeRequest {
                scope_kind: "user".to_owned(),
                scope_id: "default".to_owned(),
                requested_by_actor: "user:louis".to_owned(),
                reason: "expired diagnostic".to_owned(),
                older_than_override_ms: None,
                is_automatic_sweep: false,
            },
        },
        events: vec![],
        capacity_reservations: vec![],
        budget_reservations: vec![],
    }
}

#[expect(
    clippy::unwrap_used,
    reason = "Helper d’assertion et fixture de panne synthétique"
)]
#[expect(
    clippy::too_many_lines,
    reason = "Scénario commun C1/C2/C10c/C10d avec preuves physiques, reçu, événement et réouverture"
)]
fn purge_case(cutpoint: u8) {
    let (store, control, ids, hash, _directory, config) = fixture();
    let path = config
        .artifacts_dir
        .join(store.object_record(hash).unwrap().relative_path);
    let request = purge_command(&store, &ids);
    control.0.store(cutpoint, Ordering::SeqCst);
    assert!(matches!(
        store.commit_command(request.clone()),
        Err(StoreError::CutpointInjected { .. })
    ));
    let expected = if cutpoint == 1 {
        ObjectState::Live
    } else {
        ObjectState::Deleting
    };
    assert_eq!(store.object_record(hash).unwrap().state, expected);
    assert_eq!(
        path.exists(),
        cutpoint <= 3,
        "unlink/commit boundary {cutpoint}"
    );
    if cutpoint == 1 {
        assert_eq!(
            store.read_verified_object_bytes(hash).unwrap(),
            b"expired-q12-object"
        );
    }
    let counts: (i64, i64, i64) = store
        .read_snapshot(|conn, _| {
            Ok((
                conn.query_row("SELECT COUNT(*) FROM object_deletions", [], |r| r.get(0))?,
                conn.query_row("SELECT COUNT(*) FROM retention_purges", [], |r| r.get(0))?,
                conn.query_row("SELECT COUNT(*) FROM command_receipts", [], |r| r.get(0))?,
            ))
        })
        .unwrap();
    assert_eq!(counts, if cutpoint == 1 { (0, 0, 0) } else { (1, 1, 1) });
    if cutpoint != 1 {
        let inspection = store.inspect_recovery().unwrap();
        assert_eq!(
            inspection.retention_purges.first().unwrap().state,
            "pending"
        );
    }
    if cutpoint != 1 {
        let receipt = store
            .inspect_command_receipt(&request.actor_id, request.envelope.command_id)
            .unwrap();
        let receipt = match receipt {
            unleash_store::ReceiptInspectionOutcome::Full(receipt) => Some(receipt),
            unleash_store::ReceiptInspectionOutcome::Tombstone { .. } => None,
        }
        .unwrap();
        assert_eq!(receipt.receipt.state, ReceiptState::Accepted);
        assert!(
            receipt
                .receipt
                .result_ref
                .unwrap()
                .starts_with("retention-purge:")
        );
        let event_identity: (String, String) = store.read_snapshot(|conn, _| conn.query_row(
            "SELECT aggregate_id, causation_receipt_id FROM events WHERE event_type = 'HistoryPurged'", [],
            |row| Ok((row.get(0)?, row.get(1)?))).map_err(StoreError::from_rusqlite)).unwrap();
        assert_eq!(event_identity.0, request.envelope.aggregate_id.to_string());
        assert_eq!(event_identity.1, receipt.receipt.receipt_id.to_string());
        let payload: String = store
            .read_snapshot(|conn, _| {
                conn.query_row(
                    "SELECT payload_json FROM events WHERE event_type = 'HistoryPurged'",
                    [],
                    |r| r.get(0),
                )
                .map_err(StoreError::from_rusqlite)
            })
            .unwrap();
        let payload: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(payload.get("phase").unwrap(), "logical_purge_committed");
        assert_eq!(payload.get("objects_purged_count").unwrap(), 0);
        assert_eq!(payload.get("wal_checkpoint_truncated").unwrap(), false);
    }
    control.0.store(0, Ordering::SeqCst);
    store.shutdown().unwrap();
    let store = UnleashStore::open(config).unwrap();
    // Même command_id après réouverture : reprise, jamais une deuxième purge.
    store.commit_command(request).unwrap();
    assert_eq!(
        store.object_record(hash).unwrap().state,
        ObjectState::Tombstone
    );
    let final_counts: (i64, i64) = store
        .read_snapshot(|conn, _| {
            Ok((
                conn.query_row(
                    "SELECT COUNT(*) FROM retention_purges WHERE wal_checkpoint_truncated = 1",
                    [],
                    |r| r.get(0),
                )?,
                conn.query_row(
                    "SELECT COUNT(*) FROM object_deletions WHERE state = 'completed'",
                    [],
                    |r| r.get(0),
                )?,
            ))
        })
        .unwrap();
    assert_eq!(final_counts, (1, 1));
    assert!(!path.exists());
}

#[test]
fn busy_retention_checkpoint_does_not_claim_truncation() {
    let (store, _control, ids, _hash, _directory, _config) = fixture();
    let reader = store.begin_held_reader().unwrap();
    let request = purge_command(&store, &ids);
    let before = store.wal_telemetry().truncate_checkpoints;
    assert!(matches!(
        store.commit_command(request.clone()),
        Err(StoreError::StorageUnavailable(_))
    ));
    assert_eq!(store.wal_telemetry().truncate_checkpoints, before);
    let audit: i64 = store
        .read_snapshot(|conn, _| {
            conn.query_row(
                "SELECT wal_checkpoint_truncated FROM retention_purges",
                [],
                |r| r.get(0),
            )
            .map_err(StoreError::from_rusqlite)
        })
        .unwrap();
    assert_eq!(audit, 0);
    drop(reader);
    store.commit_command(request).unwrap();
    assert_eq!(store.wal_telemetry().wal_bytes, 0);
}

#[test]
fn quarantined_retention_object_never_becomes_success_on_replay() {
    let (store, _control, ids, hash, _directory, config) = fixture();
    let path = config
        .artifacts_dir
        .join(store.object_record(hash).unwrap().relative_path);
    std::fs::write(&path, b"foreign bytes").unwrap();
    let request = purge_command(&store, &ids);
    assert!(matches!(
        store.commit_command(request.clone()),
        Err(StoreError::StorageCorrupt(_))
    ));
    assert!(matches!(
        store.commit_command(request),
        Err(StoreError::StorageCorrupt(_))
    ));
    assert_eq!(
        store.object_record(hash).unwrap().state,
        ObjectState::Quarantined
    );
    assert!(path.exists());
    store.shutdown().unwrap();
    let reopened = UnleashStore::open(config).unwrap();
    reopened.reconcile_objects_and_deletions().unwrap();
    assert_eq!(
        reopened
            .inspect_recovery()
            .unwrap()
            .retention_purges
            .first()
            .unwrap()
            .state,
        "quarantined"
    );
}

#[test]
fn direct_retention_crash_is_finished_by_existing_recovery_without_client_retry() {
    let (store, control, _ids, hash, _directory, config) = fixture();
    control.0.store(3, Ordering::SeqCst);
    let request = RetentionPurgeRequest {
        scope_kind: "user".to_owned(),
        scope_id: "default".to_owned(),
        requested_by_actor: "user:louis".to_owned(),
        reason: "direct purge recovery".to_owned(),
        older_than_override_ms: None,
        is_automatic_sweep: false,
    };
    assert!(matches!(
        store.execute_retention_purge(request),
        Err(StoreError::CutpointInjected { .. })
    ));
    control.0.store(0, Ordering::SeqCst);
    store.shutdown().unwrap();
    let reopened = UnleashStore::open(config).unwrap();
    reopened.reconcile_objects_and_deletions().unwrap();
    assert_eq!(
        reopened.object_record(hash).unwrap().state,
        ObjectState::Tombstone
    );
    let checkpoint: i64 = reopened
        .read_snapshot(|conn, _| {
            conn.query_row(
                "SELECT wal_checkpoint_truncated FROM retention_purges",
                [],
                |r| r.get(0),
            )
            .map_err(StoreError::from_rusqlite)
        })
        .unwrap();
    assert_eq!(checkpoint, 1);
    assert_eq!(
        reopened
            .inspect_recovery()
            .unwrap()
            .retention_purges
            .first()
            .unwrap()
            .state,
        "completed"
    );
}

#[test]
fn recovery_repeats_final_checkpoint_even_when_prior_audit_is_true() {
    let (store, _control, ids, _hash, _directory, _config) = fixture();
    store.commit_command(purge_command(&store, &ids)).unwrap();
    let before = store.wal_telemetry().truncate_checkpoints;
    store.reconcile_objects_and_deletions().unwrap();
    assert!(store.wal_telemetry().truncate_checkpoints > before);
    assert_eq!(store.wal_telemetry().wal_bytes, 0);
}

#[test]
fn automatic_retention_command_with_uncertain_clock_is_explicitly_suspended() {
    let (store, _control, ids, hash, _directory, config) = fixture();
    config.clock_uncertain.store(true, Ordering::SeqCst);
    let mut request = purge_command(&store, &ids);
    let CommandMutation::PurgeHistory { request: purge } = &mut request.mutation else {
        unreachable!();
    };
    purge.is_automatic_sweep = true;
    assert!(matches!(
        store.commit_command(request.clone()),
        Err(StoreError::RecoveryInProgress(_))
    ));
    assert!(
        store
            .inspect_command_receipt(&request.actor_id, request.envelope.command_id)
            .is_err()
    );
    assert_eq!(store.object_record(hash).unwrap().state, ObjectState::Live);
    config.clock_uncertain.store(false, Ordering::SeqCst);
    assert_eq!(
        store.commit_command(request).unwrap().receipt().state,
        ReceiptState::Accepted
    );
}

struct PostUnlinkSyncFault {
    enabled: std::sync::atomic::AtomicBool,
    hits: std::sync::atomic::AtomicUsize,
}
impl unleash_store::StorageFaultInjector for PostUnlinkSyncFault {
    fn next_fault(
        &self,
        target: unleash_store::FaultTarget,
    ) -> Option<unleash_store::InjectedStorageFault> {
        if target == unleash_store::FaultTarget::ObjectDeletionSync
            && self.enabled.load(Ordering::SeqCst)
        {
            self.hits.fetch_add(1, Ordering::SeqCst);
            Some(unleash_store::InjectedStorageFault::IoErr)
        } else {
            None
        }
    }
}

#[test]
fn unlink_before_directory_sync_is_retried_even_when_path_is_absent() {
    let (store, _control, ids, hash, _directory, mut config) = fixture();
    let path = config
        .artifacts_dir
        .join(store.object_record(hash).unwrap().relative_path);
    store.shutdown().unwrap();
    let injector = Arc::new(PostUnlinkSyncFault {
        enabled: std::sync::atomic::AtomicBool::new(true),
        hits: std::sync::atomic::AtomicUsize::new(0),
    });
    config.fault_injector = injector.clone();
    let store = UnleashStore::open(config).unwrap();
    let request = purge_command(&store, &ids);
    assert!(matches!(
        store.commit_command(request.clone()),
        Err(StoreError::StorageUnavailable(_))
    ));
    assert!(!path.exists());
    assert_eq!(
        store.object_record(hash).unwrap().state,
        ObjectState::Deleting
    );
    assert!(matches!(
        store.commit_command(request.clone()),
        Err(StoreError::StorageUnavailable(_))
    ));
    assert_eq!(injector.hits.load(Ordering::SeqCst), 2);
    assert_eq!(
        store.object_record(hash).unwrap().state,
        ObjectState::Deleting
    );
    injector.enabled.store(false, Ordering::SeqCst);
    store.commit_command(request).unwrap();
    assert_eq!(
        store.object_record(hash).unwrap().state,
        ObjectState::Tombstone
    );
}

#[test]
fn purge_creation_requires_initial_revision_and_global_aggregate_uniqueness() {
    let (store, _control, ids, _hash, _directory, _config) = fixture();
    let request = purge_command(&store, &ids);
    let mut bad_revision = request.clone();
    bad_revision.envelope.expected_revision = Revision::new(1);
    assert!(matches!(
        store.commit_command(bad_revision),
        Err(StoreError::RevisionConflict { .. })
    ));
    let first = store.commit_command(request.clone()).unwrap();
    assert_eq!(
        first.receipt().result_ref.as_deref(),
        Some(format!("retention-purge:{}", request.envelope.aggregate_id).as_str())
    );
    let mut duplicate = request;
    duplicate.envelope.command_id = ids.new_command_id();
    duplicate.envelope.request_id = RequestId::from_uuid(ids.new_uuid());
    duplicate.actor_id = ActorId::new("user:other").unwrap();
    duplicate.envelope.actor_ref = Some("user:other".to_owned());
    let CommandMutation::PurgeHistory { request: purge } = &mut duplicate.mutation else {
        unreachable!();
    };
    purge.requested_by_actor = "user:other".to_owned();
    assert!(matches!(
        store.commit_command(duplicate),
        Err(StoreError::RevisionConflict { .. })
    ));
    let counts: (i64, i64, i64) = store
        .read_snapshot(|conn, _| {
            Ok((
                conn.query_row("SELECT COUNT(*) FROM retention_purges", [], |row| {
                    row.get(0)
                })?,
                conn.query_row("SELECT COUNT(*) FROM command_receipts", [], |row| {
                    row.get(0)
                })?,
                conn.query_row(
                    "SELECT COUNT(*) FROM events WHERE event_type = 'HistoryPurged'",
                    [],
                    |row| row.get(0),
                )?,
            ))
        })
        .unwrap();
    assert_eq!(counts, (1, 1, 1));
}
