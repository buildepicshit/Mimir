<!--
Keep this PR scoped and atomic. See CONTRIBUTING.md and AGENTS.md for conventions.
No AI attribution anywhere in the PR body or commit messages.
-->

## Summary

<!-- 1-3 sentences. What this PR does and why. -->

## Track

<!--
Choose the closest current track:
  - core / librarian / canonical store
  - transparent harness / adapters
  - MCP / CLI
  - recovery benchmark
  - docs / public readiness
  - specs / research
  - release / CI / dependency maintenance
-->

## Verification

<!--
Evidence the change is correct. Test output, spec cross-references, reproduction commands.
Tests passing is not correctness. Attach fresh verification output for any "done" claim.
If GitHub Actions is disabled for quota, include the local gate output required by AGENTS.md.
-->

## Checklist

- [ ] Conventional Commits format on every commit
- [ ] No AI attribution in commits, PR body, or any file
- [ ] Primary sources cited, or marked `pending verification` per AGENTS.md
- [ ] Relevant `AGENTS.md` invariants reviewed for conflict
- [ ] Public-facing status claims stay bounded to implemented and verified behavior
- [ ] Local gate output included if CI is unavailable due quota
- [ ] `STATUS.md` updated if milestone or phase state changed
- [ ] `CHANGELOG.md` updated under `[Unreleased]` if user-visible
