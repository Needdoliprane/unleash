//! T-F012-015 / AC-F012-015 / REQ-MODEL-015 : issues d'une génération managée et d'un lancement
//! refusé avant tout processus, engagées par leurs ports dédiés du writer dans `SQLite` réel.
//! Aucun réseau, aucun processus fils, aucun confinement simulé : seuls les contrats du writer.

use super::*;
use unleash_store::{BrokerPhaseCommit, ManagedGenerationObservation};

// T-F012-015 / AC-F012-015 / REQ-MODEL-015 — tout type public porte `Debug`.
#[test]
fn t_f012_015_managed_launch_and_generation_types_implement_debug() {
    fn assert_debug<T: std::fmt::Debug>() {}
    assert_debug::<unleash_store::AttestedProviderServiceLaunch>();
    assert_debug::<unleash_store::ManagedGenerationDispatchPermit>();
    assert_debug::<unleash_store::ManagedOllamaTransport>();
    assert_debug::<unleash_store::ProviderServiceLaunchPermit>();
}


// T-F012-015 / AC-F012-015 / REQ-MODEL-015 — le port d'observation dédié n'admet que l'issue
// qualifiée de ce dispatch : le port générique et tout fait divergent restent refusés.
#[test]
fn t_f012_015_managed_generation_observation_refuses_generic_or_divergent_facts()
-> Result<(), Box<dyn std::error::Error>> {
    let (store, _root, _clock, caller, _pool, mandate) = setup(ProviderDialectKind::OllamaChat);
    let (permit, request, native, intent_digest) =
        dispatched_generation(&store, &caller, mandate)?;
    let action = native.action_id;
    let success = |native: &ManagedGenerationObservation| {
        generation_observation(
            &store,
            &request,
            native,
            intent_digest,
            unleash_domain::ActionState::ObservedSuccess,
            Some(native.outcome_digest),
        )
    };

    assert!(store.commit_broker_phase(success(&native)).is_err());
    assert!(
        store
            .commit_managed_generation_observation(&permit, success(&native), None)
            .is_err()
    );
    for divergent in [
        ManagedGenerationObservation {
            dispatch_id: unleash_domain::DispatchId::from_u128(0xf012_0699),
            ..native.clone()
        },
        ManagedGenerationObservation {
            action_id: unleash_domain::ActionId::from_u128(0xf012_0698),
            ..native.clone()
        },
        ManagedGenerationObservation {
            request_digest: Sha256Digest::from_bytes(b"another native request"),
            ..native.clone()
        },
        ManagedGenerationObservation {
            schema_version: 2,
            ..native.clone()
        },
    ] {
        assert!(
            store
                .commit_managed_generation_observation(&permit, success(&native), Some(divergent))
                .is_err()
        );
    }
    let unknown = generation_observation(
        &store,
        &request,
        &native,
        intent_digest,
        unleash_domain::ActionState::OutcomeUnknown,
        None,
    );
    // Une issue inconnue n'adopte jamais un fait natif ; un succès porte toujours son empreinte.
    assert!(
        store
            .commit_managed_generation_observation(&permit, unknown.clone(), Some(native.clone()))
            .is_err()
    );
    assert!(
        store
            .commit_managed_generation_observation(
                &permit,
                generation_observation(
                    &store,
                    &request,
                    &native,
                    intent_digest,
                    unleash_domain::ActionState::OutcomeUnknown,
                    Some(native.outcome_digest),
                ),
                None,
            )
            .is_err()
    );
    assert!(
        store
            .commit_managed_generation_observation(
                &permit,
                generation_observation(
                    &store,
                    &request,
                    &native,
                    intent_digest,
                    unleash_domain::ActionState::ObservedSuccess,
                    None,
                ),
                Some(native.clone()),
            )
            .is_err()
    );
    let before = store
        .inspect_broker_action(action)?
        .ok_or("missing probe action")?;
    assert_eq!(before.state, unleash_domain::ActionState::Dispatching);
    assert_eq!(count_observations(&store, action)?, 0);

    // Les refus n'ont rien engagé : l'issue inconnue qualifiée est la seule observation admise.
    assert!(
        store
            .commit_managed_generation_observation(&permit, unknown, None)
            .is_ok()
    );
    let after = store
        .inspect_broker_action(action)?
        .ok_or("missing probe action")?;
    assert_eq!(after.state, unleash_domain::ActionState::OutcomeUnknown);
    assert_eq!(count_observations(&store, action)?, 1);
    Ok(())
}


// T-F012-015 / AC-F012-015 / REQ-MODEL-015 — une issue inconnue est consignée sans fait natif
// ni empreinte ; le port générique reste fermé.
#[test]
fn t_f012_015_managed_generation_unknown_outcome_is_recorded_without_a_native_fact()
-> Result<(), Box<dyn std::error::Error>> {
    let (store, _root, _clock, caller, _pool, mandate) = setup(ProviderDialectKind::OllamaChat);
    let (permit, request, native, intent_digest) =
        dispatched_generation(&store, &caller, mandate)?;
    let unknown = generation_observation(
        &store,
        &request,
        &native,
        intent_digest,
        unleash_domain::ActionState::OutcomeUnknown,
        None,
    );
    assert!(store.commit_broker_phase(unknown.clone()).is_err());
    assert!(
        store
            .commit_managed_generation_observation(&permit, unknown.clone(), Some(native))
            .is_err()
    );
    let commit = store.commit_managed_generation_observation(&permit, unknown.clone(), None)?;
    assert!(!commit.replayed);
    let after = store
        .inspect_broker_action(native.action_id)?
        .ok_or("missing probe action")?;
    assert_eq!(after.state, unleash_domain::ActionState::OutcomeUnknown);
    assert_eq!(after.dispatch_id, Some(permit.dispatch_id()));
    let payload = read_observed_event(&store, &commit)?;
    assert_eq!(payload.get("state"), Some(&json!("outcome_unknown")));
    assert_eq!(payload.get("output_digest"), Some(&Value::Null));
    assert!(payload.get("managed_generation_observation").is_none());
    let replay = store.commit_managed_generation_observation(&permit, unknown, None)?;
    assert!(replay.replayed);
    assert_eq!(replay.event_cursor, commit.event_cursor);
    assert_eq!(count_observations(&store, native.action_id)?, 1);
    Ok(())
}


// T-F012-015 / AC-F012-015 / REQ-MODEL-015 — le succès observé est engagé par le port dédié,
// lié au marqueur durable du dispatch, puis se rejoue sans seconde observation.
#[test]
fn t_f012_015_managed_generation_observed_success_commits_once_through_its_port()
-> Result<(), Box<dyn std::error::Error>> {
    let (store, _root, _clock, caller, _pool, mandate) = setup(ProviderDialectKind::OllamaChat);
    let (permit, request, native, intent_digest) =
        dispatched_generation(&store, &caller, mandate)?;
    let action = native.action_id;
    let success = generation_observation(
        &store,
        &request,
        &native,
        intent_digest,
        unleash_domain::ActionState::ObservedSuccess,
        Some(native.outcome_digest),
    );
    let commit =
        store.commit_managed_generation_observation(&permit, success.clone(), Some(native.clone()))?;
    assert!(!commit.replayed);
    let after = store
        .inspect_broker_action(action)?
        .ok_or("missing probe action")?;
    assert_eq!(after.state, unleash_domain::ActionState::ObservedSuccess);
    assert_eq!(after.dispatch_id, Some(native.dispatch_id));
    assert_eq!(after.last_event_cursor, commit.event_cursor);
    let payload = read_observed_event(&store, &commit)?;
    assert_eq!(payload.get("state"), Some(&json!("observed_success")));
    assert_eq!(
        payload.get("output_digest"),
        Some(&json!(native.outcome_digest))
    );
    assert_eq!(
        payload.pointer("/managed_generation_observation/request_digest"),
        Some(&json!(native.request_digest))
    );
    assert_eq!(
        payload.pointer("/managed_generation_observation/response_digest"),
        Some(&json!(native.response_digest))
    );

    let replay = store.commit_managed_generation_observation(&permit, success, Some(native))?;
    assert!(replay.replayed);
    assert_eq!(replay.event_cursor, commit.event_cursor);
    assert_eq!(count_observations(&store, action)?, 1);
    Ok(())
}


/// Permis de transport, requête autorisée, fait natif synthétique et intention de la génération.
fn dispatched_generation(
    store: &UnleashStore,
    caller: &CallerAuthority,
    mandate: unleash_domain::ReceiptId,
) -> Result<
    (
        unleash_store::ManagedGenerationDispatchPermit,
        BrokerPhaseRequest,
        ManagedGenerationObservation,
        Sha256Digest,
    ),
    Box<dyn std::error::Error>,
> {
    let (handoff, request) = authorized_generation_fixture(store, caller, mandate)?;
    let action = handoff.inputs.probe.context.action_id;
    let intent_digest = store
        .inspect_broker_action(action)?
        .ok_or("missing probe action")?
        .intent_digest;
    let transport = handoff
        .inputs
        .managed_transport
        .ok_or("missing native binding")?;
    let permit = store.commit_managed_generation_dispatch(
        &handoff.commit,
        request.clone(),
        transport.request_digest,
    )?;
    Ok((
        permit,
        request,
        ManagedGenerationObservation {
            schema_version: 1,
            action_id: action,
            dispatch_id: permit.dispatch_id(),
            request_digest: transport.request_digest,
            response_digest: Sha256Digest::from_bytes(b"fixture ndjson body"),
            outcome_digest: Sha256Digest::from_bytes(b"fixture native outcome"),
            observed_at_ms: store.config().clock.now_ms(),
        },
        intent_digest,
    ))
}


fn generation_observation(
    store: &UnleashStore,
    authorized: &BrokerPhaseRequest,
    native: &ManagedGenerationObservation,
    intent_digest: Sha256Digest,
    state: unleash_domain::ActionState,
    output_digest: Option<Sha256Digest>,
) -> BrokerPhaseRequest {
    use unleash_store::{BrokerActionReceipt, BrokerObservationCause};
    BrokerPhaseRequest {
        expected_revision: Revision::new(3),
        phase: BrokerPhase::Observation {
            action_id: native.action_id,
            intent_digest,
            observation_revision: Revision::new(1),
            cause: BrokerObservationCause::Execution,
            reason: None,
            receipt: BrokerActionReceipt {
                action_id: native.action_id,
                evaluation_id: authorized.evaluation.evaluation_id,
                dispatch_id: native.dispatch_id,
                decision_ref: "generation-transport-fixture".into(),
                state,
                output_digest,
                enforcement_refs: vec![],
                remote_audit_ref: None,
                remote_audit_unavailable: true,
                observed_at_ms: store.config().clock.now_ms(),
            },
        },
        ..authorized.clone()
    }
}


/// Payload publié aux clients de l'événement `ActionObserved` d'une phase.
fn read_observed_event(
    store: &UnleashStore,
    commit: &BrokerPhaseCommit,
) -> Result<Value, Box<dyn std::error::Error>> {
    let sequence = i64::try_from(commit.event_cursor.sequence.as_u64())?;
    let raw: String = store.read_snapshot(move |conn, _| {
        Ok(conn.query_row(
            "SELECT e.payload_json FROM events e JOIN outbox o ON o.event_id=e.event_id
             WHERE e.sequence=?1 AND e.event_type='ActionObserved' AND o.destination='clients'",
            [sequence],
            |row| row.get(0),
        )?)
    })?;
    Ok(serde_json::from_str(&raw)?)
}


fn count_observations(
    store: &UnleashStore,
    action: unleash_domain::ActionId,
) -> Result<i64, StoreError> {
    store.read_snapshot(move |conn, _| {
        Ok(conn.query_row(
            "SELECT COUNT(*) FROM broker_phase_records WHERE action_id=?1 AND phase_key LIKE 'observation:%'",
            [action.to_string()],
            |row| row.get(0),
        )?)
    })
}
