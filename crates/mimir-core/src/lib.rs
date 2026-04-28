//! Mimir core — foundational types plus the librarian pipeline and
//! durable store.
//!
//! The crate groups into three layers:
//!
//! - **Foundational types** ([`symbol`], [`workspace`], [`clock`],
//!   [`confidence`], [`memory_kind`], [`source_kind`], [`value`]) —
//!   newtypes and enums every higher component consumes.
//! - **Librarian pipeline** ([`lex`] → [`parse`] → [`bind`] →
//!   [`semantic`] → [`pipeline`] → [`canonical`]) — compiles agent
//!   S-expression input into canonical bytecode per
//!   `librarian-pipeline.md`.
//! - **Durability + domain utilities** ([`log`], [`store`],
//!   [`inference_methods`], [`decay`]) — the append-only workspace
//!   store, the 14-method inference registry, and the deterministic
//!   confidence-decay model.
//!
//! Read-only inspection and round-trip rendering live in the sibling
//! `mimir-cli` crate.
//!
//! See `docs/concepts/` in the repository root for the architectural
//! specifications these modules implement:
//!
//! - [`memory-type-taxonomy.md`](https://github.com/buildepicshit/Mimir/blob/main/docs/concepts/memory-type-taxonomy.md)
//! - [`symbol-identity-semantics.md`](https://github.com/buildepicshit/Mimir/blob/main/docs/concepts/symbol-identity-semantics.md)
//! - [`workspace-model.md`](https://github.com/buildepicshit/Mimir/blob/main/docs/concepts/workspace-model.md)
//! - [`temporal-model.md`](https://github.com/buildepicshit/Mimir/blob/main/docs/concepts/temporal-model.md)
//! - [`grounding-model.md`](https://github.com/buildepicshit/Mimir/blob/main/docs/concepts/grounding-model.md)
//! - [`ir-write-surface.md`](https://github.com/buildepicshit/Mimir/blob/main/docs/concepts/ir-write-surface.md)
//! - [`ir-canonical-form.md`](https://github.com/buildepicshit/Mimir/blob/main/docs/concepts/ir-canonical-form.md)
//! - [`librarian-pipeline.md`](https://github.com/buildepicshit/Mimir/blob/main/docs/concepts/librarian-pipeline.md)
//! - [`write-protocol.md`](https://github.com/buildepicshit/Mimir/blob/main/docs/concepts/write-protocol.md)
//! - [`confidence-decay.md`](https://github.com/buildepicshit/Mimir/blob/main/docs/concepts/confidence-decay.md)

#![forbid(unsafe_code)]
#![deny(missing_docs)]
// unwrap/expect/panic are denied at the workspace level for library
// correctness (per PRINCIPLES.md § 7). Relax inside #[cfg(test)] so unit
// tests and property tests can use idiomatic Result-assertion and
// "unreachable test state" patterns. External integration tests under
// tests/ opt in via their own crate-level attribute.
#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::approx_constant,
        clippy::similar_names,
    )
)]

pub mod bind;
pub mod canonical;
pub mod clock;
pub mod confidence;
pub mod dag;
pub mod decay;
pub mod inference_methods;
pub mod lex;
pub mod log;
pub mod memory_kind;
pub mod parse;
pub mod pipeline;
pub mod read;
pub mod resolver;
pub mod semantic;
pub mod source_kind;
pub mod store;
pub mod symbol;
pub mod value;
pub mod workspace;
pub mod workspace_lock;

pub use clock::{ClockTime, ClockTimeError};
pub use confidence::{Confidence, ConfidenceError};
pub use memory_kind::{Episodic, Inferential, MemoryKind, MemoryKindTag, Procedural, Semantic};
pub use pipeline::{EmitError, Pipeline, PipelineError};
pub use source_kind::SourceKind;
pub use store::{EpisodeId, Store, StoreError};
pub use symbol::{ScopedSymbolId, SymbolId, SymbolKind};
pub use value::Value;
pub use workspace::{WorkspaceId, WorkspaceIdError};
pub use workspace_lock::{lock_path_for_log, WorkspaceLockError, WorkspaceWriteLock};
