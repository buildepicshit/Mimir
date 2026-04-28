//! Processor observability integration tests.

use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use mimir_core::ClockTime;
use mimir_librarian::{
    Draft, DraftMetadata, DraftProcessingDecision, DraftProcessor, DraftSourceSurface,
    LibrarianError, LlmInvoker, RetryingDraftProcessor,
};
use tracing::field::{Field, Visit};
use tracing::Subscriber;
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::Layer;

static CAPTURE_LOCK: Mutex<()> = Mutex::new(());

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
    fn as_u64(&self) -> Option<u64> {
        if let Self::U64(value) = self {
            Some(*value)
        } else {
            None
        }
    }

    fn as_str(&self) -> Option<&str> {
        if let Self::Str(value) = self {
            Some(value.as_str())
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
struct CapturedSpan {
    name: String,
    fields: HashMap<String, FieldValue>,
}

#[derive(Default, Clone)]
struct CaptureShared {
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
            span_ref.extensions_mut().insert(collector);
        }
    }

    fn on_record(&self, id: &tracing::Id, values: &tracing::span::Record<'_>, ctx: Context<'_, S>) {
        if let Some(span_ref) = ctx.span(id) {
            let mut extensions = span_ref.extensions_mut();
            if let Some(collector) = extensions.get_mut::<FieldCollector>() {
                values.record(collector);
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
            if let Ok(mut spans) = self.shared.spans.lock() {
                spans.push(CapturedSpan {
                    name: span_ref.name().to_string(),
                    fields,
                });
            }
        }
    }
}

fn capture<F: FnOnce()>(f: F) -> CaptureShared {
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

#[derive(Debug, Clone)]
struct SequenceInvoker {
    responses: Arc<Mutex<VecDeque<String>>>,
}

impl SequenceInvoker {
    fn new(responses: impl IntoIterator<Item = &'static str>) -> Self {
        Self {
            responses: Arc::new(Mutex::new(
                responses.into_iter().map(str::to_string).collect(),
            )),
        }
    }
}

impl LlmInvoker for SequenceInvoker {
    fn invoke(&self, _system_prompt: &str, _user_message: &str) -> Result<String, LibrarianError> {
        let mut responses =
            self.responses
                .lock()
                .map_err(|err| LibrarianError::LlmInvocationFailed {
                    message: format!("responses lock poisoned: {err}"),
                })?;
        responses
            .pop_front()
            .ok_or_else(|| LibrarianError::LlmInvocationFailed {
                message: "no canned response left".to_string(),
            })
    }
}

fn fixed_now() -> Result<ClockTime, mimir_core::ClockTimeError> {
    ClockTime::try_from_millis(1_713_350_400_000)
}

fn draft(text: &str) -> Draft {
    Draft::with_metadata(
        text.to_string(),
        DraftMetadata::new(DraftSourceSurface::Cli, SystemTime::UNIX_EPOCH),
    )
}

#[test]
fn process_emits_retry_and_record_metrics() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let log_path = tmp.path().join("canonical.log");
    let invoker = SequenceInvoker::new([
        r#"{"records":[{"kind":"sem","lisp":"(sem @alice @knows @bob :src @agent_instruction :c 1.0 :v 2024-01-15)"}],"notes":"bad"}"#,
        r#"{"records":[{"kind":"sem","lisp":"(sem @alice @knows @bob :src @agent_instruction :c 0.95 :v 2024-01-15)"},{"kind":"sem","lisp":"(sem @carol @knows @dave :src @observation :c 0.8 :v 2024-01-15)"}],"notes":"fixed"}"#,
    ]);
    let mut processor = RetryingDraftProcessor::new_at(invoker, 3, fixed_now()?, &log_path)?;
    let mut process_result = None;

    let shared = capture(|| {
        process_result =
            Some(processor.process(&draft("Alice has a policy about Bob. Carol knows Dave.")));
    });
    let decision = match process_result {
        Some(Ok(decision)) => decision,
        Some(Err(err)) => return Err(Box::new(err)),
        None => return Err("processor did not execute".into()),
    };
    assert_eq!(decision, DraftProcessingDecision::Accepted);

    let spans = shared
        .spans
        .lock()
        .map_err(|err| format!("spans lock poisoned: {err}"))?;
    let Some(span) = spans.iter().find(|span| {
        span.name == "mimir.librarian.process"
            && span.fields.get("attempts").and_then(FieldValue::as_u64) == Some(2)
    }) else {
        return Err("processor span missing".into());
    };
    assert_eq!(
        span.fields.get("retries").and_then(FieldValue::as_u64),
        Some(1),
    );
    assert_eq!(
        span.fields
            .get("response_records")
            .and_then(FieldValue::as_u64),
        Some(3),
    );
    assert_eq!(
        span.fields
            .get("validated_records")
            .and_then(FieldValue::as_u64),
        Some(2),
    );
    assert_eq!(
        span.fields
            .get("committed_records")
            .and_then(FieldValue::as_u64),
        Some(2),
    );
    assert_eq!(
        span.fields.get("decision").and_then(FieldValue::as_str),
        Some("accepted"),
    );
    assert_eq!(
        span.fields
            .get("last_error_classification")
            .and_then(FieldValue::as_str),
        Some("semantic"),
    );
    Ok(())
}
