# Publication Readiness And Channels - 2026-04-28

This note records the minimum public-readiness bar and the best-fit publication channels for Mimir's first OSS opening. It is intentionally conservative: Mimir can go public as an experimental local-first memory governance project, but not as a production service, stable API, or benchmark-proven product.

## Source Baseline

- GitHub community profile docs: <https://docs.github.com/en/communities/setting-up-your-project-for-healthy-contributions/about-community-profiles-for-public-repositories>
- GitHub security policy docs: <https://docs.github.com/en/code-security/how-tos/report-and-fix-vulnerabilities/configure-vulnerability-reporting/adding-a-security-policy-to-your-repository>
- GitHub issue/PR template docs: <https://docs.github.com/en/communities/using-templates-to-encourage-useful-issues-and-pull-requests/about-issue-and-pull-request-templates>
- Cargo publishing docs: <https://doc.rust-lang.org/cargo/reference/publishing.html>
- Cargo manifest metadata docs: <https://doc.rust-lang.org/cargo/reference/manifest.html>
- docs.rs build docs: <https://docs.rs/about/builds>
- OpenAI Codex skills docs: <https://developers.openai.com/codex/skills>
- OpenAI Codex plugins docs: <https://developers.openai.com/codex/plugins>
- OpenAI Codex plugin authoring docs: <https://developers.openai.com/codex/plugins/build>
- Official MCP Registry docs: <https://modelcontextprotocol.io/registry/about>, <https://modelcontextprotocol.io/registry/package-types>
- Anthropic Connectors Directory docs: <https://claude.com/docs/connectors/directory>
- Anthropic remote MCP submission guide: <https://support.claude.com/en/articles/12922490>
- Anthropic local MCP submission guide: <https://support.claude.com/en/articles/12922832-local-mcp-server-submission-guide>
- Smithery publishing docs: <https://smithery.ai/docs/build>
- Glama MCP directory surface: <https://glama.ai/mcp/servers>
- MCP.Directory submit page: <https://mcp.directory/submit>
- OpenSSF Best Practices Badge: <https://openssf.org/best-practices-badge/>
- OpenSSF Scorecard: <https://openssf.org/projects/scorecard/>

## Minimum Bar Before Public Visibility

GitHub's community profile checklist expects the core community files to exist in supported locations: README, license, code of conduct, contribution guidelines, security policy, issue templates, and related repository health files. Mimir already has most of that. Before flipping visibility, do one final pass for:

- Repository description and topics that match the public stance: `agent-memory`, `mcp`, `rust`, `local-first`, `ai-agents`, `memory-governance`.
- README first screen says pre-1.0, experimental, local-first, and no benchmark/stable API claims.
- `SECURITY.md` exists and is linked from README; security reports should not go through public issues.
- Issue templates and PR template are present and useful for first contributors.
- `CONTRIBUTING.md`, `CODE_OF_CONDUCT.md`, `LICENSE`, `CHANGELOG.md`, and `STATUS.md` are current.
- `mimir doctor --project-root .` gives a clear first diagnostic path.
- Full local gate passes before the final public-flip push.

The OpenSSF Best Practices Badge and Scorecard are not launch blockers, but they are good post-open hardening targets. Scorecard/CodeQL can be revisited after public visibility unlocks GitHub code-scanning ergonomics.

## Rust Distribution Bar

For crates.io, Cargo's publishing guide calls out permanent uploads and recommends filling `license` or `license-file`, `description`, `homepage`, `repository`, and `readme`, with `keywords` and `categories` for discoverability. The dry-run/package bar before publishing any crate is:

- `cargo package -p <crate> --list` reviewed for accidental large/private files.
- `cargo publish -p <crate> --dry-run` passes.
- Workspace README and crate README match the same pre-1.0 compatibility story.
- `cargo doc --workspace --no-deps` passes.
- Package metadata has license, description, repository, readme, and sensible keywords/categories.

docs.rs automatically builds documentation for crates released on crates.io. Its build sandbox has no network access and resource limits, so crate docs must not rely on network side effects or large generated assets.

## Codex Distribution Bar

Codex is an official Mimir surface, so Codex distribution belongs in the launch plan rather than a later vague marketplace bucket. Current Codex docs make the split clear:

- Skills are the authoring format for reusable workflows. They are available in Codex CLI, IDE extension, and app, and can live at repo, user, admin, or bundled system scope.
- Plugins are the installable distribution unit for reusable skills, app integrations, and MCP server configuration.
- The Codex app has a plugin directory, and the CLI exposes the same concept through `/plugins`.
- Plugin marketplaces are JSON catalogs. A repo can provide `$REPO_ROOT/.agents/plugins/marketplace.json`; users can also add marketplace sources with `codex plugin marketplace add`.

Mimir should not publish individual skills to a skills library as if `mimir-checkpoint` alone were the product. A bare skill has too little context: it teaches Codex one command without the setup doctor, librarian boundary, repository policy, or post-session processing model. The project/user `mimir-checkpoint` skill remains a local setup artifact installed by `mimir setup-agent install`.

The first public Codex target, if we ship one, should be a coherent Mimir plugin bundle. The plugin can include the checkpoint skill as an internal component, but the public artifact should present the full Codex-side workflow: run `mimir doctor`, inspect/install native setup, submit checkpoint drafts, and keep canonical writes behind the librarian. That is materially different from publishing one isolated skill.

Minimum bar before adding the Codex plugin marketplace entry:

- Plugin manifest with stable kebab-case name, version, description, and bundled Mimir setup/checkpoint skill path. **Initial bundle exists:** [`plugins/mimir`](../../plugins/mimir).
- Repo-local marketplace entry for dogfood testing. **Blocked in the current wrapped session:** `.agents` is mounted read-only; [`plugins/mimir/marketplace-entry.example.json`](../../plugins/mimir/marketplace-entry.example.json) records the intended entry until `.agents/plugins/marketplace.json` can be added.
- README section showing Codex app and CLI install path.
- `mimir doctor` and `mimir setup-agent doctor --agent codex` shown as the verification commands.
- Explicit language that Mimir's Codex plugin submits drafts through the librarian boundary and is not a direct memory-write surface.

## MCP Distribution Bar

The official MCP Registry is the best canonical MCP discovery target, but it is still preview. It stores metadata, not artifacts. Server metadata goes in `server.json`, and the server must have a public installation method or public remote endpoint. Current official package types are npm, PyPI, NuGet, OCI/Docker, and MCPB. There is no first-class Cargo package type in the current package-type list, so Mimir's Rust MCP server needs one of these before registry submission:

- OCI image for `mimir-mcp`.
- MCPB local server bundle.
- Remote Streamable HTTP service endpoint.

For Mimir, the right first MCP publication target is:

1. Public GitHub repo plus `mimir-mcp` docs.
2. Crates.io/docs.rs for Rust users.
3. Official MCP Registry after an OCI image or MCPB package exists.
4. Anthropic Connectors Directory only after the package/deployment meets their review bar.

Anthropic's remote MCP directory expects a fully tested production remote server, OAuth 2.0 if authentication is required, proper tool safety annotations, a dedicated support channel, test account/sample data, and comprehensive docs. Their local MCP path expects a working MCPB with portable code, variable substitution, good errors, and clean bundled dependencies. Mimir is not ready for Anthropic directory submission until the local MCPB or remote service path is deliberately packaged.

Smithery is a good fit only once Mimir exposes a public Streamable HTTP endpoint or chooses a managed gateway path. Glama and MCP.Directory are useful secondary discoverability surfaces once GitHub and the official MCP metadata path are clean; they should not be treated as the canonical source of truth.

## Recommended Channel Order

1. **GitHub public repo** — best immediate fit. It lets reviewers inspect the architecture, docs, and tests while keeping all claims bounded.
2. **Launch article / writeup** — publish alongside or shortly after the public flip. This should explain the problem, the librarian boundary, the transparent harness, and the live dogfood evidence without benchmark overclaiming.
3. **Codex plugin marketplace package** — best fit for one official Mimir surface after the repo is public, but only as a coherent Mimir workflow bundle. Do not publish standalone Mimir skills.
4. **Crates.io + docs.rs alpha** — publish after package dry-runs and crate README audit. This is the best Rust-native install path.
5. **Official MCP Registry** — publish after a public MCP installation artifact exists via OCI or MCPB.
6. **MCP directories/marketplaces** — Glama, MCP.Directory, Smithery, and Anthropic Connectors Directory are follow-ons once packaging and support expectations are met.

## Article Recommendation

Yes, write a short article. The strongest angle is not "Mimir beats other memory systems"; it is:

> "Agent memory should be governed like a compiler pipeline, not appended like notes."

Suggested structure:

1. The practical pain: agents lose context, contaminate memory across repos, and restart badly.
2. The Mimir thesis: local-first, append-only, librarian-mediated, agent-native canonical memory.
3. Why the launch harness matters: `mimir <agent> ...` preserves native workflows and captures post-session drafts.
4. What is real today: governed log, draft lifecycle, context/doctor/status commands, MCP surface, BC/DR mirror, Floom/NewIdeas dogfood recovery.
5. What is not claimed yet: production readiness, hosted service, stable wire format, benchmark victory.
6. What feedback is wanted: Rust correctness, security review, adapter UX, benchmark methodology.

Best places:

- `docs/public-testing/` or `docs/blog/` in-repo as the canonical version.
- GitHub README link to the writeup.
- Hacker News "Show HN" after the repo is public and the writeup is in place.
- Dev.to / personal blog / BES Studios site as mirrors if the owner wants broader reach.

## Non-Goals For First Public Flip

- Hosted service.
- Daemonized librarian.
- Remote MCP connector directory submission.
- OS-level capture.
- Claims that recovery benchmarks are proven.
- Any direct agent write path into canonical memory.
