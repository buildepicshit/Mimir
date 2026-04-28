# BC/DR Restore Runbook

Mimir's recovery primitive is the append-only workspace `canonical.log`.
Git-backed remote sync mirrors that log and draft lifecycle files into an
operator-owned recovery repository. Push/pull commands are explicit by
default. Projects may opt into post-capture push with
`remote.auto_push_after_capture = true`; pull is always explicit.

## Configure A Recovery Remote

Create or choose a private Git repository for Mimir recovery state, then
record it in the project config:

```bash
mimir config init \
  --operator hasnobeef \
  --organization buildepicshit \
  --remote-url git@github.com:org/mimir-memory.git
```

Inspect before moving bytes:

```bash
mimir remote status
mimir remote push --dry-run
```

Publish the local governed log and draft queue:

```bash
mimir remote push
```

The push path refuses to overwrite append-only history unless the remote
log is a prefix of the local log. Draft sync is copy-only: identical files
are skipped, missing files are copied, and same-name different-content
drafts are reported as conflicts. Push verifies the local source log and
the mirrored remote log before publishing; successful log sync reports
`workspace_log_verified=true`.

To back up automatically after wrapped-session capture and librarian
handoff, opt in explicitly:

```toml
[remote]
kind = "git"
url = "git@github.com:org/mimir-memory.git"
branch = "main"
auto_push_after_capture = true
```

This runs the same verified `mimir remote push` path used by the manual
command. It never pulls remote state, never changes the child agent exit
code, and records the result under `remote_backup` in
`capture-summary.json`.

## Fresh-Machine Restore

Start from a clean checkout of the project and a `.mimir/config.toml`
that points at the same recovery remote. If the config does not exist yet,
recreate it with the same `[remote]` values.

```bash
mimir remote status --refresh
mimir remote pull
mimir status
```

`mimir remote pull` restores the remote
`workspaces/<workspace-hex>/canonical.log` to the configured local
`storage.data_root` when the local log is missing or is a prefix of the
remote log. It also copies missing draft JSON files back into the local
draft lifecycle directories. Pull verifies the remote source log and the
restored or skipped local log before reporting success.

Verify the restored log with the read-only decoder:

```bash
mimir-cli verify .mimir/state/<workspace-hex>/canonical.log
```

Then run a wrapped launch or `mimir status` to confirm the workspace is
discoverable and the pending draft count matches expectation.

## Corrupted Local Log

If local state is corrupted but the recovery remote is intact, preserve
the bad local bytes first:

```bash
cp .mimir/state/<workspace-hex>/canonical.log /tmp/mimir-canonical-corrupt.log
mimir-cli verify /tmp/mimir-canonical-corrupt.log
```

`Store::open` only truncates recoverable crash tails: cleanly decoded
orphan records after the last CHECKPOINT, or a torn final frame
(`Truncated` / `LengthMismatch`). Unknown opcodes, invalid flags,
reserved sentinels, body underflow, and other structural decoder errors
fail open without truncating so the bad bytes remain available for
inspection or remote restore.

If `mimir remote status --refresh` reports `remote_ahead`, run:

```bash
mimir remote pull
```

If it reports `diverged`, do not overwrite either side. Preserve both
logs, decode both histories, and resolve through the librarian. Divergent
logs mean append-only histories disagree; Mimir deliberately refuses an
automatic winner.

## Restore Drill

The destructive drill proves that the configured remote can rebuild local
state:

```bash
./scripts/bcdr-drill.sh --dry-run
./scripts/bcdr-drill.sh --destructive
```

The script delegates to `mimir remote drill`. The command deletes the
local workspace `canonical.log`, runs `mimir remote pull`, verifies the
restored log, opens the restored store, and executes a read-path sanity
query:

```text
(query :limit 1)
```

A passing drill prints stable key/value lines including:

```text
direction=drill
status=passed
deleted_local_log=true
workspace_log_copied=true
verify_tail=clean
sanity_query_records=1
```

Use `--project-root <dir>` or `--config <file>` to target a non-current
project. The drill refuses to delete local state unless `--destructive`
is present.

## Current Limits

Git is the implemented recovery adapter. Service remotes expose a dry-run
contract only. Auto-push currently runs only after wrapped-session
capture. There is no timer or generic on-commit scheduler yet, so
operators who do not enable `remote.auto_push_after_capture` must run
`mimir remote push` after meaningful local commits.
