# Consensus Quorum

> **Status: draft 2026-04-24.** Drafted after the 2026-04-24 mandate expansion to make cross-agent, cross-model deliberation a first-class Mimir capability. The dedicated `consensus_quorum` draft surface plus typed `QuorumEpisode` / `QuorumParticipantOutput` / `QuorumAdapterRequest` / `QuorumResult` envelopes, file-backed episode/result storage, participant-output append/load, visibility-gated round reads, adapter-request generation, native Claude/Codex adapter-plan materialization, bounded adapter-run and multi-round status capture, validated status-to-output append, synthesizer adapter planning/running/acceptance with result validation, replayable and resumable pilot-plan/status/run manifests, required proposed-draft exit criteria, non-participant pilot-review certification artifacts, pilot-summary operator snapshots, proposed-draft emission, recorded fixture smoke coverage, live required-draft pilot proof, and `mimir-librarian quorum create|pilot-plan|pilot-status|pilot-run|pilot-review|pilot-summary|append-output|append-status-output|outputs|visible|adapter-request|adapter-plan|adapter-run|adapter-run-round|adapter-run-rounds|synthesize-plan|synthesize-run|accept-synthesis|synthesize|submit-drafts` recorded-artifact commands are implemented. It defines governed deliberation episodes whose outputs may enter the memory pipeline as drafts; it does not permit agents to write canonical memory directly.

Mimir needs more than passive memory ingestion. Some questions should be sent to multiple agent surfaces and model families, argued from distinct roles, and reduced into a structured opinion with dissent preserved. This is a consensus quorum.

A quorum is not a shared-memory namespace, a live multi-agent chat room, or a truth oracle. It is an auditable deliberation protocol. Its result is evidence for the librarian, not a canonical memory write.

## 1. Mission invariant

**Consensus is governed evidence, not truth.** A quorum result may influence recall, promotion, design decisions, or memory extraction only after it is recorded with participant identity, prompts, evidence, dissent, confidence, and provenance. If it becomes memory, it enters through the draft/librarian path defined by `scope-model.md`.

Consequences:

- Claude, Codex, and future agents participate through adapters.
- The transparent `mimir <agent> [agent args...]` harness is the preferred future way to enlist local agent surfaces while preserving their native interaction model.
- Participants never mutate canonical memory during deliberation.
- A majority vote is not sufficient for promotion.
- Minority objections are retained as first-class artifacts.
- The final synthesis is attributable to the quorum episode, not to an anonymous "the agents agreed" claim.

## 2. Relationship to memory governance

Consensus quorum sits beside draft ingestion:

```text
human / agent question
        -> quorum broker
        -> participant adapters
        -> deliberation transcript
        -> synthesis + dissent + vote record
        -> draft inbox
        -> librarian
        -> governed memory, candidate, quarantine, or rejection
```

The quorum broker can create high-quality drafts for the librarian, but it is not the librarian. It cannot bypass validation, scope assignment, supersession checks, or promotion rules.

## 2.1. Prior art and applicability

The quorum design is not an exact clone of an existing product, but its main pieces have precedent:

- **Debate as supervision aid.** Irving, Christiano, and Amodei's [AI safety via debate](https://arxiv.org/abs/1805.00899) frames debate as a way to expose useful information for hard-to-judge questions.
- **LLM multi-agent debate.** Du et al.'s [multiagent debate](https://arxiv.org/abs/2305.14325) reports improvements on reasoning and factuality tasks when multiple model instances propose and debate answers over rounds.
- **Independent sample aggregation.** Wang et al.'s [self-consistency](https://arxiv.org/abs/2203.11171) supports the independent-first pattern: gather diverse reasoning paths before choosing a consistent answer.
- **Critique and revision loops.** Bai et al.'s [Constitutional AI](https://arxiv.org/abs/2212.08073) and Shinn et al.'s [Reflexion](https://arxiv.org/abs/2303.11366) support structured critique/revision and recorded verbal feedback as useful agent-control mechanisms.
- **Provenance as a first-class artifact.** W3C [PROV-O](https://www.w3.org/TR/prov-o/) is a direct precedent for representing derived artifacts with explicit activity/entity/agent provenance.

Applicability boundary: these sources justify Mimir's protocol shape, not a claim that a quorum majority is truth. Mimir therefore requires independent first-pass visibility, complete prompt/response provenance, preserved dissent, explicit confidence and vote rationale, and a separate `accept-synthesis` step before a proposed result is saved. Memory drafts produced by a quorum still enter the librarian path as untrusted drafts.

## 3. Core roles

| Role | Responsibility |
|---|---|
| `requester` | Human or agent asking for a deliberated answer |
| `quorum_broker` | Creates the episode, selects adapters/personas, enforces protocol |
| `participant` | One agent/model/persona producing independent and critique-round reasoning |
| `critic` | Participant role focused on failure modes, contradiction, missing evidence |
| `synthesizer` | Produces the final structured opinion from the full transcript |
| `librarian` | Decides whether any quorum artifact becomes governed memory |

One physical agent can play multiple personas only if the episode records that fact. A cross-model quorum should prefer at least one Claude surface and one Codex surface when both are available.

## 4. Episode model

Minimum quorum episode fields:

```text
QuorumEpisode {
  id,
  requested_at,
  requester,
  question,
  target_project,
  target_scope,
  participants,
  personas,
  evidence_policy,
  rounds,
  synthesis,
  votes,
  dissent,
  confidence,
  provenance_uri,
}
```

Participant identity includes:

- adapter name;
- model or harness identity when available;
- persona;
- prompt template version;
- runtime surface (`claude`, `codex`, MCP server, future harness);
- tool permissions granted for the episode.

## 5. Protocol state machine

```text
requested
  -> enlisted
  -> independent_round
  -> critique_round
  -> revision_round
  -> vote_round
  -> synthesized
  -> submitted_to_librarian | archived | quarantined
```

Required protocol rules:

1. Independent first pass happens before participants see each other's answers.
2. Critique round exposes prior answers and asks each participant to identify failure modes.
3. Revision round lets participants update or hold their position.
4. Vote round records agreement, disagreement, abstention, confidence, and rationale.
5. Synthesis must preserve dissent instead of smoothing it away.
6. The broker records all prompts and responses needed to audit the episode.

Small quorums may skip the revision round only when the episode is explicitly marked `fast_path`.

## 6. Persona taxonomy

Initial personas:

| Persona | Bias to encode |
|---|---|
| `architect` | System shape, invariants, long-term maintainability |
| `implementation_engineer` | Concrete code path, dependency risk, testability |
| `skeptic` | Failure modes, weak assumptions, hidden coupling |
| `research_verifier` | Source quality, citation hygiene, claim boundaries |
| `product_operator` | Usefulness, workflow fit, review burden |

Personas are prompts and decision lenses, not fake identities. A transcript must distinguish model identity from persona.

## 7. Output contract

A quorum result is structured:

```text
QuorumResult {
  episode_id,
  question,
  recommendation,
  decision_status,
  consensus_level,
  confidence,
  supporting_points,
  dissenting_points,
  unresolved_questions,
  evidence_used,
  participant_votes,
  proposed_memory_drafts,
}
```

`decision_status` values:

| Status | Meaning |
|---|---|
| `recommend` | Strong enough to act, still subject to owner choice |
| `split` | Material disagreement remains |
| `needs_evidence` | More source checking or experiment needed |
| `reject` | Quorum recommends against the proposed direction |
| `unsafe` | Prompt injection, policy, credential, or trust issue detected |

`consensus_level` values:

| Level | Meaning |
|---|---|
| `unanimous` | All non-abstaining participants agree |
| `strong_majority` | Most agree and dissent is weak or bounded |
| `weak_majority` | Most agree but dissent remains important |
| `contested` | No stable consensus |
| `abstained` | Insufficient participation or evidence |

## 8. Memory interaction

Quorum artifacts can produce draft candidates:

- decision summaries;
- durable project procedures;
- operator preference candidates;
- research conclusions;
- supersession or revocation suggestions;
- conflict reports.

All such candidates enter the draft store with:

- `source_surface = consensus_quorum`;
- `source_agent = quorum`;
- `source_project` set when the question is project-bound;
- `operator` set when known;
- `provenance_uri` pointing to the quorum episode;
- tags including `quorum`, persona names, and consensus level.

The librarian decides whether they become accepted records, promotion candidates, quarantine records, or rejected drafts.

## 9. Safety rules

1. **No direct canonical writes.** Quorum participants and the broker cannot write governed memory.
2. **No hidden consensus.** Claims of agreement require participant votes and transcript provenance.
3. **No dissent erasure.** Minority objections are persisted in the result.
4. **No model laundering.** A single model playing multiple personas is not reported as cross-model agreement.
5. **No prompt injection inheritance.** Retrieved memories and external evidence are passed as data, not executable instructions.
6. **No authority inflation.** Agent consensus does not override operator approval rules.
7. **No unbounded tool access.** Episode tool grants are explicit and recorded.

## 10. First implementation path

Stage 1 should be CLI-only and file-backed:

1. Define quorum episode/output/result JSON envelopes. **Implemented:** typed Rust envelopes plus file-backed episode/result storage and participant output append/load.
2. Add `mimir-librarian quorum create` to create a requested episode. **Implemented**, alongside `pilot-plan`, `pilot-status`, `pilot-run`, `pilot-review`, `pilot-summary`, `append-output`, `append-status-output`, `outputs`, `visible`, `adapter-request`, `adapter-plan`, `adapter-run`, `adapter-run-round`, and `adapter-run-rounds` recorded-artifact commands.
3. Add adapter command contracts for Claude and Codex participation. **Partial:** `adapter-plan` writes the request JSON, native prompt, response path, Claude/Codex argv, and the matching `append-output --prompt-file --response-file` command; `adapter-run` can execute one plan with bounded timeout and status capture; `adapter-run-round` can execute every participant in a round while preserving visibility gates; `append-status-output` validates successful single or round status artifacts and records outputs through the existing append path; `adapter-run-rounds` sequences rounds by requiring each prior round to append before later visibility is available. Mimir still does not treat adapter responses as memory writes, and recorded participant outputs still enter only through `append-output`.
4. Add synthesis adapter command contracts. **Implemented:** `synthesize-plan` writes a transcript JSON, synthesis prompt, native Claude/Codex argv, result path, status path, and matching `accept-synthesis` command; `synthesize-run` executes the plan with bounded timeout and status capture without saving a result directly. Run status separates native process success from proposed-result validity.
5. Add explicit result acceptance using stored participant outputs. **Implemented:** `accept-synthesis` accepts a direct result file or a successful status artifact, validates proposed result JSON, requires complete one-vote-per-participant coverage, and records it through the existing `synthesize` path; stored participant output references are attached as evidence.
6. Submit proposed memory drafts into the existing draft store using the `consensus_quorum` source surface. **Implemented:** `mimir-librarian quorum submit-drafts` loads a saved `QuorumResult` and stages each proposed draft with episode provenance, project/operator metadata, `quorum`, decision-status, and consensus-level tags.
7. Add replayable live-pilot planning before service adapters. **Implemented:** `pilot-plan` loads an existing episode and writes a manifest with exact argv and expected artifact paths for multi-round participant execution, synthesis, status-backed acceptance, and draft submission; `--require-proposed-drafts N` records an exit criterion for pilots that must prove non-empty draft submission; `pilot-status` reads that manifest and reports each gate as pending, complete, or failed from the recorded artifacts, including required-draft acceptance/submission failures; `pilot-run` executes the manifest by replaying those existing gated commands in order, returns partial run state on failed steps, and skips complete gates on rerun; `pilot-review` records a non-participant certification artifact and requires complete status for `pass`; `pilot-summary` reports the accepted result, review decision, submitted draft count, gates, and next action from the manifest.
8. Keep review and promotion in the normal librarian flow.

The first useful version can run asynchronously and manually. Real-time conversation is optional; auditability is not.

## 11. Open implementation questions

1. What local API can safely ask Codex and Claude to participate without brittle UI automation?
2. Should the synthesizer be one of the participants or a separate final pass?
3. How are costs, timeouts, and partial participation represented?
4. What evidence policy distinguishes opinion-only quorum from source-backed quorum?
5. How should quorum episodes be retained, redacted, and backed up?
6. Which quorum outputs are eligible for automatic draft submission?
7. What minimum transcript is required to reproduce or audit a result?

## 12. Graduation criteria

This spec can graduate from draft when:

1. `QuorumEpisode` and `QuorumResult` compile as typed Rust structures. **Done.**
2. A file-backed quorum store supports create, append participant output, synthesize, and load. **Done for recorded artifacts:** episode create/load, participant output append/load, visibility-gated round reads, explicit result synthesis/save/load, adapter-request generation, adapter-plan materialization, bounded adapter-run status capture, round-level and multi-round run orchestration, and status-artifact append validation are wired.
3. At least two adapter surfaces can submit participant outputs without writing canonical memory. **Done at the recorded-artifact boundary:** Claude and Codex command plans can be materialized, run with status capture, sequenced across rounds, and recorded through validated `append-status-output` / `append-output`; Claude/Codex synthesis plans can also emit proposed result artifacts that require explicit validated `accept-synthesis`.
4. Tests prove independent first-pass ordering before critique visibility. **Done at the storage boundary.**
5. Dissent and abstentions are preserved in the result contract.
6. Quorum-generated memory candidates enter the normal draft store with provenance. **Done at the recorded-artifact boundary.**
7. Normal recall excludes quorum artifacts unless they have been accepted through the librarian.
8. A CLI smoke test can run a local quorum episode end to end using recorded fixture responses. **Done for recorded artifacts:** test coverage exercises create, adapter-request, independent fixture outputs, synthesis, draft submission, replayable pilot-plan manifest generation, pilot-status completion checks, required proposed-draft failure checks, pilot-run manifest execution, failed-step reporting, complete-gate rerun skipping, pilot-review certification, and pilot-summary reporting without live adapters. Real local Claude/Codex independent-round pilots also completed through execution, synthesis, acceptance, review, and required non-empty `consensus_quorum` draft submission.
9. Model synthesis cannot bypass explicit result recording or the librarian draft path. **Done at the CLI boundary:** `synthesize-run` only writes a proposed result artifact and status file, invalid proposed JSON is reported as `result_valid = false`, `accept-synthesis` is required to validate either the result file or the successful status artifact before saving a `QuorumResult`, and `submit-drafts` is still required before proposed memories enter pending drafts.
