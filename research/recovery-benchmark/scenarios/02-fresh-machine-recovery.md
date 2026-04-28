# Scenario 02 - fresh machine recovery from remote mirror

> **Status:** Illustrative recovery benchmark scenario, not a completed run.

## Situation

The operator moves to a new machine or loses the local Mimir state directory. The repository checkout is available, but local agent memory, session directories, and the prior canonical log are gone unless the configured remote mirror can restore them.

## Cold-start prompt

> "I moved to a new machine. Restore Mimir memory for this repo and tell me what is safe to do next."

## Recovery pressure

The fresh agent must avoid treating recovery as a blind clone or overwrite operation. It should discover `.mimir/config.toml`, inspect remote status, explain the local/remote relation, and use explicit `mimir remote pull` or `mimir remote drill` commands only under the documented boundaries.

## Ground-truth focus

- Transparent launch remains `mimir <agent> [agent args...]`.
- Remote pull is explicit; auto-push after capture is opt-in and one-way after the governed capture path.
- Divergent append-only logs are not overwritten.
- Service remotes are still dry-run adapter contracts.
- Restore drills are explicit and destructive only after opt-in.

## Staleness risks

An agent should not claim service remotes are implemented, should not suggest `git reset` or overwrite of canonical logs, and should not treat copied markdown memory as equivalent to the governed recovery mirror.
