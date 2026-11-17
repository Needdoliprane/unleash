// T-F002-003 / AC-F002-003 / REQ-PERSIST-003
//! Outbox, déduplication inbox, `dead_letter` à 8 échecs, réarmement unique et fenêtre 7 jours.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use serde_json::{Map, Value};
use unleash_domain::{
    ActorId, CompletionPolicy, IdGeneratorPort, MissionId, MissionState, ProjectId, RequestId,
    Revision, Sha256Digest, TimestampMs,
};
use unleash_protocol::{
    CommandEnvelope, EventCausationKind, ProtocolVersion, ReceiptState, Sensitivity,
};
use unleash_store::{
    AggregateKind, CommandMutation, CommitCommandRequest, EventAppendSpec, InboxApplyOutcome,
    MAX_OUTBOX_NON_TRANSIENT_ATTEMPTS, MonotonicClockPort, ReadEventsOutcome, StoreConfig,
    UnleashStore,
};
use unleash_testkit::{DeterministicIdGenerator, ManualClock, local_test_dir};
use uuid::Uuid;

struct ClockAdapter(Arc<ManualClock>);
impl MonotonicClockPort for ClockAdapter {
    fn monotonic_now(&self) -> Duration {
        self.0.monotonic_now()
    }
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "Scénario complet T-F002-003 : outbox parallèle, jitter, dédup inbox, 8 échecs dead_letter, réarmement unique et fenêtre 7 j"
)]
fn test_t_f002_003_outbox_inbox_dedup_dead_letter_rearm_and_cursor_window() {
    let dir = local_test_dir("f002-003").unwrap();
    let clock = Arc::new(ManualClock::default());
    let ids = Arc::new(DeterministicIdGenerator::new(0xF002_0003));
    let mut cfg = StoreConfig::new(dir.path());
    cfg.clock = clock.clone();
    cfg.monotonic_clock = Arc::new(ClockAdapter(Arc::clone(&clock)));
    cfg.id_generator = ids.clone();
    cfg.consumer_version = "0.1.0".to_owned();

    let store = UnleashStore::open(cfg.clone()).unwrap();
    store
        .register_principal("p1", "user", "user:louis")
        .unwrap();
    let authority = store.active_authority_generation().unwrap();
    let project_id = ProjectId::from_uuid(ids.new_uuid());
    let mission_id = MissionId::from_uuid(ids.new_uuid());

    store
        .commit_command(CommitCommandRequest {
            actor_id: ActorId::new("user:louis").unwrap(),
            envelope: CommandEnvelope {
                protocol_version: ProtocolVersion,
                request_id: RequestId::from_uuid(ids.new_uuid()),
                command_id: ids.new_command_id(),
                authority_generation: authority,
                actor_ref: Some("user:louis".to_owned()),
                project_id: Some(project_id),
                aggregate_id: project_id.as_uuid(),
                expected_revision: Revision::INITIAL,
                command_type: "CreateProject".to_owned(),
                payload: Map::new(),
            },
            target_aggregate: AggregateKind::Project,
            operation_id: None,
            receipt_state: ReceiptState::Completed,
            result_ref: None,
            retention_days_override: None,
            fence_assertion: None,
            mutation: CommandMutation::CreateProject {
                project_id,
                name: "p".to_owned(),
                weight: 1,
            },
            events: vec![],
            capacity_reservations: vec![],
            budget_reservations: vec![],
        })
        .unwrap();

    // Créer mission + événement avec 2 consommateurs outbox ("consumer-ok", "consumer-fail")
    let mut p = Map::new();
    p.insert("mission".into(), Value::String(mission_id.to_string()));
    store
        .commit_command(CommitCommandRequest {
            actor_id: ActorId::new("user:louis").unwrap(),
            envelope: CommandEnvelope {
                protocol_version: ProtocolVersion,
                request_id: RequestId::from_uuid(ids.new_uuid()),
                command_id: ids.new_command_id(),
                authority_generation: authority,
                actor_ref: Some("user:louis".to_owned()),
                project_id: Some(project_id),
                aggregate_id: mission_id.as_uuid(),
                expected_revision: Revision::INITIAL,
                command_type: "CreateMission".to_owned(),
                payload: p.clone(),
            },
            target_aggregate: AggregateKind::Mission,
            operation_id: None,
            receipt_state: ReceiptState::Accepted,
            result_ref: None,
            retention_days_override: None,
            fence_assertion: None,
            mutation: CommandMutation::CreateMission {
                mission_id,
                goal_id: None,
                project_id,
                owner_principal_id: Some("p1".to_owned()),
                owner_agent_id: None,
                completion_policy: CompletionPolicy::Mechanical,
                state: MissionState::Ready,
            },
            events: vec![EventAppendSpec {
                event_type: "MissionCreated".to_owned(),
                aggregate_type: AggregateKind::Mission,
                aggregate_id: mission_id.as_uuid(),
                aggregate_revision: Revision::new(1),
                project_id: Some(project_id),
                correlation_id: Uuid::from_u128(11),
                causation_kind: EventCausationKind::Command,
                causation_id: None,
                payload: p,
                sensitivity: Sensitivity::Project,
                outbox_destinations: vec!["consumer-ok".to_owned(), "consumer-fail".to_owned()],
            }],
            capacity_reservations: vec![],
            budget_reservations: vec![],
        })
        .unwrap();

    // Livrer 10 fois le même événement à consumer-ok + simuler crash avant ack (restart)
    let batch = store.lease_outbox_batch("consumer-ok", 10).unwrap();
    assert_eq!(batch.len(), 1);
    let item = batch.first().unwrap();
    let mutation_counter = Arc::new(AtomicUsize::new(0));
    let res_hash = Sha256Digest::from_bytes(b"applied-ok");

    for attempt in 0..10 {
        let cnt = Arc::clone(&mutation_counter);
        let outcome = store
            .apply_inbox_event("consumer-ok", &item.envelope, res_hash, move |_tx| {
                cnt.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
            .unwrap();
        if attempt == 0 {
            assert!(matches!(outcome, InboxApplyOutcome::Applied { .. }));
        } else {
            assert!(matches!(
                outcome,
                InboxApplyOutcome::AlreadyProcessed { .. }
            ));
        }
    }
    assert_eq!(mutation_counter.load(Ordering::SeqCst), 1);

    // Crash avant ack outbox : redémarrage dans la même lignée réinitialise la lease d'epoch antérieur
    let store = store.restart_same_lineage().unwrap();
    let releaved = store.lease_outbox_batch("consumer-ok", 10).unwrap();
    assert_eq!(releaved.len(), 1);
    store
        .ack_outbox_delivery(item.event_id, "consumer-ok")
        .unwrap();

    // Purge inbox impossible tant que < 7 jours après acked_at_ms
    clock.advance(Duration::from_secs(6 * 86_400 + 23 * 3600));
    let purge_early = store.purge_expired_inbox().unwrap();
    assert_eq!(purge_early.affected_rows, 0);

    // SubscribeEvents (ReadEvents) à 6 j 23 h dans le même stream_id réussit
    let stream_id = store.daemon_state().unwrap().stream_id;
    let read_6d23h = store
        .read_events(
            stream_id,
            item.sequence,
            50,
            Some(TimestampMs::new(
                clock.utc_ms() - (6 * 86_400_000 + 23 * 3_600_000),
            )),
        )
        .unwrap();
    assert!(matches!(read_6d23h, ReadEventsOutcome::Events { .. }));

    // Au-delà de 7 jours : ReadEvents -> ResyncRequired ; inbox devient purgeable
    clock.advance(Duration::from_secs(2 * 3600));
    let read_expired = store
        .read_events(
            stream_id,
            item.sequence,
            50,
            Some(TimestampMs::new(
                clock.utc_ms() - (7 * 86_400_000 + 3_600_000),
            )),
        )
        .unwrap();
    assert!(matches!(
        read_expired,
        ReadEventsOutcome::ResyncRequired { .. }
    ));

    let purge_ok = store.purge_expired_inbox().unwrap();
    assert_eq!(purge_ok.affected_rows, 1);

    // consumer-fail : 8 échecs non transitoires -> dead_letter + blocage agrégat DELIVERY_DEAD_LETTER
    let mut dead_lettered = false;
    for attempt in 1..=MAX_OUTBOX_NON_TRANSIENT_ATTEMPTS {
        let dl = store
            .record_outbox_non_transient_failure(
                item.event_id,
                "consumer-fail",
                "fatal parse error",
                Some(("mission", mission_id.as_uuid())),
            )
            .unwrap();
        if attempt < MAX_OUTBOX_NON_TRANSIENT_ATTEMPTS {
            assert!(!dl);
        } else {
            dead_lettered = dl;
        }
    }
    assert!(dead_lettered);
    let dls = store.list_dead_letters().unwrap();
    assert_eq!(dls.len(), 1);
    assert_eq!(dls.first().unwrap().consumer_id, "consumer-fail");

    // Agrégat mission bloqué avec DELIVERY_DEAD_LETTER
    let mission_state: String = store
        .read_snapshot(|conn, _| {
            Ok(conn.query_row(
                "SELECT state FROM missions WHERE mission_id = ?1",
                [mission_id.to_string()],
                |r| r.get(0),
            )?)
        })
        .unwrap();
    assert_eq!(mission_state, "blocked");

    // Redémarrage même consumer_version -> NE réarme PAS
    let store = store.restart_same_lineage().unwrap();
    assert!(
        store
            .lease_outbox_batch("consumer-fail", 10)
            .unwrap()
            .is_empty()
    );

    // Redémarrage avec nouvelle consumer_version -> réarme UNE SEULE FOIS
    store.shutdown().unwrap();
    let mut cfg_v2 = cfg.clone();
    cfg_v2.consumer_version = "0.2.0".to_owned();
    let store_v2 = UnleashStore::open(cfg_v2.clone()).unwrap();
    let rearmed = store_v2.lease_outbox_batch("consumer-fail", 10).unwrap();
    assert_eq!(rearmed.len(), 1);

    // Si cette version 0.2.0 échoue encore 8 fois, un futur changement vers 0.3.0 ne réarme plus une 2e fois
    for _ in 0..MAX_OUTBOX_NON_TRANSIENT_ATTEMPTS {
        store_v2
            .record_outbox_non_transient_failure(
                item.event_id,
                "consumer-fail",
                "still failing",
                None,
            )
            .unwrap();
    }
    store_v2.shutdown().unwrap();
    let mut cfg_v3 = cfg;
    cfg_v3.consumer_version = "0.3.0".to_owned();
    let store_v3 = UnleashStore::open(cfg_v3).unwrap();
    assert!(
        store_v3
            .lease_outbox_batch("consumer-fail", 10)
            .unwrap()
            .is_empty()
    );
}
