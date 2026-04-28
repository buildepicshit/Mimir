# Scenario 04 - consensus quorum artifact loss and governed handoff recovery

> **Status:** Illustrative recovery benchmark scenario, not a completed run.

## Situation

A design deliberation used the consensus quorum workflow. Local session context is lost before the next agent resumes. The fresh agent must recover the structured quorum boundary: who participated, what prompts were used, what votes were cast, what dissent remains, and which artifacts are drafts rather than canonical memory.

## Cold-start prompt

> "We lost the quorum session context. What did the agents decide, what dissent remains, and what can safely become memory?"

## Recovery pressure

The agent must not flatten the quorum into "the agents agreed." It should preserve participant identity, distinguish independent outputs from critique and synthesis, identify proposed drafts, and keep the librarian as the only trusted memory writer.

## Ground-truth focus

- Consensus is governed evidence, not truth.
- Votes, dissent, prompts, provenance, and participant identity must survive.
- Quorum artifacts can propose drafts, but cannot write canonical memory directly.
- One model playing several personas is not cross-model agreement unless recorded honestly.
- Required-draft gates prevent "passed" pilot summaries that produced no memory candidates.

## Staleness risks

An agent should not report majority as truth, should not hide dissent, should not claim cross-model agreement from one physical model, and should not treat synthesis output as accepted memory without the explicit draft/librarian path.
