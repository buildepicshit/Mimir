# Mimir Fuzz Harness

`cargo-fuzz` targets for the decode boundaries of `mimir-core`. Tracked by [#33](https://github.com/buildepicshit/Mimir/issues/33).

## Targets

| Target | Input | Contract |
|---|---|---|
| `fuzz_lex` | arbitrary UTF-8 | `tokenize` returns a `Result` — never panics, never loops |
| `fuzz_parse` | arbitrary UTF-8 | `parse` returns a `Result` — every lex error propagates cleanly; no malformed AST on error paths |
| `fuzz_decoder` | arbitrary bytes | `decode_record` and `decode_all` return a `DecodeError` on any malformed input |

Encoder / decoder round-trip is already covered structurally by the proptest-generated tests in `crates/mimir-core/tests/properties.rs` (`sem_record_roundtrips`, `edge_record_roundtrip`, etc.). Those use typed strategies the byte-level fuzzer cannot easily reach, so no separate `fuzz_roundtrip` target is maintained here.

## Running

Requires the `cargo-fuzz` CLI and a nightly toolchain:

```bash
cargo install cargo-fuzz
rustup install nightly
cd fuzz
cargo +nightly fuzz run fuzz_lex
# or: fuzz_parse, fuzz_decoder
```

Stop with Ctrl-C. Crash reproducers land under `fuzz/artifacts/<target>/`.

## Seed corpus

Under `corpus/<target>/`. Seeds are valid inputs drawn from the spec examples and the project's existing test fixtures. `cargo fuzz run` starts from these plus its own generated inputs.

- `fuzz_lex` and `fuzz_parse`: spec examples for sem / epi / pro / inf / episode / flag forms
- `fuzz_decoder`: binary canonical records. Not checked in yet — generate locally via `cargo run --example generate_decoder_seeds` when that helper lands, or hand-seed from the existing unit test byte-equality assertions in `canonical.rs`

## Scope note

The fuzz workspace lives outside the main `Cargo.toml` workspace (see `fuzz/Cargo.toml`'s `[workspace]` stub) so the main `cargo check` / `cargo test` doesn't pull in libfuzzer and nightly requirements.

Scheduled CI integration (periodic runs of each target for N minutes) is a follow-up — the local harness gives developers and release engineers an immediate smoke-check surface; continuous fuzzing can layer on later without changing these targets.
