# IR Write-Surface Grammar

> **Status: authoritative — graduated 2026-04-17; criterion #4 closed 2026-04-19.** All cited sources verified (see `docs/attribution.md`). Grammar implemented by `mimir_core::lex` (12 token classes, typed `LexError`) + `mimir_core::parse` (twelve top-level forms including `episode` and the four `pin` / `unpin` / `authoritative_set` / `authoritative_clear` flag forms, `UnboundForm` AST, typed `ParseError`). Tests cover every form and every error variant. Criterion #4 (no-panic-on-malformed-input) is met by the `fuzz/` cargo-fuzz targets (`fuzz_lex`, `fuzz_parse`) plus the totality properties `lex_is_total_over_random_utf8` / `parse_is_total_over_random_utf8` in `crates/mimir_core/tests/properties.rs` that run in CI on every push. See [#33](https://github.com/buildepicshit/Mimir/issues/33) for the cargo-fuzz harness details.

This specification defines the formal grammar of the agent → librarian write surface. The write surface is a Lisp S-expression dialect chosen after tokenizer bake-off measurements. Every write, read query, and symbol-table operation an agent emits is a well-formed expression in this grammar.

## 1. Scope

This specification defines:

- Lexical tokens (symbols, barewords, strings, timestamps, numbers, booleans, nil, structural punctuation).
- Top-level forms for memory writes (`sem`, `epi`, `pro`, `inf`) and operations (`alias`, `rename`, `retire`, `correct`, `promote`, `query`).
- Per-form EBNF productions with positional and keyword-argument split.
- Reserved keywords.
- Error taxonomy for parse failures.
- Streaming vs batched input semantics.

This specification does **not** define:

- Canonical bytecode form (what the parsed write compiles to) — `ir-canonical-form.md`.
- Memory type shapes — `memory-type-taxonomy.md`.
- Source-kind taxonomy — `grounding-model.md`.
- Symbol allocation or kind inference — `symbol-identity-semantics.md`.
- Four-clock semantics — `temporal-model.md`.
- Librarian pipeline (lex → parse → bind → semantic → emit) — `librarian-pipeline.md` (this spec defines *what is parseable*; that spec defines *how parsing fits into the pipeline*).
- Agent API contract (synchronous in-process `Store::commit_batch`) — `wire-architecture.md`.

### Graduation criteria

Graduates draft → authoritative when:

1. Related art is verified in `docs/attribution.md` (R7RS Scheme grammar, Clojure EDN, PEG literature).
2. A Rust parser implementing this grammar compiles in `mimir_core`, with the invariants in § 10 covered by unit, property, and fuzz tests.
3. Round-trip tests cover every form (emit → parse → canonical form → decoder → surface) — deterministic equality.
4. Fuzz testing (`cargo-fuzz`) runs on the parser with no panics and deterministic error behavior on malformed input.

## 2. Design thesis: typed grammar over ad-hoc parsing

Agents emit structured writes. A permissive "parse whatever looks reasonable" grammar sacrifices determinism — the same input can produce different parses depending on heuristics, and malformed input silently succeeds with unintended meaning. Mimir rejects this.

The write-surface grammar is:

- **Deterministic.** One string has at most one parse. Ambiguity is eliminated by construction, not broken by precedence rules.
- **Typed at the token level.** Symbols (`@name`), predicates (bareword), timestamps (ISO format), numbers, booleans, and strings are distinct token classes. Ambiguity between classes is resolved by format, not by context.
- **Fail-fast.** Malformed input produces a typed `ParseError::*` at the first violation. No partial recovery, no best-effort reconstruction. Agents retry with a corrected emission.
- **Compile-time-checkable in Rust.** The parser's output is a typed AST matching `memory-type-taxonomy.md` enum variants. A parse success means a type-correct AST; the binder further validates against workspace state.

The write surface is Lisp S-expressions because tokenizer bake-off measurements placed the Lisp family in the token-cheap cluster, parens give structural invariants for deterministic parse-error localization, leading-opcode dispatch is trivial, and LLM emission fluency in Lisp-family syntaxes is high.

## 3. Lexical tokens

The lexer produces a stream of tokens from input bytes.

### 3.1 Token classes

| Token | Pattern | Notes |
|---|---|---|
| `Symbol` | `@[a-z][a-z0-9_]*` | workspace-scoped reference; resolves via symbol table |
| `TypedSymbol` | `@[a-z][a-z0-9_]*:[A-Z][A-Za-z]+` | symbol + kind annotation |
| `Bareword` | `[a-z][a-z0-9_]*` | predicate in Predicate slots, string literal elsewhere, opcode at form head |
| `Opcode` | `[A-Z]+` or keyword `Bareword` | top-level form head — see § 4 |
| `Timestamp` | `\d{4}-\d{2}-\d{2}(T\d{2}:\d{2}:\d{2}(\.\d+)?Z)?` | ISO-8601 UTC; date-only is accepted as midnight UTC |
| `Integer` | `-?\d+` | signed 64-bit |
| `Float` | `-?\d+\.\d+` | 64-bit IEEE 754 |
| `String` | `"([^"\\]\|\\["\\nrt])*"` | double-quoted; backslash escapes `\"`, `\\`, `\n`, `\r`, `\t` |
| `Boolean` | `true` \| `false` | reserved words |
| `Nil` | `nil` | reserved word; encodes `Option::None` in nil-accepting slots |
| `Keyword` | `:[a-z][a-z0-9_]*` | keyword-argument tag for `:key value` pairs |
| `LParen` | `(` | |
| `RParen` | `)` | |
| `Comment` | `;[^\n]*` | line comment; lexer drops |
| `Whitespace` | `[ \t\n]+` | lexer drops |

### 3.2 Token disambiguation rules

- A bareword `email` is an `Opcode` at a form head, a `Predicate` in a predicate slot, a reserved word if it matches one of § 6, otherwise a string literal.
- `true` / `false` / `nil` are always reserved; to emit these strings as literal values, use quoted form (`"true"`).
- A `@name:Kind` sequence is a single `TypedSymbol` token; the lexer does not split it into three.

### 3.3 Character encoding

Input is UTF-8. Quoted string literals may contain arbitrary UTF-8 code points except unescaped `"` and `\`. Bareword identifiers and symbol names are ASCII-restricted per § 3.1.

## 4. Top-level forms

Every valid write-surface expression is a parenthesized form with an opcode at the head:

```
form ::= "(" opcode ... ")"
```

### 4.1 Memory-write opcodes

| Opcode | Target | Defined in |
|---|---|---|
| `sem` | Semantic memory | § 5.1 |
| `epi` | Episodic memory | § 5.2 |
| `pro` | Procedural memory | § 5.3 |
| `inf` | Inferential memory | § 5.4 |

### 4.2 Operation opcodes

| Opcode | Effect | Defined in |
|---|---|---|
| `alias` | Declare `@new` and `@old` as aliases of the same symbol | § 5.5 |
| `rename` | Rename `@old` to `@new`, emitting a `@rename` Episodic event | § 5.6 |
| `retire` | Soft-retire `@name`, emitting a `@retire` Episodic event | § 5.7 |
| `correct` | Emit an Episodic memory correcting a prior Episodic `@target` | § 5.8 |
| `promote` | Promote an ephemeral memory to canonical | § 5.9 |
| `query` | Read-path query | § 5.10 |

### 4.3 Reserved opcode set

```
{ sem, epi, pro, inf, alias, rename, retire, correct, promote, query }
```

These names cannot appear as bareword values in Predicate or EventType slots without a `@` prefix. A write attempting `(sem @X rename @Y :src @Z :c 1.0)` is rejected at lex time because `rename` in the predicate slot is reserved.

## 5. Per-form productions (EBNF)

EBNF uses double-quoted terminals, bare nonterminals, `|` for alternation, `*` for zero-or-more, `+` for one-or-more, `?` for optional, parentheses for grouping.

### 5.0 Common nonterminals

```
Value      ::= Symbol | TypedSymbol | Bareword | String | Integer | Float | Boolean
Nullable   ::= Value | "nil"
SymbolList ::= "(" Symbol* ")"
KwArg      ::= Keyword Value
```

Keyword arguments are **order-insensitive** within a form. The parser collects them into a dictionary and validates against the form's expected keyword set.

### 5.1 `sem` — Semantic memory

```
SemForm ::= "(" "sem" Value Value Value KwArg* ")"
Expected keywords: :src, :c, :v [, :projected]
Required keywords: :src, :c, :v
```

Positional: `s`, `p`, `o`.

Example:
```
(sem @alain email "alice@example.com" :src @profile :c 0.95 :v 2024-01-15)
```

### 5.2 `epi` — Episodic memory

```
EpiForm ::= "(" "epi" Value Value SymbolList Value KwArg* ")"
Expected keywords: :at, :obs, :src, :c
Required keywords: :at, :obs, :src, :c
```

Positional: `event_id`, `kind`, `participants`, `location`.

Example:
```
(epi @ep_001 @rename (@luamemories @mimir) @github
     :at 2026-04-17T10:00:00Z :obs 2026-04-17T10:00:00Z
     :src @alain :c 1.0)
```

### 5.3 `pro` — Procedural memory

```
ProForm ::= "(" "pro" Value Value Value KwArg* ")"
Expected keywords: :pre (optional), :scp, :src, :c
Required keywords: :scp, :src, :c
```

Positional: `rule_id`, `trigger`, `action`.

Example:
```
(pro @proc_001 "agent about to write memory" "route via librarian"
     :scp @mimir :src @agents_md :c 1.0)
```

### 5.4 `inf` — Inferential memory

```
InfForm ::= "(" "inf" Value Value Value SymbolList Value KwArg* ")"
Expected keywords: :c, :v [, :projected]
Required keywords: :c, :v
```

Positional: `s`, `p`, `o`, `derived_from`, `method`.

Example:
```
(inf @alain prefers @coffee (@ep_order_mon @ep_order_tue @ep_order_wed) @pattern_summarize
     :c 0.7 :v 2024-03-15)
```

### 5.5 `alias`

```
AliasForm ::= "(" "alias" Symbol Symbol ")"
```

Declare that `@a` and `@b` resolve to the same symbol. Both become aliases; the canonical name is whichever existed first. Emits an `@alias` Episodic memory (see `symbol-identity-semantics.md` § 7.5).

### 5.6 `rename`

```
RenameForm ::= "(" "rename" Symbol Symbol ")"
```

Rename `@old` to `@new`. `@new` becomes the canonical name; `@old` becomes an alias. Emits an `@rename` Episodic memory.

### 5.7 `retire`

```
RetireForm ::= "(" "retire" Symbol (":reason" String)? ")"
```

Soft-retire `@name`. Emits an `@retire` Episodic memory. Optional `:reason` keyword carries a librarian-visible note.

### 5.8 `correct`

```
CorrectForm ::= "(" "correct" Symbol EpiBody ")"
EpiBody     ::= (body of an epi form without the opcode — agent emits corrected event)
```

Emit a correction to a prior Episodic memory `@target`. The new Episodic carries the corrected content; the librarian records a `Corrects` edge per `temporal-model.md` § 5.3.

### 5.9 `promote`

```
PromoteForm ::= "(" "promote" Symbol ")"
```

Promote the ephemeral memory `@name` to canonical, routing it through the full librarian pipeline.

### 5.10 `query` — read-path

```
QueryForm    ::= "(" "query" QuerySelector KwArg* ")"
QuerySelector ::= Symbol | "(" Symbol Value ")" | "(" Symbol Value Value ")"
Expected keywords: :as_of, :as_committed, :kind, :limit, :include_projected
```

A full query-form grammar is incomplete in v1; detailed read-query DSL is in `read-protocol.md`. The write-surface grammar accepts the opcode and delegates validation of the query body to the read layer.

### 5.11 Annotation form: `:projected`

Memory-write forms accept `:projected true` to declare a future-dated validity (see `temporal-model.md` § 10). Absent the keyword, future `:v` is rejected with `BindError::FutureValidity`.

## 6. Reserved words

The following barewords are reserved and cannot be used as bare-predicate references or literal values without quoting or `@`-prefixing:

```
{ sem, epi, pro, inf,
  alias, rename, retire, correct, promote, query,
  true, false, nil }
```

Reserved keyword tags (the `:key` form):

```
{ :src, :c, :v, :at, :obs, :pre, :scp, :projected,
  :as_of, :as_committed, :kind, :limit, :include_projected,
  :reason }
```

Extensions to either list require a PR and a spec update.

## 7. Positional vs keyword-argument split

Required fields are **positional**; optional fields and metadata are **keyword-tagged**. Rationale:

- Positional for required, frequently-appearing fields matches Lisp idioms and keeps per-fact tokens cheap (per the bake-off Pareto frontier).
- Keyword for optional and timestamp/confidence metadata makes the grammar robust to field reordering and forward-compatible with future metadata additions (adding a new keyword is not a grammar-breaking change per `PRINCIPLES.md` § 10 semver).
- Keyword-arg order is insensitive within a form; the parser collects and validates.

A field is **never** both positional and keyword in the same form. The per-form productions in § 5 are authoritative.

## 8. Error taxonomy

All parse failures produce a typed `ParseError`:

```rust
pub enum ParseError {
    UnexpectedToken { found: Token, expected: &'static str, pos: Position },
    UnexpectedEof { expected: &'static str, pos: Position },
    UnknownOpcode { found: String, pos: Position },
    UnknownSymbolKind { found: String, pos: Position },
    BadKeyword { found: String, form: &'static str, pos: Position },
    MissingRequiredKeyword { missing: &'static str, form: &'static str, pos: Position },
    DuplicateKeyword { keyword: String, pos: Position },
    UnterminatedString { pos: Position },
    InvalidEscape { escape: char, pos: Position },
    InvalidTimestamp { text: String, pos: Position },
    InvalidNumber { text: String, pos: Position },
    ReservedWordInBareword { word: String, pos: Position },
    InvalidSymbolName { text: String, pos: Position },
    ArityMismatch { opcode: &'static str, expected: usize, found: usize, pos: Position },
}
```

Per `PRINCIPLES.md` § 2 (Error-handling philosophy), agent-facing parse errors are structured data, not strings. Agents parse errors by variant; they never regex-match error messages.

## 9. Streaming and batching

### 9.1 Default: one form per line

On the wire, the default assumption is **one top-level form per line**. Newline terminates a form. Agents batch multiple writes as multiple lines; the librarian processes them as a stream.

```
(sem @alain email "alice@example.com" :src @profile :c 0.95 :v 2024-01-15)
(sem @alain role @founder :src @profile :c 0.98 :v 2024-01-01)
```

### 9.2 Multi-line forms

For human-readable inspection (decoder-tool output, debugging), multi-line forms are accepted. A form ends at its closing paren, not at a newline; parse continues across lines until paren balance is 0.

```
(epi @ep_001 @rename
     (@luamemories @mimir)
     @github
     :at 2026-04-17T10:00:00Z
     :obs 2026-04-17T10:00:00Z
     :src @alain :c 1.0)
```

The lexer ignores newlines as whitespace; the parser uses paren balance for form delimitation.

### 9.3 Batching semantics

Multiple forms emitted together form a **batch**. Batch boundaries are the single `input: &str` passed to one `Store::commit_batch` call (`wire-architecture.md` § 3.1), not defined by this grammar. From the grammar's perspective, a stream of forms is indistinguishable from a batch.

Episode atomicity at the write protocol (`write-protocol.md`) groups a batch into a single Episode; all forms in the batch commit atomically or the whole batch rolls back.

## 10. Invariants

1. **Deterministic parse.** A well-formed input has exactly one parse. The parser is context-free; there are no heuristic disambiguation rules.
2. **Typed error on malformed input.** Every ill-formed input produces a typed `ParseError` variant. No `ParseError::Other` or stringly-typed fallback.
3. **Reserved-word isolation.** Reserved words (`sem`, `epi`, etc.) cannot appear as predicate barewords or string-literal values. Attempting so is `ParseError::ReservedWordInBareword`.
4. **Keyword-argument well-formedness.** Each form's keyword set is closed (per § 5). Unexpected keywords are `ParseError::BadKeyword`; duplicates are `ParseError::DuplicateKeyword`; missing required keywords are `ParseError::MissingRequiredKeyword`.
5. **ASCII identifiers.** Symbol and bareword identifiers match `[a-z_][a-z0-9_]*` (ASCII only). Non-ASCII characters are valid only inside quoted string literals.
6. **Balance.** Parentheses are balanced. Unbalanced input is `ParseError::UnexpectedEof` or `ParseError::UnexpectedToken`.
7. **No grammar ambiguity.** The grammar has a unique parse. Formal verification of unambiguity is part of graduation criterion § 1.

## 11. Open questions and non-goals for v1

### 11.1 Open questions

**Numeric literal precision.** v1 accepts `Integer` (signed 64-bit) and `Float` (IEEE 754 64-bit). Arbitrary-precision decimals are out of scope. Revisit if confidence arithmetic reveals precision loss.

**Escape sequences in strings.** v1 supports `\"`, `\\`, `\n`, `\r`, `\t`. Adding `\u{XXXX}` for Unicode escapes is straightforward; deferred until a concrete workload needs it.

**Quasi-quotation.** Lisp's `` ` `` and `,` quasi-quote forms for partial interpolation are not supported. Probably never — agents assemble writes in their host language, not via quasi-quote.

**Pretty-printed round-trip.** Multi-line forms (§ 9.2) are parseable on ingress but not preserved on canonical-form output. Decoder-tool emission reconstructs them based on configured indentation. An exact-fidelity round-trip through canonical form is a non-goal in v1.

**Predicate `@` optionality.** `symbol-identity-semantics.md` § 10 accepts both `@email` and bareword `email` in predicate slots. This grammar enforces both forms; whether one becomes canonical in emitted output is a librarian-emission choice, not a grammar choice.

### 11.2 Non-goals for v1

- **Macros / expansion.** No user-defined syntactic forms.
- **Hygienic expansion / renaming.** Not applicable; the grammar has no macro system.
- **In-grammar comments beyond line comments.** No block comments (`#| ... |#`) in v1.
- **Partial recovery on parse error.** A single malformed form aborts the batch. Recovery could be added post-MVP but raises determinism concerns.
- **Streaming byte-level input to the parser.** Parser expects complete forms. Under v1's in-process API (`wire-architecture.md`), every `commit_batch` call passes a complete `input: &str`; streaming front-ends aren't part of the v1 surface.

## 12. Primary-source attribution

All entries are verified per `docs/attribution.md`. The grammar design is not load-bearing on literature — it derives from the bake-off outcome plus principled Lisp-family conventions — but prior art is worth citing for the record.

- **R7RS Scheme small language report** (verified) — authoritative Scheme grammar. Mimir's grammar is a strict subset plus typed-symbol extension.
- **Clojure EDN (Extensible Data Notation)** (verified — tool convention) — `:keyword` syntax and order-insensitive keyword args match EDN convention.
- **Ford, *Parsing Expression Grammars: A Recognition-Based Syntactic Foundation*, POPL 2004** (verified) — PEG-family grammars have unambiguous parses by construction, matching § 10.7 invariant.
- **Aho, Sethi, Ullman, *Compilers: Principles, Techniques, and Tools*** (verified; already cited for Symbol Identity Semantics) — classical lexer/parser construction informing § 3 and § 5.
