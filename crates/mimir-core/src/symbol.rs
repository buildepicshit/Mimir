//! `SymbolId`, `ScopedSymbolId`, `SymbolKind` — the symbol-identity
//! primitives from `docs/concepts/symbol-identity-semantics.md` §§ 3–4.

use std::fmt;

use crate::workspace::WorkspaceId;

/// A workspace-local symbol identifier.
///
/// Monotonic `u64` counter allocated by the librarian (see
/// `symbol-identity-semantics.md` § 5.1). `SymbolId(42)` in workspace A
/// and `SymbolId(42)` in workspace B are **distinct symbols** in
/// distinct tables — never cross-workspace equal.
///
/// # Examples
///
/// ```
/// use mimir_core::SymbolId;
///
/// let a = SymbolId::new(42);
/// assert_eq!(a.as_u64(), 42);
/// ```
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SymbolId(u64);

impl SymbolId {
    /// Construct a [`SymbolId`] from its raw `u64` value.
    #[must_use]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// The underlying numeric ID.
    #[must_use]
    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

impl fmt::Display for SymbolId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}", self.0)
    }
}

/// A fully-qualified symbol reference carrying both a workspace and a
/// local ID.
///
/// Surfaced at workspace boundaries — cross-workspace reads, decoder
/// output, audit logs — where the workspace component must be explicit.
/// Within a workspace the librarian uses bare `SymbolId` internally.
/// See `symbol-identity-semantics.md` § 3.2.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct ScopedSymbolId {
    /// The workspace that owns the underlying symbol.
    pub workspace: WorkspaceId,
    /// The workspace-local symbol identifier.
    pub local: SymbolId,
}

impl ScopedSymbolId {
    /// Construct a [`ScopedSymbolId`].
    #[must_use]
    pub const fn new(workspace: WorkspaceId, local: SymbolId) -> Self {
        Self { workspace, local }
    }
}

impl fmt::Display for ScopedSymbolId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{}", self.local, self.workspace)
    }
}

/// Kind of a symbol in the Mimir taxonomy.
///
/// Twelve kinds, matching `symbol-identity-semantics.md` § 4. The enum
/// is `#[non_exhaustive]` so additions do not break callers on semver
/// minor bumps (see `PRINCIPLES.md` § 10).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SymbolKind {
    /// An actor — profile subject, observer, reporter, rule-maker.
    Agent,
    /// A static document with a citation pointer.
    Document,
    /// An authoritative programmatic source (package manifest, DNS,
    /// filesystem metadata).
    Registry,
    /// A live third-party API.
    Service,
    /// A policy-making source distinct from an Agent (dual-kind symbols
    /// are not permitted in v1).
    Policy,
    /// A memory ID (used in `derived_from`, `event_id`, `rule_id`).
    Memory,
    /// A registered inference-method tag for an [`crate::Inferential`]
    /// memory.
    InferenceMethod,
    /// A scope tag — Procedural rule applicability, named ephemeral
    /// scopes.
    Scope,
    /// A predicate name in an `s-p-o` tuple.
    Predicate,
    /// An event-type tag for an [`crate::Episodic`] memory.
    EventType,
    /// A workspace identifier referenced as a symbol.
    Workspace,
    /// A typed-value bareword not belonging to any registry (catch-all
    /// for enum-ish values).
    Literal,
}

#[cfg(test)]
mod tests {
    use super::*;
    use ulid::Ulid;

    #[test]
    fn symbol_id_roundtrip() {
        for raw in [0_u64, 1, 42, u64::MAX] {
            assert_eq!(SymbolId::new(raw).as_u64(), raw);
        }
    }

    #[test]
    fn scoped_symbol_distinguishes_workspaces() {
        let ws_a = WorkspaceId::from_ulid(Ulid::from_parts(1, 1));
        let ws_b = WorkspaceId::from_ulid(Ulid::from_parts(2, 2));
        let a = ScopedSymbolId::new(ws_a, SymbolId::new(42));
        let b = ScopedSymbolId::new(ws_b, SymbolId::new(42));
        assert_ne!(a, b);
    }

    #[test]
    fn scoped_symbol_equal_when_workspace_and_local_match() {
        let ws = WorkspaceId::from_ulid(Ulid::from_parts(7, 7));
        let a = ScopedSymbolId::new(ws, SymbolId::new(99));
        let b = ScopedSymbolId::new(ws, SymbolId::new(99));
        assert_eq!(a, b);
    }

    #[test]
    fn symbol_kind_is_nonexhaustive_pattern() {
        // Compile-time check: exhaustive matches over #[non_exhaustive] enums
        // from the defining crate are allowed, so this test only asserts we
        // can still use the enum in a match block.
        let kind = SymbolKind::Agent;
        let label = match kind {
            SymbolKind::Agent => "agent",
            SymbolKind::Document => "document",
            SymbolKind::Registry => "registry",
            SymbolKind::Service => "service",
            SymbolKind::Policy => "policy",
            SymbolKind::Memory => "memory",
            SymbolKind::InferenceMethod => "inference_method",
            SymbolKind::Scope => "scope",
            SymbolKind::Predicate => "predicate",
            SymbolKind::EventType => "event_type",
            SymbolKind::Workspace => "workspace",
            SymbolKind::Literal => "literal",
        };
        assert_eq!(label, "agent");
    }
}
