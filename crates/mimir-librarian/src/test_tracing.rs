use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};

use tracing::field::{Field, Visit};
use tracing::Subscriber;
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::Layer;

static CAPTURE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) enum FieldValue {
    Str(String),
    U64(u64),
    I64(i64),
    Bool(bool),
    Debug(String),
}

impl FieldValue {
    pub(crate) fn as_u64(&self) -> Option<u64> {
        if let Self::U64(value) = self {
            Some(*value)
        } else {
            None
        }
    }
}

#[derive(Default)]
struct FieldCollector(HashMap<String, FieldValue>);

impl Visit for FieldCollector {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.0
            .insert(field.name().to_string(), FieldValue::Str(value.to_string()));
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.0
            .insert(field.name().to_string(), FieldValue::U64(value));
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.0
            .insert(field.name().to_string(), FieldValue::I64(value));
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.0
            .insert(field.name().to_string(), FieldValue::Bool(value));
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.0.insert(
            field.name().to_string(),
            FieldValue::Debug(format!("{value:?}")),
        );
    }
}

#[derive(Debug, Clone)]
pub(crate) struct CapturedSpan {
    pub(crate) name: String,
    pub(crate) fields: HashMap<String, FieldValue>,
}

#[derive(Default, Clone)]
pub(crate) struct CaptureShared {
    pub(crate) spans: Arc<Mutex<Vec<CapturedSpan>>>,
}

impl CaptureShared {
    fn push_span_snapshot(&self, name: String, fields: HashMap<String, FieldValue>) {
        if let Ok(mut spans) = self.spans.lock() {
            spans.push(CapturedSpan { name, fields });
        }
    }
}

struct CaptureLayer {
    shared: CaptureShared,
}

impl<S> Layer<S> for CaptureLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(
        &self,
        attrs: &tracing::span::Attributes<'_>,
        id: &tracing::Id,
        ctx: Context<'_, S>,
    ) {
        let mut collector = FieldCollector::default();
        attrs.record(&mut collector);
        if let Some(span_ref) = ctx.span(id) {
            span_ref.extensions_mut().insert(collector);
        }
    }

    fn on_record(&self, id: &tracing::Id, values: &tracing::span::Record<'_>, ctx: Context<'_, S>) {
        if let Some(span_ref) = ctx.span(id) {
            let name = span_ref.name().to_string();
            let fields = {
                let mut extensions = span_ref.extensions_mut();
                extensions.get_mut::<FieldCollector>().map(|collector| {
                    values.record(collector);
                    collector.0.clone()
                })
            };
            if let Some(fields) = fields {
                self.shared.push_span_snapshot(name, fields);
            }
        }
    }

    fn on_close(&self, id: tracing::Id, ctx: Context<'_, S>) {
        if let Some(span_ref) = ctx.span(&id) {
            let fields = span_ref
                .extensions()
                .get::<FieldCollector>()
                .map(|collector| collector.0.clone())
                .unwrap_or_default();
            self.shared
                .push_span_snapshot(span_ref.name().to_string(), fields);
        }
    }
}

pub(crate) fn capture<F: FnOnce()>(f: F) -> CaptureShared {
    let _lock = match CAPTURE_LOCK.lock() {
        Ok(lock) => lock,
        Err(poisoned) => poisoned.into_inner(),
    };
    let shared = CaptureShared::default();
    let layer = CaptureLayer {
        shared: shared.clone(),
    };
    let subscriber = tracing_subscriber::registry().with(layer);
    tracing::subscriber::with_default(subscriber, || {
        tracing::callsite::rebuild_interest_cache();
        f();
    });
    tracing::callsite::rebuild_interest_cache();
    shared
}
