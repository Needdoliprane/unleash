//! REQ-REC-009 : identité stable des agrégats admin et filtrage de projet.
use std::sync::Arc;
use unleash_domain::{ActorId, IdGeneratorPort, ProjectId, RequestId, Revision};
use unleash_protocol::{CommandEnvelope, ProtocolVersion, ReceiptState};
use unleash_store::{
    AggregateKind, CallerAuthority, CallerRole, CommandMutation, CommitCommandRequest,
    CommitOutcome, GuardConfigInput, GuardPolicyMode, GuardTransport, PolicyIsolationMode,
    PolicyScopeKind, ReadEventsOutcome, SettingsPatch, SettingsScopeKind, StoreConfig, StoreError,
    UnleashStore,
};
use unleash_testkit::{DeterministicIdGenerator, local_test_dir};
use uuid::Uuid;

#[expect(
    clippy::unwrap_used,
    reason = "Construction de commandes de test synthétiques"
)]
fn commit(
    store: &UnleashStore,
    ids: &DeterministicIdGenerator,
    target: AggregateKind,
    aggregate: Uuid,
    project: Option<ProjectId>,
    revision: u64,
    mutation: CommandMutation,
) -> Result<CommitOutcome, StoreError> {
    let command_type = match &mutation {
        CommandMutation::CreateProject { .. } => "CreateProject",
        CommandMutation::SetSettings { .. } => "SetSettings",
        CommandMutation::SetPolicy { .. } => "SetPolicy",
        CommandMutation::ConfigureGuard { .. } => "ConfigureGuard",
        _ => "unsupported_fixture",
    };
    store.commit_command(CommitCommandRequest {
        actor_id: ActorId::new("user:louis").unwrap(),
        envelope: CommandEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_uuid(ids.new_uuid()),
            command_id: ids.new_command_id(),
            authority_generation: store.active_authority_generation().unwrap(),
            actor_ref: Some("user:louis".to_owned()),
            project_id: project,
            aggregate_id: aggregate,
            expected_revision: Revision::new(revision),
            command_type: command_type.to_owned(),
            payload: serde_json::Map::new(),
        },
        target_aggregate: target,
        operation_id: None,
        receipt_state: ReceiptState::Completed,
        result_ref: None,
        retention_days_override: None,
        fence_assertion: None,
        mutation,
        events: vec![],
        capacity_reservations: vec![],
        budget_reservations: vec![],
    })
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "Deux révisions des trois opérations admin, identité de projection et visibilité projet"
)]
fn admin_revisions_reuse_envelope_aggregate_and_project_filters() {
    let directory = local_test_dir("f018-009-event-scope").unwrap();
    let ids = Arc::new(DeterministicIdGenerator::new(0xF018_0901));
    let mut config = StoreConfig::new(directory.path());
    config.id_generator = ids.clone();
    let store = UnleashStore::open(config).unwrap();
    let project = ProjectId::from_uuid(ids.new_uuid());
    let caller = CallerAuthority {
        actor_id: ActorId::new("user:louis").unwrap(),
        role: CallerRole::HumanOperator,
    };
    commit(
        &store,
        &ids,
        AggregateKind::Project,
        project.as_uuid(),
        Some(project),
        0,
        CommandMutation::CreateProject {
            project_id: project,
            name: "project".to_owned(),
            weight: 1,
        },
    )
    .unwrap();
    let settings = ids.new_uuid();
    let policy = ids.new_uuid();
    let guard = ids.new_uuid();
    for revision in 0..2 {
        commit(
            &store,
            &ids,
            AggregateKind::Settings,
            settings,
            Some(project),
            revision,
            CommandMutation::SetSettings {
                scope_kind: SettingsScopeKind::Project,
                scope_id: project.to_string(),
                caller: caller.clone(),
                patch: SettingsPatch {
                    execution_pool: None,
                    retention_policy: None,
                    confirm_retention_shortening: false,
                    analyzer_paths: None,
                    extra_typed: Some(serde_json::Map::new()),
                },
            },
        )
        .unwrap();
        commit(
            &store,
            &ids,
            AggregateKind::SecurityPolicy,
            policy,
            Some(project),
            revision,
            CommandMutation::SetPolicy {
                scope_kind: PolicyScopeKind::Project,
                scope_id: project.to_string(),
                isolation_mode: PolicyIsolationMode::Standard,
                policy_version: format!("v{revision}"),
                reason: "project policy".to_owned(),
                caller: caller.clone(),
            },
        )
        .unwrap();
        commit(
            &store,
            &ids,
            AggregateKind::GuardConfig,
            guard,
            None,
            revision,
            CommandMutation::ConfigureGuard {
                config: GuardConfigInput {
                    config_id: "default".to_owned(),
                    guard_mode: GuardPolicyMode::BrokerOnly,
                    endpoint: None,
                    transport: GuardTransport::None,
                    secret_ref_id: None,
                    evaluate_timeout_ms: 2000,
                    health_check_interval_s: 30,
                    decision_cache_max_age_s: 300,
                    fallback_policy_ref: None,
                    guard_mapping_profile: "none".to_owned(),
                    reason: format!("revision {revision}"),
                },
                caller: caller.clone(),
            },
        )
        .unwrap();
    }
    let stream = store.daemon_state().unwrap().stream_id;
    let ReadEventsOutcome::Events { events, .. } = store
        .read_events(stream, unleash_domain::Sequence::new(0), 20, None)
        .unwrap()
    else {
        panic!("unexpected resync");
    };
    for (event_type, aggregate, expected_project) in [
        ("SettingsUpdated", settings, Some(project)),
        ("SecurityPolicyUpdated", policy, Some(project)),
        ("GuardConfigured", guard, None),
    ] {
        let matching: Vec<_> = events
            .iter()
            .filter(|event| event.event_type == event_type)
            .collect();
        assert_eq!(matching.len(), 2);
        assert!(
            matching.iter().all(
                |event| event.aggregate_id == aggregate && event.project_id == expected_project
            )
        );
        assert_eq!(
            matching
                .iter()
                .map(|event| event.aggregate_revision.as_u64())
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
    }
    let scoped_count: i64 = store
        .read_snapshot(|connection, _| {
            connection
                .query_row(
                    "SELECT COUNT(*) FROM events WHERE project_id = ?1",
                    [project.to_string()],
                    |row| row.get(0),
                )
                .map_err(StoreError::from_rusqlite)
        })
        .unwrap();
    assert_eq!(scoped_count, 4);
}

#[test]
fn project_policy_requires_existing_project_in_writer_transaction() {
    let directory = local_test_dir("f018-009-orphan-policy").unwrap();
    let store = UnleashStore::open(StoreConfig::new(directory.path())).unwrap();
    let ids = DeterministicIdGenerator::new(0xF018_0902);
    let project = ProjectId::from_uuid(ids.new_uuid());
    let outcome = commit(
        &store,
        &ids,
        AggregateKind::SecurityPolicy,
        ids.new_uuid(),
        Some(project),
        0,
        CommandMutation::SetPolicy {
            scope_kind: PolicyScopeKind::Project,
            scope_id: project.to_string(),
            isolation_mode: PolicyIsolationMode::Standard,
            policy_version: "v1".to_owned(),
            reason: "orphan policy must be rejected".to_owned(),
            caller: CallerAuthority {
                actor_id: ActorId::new("user:louis").unwrap(),
                role: CallerRole::HumanOperator,
            },
        },
    );
    assert!(matches!(outcome, Err(StoreError::NotFound(_))));
    let counts: (i64, i64) = store
        .read_snapshot(|conn, _| {
            Ok((
                conn.query_row("SELECT COUNT(*) FROM security_policies", [], |row| {
                    row.get(0)
                })?,
                conn.query_row("SELECT COUNT(*) FROM command_receipts", [], |row| {
                    row.get(0)
                })?,
            ))
        })
        .unwrap();
    assert_eq!(counts, (0, 0));
}
