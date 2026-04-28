//! `Value` — the typed-value enum used in memory slots that accept either
//! a symbol, a number, a boolean, a string, or a timestamp. Matches the
//! value-tag set in `docs/concepts/ir-canonical-form.md` § 3.2.

use crate::clock::ClockTime;
use crate::symbol::SymbolId;

/// A typed value that can appear in a memory slot such as
/// [`crate::Semantic::o`] or [`crate::Procedural::trigger`].
///
/// Mirrors the canonical-form value tags:
/// `Symbol` (0x01), `Integer` (0x02), `Float` (0x03), `Boolean` (0x04),
/// `String` (0x05), `Timestamp` (0x06) — see `ir-canonical-form.md` § 3.2.
///
/// # Examples
///
/// ```
/// use mimir_core::{SymbolId, Value};
///
/// let v = Value::Symbol(SymbolId::new(42));
/// assert!(matches!(v, Value::Symbol(_)));
/// ```
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// A symbol reference (wire tag `0x01`).
    Symbol(SymbolId),
    /// A signed 64-bit integer (wire tag `0x02`, `ZigZag` varint-encoded).
    Integer(i64),
    /// An IEEE 754 binary64 float (wire tag `0x03`).
    Float(f64),
    /// A boolean (wire tag `0x04`).
    Boolean(bool),
    /// A UTF-8 string literal (wire tag `0x05`).
    String(String),
    /// A timestamp (wire tag `0x06`).
    Timestamp(ClockTime),
}

impl Value {
    /// Canonical-byte encoding used as an in-process `BTreeMap` key
    /// — `Value` cannot implement `Ord` (it carries `f64`), but its
    /// canonical-form encoding is byte-deterministic.
    ///
    /// `pub(crate)` rather than `pub` on purpose: the output is NOT
    /// stable across wire-format revisions, so exposing it invites
    /// callers to persist it as an identifier. The
    /// `mimir_core::canonical` encoder remains the public boundary
    /// for wire-format work.
    #[must_use]
    pub(crate) fn index_key_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        crate::canonical::encode_value(self, &mut out);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn value_variants_compare_by_content() {
        assert_eq!(Value::Integer(5), Value::Integer(5));
        assert_ne!(Value::Integer(5), Value::Integer(6));
        assert_ne!(Value::Integer(5), Value::Boolean(false));
    }

    #[test]
    fn float_value_preserves_precision() {
        let a = Value::Float(1.25);
        let b = Value::Float(1.25);
        assert_eq!(a, b);
    }
}
