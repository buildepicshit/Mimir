# Mimir Codex Plugin

This plugin is the Codex-side distribution bundle for Mimir. It is intentionally not a standalone checkpoint skill: the skill is only one internal component of the workflow. The public artifact must carry the setup checks, verification commands, and librarian boundary with it.

## What It Does

- Teaches Codex to run `mimir doctor --project-root .` before assuming memory is active.
- Points Codex at `mimir setup-agent doctor --agent codex --scope project --project-root .` when native setup needs inspection.
- Captures durable session facts with `mimir checkpoint`, which writes untrusted drafts for librarian validation.
- Uses `mimir context` and `mimir memory ...` as read-only governed context surfaces.
- Keeps canonical memory writes behind the librarian; the plugin never writes `canonical.log` directly.

## Install Shape

For local dogfood, add this plugin through a Codex marketplace entry that points to `./plugins/mimir`. A sample entry is included in [`marketplace-entry.example.json`](marketplace-entry.example.json).

The canonical repo-local marketplace location is `.agents/plugins/marketplace.json`. Until that catalog file is present, the plugin can still be inspected as a normal local plugin bundle under `plugins/mimir`.

## Verification

```bash
mimir doctor --project-root .
mimir setup-agent doctor --agent codex --scope project --project-root .
```

Both commands are read-only. A green `mimir doctor` report means the project memory surface is configured; it does not imply that every future memory draft is trusted.
