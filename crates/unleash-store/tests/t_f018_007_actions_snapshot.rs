//! T-F018-007: une action prepared dans le snapshot conserve son effet postérieur.
use std::sync::Arc;
use unleash_domain::{ActionId, ActorId, IdGeneratorPort, RequestId, Revision, Sha256Digest};
use unleash_protocol::{CommandEnvelope, ProtocolVersion, ReceiptState};
use unleash_store::{
    ActionDispatchInput, AggregateKind, BackupRetentionTier, CommandMutation, CommitCommandRequest,
    EmptyExecutorNeutralizer, PrepareBackupRequest, RestoreBackupRequest, RestoreCapsule,
    StoreConfig, StoreError, UnleashStore, WriteCriticality,
};
use unleash_testkit::{DeterministicIdGenerator, ManualClock, local_test_dir};

#[test]
fn prepared_action_executed_after_snapshot_is_in_capsule_and_remains_frozen() {
    run_snapshot_case(false, EffectProgress::ObserveSuccess);
}

#[test]
fn action_with_post_snapshot_parent_is_visible_after_restore() {
    run_snapshot_case(true, EffectProgress::ObserveSuccess);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EffectProgress {
    ObserveSuccess,
    DispatchOnly,
    ReceiptOnly,
    ReceiptTombstone,
}

#[test]
fn dispatch_without_action_revision_is_captured_and_scope_stays_frozen() {
    run_snapshot_case(false, EffectProgress::DispatchOnly);
}

#[test]
fn changed_dispatch_receipt_is_captured_without_action_revision() {
    run_snapshot_case(false, EffectProgress::ReceiptOnly);
}

#[test]
fn post_snapshot_tombstone_stays_historical_without_full_response_reconstruction() {
    run_snapshot_case(true, EffectProgress::ReceiptTombstone);
}

#[test]
fn full_receipt_in_snapshot_tombstoned_afterwards_never_resurrects() {
    run_snapshot_case(false, EffectProgress::ReceiptTombstone);
}

#[expect(
    clippy::unwrap_used,
    clippy::too_many_lines,
    reason = "Scénario de panne partagé : fixtures, snapshot, effet externe et inspection après restore"
)]
fn run_snapshot_case(missing_parent: bool, progress: EffectProgress) {
    let observes_success = matches!(
        progress,
        EffectProgress::ObserveSuccess | EffectProgress::ReceiptTombstone
    );
    let source = local_test_dir("f018-007-prepared-source").unwrap();
    let target = local_test_dir("f018-007-prepared-target").unwrap();
    let ids = Arc::new(DeterministicIdGenerator::new(0xF018_0701));
    let mut config = StoreConfig::new(source.path());
    let clock = Arc::new(ManualClock::default());
    config.clock = clock.clone();
    config.id_generator = ids.clone();
    let store = UnleashStore::open(config.clone()).unwrap();
    store
        .register_principal("p-sys", "system", "user:louis")
        .unwrap();
    let authority = store.active_authority_generation().unwrap();
    let backup_dir = source.path().join("backup");
    if missing_parent {
        store
            .prepare_backup(PrepareBackupRequest {
                destination_dir: backup_dir.clone(),
                coverage: "db_and_objects".to_owned(),
                retention_tier: BackupRetentionTier::Daily,
            })
            .unwrap();
    }
    let action_id = ActionId::from_uuid(ids.new_uuid());
    let created_action = store
        .commit_command(CommitCommandRequest {
            actor_id: ActorId::new("user:louis").unwrap(),
            envelope: CommandEnvelope {
                protocol_version: ProtocolVersion,
                request_id: RequestId::from_uuid(ids.new_uuid()),
                command_id: ids.new_command_id(),
                authority_generation: authority,
                actor_ref: Some("user:louis".to_owned()),
                project_id: None,
                aggregate_id: action_id.as_uuid(),
                expected_revision: Revision::INITIAL,
                command_type: "CreateAction".to_owned(),
                payload: serde_json::Map::new(),
            },
            target_aggregate: AggregateKind::Action,
            operation_id: None,
            receipt_state: ReceiptState::Completed,
            result_ref: None,
            retention_days_override: None,
            fence_assertion: None,
            mutation: CommandMutation::CreateAction {
                action_id,
                origin_kind: "control_plane".to_owned(),
                control_principal_id: Some("p-sys".to_owned()),
                run_id: None,
                step_id: None,
                invocation_id: None,
                attempt_id: None,
                replay_worker_id: None,
                action_type: "external_counter".to_owned(),
                payload_hash: Sha256Digest::from_bytes(b"increment"),
                reconcile_strategy: "observe_then_decide".to_owned(),
                policy_revision: 1,
                resource_lease_id: None,
            },
            events: vec![],
            capacity_reservations: vec![],
            budget_reservations: vec![],
        })
        .unwrap();
    if progress == EffectProgress::ReceiptOnly {
        store
            .record_action_dispatch(ActionDispatchInput {
                dispatch_id: "dispatch-post-snapshot".to_owned(),
                action_id,
                ordinal: 1,
                adapter_id: "counter-fixture".to_owned(),
                request_hash: Sha256Digest::from_bytes(b"increment"),
                provider_request_id: Some("counter-1".to_owned()),
                transport_state: "sent".to_owned(),
                receipt_hash: None,
            })
            .unwrap();
    }
    if !missing_parent {
        store
            .prepare_backup(PrepareBackupRequest {
                destination_dir: backup_dir.clone(),
                coverage: "db_and_objects".to_owned(),
                retention_tier: BackupRetentionTier::Daily,
            })
            .unwrap();
    }
    if missing_parent {
        let project = ids.new_uuid().to_string();
        let mission = ids.new_uuid().to_string();
        let run = ids.new_uuid().to_string();
        store.execute_in_writer_tx(WriteCriticality::Normal, move |tx, _| {
            tx.execute("INSERT INTO projects (project_id, name, weight, state, revision, created_at_ms) VALUES (?1, 'post-snapshot', 1, 'active', 0, 0)", [&project]).map_err(StoreError::from_rusqlite)?;
            tx.execute("INSERT INTO missions (mission_id, project_id, owner_kind, owner_user_id, completion_policy, state, revision) VALUES (?1, ?2, 'user', 'p-sys', 'human', 'active', 0)", rusqlite::params![mission, project]).map_err(StoreError::from_rusqlite)?;
            tx.execute("INSERT INTO workflow_versions (workflow_version_id, schema_version, content_hash, created_at_ms) VALUES ('post-version', 1, 'fixture', 0)", []).map_err(StoreError::from_rusqlite)?;
            tx.execute("INSERT INTO workflow_runs (run_id, mission_id, workflow_version_id, selection_reason, effective_bundle_hash, state, revision, started_at_ms) VALUES (?1, ?2, 'post-version', 'fixture', 'fixture', 'running', 0, 0)", rusqlite::params![run, mission]).map_err(StoreError::from_rusqlite)?;
            tx.execute("UPDATE actions SET run_id = ?1 WHERE action_id = ?2", rusqlite::params![run, action_id.to_string()]).map_err(StoreError::from_rusqlite)?;
            Ok(())
        }).unwrap();
    }
    // Fixture indépendante de SQLite : un effet a lieu après le snapshot.
    let external_counter = std::sync::atomic::AtomicUsize::new(0);
    external_counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    if progress == EffectProgress::ReceiptOnly {
        store.execute_in_writer_tx(WriteCriticality::Normal, move |tx, _| {
            tx.execute("UPDATE action_dispatches SET receipt_hash = ?1, ended_at_ms = 1, transport_state = 'acked' WHERE dispatch_id = 'dispatch-post-snapshot'",
                [Sha256Digest::from_bytes(b"counter=1").as_prefixed_str()]).map_err(StoreError::from_rusqlite)?;
            Ok(())
        }).unwrap();
    } else {
        store
            .record_action_dispatch(ActionDispatchInput {
                dispatch_id: "dispatch-post-snapshot".to_owned(),
                action_id,
                ordinal: 1,
                adapter_id: "counter-fixture".to_owned(),
                request_hash: Sha256Digest::from_bytes(b"increment"),
                provider_request_id: Some("counter-1".to_owned()),
                transport_state: "acked".to_owned(),
                receipt_hash: Some(Sha256Digest::from_bytes(b"counter=1")),
            })
            .unwrap();
    }
    if observes_success {
        store.execute_in_writer_tx(WriteCriticality::Normal, move |tx, _| {
        tx.execute("UPDATE actions SET state = 'observed_success', revision = revision + 1 WHERE action_id = ?1", [action_id.to_string()])
            .map_err(StoreError::from_rusqlite)?;
        Ok(())
    }).unwrap();
    }
    if progress == EffectProgress::ReceiptTombstone {
        clock.advance(std::time::Duration::from_secs(8 * 86_400));
        store.purge_expired_command_receipts().unwrap();
    }
    let source_cutoff: i64 = store
        .read_snapshot(|conn, _| {
            conn.query_row(
                "SELECT response_retained_until_ms FROM command_receipts WHERE receipt_id = ?1",
                [created_action.receipt().receipt_id.to_string()],
                |row| row.get(0),
            )
            .map_err(StoreError::from_rusqlite)
        })
        .unwrap();
    let restore_request = RestoreBackupRequest {
        backup_dir,
        target_data_root: target.path().to_owned(),
        command_id: ids.new_command_id(),
        actor_id: "user:louis".to_owned(),
        command_digest: Sha256Digest::from_bytes(b"restore-prepared"),
        source_authority_generation: authority,
        source_db_unreadable: false,
        source_db_path: Some(config.db_path),
        source_artifacts_dir: Some(config.artifacts_dir),
        neutralizer: Arc::new(EmptyExecutorNeutralizer),
    };
    let result =
        UnleashStore::restore_backup(restore_request.clone(), StoreConfig::new(target.path()))
            .unwrap();
    let retry =
        UnleashStore::restore_backup(restore_request.clone(), StoreConfig::new(target.path()))
            .unwrap_err();
    assert!(matches!(retry, StoreError::StaleAuthorityGeneration { .. }));
    let restored = UnleashStore::open(StoreConfig::new(target.path())).unwrap();
    let capsule: RestoreCapsule = serde_json::from_slice(
        &restored
            .read_verified_object_bytes(result.capsule_object_hash)
            .unwrap(),
    )
    .unwrap();
    let expected_state = if observes_success {
        "observed_success"
    } else {
        "outcome_unknown"
    };
    assert!(
        capsule
            .actions
            .iter()
            .any(|a| a.action_id == action_id && a.state == expected_state)
    );
    let action_delta = capsule
        .actions
        .iter()
        .find(|action| action.action_id == action_id)
        .unwrap();
    assert_eq!(
        action_delta.source_state.as_deref(),
        Some(if observes_success {
            "observed_success"
        } else {
            "prepared"
        })
    );
    assert_eq!(
        action_delta.source_revision,
        if observes_success { 2 } else { 1 }
    );
    let report = restored.inspect_recovery().unwrap();
    assert!(report.items.iter().any(|a| a.action_id == action_id
        && a.state
            == if observes_success {
                unleash_domain::ActionState::ObservedSuccess
            } else {
                unleash_domain::ActionState::OutcomeUnknown
            }));
    let item = report
        .items
        .iter()
        .find(|item| item.action_id == action_id)
        .unwrap();
    assert_eq!(
        item.provenance.as_ref().unwrap().present_in_active_ledger,
        !missing_parent
    );
    assert_eq!(
        item.last_dispatch_id.as_deref(),
        Some("dispatch-post-snapshot")
    );
    let historic_authority: String = restored
        .read_snapshot(|conn, _| {
            conn.query_row(
                "SELECT authority_generation FROM command_receipts WHERE receipt_id = ?1",
                [result.command_receipt_id.to_string()],
                |r| r.get(0),
            )
            .map_err(StoreError::from_rusqlite)
        })
        .unwrap();
    assert_eq!(historic_authority, authority.to_string());
    let historical_receipt = restored
        .inspect_command_receipt(
            &restore_request.actor_id.parse().unwrap(),
            restore_request.command_id,
        )
        .unwrap();
    let historical_receipt = match historical_receipt {
        unleash_store::ReceiptInspectionOutcome::Full(receipt) => Some(receipt),
        unleash_store::ReceiptInspectionOutcome::Tombstone { .. } => None,
    }
    .unwrap();
    assert_eq!(historical_receipt.authority_generation, authority);
    assert_eq!(
        historical_receipt.lineage_status,
        unleash_protocol::LineageStatus::Historical
    );
    assert_eq!(historical_receipt.receipt.state, ReceiptState::Accepted);

    let source_receipt = restored
        .inspect_command_receipt(
            &ActorId::new("user:louis").unwrap(),
            created_action.receipt().command_id,
        )
        .unwrap();
    match source_receipt {
        unleash_store::ReceiptInspectionOutcome::Full(receipt) => {
            assert_ne!(progress, EffectProgress::ReceiptTombstone);
            assert_eq!(receipt.authority_generation, authority);
            assert_eq!(
                receipt.lineage_status,
                unleash_protocol::LineageStatus::Historical
            );
            assert_eq!(&receipt.receipt, created_action.receipt());
        }
        unleash_store::ReceiptInspectionOutcome::Tombstone {
            authority_generation,
            lineage_status,
            receipt_id,
            ..
        } => {
            assert!(progress == EffectProgress::ReceiptTombstone);
            assert_eq!(receipt_id, created_action.receipt().receipt_id);
            assert_eq!(authority_generation, authority);
            assert_eq!(lineage_status, unleash_protocol::LineageStatus::Historical);
        }
    }
    let restored_cutoff: i64 = restored
        .read_snapshot(|conn, _| {
            conn.query_row(
                "SELECT response_retained_until_ms FROM command_receipts WHERE receipt_id = ?1",
                [created_action.receipt().receipt_id.to_string()],
                |row| row.get(0),
            )
            .map_err(StoreError::from_rusqlite)
        })
        .unwrap();
    assert_eq!(restored_cutoff, source_cutoff);
    let source_scope = &capsule
        .actions
        .iter()
        .find(|a| a.action_id == action_id)
        .unwrap()
        .scope_id;
    assert!(
        report
            .frozen_scopes
            .iter()
            .any(|s| s.state == "frozen" && &s.scope_id == source_scope)
    );
    assert_eq!(
        item.last_receipt_hash.as_deref(),
        Some(
            Sha256Digest::from_bytes(b"counter=1")
                .as_prefixed_str()
                .as_str()
        )
    );
    assert_eq!(
        external_counter.load(std::sync::atomic::Ordering::SeqCst),
        1
    );
}
