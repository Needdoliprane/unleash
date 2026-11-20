//! Writer/clock model only: published proof bytes and OS tuples are synthetic.
//! The separate daemon tests must establish actual handshake/host/PTy behavior.
use super::*;
use std::sync::{
    Mutex,
    atomic::{AtomicI64, AtomicUsize},
};
use unleash_store::terminal_allocations::terminal_renewal_types::{
    ResolveTerminalRenewalRequest, TERMINAL_RENEWAL_VERB, TerminalRenewalObservation,
    TerminalRenewalRecord, TerminalRenewalState,
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

fn set_global_policy(store: &UnleashStore, allocation: &TerminalAllocationRecord) {
    let mut request = registered_request(store, allocation, 0xf011_1720);
    request.target_aggregate = AggregateKind::SecurityPolicy;
    request.envelope.project_id = None;
    request.envelope.aggregate_id = request.envelope.authority_generation.as_uuid();
    request.envelope.command_type = "SetPolicy".into();
    request.envelope.payload.clone_from(
        json!({"scope_kind":"global","scope_id":"global",
            "isolation_mode":"standard","policy_version":"v017","reason":"fixture policy change"})
        .as_object()
        .unwrap(),
    );
    request.mutation = CommandMutation::SetPolicy {
        caller: CallerAuthority {
            actor_id: request.actor_id.clone(),
            role: CallerRole::PolicyAdmin,
        },
        scope_kind: PolicyScopeKind::Global,
        scope_id: "global".into(),
        isolation_mode: PolicyIsolationMode::Standard,
        policy_version: "v017".into(),
        reason: "fixture policy change".into(),
    };
    assert_eq!(
        store
            .commit_command(request)
            .unwrap()
            .receipt()
            .new_revision,
        Revision::new(1)
    );
}

fn pending_renewal(
    store: &UnleashStore,
    allocation: &TerminalAllocationRecord,
) -> TerminalRenewalRecord {
    let request = registered_request(store, allocation, 0xf011_1721);
    store.commit_command(request.clone()).unwrap();
    store
        .inspect_terminal_renewal_request(&request.actor_id, request.envelope.command_id)
        .unwrap()
        .value
        .unwrap()
}

#[test]
fn t_f011_v017_refusal_blocks_late_dispatch_after_payload_purge() {
    let (store, clock, allocation, _root) =
        observed_with_observer(false, Arc::new(NoopCutpointObserver));
    let renewal = pending_renewal(&store, &allocation);
    store.commit_broker_phase(authorizing(&renewal)).unwrap();
    set_global_policy(&store, &allocation);
    let c = &renewal.context;
    let disposed = || {
        store
            .dispose_undispatched_terminal_renewal(c.action_id, renewal.context_digest)
            .unwrap()
    };
    let cursor = disposed().expect("a policy change makes the unstarted intent obsolete");
    assert_eq!(disposed(), None);
    let actual = store
        .inspect_terminal_renewal_request(&c.actor_id, c.command_id)
        .unwrap()
        .value
        .unwrap();
    assert_eq!(actual.state, TerminalRenewalState::Refused);
    assert_eq!(actual.broker_action_state, Some(ActionState::Cancelled));
    assert!(actual.dispatch_id.is_none());

    let expiry: i64 = store
        .read_snapshot(|conn, _| {
            Ok(conn.query_row(
                "SELECT expires_at_ms FROM resource_leases WHERE lease_id=?1",
                [c.lease_id.to_string()],
                |r| r.get(0),
            )?)
        })
        .unwrap();
    assert_eq!(expiry, c.previous_expires_at_ms.as_i64());
    store
        .commit_command(registered_request(&store, &allocation, 0xf011_1722))
        .unwrap();

    let sequence = i64::try_from(cursor.sequence.as_u64()).unwrap();
    let header: (String, String) = store.execute_in_writer_tx(WriteCriticality::Normal, move |tx,now| {
        let header: (String, String) = tx.query_row(
            "SELECT event_id,payload_hash FROM events WHERE sequence=?1 AND event_type='TerminalControlDispatchRefused'",
            [sequence], |r| Ok((r.get(0)?,r.get(1)?)),
        )?;
        assert_eq!(tx.execute("UPDATE outbox SET state='acked',acked_at_ms=?2 WHERE event_id=?1",
            rusqlite::params![header.0,now.as_i64()])?, 1);
        assert!(tx.execute("UPDATE events SET payload_hash='forged' WHERE event_id=?1", [&header.0]).is_err());
        Ok(header)
    }).unwrap();
    let elapsed = 8 * 24 * 60 * 60 * 1000;
    clock.set_utc_ms(elapsed);
    store.purge_history(TimestampMs::new(elapsed)).unwrap();
    store.read_snapshot(|conn,_| {
        assert!(conn.prepare("SELECT 1 FROM events WHERE event_id=?1 AND payload_hash=?2 AND payload_state='tombstone'")?
            .exists([&header.0,&header.1])?);
        Ok(())
    }).unwrap();

    let late = ActionDispatchInput {
        dispatch_id: DispatchId::from_u128(0xf011_1723).to_string(),
        action_id: c.action_id,
        ordinal: 1,
        adapter_id: "owned-fixture".into(),
        request_hash: renewal.context_digest,
        provider_request_id: None,
        transport_state: "dispatched".into(),
        receipt_hash: None,
    };
    let Err(error) = store.record_action_dispatch(late) else {
        panic!("refused action got a marker")
    };
    assert!(
        error
            .to_string()
            .contains("terminal refusal forbids a dispatch"),
        "{error}"
    );
    assert_eq!(disposed(), None);
}

#[test]
fn t_f011_v017_dispatch_wins_over_pre_dispatch_refusal() {
    let (store, clock, allocation, _root) =
        observed_with_observer(false, Arc::new(NoopCutpointObserver));
    let renewal = pending_renewal(&store, &allocation);
    let (_, unknown) = dispatch_authorizing(&store, &authorizing(&renewal));
    set_global_policy(&store, &allocation);
    let c = &renewal.context;
    assert_eq!(
        store
            .dispose_undispatched_terminal_renewal(c.action_id, renewal.context_digest)
            .unwrap(),
        None
    );
    clock.set_utc_ms(110);
    store.commit_broker_phase(unknown).unwrap();
    let view = store
        .inspect_terminal_renewal_request(&c.actor_id, c.command_id)
        .unwrap()
        .value
        .unwrap();
    assert_eq!(view.state, TerminalRenewalState::OutcomeUnknown);
    assert!(view.dispatch_id.is_some());
    assert!(matches!(
        store.commit_command(registered_request(&store, &allocation, 0xf011_1724)),
        Err(StoreError::CapacityBlocked(_))
    ));
}

#[test]
fn t_f011_v017_sql_cannot_dispose_unknown_without_a_marker() {
    let (store, _clock, allocation, _root) =
        observed_with_observer(false, Arc::new(NoopCutpointObserver));
    let renewal = pending_renewal(&store, &allocation);
    store.commit_broker_phase(authorizing(&renewal)).unwrap();
    let action = renewal.context.action_id;
    store
        .execute_in_writer_tx(WriteCriticality::Normal, move |tx, _| {
            assert_eq!(
                tx.execute(
                    "UPDATE actions SET state='outcome_unknown' WHERE action_id=?1",
                    [action.to_string()]
                )?,
                1
            );
            Ok(())
        })
        .unwrap();
    let stream = store.daemon_state().unwrap().stream_id;
    let event = uuid::Uuid::from_u128(0xf011_1725).to_string();
    let event_test = event.clone();
    let attempt = store.execute_in_writer_tx(WriteCriticality::Normal, move |tx, now| {
        use rusqlite::params;
        let c = &renewal.context;
        let payload = json!({"action_id":action,"command_id":c.command_id,
            "receipt_id":c.receipt_id,"terminal_id":c.terminal_id,
            "context_digest":renewal.context_digest,"operation":"renewal","reason":"policy_changed"});
        tx.execute("INSERT INTO events(event_id,stream_id,aggregate_type,aggregate_id,aggregate_revision,event_ordinal,event_type,schema_version,project_id,actor_ref,actor_id,occurred_at_ms,recorded_at_ms,payload_json,payload_state,payload_hash,sensitivity,causation_kind,causation_id,causation_receipt_id,correlation_id) VALUES(?1,?2,'terminal_control_intent',?3,2,0,'TerminalControlDispatchRefused',1,?4,?5,?5,?6,?6,?7,'full',?8,'restricted','command',?9,?9,?10)",
            params![event_test,stream.to_string(),action.to_string(),c.project_id.to_string(),c.actor_id.as_str(),now.as_i64(),serde_json::to_string(&payload)?,canonical_sha256(&payload)?.to_string(),c.receipt_id.to_string(),c.terminal_id.to_string()])?;
        tx.execute("UPDATE terminal_renewal_intents SET revision=2,refusal_event_id=?2 WHERE action_id=?1",
            params![action.to_string(),event_test])?;
        Ok(())
    });
    let Err(error) = attempt else {
        panic!("SQL allowed a refusal to erase the Unknown barrier")
    };
    assert!(
        error
            .to_string()
            .contains("terminal renewal transition invalid"),
        "{error}"
    );
    store
        .read_snapshot(|conn, _| {
            assert!(
                !conn
                    .prepare("SELECT 1 FROM events WHERE event_id=?1")?
                    .exists([&event])?
            );
            assert!(
                conn.prepare(
                    "SELECT 1 FROM terminal_renewal_intents r JOIN actions a USING(action_id)
           WHERE r.action_id=?1 AND r.refusal_event_id IS NULL AND a.state='outcome_unknown'"
                )?
                .exists([action.to_string()])?
            );
            Ok(())
        })
        .unwrap();
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
        handshake_response_digest: Some(Sha256Digest::from_bytes(
            b"synthetic writer fixture; no native handshake",
        )),
        resize_ack: None,
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
    let (_, mut phase) = dispatch_authorizing(&store, &authorizing(&renewal));
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

#[test]
fn t_f011_renewal_generic_success_without_typed_proof_is_refused() {
    let (store, clock, _fault, allocation, _root) = observed(false);
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
    let (_, mut phase) = dispatch_authorizing(&store, &authorizing(&renewal));
    let BrokerPhase::Observation { receipt, .. } = &phase.phase else {
        unreachable!()
    };
    let resolved = store
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
    // A published object and a claimed success do not replace the typed
    // producer port used by the daemon journal's observation slot.
    publish(&store, &renewal, &mut phase);
    clock.set_utc_ms(10000);
    assert!(matches!(store.commit_broker_phase(phase),
        Err(StoreError::InvalidArgument {message,..}) if message=="terminal renewal observation producer not qualified"));
    let after = store
        .inspect_terminal_renewal_request(
            &allocation.context.actor_id,
            unleash_domain::CommandId::from_u128(CONTROL),
        )
        .unwrap()
        .value
        .unwrap();
    assert_eq!(after, resolved);
    store.read_snapshot(|conn,_| {
        let expiry:i64=conn.query_row("SELECT expires_at_ms FROM resource_leases WHERE lease_id=?1",[renewal.context.lease_id.to_string()],|row|row.get(0))?;
        assert_eq!(expiry,renewal.context.previous_expires_at_ms.as_i64());
        assert!(!conn.prepare("SELECT 1 FROM broker_phase_records WHERE action_id=?1 AND phase_key LIKE 'observation:%'")?.exists([renewal.context.action_id.to_string()])?);
        Ok(())
    }).unwrap();
}
