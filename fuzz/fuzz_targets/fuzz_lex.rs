//! Fuzz target: `mimir_core::lex::tokenize` on arbitrary input bytes.
//!
//! Contract: `tokenize` must return a `Result` — never panic, never
//! loop infinitely, never produce different outputs for the same
//! input. A panic here is a bug; a `LexError` is correct behaviour.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Only valid UTF-8 is in the lexer's contract; clip on invalid
    // sequences so the fuzzer spends its time on the interesting
    // state space.
    if let Ok(s) = std::str::from_utf8(data) {
        let _ = mimir_core::lex::tokenize(s);
    }
});
