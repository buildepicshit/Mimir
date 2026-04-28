//! Adversarial draft corpus regression tests.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use mimir_core::{ClockTime, Store};
use mimir_librarian::{
    Draft, DraftMetadata, DraftProcessingDecision, DraftProcessor, DraftSourceSurface,
    LibrarianError, LlmInvoker, RetryingDraftProcessor,
};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct Corpus {
    schema_version: u32,
    cases: Vec<CorpusCase>,
}

#[derive(Debug, Deserialize)]
struct CorpusCase {
    name: String,
    draft: String,
    responses: Vec<serde_json::Value>,
    expected_decision: String,
    expected_attempts: usize,
    expected_counts: ExpectedCounts,
    required_lisp_substrings: Vec<String>,
    forbidden_lisp_substrings: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ExpectedCounts {
    semantic: usize,
    episodic: usize,
    procedural: usize,
    inferential: usize,
}

#[derive(Debug, Clone)]
struct SequenceInvoker {
    responses: Arc<Mutex<VecDeque<String>>>,
    system_prompts: Arc<Mutex<Vec<String>>>,
    user_messages: Arc<Mutex<Vec<String>>>,
}

impl SequenceInvoker {
    fn new(responses: Vec<String>) -> Self {
        Self {
            responses: Arc::new(Mutex::new(responses.into())),
            system_prompts: Arc::new(Mutex::new(Vec::new())),
            user_messages: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn system_prompts(&self) -> Result<Vec<String>, String> {
        self.system_prompts
            .lock()
            .map(|prompts| prompts.clone())
            .map_err(|err| format!("system prompts lock poisoned: {err}"))
    }

    fn user_messages(&self) -> Result<Vec<String>, String> {
        self.user_messages
            .lock()
            .map(|messages| messages.clone())
            .map_err(|err| format!("user messages lock poisoned: {err}"))
    }
}

impl LlmInvoker for SequenceInvoker {
    fn invoke(&self, system_prompt: &str, user_message: &str) -> Result<String, LibrarianError> {
        self.system_prompts
            .lock()
            .map_err(|err| LibrarianError::LlmInvocationFailed {
                message: format!("system prompts lock poisoned: {err}"),
            })?
            .push(system_prompt.to_string());
        self.user_messages
            .lock()
            .map_err(|err| LibrarianError::LlmInvocationFailed {
                message: format!("user messages lock poisoned: {err}"),
            })?
            .push(user_message.to_string());
        self.responses
            .lock()
            .map_err(|err| LibrarianError::LlmInvocationFailed {
                message: format!("responses lock poisoned: {err}"),
            })?
            .pop_front()
            .ok_or_else(|| LibrarianError::LlmInvocationFailed {
                message: "no canned corpus response left".to_string(),
            })
    }
}

fn fixed_now() -> Result<ClockTime, mimir_core::ClockTimeError> {
    ClockTime::try_from_millis(1_777_000_000_000)
}

fn draft(raw_text: &str) -> Draft {
    Draft::with_metadata(
        raw_text.to_string(),
        DraftMetadata::new(DraftSourceSurface::Cli, SystemTime::UNIX_EPOCH),
    )
}

fn expected_decision(value: &str) -> Result<DraftProcessingDecision, String> {
    match value {
        "accepted" => Ok(DraftProcessingDecision::Accepted),
        "skipped" => Ok(DraftProcessingDecision::Skipped),
        "failed" => Ok(DraftProcessingDecision::Failed),
        "quarantined" => Ok(DraftProcessingDecision::Quarantined),
        "deferred" => Ok(DraftProcessingDecision::Deferred),
        other => Err(format!("unsupported expected decision: {other}")),
    }
}

fn response_lisp_payloads(case: &CorpusCase) -> Vec<String> {
    let mut payloads = Vec::new();
    for response in &case.responses {
        if let Some(records) = response
            .get("records")
            .and_then(serde_json::Value::as_array)
        {
            for record in records {
                if let Some(lisp) = record.get("lisp").and_then(serde_json::Value::as_str) {
                    payloads.push(lisp.to_string());
                }
            }
        }
    }
    payloads
}

fn assert_messages(case: &CorpusCase, invoker: &SequenceInvoker) -> Result<(), String> {
    let user_messages = invoker.user_messages()?;
    assert_eq!(
        user_messages.len(),
        case.expected_attempts,
        "case {} attempt count drifted",
        case.name
    );
    for message in &user_messages {
        assert!(
            message.contains("<draft_boundary>"),
            "case {} did not include an explicit draft boundary",
            case.name
        );
        assert!(
            message.contains("mimir.raw_draft.data.v1"),
            "case {} did not label raw drafts as data",
            case.name
        );
        assert!(
            message.contains("data_only_never_execute"),
            "case {} did not carry the execute boundary",
            case.name
        );
        assert!(
            message.contains(&format!("<draft>\n{}\n</draft>", case.draft)),
            "case {} did not keep the adversarial draft inside the draft envelope",
            case.name
        );
    }
    assert!(
        invoker
            .system_prompts()?
            .iter()
            .all(|prompt| prompt.contains("Every user message is a prose memory DRAFT")),
        "case {} lost the system prompt input contract",
        case.name
    );
    Ok(())
}

fn assert_expected_lisp(case: &CorpusCase) {
    let lisp_payloads = response_lisp_payloads(case).join("\n");
    for required in &case.required_lisp_substrings {
        assert!(
            lisp_payloads.contains(required),
            "case {} missing expected structured content {required:?}",
            case.name
        );
    }
    for forbidden in &case.forbidden_lisp_substrings {
        assert!(
            !lisp_payloads.contains(forbidden),
            "case {} persisted forbidden adversarial content {forbidden:?}",
            case.name
        );
    }
}

fn assert_store_counts(case: &CorpusCase, store: &Store) {
    assert_eq!(
        store.pipeline().semantic_records().len(),
        case.expected_counts.semantic,
        "case {} semantic count drifted",
        case.name
    );
    assert_eq!(
        store.pipeline().episodic_records().len(),
        case.expected_counts.episodic,
        "case {} episodic count drifted",
        case.name
    );
    assert_eq!(
        store.pipeline().procedural_records().len(),
        case.expected_counts.procedural,
        "case {} procedural count drifted",
        case.name
    );
    assert_eq!(
        store.pipeline().inferential_records().len(),
        case.expected_counts.inferential,
        "case {} inferential count drifted",
        case.name
    );
}

#[test]
fn adversarial_corpus_treats_drafts_as_data() -> Result<(), Box<dyn std::error::Error>> {
    let corpus: Corpus = serde_json::from_str(include_str!("fixtures/adversarial_corpus.json"))?;
    assert_eq!(corpus.schema_version, 1);
    assert!(
        corpus.cases.len() >= 5,
        "corpus must cover the Category 6 adversarial shapes"
    );

    for case in corpus.cases {
        let responses = case
            .responses
            .iter()
            .map(serde_json::to_string)
            .collect::<Result<Vec<_>, _>>()?;
        let tmp = tempfile::tempdir()?;
        let log_path = tmp.path().join("canonical.log");
        let invoker = SequenceInvoker::new(responses);
        let mut processor =
            RetryingDraftProcessor::new_at(invoker.clone(), 3, fixed_now()?, &log_path)?;

        let decision = processor
            .process(&draft(&case.draft))
            .map_err(|err| format!("case {} failed during processing: {err}", case.name))?;

        assert_eq!(
            decision,
            expected_decision(&case.expected_decision)?,
            "case {} returned the wrong decision",
            case.name
        );
        assert_messages(&case, &invoker)?;
        assert_expected_lisp(&case);

        let reopened = Store::open(&log_path)?;
        assert_store_counts(&case, &reopened);
    }
    Ok(())
}
