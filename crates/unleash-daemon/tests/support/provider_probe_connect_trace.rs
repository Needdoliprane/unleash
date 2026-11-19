//! Diagnostic subscriber limited to the connect service's closed stage/code fields.

use std::{
    collections::BTreeSet,
    sync::{Mutex, OnceLock},
    time::Duration,
};
use uuid::Uuid;

#[derive(Default)]
struct CapacityWaits {
    probes: Mutex<BTreeSet<Uuid>>,
    changed: tokio::sync::Notify,
}

fn capacity_waits() -> &'static CapacityWaits {
    static WAITS: OnceLock<CapacityWaits> = OnceLock::new();
    WAITS.get_or_init(CapacityWaits::default)
}

pub(super) async fn wait_for_capacity_blocked(probe: Uuid) {
    let waits = capacity_waits();
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let notified = waits.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if waits.probes.lock().unwrap().contains(&probe) {
                return;
            }
            notified.await;
        }
    })
    .await
    .unwrap();
}

#[derive(Default)]
struct Fields {
    log: String,
    capacity_blocked: bool,
    probe: Option<Uuid>,
}

impl tracing::field::Visit for Fields {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "stage" && value == "v008_capacity_blocked" {
            self.capacity_blocked = true;
        }
        self.record_debug(field, &value);
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        use std::fmt::Write;
        let _ = write!(self.log, " {}={value:?}", field.name());
        if field.name() == "probe_id" {
            self.probe = Uuid::parse_str(&format!("{value:?}")).ok();
        }
    }
}

struct ConnectTrace;
impl tracing::Subscriber for ConnectTrace {
    fn enabled(&self, metadata: &tracing::Metadata<'_>) -> bool {
        metadata
            .target()
            .starts_with("unleash_daemon::provider_probe_connect_service")
    }
    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn event(&self, event: &tracing::Event<'_>) {
        let mut fields = Fields::default();
        event.record(&mut fields);
        if fields.capacity_blocked
            && let Some(probe) = fields.probe
        {
            let waits = capacity_waits();
            waits.probes.lock().unwrap().insert(probe);
            waits.changed.notify_waiters();
        }
        eprintln!("connect_trace{}", fields.log);
    }
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
}

pub(super) fn install() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let _ = tracing::subscriber::set_global_default(ConnectTrace);
    });
}
