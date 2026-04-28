//! Symbol binder — resolves [`crate::parse::RawSymbolName`] into
//! [`crate::SymbolId`] against a per-workspace [`SymbolTable`] and
//! produces a fully-typed [`BoundForm`].
//!
//! Implements the semantics specified in
//! `docs/concepts/symbol-identity-semantics.md` §§ 3–9.

use std::collections::{BTreeMap, HashMap, HashSet};

use thiserror::Error;

use crate::confidence::ConfidenceError;
use crate::parse::{KeywordArgs, RawSymbolName, RawValue, UnboundForm};
use crate::symbol::{ScopedSymbolId, SymbolId, SymbolKind};
use crate::value::Value;

/// Maximum length of an alias chain before the binder rejects further
/// extensions with [`BindError::AliasChainLengthExceeded`]. Matches
/// `symbol-identity-semantics.md` § 7.3.
pub const ALIAS_CHAIN_LIMIT: usize = 16;

// Inference-method registration is owned by
// [`crate::inference_methods::InferenceMethod`]; bind validates against
// that enum via `InferenceMethod::from_symbol_name`.

/// One symbol-table mutation performed by the binder while processing a
/// batch. The emit stage serializes the journal into `SYMBOL_*`
/// canonical records (opcodes 0x30–0x33 per `ir-canonical-form.md`
/// § 6.6) so that replay from the log can reconstitute the workspace's
/// symbol table. Per `librarian-pipeline.md` § 3.4 the journal is part
/// of bind's output alongside the bound AST.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SymbolMutation {
    /// A first-use allocation — `id`, `name` (canonical), `kind`.
    Allocate {
        /// Allocated symbol ID.
        id: SymbolId,
        /// Canonical name at allocation time.
        name: String,
        /// Locked kind.
        kind: SymbolKind,
    },
    /// Rename — old canonical becomes an alias; new canonical attaches
    /// to the same `id`. Replay reconstructs the canonical/alias state.
    Rename {
        /// Subject symbol.
        id: SymbolId,
        /// New canonical name.
        new_canonical: String,
        /// Locked kind at the time of the rename.
        kind: SymbolKind,
    },
    /// Alias — an additional name resolves to the same `id`.
    Alias {
        /// Subject symbol.
        id: SymbolId,
        /// The alias attached.
        alias: String,
        /// Locked kind.
        kind: SymbolKind,
    },
    /// Retire — sets the symbol's retired flag.
    Retire {
        /// Subject symbol.
        id: SymbolId,
        /// Canonical name at retire time (for the log record).
        name: String,
        /// Locked kind.
        kind: SymbolKind,
    },
}

/// A single entry in the symbol table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SymbolEntry {
    /// The canonical name as currently recorded.
    pub canonical_name: String,
    /// Alternate names (aliases) that resolve to the same symbol.
    pub aliases: Vec<String>,
    /// Locked kind for this symbol.
    pub kind: SymbolKind,
    /// Whether the symbol is currently retired.
    pub retired: bool,
}

/// A per-workspace symbol table.
///
/// Symbol IDs are allocated monotonically as a `u64`; canonical names
/// and alias names are stored in a flat name→id lookup for O(1)
/// resolution.
///
/// See `symbol-identity-semantics.md` § 3 for the design.
///
/// # Examples
///
/// ```
/// # #![allow(clippy::unwrap_used)]
/// use mimir_core::bind::SymbolTable;
/// use mimir_core::SymbolKind;
///
/// let mut table = SymbolTable::new();
/// let id = table
///     .allocate("alice".into(), SymbolKind::Agent)
///     .unwrap();
/// assert_eq!(table.lookup("alice"), Some(id));
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SymbolTable {
    next_id: u64,
    entries: HashMap<SymbolId, SymbolEntry>,
    names_to_id: HashMap<String, SymbolId>,
    /// Fast membership test for the `retired` flag; mirrors
    /// `entries[id].retired`.
    retired: HashSet<SymbolId>,
}

impl SymbolTable {
    /// Construct an empty symbol table.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Allocate a new symbol with the given canonical name and kind.
    ///
    /// # Errors
    ///
    /// - [`BindError::SymbolRenameConflict`] if `name` is already in
    ///   the table (its existing binding must be used or renamed).
    pub fn allocate(&mut self, name: String, kind: SymbolKind) -> Result<SymbolId, BindError> {
        if self.names_to_id.contains_key(&name) {
            return Err(BindError::SymbolRenameConflict { name });
        }
        let id = SymbolId::new(self.next_id);
        self.next_id += 1;
        self.entries.insert(
            id,
            SymbolEntry {
                canonical_name: name.clone(),
                aliases: Vec::new(),
                kind,
                retired: false,
            },
        );
        self.names_to_id.insert(name, id);
        Ok(id)
    }

    /// Resolve a name (canonical or alias) to a [`SymbolId`].
    #[must_use]
    pub fn lookup(&self, name: &str) -> Option<SymbolId> {
        self.names_to_id.get(name).copied()
    }

    /// Return the [`SymbolKind`] for an already-allocated symbol.
    #[must_use]
    pub fn kind_of(&self, id: SymbolId) -> Option<SymbolKind> {
        self.entries.get(&id).map(|e| e.kind)
    }

    /// Return the entry for an already-allocated symbol.
    #[must_use]
    pub fn entry(&self, id: SymbolId) -> Option<&SymbolEntry> {
        self.entries.get(&id)
    }

    /// Iterate all entries in the table, yielding `(SymbolId, &SymbolEntry)`
    /// pairs. Iteration order is undefined; callers that need stable
    /// output should sort on the consuming side.
    pub fn iter_entries(&self) -> impl Iterator<Item = (SymbolId, &SymbolEntry)> + '_ {
        self.entries.iter().map(|(id, entry)| (*id, entry))
    }

    /// Declare an alias — both names resolve to the same symbol.
    ///
    /// Both symbols must already be allocated; they must resolve to
    /// the same [`SymbolId`] already, OR one must not be an alias of
    /// the other yet. See `symbol-identity-semantics.md` § 7.
    ///
    /// # Errors
    ///
    /// - [`BindError::AliasChainLengthExceeded`] if adding this alias
    ///   would push the chain past [`ALIAS_CHAIN_LIMIT`].
    /// - [`BindError::SymbolRenameConflict`] if `b_name` resolves to a
    ///   different symbol than `a_name` (merging two distinct symbols
    ///   is not an alias operation; it must go through rename).
    pub fn add_alias(&mut self, a_name: &str, b_name: &str) -> Result<(), BindError> {
        let a_id = self.names_to_id.get(a_name).copied();
        let b_id = self.names_to_id.get(b_name).copied();
        match (a_id, b_id) {
            (Some(id_a), Some(id_b)) if id_a == id_b => Ok(()),
            (Some(_), Some(_)) => Err(BindError::SymbolRenameConflict {
                name: b_name.to_string(),
            }),
            (Some(id), None) => self.attach_alias(id, b_name.to_string()),
            (None, Some(id)) => self.attach_alias(id, a_name.to_string()),
            (None, None) => Err(BindError::UnknownSymbol {
                name: a_name.to_string(),
            }),
        }
    }

    /// Rename a symbol. Old name becomes an alias; new name becomes the
    /// canonical.
    ///
    /// # Errors
    ///
    /// - [`BindError::UnknownSymbol`] if `old_name` does not resolve.
    /// - [`BindError::SymbolRenameConflict`] if `new_name` is already
    ///   bound to a different symbol.
    /// - [`BindError::AliasChainLengthExceeded`] if adding the old name
    ///   as an alias would push the chain past the cap.
    pub fn rename(&mut self, old_name: &str, new_name: &str) -> Result<SymbolId, BindError> {
        let id =
            self.names_to_id
                .get(old_name)
                .copied()
                .ok_or_else(|| BindError::UnknownSymbol {
                    name: old_name.to_string(),
                })?;
        if let Some(existing) = self.names_to_id.get(new_name).copied() {
            if existing != id {
                return Err(BindError::SymbolRenameConflict {
                    name: new_name.to_string(),
                });
            }
            // already aliased to the same symbol; promote the new_name to canonical.
        }
        // Rotate the canonical name; push old canonical into aliases.
        let entry = self
            .entries
            .get_mut(&id)
            .ok_or_else(|| BindError::UnknownSymbol {
                name: old_name.to_string(),
            })?;
        let previous_canonical = std::mem::replace(&mut entry.canonical_name, new_name.to_string());
        if entry.aliases.len() >= ALIAS_CHAIN_LIMIT {
            return Err(BindError::AliasChainLengthExceeded {
                name: new_name.to_string(),
                limit: ALIAS_CHAIN_LIMIT,
            });
        }
        if previous_canonical != new_name {
            entry.aliases.push(previous_canonical);
        }
        self.names_to_id.insert(new_name.to_string(), id);
        Ok(id)
    }

    /// Mark a symbol retired. Existing references still resolve; new
    /// references through the agent API trigger `stale_symbol`
    /// warnings on read.
    ///
    /// # Errors
    ///
    /// - [`BindError::UnknownSymbol`] if `name` does not resolve.
    pub fn retire(&mut self, name: &str) -> Result<SymbolId, BindError> {
        let id = self
            .names_to_id
            .get(name)
            .copied()
            .ok_or_else(|| BindError::UnknownSymbol {
                name: name.to_string(),
            })?;
        if let Some(entry) = self.entries.get_mut(&id) {
            entry.retired = true;
        }
        self.retired.insert(id);
        Ok(id)
    }

    /// Clear a retirement flag. Symmetric with [`Self::retire`].
    ///
    /// # Errors
    ///
    /// - [`BindError::UnknownSymbol`] if `name` does not resolve.
    pub fn unretire(&mut self, name: &str) -> Result<SymbolId, BindError> {
        let id = self
            .names_to_id
            .get(name)
            .copied()
            .ok_or_else(|| BindError::UnknownSymbol {
                name: name.to_string(),
            })?;
        if let Some(entry) = self.entries.get_mut(&id) {
            entry.retired = false;
        }
        self.retired.remove(&id);
        Ok(id)
    }

    /// Whether the symbol is currently retired.
    #[must_use]
    pub fn is_retired(&self, id: SymbolId) -> bool {
        self.retired.contains(&id)
    }

    /// Replay an `SYMBOL_ALLOC` canonical record into this table.
    /// Unlike [`Self::allocate`] the caller supplies the original
    /// `SymbolId`; the `next_id` monotonic counter is advanced past the
    /// replayed ID so future agent allocations stay unique.
    ///
    /// Used by [`crate::store::Store::open`] to rebuild the table from
    /// a durable log.
    ///
    /// # Errors
    ///
    /// - [`BindError::SymbolRenameConflict`] if `id` or `name` is
    ///   already bound (log corruption; replay must be strictly
    ///   monotonic).
    pub fn replay_allocate(
        &mut self,
        id: SymbolId,
        name: String,
        kind: SymbolKind,
    ) -> Result<(), BindError> {
        if self.entries.contains_key(&id) || self.names_to_id.contains_key(&name) {
            return Err(BindError::SymbolRenameConflict { name });
        }
        self.entries.insert(
            id,
            SymbolEntry {
                canonical_name: name.clone(),
                aliases: Vec::new(),
                kind,
                retired: false,
            },
        );
        self.names_to_id.insert(name, id);
        let next_after = id.as_u64().saturating_add(1);
        if next_after > self.next_id {
            self.next_id = next_after;
        }
        Ok(())
    }

    /// Replay an `SYMBOL_ALIAS` canonical record. Attaches `alias` as an
    /// additional name resolving to `id`.
    ///
    /// # Errors
    ///
    /// - [`BindError::UnknownSymbol`] if `id` has never been allocated.
    /// - [`BindError::AliasChainLengthExceeded`] if adding the alias
    ///   would exceed [`ALIAS_CHAIN_LIMIT`].
    pub fn replay_alias(&mut self, id: SymbolId, alias: String) -> Result<(), BindError> {
        self.attach_alias(id, alias)
    }

    /// Replay an `SYMBOL_RENAME` canonical record. The previous canonical
    /// name is rotated into aliases.
    ///
    /// # Errors
    ///
    /// - [`BindError::UnknownSymbol`] if `id` has never been allocated.
    pub fn replay_rename(&mut self, id: SymbolId, new_canonical: String) -> Result<(), BindError> {
        let entry = self
            .entries
            .get_mut(&id)
            .ok_or_else(|| BindError::UnknownSymbol {
                name: new_canonical.clone(),
            })?;
        let previous_canonical =
            std::mem::replace(&mut entry.canonical_name, new_canonical.clone());
        if previous_canonical != new_canonical {
            entry.aliases.push(previous_canonical);
        }
        self.names_to_id.insert(new_canonical, id);
        Ok(())
    }

    /// Replay an `SYMBOL_RETIRE` canonical record. Marks the symbol
    /// retired. `name` is the symbol's canonical name at retire time;
    /// propagated into the error for diagnosability.
    ///
    /// # Errors
    ///
    /// - [`BindError::UnknownSymbol`] if `id` has never been allocated.
    pub fn replay_retire(&mut self, id: SymbolId, name: String) -> Result<(), BindError> {
        let entry = self
            .entries
            .get_mut(&id)
            .ok_or(BindError::UnknownSymbol { name })?;
        entry.retired = true;
        self.retired.insert(id);
        Ok(())
    }

    fn attach_alias(&mut self, id: SymbolId, alias: String) -> Result<(), BindError> {
        let entry = self
            .entries
            .get_mut(&id)
            .ok_or_else(|| BindError::UnknownSymbol {
                name: alias.clone(),
            })?;
        if entry.aliases.len() >= ALIAS_CHAIN_LIMIT {
            return Err(BindError::AliasChainLengthExceeded {
                name: alias,
                limit: ALIAS_CHAIN_LIMIT,
            });
        }
        entry.aliases.push(alias.clone());
        self.names_to_id.insert(alias, id);
        Ok(())
    }
}

/// Errors produced by the binder.
///
/// Typed per `PRINCIPLES.md` § 2. Agents route recovery on the error
/// variant, never by matching message text.
#[derive(Debug, Error, PartialEq)]
pub enum BindError {
    /// A symbol was used in a slot expecting a different kind than the
    /// one locked at first allocation.
    #[error("symbol kind mismatch for {name:?}: expected {expected:?}, locked as {existing:?}")]
    SymbolKindMismatch {
        /// The symbol name.
        name: String,
        /// The slot's expected kind.
        expected: SymbolKind,
        /// The kind this symbol was allocated with.
        existing: SymbolKind,
    },

    /// Two distinct symbols cannot share a canonical name.
    #[error("rename conflict: {name:?} already bound")]
    SymbolRenameConflict {
        /// The conflicting name.
        name: String,
    },

    /// Alias chain length exceeded per [`ALIAS_CHAIN_LIMIT`].
    #[error("alias chain for {name:?} exceeded length limit {limit}")]
    AliasChainLengthExceeded {
        /// The offending alias.
        name: String,
        /// The limit.
        limit: usize,
    },

    /// A rename or retire referenced an unknown symbol.
    #[error("unknown symbol {name:?}")]
    UnknownSymbol {
        /// The offending name.
        name: String,
    },

    /// A `@name:Kind` annotation used a Kind that isn't in the
    /// [`SymbolKind`] taxonomy.
    #[error("unknown SymbolKind annotation {found:?}")]
    BadKind {
        /// The annotation text.
        found: String,
    },

    /// An Inferential memory used a `method` symbol whose name is not
    /// in the registered inference-method set
    /// ([`crate::inference_methods::InferenceMethod`]).
    #[error("unregistered inference method {found:?}")]
    UnregisteredInferenceMethod {
        /// The offending method name.
        found: String,
    },

    /// A keyword argument's value did not have the expected shape.
    #[error("invalid keyword value for {keyword:?}: {reason}")]
    InvalidKeywordValue {
        /// The keyword.
        keyword: String,
        /// A short diagnostic.
        reason: &'static str,
    },

    /// A confidence value violated its range at bind time.
    #[error("confidence out of range: {0}")]
    ConfidenceOutOfRange(#[from] ConfidenceError),

    /// A value position received a [`RawValue::List`] where a scalar
    /// was required.
    #[error("unexpected list value at {slot:?}")]
    UnexpectedList {
        /// The slot name.
        slot: &'static str,
    },

    /// A timestamp keyword value was missing or not a timestamp.
    #[error("missing or malformed timestamp for keyword {keyword:?}")]
    InvalidTimestampKeyword {
        /// The keyword.
        keyword: String,
    },

    /// A cross-workspace symbol was referenced but this binder is
    /// scoped to a single workspace and cannot locally allocate.
    #[error("cross-workspace symbol reference not allowed locally: {scoped:?}")]
    ForeignSymbolForbidden {
        /// The offending scoped reference.
        scoped: ScopedSymbolId,
    },

    /// A `(episode :start :label …)` label exceeds the
    /// `episode-semantics.md` § 4.3 256-byte cap.
    #[error("episode label length {len} exceeds {cap}-byte cap")]
    LabelTooLong {
        /// Actual byte length of the offending label.
        len: usize,
        /// Configured cap (spec § 4.3 — 256).
        cap: usize,
    },
}

/// Bound keyword arguments — keys preserved as strings, values typed.
pub type BoundKeywords = BTreeMap<String, Value>;

/// An AST form with all `RawSymbolName`s resolved to `SymbolId`s and
/// all `RawValue`s converted to typed [`Value`]s.
///
/// Produced by [`bind`]. Consumed by the Semantic pipeline stage.
#[derive(Clone, Debug, PartialEq)]
#[allow(clippy::module_name_repetitions)]
pub enum BoundForm {
    /// Semantic memory write.
    Sem {
        /// Subject.
        s: SymbolId,
        /// Predicate.
        p: SymbolId,
        /// Object.
        o: Value,
        /// Keyword arguments: `src`, `c`, `v`, optionally `projected`.
        keywords: BoundKeywords,
    },
    /// Episodic memory write.
    Epi {
        /// Stable event ID.
        event_id: SymbolId,
        /// Event-type symbol.
        kind: SymbolId,
        /// Participant symbols (may be empty).
        participants: Vec<SymbolId>,
        /// Location symbol.
        location: SymbolId,
        /// Keyword arguments: `at`, `obs`, `src`, `c`.
        keywords: BoundKeywords,
    },
    /// Procedural memory write.
    Pro {
        /// Stable rule ID.
        rule_id: SymbolId,
        /// Trigger value.
        trigger: Value,
        /// Action value.
        action: Value,
        /// Keyword arguments: `scp`, `src`, `c`, optional `pre`.
        keywords: BoundKeywords,
    },
    /// Inferential memory write.
    Inf {
        /// Subject.
        s: SymbolId,
        /// Predicate.
        p: SymbolId,
        /// Object.
        o: Value,
        /// Parent memory symbols.
        derived_from: Vec<SymbolId>,
        /// Registered inference-method symbol.
        method: SymbolId,
        /// Keyword arguments: `c`, `v`, optional `projected`.
        keywords: BoundKeywords,
    },
    /// Alias declaration.
    Alias {
        /// First symbol.
        a: SymbolId,
        /// Second symbol.
        b: SymbolId,
    },
    /// Rename — old name becomes alias of new (already canonical).
    Rename {
        /// The previous canonical.
        old: SymbolId,
        /// The new canonical.
        new: SymbolId,
    },
    /// Retire a symbol.
    Retire {
        /// Target.
        name: SymbolId,
        /// Optional reason.
        reason: Option<String>,
    },
    /// Correct a prior Episodic memory.
    Correct {
        /// The Episode being corrected.
        target_episode: SymbolId,
        /// The corrected Episodic body.
        corrected: Box<BoundForm>,
    },
    /// Promote an ephemeral memory to canonical.
    Promote {
        /// The ephemeral memory symbol.
        name: SymbolId,
    },
    /// Read-path query.
    Query {
        /// Optional positional selector.
        selector: Option<Value>,
        /// Remaining keyword arguments.
        keywords: BoundKeywords,
    },
    /// Explicit Episode-boundary directive (`episode-semantics.md`
    /// § 3.2). `:close` variants carry no metadata; `:start` variants
    /// may carry any combination of label / parent / retracts.
    Episode {
        /// Whether this form opens or closes an Episode.
        action: crate::parse::EpisodeAction,
        /// Optional human-readable label — already checked for
        /// length during bind so the semantic stage can trust it.
        label: Option<String>,
        /// Resolved parent Episode `SymbolId`, if set.
        parent_episode: Option<SymbolId>,
        /// Resolved retracted-Episode `SymbolId`s.
        retracts: Vec<SymbolId>,
    },
    /// Pin / unpin / authoritative flag write.
    Flag {
        /// Which flag operation.
        action: crate::parse::FlagAction,
        /// The memory being pinned / unpinned / (un)marked.
        memory: SymbolId,
        /// The invoking agent / operator (required per
        /// `confidence-decay.md` §§ 7 / 8 audit trail).
        actor: SymbolId,
    },
}

/// Bind a sequence of unbound forms against the given symbol table.
///
/// Mutations to `table` (symbol allocations, renames, alias attachments,
/// retirement flag flips) are applied as each form binds. Pipeline
/// callers that need transactional semantics should snapshot the table
/// before calling [`bind`] and roll back on error.
///
/// # Errors
///
/// Returns the first [`BindError`] encountered.
pub fn bind(
    forms: Vec<UnboundForm>,
    table: &mut SymbolTable,
) -> Result<(Vec<BoundForm>, Vec<SymbolMutation>), BindError> {
    let mut journal = Vec::new();
    let bound = forms
        .into_iter()
        .map(|form| bind_form(form, table, &mut journal))
        .collect::<Result<Vec<_>, _>>()?;
    Ok((bound, journal))
}

#[allow(clippy::too_many_lines)]
fn bind_form(
    form: UnboundForm,
    table: &mut SymbolTable,
    journal: &mut Vec<SymbolMutation>,
) -> Result<BoundForm, BindError> {
    match form {
        UnboundForm::Sem { s, p, o, keywords } => {
            let s = resolve_or_allocate(table, journal, &s, SymbolKind::Agent)?;
            let p = resolve_or_allocate(table, journal, &p, SymbolKind::Predicate)?;
            let o = bind_value(o, table, journal, "sem.o", SymbolKind::Literal)?;
            let keywords = bind_keywords(keywords, table, journal, sem_keyword_kinds())?;
            Ok(BoundForm::Sem { s, p, o, keywords })
        }
        UnboundForm::Epi {
            event_id,
            kind,
            participants,
            location,
            keywords,
        } => {
            let event_id = resolve_or_allocate(table, journal, &event_id, SymbolKind::Memory)?;
            let kind = resolve_or_allocate(table, journal, &kind, SymbolKind::EventType)?;
            let participants: Vec<SymbolId> = participants
                .iter()
                .map(|name| resolve_or_allocate(table, journal, name, SymbolKind::Agent))
                .collect::<Result<_, _>>()?;
            let location = resolve_or_allocate(table, journal, &location, SymbolKind::Literal)?;
            let keywords = bind_keywords(keywords, table, journal, epi_keyword_kinds())?;
            Ok(BoundForm::Epi {
                event_id,
                kind,
                participants,
                location,
                keywords,
            })
        }
        UnboundForm::Pro {
            rule_id,
            trigger,
            action,
            keywords,
        } => {
            let rule_id = resolve_or_allocate(table, journal, &rule_id, SymbolKind::Memory)?;
            let trigger = bind_value(trigger, table, journal, "pro.trigger", SymbolKind::Literal)?;
            let action = bind_value(action, table, journal, "pro.action", SymbolKind::Literal)?;
            let keywords = bind_keywords(keywords, table, journal, pro_keyword_kinds())?;
            Ok(BoundForm::Pro {
                rule_id,
                trigger,
                action,
                keywords,
            })
        }
        UnboundForm::Inf {
            s,
            p,
            o,
            derived_from,
            method,
            keywords,
        } => {
            let s = resolve_or_allocate(table, journal, &s, SymbolKind::Agent)?;
            let p = resolve_or_allocate(table, journal, &p, SymbolKind::Predicate)?;
            let o = bind_value(o, table, journal, "inf.o", SymbolKind::Literal)?;
            let derived_from: Vec<SymbolId> = derived_from
                .iter()
                .map(|name| resolve_or_allocate(table, journal, name, SymbolKind::Memory))
                .collect::<Result<_, _>>()?;
            let method = resolve_or_allocate(table, journal, &method, SymbolKind::InferenceMethod)?;
            let method_name = method_name_for(method, table);
            if crate::inference_methods::InferenceMethod::from_symbol_name(&method_name).is_none() {
                return Err(BindError::UnregisteredInferenceMethod { found: method_name });
            }
            let keywords = bind_keywords(keywords, table, journal, inf_keyword_kinds())?;
            Ok(BoundForm::Inf {
                s,
                p,
                o,
                derived_from,
                method,
                keywords,
            })
        }
        UnboundForm::Alias { a, b } => {
            let a_id = ensure_allocated(table, journal, &a, SymbolKind::Literal)?;
            let b_id = ensure_allocated(table, journal, &b, SymbolKind::Literal)?;
            // Pre-check: if both names already resolve to the same
            // symbol, add_alias is a no-op and we emit no journal entry.
            let already_aliased = a_id == b_id
                && table.entry(a_id).is_some_and(|e| {
                    e.canonical_name == b.as_str() || e.aliases.iter().any(|n| n == b.as_str())
                });
            table.add_alias(a.as_str(), b.as_str())?;
            if !already_aliased {
                // One of the two names has just been attached as an
                // alias of the shared symbol. Record the newly-attached
                // name; the kind is whichever side held the existing
                // allocation.
                let (attached_to, new_alias) = if let Some(entry) = table.entry(a_id) {
                    if entry.aliases.iter().any(|n| n == b.as_str()) {
                        (a_id, b.as_str().to_string())
                    } else {
                        (b_id, a.as_str().to_string())
                    }
                } else {
                    (a_id, b.as_str().to_string())
                };
                let kind = table.kind_of(attached_to).unwrap_or(SymbolKind::Literal);
                journal.push(SymbolMutation::Alias {
                    id: attached_to,
                    alias: new_alias,
                    kind,
                });
            }
            Ok(BoundForm::Alias { a: a_id, b: b_id })
        }
        UnboundForm::Rename { old, new } => {
            let id = table.rename(old.as_str(), new.as_str())?;
            let kind = table.kind_of(id).unwrap_or(SymbolKind::Literal);
            journal.push(SymbolMutation::Rename {
                id,
                new_canonical: new.as_str().to_string(),
                kind,
            });
            Ok(BoundForm::Rename { old: id, new: id })
        }
        UnboundForm::Retire { name, keywords } => {
            let id = table.retire(name.as_str())?;
            let kind = table.kind_of(id).unwrap_or(SymbolKind::Literal);
            // Record the symbol's current canonical name (which may
            // differ from `name` if the agent retired by an alias) so
            // the log entry and any replay error reference the
            // canonical identifier.
            let canonical = table
                .entry(id)
                .map_or_else(|| name.as_str().to_string(), |e| e.canonical_name.clone());
            journal.push(SymbolMutation::Retire {
                id,
                name: canonical,
                kind,
            });
            let reason = keywords.get("reason").and_then(|v| match v {
                RawValue::String(s) => Some(s.clone()),
                _ => None,
            });
            Ok(BoundForm::Retire { name: id, reason })
        }
        UnboundForm::Correct {
            target_episode,
            corrected,
        } => {
            let target = resolve_or_allocate(table, journal, &target_episode, SymbolKind::Memory)?;
            let bound = bind_form(*corrected, table, journal)?;
            Ok(BoundForm::Correct {
                target_episode: target,
                corrected: Box::new(bound),
            })
        }
        UnboundForm::Promote { name } => {
            let id = resolve_or_allocate(table, journal, &name, SymbolKind::Memory)?;
            Ok(BoundForm::Promote { name: id })
        }
        UnboundForm::Query { selector, keywords } => {
            let selector = selector
                .map(|v| bind_value(v, table, journal, "query.selector", SymbolKind::Literal))
                .transpose()?;
            // Query keyword types are heterogeneous; accept any typed value.
            let keywords = bind_keywords(keywords, table, journal, &BTreeMap::new())?;
            Ok(BoundForm::Query { selector, keywords })
        }
        UnboundForm::Flag {
            action,
            memory,
            actor,
        } => {
            let memory = resolve_or_allocate(table, journal, &memory, SymbolKind::Memory)?;
            let actor = resolve_or_allocate(table, journal, &actor, SymbolKind::Agent)?;
            Ok(BoundForm::Flag {
                action,
                memory,
                actor,
            })
        }
        UnboundForm::Episode {
            action,
            label,
            parent_episode,
            retracts,
        } => {
            // Label length cap per episode-semantics.md § 4.3.
            if let Some(ref l) = label {
                if l.len() > MAX_EPISODE_LABEL_BYTES {
                    return Err(BindError::LabelTooLong {
                        len: l.len(),
                        cap: MAX_EPISODE_LABEL_BYTES,
                    });
                }
            }
            let parent_episode = parent_episode
                .map(|raw| resolve_or_allocate(table, journal, &raw, SymbolKind::Memory))
                .transpose()?;
            let retracts = retracts
                .into_iter()
                .map(|raw| resolve_or_allocate(table, journal, &raw, SymbolKind::Memory))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(BoundForm::Episode {
                action,
                label,
                parent_episode,
                retracts,
            })
        }
    }
}

/// `episode-semantics.md` § 4.3 label cap.
const MAX_EPISODE_LABEL_BYTES: usize = 256;

fn method_name_for(method: SymbolId, table: &SymbolTable) -> String {
    table
        .entry(method)
        .map_or_else(String::new, |e| e.canonical_name.clone())
}

fn resolve_or_allocate(
    table: &mut SymbolTable,
    journal: &mut Vec<SymbolMutation>,
    name: &RawSymbolName,
    default_kind: SymbolKind,
) -> Result<SymbolId, BindError> {
    // If the source carried an explicit `:Kind` annotation, it overrides
    // the position default and must be used both for first allocation
    // and for consistency validation on reuse.
    let effective_kind = if let Some(annotation) = &name.kind {
        parse_symbol_kind(annotation)?
    } else {
        default_kind
    };
    if let Some(id) = table.lookup(name.as_str()) {
        let existing = table.kind_of(id).ok_or_else(|| BindError::UnknownSymbol {
            name: name.name.clone(),
        })?;
        if existing != effective_kind {
            return Err(BindError::SymbolKindMismatch {
                name: name.name.clone(),
                expected: effective_kind,
                existing,
            });
        }
        return Ok(id);
    }
    let id = table.allocate(name.name.clone(), effective_kind)?;
    journal.push(SymbolMutation::Allocate {
        id,
        name: name.name.clone(),
        kind: effective_kind,
    });
    Ok(id)
}

fn ensure_allocated(
    table: &mut SymbolTable,
    journal: &mut Vec<SymbolMutation>,
    name: &RawSymbolName,
    default_kind: SymbolKind,
) -> Result<SymbolId, BindError> {
    if let Some(id) = table.lookup(name.as_str()) {
        return Ok(id);
    }
    let id = table.allocate(name.name.clone(), default_kind)?;
    journal.push(SymbolMutation::Allocate {
        id,
        name: name.name.clone(),
        kind: default_kind,
    });
    Ok(id)
}

fn bind_value(
    raw: RawValue,
    table: &mut SymbolTable,
    journal: &mut Vec<SymbolMutation>,
    slot: &'static str,
    default_kind_for_symbols: SymbolKind,
) -> Result<Value, BindError> {
    match raw {
        RawValue::RawSymbol(name) => {
            let id = resolve_or_allocate(table, journal, &name, default_kind_for_symbols)?;
            Ok(Value::Symbol(id))
        }
        RawValue::TypedSymbol { name, kind } => {
            let parsed_kind = parse_symbol_kind(&kind)?;
            let id = resolve_or_allocate(table, journal, &name, parsed_kind)?;
            Ok(Value::Symbol(id))
        }
        RawValue::Bareword(s) | RawValue::String(s) => Ok(Value::String(s)),
        RawValue::Integer(i) => Ok(Value::Integer(i)),
        RawValue::Float(f) => Ok(Value::Float(f)),
        RawValue::Boolean(b) => Ok(Value::Boolean(b)),
        RawValue::Timestamp(ct) => Ok(Value::Timestamp(ct)),
        RawValue::TimestampRaw(text) => Err(BindError::InvalidTimestampKeyword { keyword: text }),
        RawValue::Nil | RawValue::List(_) => Err(BindError::UnexpectedList { slot }),
    }
}

fn bind_keywords(
    raw: KeywordArgs,
    table: &mut SymbolTable,
    journal: &mut Vec<SymbolMutation>,
    kind_hints: &BTreeMap<&'static str, SymbolKind>,
) -> Result<BoundKeywords, BindError> {
    let mut out = BoundKeywords::new();
    for (key, value) in raw {
        let fallback_kind = kind_hints
            .get(key.as_str())
            .copied()
            .unwrap_or(SymbolKind::Literal);
        // Confidence keyword `c` is preserved as Value::Float here; the
        // semantic stage converts to Confidence and enforces source-bound
        // + range (per grounding-model.md § 4).
        let bound = if key == "c" {
            #[allow(clippy::cast_precision_loss)]
            let f = match value {
                RawValue::Float(f) => f,
                RawValue::Integer(i) => i as f64,
                _ => {
                    return Err(BindError::InvalidKeywordValue {
                        keyword: key,
                        reason: "expected numeric confidence in [0.0, 1.0]",
                    });
                }
            };
            Value::Float(f)
        } else if key == "projected"
            || key == "include_retired"
            || key == "include_projected"
            || key == "show_framing"
            || key == "explain_filtered"
            || key == "debug_mode"
        {
            let RawValue::Boolean(b) = value else {
                return Err(BindError::InvalidKeywordValue {
                    keyword: key,
                    reason: "expected boolean",
                });
            };
            Value::Boolean(b)
        } else {
            bind_value_with_fallback(value, table, journal, fallback_kind)?
        };
        out.insert(key, bound);
    }
    Ok(out)
}

fn bind_value_with_fallback(
    raw: RawValue,
    table: &mut SymbolTable,
    journal: &mut Vec<SymbolMutation>,
    fallback_kind: SymbolKind,
) -> Result<Value, BindError> {
    // Shared-slot entry point — delegates to bind_value with the fallback kind.
    bind_value(raw, table, journal, "keyword value", fallback_kind)
}

/// Map a `@name:Kind` annotation's kind portion to [`SymbolKind`].
///
/// # Errors
///
/// Returns [`BindError::BadKind`] if `text` is not one of the twelve
/// [`SymbolKind`] variant names per `symbol-identity-semantics.md` § 4.
pub fn parse_symbol_kind(text: &str) -> Result<SymbolKind, BindError> {
    let kind = match text {
        "Agent" => SymbolKind::Agent,
        "Document" => SymbolKind::Document,
        "Registry" => SymbolKind::Registry,
        "Service" => SymbolKind::Service,
        "Policy" => SymbolKind::Policy,
        "Memory" => SymbolKind::Memory,
        "InferenceMethod" => SymbolKind::InferenceMethod,
        "Scope" => SymbolKind::Scope,
        "Predicate" => SymbolKind::Predicate,
        "EventType" => SymbolKind::EventType,
        "Workspace" => SymbolKind::Workspace,
        "Literal" => SymbolKind::Literal,
        _ => {
            return Err(BindError::BadKind {
                found: text.to_string(),
            });
        }
    };
    Ok(kind)
}

fn sem_keyword_kinds() -> &'static BTreeMap<&'static str, SymbolKind> {
    static KINDS: std::sync::OnceLock<BTreeMap<&'static str, SymbolKind>> =
        std::sync::OnceLock::new();
    KINDS.get_or_init(|| {
        let mut m = BTreeMap::new();
        m.insert("src", SymbolKind::Agent);
        m
    })
}

fn epi_keyword_kinds() -> &'static BTreeMap<&'static str, SymbolKind> {
    static KINDS: std::sync::OnceLock<BTreeMap<&'static str, SymbolKind>> =
        std::sync::OnceLock::new();
    KINDS.get_or_init(|| {
        let mut m = BTreeMap::new();
        m.insert("src", SymbolKind::Agent);
        m
    })
}

fn pro_keyword_kinds() -> &'static BTreeMap<&'static str, SymbolKind> {
    static KINDS: std::sync::OnceLock<BTreeMap<&'static str, SymbolKind>> =
        std::sync::OnceLock::new();
    KINDS.get_or_init(|| {
        let mut m = BTreeMap::new();
        m.insert("src", SymbolKind::Agent);
        m.insert("scp", SymbolKind::Scope);
        m
    })
}

fn inf_keyword_kinds() -> &'static BTreeMap<&'static str, SymbolKind> {
    static KINDS: std::sync::OnceLock<BTreeMap<&'static str, SymbolKind>> =
        std::sync::OnceLock::new();
    KINDS.get_or_init(BTreeMap::new)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse;

    fn fresh_table() -> SymbolTable {
        SymbolTable::new()
    }

    #[test]
    fn allocate_and_lookup() {
        let mut table = fresh_table();
        let id = table.allocate("alice".into(), SymbolKind::Agent).unwrap();
        assert_eq!(table.lookup("alice"), Some(id));
        assert_eq!(table.kind_of(id), Some(SymbolKind::Agent));
    }

    #[test]
    fn monotonic_allocation() {
        let mut table = fresh_table();
        let a = table.allocate("a".into(), SymbolKind::Agent).unwrap();
        let b = table.allocate("b".into(), SymbolKind::Agent).unwrap();
        let c = table.allocate("c".into(), SymbolKind::Agent).unwrap();
        assert!(a.as_u64() < b.as_u64());
        assert!(b.as_u64() < c.as_u64());
    }

    #[test]
    fn rename_preserves_id_and_swaps_canonical() {
        let mut table = fresh_table();
        let id = table.allocate("old".into(), SymbolKind::Agent).unwrap();
        let after = table.rename("old", "new").unwrap();
        assert_eq!(id, after);
        assert_eq!(table.lookup("new"), Some(id));
        assert_eq!(table.lookup("old"), Some(id));
        let entry = table.entry(id).unwrap();
        assert_eq!(entry.canonical_name, "new");
        assert!(entry.aliases.contains(&"old".to_string()));
    }

    #[test]
    fn alias_collapses_to_same_id() {
        let mut table = fresh_table();
        let a = table.allocate("a".into(), SymbolKind::Agent).unwrap();
        table.allocate("b".into(), SymbolKind::Agent).unwrap();
        // `a` and `b` are distinct allocations — alias should refuse to
        // merge them.
        assert!(matches!(
            table.add_alias("a", "b"),
            Err(BindError::SymbolRenameConflict { .. })
        ));
        assert_eq!(table.lookup("a"), Some(a));
    }

    #[test]
    fn retire_and_unretire_round_trip() {
        let mut table = fresh_table();
        let id = table.allocate("tmp".into(), SymbolKind::Agent).unwrap();
        assert!(!table.is_retired(id));
        table.retire("tmp").unwrap();
        assert!(table.is_retired(id));
        table.unretire("tmp").unwrap();
        assert!(!table.is_retired(id));
    }

    #[test]
    fn bind_sem_form_produces_bound_ids() {
        let mut table = fresh_table();
        let forms =
            parse(r#"(sem @alice email "alice@example.com" :src @profile :c 0.95 :v 2024-01-15)"#)
                .unwrap();
        let (bound, _journal) = bind(forms, &mut table).unwrap();
        assert_eq!(bound.len(), 1);
        let BoundForm::Sem { s, p, o, keywords } = &bound[0] else {
            panic!("expected Sem");
        };
        assert_eq!(table.kind_of(*s), Some(SymbolKind::Agent));
        assert_eq!(table.kind_of(*p), Some(SymbolKind::Predicate));
        assert_eq!(o, &Value::String("alice@example.com".into()));
        assert!(keywords.contains_key("src"));
        assert!(keywords.contains_key("c"));
        assert!(keywords.contains_key("v"));
    }

    #[test]
    fn kind_mismatch_on_reuse_is_reported() {
        let mut table = fresh_table();
        // First allocation locks `@x` as Agent.
        let _ = table.allocate("x".into(), SymbolKind::Agent).unwrap();
        // The parser in a sem form uses `@x` as the predicate — which
        // is locked to Predicate kind. That conflicts.
        let forms = parse(r#"(sem @alice @x "v" :src @profile :c 0.5 :v 2024-01-15)"#).unwrap();
        let err = bind(forms, &mut table).unwrap_err();
        assert!(matches!(err, BindError::SymbolKindMismatch { .. }));
    }

    #[test]
    fn unregistered_inference_method_errors() {
        let mut table = fresh_table();
        let forms = parse("(inf @a p @b (@m1) @bogus_method :c 0.5 :v 2024-01-15)").unwrap();
        let err = bind(forms, &mut table).unwrap_err();
        assert!(matches!(err, BindError::UnregisteredInferenceMethod { .. }));
    }

    #[test]
    fn registered_method_binds_cleanly() {
        let mut table = fresh_table();
        let forms = parse("(inf @a p @b (@m1) @pattern_summarize :c 0.7 :v 2024-03-15)").unwrap();
        let (bound, _journal) = bind(forms, &mut table).unwrap();
        assert_eq!(bound.len(), 1);
    }

    #[test]
    fn rename_and_retire_forms_apply_to_table() {
        let mut table = fresh_table();
        let id = table.allocate("old".into(), SymbolKind::Agent).unwrap();
        let forms = parse("(rename @old @new) (retire @new)").unwrap();
        let (_bound, _journal) = bind(forms, &mut table).unwrap();
        let entry = table.entry(id).unwrap();
        assert_eq!(entry.canonical_name, "new");
        assert!(table.is_retired(id));
    }

    #[test]
    fn typed_symbol_annotation_locks_kind() {
        let mut table = fresh_table();
        let forms =
            parse(r"(sem @alice:Agent email @book:Document :src @profile :c 0.5 :v 2024-01-15)")
                .unwrap();
        let (_bound, _journal) = bind(forms, &mut table).unwrap();
        let alice = table.lookup("alice").unwrap();
        let book = table.lookup("book").unwrap();
        assert_eq!(table.kind_of(alice), Some(SymbolKind::Agent));
        assert_eq!(table.kind_of(book), Some(SymbolKind::Document));
    }

    #[test]
    fn bad_kind_annotation_errors() {
        let mut table = fresh_table();
        let forms =
            parse(r#"(sem @alice:Bogus email "v" :src @profile :c 0.5 :v 2024-01-15)"#).unwrap();
        let err = bind(forms, &mut table).unwrap_err();
        assert!(matches!(err, BindError::BadKind { .. }));
    }
}
