//! Lisp S-expression parser for the Mimir write surface.
//!
//! Implements `docs/concepts/ir-write-surface.md` § 5 — the ten top-level
//! forms (`sem`, `epi`, `pro`, `inf`, `alias`, `rename`, `retire`,
//! `correct`, `promote`, `query`) with positional required fields and
//! order-insensitive keyword arguments.
//!
//! The parser produces [`UnboundForm`] ASTs where every symbol is still
//! a [`RawSymbolName`] — binding into a workspace-scoped `SymbolId` is
//! the next pipeline stage (see `librarian-pipeline.md` § 3.4).

use std::collections::BTreeMap;

use thiserror::Error;

use crate::clock::ClockTime;
use crate::lex::{LexError, Position, Spanned, Token};

/// A raw symbol name as written in the source (without the leading `@`),
/// optionally carrying the `:Kind` annotation for the binder.
///
/// The parser produces these rather than resolved `SymbolId`s because
/// symbol tables are workspace-scoped and binding happens in a later
/// pipeline stage. When the surface uses `@name:Kind`, the annotation
/// is preserved in [`Self::kind`] so the binder can override the
/// position-default kind.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RawSymbolName {
    /// The `@name` part without the `@`.
    pub name: String,
    /// Optional `:Kind` annotation — passed through to the binder for
    /// kind override or validation.
    pub kind: Option<String>,
}

impl RawSymbolName {
    /// Construct a raw symbol name without a kind annotation.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            kind: None,
        }
    }

    /// Construct a raw symbol name with a `:Kind` annotation.
    #[must_use]
    pub fn with_kind(name: impl Into<String>, kind: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            kind: Some(kind.into()),
        }
    }

    /// The underlying name string (without the `@` or `:Kind`).
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.name
    }
}

/// A raw value in a memory slot — pre-binding analogue of [`crate::Value`].
///
/// Differs from `Value` in two ways:
///
/// - `RawSymbol` carries a [`RawSymbolName`] rather than a resolved
///   `SymbolId`.
/// - Extra variants that only exist in surface syntax: `TypedSymbol`
///   (the `@name:Kind` annotation), `Bareword` (predicate or string
///   literal depending on slot), `List` (parenthesized lists like
///   `participants` / `derived_from`), and `Nil`.
#[derive(Clone, Debug, PartialEq)]
pub enum RawValue {
    /// `@name`.
    RawSymbol(RawSymbolName),
    /// `@name:Kind`.
    TypedSymbol {
        /// Symbol name without the `@`.
        name: RawSymbolName,
        /// Kind annotation without the leading `:`.
        kind: String,
    },
    /// A bareword. In predicate slots resolves to a `Predicate`-kind
    /// symbol; elsewhere resolves to a string literal (`Value::String`).
    Bareword(String),
    /// A quoted UTF-8 string literal.
    String(String),
    /// A signed integer.
    Integer(i64),
    /// An IEEE 754 binary64 float.
    Float(f64),
    /// A boolean.
    Boolean(bool),
    /// `nil` — represents `Option::None` in nullable positions.
    Nil,
    /// A parenthesized `(...)` list — used for `participants`,
    /// `derived_from`, and similar multi-value slots. Each element is
    /// itself a [`RawValue`].
    List(Vec<RawValue>),
    /// A timestamp token — stored as milliseconds. Pre-validated by the
    /// lexer to look like an ISO-8601 timestamp.
    Timestamp(ClockTime),
    /// A raw timestamp text that the lexer could not yet convert to a
    /// `ClockTime` (e.g. because the parser is collecting the value
    /// for a slot where the binder does the conversion). Escape hatch
    /// used by the bind stage; the parser itself always produces
    /// `Timestamp(ClockTime)` where possible.
    TimestampRaw(String),
}

/// Order-insensitive keyword arguments for a form.
///
/// Per `ir-write-surface.md` § 5.0, keyword pairs are collected into a
/// dictionary; the form's production specifies the expected key set.
pub type KeywordArgs = BTreeMap<String, RawValue>;

/// A selector in a `query` form — currently parsed as a `RawValue` for
/// forward compatibility; the read-protocol milestone refines this
/// into a richer query DSL.
pub type QuerySelector = RawValue;

/// An unbound AST form — the parser's output.
///
/// Binding (resolving `RawSymbolName` into `SymbolId`, validating kind
/// annotations against `SymbolKind`, materialising `RawValue` into
/// `Value`) happens in a later stage — see `librarian-pipeline.md` § 3.4.
#[derive(Clone, Debug, PartialEq)]
pub enum UnboundForm {
    /// Semantic memory write — `(sem s p o :src SRC :c CONF :v V)`.
    Sem {
        /// Subject.
        s: RawSymbolName,
        /// Predicate.
        p: RawSymbolName,
        /// Object.
        o: RawValue,
        /// Keyword arguments — must include `src`, `c`, `v`; may include
        /// `projected`.
        keywords: KeywordArgs,
    },
    /// Episodic memory write — `(epi EVENT_ID KIND (PAR*) LOC …)`.
    Epi {
        /// Stable memory ID for this event.
        event_id: RawSymbolName,
        /// Event-type symbol.
        kind: RawSymbolName,
        /// List of participant symbols.
        participants: Vec<RawSymbolName>,
        /// Location symbol.
        location: RawSymbolName,
        /// Expected keys: `at`, `obs`, `src`, `c`.
        keywords: KeywordArgs,
    },
    /// Procedural memory write — `(pro RULE_ID TRIGGER ACTION …)`.
    Pro {
        /// Stable memory ID for this rule.
        rule_id: RawSymbolName,
        /// Trigger — typically a string literal.
        trigger: RawValue,
        /// Action — typically a string literal.
        action: RawValue,
        /// Expected keys: `scp`, `src`, `c`; optional `pre`.
        keywords: KeywordArgs,
    },
    /// Inferential memory write — `(inf s p o (DERIVED*) METHOD …)`.
    Inf {
        /// Subject.
        s: RawSymbolName,
        /// Predicate.
        p: RawSymbolName,
        /// Object.
        o: RawValue,
        /// Parent memory symbols (must be non-empty).
        derived_from: Vec<RawSymbolName>,
        /// Registered inference method symbol.
        method: RawSymbolName,
        /// Expected keys: `c`, `v`; optional `projected`.
        keywords: KeywordArgs,
    },
    /// `(alias @a @b)` — declare two names as aliases.
    Alias {
        /// First symbol.
        a: RawSymbolName,
        /// Second symbol.
        b: RawSymbolName,
    },
    /// `(rename @old @new)` — rename a symbol.
    Rename {
        /// The old canonical name.
        old: RawSymbolName,
        /// The new canonical name.
        new: RawSymbolName,
    },
    /// `(retire @name [:reason STRING])` — soft-retire a symbol.
    Retire {
        /// Target symbol.
        name: RawSymbolName,
        /// Optional `:reason` keyword.
        keywords: KeywordArgs,
    },
    /// `(correct @target_episode … epi body …)` — correct a prior
    /// Episodic memory. The corrected body is itself a parenthesised
    /// Episodic form.
    Correct {
        /// The Episode being corrected.
        target_episode: RawSymbolName,
        /// The corrected Episodic memory (must be an `Epi` form).
        corrected: Box<UnboundForm>,
    },
    /// `(promote @name)` — promote an ephemeral memory to canonical.
    Promote {
        /// The ephemeral memory symbol.
        name: RawSymbolName,
    },
    /// `(query … keyword args …)` — read-path query.
    ///
    /// v1 parser treats the body as a keyword-arg bag; selector is an
    /// optional single positional. Detailed query DSL validation is in
    /// `read-protocol.md` and will land with the read-protocol
    /// milestone.
    Query {
        /// Optional positional selector — a symbol or list.
        selector: Option<QuerySelector>,
        /// Remaining keyword arguments.
        keywords: KeywordArgs,
    },
    /// `(episode :start [:label S] [:parent_episode @E] [:retracts (@E1 …)])`
    /// or `(episode :close)` — explicit Episode-boundary directive.
    ///
    /// `:close` is a no-op under the single-`compile_batch`-per-Episode
    /// model (the batch closes the Episode implicitly); the form is
    /// still accepted so agents can emit it spec-compliantly.
    ///
    /// Note on `:retracts`: the spec text uses `[ … ]` brackets
    /// (§ 9.1), but Mimir's write surface doesn't tokenize brackets.
    /// The implementation accepts parenthesised symbol lists —
    /// `:retracts (@E1 @E2)` — matching the existing list convention
    /// used by Epi's participants and Inf's `derived_from`.
    Episode {
        /// Whether this form opens or closes an Episode.
        action: EpisodeAction,
        /// Optional human-readable label (spec § 4.3 — capped at 256
        /// bytes; the semantic stage enforces).
        label: Option<String>,
        /// Optional parent Episode symbol (spec § 5.1).
        parent_episode: Option<RawSymbolName>,
        /// Zero or more Episodes this Episode retracts (spec § 5.2).
        retracts: Vec<RawSymbolName>,
    },
    /// Pin / unpin / authoritative flag write — one of the four
    /// `(pin @mem :actor @A)` / `(unpin @mem :actor @A)` /
    /// `(authoritative-set @mem :actor @A)` /
    /// `(authoritative-clear @mem :actor @A)` forms per
    /// `confidence-decay.md` §§ 7 / 8 and `ir-canonical-form.md`
    /// opcodes `0x35`–`0x38`.
    Flag {
        /// Which flag operation this form carries.
        action: FlagAction,
        /// The memory the flag applies to.
        memory: RawSymbolName,
        /// The agent or user invoking the flag change — required
        /// for audit. Must resolve to an `Agent`-kind symbol at
        /// bind time.
        actor: RawSymbolName,
    },
}

/// Which Episode-boundary action a `(episode …)` form carries.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum EpisodeAction {
    /// `(episode :start …)` — opens an Episode with optional metadata.
    Start,
    /// `(episode :close)` — closes the current Episode. No-op under
    /// the single-batch-per-Episode model; accepted for spec parity.
    Close,
}

/// Which flag a `(pin …)` / `(unpin …)` /
/// `(authoritative-set …)` / `(authoritative-clear …)` form
/// operates on. Emitted into `FlagEventRecord`s at the canonical
/// layer per `ir-canonical-form.md` opcodes `0x35`–`0x38`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum FlagAction {
    /// `(pin @mem :actor @agent)` — suspends decay (`confidence-decay.md` § 7).
    Pin,
    /// `(unpin @mem :actor @agent)` — resumes decay.
    Unpin,
    /// `(authoritative-set @mem :actor @operator)` — operator-authoritative flag on.
    AuthoritativeSet,
    /// `(authoritative-clear @mem :actor @operator)` — operator-authoritative flag off.
    AuthoritativeClear,
}

/// Errors produced by [`parse`].
///
/// Per `ir-write-surface.md` § 8 — fail-fast on first violation, no
/// partial recovery.
#[derive(Debug, Error, PartialEq)]
pub enum ParseError {
    /// The lexer failed before the parser could start.
    #[error("lex error: {0}")]
    Lex(#[from] LexError),

    /// Got a token that isn't allowed here.
    #[error("unexpected token {found:?} at {pos:?}; expected {expected}")]
    UnexpectedToken {
        /// The token we saw.
        found: Token,
        /// Human-readable description of what was expected.
        expected: &'static str,
        /// Position of the token.
        pos: Position,
    },

    /// Input ended before the parser could complete a form.
    #[error("unexpected end of input; expected {expected}")]
    UnexpectedEof {
        /// Human-readable description of what was expected.
        expected: &'static str,
    },

    /// The opcode at the head of a form isn't one of the registered ten.
    #[error("unknown opcode {found:?} at {pos:?}")]
    UnknownOpcode {
        /// The offending opcode text.
        found: String,
        /// Position of the opcode bareword.
        pos: Position,
    },

    /// A form received an unexpected keyword.
    #[error("unexpected keyword {keyword:?} for form {form:?} at {pos:?}")]
    BadKeyword {
        /// The offending keyword.
        keyword: String,
        /// The form being parsed.
        form: &'static str,
        /// Position of the keyword.
        pos: Position,
    },

    /// A keyword appears twice in the same form.
    #[error("duplicate keyword {keyword:?} at {pos:?}")]
    DuplicateKeyword {
        /// The offending keyword.
        keyword: String,
        /// Position of the second occurrence.
        pos: Position,
    },

    /// A form is missing a required keyword.
    #[error("missing required keyword {missing:?} for form {form:?}")]
    MissingRequiredKeyword {
        /// The missing keyword.
        missing: &'static str,
        /// The form being parsed.
        form: &'static str,
    },

    /// A form has the wrong number of positional arguments.
    #[error("arity mismatch for {form:?}: expected {expected}, found {found} at {pos:?}")]
    ArityMismatch {
        /// The form being parsed.
        form: &'static str,
        /// Expected positional arity.
        expected: usize,
        /// Actual positional arity.
        found: usize,
        /// Position of the form's opening paren.
        pos: Position,
    },

    /// A `RawValue` was expected but a non-value token was found.
    #[error("expected value at {pos:?}, got {found:?}")]
    ExpectedValue {
        /// The offending token.
        found: Token,
        /// Position.
        pos: Position,
    },

    /// A list parse saw unbalanced parens.
    #[error("unbalanced list at {pos:?}")]
    UnbalancedList {
        /// Start position of the list.
        pos: Position,
    },

    /// A list position required symbols but saw a non-symbol value.
    #[error("expected symbol list element at {pos:?}, got {found:?}")]
    ExpectedSymbolInList {
        /// The offending value.
        found: RawValue,
        /// Position of the list.
        pos: Position,
    },

    /// An ISO timestamp value could not be normalised to a [`ClockTime`].
    #[error("invalid timestamp {text:?} at {pos:?}")]
    InvalidTimestamp {
        /// The raw timestamp text.
        text: String,
        /// Position of the timestamp.
        pos: Position,
    },

    /// Parser nesting exceeded [`MAX_NESTING_DEPTH`]. Surfaced before
    /// the recursive descent blows the host stack — closes Security
    /// F3 (P2) from the v1.1 fresh assessment. Triggered by inputs
    /// like `(((…)))` of pathological depth, whether through nested
    /// list values (`parse_value` → `parse_value_list_body`) or
    /// through nested `correct` forms (`parse_correct` →
    /// `parse_form`).
    #[error("nesting too deep at {pos:?}: limit is {max}")]
    NestingTooDeep {
        /// Position where the over-limit nesting was attempted.
        pos: Position,
        /// The hard limit (currently [`MAX_NESTING_DEPTH`]).
        max: usize,
    },
}

/// Maximum recursion depth permitted in the recursive-descent parser.
///
/// Mimir's grammar has no legitimate use case beyond a few nesting
/// levels (a form might contain a list value containing a list value),
/// so 256 is generous by orders of magnitude. A flat ~8 MiB main-thread
/// stack on Linux blows around 5–10k of these frames; capping at 256
/// keeps the worst-case stack consumption well under 1 MiB.
pub const MAX_NESTING_DEPTH: usize = 256;

/// Parse a UTF-8 input into a sequence of [`UnboundForm`]s.
///
/// # Errors
///
/// Returns the first [`ParseError`] encountered. No partial recovery.
///
/// # Examples
///
/// ```
/// # #![allow(clippy::unwrap_used)]
/// use mimir_core::parse::parse;
///
/// let forms = parse("(promote @ephemeral_42)").unwrap();
/// assert_eq!(forms.len(), 1);
/// ```
pub fn parse(input: &str) -> Result<Vec<UnboundForm>, ParseError> {
    let tokens = crate::lex::tokenize(input)?;
    let mut parser = Parser::new(tokens);
    let mut out = Vec::new();
    while parser.peek().is_some() {
        out.push(parser.parse_form()?);
    }
    Ok(out)
}

struct Parser {
    tokens: Vec<Spanned>,
    idx: usize,
    /// Current recursive-descent depth. Bounded by [`MAX_NESTING_DEPTH`]
    /// — see [`Parser::parse_value`]'s `LParen` branch and
    /// [`Parser::parse_correct`] for the two recursion sites that
    /// increment/decrement this.
    depth: usize,
}

impl Parser {
    fn new(tokens: Vec<Spanned>) -> Self {
        Self {
            tokens,
            idx: 0,
            depth: 0,
        }
    }

    fn peek(&self) -> Option<&Spanned> {
        self.tokens.get(self.idx)
    }

    fn bump(&mut self) -> Option<Spanned> {
        let t = self.tokens.get(self.idx).cloned()?;
        self.idx += 1;
        Some(t)
    }

    fn expect_lparen(&mut self, expected: &'static str) -> Result<Position, ParseError> {
        let Some(spanned) = self.bump() else {
            return Err(ParseError::UnexpectedEof { expected });
        };
        if spanned.token == Token::LParen {
            Ok(spanned.position)
        } else {
            Err(ParseError::UnexpectedToken {
                found: spanned.token,
                expected,
                pos: spanned.position,
            })
        }
    }

    fn expect_rparen(&mut self, expected: &'static str) -> Result<(), ParseError> {
        let Some(spanned) = self.bump() else {
            return Err(ParseError::UnexpectedEof { expected });
        };
        if spanned.token == Token::RParen {
            Ok(())
        } else {
            Err(ParseError::UnexpectedToken {
                found: spanned.token,
                expected,
                pos: spanned.position,
            })
        }
    }

    fn expect_symbol(&mut self, expected: &'static str) -> Result<RawSymbolName, ParseError> {
        let Some(spanned) = self.bump() else {
            return Err(ParseError::UnexpectedEof { expected });
        };
        match spanned.token {
            Token::Symbol(name) => Ok(RawSymbolName::new(name)),
            Token::TypedSymbol { name, kind } => Ok(RawSymbolName::with_kind(name, kind)),
            other => Err(ParseError::UnexpectedToken {
                found: other,
                expected,
                pos: spanned.position,
            }),
        }
    }

    /// Accept a symbol OR a bareword in predicate slots. Per
    /// `ir-write-surface.md` § 10, predicates may omit the `@`; the
    /// binder normalises both to a `Predicate`-kind symbol.
    fn expect_predicate(&mut self, expected: &'static str) -> Result<RawSymbolName, ParseError> {
        let Some(spanned) = self.bump() else {
            return Err(ParseError::UnexpectedEof { expected });
        };
        match spanned.token {
            Token::Symbol(name) | Token::Bareword(name) => Ok(RawSymbolName::new(name)),
            Token::TypedSymbol { name, kind } => Ok(RawSymbolName::with_kind(name, kind)),
            other => Err(ParseError::UnexpectedToken {
                found: other,
                expected,
                pos: spanned.position,
            }),
        }
    }

    fn parse_form(&mut self) -> Result<UnboundForm, ParseError> {
        let open = self.expect_lparen("top-level `(`")?;
        let Some(head) = self.bump() else {
            return Err(ParseError::UnexpectedEof {
                expected: "opcode after `(`",
            });
        };
        let opcode = match head.token {
            Token::Bareword(ref b) => b.clone(),
            other => {
                return Err(ParseError::UnexpectedToken {
                    found: other,
                    expected: "opcode bareword at form head",
                    pos: head.position,
                });
            }
        };
        match opcode.as_str() {
            "sem" => self.parse_sem(open),
            "epi" => self.parse_epi(open),
            "pro" => self.parse_pro(open),
            "inf" => self.parse_inf(open),
            "alias" => self.parse_alias(),
            "rename" => self.parse_rename(),
            "retire" => self.parse_retire(),
            "correct" => self.parse_correct(open),
            "promote" => self.parse_promote(),
            "query" => self.parse_query(),
            "episode" => self.parse_episode(),
            "pin" => self.parse_flag(FlagAction::Pin, "pin"),
            "unpin" => self.parse_flag(FlagAction::Unpin, "unpin"),
            // Spec text uses `(authoritative-set @mem)` but the
            // bareword grammar is `[a-z_][a-z0-9_]*` (no hyphens),
            // so the surface accepts underscores.
            "authoritative_set" => {
                self.parse_flag(FlagAction::AuthoritativeSet, "authoritative_set")
            }
            "authoritative_clear" => {
                self.parse_flag(FlagAction::AuthoritativeClear, "authoritative_clear")
            }
            _ => Err(ParseError::UnknownOpcode {
                found: opcode,
                pos: head.position,
            }),
        }
    }

    // ---- individual form productions ----

    fn parse_sem(&mut self, _open: Position) -> Result<UnboundForm, ParseError> {
        let s = self.expect_symbol("sem subject")?;
        let p = self.expect_predicate("sem predicate")?;
        let o = self.parse_value("sem object")?;
        let keywords = self.parse_keywords("sem", &["src", "c", "v", "projected"])?;
        Self::require_keywords("sem", &keywords, &["src", "c", "v"])?;
        self.expect_rparen("closing `)` for sem")?;
        Ok(UnboundForm::Sem { s, p, o, keywords })
    }

    fn parse_epi(&mut self, open: Position) -> Result<UnboundForm, ParseError> {
        let event_id = self.expect_symbol("epi event_id")?;
        let kind = self.expect_symbol("epi kind")?;
        let participants = self.parse_symbol_list(open, "epi participants")?;
        let location = self.expect_symbol("epi location")?;
        let keywords = self.parse_keywords("epi", &["at", "obs", "src", "c"])?;
        Self::require_keywords("epi", &keywords, &["at", "obs", "src", "c"])?;
        self.expect_rparen("closing `)` for epi")?;
        Ok(UnboundForm::Epi {
            event_id,
            kind,
            participants,
            location,
            keywords,
        })
    }

    fn parse_pro(&mut self, _open: Position) -> Result<UnboundForm, ParseError> {
        let rule_id = self.expect_symbol("pro rule_id")?;
        let trigger = self.parse_value("pro trigger")?;
        let action = self.parse_value("pro action")?;
        let keywords = self.parse_keywords("pro", &["scp", "src", "c", "pre"])?;
        Self::require_keywords("pro", &keywords, &["scp", "src", "c"])?;
        self.expect_rparen("closing `)` for pro")?;
        Ok(UnboundForm::Pro {
            rule_id,
            trigger,
            action,
            keywords,
        })
    }

    fn parse_inf(&mut self, open: Position) -> Result<UnboundForm, ParseError> {
        let s = self.expect_symbol("inf subject")?;
        let p = self.expect_predicate("inf predicate")?;
        let o = self.parse_value("inf object")?;
        let derived_from = self.parse_symbol_list(open, "inf derived_from")?;
        let method = self.expect_symbol("inf method")?;
        let keywords = self.parse_keywords("inf", &["c", "v", "projected"])?;
        Self::require_keywords("inf", &keywords, &["c", "v"])?;
        self.expect_rparen("closing `)` for inf")?;
        Ok(UnboundForm::Inf {
            s,
            p,
            o,
            derived_from,
            method,
            keywords,
        })
    }

    fn parse_alias(&mut self) -> Result<UnboundForm, ParseError> {
        let a = self.expect_symbol("alias first arg")?;
        let b = self.expect_symbol("alias second arg")?;
        self.expect_rparen("closing `)` for alias")?;
        Ok(UnboundForm::Alias { a, b })
    }

    fn parse_rename(&mut self) -> Result<UnboundForm, ParseError> {
        let old = self.expect_symbol("rename old name")?;
        let new = self.expect_symbol("rename new name")?;
        self.expect_rparen("closing `)` for rename")?;
        Ok(UnboundForm::Rename { old, new })
    }

    fn parse_retire(&mut self) -> Result<UnboundForm, ParseError> {
        let name = self.expect_symbol("retire target")?;
        let keywords = self.parse_keywords("retire", &["reason"])?;
        self.expect_rparen("closing `)` for retire")?;
        Ok(UnboundForm::Retire { name, keywords })
    }

    fn parse_correct(&mut self, open: Position) -> Result<UnboundForm, ParseError> {
        let target_episode = self.expect_symbol("correct target_episode")?;
        // The corrected body must be a parenthesised Epi form. Bound
        // recursion depth: nested `(correct (correct (correct …)))`
        // would otherwise blow the stack via parse_form (Security F3).
        if self.depth >= MAX_NESTING_DEPTH {
            return Err(ParseError::NestingTooDeep {
                pos: open,
                max: MAX_NESTING_DEPTH,
            });
        }
        self.depth += 1;
        let inner = self.parse_form();
        self.depth -= 1;
        let corrected = Box::new(inner?);
        if !matches!(&*corrected, UnboundForm::Epi { .. }) {
            return Err(ParseError::UnexpectedToken {
                found: Token::LParen,
                expected: "corrected body must be an `epi` form",
                pos: Position::start(),
            });
        }
        self.expect_rparen("closing `)` for correct")?;
        Ok(UnboundForm::Correct {
            target_episode,
            corrected,
        })
    }

    fn parse_promote(&mut self) -> Result<UnboundForm, ParseError> {
        let name = self.expect_symbol("promote target")?;
        self.expect_rparen("closing `)` for promote")?;
        Ok(UnboundForm::Promote { name })
    }

    #[allow(clippy::too_many_lines)]
    fn parse_episode(&mut self) -> Result<UnboundForm, ParseError> {
        // First keyword must be either `:start` or `:close` — no
        // value, just a flag token. Custom-parsed because the normal
        // `parse_keywords` helper expects `:key value` pairs.
        let head = self.bump().ok_or(ParseError::UnexpectedEof {
            expected: "`:start` or `:close`",
        })?;
        let action_name = match head.token {
            Token::Keyword(name) => name,
            other => {
                return Err(ParseError::UnexpectedToken {
                    found: other,
                    expected: "`:start` or `:close`",
                    pos: head.position,
                });
            }
        };
        let action = match action_name.as_str() {
            "start" => EpisodeAction::Start,
            "close" => EpisodeAction::Close,
            other => {
                return Err(ParseError::BadKeyword {
                    keyword: other.to_string(),
                    form: "episode",
                    pos: head.position,
                });
            }
        };

        if matches!(action, EpisodeAction::Close) {
            // `(episode :close)` accepts no further keywords.
            self.expect_rparen("closing `)` for episode :close")?;
            return Ok(UnboundForm::Episode {
                action,
                label: None,
                parent_episode: None,
                retracts: Vec::new(),
            });
        }

        // `:start` — parse optional `:label`, `:parent_episode`,
        // `:retracts` in any order. Duplicates reject.
        let mut label: Option<String> = None;
        let mut parent_episode: Option<RawSymbolName> = None;
        let mut retracts: Option<Vec<RawSymbolName>> = None;

        while let Some(spanned) = self.peek() {
            match &spanned.token {
                Token::RParen => break,
                Token::Keyword(k) => {
                    let key = k.clone();
                    let pos = spanned.position;
                    self.bump();
                    match key.as_str() {
                        "label" => {
                            if label.is_some() {
                                return Err(ParseError::DuplicateKeyword { keyword: key, pos });
                            }
                            let Some(v) = self.bump() else {
                                return Err(ParseError::UnexpectedEof {
                                    expected: "`:label` string value",
                                });
                            };
                            match v.token {
                                Token::String(s) => label = Some(s),
                                other => {
                                    return Err(ParseError::UnexpectedToken {
                                        found: other,
                                        expected: "string literal for `:label`",
                                        pos: v.position,
                                    });
                                }
                            }
                        }
                        "parent_episode" => {
                            if parent_episode.is_some() {
                                return Err(ParseError::DuplicateKeyword { keyword: key, pos });
                            }
                            parent_episode = Some(self.expect_symbol("`:parent_episode` symbol")?);
                        }
                        "retracts" => {
                            if retracts.is_some() {
                                return Err(ParseError::DuplicateKeyword { keyword: key, pos });
                            }
                            // Parenthesised symbol list — matches the
                            // convention from Epi's participants and
                            // Inf's derived_from.
                            let list_open = self.expect_lparen("`:retracts (`")?;
                            retracts = Some(self.parse_retracts_list(list_open)?);
                        }
                        _ => {
                            return Err(ParseError::BadKeyword {
                                form: "episode :start",
                                keyword: key,
                                pos,
                            });
                        }
                    }
                }
                _ => {
                    let t = spanned.token.clone();
                    let pos = spanned.position;
                    return Err(ParseError::UnexpectedToken {
                        found: t,
                        expected: "keyword argument in `episode :start`",
                        pos,
                    });
                }
            }
        }

        self.expect_rparen("closing `)` for episode :start")?;
        Ok(UnboundForm::Episode {
            action,
            label,
            parent_episode,
            retracts: retracts.unwrap_or_default(),
        })
    }

    fn parse_flag(
        &mut self,
        action: FlagAction,
        form: &'static str,
    ) -> Result<UnboundForm, ParseError> {
        // `(<opcode> @memory :actor @agent)` — memory positional,
        // `:actor` required per confidence-decay.md § 7 / § 8
        // audit-trail contract.
        let memory = self.expect_symbol(match action {
            FlagAction::Pin => "pin target",
            FlagAction::Unpin => "unpin target",
            FlagAction::AuthoritativeSet => "authoritative_set target",
            FlagAction::AuthoritativeClear => "authoritative_clear target",
        })?;
        let keywords = self.parse_keywords(form, &["actor"])?;
        Self::require_keywords(form, &keywords, &["actor"])?;
        self.expect_rparen("closing `)` for flag form")?;
        let actor = match keywords.get("actor") {
            Some(RawValue::RawSymbol(s) | RawValue::TypedSymbol { name: s, .. }) => s.clone(),
            _ => {
                return Err(ParseError::BadKeyword {
                    keyword: "actor".into(),
                    form,
                    pos: Position::start(),
                });
            }
        };
        Ok(UnboundForm::Flag {
            action,
            memory,
            actor,
        })
    }

    fn parse_retracts_list(&mut self, open: Position) -> Result<Vec<RawSymbolName>, ParseError> {
        let raw = self.parse_value_list_body(open)?;
        raw.into_iter()
            .map(|v| match v {
                RawValue::RawSymbol(name) | RawValue::TypedSymbol { name, .. } => Ok(name),
                other => Err(ParseError::ExpectedSymbolInList {
                    found: other,
                    pos: open,
                }),
            })
            .collect()
    }

    fn parse_query(&mut self) -> Result<UnboundForm, ParseError> {
        // Optional positional selector: a value (symbol or list) that
        // is NOT a keyword. If the next token is `:`, skip selector.
        let selector = if matches!(self.peek().map(|s| &s.token), Some(Token::Keyword(_)))
            || matches!(self.peek().map(|s| &s.token), Some(Token::RParen))
        {
            None
        } else {
            Some(self.parse_value("query selector")?)
        };
        let keywords = self.parse_keywords(
            "query",
            &[
                "kind",
                "s",
                "p",
                "o",
                "in_episode",
                "after_episode",
                "before_episode",
                "episode_chain",
                "as_of",
                "as_committed",
                "include_retired",
                "include_projected",
                "confidence_threshold",
                "limit",
                "explain_filtered",
                "show_framing",
                "debug_mode",
                "read_after",
                "timeout_ms",
            ],
        )?;
        self.expect_rparen("closing `)` for query")?;
        Ok(UnboundForm::Query { selector, keywords })
    }

    // ---- shared helpers ----

    fn parse_value(&mut self, expected: &'static str) -> Result<RawValue, ParseError> {
        let Some(spanned) = self.bump() else {
            return Err(ParseError::UnexpectedEof { expected });
        };
        match spanned.token {
            Token::Symbol(name) => Ok(RawValue::RawSymbol(RawSymbolName::new(name))),
            Token::TypedSymbol { name, kind } => Ok(RawValue::TypedSymbol {
                name: RawSymbolName::new(name),
                kind,
            }),
            Token::Bareword(b) => Ok(RawValue::Bareword(b)),
            Token::String(s) => Ok(RawValue::String(s)),
            Token::Integer(i) => Ok(RawValue::Integer(i)),
            Token::Float(f) => Ok(RawValue::Float(f)),
            Token::Boolean(b) => Ok(RawValue::Boolean(b)),
            Token::Nil => Ok(RawValue::Nil),
            Token::Timestamp(text) => parse_timestamp(&text, spanned.position)
                .map(RawValue::Timestamp)
                .or(Ok(RawValue::TimestampRaw(text))),
            Token::LParen => {
                // Bound stack consumption: each nested LParen recurses
                // through parse_value_list_body → parse_value → here.
                // Without this guard, `(((…)))` of pathological depth
                // exhausts the host stack uncatchably (Security F3).
                if self.depth >= MAX_NESTING_DEPTH {
                    return Err(ParseError::NestingTooDeep {
                        pos: spanned.position,
                        max: MAX_NESTING_DEPTH,
                    });
                }
                self.depth += 1;
                let result = self.parse_value_list_body(spanned.position);
                self.depth -= 1;
                let inner = result?;
                Ok(RawValue::List(inner))
            }
            other @ (Token::RParen | Token::Keyword(_)) => Err(ParseError::ExpectedValue {
                found: other,
                pos: spanned.position,
            }),
        }
    }

    fn parse_value_list_body(&mut self, open: Position) -> Result<Vec<RawValue>, ParseError> {
        let mut out = Vec::new();
        loop {
            match self.peek().map(|s| &s.token) {
                None => return Err(ParseError::UnbalancedList { pos: open }),
                Some(Token::RParen) => {
                    self.bump();
                    return Ok(out);
                }
                _ => {
                    out.push(self.parse_value("list element")?);
                }
            }
        }
    }

    fn parse_symbol_list(
        &mut self,
        _open: Position,
        expected: &'static str,
    ) -> Result<Vec<RawSymbolName>, ParseError> {
        let list_open = self.expect_lparen(expected)?;
        let raw = self.parse_value_list_body(list_open)?;
        raw.into_iter()
            .map(|v| match v {
                RawValue::RawSymbol(name) | RawValue::TypedSymbol { name, .. } => Ok(name),
                other => Err(ParseError::ExpectedSymbolInList {
                    found: other,
                    pos: list_open,
                }),
            })
            .collect()
    }

    fn parse_keywords(
        &mut self,
        form: &'static str,
        allowed: &[&str],
    ) -> Result<KeywordArgs, ParseError> {
        let mut out = BTreeMap::new();
        while let Some(spanned) = self.peek() {
            match &spanned.token {
                Token::RParen => break,
                Token::Keyword(k) => {
                    let key = k.clone();
                    let pos = spanned.position;
                    if !allowed.iter().any(|allowed| *allowed == key) {
                        return Err(ParseError::BadKeyword {
                            keyword: key,
                            form,
                            pos,
                        });
                    }
                    self.bump(); // consume the keyword token
                    let value = self.parse_value("keyword value")?;
                    if out.insert(key.clone(), value).is_some() {
                        return Err(ParseError::DuplicateKeyword { keyword: key, pos });
                    }
                }
                other => {
                    return Err(ParseError::UnexpectedToken {
                        found: other.clone(),
                        expected: "`:keyword value` pair or closing `)`",
                        pos: spanned.position,
                    });
                }
            }
        }
        Ok(out)
    }

    fn require_keywords(
        form: &'static str,
        keywords: &KeywordArgs,
        required: &[&'static str],
    ) -> Result<(), ParseError> {
        for k in required {
            if !keywords.contains_key(*k) {
                return Err(ParseError::MissingRequiredKeyword { missing: k, form });
            }
        }
        Ok(())
    }
}

fn parse_timestamp(text: &str, pos: Position) -> Result<ClockTime, ParseError> {
    // Accept YYYY-MM-DD (midnight UTC) and YYYY-MM-DDTHH:MM:SS[Z|.frac Z].
    // Returns ms since Unix epoch.
    let bad = || ParseError::InvalidTimestamp {
        text: text.to_string(),
        pos,
    };
    if text.len() == 10 {
        // Date-only: YYYY-MM-DD → midnight UTC.
        let millis = date_to_millis(text).ok_or_else(bad)?;
        return ClockTime::try_from_millis(millis).map_err(|_| bad());
    }
    // Full date-time. Expect 'T' at offset 10.
    if text.len() < 20 || !text.is_char_boundary(10) || &text[10..11] != "T" {
        return Err(bad());
    }
    let (date_part, rest) = text.split_at(10);
    // `rest` = "THH:MM:SS[.frac]Z" or similar.
    let rest = rest
        .strip_prefix('T')
        .ok_or_else(bad)?
        .trim_end_matches('Z');
    let (hms_part, frac_millis) = if let Some(dot) = rest.find('.') {
        let (hms, frac) = rest.split_at(dot);
        let frac = &frac[1..];
        if frac.is_empty() || !frac.chars().all(|c| c.is_ascii_digit()) {
            return Err(bad());
        }
        let millis_str = if frac.len() >= 3 { &frac[..3] } else { frac };
        let mut millis: u64 = millis_str.parse().map_err(|_| bad())?;
        // Pad to 3 digits: e.g. "5" → 500ms, "50" → 500ms, "500" → 500ms.
        for _ in millis_str.len()..3 {
            millis *= 10;
        }
        (hms, millis)
    } else {
        (rest, 0_u64)
    };
    let parts: Vec<&str> = hms_part.split(':').collect();
    if parts.len() != 3 {
        return Err(bad());
    }
    let hours: u64 = parts[0].parse().map_err(|_| bad())?;
    let minutes: u64 = parts[1].parse().map_err(|_| bad())?;
    let seconds: u64 = parts[2].parse().map_err(|_| bad())?;
    let date_millis = date_to_millis(date_part).ok_or_else(bad)?;
    let total = date_millis + hours * 3_600_000 + minutes * 60_000 + seconds * 1_000 + frac_millis;
    ClockTime::try_from_millis(total).map_err(|_| bad())
}

fn date_to_millis(date: &str) -> Option<u64> {
    // Proleptic-Gregorian conversion for 1970-01-01 through 9999-12-31.
    // Keeps the dependency surface minimal — no chrono in foundations.
    if date.len() != 10 {
        return None;
    }
    let b = date.as_bytes();
    if b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    let year: i64 = std::str::from_utf8(&b[..4]).ok()?.parse().ok()?;
    let month: u32 = std::str::from_utf8(&b[5..7]).ok()?.parse().ok()?;
    let day: u32 = std::str::from_utf8(&b[8..10]).ok()?.parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }

    // Days since 1970-01-01 via Howard Hinnant's civil_from_days inverse.
    let year_adjusted = if month <= 2 { year - 1 } else { year };
    let era = if year_adjusted >= 0 {
        year_adjusted
    } else {
        year_adjusted - 399
    } / 400;
    let year_of_era: u32 = u32::try_from(year_adjusted - era * 400).ok()?;
    let day_of_year = (153_u32 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146_097 + i64::from(day_of_era) - 719_468;
    if days < 0 {
        return None;
    }
    let millis = u64::try_from(days).ok()? * 86_400_000;
    Some(millis)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn promote_form_is_single_symbol() {
        let forms = parse("(promote @scratch_42)").unwrap();
        assert_eq!(
            forms[0],
            UnboundForm::Promote {
                name: RawSymbolName::new("scratch_42"),
            }
        );
    }

    #[test]
    fn alias_and_rename() {
        let a = parse("(alias @a @b)").unwrap();
        assert_eq!(
            a[0],
            UnboundForm::Alias {
                a: RawSymbolName::new("a"),
                b: RawSymbolName::new("b"),
            }
        );
        let r = parse("(rename @old @new)").unwrap();
        assert_eq!(
            r[0],
            UnboundForm::Rename {
                old: RawSymbolName::new("old"),
                new: RawSymbolName::new("new"),
            }
        );
    }

    #[test]
    fn sem_form_with_all_required_keywords() {
        let src = r#"(sem @alice email "alice@example.com" :src @profile :c 0.95 :v 2024-01-15)"#;
        let forms = parse(src).unwrap();
        let UnboundForm::Sem { s, p, o, keywords } = &forms[0] else {
            panic!("expected sem form");
        };
        assert_eq!(s, &RawSymbolName::new("alice"));
        assert_eq!(p, &RawSymbolName::new("email"));
        assert_eq!(o, &RawValue::String("alice@example.com".into()));
        assert!(keywords.contains_key("src"));
        assert!(keywords.contains_key("c"));
        assert!(keywords.contains_key("v"));
        assert!(matches!(keywords.get("v"), Some(RawValue::Timestamp(_))));
    }

    #[test]
    fn sem_missing_required_keyword_errors() {
        let src = r#"(sem @alice email "a" :src @profile :c 0.95)"#;
        let err = parse(src).unwrap_err();
        assert!(matches!(
            err,
            ParseError::MissingRequiredKeyword {
                missing: "v",
                form: "sem"
            }
        ));
    }

    #[test]
    fn unknown_opcode_errors() {
        let err = parse("(xyz @a @b)").unwrap_err();
        assert!(matches!(err, ParseError::UnknownOpcode { .. }));
    }

    #[test]
    fn unknown_keyword_errors() {
        let src = r#"(sem @a b "x" :src @y :c 0.5 :v 2024-01-15 :bogus 1)"#;
        let err = parse(src).unwrap_err();
        assert!(matches!(err, ParseError::BadKeyword { .. }));
    }

    #[test]
    fn duplicate_keyword_errors() {
        let src = r#"(sem @a b "x" :src @y :src @y :c 0.5 :v 2024-01-15)"#;
        let err = parse(src).unwrap_err();
        assert!(matches!(err, ParseError::DuplicateKeyword { .. }));
    }

    #[test]
    fn epi_parses_participants_list() {
        let src = r"(epi @ep_001 @rename (@old @new) @github
            :at 2026-04-17T10:00:00Z :obs 2026-04-17T10:00:00Z
            :src @alice :c 1.0)";
        let forms = parse(src).unwrap();
        let UnboundForm::Epi {
            event_id,
            kind,
            participants,
            location,
            ..
        } = &forms[0]
        else {
            panic!("expected epi form");
        };
        assert_eq!(event_id, &RawSymbolName::new("ep_001"));
        assert_eq!(kind, &RawSymbolName::new("rename"));
        assert_eq!(participants.len(), 2);
        assert_eq!(participants[0], RawSymbolName::new("old"));
        assert_eq!(location, &RawSymbolName::new("github"));
    }

    #[test]
    fn pro_with_optional_precondition() {
        let src = r#"(pro @rule_1 "agent about to write" "route via librarian"
            :pre nil :scp @mimir :src @agents_md :c 1.0)"#;
        let forms = parse(src).unwrap();
        let UnboundForm::Pro {
            rule_id, keywords, ..
        } = &forms[0]
        else {
            panic!("expected pro form");
        };
        assert_eq!(rule_id, &RawSymbolName::new("rule_1"));
        assert_eq!(keywords.get("pre"), Some(&RawValue::Nil));
    }

    #[test]
    fn inf_requires_method_and_derived_from() {
        let src = r"(inf @a p @b (@m1 @m2) @pattern_summarize :c 0.7 :v 2024-03-15)";
        let forms = parse(src).unwrap();
        let UnboundForm::Inf {
            derived_from,
            method,
            ..
        } = &forms[0]
        else {
            panic!("expected inf form");
        };
        assert_eq!(derived_from.len(), 2);
        assert_eq!(method, &RawSymbolName::new("pattern_summarize"));
    }

    #[test]
    fn query_with_keywords_only() {
        let src = "(query :s @alice :p email :debug_mode true)";
        let forms = parse(src).unwrap();
        let UnboundForm::Query {
            selector, keywords, ..
        } = &forms[0]
        else {
            panic!("expected query form");
        };
        assert!(selector.is_none());
        assert_eq!(keywords.get("debug_mode"), Some(&RawValue::Boolean(true)));
    }

    #[test]
    fn query_with_positional_selector() {
        let src = "(query @mem_x)";
        let forms = parse(src).unwrap();
        let UnboundForm::Query {
            selector,
            keywords: _,
        } = &forms[0]
        else {
            panic!("expected query form");
        };
        assert_eq!(
            selector.as_ref(),
            Some(&RawValue::RawSymbol(RawSymbolName::new("mem_x"))),
        );
    }

    #[test]
    fn timestamp_converts_to_clocktime() {
        let src = r#"(sem @a b "x" :src @y :c 0.5 :v 2024-01-15)"#;
        let forms = parse(src).unwrap();
        let UnboundForm::Sem { keywords, .. } = &forms[0] else {
            panic!();
        };
        match keywords.get("v") {
            Some(RawValue::Timestamp(ct)) => {
                // 2024-01-15 = 1705276800 seconds since epoch = 1705276800000 ms.
                assert_eq!(ct.as_millis(), 1_705_276_800_000);
            }
            other => panic!("expected Timestamp, got {other:?}"),
        }
    }

    #[test]
    fn multiple_forms_in_one_input() {
        let src = r"
            (alias @a @b)
            (rename @old @new)
            (promote @tmp)
        ";
        let forms = parse(src).unwrap();
        assert_eq!(forms.len(), 3);
    }

    // ---- Security F3 (P2) regression: parser must not stack-overflow
    // on adversarially-deep nested input. Pre-fix, `parse_value` and
    // `parse_correct` recurred without bound; an input of N nested
    // parens consumed ~N stack frames, blowing the default 8 MiB
    // main-thread stack at a few thousand levels uncatchably (no
    // `Result::Err`, no `ParseError` variant).

    /// Build a Sem form whose object-position value is a `depth`-deep
    /// nested list: `(sem @s @p ((((...0))))  :src @observation :c 0.5
    /// :v 2024-01-15)`. The outermost `(` opens the form (depth not
    /// incremented — it's the form opener, not a value); each
    /// subsequent `(` is one parser nesting level.
    fn nested_value_input(depth: usize) -> String {
        let opens = "(".repeat(depth);
        let closes = ")".repeat(depth);
        format!("(sem @s @p {opens}0{closes} :src @observation :c 0.5 :v 2024-01-15)")
    }

    #[test]
    fn parser_accepts_value_nesting_at_limit() {
        // MAX_NESTING_DEPTH levels deep — the maximum permitted.
        let src = nested_value_input(MAX_NESTING_DEPTH);
        let forms = parse(&src).expect("must accept depth at the limit");
        assert_eq!(forms.len(), 1);
    }

    #[test]
    fn parser_rejects_value_nesting_one_over_limit() {
        // One level too deep.
        let src = nested_value_input(MAX_NESTING_DEPTH + 1);
        let err = parse(&src).expect_err("must reject depth over the limit");
        match err {
            ParseError::NestingTooDeep { max, .. } => {
                assert_eq!(max, MAX_NESTING_DEPTH);
            }
            other => panic!("expected NestingTooDeep, got {other:?}"),
        }
    }

    #[test]
    fn parser_rejects_pathologically_deep_value_nesting_without_stack_overflow() {
        // 10x the limit — pre-fix would have blown the stack
        // uncatchably. Post-fix returns a typed error. Test execution
        // not aborting the process is the load-bearing assertion;
        // checking the error variant is the cherry on top.
        let src = nested_value_input(MAX_NESTING_DEPTH * 10);
        let err = parse(&src).expect_err("must reject pathological nesting");
        assert!(
            matches!(err, ParseError::NestingTooDeep { .. }),
            "expected NestingTooDeep, got {err:?}"
        );
    }

    #[test]
    fn parser_rejects_nested_correct_forms_past_limit() {
        // The other recursion site: parse_correct → parse_form →
        // parse_correct... Build N nested `correct` forms.
        // (correct @e1 (correct @e2 (correct @e3 ... (epi ...))))
        let depth = MAX_NESTING_DEPTH + 1;
        let mut src = String::new();
        for i in 0..depth {
            std::fmt::Write::write_fmt(&mut src, format_args!("(correct @e{i} "))
                .expect("write to String never fails");
        }
        src.push_str("(epi @ev @kind () @loc :at 2024-01-15 :obs 2024-01-15 :src @y :c 0.5)");
        for _ in 0..depth {
            src.push(')');
        }
        let err = parse(&src).expect_err("must reject deep `correct` nesting");
        assert!(
            matches!(err, ParseError::NestingTooDeep { .. }),
            "expected NestingTooDeep, got {err:?}"
        );
    }
}
