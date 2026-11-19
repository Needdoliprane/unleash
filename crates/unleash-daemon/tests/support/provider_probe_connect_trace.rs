//! Diagnostic subscriber limited to the connect service's closed stage/code fields.

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
        struct Fields(String);
        impl tracing::field::Visit for Fields {
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                use std::fmt::Write;
                let _ = write!(self.0, " {}={value:?}", field.name());
            }
        }
        let mut fields = Fields(String::new());
        event.record(&mut fields);
        eprintln!("connect_trace{}", fields.0);
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
