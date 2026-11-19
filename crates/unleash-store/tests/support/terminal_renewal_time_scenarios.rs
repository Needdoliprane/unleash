//! Writer/clock model only: published proof bytes and OS tuples are synthetic.
//! The separate daemon tests must establish actual handshake/host/PTy behavior.
use super::*;
use std::sync::{
    Mutex,
    atomic::{AtomicI64, AtomicUsize},
};
use unleash_store::terminal_allocations::terminal_renewal_types::{
    ResolveTerminalRenewalRequest, TERMINAL_RENEWAL_VERB, TerminalRenewalObservation,
    TerminalRenewalRecord,
};

#[derive(Default)]
struct ClockCutpoint {
    armed: AtomicBool,
    hits: AtomicUsize,
    target: AtomicI64,
    clock: Mutex<Option<Arc<ManualClock>>>,
    uncertain: Mutex<Option<Arc<AtomicBool>>>,
    make_uncertain: AtomicBool,
}
impl CutpointObserver for ClockCutpoint {
    fn hit(&self, point: StoreCutpoint) -> Result<(), StoreError> {
        if point == StoreCutpoint::C1BeforeTerminalRenewalObservationCommit
            && self.armed.swap(false, Ordering::AcqRel)
        {
            self.hits.fetch_add(1, Ordering::AcqRel);
            self.clock
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .set_utc_ms(self.target.load(Ordering::Acquire));
            if self.make_uncertain.load(Ordering::Acquire) {
                self.uncertain
                    .lock()
                    .unwrap()
                    .as_ref()
                    .unwrap()
                    .store(true, Ordering::Release);
            }
        }
        Ok(())
    }
}

fn authorizing(record: &TerminalRenewalRecord) -> BrokerPhaseRequest {
    let c = &record.context;
    let fence = LineageFence::new(c.authority_generation, c.daemon_epoch, Fence::new(0));
    let manifest=json!({"action_id":c.action_id,"subject":{"kind":"control_plane","principal_id":c.principal_id,"cause_ref":format!("receipt:{}",c.receipt_id),"invocation_id":null,"attempt_id":null,"replay_worker_id":null,"agent_id":null,"mission_id":null,"role_version":null},"project_id":c.project_id,"verb":TERMINAL_RENEWAL_VERB,"resource_refs":[{"kind":"terminal","value":c.terminal_id}],"payload_digest":record.context_digest,"reconcile_strategy":"observe_then_decide","effects_class":"process_exec","scope_revision":c.scope_revision,"policy_revision":c.policy_revision,"lineage_fence":fence,"action_deadline":c.previous_expires_at_ms,"isolation_requirements":{"mode":"standard","require_process_confinement":false,"required_capabilities":[]},"secret_refs":[],"provenance":{"origin":"engine_internal"}}).as_object().unwrap().clone();
    BrokerPhaseRequest {
        authority_generation: c.authority_generation,
        expected_revision: Revision::INITIAL,
        evaluation: BrokerEvaluation {
            evaluation_id: AuthorizationEvaluationId::from_uuid(c.action_id.as_uuid()),
            previous_evaluation_id: None,
        },
        phase: BrokerPhase::Authorizing {
            intent: BrokerIntent {
                action_id: c.action_id,
                origin: ActionOrigin::ControlPlane {
                    control_principal_id: c.principal_id.clone(),
                },
                principal_id: c.principal_id.clone(),
                cause: BrokerCause::CommandReceipt {
                    receipt_id: c.receipt_id,
                },
                project_id: Some(c.project_id),
                payload_digest: record.context_digest,
                scope_revision: c.scope_revision,
                policy_revision: c.policy_revision,
                lineage_fence: fence,
                action_deadline: c.previous_expires_at_ms,
                reconcile_strategy: unleash_domain::ReconcileStrategy::ObserveThenDecide,
                intent_digest: canonical_sha256(&manifest).unwrap(),
                manifest,
            },
        },
    }
}

fn publish(
    store: &UnleashStore,
    record: &TerminalRenewalRecord,
    phase: &mut BrokerPhaseRequest,
) -> TerminalRenewalObservation {
    let BrokerPhase::Observation { receipt, .. } = &mut phase.phase else {
        unreachable!()
    };
    let c = &record.context;
    let observation = TerminalRenewalObservation {
        action_id: c.action_id,
        dispatch_id: receipt.dispatch_id,
        context_digest: record.context_digest,
        terminal_id: c.terminal_id,
        host_id: c.host_id,
        process_run_uuid: c.process_run_uuid,
        os_identity: c.expected_os_identity.clone(),
        daemon_epoch: c.daemon_epoch,
        resource_fence: c.resource_fence,
        handshake_response_digest: Sha256Digest::from_bytes(
            b"synthetic writer fixture; no native handshake",
        ),
        manifest_digest: Sha256Digest::from_bytes(b"synthetic writer fixture manifest"),
        effective_environment_digest: Sha256Digest::from_bytes(
            b"synthetic writer fixture environment",
        ),
        durable_log_seq: 0,
        last_confirmed_input_seq: 0,
        unknown_input_seqs: vec![],
        observed_at_ms: TimestampMs::new(110),
    };
    let bytes = canonicalize_to_vec(&observation).unwrap();
    let hash = canonical_sha256(&observation).unwrap();
    let prepared = store
        .prepare_object(PrepareObjectRequest {
            object_hash: hash,
            byte_length: u64::try_from(bytes.len()).unwrap(),
            media_type: "application/json".into(),
            owner_kind: "terminal_renewal_observation".into(),
            owner_id: c.action_id.to_string(),
        })
        .unwrap();
    store
        .write_prepared_object_bytes(&prepared, &bytes)
        .unwrap();
    store
        .publish_object_reference(PublishObjectReferenceRequest {
            pin_id: prepared.pin_id,
            object_hash: hash,
            object_generation: prepared.object_generation,
            owner_kind: "terminal_renewal_observation".into(),
            owner_id: c.action_id.to_string(),
            source_field: receipt.dispatch_id.to_string(),
        })
        .unwrap();
    receipt.state = ActionState::ObservedSuccess;
    receipt.output_digest = Some(hash);
    receipt.observed_at_ms = TimestampMs::new(120);
    observation
}

#[derive(Clone, Copy)]
enum ClockStep {
    Stable,
    Backward,
    Expired,
    Uncertain,
}

fn check(step: ClockStep) {
    let point = Arc::new(ClockCutpoint::default());
    let (store, clock, allocation, _root) = observed_with_observer(false, point.clone());
    *point.clock.lock().unwrap() = Some(clock.clone());
    *point.uncertain.lock().unwrap() = Some(store.config().clock_uncertain.clone());
    store
        .commit_command(registered_request(&store, &allocation, CONTROL))
        .unwrap();
    let renewal = store
        .inspect_terminal_renewal_request(
            &allocation.context.actor_id,
            unleash_domain::CommandId::from_u128(CONTROL),
        )
        .unwrap()
        .value
        .unwrap();
    let (_, mut phase) = dispatch_authorizing(&store, authorizing(&renewal));
    let BrokerPhase::Observation { receipt, .. } = &phase.phase else {
        unreachable!()
    };
    store
        .resolve_terminal_renewal(ResolveTerminalRenewalRequest {
            action_id: renewal.context.action_id,
            dispatch_id: receipt.dispatch_id,
            context_digest: renewal.context_digest,
            expected_revision: renewal.revision,
            caller: CallerAuthority {
                actor_id: renewal.context.actor_id.clone(),
                role: CallerRole::HumanOperator,
            },
        })
        .unwrap();
    let observation = publish(&store, &renewal, &mut phase);
    // Stay above capture/receipt after a pure backward step: only comparing
    // final_now to the writer's initial UTC can detect this discontinuity.
    clock.set_utc_ms(10000);
    point.target.store(
        match step {
            ClockStep::Backward => 9999,
            ClockStep::Expired => renewal.context.previous_expires_at_ms.as_i64(),
            _ => 10000,
        },
        Ordering::Release,
    );
    point
        .make_uncertain
        .store(matches!(step, ClockStep::Uncertain), Ordering::Release);
    point.armed.store(true, Ordering::Release);
    let result = store.commit_terminal_renewal_observation(observation, phase);
    assert_eq!(
        point.hits.load(Ordering::Acquire),
        1,
        "the exact precommit cutpoint must be reached"
    );
    if matches!(step, ClockStep::Stable) {
        assert!(result.is_ok(), "{result:?}");
    } else {
        let refusal = match (step, &result) {
            (ClockStep::Backward, Err(StoreError::InvalidArgument { message, .. })) => {
                message == "renewal clock moved backwards during observation"
            }
            (ClockStep::Expired, Err(StoreError::StaleFence { message })) => {
                message == "terminal renewal authority or lease expired"
            }
            (ClockStep::Uncertain, Err(StoreError::InvalidArgument { message, .. })) => {
                message == "renewal clock became uncertain"
            }
            _ => false,
        };
        assert!(
            refusal,
            "the exact post-barrier clock guard must refuse renewal: {result:?}"
        );
    }
    store.read_snapshot(|conn,_| {
        let row:(String,i64,i64,i64,String,i64)=conn.query_row(
            "SELECT s.lease_id,l.fence,s.control_revision,l.expires_at_ms,r.state,(SELECT COUNT(*) FROM broker_phase_records WHERE action_id=r.action_id AND phase_key LIKE 'observation:%') FROM terminal_sessions s JOIN resource_leases l ON l.lease_id=s.lease_id JOIN terminal_renewal_intents r ON r.terminal_id=s.terminal_id WHERE r.action_id=?1",
            [renewal.context.action_id.to_string()],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?)))?;
        assert_eq!(row.0,renewal.context.lease_id.to_string());
        assert_eq!(row.1,i64::try_from(renewal.context.resource_fence.as_u64()).unwrap());
        assert_eq!(row.2,i64::try_from(renewal.context.control_revision.as_u64()).unwrap());
        let known=matches!(step,ClockStep::Stable);
        assert_eq!(row.3,if known {renewal.context.proposed_expires_at_ms.as_i64()} else {renewal.context.previous_expires_at_ms.as_i64()});
        assert_eq!(row.4,if known {"observed"} else {"handshake_resolved"});
        assert_eq!(row.5,i64::from(known));
        Ok(())
    }).unwrap();
}

#[test]
fn t_f011_renewal_observation_stable_clock_commits_same_controller() {
    check(ClockStep::Stable);
}
#[test]
fn t_f011_renewal_observation_rechecks_backward_utc_after_barrier() {
    check(ClockStep::Backward);
}
#[test]
fn t_f011_renewal_observation_rechecks_expiry_after_barrier() {
    check(ClockStep::Expired);
}
#[test]
fn t_f011_renewal_observation_rechecks_uncertainty_after_barrier() {
    check(ClockStep::Uncertain);
}
