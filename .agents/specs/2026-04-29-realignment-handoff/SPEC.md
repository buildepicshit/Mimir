---
id: mimir-realignment-handoff-2026-04-29
status: draft-handoff
owner: HasNoBeef
repo: Mimir
source_spec: root:.agents/specs/2026-04-29-fleet-realignment-and-handoff/SPEC.md
branch_policy: local-only-public-oss
risk: medium
requires_network: false
requires_secrets: []
acceptance_commands:
  - "cargo build --workspace"
  - "cargo test --workspace"
  - "cargo test --workspace --all-features"
  - "cargo fmt --all -- --check"
  - "cargo clippy --all-targets --all-features -- -D warnings"
  - "cargo deny check"
  - "cargo doc --workspace --no-deps"
---

# SPEC: Mimir Realignment Handoff

## 1. Handoff Purpose

Mimir is a public pre-1.0 product, while BES company agents have temporarily
moved to spec evidence instead of Mimir hook authority. This handoff keeps those
two facts separate so product work does not get accidentally deprecated and BES
agent policy does not drift back to memory authority.

## 2. Current Branch And Dirty State

Observed on 2026-04-29:

```text
## main...origin/main
 M .gitignore
 M AGENTS.md
?? .agents/
?? .claude/
?? CLAUDE.md
?? WORKFLOW.md
```

Recent head:

```text
9e81c0f feat(librarian): support active processing adapters
4d38614 Delete docs/launch-posting-plan.md (#16)
1650d18 ci: make release publishing idempotent
```

Local MCP posture: no repo-local `.mcp.json` is present. Mimir product docs may
still mention MCP and hook features as product surfaces, but BES root operating
policy currently uses zero default MCP servers.

## 3. Source Docs Read

- `AGENTS.md`
- `CLAUDE.md`
- `WORKFLOW.md`
- `STATUS.md`
- `.agents/specs/2026-04-29-repo-audit/SPEC.md`

## 4. Preserve

- Public pre-1.0 honesty and launch-readiness discipline.
- Mimir product features around governed memory, librarian-mediated writes,
  recovery mirroring, hooks, MCP, and benchmarks.
- The BES distinction: spec evidence is current company authority; raw memory
  and hooks are not active work authority.
- Root-installed agent surfaces and all existing local/untracked work.

## 5. Work Classification

| Item | State | Required next action |
| --- | --- | --- |
| Shared agent setup | preserve | Keep local/draft until owner approves public-facing PR posture. |
| Active processing adapters commit | verify | Confirm local cargo gates before any release/PR closeout. |
| Pre-1.0 launch cleanup | ready-for-dispatch | Use a public-OSS-aware spec and avoid noisy CI churn. |
| BES integration pause note | owner-decision | Decide whether this belongs in Mimir product docs or root-only docs. |
| Spec authority research design | ready-for-dispatch | Design only; do not re-enable hooks without approval. |
| OSS release rollout | owner-decision | Requires explicit owner approval for tag/publish/push. |

## 6. Verification Gate

Run before claiming Mimir product work complete:

```bash
cargo build --workspace
cargo test --workspace
cargo test --workspace --all-features
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo deny check
cargo doc --workspace --no-deps
```

This handoff did not run product gates because it changed only agent-control
handoff documentation.

## 7. Recommended Next Agent Engagement

Start from inside `Mimir` and ask the agent to:

```text
Orient with repo-orientation. Read AGENTS.md, CLAUDE.md, WORKFLOW.md, STATUS.md,
docs/launch-readiness.md, and this handoff. Draft a closeout SPEC for the next
Mimir public-OSS-safe step. Do not push, tag, publish, or add BES hook authority
without owner approval.
```

## 8. Owner Decisions Before Execution

- Should Mimir resume launch cleanup first, or should spec-authority research
  happen first?
- Should the BES integration pause be visible in Mimir public docs?
- What release/tag posture is acceptable after local gates pass?

## 9. Residual Risk

Mimir is externally visible. Even doc-only agent scaffolding can create public
noise, so keep all new work draft/local until a low-noise PR plan is approved.
