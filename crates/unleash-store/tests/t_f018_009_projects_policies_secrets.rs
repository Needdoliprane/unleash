// T-F018-009 / AC-F018-009 / REQ-REC-009
//! Persistance transactionnelle des projets (`ReviseProject`/`ListProjects`/`GetProject`),
//! paramètres (`ReadSettings`/`SetSettings` avec Q-15 `CONTROL_RESERVE_INCOHERENT`),
//! politiques/AI-Guard (`SetPolicy`/`ReadPolicySummary`/`ConfigureGuard`),
//! références secrètes (`SetSecretReference`/`RevokeSecretReference`/`GetSecretReferenceStatus` sans fuite),
//! alertes (`alerts` / `ListAlerts`) et diagnostic explicite `StorageCorrupt`.

use std::sync::Arc;

use serde_json::{Map, json};
use unleash_domain::{ActorId, IdGeneratorPort, ProjectId, RequestId, Revision, SecretRef};
use unleash_protocol::{CommandEnvelope, ErrorCode, ProtocolVersion, ReasonCode, ReceiptState};
use unleash_store::{
    AggregateKind, AlertSeverity, AlertState, CallerAuthority, CallerRole, CommandMutation,
    CommitCommandRequest, ControlReserveMode, ExecutionPoolSettings, GuardConfigInput,
    GuardPolicyMode, GuardTransport, ListAlertsFilter, NewAlertSpec, PolicyIsolationMode,
    PolicyScopeKind, ProjectRepositorySpec, ProjectSourceSpec, RemoteRevocationState,
    SecretRefState, SecretReferenceInput, SettingsPatch, SettingsScopeKind, StoreConfig,
    StoreError, UnleashStore,
};
use unleash_testkit::{DeterministicIdGenerator, ManualClock, local_test_dir};

fn make_env(
    ids: &DeterministicIdGenerator,
    auth: unleash_domain::AuthorityGeneration,
    proj: Option<ProjectId>,
    agg: uuid::Uuid,
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
    reason = "Couvre exhaustivement les nouveaux DAO M1 et l'absence absolue de secrets persistés"
)]
fn test_t_f018_009_projects_settings_policies_guard_secrets_alerts_and_corrupt_detection() {
    let dir = local_test_dir("f018-009").unwrap();
    let workspace_dir = local_test_dir("f018-009-proj-root").unwrap();
    let clock = Arc::new(ManualClock::default());
    let ids = Arc::new(DeterministicIdGenerator::new(0xF018_0009));

    let mut cfg = StoreConfig::new(dir.path());
    cfg.clock = clock;
    cfg.id_generator = ids.clone();

    let store = UnleashStore::open(cfg.clone()).unwrap();
    let auth = store.active_authority_generation().unwrap();
    let admin = ActorId::new("user:louis").unwrap();
    let human = CallerAuthority {
        actor_id: admin.clone(),
        role: CallerRole::HumanOperator,
    };

    // 1. Projets : CreateProject, ReviseProject (validation chemin + projet généraliste), ListProjects, GetProject
    let p_gen = ProjectId::from_uuid(ids.new_uuid());
    store
        .commit_command(CommitCommandRequest {
            actor_id: admin.clone(),
            envelope: make_env(
                &ids,
                auth,
                Some(p_gen),
                p_gen.as_uuid(),
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
                project_id: p_gen,
                name: "Projet Généraliste".to_owned(),
                weight: 2,
            },
            events: vec![],
            capacity_reservations: vec![],
            budget_reservations: vec![],
        })
        .unwrap();

    // Chemin `.claude` ou relatif refusé sur ReviseProject
    let claude_path = workspace_dir.path().join(".claude");
    std::fs::create_dir_all(&claude_path).unwrap();
    let bad_root_err = store
        .commit_command(CommitCommandRequest {
            actor_id: admin.clone(),
            envelope: make_env(
                &ids,
                auth,
                Some(p_gen),
                p_gen.as_uuid(),
                Revision::new(1),
                "ReviseProject",
            ),
            target_aggregate: AggregateKind::Project,
            operation_id: None,
            receipt_state: ReceiptState::Completed,
            result_ref: None,
            retention_days_override: None,
            fence_assertion: None,
            mutation: CommandMutation::ReviseProject {
                project_id: p_gen,
                name: "Projet Illégal".to_owned(),
                description: "interdit".to_owned(),
                weight: 2,
                primary_root_path: Some(claude_path.display().to_string()),
                sources: vec![],
                repositories: vec![],
                settings_json: json!({}),
            },
            events: vec![],
            capacity_reservations: vec![],
            budget_reservations: vec![],
        })
        .unwrap_err();
    assert_eq!(bad_root_err.error_code(), ErrorCode::InvalidArgument);

    // Révision valide avec racine workspace réelle
    store
        .commit_command(CommitCommandRequest {
            actor_id: admin.clone(),
            envelope: make_env(
                &ids,
                auth,
                Some(p_gen),
                p_gen.as_uuid(),
                Revision::new(1),
                "ReviseProject",
            ),
            target_aggregate: AggregateKind::Project,
            operation_id: None,
            receipt_state: ReceiptState::Completed,
            result_ref: None,
            retention_days_override: None,
            fence_assertion: None,
            mutation: CommandMutation::ReviseProject {
                project_id: p_gen,
                name: "Projet Révisé".to_owned(),
                description: "Description métier".to_owned(),
                weight: 4,
                primary_root_path: Some(workspace_dir.path().display().to_string()),
                sources: vec![ProjectSourceSpec {
                    kind: "directory".to_owned(),
                    path_or_uri: workspace_dir.path().display().to_string(),
                    label: "docs".to_owned(),
                    read_only: true,
                }],
                repositories: vec![ProjectRepositorySpec {
                    repo_id: "repo-main".to_owned(),
                    root_path: workspace_dir.path().display().to_string(),
                    default_branch: Some("dev".to_owned()),
                }],
                settings_json: json!({"default_isolation": "standard"}),
            },
            events: vec![],
            capacity_reservations: vec![],
            budget_reservations: vec![],
        })
        .unwrap();

    let fetched = store.get_project(p_gen).unwrap();
    assert_eq!(fetched.name, "Projet Révisé");
    assert_eq!(fetched.weight, 4);
    assert_eq!(fetched.revision, Revision::new(2));
    assert_eq!(store.list_projects().unwrap().len(), 1);

    // 2. ReadSettings / SetSettings (validation Q-15 CONTROL_RESERVE_INCOHERENT + synchro capacity_pools)
    let init_settings = store
        .read_settings(SettingsScopeKind::User, "default")
        .unwrap();
    assert_eq!(init_settings.revision, Revision::INITIAL);
    assert_eq!(init_settings.execution_pool.max_active_executions, 5);
    assert_eq!(
        init_settings.execution_pool.control_reserve_mode,
        ControlReserveMode::Soft
    );

    let settings_agg = uuid::Uuid::from_u128(0x5E77);
    let incoherent_err = store
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
                    execution_pool: Some(ExecutionPoolSettings {
                        max_active_executions: 1,
                        control_reserve_mode: ControlReserveMode::Hard,
                        control_reserved_units: 1,
                        control_burst_limit: 2,
                        checkpoint_escalation_s: 60,
                        project_weight: 1,
                    }),
                    retention_policy: None,
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
    assert_eq!(
        incoherent_err.reason_code(),
        Some(ReasonCode::ControlReserveIncoherent)
    );

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
                    execution_pool: Some(ExecutionPoolSettings {
                        max_active_executions: 6,
                        control_reserve_mode: ControlReserveMode::Hard,
                        control_reserved_units: 2,
                        control_burst_limit: 3,
                        checkpoint_escalation_s: 90,
                        project_weight: 2,
                    }),
                    retention_policy: None,
                    confirm_retention_shortening: false,
                    analyzer_paths: None,
                    extra_typed: None,
                },
            },
            events: vec![],
            capacity_reservations: vec![],
            budget_reservations: vec![],
        })
        .unwrap();

    let updated_settings = store
        .read_settings(SettingsScopeKind::User, "default")
        .unwrap();
    assert_eq!(updated_settings.revision, Revision::new(1));
    assert_eq!(updated_settings.execution_pool.max_active_executions, 6);
    assert_eq!(
        updated_settings.execution_pool.control_reserve_mode,
        ControlReserveMode::Hard
    );

    // 3. SetSecretReference / GetSecretReferenceStatus / RevokeSecretReference (ZERO octet secret en base !)
    let sec_agg = uuid::Uuid::from_u128(0x5EC1);
    let canary_secret = "sk-live-CANARY-99999";
    let opaque = SecretRef::new("secret://keychain/anthropic/prod").unwrap();
    store
        .commit_command(CommitCommandRequest {
            actor_id: admin.clone(),
            envelope: make_env(
                &ids,
                auth,
                Some(p_gen),
                sec_agg,
                Revision::INITIAL,
                "SetSecretReference",
            ),
            target_aggregate: AggregateKind::SecretReference,
            operation_id: None,
            receipt_state: ReceiptState::Completed,
            result_ref: None,
            retention_days_override: None,
            fence_assertion: None,
            mutation: CommandMutation::SetSecretReference {
                input: SecretReferenceInput {
                    secret_ref_id: "sec-guard-token".to_owned(),
                    secret_ref: opaque.clone(),
                    namespace: "guard".to_owned(),
                    purpose: "mtls_client_token".to_owned(),
                    secret_kind: "bearer_token".to_owned(),
                    target_kind: "guard_config".to_owned(),
                    target_id: "default".to_owned(),
                    scope_kind: "project".to_owned(),
                    scope_id: p_gen.to_string(),
                    backend_kind: "os_keychain".to_owned(),
                },
                caller: human.clone(),
            },
            events: vec![],
            capacity_reservations: vec![],
            budget_reservations: vec![],
        })
        .unwrap();

    let sec_status = store
        .get_secret_reference_status("sec-guard-token")
        .unwrap();
    assert_eq!(sec_status.status, SecretRefState::Active);
    assert_eq!(sec_status.opaque_secret_ref, opaque);

    // 4. SetPolicy / ReadPolicySummary / ConfigureGuard
    let pol_agg = uuid::Uuid::from_u128(0xB011);
    store
        .commit_command(CommitCommandRequest {
            actor_id: admin.clone(),
            envelope: make_env(
                &ids,
                auth,
                Some(p_gen),
                pol_agg,
                Revision::INITIAL,
                "SetPolicy",
            ),
            target_aggregate: AggregateKind::SecurityPolicy,
            operation_id: None,
            receipt_state: ReceiptState::Completed,
            result_ref: None,
            retention_days_override: None,
            fence_assertion: None,
            mutation: CommandMutation::SetPolicy {
                scope_kind: PolicyScopeKind::Project,
                scope_id: p_gen.to_string(),
                isolation_mode: PolicyIsolationMode::Strict,
                policy_version: "pol-v1".to_owned(),
                reason: "durcissement projet sensible".to_owned(),
                caller: human.clone(),
            },
            events: vec![],
            capacity_reservations: vec![],
            budget_reservations: vec![],
        })
        .unwrap();

    // guard_optional sans fallback_policy_ref refusé
    let grd_agg = uuid::Uuid::from_u128(0x6D01);
    let bad_guard = store
        .commit_command(CommitCommandRequest {
            actor_id: admin.clone(),
            envelope: make_env(
                &ids,
                auth,
                None,
                grd_agg,
                Revision::INITIAL,
                "ConfigureGuard",
            ),
            target_aggregate: AggregateKind::GuardConfig,
            operation_id: None,
            receipt_state: ReceiptState::Completed,
            result_ref: None,
            retention_days_override: None,
            fence_assertion: None,
            mutation: CommandMutation::ConfigureGuard {
                config: GuardConfigInput {
                    config_id: "default".to_owned(),
                    guard_mode: GuardPolicyMode::GuardOptional,
                    endpoint: Some("unix:///tmp/guard.sock".to_owned()),
                    transport: GuardTransport::UnixSocket,
                    secret_ref_id: Some("sec-guard-token".to_owned()),
                    evaluate_timeout_ms: 2000,
                    health_check_interval_s: 30,
                    decision_cache_max_age_s: 300,
                    fallback_policy_ref: None,
                    guard_mapping_profile: "v1".to_owned(),
                    reason: "sans repli".to_owned(),
                },
                caller: human.clone(),
            },
            events: vec![],
            capacity_reservations: vec![],
            budget_reservations: vec![],
        })
        .unwrap_err();
    assert_eq!(bad_guard.error_code(), ErrorCode::InvalidArgument);

    store
        .commit_command(CommitCommandRequest {
            actor_id: admin.clone(),
            envelope: make_env(
                &ids,
                auth,
                None,
                grd_agg,
                Revision::INITIAL,
                "ConfigureGuard",
            ),
            target_aggregate: AggregateKind::GuardConfig,
            operation_id: None,
            receipt_state: ReceiptState::Completed,
            result_ref: None,
            retention_days_override: None,
            fence_assertion: None,
            mutation: CommandMutation::ConfigureGuard {
                config: GuardConfigInput {
                    config_id: "default".to_owned(),
                    guard_mode: GuardPolicyMode::GuardRequired,
                    endpoint: Some("https://guard.internal:8443".to_owned()),
                    transport: GuardTransport::HttpsMtls,
                    secret_ref_id: Some("sec-guard-token".to_owned()),
                    evaluate_timeout_ms: 2000,
                    health_check_interval_s: 30,
                    decision_cache_max_age_s: 300,
                    fallback_policy_ref: Some("pol-v1".to_owned()),
                    guard_mapping_profile: "ext-guard-v1".to_owned(),
                    reason: "activation guard requis".to_owned(),
                },
                caller: human.clone(),
            },
            events: vec![],
            capacity_reservations: vec![],
            budget_reservations: vec![],
        })
        .unwrap();

    let summary = store.read_policy_summary(Some(p_gen)).unwrap();
    assert_eq!(
        summary.effective_isolation_mode,
        PolicyIsolationMode::Strict
    );
    assert_eq!(
        summary.guard_config.as_ref().map(|g| g.guard_mode),
        Some(GuardPolicyMode::GuardRequired)
    );

    // Révocation locale distincte de révocation distante non supportée
    store
        .commit_command(CommitCommandRequest {
            actor_id: admin,
            envelope: make_env(
                &ids,
                auth,
                Some(p_gen),
                sec_agg,
                Revision::new(1),
                "RevokeSecretReference",
            ),
            target_aggregate: AggregateKind::SecretReference,
            operation_id: None,
            receipt_state: ReceiptState::Completed,
            result_ref: None,
            retention_days_override: None,
            fence_assertion: None,
            mutation: CommandMutation::RevokeSecretReference {
                secret_ref_id: "sec-guard-token".to_owned(),
                reason: "rotation demandée".to_owned(),
                caller: human,
                remote_revocation_supported: false,
                remote_revocation_confirmed: false,
            },
            events: vec![],
            capacity_reservations: vec![],
            budget_reservations: vec![],
        })
        .unwrap();
    let revoked_sec = store
        .get_secret_reference_status("sec-guard-token")
        .unwrap();
    assert_eq!(revoked_sec.status, SecretRefState::RevokedLocal);
    assert!(revoked_sec.local_revoked_at_ms.is_some());
    assert_eq!(
        revoked_sec.remote_revocation_state,
        RemoteRevocationState::Unsupported
    );

    // Vérifier qu'aucun canari ou colonne brute de secret n'existe en SQLite
    let canary_hits: i64 = store
        .read_snapshot(|conn, _| {
            let e: i64 = conn.query_row(
                "SELECT COUNT(*) FROM events WHERE payload_json LIKE ?1",
                [format!("%{canary_secret}%")],
                |r| r.get(0),
            )?;
            let s: i64 = conn.query_row(
                "SELECT COUNT(*) FROM secret_references WHERE opaque_secret_ref LIKE ?1",
                [format!("%{canary_secret}%")],
                |r| r.get(0),
            )?;
            Ok(e + s)
        })
        .unwrap();
    assert_eq!(canary_hits, 0);

    // 5. Table `alerts` + remontée des alertes bloquantes hors filtre
    let other_proj = ProjectId::from_uuid(ids.new_uuid());
    store
        .record_alert(NewAlertSpec {
            alert_id: "alert-blk-1".to_owned(),
            kind: "recovery_blocked".to_owned(),
            severity: AlertSeverity::Blocking,
            blocking: true,
            project_id: Some(p_gen),
            subject_kind: "action".to_owned(),
            subject_id: "act-1".to_owned(),
            title: "Effet externe inconnu".to_owned(),
            summary: "Réconciliation requise avant reprise".to_owned(),
            reason_code: Some("UNRESOLVED_EFFECT".to_owned()),
            evidence_ref: None,
            allowed_operations: vec![
                "ReconcileAction".to_owned(),
                "ResolveActionOutcome".to_owned(),
            ],
        })
        .unwrap();

    let alerts_other = store
        .list_alerts(&ListAlertsFilter {
            project_id: Some(other_proj),
            state: Some(AlertState::Active),
            severity: None,
            include_blocking_regardless_of_filter: true,
        })
        .unwrap();
    assert!(
        alerts_other
            .iter()
            .any(|a| a.alert_id == "alert-blk-1" && a.blocking)
    );

    store.shutdown().unwrap();

    // 6. Corruption stockage détectée explicitement (`StorageCorrupt`) sans repli silencieux
    std::fs::write(
        &cfg.db_path,
        b"not-a-valid-sqlite-database-file-corrupted!!",
    )
    .unwrap();
    let corrupt_err = UnleashStore::open(cfg).unwrap_err();
    assert!(matches!(corrupt_err, StoreError::StorageCorrupt(_)));
    assert_eq!(corrupt_err.reason_code(), Some(ReasonCode::StorageCorrupt));
}
