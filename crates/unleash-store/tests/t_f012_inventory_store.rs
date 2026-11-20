//! REQ-MODEL-013 / T-F012-013: inventory cannot enter the generic broker writer.
//! Synthetic trusted caller only; the source receipt uses the real command Tx.

use std::error::Error;
use std::sync::Arc;

use rusqlite::{Connection, OpenFlags};
use serde_json::json;
use unleash_domain::{
    ActionId, ActionOrigin, ActorId, AuthorizationEvaluationId, CommandId, Fence, LineageFence,
    ReconcileStrategy, RequestId, Revision, Sha256Digest, TimestampMs,
};
use unleash_protocol::{CommandEnvelope, ProtocolVersion, ReceiptState, canonical_sha256};
use unleash_store::{
    AggregateKind, BrokerCause, BrokerEvaluation, BrokerIntent, BrokerPhase, BrokerPhaseRequest,
    CallerAuthority, CallerRole, CommandMutation, CommitCommandRequest, SettingsPatch,
    SettingsScopeKind, StoreConfig, StoreError, UnleashStore,
};
use unleash_testkit::{ManualClock, local_test_dir};
use uuid::Uuid;

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

struct Fixture {
    store: UnleashStore,
    root: tempfile::TempDir,
}

impl Fixture {
    fn new() -> TestResult<Self> {
        let root = local_test_dir("f012-inventory-store")?;
        let mut config = StoreConfig::new(root.path());
        config.clock = Arc::new(ManualClock::new(1_000));
        let store = UnleashStore::open(config)?;
        store.register_principal("inventory-principal", "user", "user:inventory-store")?;
        Ok(Self { store, root })
    }

    fn source(&self) -> TestResult<unleash_protocol::CommandReceipt> {
        let actor = ActorId::new("user:inventory-store")?;
        let envelope = CommandEnvelope {
            protocol_version: ProtocolVersion,
            request_id: RequestId::from_u128(0xf012_1001),
            command_id: CommandId::from_u128(0xf012_1001),
            authority_generation: self.store.active_authority_generation()?,
            actor_ref: Some(actor.to_string()),
            project_id: None,
            aggregate_id: Uuid::from_u128(0xf012_1001),
            expected_revision: Revision::INITIAL,
            command_type: "SetSettings".to_owned(),
            payload: serde_json::Map::new(),
        };
        Ok(self
            .store
            .commit_command(CommitCommandRequest {
                actor_id: actor.clone(),
                envelope,
                target_aggregate: AggregateKind::Settings,
                operation_id: None,
                receipt_state: ReceiptState::Completed,
                result_ref: None,
                retention_days_override: None,
                fence_assertion: None,
                mutation: CommandMutation::SetSettings {
                    scope_kind: SettingsScopeKind::User,
                    scope_id: "default".to_owned(),
                    caller: CallerAuthority {
                        actor_id: actor,
                        role: CallerRole::HumanOperator,
                    },
                    patch: SettingsPatch {
                        execution_pool: None,
                        retention_policy: None,
                        confirm_retention_shortening: false,
                        analyzer_paths: None,
                        extra_typed: Some(serde_json::Map::new()),
                    },
                },
                events: vec![],
                capacity_reservations: vec![],
                budget_reservations: vec![],
            })?
            .receipt()
            .clone())
    }

    fn counts(&self) -> TestResult<Vec<i64>> {
        let db = Connection::open_with_flags(
            self.root.path().join("unleash-authority.db"),
            OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        [
            "actions",
            "broker_intents",
            "broker_evaluations",
            "broker_phase_records",
            "action_dispatches",
            "provider_probes",
            "provider_probe_transport_contexts",
            "capacity_reservations",
            "resource_leases",
            "model_calls",
            "model_call_manifests",
            "events",
        ]
        .into_iter()
        .map(|table| {
            db.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .map_err(Into::into)
        })
        .collect()
    }

    fn authorizing(
        &self,
        source: &unleash_protocol::CommandReceipt,
    ) -> TestResult<BrokerPhaseRequest> {
        let daemon = self.store.daemon_state()?;
        let principal = ActorId::new("inventory-principal")?;
        let action = ActionId::from_u128(0xf012_1002);
        let digest = Sha256Digest::from_bytes(b"synthetic inventory without a transport claim");
        let fence = LineageFence::new(
            daemon.authority_generation,
            daemon.daemon_epoch,
            Fence::ZERO,
        );
        let manifest = json!({
            "action_id":action,"subject":{"kind":"control_plane","principal_id":principal,"cause_ref":format!("receipt:{}",source.receipt_id),"invocation_id":null,"attempt_id":null,"replay_worker_id":null,"agent_id":null,"mission_id":null,"role_version":null},
            "project_id":null,"verb":"model.provider.inventory","resource_refs":[{"kind":"network_endpoint","value":{"host":"127.0.0.1","port":11434,"is_private_or_loopback":true}}],
            "payload_digest":digest,"reconcile_strategy":"observe_then_decide","effects_class":"network_egress","scope_revision":source.new_revision,"policy_revision":0,"lineage_fence":fence,"action_deadline":60_000,
            "isolation_requirements":{"mode":"standard","require_process_confinement":false,"required_capabilities":[]},"secret_refs":[],"provenance":{"origin":"engine_internal"}
        }).as_object().cloned().ok_or("manifest object required")?;
        let intent = BrokerIntent {
            action_id: action,
            origin: ActionOrigin::ControlPlane {
                control_principal_id: principal.clone(),
            },
            principal_id: principal,
            cause: BrokerCause::CommandReceipt {
                receipt_id: source.receipt_id,
            },
            project_id: None,
            payload_digest: digest,
            scope_revision: source.new_revision,
            policy_revision: Revision::INITIAL,
            lineage_fence: fence,
            action_deadline: TimestampMs::new(60_000),
            reconcile_strategy: ReconcileStrategy::ObserveThenDecide,
            intent_digest: canonical_sha256(&manifest)?,
            manifest,
        };
        Ok(BrokerPhaseRequest {
            authority_generation: daemon.authority_generation,
            expected_revision: Revision::INITIAL,
            evaluation: BrokerEvaluation {
                evaluation_id: AuthorizationEvaluationId::from_u128(0xf012_1003),
                previous_evaluation_id: None,
            },
            phase: BrokerPhase::Authorizing { intent },
        })
    }
}

#[test]
fn t_f012_013_inventory_generic_authorizing_requires_its_original_claim() -> TestResult {
    let fixture = Fixture::new()?;
    let source = fixture.source()?;
    let request = fixture.authorizing(&source)?;
    let before = fixture.counts()?;
    for _ in 0..2 {
        let rejected = fixture.store.commit_broker_phase(request.clone());
        match rejected {
            Err(StoreError::NotFound(_) | StoreError::EvidenceUnavailable(_)) => {}
            Ok(_) => return Err("generic inventory append was acknowledged without its prepared context and original claim".into()),
            Err(error) => return Err(format!("unexpected generic inventory refusal: code={}, reason={:?}", error.error_code(), error.reason_code()).into()),
        }
        if fixture.counts()? != before {
            return Err("generic inventory refusal changed actions, admission or events".into());
        }
    }
    Ok(())
}
