//! `PreEmitValidator` — in-process validation of candidate canonical
//! Lisp against a scratch [`mimir_core::Pipeline`].
//!
//! Given a candidate record (a single Lisp form as a string), the
//! validator runs the full pipeline — lex, parse, bind, semantic,
//! emit — against a scratch pipeline instance and returns either
//! `Ok(())` or the specific [`mimir_core::PipelineError`] that
//! caused the rejection. The pipeline's clone-on-write rollback
//! semantics mean rejected candidates leave no durable state.
//!
//! Successful validations commit to the scratch pipeline so later
//! validations see the symbols and supersession constraints that
//! earlier accepted records introduced.

use mimir_core::{ClockTime, Pipeline};

use crate::LibrarianError;

/// Validates a candidate canonical Lisp record against a scratch
/// `mimir_core::Pipeline` before the librarian attempts to commit
/// it to the real log.
///
/// Operates in-process. No subprocess spawning; no IPC. The
/// pipeline's clone-on-write symbol-table semantics mean a rejected
/// candidate leaves no residue.
#[derive(Debug, Clone)]
pub struct PreEmitValidator {
    scratch: Pipeline,
}

impl PreEmitValidator {
    /// Construct a fresh validator with an empty scratch pipeline.
    ///
    /// Each validator instance owns its own pipeline state. Successful
    /// validations advance that scratch state; rejected candidates do
    /// not mutate it.
    #[must_use]
    pub fn new() -> Self {
        Self {
            scratch: Pipeline::new(),
        }
    }

    /// Validate a candidate Lisp record.
    ///
    /// # Errors
    ///
    /// Returns [`LibrarianError::ValidationClock`] if wall-clock
    /// capture fails, or [`LibrarianError::ValidationRejected`] when
    /// the scratch pipeline rejects the candidate.
    pub fn validate(&mut self, candidate_lisp: &str) -> Result<(), LibrarianError> {
        let now = ClockTime::now().map_err(|err| LibrarianError::ValidationClock {
            message: err.to_string(),
        })?;
        self.validate_at(candidate_lisp, now)
    }

    /// Validate a candidate Lisp record at a caller-supplied clock.
    ///
    /// This is the deterministic entry point used by tests and future
    /// run-loop code that wants all candidates in a draft to share a
    /// controlled wall clock.
    ///
    /// # Errors
    ///
    /// Returns [`LibrarianError::ValidationRejected`] when the scratch
    /// pipeline rejects the candidate.
    pub fn validate_at(
        &mut self,
        candidate_lisp: &str,
        now: ClockTime,
    ) -> Result<(), LibrarianError> {
        let mut working = self.scratch.clone();
        working
            .compile_batch(candidate_lisp, now)
            .map_err(|source| LibrarianError::ValidationRejected { source })?;
        self.scratch = working;
        Ok(())
    }
}

impl Default for PreEmitValidator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constructor_succeeds() {
        let _validator = PreEmitValidator::new();
    }

    #[test]
    fn default_is_equivalent_to_new() {
        let _a = PreEmitValidator::new();
        let _b = PreEmitValidator::default();
    }

    fn fixed_now() -> Result<ClockTime, mimir_core::ClockTimeError> {
        ClockTime::try_from_millis(1_713_350_400_000)
    }

    #[test]
    fn validate_accepts_valid_candidate() -> Result<(), Box<dyn std::error::Error>> {
        let mut validator = PreEmitValidator::new();
        validator.validate_at(
            "(sem @alice @knows @bob :src @observation :c 0.8 :v 2024-01-15)",
            fixed_now()?,
        )?;
        Ok(())
    }

    #[test]
    fn validate_rejects_parse_error_without_poisoning_scratch(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut validator = PreEmitValidator::new();
        assert!(matches!(
            validator.validate_at("(sem @alice", fixed_now()?),
            Err(LibrarianError::ValidationRejected {
                source: mimir_core::PipelineError::Parse(_)
            })
        ));

        validator.validate_at(
            "(sem @alice @knows @bob :src @observation :c 0.8 :v 2024-01-15)",
            fixed_now()?,
        )?;
        Ok(())
    }

    #[test]
    fn validate_commits_successes_to_scratch_state() -> Result<(), Box<dyn std::error::Error>> {
        let mut validator = PreEmitValidator::new();
        let candidate = "(sem @alice @knows @bob :src @observation :c 0.8 :v 2024-01-15)";

        validator.validate_at(candidate, fixed_now()?)?;
        assert!(matches!(
            validator.validate_at(candidate, fixed_now()?),
            Err(LibrarianError::ValidationRejected {
                source: mimir_core::PipelineError::Emit(_)
            })
        ));
        Ok(())
    }
}
