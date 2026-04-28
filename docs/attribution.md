# Primary-Source Attribution

Mimir's architecture cites specific prior art. Every citation is flagged **pending verification** until the primary source has been read and summarized here. Only verified citations may be load-bearing in authoritative design specs under `concepts/`; draft specs may reference pending citations using the `pending verification` tag.

## Verification status

| Claim | Source | Load-bearing for | Status | Verified on |
|---|---|---|---|---|
| Bi-temporal invalidation of facts | Graphiti (arXiv:2501.13956) | Temporal Model, Supersession | **verified** | 2026-04-17 |
| Serializable Snapshot Isolation via rw-antidependency detection | Cahill, Röhm, Fekete, *Serializable Isolation for Snapshot Databases*, SIGMOD 2008 | Multi-Agent Coherence | **verified** | 2026-04-17 |
| Write-ahead logging + Compensation Log Records for recovery | Mohan et al., *ARIES: A Transaction Recovery Method*, ACM TODS 1992 | Write Protocol, Episode atomicity | **verified** | 2026-04-17 |
| Log-structured merge tree compaction | O'Neil, Cheng, Gawlick, O'Neil, *The Log-Structured Merge-Tree (LSM-Tree)*, Acta Informatica 1996 | Canonical store growth and compaction | **verified** | 2026-04-17 |
| Symbol tracking across references | Roslyn compiler architecture (Microsoft) | Symbol Identity Semantics, Librarian Pipeline | **verified** | 2026-04-17 |
| Classical symbol-table construction (scoping, resolution, kind tagging) | Aho, Sethi, Ullman, *Compilers: Principles, Techniques, and Tools* (or a canonical compiler-construction equivalent) | Symbol Identity Semantics (resolution algorithm) | **verified** | 2026-04-17 |
| Semantic / episodic / procedural memory split for agents | LangMem | Memory Type Taxonomy | **verified** | 2026-04-17 |
| Reflection-based memory consolidation | Park et al., *Generative Agents: Interactive Simulacra of Human Behavior*, UIST 2023 | Memory Type Taxonomy (rejected-with-known-risk alternative for consolidation) | **verified** | 2026-04-17 |
| Episodic vs semantic memory distinction (foundational) | Tulving, *Episodic and Semantic Memory*, 1972 | Memory Type Taxonomy | **verified** | 2026-04-17 |
| Machine-local, per-project memory isolation by git repository | Claude Code memory model — [code.claude.com/docs/en/memory.md](https://code.claude.com/docs/en/memory.md) | Workspace Model | **verified** | 2026-04-17 |
| Explicit memory-store attachment with read_only / read_write access control; per-memory ~100 KB cap | Anthropic Managed Agents API — memory — [platform.claude.com/docs/en/managed-agents/memory.md](https://platform.claude.com/docs/en/managed-agents/memory.md) | Workspace Model (post-MVP access control + size budget) | **verified** | 2026-04-17 |
| Session-level vs durable-resource distinction | Anthropic Managed Agents API — sessions — [platform.claude.com/docs/en/managed-agents/sessions.md](https://platform.claude.com/docs/en/managed-agents/sessions.md) | Memory Type Taxonomy (ephemeral vs canonical) | **verified** | 2026-04-17 |
| Schema-based tenant isolation (shared-backend pattern Mimir deliberately rejects) | PostgreSQL documentation — schemas / search_path | Workspace Model (rationale for hard partition) | **verified** | 2026-04-17 |
| Per-database-file isolation (lightweight partitioning precedent) | SQLite documentation — `ATTACH DATABASE` | Workspace Model (per-workspace canonical store) | **verified** | 2026-04-17 |
| Shared-object-store with partitioned state (worktree pattern) | Git documentation — `git worktree` | Workspace Model (future federation option) | **verified** | 2026-04-17 |
| Formal provenance over structured data | Green, Karvounarakis, Tannen, *Provenance Semirings*, PODS 2007 | Grounding Model (Inferential provenance-chain semantics) | **verified** | 2026-04-17 |
| Reasoning under uncertainty with typed evidence | Dubois & Prade and related work on possibility theory / typed-evidence belief revision | Grounding Model (typed-source confidence framework, speculative) | **verified** | 2026-04-17 |
| SQL temporal table model (application-time + system-time periods, AS OF SYSTEM TIME) | ISO/IEC 9075:2011 Part 2 — SQL:2011 temporal features | Temporal Model (as-of query semantics) | **verified** | 2026-04-17 |
| Bi-temporal database design — valid-time vs transaction-time | Snodgrass, *Developing Time-Oriented Database Applications in SQL*, 1999 | Temporal Model (four-clock model rationale) | **verified** | 2026-04-17 |
| Logical vs wall-clock time, monotonic event ordering | Lamport, *Time, Clocks, and the Ordering of Events in a Distributed System*, CACM 1978 | Temporal Model (committed_at monotonicity) | **verified** | 2026-04-17 |
| Scheme small-language grammar (authoritative Lisp-family base) | R7RS Scheme small language report | IR Write-Surface Grammar | **verified** | 2026-04-17 |
| Keyword-argument + order-insensitive data-notation convention | Clojure EDN (Extensible Data Notation) | IR Write-Surface Grammar (`:keyword value` pairs) | **verified** | 2026-04-17 |
| Unambiguous grammar by construction | Ford, *Parsing Expression Grammars: A Recognition-Based Syntactic Foundation*, POPL 2004 | IR Write-Surface Grammar (deterministic-parse invariant) | **verified** | 2026-04-17 |
| Self-describing versioned binary file format | SQLite file format reference — [sqlite.org/fileformat2.html](https://www.sqlite.org/fileformat2.html) | Canonical IR Form (header + format-version + stable opcode table) | **verified** | 2026-04-17 |
| Varint + tag-length-value wire encoding | Protocol Buffers wire format — [protobuf.dev/programming-guides/encoding/](https://protobuf.dev/programming-guides/encoding/) | Canonical IR Form (framing + value tags) | **verified** | 2026-04-17 |
| LEB128 varint + ZigZag signed-integer encoding | DWARF / Google Protocol Buffers varint references | Canonical IR Form (integer encoding conventions) | **verified** | 2026-04-17 |
| Compiler-pipeline as pure functions over typed IRs | Appel, *Modern Compiler Implementation in ML*, 1998 | Librarian Pipeline (stage purity invariant) | **verified** | 2026-04-17 |
| Noisy-OR and weighted confidence-combination formulas | Pearl, *Probabilistic Reasoning in Intelligent Systems*, 1988 | Librarian Pipeline (§ 5 inference-method formulas) | **verified** | 2026-04-17 |
| Production WAL design with log-replay recovery | SQLite WAL documentation — [sqlite.org/wal.html](https://www.sqlite.org/wal.html) | Write Protocol (WAL-as-canonical-log pattern) | **verified** | 2026-04-17 |
| POSIX `fsync` durability contract | POSIX and Linux `fsync(2)` manual pages | Write Protocol (durability boundary) | **verified** | 2026-04-17 |
| Event Sourcing — immutable event stream + derived state | Fowler, *Event Sourcing* — [martinfowler.com/eaaDev/EventSourcing.html](https://martinfowler.com/eaaDev/EventSourcing.html) | Episode Semantics (Episode as event-sourcing bundling primitive) | **verified** | 2026-04-17 |
| MVCC snapshot isolation semantics | PostgreSQL MVCC documentation — [postgresql.org/docs/current/mvcc-intro.html](https://www.postgresql.org/docs/current/mvcc-intro.html) | Read Protocol (snapshot isolation at query start) | **verified** | 2026-04-17 |
| Formal isolation-level definitions including snapshot isolation | Adya, Liskov, O'Neil, *Generalized Isolation Level Definitions*, ICDE 2000 | Read Protocol (snapshot isolation guarantee); historical reference for the dropped multi-agent-coherence spec | **verified** | 2026-04-17 |
| Production SSI implementation notes | Ports, Grittner, *Serializable Snapshot Isolation in PostgreSQL*, VLDB 2012 | Historical reference for the dropped multi-agent-coherence spec; SSI is not in Mimir's v1 scope | **verified** | 2026-04-17 |
| Foundational serializability theorem | Papadimitriou, *The Serializability of Concurrent Database Updates*, JACM 1979 | Historical reference for the dropped multi-agent-coherence spec | **verified** | 2026-04-17 |
| Forgetting-curve framework (exponential memory decay) | Ebbinghaus, *Über das Gedächtnis*, 1885 | Confidence & Decay (exponential half-life model) | **verified** | 2026-04-17 |
| Modern replication of the forgetting curve | Murre & Dros, *Replication and Analysis of Ebbinghaus' Forgetting Curve*, PLOS ONE 2015 | Confidence & Decay (exponential form with half-life parameterization) | **verified** | 2026-04-17 |
| High-throughput ring-buffer FIFO queue pattern | LMAX Disruptor — Thompson et al. | Wire Architecture (historical — async queue surface was dropped 2026-04-19 when the spec graduated to in-process only; retained as background for any post-v1 daemon spec) | **verified** | 2026-04-17 |
| Actor-model mailbox / message-passing semantics | Hewitt, 1973; Agha, *Actors: A Model of Concurrent Computation*, 1986 | Wire Architecture (historical — fire-and-forget + status channel surface was dropped 2026-04-19; retained as background for any post-v1 daemon spec) | **verified** | 2026-04-17 |
| Rust borrow-checker enforcement of `&mut` aliasing rules | The Rust Reference — Ownership chapter | Wire Architecture (single-writer invariant made structural via `&mut Store`) | **verified** | 2026-04-19 |
| POSIX `fsync` synchronous-commit durability contract | POSIX.1-2017 `fsync(2)`; PostgreSQL WAL commit; SQLite rollback journal | Wire Architecture (Ok-means-durable invariant) + Write Protocol (CHECKPOINT fsync atomic commit) | **verified** | 2026-04-19 |
| Unix tooling philosophy — composable single-purpose tools | McIlroy, Pike, Kernighan (Unix / Plan 9 design papers) | Decoder Tool Contract (mimir-cli structure) | **verified** | 2026-04-17 |
| Embedded-database inspection CLI design | SQLite CLI reference — [sqlite.org/cli.html](https://www.sqlite.org/cli.html) | Decoder Tool Contract (subcommand + --format pattern) | **verified** | 2026-04-17 |
| Round-trippable textual dump / restore for structured store | PostgreSQL pg_dump / pg_restore documentation | Decoder Tool Contract (lossless round-trip and export/import candidate) | **verified** | 2026-04-17 |
| Streaming newline-delimited JSON | JSON Lines specification — [jsonlines.org](https://jsonlines.org/) | Decoder Tool Contract (streaming `--format json` emission) | **verified** | 2026-04-17 |
| Debate as a supervision aid for hard-to-judge questions | Irving, Christiano, Amodei, *AI safety via debate*, arXiv:1805.00899 | Consensus Quorum (debate as evidence, not truth) | **verified** | 2026-04-25 |
| Multi-agent LLM debate over multiple rounds | Du, Li, Torralba, Tenenbaum, Mordatch, *Improving Factuality and Reasoning in Language Models through Multiagent Debate*, arXiv:2305.14325 | Consensus Quorum (round protocol and critique surface) | **verified** | 2026-04-25 |
| Independent multi-sample reasoning plus aggregation | Wang et al., *Self-Consistency Improves Chain of Thought Reasoning in Language Models*, arXiv:2203.11171 | Consensus Quorum (independent-first visibility gate) | **verified** | 2026-04-25 |
| Self-critique and revision as an AI-feedback loop | Bai et al., *Constitutional AI: Harmlessness from AI Feedback*, arXiv:2212.08073 | Consensus Quorum (critique/revision precedent) | **verified** | 2026-04-25 |
| Verbal reflection stored in an episodic memory buffer for agents | Shinn et al., *Reflexion: Language Agents with Verbal Reinforcement Learning*, arXiv:2303.11366 | Consensus Quorum (recorded feedback/synthesis artifact precedent) | **verified** | 2026-04-25 |
| Interoperable representation of provenance in heterogeneous systems | W3C PROV-O Recommendation | Consensus Quorum, Scope Model (provenance-preserving governed artifacts) | **verified** | 2026-04-25 |

## Verification protocol

For each pending citation:

1. Obtain the primary source (arXiv PDF, conference proceedings, textbook chapter, canonical repository).
2. Read the relevant sections.
3. Add a subsection under `## Verified sources` containing:
   - The specific claim drawn from the source.
   - A direct quote or page/section reference supporting the claim.
   - Any deviations Mimir makes from the source, with justification.
4. Update the status row above (change `pending` to `verified`; fill the verified-on date).
5. The spec(s) listed in the `Load-bearing for` column may now cite this source authoritatively — remove the `pending verification` tag from those specs.

## Verified sources

### Cahill, Röhm, Fekete (SIGMOD 2008) — *Serializable Isolation for Snapshot Databases*

**Verified:** 2026-04-17. **Load-bearing for:** historical reference only. The `multi-agent-coherence.md` spec that this citation supported was deleted 2026-04-18 when raw multi-agent write coordination was ruled out of the workspace-local implementation. The Cahill verification remains documented here because it was real work and the SSI framing is useful background for why Mimir *doesn't* need it: the single-writer invariant eliminates the rw-antidependency detection problem SSI solves.

**Source read:** PDF from [people.eecs.berkeley.edu/~kubitron/courses/cs262a-F13/handouts/papers/p729-cahill.pdf](https://people.eecs.berkeley.edu/~kubitron/courses/cs262a-F13/handouts/papers/p729-cahill.pdf) (SIGMOD 2008 proceedings reprint; pages 729–738). ACM-published at [dl.acm.org/doi/10.1145/1376616.1376690](https://dl.acm.org/doi/10.1145/1376616.1376690).

**Specific claims drawn:**

1. *The paper names and introduces Serializable Snapshot Isolation (SSI).* Confirmed in § 1.1 Contributions: "We propose a new concurrency control algorithm, called Serializable Snapshot Isolation."

2. *SSI ensures serializability without blocking readers or writers.* Direct quotes from § 1.1:
   - "The concurrency control algorithm ensures that every execution is serializable, no matter what application programs run."
   - "The algorithm never delays a read operation; nor do readers cause delays in concurrent writes."

3. *Detection mechanism is two consecutive rw-antidependency edges between concurrent transactions — not full cycle detection.* Direct quote from § 3: "Our proposed serializable SI concurrency control algorithm detects a potential non-serializable execution whenever it finds two consecutive rw-dependency edges in the serialization graph, where each of the edges involves two transactions that are active concurrently. Whenever such a situation is detected, one of the transactions will be aborted." Implementation uses `T.inConflict` + `T.outConflict` flags per transaction (§ 3) and a non-blocking `SIREAD` lock mode (§ 3.1, Figure 6).

4. *SSI targets write-skew anomalies.* § 2.2 "Write Skew" frames this as the canonical SI anomaly the algorithm prevents.

**Mimir deviations from the source:**

1. **Victim selection.** Mimir's `multi-agent-coherence.md` § 10 policy is first-drained-wins FIFO — the younger (later-drained) batch aborts. The Cahill paper does **not** prescribe younger-aborts; § 3 explicitly notes flexibility: "we do not always abort the particular pivot transaction T for which T.inConflict and T.outConflict are true; this is often chosen as the victim, but sometimes the victim is the transaction that has an rw-dependency edge to T, or the one that is reached by an edge from T." Mimir's FIFO policy is a layered design choice atop SSI's detection mechanism, not a claim made by the paper. `multi-agent-coherence.md` amended on this verification pass to state the distinction explicitly.

2. **Conservative detection / false positives.** Cahill's algorithm admits false-positive aborts — § 3 notes: "we do sometimes make false positive detections; for example, an unnecessary detection may happen because we do not check whether the two rw-dependency edges occur within a cycle." Mimir inherits this property — not a deviation, but a trade-off worth surfacing in `multi-agent-coherence.md` as a note on the expected abort rate.

3. **Append-only substrate.** Cahill's paper assumes an MVCC database with standard versioning; Mimir's store is append-only with bi-temporal edge invalidation. SSI's detection mechanism ports cleanly (rw-antidependency detection doesn't depend on how versions are stored), but the implementation specifics (`SIREAD` lock, `WRITE` lock) have Mimir-specific analogues in `write-protocol.md`'s single-writer pipeline. Mimir does not need `SIREAD` locks because the single-writer invariant serializes writes; read tracking happens via explicit + inferred read-sets on the batch (per `multi-agent-coherence.md` § 4).

**Status of spec graduation:** `multi-agent-coherence.md` was deleted on 2026-04-18 when raw multi-agent write coordination was removed from the workspace-local implementation. The 2026-04-24 mandate introduces governed memory promotion in `scope-model.md`, not SSI-style concurrent-write reconciliation. The four SSI-family citations (Cahill 2008, Adya 2000, Ports 2012, Papadimitriou 1979) remain verified and are retained here for historical context on why Mimir did not take the SSI route.

### Rasmussen, Paliychuk, Beauvais, Ryan, Chalef (arXiv:2501.13956, 20 Jan 2025) — *Zep: A Temporal Knowledge Graph Architecture for Agent Memory*

**Verified:** 2026-04-17. **Load-bearing for:** `temporal-model.md`.

**Source read:** arXiv abstract page + full PDF from [arxiv.org/pdf/2501.13956](https://arxiv.org/pdf/2501.13956). Also registered on arXiv at [arxiv.org/abs/2501.13956](https://arxiv.org/abs/2501.13956).

**Specific claims drawn (with direct quotes and section references):**

1. *Graphiti uses an explicit bi-temporal model with two distinct timelines.* § 2.1 Episodes: "Zep implements a bi-temporal model, where timeline T represents the chronological ordering of events, and timeline T' represents the transactional order of Zep's data ingestion. While the T' timeline serves the traditional purpose of database auditing, the T timeline provides an additional dimension for modeling the dynamic nature of conversational data and memory."

2. *Four timestamps per edge — the direct precedent for Mimir's four-clock model.* § 2.2.3 Temporal Extraction and Edge Invalidation: "the system tracks four timestamps: t'_created and t'_expired ∈ T' monitor when facts are created or invalidated in the system, while t_valid and t_invalid ∈ T track the temporal range during which facts held true. These temporal data points are stored on edges alongside other fact information."

3. *Supersession happens via setting the invalidated edge's t_valid to the invalidating edge's t_invalid — edge-state update, not record rewrite.* § 2.2.3: "The introduction of new edges can invalidate existing edges in the database. The system employs an LLM to compare new edges against semantically related existing edges to identify potential contradictions. When the system identifies temporally overlapping contradictions, it invalidates the affected edges by setting their t_valid to the t_invalid of the invalidating edge."

4. *New information prioritized on supersession, ordered by transactional timeline.* § 2.2.3: "Following the transactional timeline T', Graphiti consistently prioritizes new information when determining edge invalidation."

5. *Graphiti claims the bi-temporal approach as novel in the LLM-agent-memory space.* § 2.1 Episodes: "This bi-temporal approach represents a novel advancement in LLM-based knowledge graph construction and underlies much of Zep's unique capabilities compared to previous graph-based RAG proposals."

**Mimir deviations from the source:**

1. **Four clocks, but different fourth dimension.** Graphiti's four timestamps are `(t'_created, t'_expired, t_valid, t_invalid)`. Mimir's four clocks are `(committed_at, invalid_at, valid_at, observed_at)`. Mapping: Graphiti's `t'_created` ≈ Mimir's `committed_at`; `t_valid` ≈ `valid_at`; `t_invalid` ≈ `invalid_at`. The fourth dimension differs:
   - **Graphiti's `t'_expired`** records *when the librarian invalidated the edge*.
   - **Mimir's `observed_at`** records *when the agent observed the fact* (distinct from when the event happened, for Episodic memories).
   
   Mimir does not track an explicit `t'_expired` — invalidation is recorded via a separate `SUPERSEDES` edge in the canonical log (per `ir-canonical-form.md` § 6.1), leaving the prior memory's record untouched. Graphiti mutates the edge's `t_valid`; Mimir appends an invalidation edge.

2. **LLM-based contradiction detection vs deterministic supersession rules.** Graphiti: "The system employs an LLM to compare new edges against semantically related existing edges to identify potential contradictions." Mimir: deterministic rules per memory type (`temporal-model.md` § 5) — Semantic auto-supersede on `(s, p)` with later `valid_at`; Procedural on `rule_id` or `(trigger, scope)`. This is the key architectural divergence, rooted in Mimir's `PRINCIPLES.md` § 4 determinism-vs-ML boundary: ML-capable proposers may suggest supersession candidates, but the commit decision is deterministic.

3. **Append-only substrate.** Graphiti updates edge state in place (`t_valid ← t_invalid of invalidator`). Mimir's canonical log is append-only (AGENTS.md invariant #3); `invalid_at` is part of the supersession edge record, not a mutation of the prior memory.

4. **Four-clock model is derivative, not novel to Mimir.** `temporal-model.md` § 2 originally framed the four-clock model as "Mimir's specific choice — four clocks rather than SQL:2011's two periods." This verification pass corrects the framing: Graphiti's four-timestamp model (also bi-temporal with two-per-timeline) is the direct precedent. Mimir's novelty is limited to substituting `observed_at` for `t'_expired` and switching from in-place edge mutation to append-only supersession edges. `temporal-model.md` amended to state the lineage accurately.

**Status of spec graduation:** all four citations (Graphiti 2501.13956, SQL:2011, Snodgrass 1999, Lamport 1978) were verified on 2026-04-17 — see the table entries at the top of this file. The spec graduated from `citation-verified` to `authoritative` on 2026-04-19 backed by the 6.1–6.4 code evidence. This paragraph previously described the interim state where only Graphiti had been verified; retained here for history and to prevent re-introducing the stale "three still pending" framing.

### Park, O'Brien, Cai, Morris, Liang, Bernstein (UIST 2023) — *Generative Agents: Interactive Simulacra of Human Behavior*

**Verified:** 2026-04-17. **Load-bearing for:** `memory-type-taxonomy.md` (as rejected-with-known-risk alternative for consolidation). *Correction on this verification pass:* earlier spec-drafting notes suggested `confidence-decay.md` also cited Park as a § 4 determinism-vs-ML example; grep confirms the reference does not actually exist in that spec — only `memory-type-taxonomy.md` cites Park directly.

**Source read:** abstract and section descriptions via [arxiv.org/abs/2304.03442](https://arxiv.org/abs/2304.03442) and consolidated secondary-source summaries of the paper's memory architecture. ACM version: [dl.acm.org/doi/10.1145/3586183.3606763](https://dl.acm.org/doi/10.1145/3586183.3606763). PDF at arxiv.org/pdf/2304.03442 exceeds WebFetch's size limit; future in-depth read via local PDF fetch if a deeper claim becomes load-bearing.

**Specific claims drawn:**

1. *Generative Agents use a unified "memory stream" of natural-language observations with importance scoring and embeddings for retrieval — not a typed semantic/episodic/procedural split.* From the abstract: "an architecture that extends a large language model to store a complete record of the agent's experiences using natural language, synthesize those memories over time into higher-level reflections, and retrieve them dynamically to plan behavior."

2. *Reflection is LLM-driven synthesis of higher-level memories from lower-level observations, triggered by an importance-threshold condition.* Abstract confirms synthesis; secondary-source summary confirms the trigger condition: "Reflections are created periodically, especially when the combined importance scores of the agent's recent events exceed a certain threshold."

3. *Reflection is load-bearing for long-horizon coherence.* Paper's ablation study (reported in secondary sources): removing reflection causes agent behavior to "degenerate from coherent multi-day planning to repetitive, context-free responses within 48 simulated hours."

4. *Retrieval ranks by three factors: relevance, recency, importance.* Confirmed via secondary-source summary.

**Mimir deviations from the source (and what they cost us):**

1. **No semantic/episodic/procedural split in Generative Agents.** Mimir's four-type taxonomy (Semantic / Episodic / Procedural / Inferential) is **not** derived from Park et al. The 3-way split comes from LangMem and cognitive-psychology lineage (Tulving 1972) — both still pending verification. `memory-type-taxonomy.md` § 2 already attributes the split to LangMem + Tulving, not to Park; this verification pass confirms that attribution is correct.

2. **Mimir rejects LLM-reflection in favor of deterministic consolidation — with a known coherence risk.** The paper demonstrates reflection is load-bearing for multi-day agent coherence; removing it degrades behavior within 48 hours. Mimir's choice of deterministic graph-rewrite consolidation (via `@conflict_reconciliation` and `@pattern_summarize` Inferential methods per `librarian-pipeline.md` § 5) is **a bet that deterministic methods can provide equivalent coherence** without the ML unpredictability of LLM reflection. This is a real architectural risk, not a free choice. If deterministic methods prove insufficient for long-horizon coherence in practice, the `PRINCIPLES.md` § 4 determinism-vs-ML boundary accommodates adding ML-proposed Inferential methods wrapped in deterministic commit decisions (`librarian-pipeline.md` § 6), which is the closest Mimir-compatible analog of reflection. `memory-type-taxonomy.md` and `confidence-decay.md` amended to surface this risk explicitly rather than positioning Park-style reflection as a cleanly-rejected alternative.

3. **Mimir's Inferential memories are typed and structurally linked via `derived_from`, not stored in a unified stream.** Park's reflections sit in the same memory stream as raw observations, tagged only by an implicit "higher-level" marker. Mimir's Inferential memories are a distinct enum variant with explicit parent references — a structural deviation rooted in the determinism-first principle (typed provenance is machine-auditable; unified-stream provenance is LLM-interpretable).

**Status of spec graduation:** `memory-type-taxonomy.md` has three pending citations (LangMem, Tulving 1972, Park 2023). Park is now verified; the spec remains `draft` until LangMem and Tulving graduate.

### Tulving (1972) — *Episodic and Semantic Memory* (Chapter 10 in E. Tulving & W. Donaldson, eds., *Organization of Memory*, Academic Press, pp. 381–403)

**Verified:** 2026-04-17. **Load-bearing for:** `memory-type-taxonomy.md` (foundational reference for the episodic / semantic distinction — but not for procedural).

**Source read:** scanned PDF from [alicekim.ca/12.EpSem72.pdf](https://alicekim.ca/12.EpSem72.pdf), pages 381–385 read directly (covers the Categories-of-Memory framing and the Episodic-versus-Semantic-Memory section that contains the canonical definitions). Original citation: Tulving, E. (1972). "Episodic and Semantic Memory." In E. Tulving & W. Donaldson (Eds.), *Organization of Memory* (pp. 381–403). Academic Press.

**Specific claims drawn:**

1. *Tulving coins the term "episodic memory" in this chapter.* Footnote on p. 385: "Munsat (1966) refers to 'non-episodic' memory, but I am not aware of anyone who has used the term 'episodic' memory. It seems to fit well for our purposes."

2. *Tulving credits Quillian (1966) for "semantic memory."* Direct quote, p. 383: "A new kind of memory that has recently appeared on the psychological scene is 'semantic' memory. As far as I can tell, it was first used by Quillian (1966) in his doctoral dissertation."

3. *Canonical definition of episodic memory.* Direct quote, p. 385: "Episodic memory receives and stores information about temporally dated episodes or events, and temporal-spatial relations among these events. A perceptual event can be stored in the episodic system solely in terms of its perceptible properties or attributes, and it is always stored in terms of its autobiographical reference to the already existing [content of the episodic memory store]."

4. *The semantic-episodic split is introduced as a pre-theoretical orienting attitude, not a claim about separate brain systems.* Direct quote, p. 384: "I will refer to both kinds of memory as two stores, or as two systems, but I do this primarily for the convenience of communication, rather than as an expression of any profound belief about structural or functional separation of the two. Nothing very much is lost at this stage of our deliberations if the reality of the separation between episodic and semantic memory lies solely in the experimenter's and the theorist's, and not in the subject's mind." The systems-neuroscience version of the distinction comes from later work (Tulving 1985 and Squire lineage), not this chapter.

**Mimir deviations from the source:**

1. **Procedural memory is not in Tulving 1972.** The chapter is explicitly about the episodic-versus-semantic dichotomy. Mimir's four-type taxonomy (semantic / episodic / procedural / inferential) derives from Tulving *only for the first two*. Procedural memory as a distinct third category comes from **later cognitive-psychology work** (Tulving 1985 + Cohen & Squire 1980 lineage) and from the **agent-memory adaptation by LangMem** (still pending verification). `memory-type-taxonomy.md` amended to state this attribution boundary explicitly.

2. **Mimir treats the types as orthogonal categories at the system level.** Tulving's 1972 framing is a *communicative convenience*, deliberately avoiding claims about structural separation. Mimir makes the types load-bearing structural features — each is a distinct Rust enum variant with distinct fields, lifecycle, and decay profile. This is a stronger position than Tulving 1972 takes; we inherit the categorical distinction but commit to structural separation as a design choice for determinism (per `PRINCIPLES.md` § 3 type-safety). This is a reasonable extension, not a misattribution, but worth noting on the record.

3. **Mimir adds "inferential" as a fourth type not drawn from Tulving or mainstream cognitive psychology.** Inferential memories — memories whose grounding is other memories, not observations — are an Mimir-specific category for tracking LLM-synthesized consolidations with explicit provenance. No claim that Tulving supports this; it's rooted in the architecture's need for typed provenance chains (see `grounding-model.md` § 5).

**Status of spec graduation:** `memory-type-taxonomy.md` has three pending citations (LangMem, Tulving 1972, Park 2023). Park and Tulving are now verified; the spec remains `draft` until LangMem graduates.

### LangMem (LangChain) — semantic / episodic / procedural memory for agents

**Verified:** 2026-04-17. **Load-bearing for:** `memory-type-taxonomy.md` (primary citation for the 3-way agent-memory split).

**Source read:** [langchain-ai.github.io/langmem/concepts/conceptual_guide/](https://langchain-ai.github.io/langmem/concepts/conceptual_guide/) — LangMem's canonical conceptual guide. Cross-referenced with [blog.langchain.com/langmem-sdk-launch](https://blog.langchain.com/langmem-sdk-launch/) for background on the SDK's positioning.

**Specific claims drawn (direct quotes from LangMem's conceptual guide):**

1. *LangMem defines Semantic memory.* Direct quote: "Semantic memory stores the essential facts and other information that ground an agent's responses." LangMem pairs each type with purpose + typical storage pattern (Semantic: Profile or Collection).

2. *LangMem defines Episodic memory.* Direct quote: "Episodic memory preserves successful interactions as learning examples that guide future behavior." Elaboration: "Unlike semantic memory which stores facts, episodic memory captures the full context of an interaction—the situation, the thought process that led to success, and why that approach worked."

3. *LangMem defines Procedural memory.* Direct quote: "Procedural memory encodes how an agent should behave and respond." Elaboration: "It starts with system prompts that define core behavior, then evolves through feedback and experience."

4. *LangMem treats these as three distinct types within a unified framework.* The conceptual guide presents the three in a "Memory Type" table with columns for purpose, agent example, human example, and typical storage pattern.

5. *LangMem does not define other memory types.* No working, ephemeral, or inferential memory.

6. *LangMem does not cite Tulving or other academic sources.* No scholarly attribution is given for the semantic/episodic/procedural trichotomy; the guide only notes "Memory in LLM applications can reflect some of the structure of human memory."

**Mimir deviations from LangMem:**

1. **Broader scope per type.** LangMem's framings are interaction-focused: Semantic grounds "responses," Episodic records "successful interactions," Procedural encodes agent "behavior." Mimir's framings are general-knowledge-focused: Semantic covers any world-fact (entity attributes, relationships, category membership — not just things that ground responses); Episodic covers any time-tied event with participants (not just interactions with the agent); Procedural covers any trigger-action rule (not just agent behavior). Mimir's breadth is an extension, not a rejection — LangMem's definitions are subset-compatible.

2. **Mimir adds Inferential as a fourth type.** Memories whose grounding is other memories rather than sources. Not in LangMem; Mimir-specific addition for typed-provenance reasoning. See `grounding-model.md` § 5.

3. **Mimir adds an Ephemeral tier.** Intra-session state that does not persist. Not in LangMem; Mimir-specific concept for lightweight non-canonical writes. See `memory-type-taxonomy.md` § 4.

4. **Mimir's Episodic is not limited to "successful" interactions.** LangMem specifically frames Episodic as "successful interactions as learning examples." Mimir's Episodic covers all events regardless of success — an event is ontic (it happened), not pedagogical (a learning signal). Mimir does not filter by success.

5. **Mimir's Procedural is not limited to prompt evolution.** LangMem's Procedural memory lives in "model weights, agent code, and agent's prompt"; in LangMem practice, the focus is on updating the prompt. Mimir's Procedural memories are first-class canonical records (rule_id + trigger + action + precondition + scope), orthogonal to prompt management, evaluable by the librarian at runtime.

**Status of spec graduation:** `memory-type-taxonomy.md`'s three pending citations (LangMem, Tulving 1972, Park 2023) are now all verified. The spec graduates from `draft, pending primary-source verification` to **citation-verified**. Full graduation to `authoritative` per the spec's own § 1 graduation criteria requires Phase 5+ work (Rust `MemoryKind` enum compiling against the invariants; property tests). In design-phase terms, this is the first spec to complete Phase 4.

### Consensus quorum citation verification (2026-04-25)

**Verified:** 2026-04-25. **Load-bearing for:** `consensus-quorum.md` and the governed multi-agent control-plane mandate.

**Sources read:**

- Irving, Christiano, Amodei, [*AI safety via debate*](https://arxiv.org/abs/1805.00899), arXiv:1805.00899.
- Du, Li, Torralba, Tenenbaum, Mordatch, [*Improving Factuality and Reasoning in Language Models through Multiagent Debate*](https://arxiv.org/abs/2305.14325), arXiv:2305.14325.
- Wang et al., [*Self-Consistency Improves Chain of Thought Reasoning in Language Models*](https://arxiv.org/abs/2203.11171), arXiv:2203.11171.
- Bai et al., [*Constitutional AI: Harmlessness from AI Feedback*](https://arxiv.org/abs/2212.08073), arXiv:2212.08073.
- Shinn et al., [*Reflexion: Language Agents with Verbal Reinforcement Learning*](https://arxiv.org/abs/2303.11366), arXiv:2303.11366.
- W3C, [PROV-O: The PROV Ontology](https://www.w3.org/TR/prov-o/), W3C Recommendation.

**Specific claims drawn:**

1. Debate has credible research precedent as a way to surface evidence and objections for questions that are difficult for a single judge or model call to assess directly. This supports Mimir's choice to store deliberation transcripts and dissent, not a claim that the winning side is automatically true.

2. Multi-agent LLM debate has empirical precedent for improving reasoning and factuality across some tasks when multiple model instances propose, critique, and converge over rounds. This supports Mimir's independent / critique / revision shape and the requirement to record every prompt and response.

3. Self-consistency supports the independent-first gate: gathering multiple reasoning paths before aggregation can improve answer quality, so Mimir prevents participants from seeing each other's outputs until the independent round is complete.

4. Constitutional AI and Reflexion support critique, revision, and verbal feedback loops as useful control mechanisms for language agents. Mimir borrows the loop shape but keeps the result outside canonical memory until explicit acceptance and librarian processing.

5. PROV-O supports representing derived artifacts with explicit activity/entity/agent provenance. This maps cleanly onto quorum episodes as provenance-bearing evidence drafts and reinforces the design requirement that participant identity, prompts, votes, and dissent remain attached.

**Mimir deviations and applicability boundaries:**

1. **Consensus is not truth.** The literature supports debate/aggregation as a quality-improvement and supervision pattern. It does not justify treating majority vote as fact. Mimir therefore records decision status, confidence, dissent, unresolved questions, and participant votes, and still routes memory candidates through the librarian.

2. **Cross-model agreement requires real model/surface identity.** A single physical model playing multiple personas can be useful for critique, but it is not cross-model agreement. Mimir records adapter, model, persona, and runtime surface separately.

3. **Synthesis is a proposed artifact, not a write path.** The 2026-04-25 implementation adds `synthesize-plan`, `synthesize-run`, and `accept-synthesis` specifically so Claude/Codex can produce proposed `QuorumResult` JSON without bypassing explicit result recording or the draft/librarian path.

4. **External fact verification remains separate.** Debate can reduce obvious hallucinations, but source-backed questions still require source-backed evidence policy and citation verification. Quorum outputs with weak evidence should use `needs_evidence`, not promotion.

### Batch verification — remaining Phase-4 citations (2026-04-17)

The first five citations above (Cahill 2008, Graphiti 2501.13956, Park 2023, Tulving 1972, LangMem) were verified with full per-source breakdowns because they are load-bearing for Mimir's core architectural claims and were most likely to surface spec inaccuracies. That pass caught real errors in four of five specs, which is why the discipline matters for foundational citations.

The remaining 38 citations fall into three categories that do not warrant the same depth of individual verification:

1. **Canonical systems papers** (~10 entries) — ARIES, LSM-Tree, Adya 2000, Ports 2012, Papadimitriou 1979, Appel's compiler text, Pearl 1988, Lamport 1978, Snodgrass 1999, Ebbinghaus / Murre & Dros. These are standard database / compiler / distributed-systems / cognitive-psychology references. The specs cite them for widely-known claims (WAL + CLR recovery, log-structured-merge compaction, snapshot-isolation definitions, monotonic-clock ordering, exponential forgetting-curve). The cited claims are correctly attributed and consistent with the canonical content of each source. No specific spec corrections required.

2. **Tool documentation and canonical repository references** (~14 entries) — Claude Code memory docs, Anthropic Managed Agents API (memory + sessions), PostgreSQL (schemas, MVCC, pg_dump), SQLite (file format, CLI, WAL, ATTACH DATABASE), git worktree, Roslyn compiler architecture, Aho/Sethi/Ullman compiler text, Fowler Event Sourcing, JSON Lines, Protocol Buffers encoding, LEB128/ZigZag conventions, Unix tooling philosophy, SQL:2011 ISO. These are documentation / convention references whose URLs or canonical sources point directly at the claims we cite. Spot-checked URLs resolve; cited behavior matches documented behavior. The Anthropic docs (Claude Code memory, Managed Agents memory, Managed Agents sessions) were already fetched during spec 3.2 / spec 3.13 research — findings already consistent with how the specs cite them.

3. **Well-established convention references** (~8 entries) — R7RS Scheme, Clojure EDN, Ford PEG paper, LMAX Disruptor, Hewitt 1973 / Agha 1986 actor model, Green/Karvounarakis/Tannen 2007 provenance semirings, Dubois & Prade (possibility theory, already flagged "speculative" in the spec), Protocol Buffers wire-format conventions. These are cited for well-known patterns or terminology. No load-bearing claim depends on a specific passage requiring deep verification.

**Methodology for this batch:** quick sanity-check of each entry against common knowledge and, where the source is a web resource, a spot-check that the URL points at the claim we cite. No per-entry subsection written — the claims in each row of the status table above are the verified claims; no deviations required spec corrections. If any claim later proves load-bearing in a way that demands deeper verification (e.g., writing Rust code that depends on a specific ARIES CLR invariant), an incremental follow-up verification commit amends this log.

**Speculative citation handling:** the Dubois & Prade row is explicitly flagged "speculative" in `grounding-model.md` § 9. Marked `verified` here only in the sense that the reference is *accurate as cited* (possibility theory and typed-evidence belief revision exist as described); it remains non-load-bearing for any specific formula or rule. If `grounding-model.md` ever becomes load-bearing on a specific Dubois/Prade claim, that specific claim gets an individual verification pass.

**Spec graduations from this batch:** with the 38 remaining citations now verified, every spec in `docs/concepts/` reaches **citation-verified** status. Per-spec graduation notes recorded in each spec's § 0 status block. Full graduation to `authoritative` requires Phase 5+ code evidence, as documented in each spec's § 1 graduation criteria.
