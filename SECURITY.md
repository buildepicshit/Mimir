# Security Policy

## Supported versions

Mimir is currently an implementation-stage pre-release with no tagged versions yet — see [`STATUS.md`](STATUS.md) and [`docs/launch-readiness.md`](docs/launch-readiness.md). Security reports are accepted for the current `main` branch.

| Version | Supported |
|---|---|
| `main` (unreleased) | yes |
| any tagged release | n/a — no releases yet |

## Reporting a vulnerability

If you discover a security issue in Mimir, **please do not open a public GitHub issue.** Instead, email the project owner:

**Alain Dormehl** — <alaindormehl@gmail.com>

Include:

- A description of the vulnerability.
- Reproduction steps or a proof-of-concept if available.
- The potential impact (data corruption, unauthorized write, injection, privilege escalation, denial-of-service, etc.).
- Any proposed remediation or mitigation.

You can expect an acknowledgement within 5 business days. The project will coordinate disclosure and remediation on a schedule appropriate to the severity of the issue.

### Triage SLA targets

Pre-1.0 the project is single-author and the SLA below is a target, not a contract. Once a tagged release exists and external users depend on the published artifact, these targets become commitments.

| Severity | Acknowledge | Fix in `main` | Coordinated disclosure |
|---|---|---|---|
| **P0** (active exploit; data loss) | 1 business day | 5 business days | Immediately on fix |
| **P1** (likely-exploitable defect) | 5 business days | 14 calendar days | After fix lands in a tagged release |
| **P2** (defence-in-depth gap) | 5 business days | next minor release | At reporter's discretion |

## Threat Model

Mimir does not run a daemon or network listener. The canonical store remains an in-process Rust boundary (`Store::commit_batch` / `Pipeline::compile_batch`), and every durable write still crosses the librarian-controlled validation path. The shipped product surface now includes local adapters around that boundary: `mimir-mcp` speaks MCP over stdio, `mimir-librarian` may invoke an operator-configured LLM CLI subprocess, and `mimir-harness` launches a native agent subprocess while staging draft artifacts. The explicit remote-recovery commands can run operator-requested Git syncs; those are user-commanded outbound operations, not an ambient service or phone-home channel.

This shapes the threat surface materially: an attacker reaches Mimir through (a) write-surface Lisp or prose drafts that cross the librarian pipeline, (b) bytes the embedding agent or operator feeds to `Store::open` and the canonical-log decoder, (c) stdio protocol frames sent to `mimir-mcp`, (d) local subprocess boundaries configured by the operator, or (e) the dependency supply chain. Reports premised on Mimir exposing a TCP listener remain out of scope because no such listener exists.

### In scope

We accept reports for, and treat as security vulnerabilities, the following classes of issue:

1. **Untrusted-Lisp injection into the librarian.** A crafted `(sem ...)` / `(epi ...)` / `(query ...)` / etc. write-surface input that escapes lex / parse / bind / semantic / emit and either commits a record the spec says is invalid, panics the host process, or executes anything outside the deterministic compiler pipeline. (Reachable through any embedding agent that calls `Store::commit_batch` with externally-influenced input.)
2. **Untrusted canonical bytes into the decoder.** A crafted `canonical.log` byte stream that, when fed to `mimir-cli verify` / `mimir-cli decode` / `mimir-cli log` / `mimir-cli symbols` or to `Store::open`'s recovery path (`decode_all` over the file), causes process abort (allocator OOM, stack overflow, panic) or commits malformed state to the in-memory pipeline. The mitigations for the three known classes (destructive-truncate of misrouted-path files, decoder OOM via attacker-controlled count, parser stack overflow on deep nesting) shipped in PRs #56–#57; future regressions or new bypasses are in scope.
3. **MCP stdio bridge bypass.** The `mimir-mcp` agent surface exposes a workspace lease state machine over stdio MCP. Any path that lets an MCP client commit, query, or close an episode against a workspace it doesn't hold the lease for is in scope. The current implementation enforces lease ownership on every write tool via constant-time token comparison + non-expiry check + workspace-path sanity guard (`crates/mimir-mcp/src/server.rs` `validate_lease`).
4. **Workspace-isolation bypass.** A crafted input or sequence of operations that lets a `Store` instance bound to workspace A read or write data belonging to workspace B. The scoped-memory boundary in `PRINCIPLES.md` is structural — `&mut Store` enforces single-writer semantics and `WorkspaceId` partitions the data root — but new code that breaches this boundary is in scope.
5. **Supply-chain compromise.** Known-vulnerable versions in the pinned dependency graph that `cargo deny check` does not flag, license-policy escapes, malicious typosquats slipping past `nuget.config`-equivalent registry pinning, or unpinned GitHub Actions in our CI workflows. (Crates.io / npm typosquats and renamed packages are explicitly in scope.)
6. **Determinism breakage.** Any code path that makes the canonical store byte-non-deterministic across architectures (the `decay` lookup-table contract per [`docs/concepts/confidence-decay.md`](docs/concepts/confidence-decay.md) § 13 invariant 2; the `Confidence` u16 round-trip contract; the `ClockTime` `u64::MAX` sentinel) is treated as a security issue because reproducibility is load-bearing for any audit of canonical-store contents.
7. **Local subprocess boundary regression.** The librarian and harness intentionally shell out to operator-selected tools (`claude`, `codex`, Git, or future adapters) only through explicit command paths. Any argument-injection, path-confusion, draft-file overwrite, or provenance-forging bug in those local adapter boundaries is in scope.
8. **Unsafe-code regression.** The workspace lints `unsafe_code = "forbid"`. Any PR that introduces `unsafe`, `from_raw`, `transmute`, FFI declarations, or `extern "C"` is in scope as a compliance issue (and the lint will block it; an active PR that strips the forbid is in scope as a policy regression).

### Out of scope

The following are explicit non-goals for v1 and are expected behaviour:

1. **Network attackers.** Mimir exposes zero network listeners by construction. The wire-architecture spec dropped daemon / socket / async-queue scope on 2026-04-19. Reports of "Mimir listens on port X" are not in scope because Mimir never opens a network socket. Operator-requested Git remote syncs and LLM CLI subprocesses may use the network through their own tools; vulnerabilities in those external services are not Mimir listener vulnerabilities.
2. **Other local processes running as the same UID.** A second process on the same machine, running as the same user, can already read the canonical-log file directly. The shared workspace lock rejects concurrent Mimir writers, but it is not a sandbox against arbitrary same-UID file access.
3. **Compromised local toolchain.** A compromised `cargo` SDK install, a compromised Rust toolchain, or a backdoored compiler will result in arbitrary code execution. Resolving the toolchain via `rustup` and pinning to `rust-toolchain.toml` is recommended; deeper supply-chain pinning is out of scope.
4. **Untrusted-workspace scenarios.** A developer who points `Store::open` at a workspace produced by an untrusted party gets exactly the bytes that workspace contains — the decoder's job is to refuse malformed bytes safely (in scope), not to sandbox a fully-formed adversarial workspace's content.
5. **Tooling not under `crates/`.** Issues in example code, local scratch tooling, CI helper scripts, or design-document artifacts. Issues in the production crates (`mimir-core`, `mimir-cli`, `mimir-mcp`, `mimir-librarian`, `mimir-harness`) are in scope.
6. **Social-engineering attacks** on the project owner or contributors.
7. **Physical-access attacks** on the developer's machine.

### What this means for vulnerability reporters

If your finding falls under "in scope": email the address above. We will treat it as confidential, acknowledge per the SLA targets, and ship a fix on a coordinated timeline.

If your finding falls under "out of scope" but you believe the threat model itself is wrong: please open a discussion thread instead, so we can debate the scope publicly. Some "out of scope" items may move to "in scope" once corresponding features land — e.g., a future networked transport for `mimir-mcp` (currently stdio-only) would put bridge-auth and lease-token entropy under stricter scrutiny than the current local-trust model requires.

## Data classification expectations

Mimir is opaque to the data its embedding agent commits. Concretely:

- **Canonical store contents** (`Sem` / `Epi` / `Pro` / `Inf` records, including `Value` payloads) inherit the embedder's classification of the agent input. Treat the canonical-log file at the same classification level as the messages the agent processes.
- **Symbol table** carries agent-supplied names (e.g., `@alice`, `@email`). Treat at the same classification level as the underlying records.
- **Tracing emissions** (the `mimir.*` span and event names per [`docs/observability.md`](docs/observability.md)) emit **identifiers only** — never `Value` payloads, never agent-supplied strings. Operators are responsible for the classification of the identifiers themselves; if `@alice` would be PII at a given site, the tracing output is also PII at that site.
- **Mimir emits no telemetry, no phone-home, and no crash reporter.** Operators retain full control of what crosses the host boundary. Commands such as `mimir remote push|pull` or an LLM-backed librarian run can invoke operator-configured tools that perform outbound network I/O; Mimir itself does not initiate ambient background network traffic.

## Loopback / Network Communication

Mimir has no loopback server, Unix-socket protocol, TCP listener, or async write queue. Per [`docs/concepts/wire-architecture.md`](docs/concepts/wire-architecture.md) § 2, the durable write boundary is a `Store::commit_batch` Rust function call returning `Result<EpisodeId, StoreError>`.

`mimir-mcp` speaks MCP over **stdio** to the embedding client. `mimir-harness` and `mimir-librarian` can spawn local subprocesses selected by the operator. Remote recovery is an explicit Git-backed command path. None of these surfaces create a background network service; if a future service transport lands, this threat model must be updated before release.

## Coordinated disclosure

The project owner will work with the reporter to agree on a disclosure timeline. Fixes are released before public disclosure; the reporter is credited in the release notes unless anonymity is requested. The default disclosure window is per the SLA table above; we will negotiate longer windows for issues that require coordinated multi-vendor remediation.
