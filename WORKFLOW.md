---
tracker:
  kind: linear
  endpoint: https://api.linear.app/graphql
  api_key: $LINEAR_API_KEY
  project_slug: mimir
  active_states:
    - Todo
    - In Progress
    - In Review
  terminal_states:
    - Done
    - Canceled
    - Duplicate
polling:
  interval_ms: 30000
workspace:
  root: /var/home/hasnobeef/buildepicshit/.symphony/workspaces/Mimir
hooks:
  after_create: |
    git clone git@github.com:buildepicshit/Mimir.git .
  before_run: null
  after_run: null
  before_remove: null
  timeout_ms: 60000
agent:
  max_concurrent_agents: 1
  max_turns: 20
  max_retry_backoff_ms: 300000
codex:
  command: codex app-server
  approval_policy: on-request
  thread_sandbox: workspace-write
  turn_timeout_ms: 3600000
  read_timeout_ms: 5000
  stall_timeout_ms: 300000
bes:
  repo: Mimir
  default_branch: main
  canonical_verify: cargo build --workspace && cargo test --workspace && cargo fmt --all -- --check && cargo clippy --all-targets --all-features -- -D warnings
---

# Mimir Workflow

You are working on Mimir under the BES spec-first model.

## Issue

- Identifier: `{{ issue.identifier }}`
- Title: `{{ issue.title }}`
- State: `{{ issue.state }}`
- Priority: `{{ issue.priority }}`
- URL: `{{ issue.url }}`
- Attempt: `{{ attempt }}`

## Required Procedure

1. Read `AGENTS.md`, `WORKFLOW.md`, `.agents/DOCUMENTATION_GUIDE.md`,
   `STATUS.md`, and the relevant concept docs.
2. For non-trivial work, create or update an executable `SPEC.md` from
   `.agents/specs/SPEC.template.md`.
3. Preserve librarian-mediated writes, append-only canonical storage, and
   provenance-preserving memory governance.
4. Verify locally before pushing to protect CI quota.
5. Report files changed, commands run, verification result, residual risk, and
   spec evidence candidates.

## Safety

- Do not write trusted shared memory directly.
- Do not bypass the librarian boundary.
- Do not push before running the local gate unless the owner explicitly says
  the CI budget tradeoff is acceptable.
- Do not treat quorum majority as truth.
