// SPDX-License-Identifier: Apache-2.0
//! Hub borné d'abonnements `SubscribeEvents` avec backpressure et `ResyncRequired` (`REQ-CORE-009`, `REQ-CORE-010`).

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use tokio::net::UnixStream;
use tokio::sync::mpsc;
use tokio::sync::mpsc::error::TrySendError;
use unleash_domain::Sequence;
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

/// Exécute la boucle d'envoi `SubscribeEvents` sur `stream` depuis le curseur demandé.
pub async fn run_subscription_stream(
    stream: &mut UnixStream,
    req: SubscribeEventsRequest,
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

    let Some(mut sent_after) = catch_up_replay(stream, &req, &store, write_timeout).await else {
        hub.unregister(sub_id);
        return;
    };

    stream_live_batches(
        stream,
        &req,
        &store,
        &mut rx,
        &abort_flag,
        &mut sent_after,
        write_timeout,
    )
    .await;

    hub.unregister(sub_id);
}

async fn catch_up_replay(
    stream: &mut UnixStream,
    req: &SubscribeEventsRequest,
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
                    .filter(|ev| matches_filter(ev, req))
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

async fn stream_live_batches(
    stream: &mut UnixStream,
    req: &SubscribeEventsRequest,
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
                let Ok(filtered) = filter_live_batch(stream, req, store, batch, sent_after, write_timeout).await else {
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

async fn filter_live_batch(
    stream: &mut UnixStream,
    req: &SubscribeEventsRequest,
    store: &Arc<UnleashStore>,
    batch: Vec<EventEnvelope>,
    sent_after: &mut u64,
    write_timeout: Duration,
) -> Result<Vec<EventEnvelope>, ()> {
    let mut filtered = Vec::new();
    let mut highest = *sent_after;
    for ev in batch {
        if ev.stream_id != req.stream_id {
            if let Ok(ReadEventsOutcome::ResyncRequired {
                stream_id,
                current_watermark,
                reason,
            }) = store.read_events(req.stream_id, Sequence::new(*sent_after), 1, None)
            {
                let frame = SubscribeEventsFrame::ResyncRequired {
                    stream_id,
                    current_watermark,
                    reason,
                };
                let _ = write_json_frame_timeout(stream, &frame, write_timeout).await;
            }
            return Err(());
        }
        if ev.sequence.as_u64() <= *sent_after {
            continue;
        }
        highest = highest.max(ev.sequence.as_u64());
        if matches_filter(&ev, req) {
            filtered.push(ev);
        }
    }
    *sent_after = highest;
    Ok(filtered)
}

fn matches_filter(ev: &EventEnvelope, req: &SubscribeEventsRequest) -> bool {
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
