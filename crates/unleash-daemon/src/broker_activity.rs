//! Admission and bounded shutdown of in-process broker work.

use std::sync::{Arc, Mutex, PoisonError};
use tokio::sync::watch;

#[derive(Default)]
struct State {
    closed: bool,
    active: usize,
}

pub(crate) struct BrokerActivity {
    state: Mutex<State>,
    active: watch::Sender<usize>,
}

impl BrokerActivity {
    pub(crate) fn new() -> Arc<Self> {
        let (active, _) = watch::channel(0);
        Arc::new(Self {
            state: Mutex::new(State::default()),
            active,
        })
    }

    pub(crate) fn admit(self: &Arc<Self>) -> Option<BrokerActivityGuard> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.closed {
            return None;
        }
        state.active += 1;
        self.active.send_replace(state.active);
        Some(BrokerActivityGuard {
            activity: Arc::clone(self),
        })
    }

    pub(crate) async fn close_and_wait(&self) {
        {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            state.closed = true;
        }
        let mut active = self.active.subscribe();
        loop {
            if *active.borrow_and_update() == 0 {
                return;
            }
            if active.changed().await.is_err() {
                return;
            }
        }
    }
}

pub(crate) struct BrokerActivityGuard {
    activity: Arc<BrokerActivity>,
}

impl Drop for BrokerActivityGuard {
    fn drop(&mut self) {
        let mut state = self
            .activity
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        state.active -= 1;
        self.activity.active.send_replace(state.active);
    }
}
