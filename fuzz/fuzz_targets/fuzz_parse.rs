//! Fuzz target: `mimir_core::parse::parse` on arbitrary UTF-8 input.
//!
//! Contract: every byte sequence that is valid UTF-8 either parses
//! to a `Vec<UnboundForm>` or returns a `ParseError`. Panics indicate
//! a bug in the lexer or parser.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        let _ = mimir_core::parse::parse(s);
    }
});
