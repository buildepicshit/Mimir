# Claude/Codex Harness Integration Research - 2026-04-24

> **Document type:** implementation research note.
> **Status:** accepted for the current harness ergonomics slice.
> **Primary sources:** Claude Code and OpenAI Codex official documentation, checked on 2026-04-24. Codex skills/plugins distribution refreshed against current OpenAI docs on 2026-04-28.

## Decision

Optimize Mimir's transparent harness for Claude Code and Codex CLI directly instead of hiding them behind a generic integration layer.

The shared primitive is still simple: `mimir checkpoint` writes an intentional markdown note into `MIMIR_SESSION_DRAFTS_DIR`. The agent-specific part is how the wrapped agent learns that primitive at startup:

- Claude Code receives the session guide through `--append-system-prompt-file`.
- Codex CLI receives a concise Mimir instruction through `-c developer_instructions=...`.
- Both receive `MIMIR_AGENT_GUIDE_PATH`, `MIMIR_CHECKPOINT_COMMAND`, and `MIMIR_SESSION_DRAFTS_DIR`.
- Unknown agents receive only the generic environment/session files; no CLI flags are injected.

## Claude Code Fit

Claude Code has a first-class system-prompt append path. The CLI reference documents `--append-system-prompt` and `--append-system-prompt-file`, and explicitly recommends append flags for preserving Claude Code's built-in behavior while adding requirements: <https://code.claude.com/docs/en/cli-reference>.

Claude Code also has rich hooks. `SessionStart` runs on startup/resume and can add context; it can persist environment variables into later Bash commands through `CLAUDE_ENV_FILE`: <https://code.claude.com/docs/en/hooks>. Hook settings live in `~/.claude/settings.json`, `.claude/settings.json`, `.claude/settings.local.json`, managed policy, plugins, or skill/agent frontmatter. Mimir's installer targets only explicit project/user settings and only the `mimir hook-context` SessionStart entry.

Claude skills live at `~/.claude/skills/<skill-name>/SKILL.md` for personal scope and `.claude/skills/<skill-name>/SKILL.md` for project scope: <https://code.claude.com/docs/en/skills>. Mimir installs `mimir-checkpoint` only when the operator runs `mimir setup-agent install`.

## Codex CLI Fit

Codex CLI supports per-invocation config overrides with `-c key=value`: <https://developers.openai.com/codex/cli/reference>. The config reference includes `developer_instructions`, which is the right launch-time surface for a concise Mimir instruction: <https://developers.openai.com/codex/config-reference>.

Codex also reads `AGENTS.md` at startup, layering global and project guidance: <https://developers.openai.com/codex/guides/agents-md>. That remains the right repository-level instruction surface, but Mimir should not rewrite a user's project files during a transparent launch.

Codex skills are available in the CLI and use progressive disclosure: <https://developers.openai.com/codex/skills>. Codex reads repository skills from `.agents/skills` while walking from CWD to repo root, user skills from `$HOME/.agents/skills`, admin skills from `/etc/codex/skills`, and bundled system skills. Mimir targets only project/user `mimir-checkpoint` skill installs.

The current Codex docs distinguish **skills** from **plugins**: skills are the authoring format for reusable workflows, while plugins are the installable distribution unit for reusable skills and app integrations. Skills are available in Codex CLI, IDE extension, and the Codex app. Local/repo skills remain the right first-run setup target for Mimir, but Mimir should not publish a standalone `mimir-checkpoint` skill as a public artifact. A single skill without the setup checks, librarian boundary, and verification flow is too easy to misunderstand. Public/reusable Codex distribution should be a coherent Mimir plugin bundle once the OSS repo is public: <https://developers.openai.com/codex/skills>.

Codex has a plugin directory in both the app and CLI. The CLI exposes it through `/plugins`, groups plugins by marketplace, and lets users install, inspect, enable, or disable plugins from those marketplace sources: <https://developers.openai.com/codex/plugins>. A custom marketplace is a JSON catalog that can live at `$REPO_ROOT/.agents/plugins/marketplace.json` or `~/.agents/plugins/marketplace.json`; `codex plugin marketplace add` can add GitHub, Git URL, or local marketplace sources. A Mimir Codex plugin should present the complete Codex-side Mimir workflow: setup doctor, explicit install/remove commands, checkpoint draft submission, and boundary language. The checkpoint skill may be one internal component of that plugin, not the published product by itself: <https://developers.openai.com/codex/plugins/build>.

Codex hooks can add extra developer context on `SessionStart`, and repo-local hooks should resolve from the git root: <https://developers.openai.com/codex/hooks>. Codex discovers hooks in `hooks.json` or inline `[hooks]` next to active config layers, including `~/.codex/hooks.json`, `~/.codex/config.toml`, `<repo>/.codex/hooks.json`, and `<repo>/.codex/config.toml`. Hooks require `[features] codex_hooks = true`, so Mimir's installer writes a `hooks.json` entry and enables that feature in the matching config layer.

## Consequences

This keeps the wrapper transparent while still making checkpoint capture visible inside the active model context.

The harness may prepend adapter args for known agents, but user-supplied child arguments remain unchanged and in the same relative order. This preserves native resume flows like `mimir claude --r` and `mimir codex resume`.

The first persistent-integration upgrade generates setup artifacts and installs them only when explicitly requested:

- a Claude project skill or command for checkpointing;
- a Codex project/user skill for checkpointing as local setup, followed only by a coherent Mimir Codex plugin/marketplace package for public distribution;
- optional hooks for automatic context injection where the operator wants them.

Those are explicit setup actions through `mimir setup-agent status|doctor|install|remove`, not hidden side effects of every launch.

## Pre-agent harness patterns

Checked adjacent command/session harnesses on 2026-04-24:

- **Nix `develop`** starts a shell with a prepared build environment and can run a target command with `--command`, preserving the idea that environment preparation is distinct from the command being run: <https://nix.dev/manual/nix/2.26/command-ref/new-cli/nix3-develop>.
- **Devbox** recursively discovers project config for `devbox shell`, and `devbox run` starts a shell then runs the target script/command, with quiet mode available for low-noise operation: <https://www.jetify.com/docs/devbox/cli-reference/devbox-shell> and <https://www.jetify.com/docs/devbox/cli-reference/devbox-run>.
- **direnv** hooks into the shell but loads `.envrc` in a subprocess and exports the environment diff back to the original shell. That is the right model for Mimir: explicit, inspectable deltas instead of mutating the child app's terminal stream: <https://direnv.net/>.
- **mise** requires `mise trust` before parsing config features it classifies as potentially dangerous, including environment variables, templates, and `path:` plugin versions. Its FAQ notes untrusted configs are skipped in non-interactive shells. Mimir should treat persistent skills/hooks the same way: generated setup plus explicit trust/install, not silent mutation: <https://mise.jdx.dev/cli/trust.html> and <https://mise.jdx.dev/faq.html>.
- **asdf** uses PATH shims that `exec` the selected tool version after resolving `.tool-versions` or env overrides. The useful lesson is that a wrapper can be second-nature when it performs deterministic resolution and then gets out of the way: <https://asdf-vm.com/manage/versions.html>.
- **Dev Containers** lifecycle scripts separate one-time creation, every-start, and every-attach phases. The spec also records merge semantics for lifecycle commands and environment variables. Mimir should keep bootstrap/setup, launch context, hook context, and checkpoint capture as separate lifecycle phases: <https://github.com/devcontainers/spec/blob/main/docs/specs/devcontainer-reference.md>.
- **pre-commit** installs hooks only after `pre-commit install`, runs them automatically afterward, bootstraps hook environments on first run, and reports concise per-hook statuses. Mimir should copy that trust and status model for optional hooks: <https://pre-commit.com/>.

Applied Mimir rule from this scan: the harness can print a small preflight banner and generate native setup artifacts, but persistent hook/skill installation remains an explicit setup act performed by the launched agent with operator approval. Hook output must add context only; canonical memory writes still require checkpoint drafts and the librarian.
