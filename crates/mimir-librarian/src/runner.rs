//! Draft processing runner.
//!
//! This module owns the mechanics around a one-shot librarian run:
//! recover stale `processing/` drafts, claim pending drafts, invoke a
//! processor, then move each draft to its resulting lifecycle state.
//! The actual LLM / validation / commit logic is injected through
//! [`DraftProcessor`] and lands in later Category 1 slices.

use std::time::{Duration, SystemTime};

use serde::Serialize;

use crate::{Draft, DraftState, DraftStore, LibrarianError};

/// Default age after which a draft left in `processing/` is assumed
/// abandoned and recovered to `pending/`.
pub const DEFAULT_PROCESSING_STALE_SECS: u64 = 15 * 60;

/// Decision returned by a draft processor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DraftProcessingDecision {
    /// Draft was accepted by the processor.
    ///
    /// In the current Category 1 slice this means the LLM output
    /// passed bounded pre-emit validation and committed durably to the
    /// canonical store.
    Accepted,
    /// Draft was intentionally skipped.
    Skipped,
    /// Draft failed processing and should be retained for review.
    Failed,
    /// Draft is unsafe, conflicting, or unresolved.
    Quarantined,
    /// Processor is not ready to make progress; draft returns to pending.
    Deferred,
}

impl DraftProcessingDecision {
    fn target_state(self) -> DraftState {
        match self {
            Self::Accepted => DraftState::Accepted,
            Self::Skipped => DraftState::Skipped,
            Self::Failed => DraftState::Failed,
            Self::Quarantined => DraftState::Quarantined,
            Self::Deferred => DraftState::Pending,
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Skipped => "skipped",
            Self::Failed => "failed",
            Self::Quarantined => "quarantined",
            Self::Deferred => "deferred",
        }
    }
}

/// Processor invoked for each claimed draft.
pub trait DraftProcessor {
    /// Process one draft and decide where it should move next.
    ///
    /// The runner guarantees the draft has already been claimed into
    /// `processing/`. It also owns the post-decision state move.
    ///
    /// # Errors
    ///
    /// Returns a librarian error when the processor cannot make a
    /// safe lifecycle decision. The runner leaves the draft in
    /// `processing/` in this case so a later stale-processing recovery
    /// can retry it rather than silently marking it terminal.
    fn process(&mut self, draft: &Draft) -> Result<DraftProcessingDecision, LibrarianError>;
}

/// Placeholder processor used until LLM structuring / validation /
/// commit processing is wired.
#[derive(Debug, Default)]
pub struct DeferredDraftProcessor;

impl DraftProcessor for DeferredDraftProcessor {
    fn process(&mut self, _draft: &Draft) -> Result<DraftProcessingDecision, LibrarianError> {
        Ok(DraftProcessingDecision::Deferred)
    }
}

/// One processed draft row for operator-visible run summaries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DraftRunItem {
    /// Draft ID.
    pub id: String,
    /// Processor decision.
    pub decision: DraftProcessingDecision,
    /// Final lifecycle state after the runner moved the file.
    pub final_state: String,
}

/// Summary of one run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DraftRunSummary {
    /// Stale `processing/` drafts recovered before claiming pending work.
    pub recovered_processing: usize,
    /// Pending drafts listed after recovery.
    pub pending_seen: usize,
    /// Pending drafts successfully claimed into `processing/`.
    pub claimed: usize,
    /// Drafts accepted by the processor.
    pub accepted: usize,
    /// Drafts skipped by the processor.
    pub skipped: usize,
    /// Drafts failed by the processor.
    pub failed: usize,
    /// Drafts quarantined by the processor.
    pub quarantined: usize,
    /// Drafts returned to `pending/` without terminal handling.
    pub deferred: usize,
    /// Drafts that disappeared between list and claim, usually due to
    /// a concurrent runner. This is not fatal.
    pub claim_misses: usize,
    /// Per-draft movement report.
    pub items: Vec<DraftRunItem>,
}

impl DraftRunSummary {
    fn new(recovered_processing: usize, pending_seen: usize) -> Self {
        Self {
            recovered_processing,
            pending_seen,
            claimed: 0,
            accepted: 0,
            skipped: 0,
            failed: 0,
            quarantined: 0,
            deferred: 0,
            claim_misses: 0,
            items: Vec::new(),
        }
    }

    fn record(&mut self, id: String, decision: DraftProcessingDecision, final_state: DraftState) {
        match decision {
            DraftProcessingDecision::Accepted => self.accepted += 1,
            DraftProcessingDecision::Skipped => self.skipped += 1,
            DraftProcessingDecision::Failed => self.failed += 1,
            DraftProcessingDecision::Quarantined => self.quarantined += 1,
            DraftProcessingDecision::Deferred => self.deferred += 1,
        }
        self.items.push(DraftRunItem {
            id,
            decision,
            final_state: final_state.dir_name().to_string(),
        });
    }
}

/// Run pending draft processing once.
///
/// `now` and `stale_after` are explicit so tests can be deterministic.
///
/// # Errors
///
/// Returns any draft store or processor error that prevents the run
/// from preserving a clear lifecycle state.
pub fn run_once<P: DraftProcessor>(
    store: &DraftStore,
    processor: &mut P,
    now: SystemTime,
    stale_after: Duration,
) -> Result<DraftRunSummary, LibrarianError> {
    let span = tracing::info_span!(
        target: "mimir.librarian.run",
        "mimir.librarian.run",
        recovered_processing = tracing::field::Empty,
        pending_seen = tracing::field::Empty,
        claimed = tracing::field::Empty,
        accepted = tracing::field::Empty,
        skipped = tracing::field::Empty,
        failed = tracing::field::Empty,
        quarantined = tracing::field::Empty,
        deferred = tracing::field::Empty,
        claim_misses = tracing::field::Empty,
    );
    let _guard = span.enter();

    let stale_before = now
        .checked_sub(stale_after)
        .unwrap_or(SystemTime::UNIX_EPOCH);
    let recovered = store.recover_stale_processing(stale_before)?;
    let pending = store.list(DraftState::Pending)?;
    let mut summary = DraftRunSummary::new(recovered.len(), pending.len());
    record_summary_fields(&span, &summary);

    for draft in pending {
        let id = draft.id();
        let id_hex = id.to_hex();
        match store.transition(id, DraftState::Pending, DraftState::Processing) {
            Ok(_) => {
                summary.claimed += 1;
                span.record("claimed", count_u64(summary.claimed));
            }
            Err(LibrarianError::DraftNotFound {
                state: DraftState::Pending,
                id: missing,
            }) if missing == id => {
                summary.claim_misses += 1;
                span.record("claim_misses", count_u64(summary.claim_misses));
                continue;
            }
            Err(err) => return Err(err),
        }

        let decision = processor.process(&draft)?;
        let final_state = decision.target_state();
        store.transition(id, DraftState::Processing, final_state)?;
        summary.record(id_hex, decision, final_state);
        record_summary_fields(&span, &summary);
        tracing::info!(
            target: "mimir.librarian.draft_processed",
            draft_id = %id,
            decision = decision.as_str(),
            final_state = final_state.dir_name(),
            "draft processed"
        );
    }

    Ok(summary)
}

fn record_summary_fields(span: &tracing::Span, summary: &DraftRunSummary) {
    span.record(
        "recovered_processing",
        count_u64(summary.recovered_processing),
    );
    span.record("pending_seen", count_u64(summary.pending_seen));
    span.record("claimed", count_u64(summary.claimed));
    span.record("accepted", count_u64(summary.accepted));
    span.record("skipped", count_u64(summary.skipped));
    span.record("failed", count_u64(summary.failed));
    span.record("quarantined", count_u64(summary.quarantined));
    span.record("deferred", count_u64(summary.deferred));
    span.record("claim_misses", count_u64(summary.claim_misses));
}

fn count_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::test_tracing::{capture, FieldValue};
    use crate::{DraftMetadata, DraftSourceSurface};

    #[derive(Debug)]
    struct TextMatchingProcessor;

    impl DraftProcessor for TextMatchingProcessor {
        fn process(&mut self, draft: &Draft) -> Result<DraftProcessingDecision, LibrarianError> {
            if draft.raw_text().contains("accept") {
                Ok(DraftProcessingDecision::Accepted)
            } else if draft.raw_text().contains("skip") {
                Ok(DraftProcessingDecision::Skipped)
            } else if draft.raw_text().contains("fail") {
                Ok(DraftProcessingDecision::Failed)
            } else if draft.raw_text().contains("quarantine") {
                Ok(DraftProcessingDecision::Quarantined)
            } else {
                Err(LibrarianError::NotYetImplemented {
                    component: "test processor missing decision",
                })
            }
        }
    }

    #[derive(Debug)]
    struct SkipProcessor;

    impl DraftProcessor for SkipProcessor {
        fn process(&mut self, _draft: &Draft) -> Result<DraftProcessingDecision, LibrarianError> {
            Ok(DraftProcessingDecision::Skipped)
        }
    }

    fn draft(text: &str) -> Draft {
        Draft::with_metadata(
            text.to_string(),
            DraftMetadata::new(DraftSourceSurface::Cli, SystemTime::UNIX_EPOCH),
        )
    }

    #[test]
    fn run_once_moves_pending_drafts_to_processor_terminal_states(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let tmp = tempfile::tempdir()?;
        let store = DraftStore::new(tmp.path());
        let accepted = draft("accept this draft");
        let skipped = draft("skip this draft");
        let failed = draft("fail this draft");
        let quarantined = draft("quarantine this draft");
        for draft in [&accepted, &skipped, &failed, &quarantined] {
            store.submit(draft)?;
        }

        let mut processor = TextMatchingProcessor;
        let summary = run_once(
            &store,
            &mut processor,
            SystemTime::UNIX_EPOCH + Duration::from_secs(30),
            Duration::from_secs(10),
        )?;

        assert_eq!(summary.recovered_processing, 0);
        assert_eq!(summary.pending_seen, 4);
        assert_eq!(summary.claimed, 4);
        assert_eq!(summary.accepted, 1);
        assert_eq!(summary.skipped, 1);
        assert_eq!(summary.failed, 1);
        assert_eq!(summary.quarantined, 1);
        assert_eq!(summary.deferred, 0);
        assert_eq!(store.list(DraftState::Pending)?.len(), 0);
        assert_eq!(
            store.load(DraftState::Accepted, accepted.id())?.id(),
            accepted.id()
        );
        assert_eq!(
            store.load(DraftState::Skipped, skipped.id())?.id(),
            skipped.id()
        );
        assert_eq!(
            store.load(DraftState::Failed, failed.id())?.id(),
            failed.id()
        );
        assert_eq!(
            store.load(DraftState::Quarantined, quarantined.id())?.id(),
            quarantined.id()
        );
        Ok(())
    }

    #[test]
    fn run_once_defers_without_terminal_state() -> Result<(), Box<dyn std::error::Error>> {
        let tmp = tempfile::tempdir()?;
        let store = DraftStore::new(tmp.path());
        let draft = draft("processor is not ready yet");
        store.submit(&draft)?;

        let mut processor = DeferredDraftProcessor;
        let summary = run_once(
            &store,
            &mut processor,
            SystemTime::UNIX_EPOCH + Duration::from_secs(30),
            Duration::from_secs(10),
        )?;

        assert_eq!(summary.claimed, 1);
        assert_eq!(summary.deferred, 1);
        assert_eq!(summary.items[0].final_state, "pending");
        assert_eq!(store.list(DraftState::Pending)?.len(), 1);
        assert_eq!(store.list(DraftState::Processing)?.len(), 0);
        Ok(())
    }

    #[test]
    fn run_once_recovers_stale_processing_before_pending_scan(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let tmp = tempfile::tempdir()?;
        let store = DraftStore::new(tmp.path());
        let draft = draft("recover me first");
        store.submit(&draft)?;
        store.transition(draft.id(), DraftState::Pending, DraftState::Processing)?;

        let mut processor = SkipProcessor;
        let summary = run_once(
            &store,
            &mut processor,
            SystemTime::now() + Duration::from_secs(60),
            Duration::from_secs(1),
        )?;

        assert_eq!(summary.recovered_processing, 1);
        assert_eq!(summary.pending_seen, 1);
        assert_eq!(summary.skipped, 1);
        assert_eq!(store.list(DraftState::Processing)?.len(), 0);
        assert_eq!(store.list(DraftState::Skipped)?.len(), 1);
        Ok(())
    }

    #[test]
    fn run_once_emits_summary_span() -> Result<(), Box<dyn std::error::Error>> {
        let tmp = tempfile::tempdir()?;
        let store = DraftStore::new(tmp.path());
        let accepted = draft("accept this draft");
        let skipped = draft("skip this draft");
        store.submit(&accepted)?;
        store.submit(&skipped)?;

        let mut run_result = None;
        let shared = capture(|| {
            let mut processor = TextMatchingProcessor;
            run_result = Some(run_once(
                &store,
                &mut processor,
                SystemTime::UNIX_EPOCH + Duration::from_secs(30),
                Duration::from_secs(10),
            ));
        });
        let summary = match run_result {
            Some(Ok(summary)) => summary,
            Some(Err(err)) => return Err(Box::new(err)),
            None => return Err("run did not execute".into()),
        };
        assert_eq!(summary.claimed, 2);

        let spans = shared
            .spans
            .lock()
            .map_err(|err| format!("spans lock poisoned: {err}"))?;
        let Some(span) = spans.iter().find(|span| {
            span.name == "mimir.librarian.run"
                && span.fields.get("pending_seen").and_then(FieldValue::as_u64) == Some(2)
                && span.fields.get("accepted").and_then(FieldValue::as_u64) == Some(1)
                && span.fields.get("skipped").and_then(FieldValue::as_u64) == Some(1)
        }) else {
            return Err("run span missing".into());
        };
        assert_eq!(
            span.fields.get("pending_seen").and_then(FieldValue::as_u64),
            Some(2),
        );
        assert_eq!(
            span.fields.get("accepted").and_then(FieldValue::as_u64),
            Some(1),
        );
        assert_eq!(
            span.fields.get("skipped").and_then(FieldValue::as_u64),
            Some(1),
        );
        Ok(())
    }
}
