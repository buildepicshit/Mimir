# mimir-cli

Read-only inspection CLI for [Mimir](https://github.com/buildepicshit/Mimir) canonical logs. Decoder tool per `decoder-tool-contract.md`.

> **Pre-1.0 status.** This crate is part of Mimir's active-development tree. CLI output and library APIs may change before v1. Public crates.io releases wait for the first alpha.

## Install

Until the first alpha release, build from the repository root:

```bash
cargo install --locked --path crates/mimir-cli
```

## Subcommands

```text
mimir-cli log     <path>   Stream summary of a canonical log.
mimir-cli decode  <path>   Emit re-parseable Lisp for memory records.
mimir-cli symbols <path>   Print the reconstructed symbol table.
mimir-cli verify  <path>   Integrity + corruption report.
```

All subcommands are read-only by construction: the binary never writes to the log, never appends, never truncates.

## Exit codes

- `0` — success / clean log.
- `1` — corruption signals (decode error, dangling symbols, corrupt tail).
- `2` — argument errors / file-not-found.

## Example

```bash
$ mimir-cli verify ~/.mimir/<workspace_hex>/canonical.log
Records decoded: 1247
Checkpoints: 38
Memory records: 412
Symbol events: 797
Trailing bytes: 0
Dangling symbols: 0
Tail status: Clean
```

## Companion library

The CLI's rendering + verification logic is exposed as the [`mimir_cli`](https://docs.rs/mimir-cli) library so you can embed it (e.g., in a custom inspector or a CI gate).

## License

[Apache-2.0](LICENSE).
