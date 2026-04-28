//! Fuzz target: `mimir_core::canonical::decode_record` +
//! `decode_all` on arbitrary bytes.
//!
//! Contract: the canonical decoder must return a `DecodeError` on
//! any malformed input — never panic, never overrun, never report
//! success for an incomplete record.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Single-record decode — must always return Result.
    let _ = mimir_core::canonical::decode_record(data);
    // Bulk decode — same contract for the streaming boundary.
    let _ = mimir_core::canonical::decode_all(data);
});
