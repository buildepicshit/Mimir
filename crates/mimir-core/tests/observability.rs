//! Integration tests for `docs/observability.md`: assert spans and
//! events emit the documented target + field schema. Structural
//! assertions (typed fields), never string comparison on formatted
//! messages — PRINCIPLES.md § 5 "structured, not formatted".

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};

use mimir_core::pipeline::Pipeline;
use mimir_core::ClockTime;
use tracing::field::{Field, Visit};
use tracing::Subscriber;
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::Layer;

#[derive(Debug, Clone)]
#[allow(dead_code)]
enum FieldValue {
    Str(String),
    U64(u64),
    I64(i64),
    Bool(bool),
    Debug(String),
}

impl FieldValue {
    fn as_str(&self) -> Option<&str> {
        match self {
            FieldValue::Str(s) | FieldValue::Debug(s) => Some(s.as_str()),
            _ => None,
        }
    }

    fn as_u64(&self) -> Option<u64> {
        if let FieldValue::U64(v) = self {
            Some(*v)
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
struct CapturedEvent {
    target: String,
    fields: HashMap<String, FieldValue>,
}

#[derive(Debug, Clone)]
struct CapturedSpan {
    name: String,
    fields: HashMap<String, FieldValue>,
}

#[derive(Default, Clone)]
struct CaptureShared {
    events: Arc<Mutex<Vec<CapturedEvent>>>,
    spans: Arc<Mutex<Vec<CapturedSpan>>>,
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
            let mut exts = span_ref.extensions_mut();
            exts.insert(collector);
        }
    }

    fn on_record(&self, id: &tracing::Id, values: &tracing::span::Record<'_>, ctx: Context<'_, S>) {
        if let Some(span_ref) = ctx.span(id) {
            let mut exts = span_ref.extensions_mut();
            if let Some(collector) = exts.get_mut::<FieldCollector>() {
                values.record(collector);
            }
        }
    }

    fn on_close(&self, id: tracing::Id, ctx: Context<'_, S>) {
        if let Some(span_ref) = ctx.span(&id) {
            let name = span_ref.name().to_string();
            let fields = span_ref
                .extensions()
                .get::<FieldCollector>()
                .map(|c| c.0.clone())
                .unwrap_or_default();
            self.shared
                .spans
                .lock()
                .unwrap()
                .push(CapturedSpan { name, fields });
        }
    }

    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        let mut collector = FieldCollector::default();
        event.record(&mut collector);
        self.shared.events.lock().unwrap().push(CapturedEvent {
            target: event.metadata().target().to_string(),
            fields: collector.0,
        });
    }
}

fn capture<F: FnOnce()>(f: F) -> CaptureShared {
    let shared = CaptureShared::default();
    let layer = CaptureLayer {
        shared: shared.clone(),
    };
    let subscriber = tracing_subscriber::registry().with(layer);
    tracing::subscriber::with_default(subscriber, f);
    shared
}

#[test]
fn compile_batch_span_carries_record_counts() {
    let shared = capture(|| {
        let mut pipe = Pipeline::new();
        let input = "(sem @alice knows @bob :src @profile :c 0.9 :v 2023-01-01)";
        let now = ClockTime::try_from_millis(1_700_000_000_000).unwrap();
        let records = pipe.compile_batch(input, now).expect("compile");
        assert!(!records.is_empty());
    });

    let spans = shared.spans.lock().unwrap();
    let span = spans
        .iter()
        .find(|s| s.name == "mimir.pipeline.compile_batch")
        .expect("compile_batch span");
    assert!(
        span.fields
            .get("input_len")
            .and_then(FieldValue::as_u64)
            .unwrap_or(0)
            > 0,
        "input_len should be the byte length of the input",
    );
    // record_count includes SymbolAlloc records (new symbols) plus the
    // memory record. memory_count isolates the latter.
    assert!(
        span.fields
            .get("record_count")
            .and_then(FieldValue::as_u64)
            .unwrap_or(0)
            >= 1,
    );
    assert_eq!(
        span.fields.get("memory_count").and_then(FieldValue::as_u64),
        Some(1),
        "single sem form produces exactly one memory record",
    );
    assert_eq!(
        span.fields.get("edge_count").and_then(FieldValue::as_u64),
        Some(0),
    );
}

#[test]
fn semantic_forward_supersession_emits_event_with_identifiers_only() {
    let shared = capture(|| {
        let mut pipe = Pipeline::new();
        // First write at valid_at = T1.
        let first = "(sem @alice knows @bob :src @profile :c 0.9 :v 2023-01-01)";
        pipe.compile_batch(
            first,
            ClockTime::try_from_millis(1_700_000_000_000).unwrap(),
        )
        .expect("first compile");
        // Second write at valid_at = T2 > T1 on same (s, p) => forward supersession.
        let second = "(sem @alice knows @carol :src @profile :c 0.9 :v 2023-02-01)";
        pipe.compile_batch(
            second,
            ClockTime::try_from_millis(1_700_000_000_001).unwrap(),
        )
        .expect("second compile");
    });

    let events = shared.events.lock().unwrap();
    let sup = events
        .iter()
        .find(|e| e.target == "mimir.supersession")
        .expect("supersession event fired");
    assert_eq!(
        sup.fields.get("kind").and_then(FieldValue::as_str),
        Some("semantic"),
    );
    assert_eq!(
        sup.fields.get("direction").and_then(FieldValue::as_str),
        Some("forward"),
    );
    // Identifier fields must be present. We don't hard-code the actual
    // SymbolId value because memory-id allocation is internal, but we
    // do assert the fields exist and the privacy invariant holds
    // (no `o` / `trigger` / payload fields leak in).
    for required in ["s", "p", "old_memory_id", "new_memory_id"] {
        assert!(
            sup.fields.contains_key(required),
            "supersession event missing `{required}` field; got {:?}",
            sup.fields.keys().collect::<Vec<_>>(),
        );
    }
    for forbidden in ["o", "trigger", "action", "precondition", "label"] {
        assert!(
            !sup.fields.contains_key(forbidden),
            "supersession event leaked payload field `{forbidden}` — PRINCIPLES.md § 5 privacy violation",
        );
    }
}
