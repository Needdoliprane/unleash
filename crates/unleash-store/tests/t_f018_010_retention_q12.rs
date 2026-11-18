// T-F018-010 / AC-F018-010 / REQ-REC-010
//! Rétention Q-12 (`unleash.retention.default.v1`), préavis >= 24 h lors de raccourcissement,
//! préservation stricte des objets/preuves/runs actifs ou `outcome_unknown`,
//! audit `retention_purges` et `PRAGMA wal_checkpoint(TRUNCATE)`.

use std::sync::Arc;
use std::time::Duration;

use serde_json::Map;
use unleash_domain::{
    ActionId, ActorId, CompletionPolicy, IdGeneratorPort, InvocationId, MissionId, MissionState,
    ProjectId, RequestId, Revision, RunId, Sha256Digest,
};
use unleash_protocol::{
    CommandEnvelope, ErrorCode, EventCausationKind, ProtocolVersion, ReceiptState, Sensitivity,
};
use unleash_store::{
    AggregateKind, BackupRetentionTier, CallerAuthority, CallerRole, CommandMutation,
    CommitCommandRequest, EventAppendSpec, PrepareBackupRequest, PrepareObjectRequest,
    PublishObjectReferenceRequest, RetentionClass, RetentionPurgeRequest, SettingsPatch,
    SettingsScopeKind, StoreConfig, UnleashStore, WriteCriticality,
};
use unleash_testkit::{DeterministicIdGenerator, ManualClock, local_test_dir};
use uuid::Uuid;

fn make_env(
    ids: &DeterministicIdGenerator,
    auth: unleash_domain::AuthorityGeneration,
    proj: Option<ProjectId>,
    agg: Uuid,
    rev: Revision,
    cmd_type: &str,
) -> CommandEnvelope {
    CommandEnvelope {
        protocol_version: ProtocolVersion,
        request_id: RequestId::from_uuid(ids.new_uuid()),
        command_id: ids.new_command_id(),
        authority_generation: auth,
        actor_ref: Some("user:louis".to_owned()),
        project_id: proj,
        aggregate_id: agg,
        expected_revision: rev,
        command_type: cmd_type.to_owned(),
        payload: Map::new(),
    }
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "Couvre Q-12 complet : défauts, raccourcissement 24h, protections run/action/preuve/version/pin, backup impact, WAL TRUNCATE et clock_uncertain"
)]
fn test_t_f018_010_retention_q12_delayed_shortening_protections_and_wal_truncate() {
    let dir = local_test_dir("f018-010").unwrap();
    let clock = Arc::new(ManualClock::default());
    let ids = Arc::new(DeterministicIdGenerator::new(0xF018_0010));
    let mut cfg = StoreConfig::new(dir.path());
    cfg.clock = clock.clone();
    cfg.id_generator = ids.clone();

    let store = UnleashStore::open(cfg.clone()).unwrap();
    store
        .register_principal("p-louis", "user", "user:louis")
        .unwrap();
    let auth = store.active_authority_generation().unwrap();
    let admin = ActorId::new("user:louis").unwrap();
    let human = CallerAuthority {
        actor_id: admin.clone(),
        role: CallerRole::HumanOperator,
    };

    // 1. Défauts Q-12 `unleash.retention.default.v1`
    let settings = store
        .read_settings(SettingsScopeKind::User, "default")
        .unwrap();
    assert_eq!(
        settings.retention_policy.policy_version,
        "unleash.retention.default.v1"
    );
    assert_eq!(settings.retention_policy.command_receipts_min_days, 7);
    assert_eq!(settings.retention_policy.events_min_days, 7);
    assert_eq!(settings.retention_policy.model_payloads_days, 90);
    assert_eq!(settings.retention_policy.tool_terminal_logs_days, 30);
    assert_eq!(settings.retention_policy.browser_artifacts_days, 30);
    assert_eq!(settings.retention_policy.dirty_captures_days, 30);
    assert_eq!(settings.retention_policy.automation_data_days, 90);
    assert_eq!(settings.retention_policy.document_copies_days, 30);
    assert_eq!(settings.retention_policy.export_artifacts_days, 7);
    assert_eq!(settings.retention_policy.logs_telemetry_days, 30);
    assert_eq!(settings.retention_policy.local_backups_daily, 7);
    assert_eq!(settings.retention_policy.local_backups_weekly, 4);
    assert_eq!(settings.retention_policy.max_backup_rotation_days, 35);

    // 2. Fenêtre < 7 j pour reçus/événements toujours refusée ; raccourcissement sans confirmation refusé
    let settings_agg = Uuid::from_u128(0x5E77);
    let mut invalid_ret = settings.retention_policy.clone();
    invalid_ret.command_receipts_min_days = 3;
    let below_min_err = store
        .commit_command(CommitCommandRequest {
            actor_id: admin.clone(),
            envelope: make_env(
                &ids,
                auth,
                None,
                settings_agg,
                Revision::INITIAL,
                "SetSettings",
            ),
            target_aggregate: AggregateKind::Settings,
            operation_id: None,
            receipt_state: ReceiptState::Completed,
            result_ref: None,
            retention_days_override: None,
            fence_assertion: None,
            mutation: CommandMutation::SetSettings {
                scope_kind: SettingsScopeKind::User,
                scope_id: "default".to_owned(),
                caller: human.clone(),
                patch: SettingsPatch {
                    execution_pool: None,
                    retention_policy: Some(invalid_ret),
                    confirm_retention_shortening: true,
                    analyzer_paths: None,
                    extra_typed: None,
                },
            },
            events: vec![],
            capacity_reservations: vec![],
            budget_reservations: vec![],
        })
        .unwrap_err();
    assert_eq!(below_min_err.error_code(), ErrorCode::InvalidArgument);

    let mut shortened = settings.retention_policy.clone();
    shortened.tool_terminal_logs_days = 10;
    let unconfirmed_err = store
        .commit_command(CommitCommandRequest {
            actor_id: admin.clone(),
            envelope: make_env(
                &ids,
                auth,
                None,
                settings_agg,
                Revision::INITIAL,
                "SetSettings",
            ),
            target_aggregate: AggregateKind::Settings,
            operation_id: None,
            receipt_state: ReceiptState::Completed,
            result_ref: None,
            retention_days_override: None,
            fence_assertion: None,
            mutation: CommandMutation::SetSettings {
                scope_kind: SettingsScopeKind::User,
                scope_id: "default".to_owned(),
                caller: human.clone(),
                patch: SettingsPatch {
                    execution_pool: None,
                    retention_policy: Some(shortened.clone()),
                    confirm_retention_shortening: false,
                    analyzer_paths: None,
                    extra_typed: None,
                },
            },
            events: vec![],
            capacity_reservations: vec![],
            budget_reservations: vec![],
        })
        .unwrap_err();
    assert_eq!(unconfirmed_err.error_code(), ErrorCode::InvalidArgument);

    // 3. Créer projet + run bloqué + run non terminal + mission non terminale + action outcome_unknown
    //    + preuve + version publiée + pin + backup géré
    let project_id = ProjectId::from_uuid(ids.new_uuid());
    let proj_cmd = ids.new_command_id();
    store
        .commit_command(CommitCommandRequest {
            actor_id: admin.clone(),
            envelope: make_env(
                &ids,
                auth,
                Some(project_id),
                project_id.as_uuid(),
                Revision::INITIAL,
                "CreateProject",
            ),
            target_aggregate: AggregateKind::Project,
            operation_id: None,
            receipt_state: ReceiptState::Completed,
            result_ref: None,
            retention_days_override: None,
            fence_assertion: None,
            mutation: CommandMutation::CreateProject {
                project_id,
                name: "proj-q12".to_owned(),
                weight: 1,
            },
            events: vec![EventAppendSpec {
                event_type: "ProjectCreated".to_owned(),
                aggregate_type: AggregateKind::Project,
                aggregate_id: project_id.as_uuid(),
                aggregate_revision: Revision::new(1),
                project_id: Some(project_id),
                correlation_id: ids.new_uuid(),
                causation_kind: EventCausationKind::Command,
                causation_id: None,
                payload: Map::new(),
                sensitivity: Sensitivity::Project,
                outbox_destinations: vec!["audit".to_owned()],
            }],
            capacity_reservations: vec![],
            budget_reservations: vec![],
        })
        .unwrap();

    let promised_until_before: i64 = store
        .read_snapshot(|conn, _| {
            Ok(conn.query_row(
                "SELECT response_retained_until_ms FROM command_receipts WHERE aggregate_id = ?1 LIMIT 1",
                [project_id.to_string()],
                |r| r.get(0),
            )?)
        })
        .unwrap();
    let _ = proj_cmd;

    let mission_id = MissionId::from_uuid(ids.new_uuid());
    store
        .commit_command(CommitCommandRequest {
            actor_id: admin.clone(),
            envelope: make_env(
                &ids,
                auth,
                Some(project_id),
                mission_id.as_uuid(),
                Revision::INITIAL,
                "CreateMission",
            ),
            target_aggregate: AggregateKind::Mission,
            operation_id: None,
            receipt_state: ReceiptState::Completed,
            result_ref: None,
            retention_days_override: None,
            fence_assertion: None,
            mutation: CommandMutation::CreateMission {
                mission_id,
                goal_id: None,
                project_id,
                owner_principal_id: Some("p-louis".to_owned()),
                owner_agent_id: None,
                completion_policy: CompletionPolicy::Mechanical,
                state: MissionState::Active,
            },
            events: vec![],
            capacity_reservations: vec![],
            budget_reservations: vec![],
        })
        .unwrap();

    let run_running = RunId::from_uuid(ids.new_uuid());
    let run_blocked = RunId::from_uuid(ids.new_uuid());
    let act_unknown = ActionId::from_uuid(ids.new_uuid());
    let inv_active = InvocationId::from_uuid(ids.new_uuid());

    let create_obj = |tag: &[u8], class: RetentionClass, owner_kind: &str, owner_id: &str| {
        let h = Sha256Digest::from_bytes(tag);
        let prep = store
            .prepare_object(PrepareObjectRequest {
                object_hash: h,
                byte_length: tag.len() as u64,
                media_type: "application/octet-stream".to_owned(),
                owner_kind: owner_kind.to_owned(),
                owner_id: owner_id.to_owned(),
            })
            .unwrap();
        store.write_prepared_object_bytes(&prep, tag).unwrap();
        store
            .publish_object_reference(PublishObjectReferenceRequest {
                pin_id: prep.pin_id,
                object_hash: h,
                object_generation: prep.object_generation,
                owner_kind: owner_kind.to_owned(),
                owner_id: owner_id.to_owned(),
                source_field: "payload".to_owned(),
            })
            .unwrap();
        store
            .tag_object_retention(h, class, false, Some(project_id))
            .unwrap();
        h
    };

    // Objets âgés à créer à t = 0 :
    // - `h_expired_ok`: sera purgé à t = 40j
    // - `h_short_15d`: aura 15j lors du test du préavis 24h (créé à t = 25j)
    // - 7 objets protégés : run actif, run bloqué, action outcome_unknown, mission active, preuve active, version publiée, keep_by_policy/pin
    let h_expired_ok = create_obj(
        b"old-unreferenced-log",
        RetentionClass::ToolTerminalLogs,
        "tool_log",
        "old-1",
    );
    let h_prot_run = create_obj(
        b"run-running-log",
        RetentionClass::ToolTerminalLogs,
        "workflow_run",
        &run_running.to_string(),
    );
    let h_prot_blocked = create_obj(
        b"run-blocked-log",
        RetentionClass::ToolTerminalLogs,
        "workflow_run",
        &run_blocked.to_string(),
    );
    let h_prot_unknown = create_obj(
        b"action-unknown-log",
        RetentionClass::ToolTerminalLogs,
        "action",
        &act_unknown.to_string(),
    );
    let h_prot_mission = create_obj(
        b"mission-active-log",
        RetentionClass::ToolTerminalLogs,
        "mission",
        &mission_id.to_string(),
    );
    let h_prot_inv = create_obj(
        b"inv-active-log",
        RetentionClass::ToolTerminalLogs,
        "invocation",
        &inv_active.to_string(),
    );
    let h_prot_ev = create_obj(
        b"evidence-blob",
        RetentionClass::ToolTerminalLogs,
        "evidence",
        "ev-q12",
    );
    let h_prot_pub = create_obj(
        b"published-wf-blob",
        RetentionClass::ToolTerminalLogs,
        "workflow_version",
        "wf-pub-1",
    );
    let h_prot_keep = create_obj(
        b"pinned-keep-blob",
        RetentionClass::ToolTerminalLogs,
        "pin",
        "pin-1",
    );
    store
        .tag_object_retention(
            h_prot_keep,
            RetentionClass::ToolTerminalLogs,
            true,
            Some(project_id),
        )
        .unwrap();

    // Insérer entités SQLite correspondantes aux protections + backup géré
    store
        .execute_in_writer_tx(WriteCriticality::Critical, {
            let wf_hash = h_prot_pub.as_prefixed_str();
            let ev_hash = h_prot_ev.as_prefixed_str();
            let agent_id = ids.new_agent_id();
            let sess_id = ids.new_session_id();
            let man_id = ids.new_uuid();
            let b_hash = wf_hash.clone();
            let ev_id = ids.new_uuid();
            move |tx, now| {
                tx.execute("UPDATE outbox SET state = 'acked', acked_at_ms = ?1", [now.as_i64()])?;
                tx.execute(
                    "INSERT INTO workflow_versions(workflow_version_id, schema_version, visibility, content_hash, created_at_ms)
                     VALUES ('wf-pub-1', 1, 'published', ?1, ?2)",
                    rusqlite::params![wf_hash, now.as_i64()],
                )?;
                tx.execute(
                    "INSERT INTO workflow_runs(run_id, mission_id, workflow_version_id, purpose, selection_reason, effective_bundle_hash, state, revision, started_at_ms)
                     VALUES (?1, ?2, 'wf-pub-1', 'operational', 'ok', ?3, 'running', 1, ?4)",
                    rusqlite::params![run_running.to_string(), mission_id.to_string(), b_hash, now.as_i64()],
                )?;
                tx.execute(
                    "INSERT INTO workflow_runs(run_id, mission_id, workflow_version_id, purpose, selection_reason, effective_bundle_hash, state, revision, started_at_ms)
                     VALUES (?1, ?2, 'wf-pub-1', 'operational', 'ok', ?3, 'blocked', 1, ?4)",
                    rusqlite::params![run_blocked.to_string(), mission_id.to_string(), b_hash, now.as_i64()],
                )?;
                tx.execute(
                    "INSERT INTO actions(action_id, origin_kind, control_principal_id, run_id, action_type, payload_hash, policy_revision, authority_generation, daemon_epoch, fence, state, reconcile_strategy, revision, created_at_ms)
                     VALUES (?1, 'control_plane', 'p-louis', ?2, 'http_call', ?3, 1, ?4, 1, 0, 'outcome_unknown', 'observe_then_decide', 1, ?5)",
                    rusqlite::params![act_unknown.to_string(), run_blocked.to_string(), b_hash, auth.to_string(), now.as_i64()],
                )?;
                tx.execute(
                    "INSERT INTO evidence(evidence_id, kind, run_id, object_hash, producer_id, observed_at_ms, schema_version, availability)
                     VALUES ('ev-q12', 'observation', ?1, ?2, 'harness', ?3, 1, 'available')",
                    rusqlite::params![run_running.to_string(), ev_hash, now.as_i64()],
                )?;
                tx.execute(
                    "INSERT INTO agents(agent_id, level, project_id, state, revision) VALUES (?1, 'N', ?2, 'active', 1)",
                    rusqlite::params![agent_id.to_string(), project_id.to_string()],
                )?;
                tx.execute(
                    "INSERT INTO sessions(session_id, agent_id, revision, state, created_at_ms) VALUES (?1, ?2, 0, 'open', ?3)",
                    rusqlite::params![sess_id.to_string(), agent_id.to_string(), now.as_i64()],
                )?;
                tx.execute(
                    "INSERT INTO context_manifests(manifest_id, session_id, base_session_revision, content_hash, schema_version, created_at_ms) VALUES (?1, ?2, 0, ?3, 1, ?4)",
                    rusqlite::params![man_id.to_string(), sess_id.to_string(), b_hash, now.as_i64()],
                )?;
                tx.execute(
                    "INSERT INTO effective_bundles(bundle_hash, schema_version, role_version_id, requested_model_profile_version_id, allowed_model_policy_hash, initial_context_manifest_id, scope_id, scope_revision, content_object_hash) VALUES (?1, 1, 'rv1', 'mp1', ?1, ?2, 's', 0, ?1)",
                    rusqlite::params![b_hash, man_id.to_string()],
                )?;
                tx.execute(
                    "INSERT INTO events(event_id, stream_id, aggregate_type, aggregate_id, aggregate_revision, event_ordinal, event_type, schema_version, actor_ref, actor_id, occurred_at_ms, recorded_at_ms, payload_json, payload_state, payload_hash, sensitivity, causation_kind, causation_id, causation_receipt_id, correlation_id)
                     VALUES (?1, ?2, 'invocation', ?3, 1, 0, 'InvocationQueued', 1, 'system', 'system', ?4, ?4, '{}', 'full', ?5, 'project', 'event', ?1, NULL, ?1)",
                    rusqlite::params![ev_id.to_string(), Uuid::from_u128(1).to_string(), inv_active.to_string(), now.as_i64(), b_hash],
                )?;
                tx.execute(
                    "INSERT INTO invocations(invocation_id, agent_id, session_id, trigger_event_id, run_id, effective_bundle_hash, context_manifest_id, state, admission_class, revision, created_at_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'running', 'work', 1, ?8)",
                    rusqlite::params![inv_active.to_string(), agent_id.to_string(), sess_id.to_string(), ev_id.to_string(), run_running.to_string(), b_hash, man_id.to_string(), now.as_i64()],
                )?;
                Ok(())
            }
        })
        .unwrap();

    // Créer une sauvegarde gérée contenant ces octets avant purge
    let bkp_dir = dir.path().join("backups/bkp-q12");
    store
        .prepare_backup(PrepareBackupRequest {
            destination_dir: bkp_dir,
            retention_tier: BackupRetentionTier::Daily,
            coverage: "db_and_objects".to_owned(),
        })
        .unwrap();

    // Avancer de 25 jours, créer `h_short_15d` qui aura 15 jours à t=40j
    clock.advance(Duration::from_secs(25 * 86_400));
    let h_short_15d = create_obj(
        b"mid-age-log-15d",
        RetentionClass::ToolTerminalLogs,
        "tool_log",
        "mid-1",
    );

    // Avancer à t = 40 jours et planifier le raccourcissement confirmed (`tool_terminal_logs_days` 30 -> 10)
    clock.advance(Duration::from_secs(15 * 86_400));
    let set_short_time = clock.utc_ms();
    store
        .commit_command(CommitCommandRequest {
            actor_id: admin.clone(),
            envelope: make_env(
                &ids,
                auth,
                None,
                settings_agg,
                Revision::INITIAL,
                "SetSettings",
            ),
            target_aggregate: AggregateKind::Settings,
            operation_id: None,
            receipt_state: ReceiptState::Completed,
            result_ref: None,
            retention_days_override: None,
            fence_assertion: None,
            mutation: CommandMutation::SetSettings {
                scope_kind: SettingsScopeKind::User,
                scope_id: "default".to_owned(),
                caller: human.clone(),
                patch: SettingsPatch {
                    execution_pool: None,
                    retention_policy: Some(shortened),
                    confirm_retention_shortening: true,
                    analyzer_paths: None,
                    extra_typed: None,
                },
            },
            events: vec![],
            capacity_reservations: vec![],
            budget_reservations: vec![],
        })
        .unwrap();

    let after_set = store
        .read_settings(SettingsScopeKind::User, "default")
        .unwrap();
    let pending = after_set
        .pending_retention
        .expect("raccourcissement doit attendre le préavis 24h");
    assert!(pending.effective_at_ms.as_i64() >= set_short_time + 86_400_000);
    // La politique active est toujours 30j avant 24h
    assert_eq!(after_set.retention_policy.tool_terminal_logs_days, 30);

    // Promesse de reçu existante jamais raccourcie
    let promised_after: i64 = store
        .read_snapshot(|conn, _| {
            Ok(conn.query_row(
                "SELECT response_retained_until_ms FROM command_receipts WHERE aggregate_id = ?1 LIMIT 1",
                [project_id.to_string()],
                |r| r.get(0),
            )?)
        })
        .unwrap();
    assert_eq!(promised_after, promised_until_before);

    // 4. PurgeHistory AVANT les 24h :
    //    - `h_expired_ok` (40j > 30j) est purgé ; événement >= 7j acked est tombstoné ;
    //    - `h_short_15d` (15j < 30j) est conservé car le raccourcissement à 10j n'est PAS encore effectif !
    //    - Tous les objets protégés sont conservés (`protected_skipped_count >= 8`) ;
    //    - `retention_purges` + `wal_checkpoint(TRUNCATE)` vérifiés.
    let rep1 = store
        .execute_retention_purge(RetentionPurgeRequest {
            scope_kind: "user".to_owned(),
            scope_id: "default".to_owned(),
            requested_by_actor: "user:louis".to_owned(),
            reason: "purge Q-12 pré-préavis".to_owned(),
            older_than_override_ms: None,
            is_automatic_sweep: false,
        })
        .unwrap();
    assert!(rep1.wal_checkpoint_truncated);
    assert_eq!(rep1.wal_bytes_after, 0);
    assert!(rep1.objects_purged_count >= 1);
    assert!(rep1.events_tombstoned_count >= 1);
    assert!(rep1.protected_skipped_count >= 8);
    assert!(!rep1.affected_backups.is_empty());
    assert!(
        rep1.affected_backups
            .iter()
            .all(|b| b.max_residual_days <= 35)
    );
    assert!(!rep1.limitations.is_empty());

    // Vérifier états objets
    let state_of = |h: Sha256Digest| -> String {
        store
            .read_snapshot(|conn, _| {
                Ok(conn.query_row(
                    "SELECT state FROM object_records WHERE object_hash = ?1",
                    [h.as_prefixed_str()],
                    |r| r.get(0),
                )?)
            })
            .unwrap()
    };
    assert_eq!(state_of(h_expired_ok), "tombstone");
    assert_eq!(state_of(h_short_15d), "live");
    for prot in [
        h_prot_run,
        h_prot_blocked,
        h_prot_unknown,
        h_prot_mission,
        h_prot_inv,
        h_prot_ev,
        h_prot_pub,
        h_prot_keep,
    ] {
        assert_eq!(
            state_of(prot),
            "live",
            "objet protégé {prot} ne doit jamais être purgé"
        );
    }

    // 5. Après expiration du préavis (>= 24h) : le seuil 10j s'applique à `h_short_15d` !
    clock.advance(Duration::from_secs(25 * 3600));
    let rep2 = store
        .execute_retention_purge(RetentionPurgeRequest {
            scope_kind: "user".to_owned(),
            scope_id: "default".to_owned(),
            requested_by_actor: "user:louis".to_owned(),
            reason: "purge Q-12 post-préavis 24h".to_owned(),
            older_than_override_ms: None,
            is_automatic_sweep: false,
        })
        .unwrap();
    assert!(rep2.objects_purged_count >= 1);
    assert_eq!(state_of(h_short_15d), "tombstone");

    let purges_logged: i64 = store
        .read_snapshot(|conn, _| {
            Ok(conn.query_row(
                "SELECT COUNT(*) FROM retention_purges WHERE wal_checkpoint_truncated = 1",
                [],
                |r| r.get(0),
            )?)
        })
        .unwrap();
    assert_eq!(purges_logged, 2);

    // 6. Horloge incertaine suspend le balayage automatique Q-12
    cfg.clock_uncertain
        .store(true, std::sync::atomic::Ordering::Release);
    let rep_uncertain = store
        .execute_retention_purge(RetentionPurgeRequest {
            scope_kind: "user".to_owned(),
            scope_id: "default".to_owned(),
            requested_by_actor: "system:sweep".to_owned(),
            reason: "balayage quotidien".to_owned(),
            older_than_override_ms: None,
            is_automatic_sweep: true,
        })
        .unwrap();
    assert!(rep_uncertain.suspended_clock_uncertain);
    assert_eq!(rep_uncertain.objects_purged_count, 0);
}
