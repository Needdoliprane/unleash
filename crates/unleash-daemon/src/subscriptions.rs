// SPDX-License-Identifier: Apache-2.0
//! Hub borné d'abonnements `SubscribeEvents` avec backpressure et `ResyncRequired` (`REQ-CORE-009`, `REQ-CORE-010`).

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use tokio::net::UnixStream;
use tokio::sync::mpsc;
use tokio::sync::mpsc::error::TrySendError;
use unleash_domain::{ProjectId, Sequence};
use unleash_protocol::{
    ErrorCode, EventCursor, EventEnvelope, IpcError, RetryClass, SubscribeEventsFrame,
    SubscribeEventsRequest,
};
use unleash_store::{ReadEventsOutcome, UnleashStore};

use crate::ipc::write_json_frame_timeout;

#[derive(Debug)]
struct SubscriberEntry {
    sender: mpsc::Sender<Vec<EventEnvelope>>,
    abort_flag: Arc<tokio::sync::Notify>,
}

/// Hub non bloquant distribuant les événements engagés vers les abonnés actifs via files bornées.
#[derive(Debug)]
pub struct SubscriptionHub {
    capacity: usize,
    next_id: AtomicU64,
    subscribers: Mutex<HashMap<u64, SubscriberEntry>>,
}

impl SubscriptionHub {
    pub(crate) fn observe_internal(self: &Arc<Self>) -> InternalEventSubscription {
        let (id, receiver, aborted) = self.register();
        InternalEventSubscription {
            hub: Arc::clone(self),
            id,
            receiver,
            aborted,
        }
    }
    /// Crée un hub avec une file bornée de `capacity` lots par abonné.
    #[must_use]
    pub fn new(capacity: usize) -> Arc<Self> {
        Arc::new(Self {
            capacity: capacity.max(1),
            next_id: AtomicU64::new(1),
            subscribers: Mutex::new(HashMap::new()),
        })
    }

    fn register(
        &self,
    ) -> (
        u64,
        mpsc::Receiver<Vec<EventEnvelope>>,
        Arc<tokio::sync::Notify>,
    ) {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = mpsc::channel(self.capacity);
        let abort_flag = Arc::new(tokio::sync::Notify::new());
        let mut guard = self
            .subscribers
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        guard.insert(
            id,
            SubscriberEntry {
                sender: tx,
                abort_flag: Arc::clone(&abort_flag),
            },
        );
        (id, rx, abort_flag)
    }

    fn unregister(&self, id: u64) {
        let mut guard = self
            .subscribers
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        guard.remove(&id);
    }

    /// Diffuse de façon non bloquante un lot d'événements engagés à tous les abonnés actifs.
    ///
    /// Tout abonné lent dont la file bornée sature (`TrySendError::Full`) est immédiatement
    /// déconnecté sans jamais bloquer le writer ni croître en mémoire (`T-F001-009`).
    pub fn publish_events(&self, events: &[EventEnvelope]) {
        if events.is_empty() {
            return;
        }
        let mut to_remove = Vec::new();
        {
            let guard = self
                .subscribers
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            for (&id, entry) in &*guard {
                match entry.sender.try_send(events.to_vec()) {
                    Ok(()) => {}
                    Err(TrySendError::Full(_)) => {
                        tracing::warn!(
                            subscriber_id = id,
                            capacity = self.capacity,
                            "abonné SubscribeEvents saturé (BACKPRESSURE) : coupure du flux"
                        );
                        entry.abort_flag.notify_waiters();
                        to_remove.push(id);
                    }
                    Err(TrySendError::Closed(_)) => {
                        to_remove.push(id);
                    }
                }
            }
        }
        if !to_remove.is_empty() {
            let mut guard = self
                .subscribers
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            for id in to_remove {
                guard.remove(&id);
            }
        }
    }

    /// Ferme proprement les canaux d'abonnement après diffusion des derniers événements d'arrêt.
    pub fn close_all(&self) {
        let mut guard = self
            .subscribers
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        guard.clear();
    }
}

/// An internal wake-up observes the existing outbox stream, never grants admission.
pub(crate) struct InternalEventSubscription {
    hub: Arc<SubscriptionHub>,
    id: u64,
    receiver: mpsc::Receiver<Vec<EventEnvelope>>,
    aborted: Arc<tokio::sync::Notify>,
}

impl InternalEventSubscription {
    pub(crate) async fn changed(&mut self) -> Result<(), ()> {
        if self.receiver.is_closed() {
            return Err(());
        }
        tokio::select! {
            biased;
            ()=self.aborted.notified()=>Err(()),
            events=self.receiver.recv()=>events.map(|_|()).ok_or(()),
        }
    }
}

impl Drop for InternalEventSubscription {
    fn drop(&mut self) {
        self.hub.unregister(self.id);
    }
}

/// Exécute la boucle d'envoi `SubscribeEvents` sur `stream` depuis le curseur demandé.
pub async fn run_subscription_stream<S: std::hash::BuildHasher>(
    stream: &mut UnixStream,
    req: SubscribeEventsRequest,
    allowed_projects: Option<HashSet<ProjectId, S>>,
    store: Arc<UnleashStore>,
    hub: Arc<SubscriptionHub>,
    write_timeout: Duration,
) {
    if req.batch_limit == 0 || req.batch_limit > 10_000 {
        let err = IpcError::new(
            ErrorCode::InvalidArgument,
            None,
            "batch_limit doit être compris entre 1 et 10000",
            RetryClass::Never,
        );
        let _ = write_json_frame_timeout(stream, &err, write_timeout).await;
        return;
    }

    // Enregistrer l'abonné AVANT le rattrapage pour ne manquer aucun événement concurrent.
    let (sub_id, mut rx, abort_flag) = hub.register();

    let Some(mut sent_after) = catch_up_replay(
        stream,
        &req,
        allowed_projects.as_ref(),
        &store,
        write_timeout,
    )
    .await
    else {
        hub.unregister(sub_id);
        return;
    };

    stream_live_batches(
        stream,
        &req,
        allowed_projects.as_ref(),
        &store,
        &mut rx,
        &abort_flag,
        &mut sent_after,
        write_timeout,
    )
    .await;

    hub.unregister(sub_id);
}

async fn catch_up_replay<S: std::hash::BuildHasher>(
    stream: &mut UnixStream,
    req: &SubscribeEventsRequest,
    allowed_projects: Option<&HashSet<ProjectId, S>>,
    store: &Arc<UnleashStore>,
    write_timeout: Duration,
) -> Option<u64> {
    let mut sent_after = req.after_sequence.as_u64();
    let batch_limit = req.batch_limit;

    loop {
        let stream_id = req.stream_id;
        let store_ref = Arc::clone(store);
        let res = tokio::task::spawn_blocking(move || {
            store_ref.read_events(stream_id, Sequence::new(sent_after), batch_limit, None)
        })
        .await;

        let outcome = match res {
            Ok(Ok(o)) => o,
            Ok(Err(err)) => {
                let ipc = err.to_ipc_error();
                let _ = write_json_frame_timeout(stream, &ipc, write_timeout).await;
                return None;
            }
            Err(_) => return None,
        };

        match outcome {
            ReadEventsOutcome::ResyncRequired {
                stream_id,
                current_watermark,
                reason,
            } => {
                let frame = SubscribeEventsFrame::ResyncRequired {
                    stream_id,
                    current_watermark,
                    reason,
                };
                let _ = write_json_frame_timeout(stream, &frame, write_timeout).await;
                return None;
            }
            ReadEventsOutcome::Events { events, cursor } => {
                if events.is_empty() {
                    return Some(sent_after.max(cursor.sequence.as_u64()));
                }
                let max_scanned = events
                    .iter()
                    .map(|e| e.sequence.as_u64())
                    .max()
                    .unwrap_or(sent_after);
                let filtered: Vec<EventEnvelope> = events
                    .into_iter()
                    .filter(|ev| matches_filter(ev, req, allowed_projects))
                    .collect();
                sent_after = sent_after.max(max_scanned);
                if !filtered.is_empty() {
                    let frame = SubscribeEventsFrame::Events {
                        events: filtered,
                        cursor: EventCursor::new(req.stream_id, Sequence::new(sent_after)),
                    };
                    if write_json_frame_timeout(stream, &frame, write_timeout)
                        .await
                        .is_err()
                    {
                        return None;
                    }
                }
            }
        }
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "Boucle de diffusion live recevant flux Unix, filtre scope/requête, store, canal et curseur"
)]
async fn stream_live_batches<S: std::hash::BuildHasher>(
    stream: &mut UnixStream,
    req: &SubscribeEventsRequest,
    allowed_projects: Option<&HashSet<ProjectId, S>>,
    store: &Arc<UnleashStore>,
    rx: &mut mpsc::Receiver<Vec<EventEnvelope>>,
    abort_flag: &tokio::sync::Notify,
    sent_after: &mut u64,
    write_timeout: Duration,
) {
    loop {
        tokio::select! {
            biased;
            () = abort_flag.notified() => {
                return;
            }
            maybe_batch = rx.recv() => {
                let Some(batch) = maybe_batch else {
                    return;
                };
                let Ok(filtered) = filter_live_batch(
                    stream,
                    req,
                    allowed_projects,
                    store,
                    batch,
                    sent_after,
                    write_timeout,
                )
                .await
                else {
                    return;
                };
                for chunk in filtered.chunks(req.batch_limit as usize) {
                    let chunk_max = chunk
                        .iter()
                        .map(|e| e.sequence.as_u64())
                        .max()
                        .unwrap_or(*sent_after);
                    let frame = SubscribeEventsFrame::Events {
                        events: chunk.to_vec(),
                        cursor: EventCursor::new(req.stream_id, Sequence::new(chunk_max)),
                    };
                    let send_fut = write_json_frame_timeout(stream, &frame, write_timeout);
                    tokio::select! {
                        biased;
                        () = abort_flag.notified() => return,
                        res = send_fut => {
                            if res.is_err() {
                                return;
                            }
                        }
                    }
                }
            }
        }
    }
}

async fn filter_live_batch<S: std::hash::BuildHasher>(
    stream: &mut UnixStream,
    req: &SubscribeEventsRequest,
    allowed_projects: Option<&HashSet<ProjectId, S>>,
    store: &Arc<UnleashStore>,
    mut batch: Vec<EventEnvelope>,
    sent_after: &mut u64,
    write_timeout: Duration,
) -> Result<Vec<EventEnvelope>, ()> {
    batch.sort_by_key(|ev| ev.sequence.as_u64());

    let mut filtered = Vec::new();
    let mut highest = *sent_after;
    for ev in batch {
        if ev.stream_id != req.stream_id {
            emit_resync_if_needed(stream, store, req.stream_id, *sent_after, write_timeout).await;
            return Err(());
        }
        let seq = ev.sequence.as_u64();
        if seq <= highest {
            continue;
        }
        if seq > highest.saturating_add(1) {
            catch_up_gap_before_sequence(
                stream,
                req,
                allowed_projects,
                store,
                &mut highest,
                seq,
                &mut filtered,
                write_timeout,
            )
            .await?;
            if seq <= highest {
                continue;
            }
        }
        highest = highest.max(seq);
        if matches_filter(&ev, req, allowed_projects) {
            filtered.push(ev);
        }
    }
    *sent_after = highest;
    Ok(filtered)
}

async fn emit_resync_if_needed(
    stream: &mut UnixStream,
    store: &Arc<UnleashStore>,
    stream_id: unleash_domain::StreamId,
    after_seq: u64,
    write_timeout: Duration,
) {
    let store_ref = Arc::clone(store);
    let outcome = tokio::task::spawn_blocking(move || {
        store_ref.read_events(stream_id, Sequence::new(after_seq), 1, None)
    })
    .await;
    if let Ok(Ok(ReadEventsOutcome::ResyncRequired {
        stream_id,
        current_watermark,
        reason,
    })) = outcome
    {
        let frame = SubscribeEventsFrame::ResyncRequired {
            stream_id,
            current_watermark,
            reason,
        };
        let _ = write_json_frame_timeout(stream, &frame, write_timeout).await;
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "Rattrapage ciblé de trou de séquence hors thread Tokio avant émission live"
)]
async fn catch_up_gap_before_sequence<S: std::hash::BuildHasher>(
    stream: &mut UnixStream,
    req: &SubscribeEventsRequest,
    allowed_projects: Option<&HashSet<ProjectId, S>>,
    store: &Arc<UnleashStore>,
    highest: &mut u64,
    target_seq: u64,
    out_filtered: &mut Vec<EventEnvelope>,
    write_timeout: Duration,
) -> Result<(), ()> {
    while *highest < target_seq.saturating_sub(1) {
        let after = *highest;
        let stream_id = req.stream_id;
        let span = target_seq.saturating_sub(after).clamp(1, 256);
        let limit = u32::try_from(span).unwrap_or(256);
        let store_ref = Arc::clone(store);
        let outcome = tokio::task::spawn_blocking(move || {
            store_ref.read_events(stream_id, Sequence::new(after), limit, None)
        })
        .await
        .map_err(|_| ())?
        .map_err(|_| ())?;

        match outcome {
            ReadEventsOutcome::ResyncRequired {
                stream_id,
                current_watermark,
                reason,
            } => {
                let frame = SubscribeEventsFrame::ResyncRequired {
                    stream_id,
                    current_watermark,
                    reason,
                };
                let _ = write_json_frame_timeout(stream, &frame, write_timeout).await;
                return Err(());
            }
            ReadEventsOutcome::Events { events, .. } => {
                if events.is_empty() {
                    break;
                }
                let mut progressed = false;
                for gev in events {
                    let gseq = gev.sequence.as_u64();
                    if gseq <= *highest {
                        continue;
                    }
                    if gseq >= target_seq {
                        return Ok(());
                    }
                    *highest = gseq;
                    progressed = true;
                    if matches_filter(&gev, req, allowed_projects) {
                        out_filtered.push(gev);
                    }
                }
                if !progressed {
                    break;
                }
            }
        }
    }
    Ok(())
}

fn matches_filter<S: std::hash::BuildHasher>(
    ev: &EventEnvelope,
    req: &SubscribeEventsRequest,
    allowed_projects: Option<&HashSet<ProjectId, S>>,
) -> bool {
    if let Some(allowed) = allowed_projects
        && ev.project_id.is_none_or(|pid| !allowed.contains(&pid))
    {
        return false;
    }
    if let Some(proj) = req.project_id
        && ev.project_id != Some(proj)
    {
        return false;
    }
    if let Some(agg) = req.aggregate_id
        && ev.aggregate_id != agg
    {
        return false;
    }
    true
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests {
    use serde_json::Map;
    use tokio::io::AsyncReadExt;
    use unleash_domain::{ActorId, CommandId, RequestId, Revision, Sha256Digest, StreamId};
    use unleash_protocol::{
        CommandEnvelope, EventCausation, EventCausationKind, ProtocolVersion, ReceiptState,
        Sensitivity, from_json_str_strict,
    };
    use unleash_store::{
        AggregateKind, CommandMutation, CommitCommandRequest, EventAppendSpec, StoreConfig,
    };
    use unleash_testkit::local_test_dir;
    use uuid::Uuid;

    use super::*;

    fn mk_event(
        stream_id: StreamId,
        seq: u64,
        project_id: Option<ProjectId>,
        agg: Uuid,
    ) -> EventEnvelope {
        EventEnvelope {
            event_id: Uuid::from_u128(u128::from(seq) + 1000),
            stream_id,
            sequence: Sequence::new(seq),
            event_type: String::from("ProjectCreated"),
            schema_version: 1,
            occurred_at: chrono::DateTime::UNIX_EPOCH,
            recorded_at: chrono::DateTime::UNIX_EPOCH,
            project_id,
            aggregate_id: agg,
            aggregate_revision: Revision::new(1),
            actor_ref: String::from("uid:1"),
            correlation_id: Uuid::from_u128(1),
            causation: EventCausation {
                kind: EventCausationKind::Command,
                id: String::from("1"),
            },
            command_id: None,
            payload_hash: Sha256Digest::from_bytes(b"{}"),
            payload: Map::new(),
            sensitivity: Sensitivity::Project,
        }
    }

    #[test]
    fn matches_filter_enforces_allowed_projects_and_explicit_filters() {
        let sid = StreamId::from_u128(1);
        let p1 = ProjectId::from_u128(0x1);
        let p2 = ProjectId::from_u128(0x2);
        let agg1 = Uuid::from_u128(0xA1);
        let mut allowed = HashSet::new();
        allowed.insert(p1);

        let req_all = SubscribeEventsRequest {
            stream_id: sid,
            after_sequence: Sequence::new(0),
            project_id: None,
            aggregate_id: None,
            batch_limit: 16,
        };

        assert!(matches_filter(
            &mk_event(sid, 1, Some(p1), agg1),
            &req_all,
            Some(&allowed)
        ));
        assert!(!matches_filter(
            &mk_event(sid, 2, Some(p2), agg1),
            &req_all,
            Some(&allowed)
        ));
        assert!(!matches_filter(
            &mk_event(sid, 3, None, agg1),
            &req_all,
            Some(&allowed)
        ));
        assert!(matches_filter(
            &mk_event(sid, 4, None, agg1),
            &req_all,
            None::<&HashSet<ProjectId>>
        ));

        let req_proj = SubscribeEventsRequest {
            project_id: Some(p2),
            aggregate_id: Some(agg1),
            ..req_all
        };
        assert!(matches_filter(
            &mk_event(sid, 5, Some(p2), agg1),
            &req_proj,
            None::<&HashSet<ProjectId>>
        ));
        assert!(!matches_filter(
            &mk_event(sid, 6, Some(p2), Uuid::from_u128(0xA2)),
            &req_proj,
            None::<&HashSet<ProjectId>>
        ));
    }

    #[test]
    fn subscription_hub_backpressure_disconnects_slow_subscriber_only() {
        let hub = SubscriptionHub::new(1);
        let (_id_slow, mut rx_slow, abort_slow) = hub.register();
        let (_id_fast, mut rx_fast, _abort_fast) = hub.register();
        let sid = StreamId::from_u128(7);

        hub.publish_events(&[mk_event(sid, 1, None, Uuid::from_u128(1))]);
        let ev_fast_1 = rx_fast.try_recv().unwrap();
        assert_eq!(ev_fast_1[0].sequence, Sequence::new(1));

        // La file de slow (capacité 1) déborde au second publish, fast reçoit normalement.
        hub.publish_events(&[mk_event(sid, 2, None, Uuid::from_u128(2))]);
        let ev_fast_2 = rx_fast.try_recv().unwrap();
        assert_eq!(ev_fast_2[0].sequence, Sequence::new(2));

        // Purge du buffer résiduel de slow puis vérification que hub l'a désenregistré.
        let _ = rx_slow.try_recv();
        drop(abort_slow);
        hub.publish_events(&[mk_event(sid, 3, None, Uuid::from_u128(3))]);
        let ev_fast_3 = rx_fast.try_recv().unwrap();
        assert_eq!(ev_fast_3[0].sequence, Sequence::new(3));
        assert!(rx_slow.try_recv().is_err());
    }

    fn create_two_test_projects_store(
        dir_path: &std::path::Path,
    ) -> (Arc<UnleashStore>, StreamId, EventEnvelope, EventEnvelope) {
        let store = Arc::new(UnleashStore::open(StoreConfig::new(dir_path)).unwrap());
        store.register_principal("uid:1", "user", "uid:1").unwrap();
        let d = store.daemon_state().unwrap();
        let auth = d.authority_generation;
        let sid = d.stream_id;

        for idx in 1_u128..=2_u128 {
            let pid = ProjectId::from_u128(idx);
            store
                .commit_command(CommitCommandRequest {
                    actor_id: ActorId::new("uid:1").unwrap(),
                    envelope: CommandEnvelope {
                        protocol_version: ProtocolVersion,
                        request_id: RequestId::from_u128(idx),
                        command_id: CommandId::from_u128(idx + 100),
                        authority_generation: auth,
                        actor_ref: None,
                        project_id: Some(pid),
                        aggregate_id: pid.as_uuid(),
                        expected_revision: Revision::INITIAL,
                        command_type: String::from("CreateProject"),
                        payload: Map::new(),
                    },
                    target_aggregate: AggregateKind::Project,
                    operation_id: None,
                    receipt_state: ReceiptState::Accepted,
                    result_ref: None,
                    retention_days_override: None,
                    fence_assertion: None,
                    mutation: CommandMutation::CreateProject {
                        project_id: pid,
                        name: format!("P-{idx}"),
                        weight: 1,
                    },
                    events: vec![EventAppendSpec {
                        event_type: String::from("ProjectCreated"),
                        aggregate_type: AggregateKind::Project,
                        aggregate_id: pid.as_uuid(),
                        aggregate_revision: Revision::new(1),
                        project_id: Some(pid),
                        correlation_id: pid.as_uuid(),
                        causation_kind: EventCausationKind::Command,
                        causation_id: None,
                        payload: Map::new(),
                        sensitivity: Sensitivity::Project,
                        outbox_destinations: Vec::new(),
                    }],
                    capacity_reservations: Vec::new(),
                    budget_reservations: Vec::new(),
                })
                .unwrap();
        }
        let ReadEventsOutcome::Events { events, .. } =
            store.read_events(sid, Sequence::new(0), 16, None).unwrap()
        else {
            panic!("events attendus");
        };
        (store, sid, events[0].clone(), events[1].clone())
    }

    #[tokio::test]
    async fn out_of_order_live_batches_recover_missing_sequence_without_loss() {
        let dir = local_test_dir("sub-ooo").unwrap();
        let dir_path = dir.path().to_path_buf();
        let (store, stream_id, ev1, ev2) =
            tokio::task::spawn_blocking(move || create_two_test_projects_store(&dir_path))
                .await
                .unwrap();

        let (mut server_stream, mut client_stream) = UnixStream::pair().unwrap();
        let hub = SubscriptionHub::new(8);
        let req = SubscribeEventsRequest {
            stream_id,
            after_sequence: Sequence::new(0),
            project_id: None,
            aggregate_id: None,
            batch_limit: 16,
        };

        let (_, mut rx, abort_flag) = hub.register();
        let mut sent_after = 0_u64;
        // Injecte délibérément ev2 (seq=2) AVANT ev1 (seq=1) pour vérifier le rattrapage sans perte.
        hub.publish_events(&[ev2]);
        hub.publish_events(&[ev1]);

        let server_task = tokio::spawn(async move {
            stream_live_batches(
                &mut server_stream,
                &req,
                None::<&HashSet<ProjectId>>,
                &store,
                &mut rx,
                &abort_flag,
                &mut sent_after,
                Duration::from_secs(2),
            )
            .await;
            let _ = tokio::task::spawn_blocking(move || drop(store)).await;
        });

        let mut hdr = [0u8; 4];
        client_stream.read_exact(&mut hdr).await.unwrap();
        let len = u32::from_be_bytes(hdr) as usize;
        let mut body = vec![0u8; len];
        client_stream.read_exact(&mut body).await.unwrap();
        let frame =
            from_json_str_strict::<SubscribeEventsFrame>(std::str::from_utf8(&body).unwrap())
                .unwrap();

        match frame {
            SubscribeEventsFrame::Events { events, .. } => {
                let seqs: Vec<u64> = events.iter().map(|e| e.sequence.as_u64()).collect();
                assert_eq!(seqs, vec![1, 2]);
            }
            other => panic!("trame inattendue : {other:?}"),
        }

        hub.close_all();
        let _ = server_task.await;
    }
}
