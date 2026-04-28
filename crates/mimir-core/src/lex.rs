//! Lisp S-expression lexer for the Mimir write surface.
//!
//! Implements the lexical grammar in `docs/concepts/ir-write-surface.md`
//! § 3. Produces a stream of [`Token`]s from UTF-8 input; errors carry a
//! [`Position`] pointing at the offending byte.

use std::str::Chars;

use thiserror::Error;

/// A byte-and-line position in the input, 1-based line/column.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Position {
    /// 0-based byte offset into the input.
    pub offset: usize,
    /// 1-based line number (newline-delimited).
    pub line: usize,
    /// 1-based column number.
    pub column: usize,
}

impl Position {
    /// Start-of-input position.
    #[must_use]
    pub const fn start() -> Self {
        Self {
            offset: 0,
            line: 1,
            column: 1,
        }
    }
}

/// A lexical token.
///
/// Matches the token classes in `ir-write-surface.md` § 3.1. String-
/// bearing variants own their content (copying out of the input); this
/// trades a small allocation per token for simpler parser lifetimes.
#[derive(Clone, Debug, PartialEq)]
pub enum Token {
    /// `@name` — a symbol reference. Leading `@` stripped.
    Symbol(String),
    /// `@name:Kind` — a symbol reference with a kind annotation. The
    /// `Kind` string is passed through for later validation in the
    /// parser / binder.
    TypedSymbol {
        /// The `@name` part without the `@` prefix.
        name: String,
        /// The `:Kind` part without the leading `:`.
        kind: String,
    },
    /// A bareword identifier — matches `[a-z_][a-z0-9_]*`. Serves as
    /// opcode at form heads, predicate in predicate slots, string
    /// literal elsewhere (disambiguation is the parser's job).
    Bareword(String),
    /// An ISO-8601-UTC timestamp bareword (date-only or full date-time
    /// with optional millisecond fraction and a `Z` suffix).
    Timestamp(String),
    /// A signed 64-bit integer literal.
    Integer(i64),
    /// An IEEE 754 binary64 float literal (contains a `.`).
    Float(f64),
    /// A double-quoted UTF-8 string with escape sequences resolved.
    String(String),
    /// Boolean literal `true` or `false`.
    Boolean(bool),
    /// `nil` null literal.
    Nil,
    /// `:keyword` — keyword argument tag, without the leading `:`.
    Keyword(String),
    /// Open parenthesis `(`.
    LParen,
    /// Close parenthesis `)`.
    RParen,
}

/// A [`Token`] paired with its source position.
#[derive(Clone, Debug, PartialEq)]
pub struct Spanned {
    /// The token.
    pub token: Token,
    /// Start position of the token in the input.
    pub position: Position,
}

/// Errors produced by [`tokenize`].
#[derive(Debug, Error, PartialEq)]
pub enum LexError {
    /// A `"`-quoted string was not terminated before end-of-input.
    #[error("unterminated string starting at {start:?}")]
    UnterminatedString {
        /// Position of the opening `"`.
        start: Position,
    },

    /// A `\x` escape sequence used an unsupported character.
    #[error("invalid escape '\\{escape}' at {pos:?}")]
    InvalidEscape {
        /// The character after the backslash.
        escape: char,
        /// Position of the backslash.
        pos: Position,
    },

    /// A numeric token could not be parsed.
    #[error("invalid number {text:?} at {pos:?}")]
    InvalidNumber {
        /// The raw text.
        text: String,
        /// Start position.
        pos: Position,
    },

    /// An identifier or kind annotation is ill-formed.
    #[error("invalid identifier {text:?} at {pos:?}")]
    InvalidIdentifier {
        /// The raw text.
        text: String,
        /// Start position.
        pos: Position,
    },

    /// A byte that cannot start any token (e.g. stray punctuation).
    #[error("unexpected byte {byte:#04x} at {pos:?}")]
    UnexpectedByte {
        /// The offending byte.
        byte: u8,
        /// Start position.
        pos: Position,
    },

    /// Input was not valid UTF-8 at the cursor.
    #[error("invalid UTF-8 at {pos:?}")]
    InvalidUtf8 {
        /// Position of the bad byte.
        pos: Position,
    },
}

/// Tokenize a UTF-8 input into a vector of [`Spanned`] tokens.
///
/// Comments (`; … \n`) and whitespace are dropped.
///
/// # Errors
///
/// Returns a [`LexError`] on any lexical violation; the error carries a
/// [`Position`] pointing at the offending byte.
///
/// # Examples
///
/// ```
/// # #![allow(clippy::unwrap_used)]
/// use mimir_core::lex::{tokenize, Token};
///
/// let tokens = tokenize("(sem @alice email \"alice@example.com\")").unwrap();
/// assert_eq!(tokens.first().map(|s| &s.token), Some(&Token::LParen));
/// assert_eq!(tokens.last().map(|s| &s.token), Some(&Token::RParen));
/// ```
pub fn tokenize(input: &str) -> Result<Vec<Spanned>, LexError> {
    let mut lexer = Lexer::new(input);
    let mut out = Vec::new();
    while let Some(spanned) = lexer.next_token()? {
        out.push(spanned);
    }
    Ok(out)
}

struct Lexer<'a> {
    input: &'a str,
    chars: Chars<'a>,
    pos: Position,
}

impl<'a> Lexer<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            input,
            chars: input.chars(),
            pos: Position::start(),
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.clone().next()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.chars.next()?;
        let len = c.len_utf8();
        self.pos.offset += len;
        if c == '\n' {
            self.pos.line += 1;
            self.pos.column = 1;
        } else {
            self.pos.column += 1;
        }
        Some(c)
    }

    fn skip_whitespace_and_comments(&mut self) {
        while let Some(c) = self.peek() {
            if c.is_whitespace() {
                self.bump();
            } else if c == ';' {
                // Line comment runs to newline (not included).
                while let Some(cc) = self.peek() {
                    if cc == '\n' {
                        break;
                    }
                    self.bump();
                }
            } else {
                break;
            }
        }
    }

    fn next_token(&mut self) -> Result<Option<Spanned>, LexError> {
        self.skip_whitespace_and_comments();
        let start = self.pos;
        let Some(c) = self.peek() else {
            return Ok(None);
        };
        let token = match c {
            '(' => {
                self.bump();
                Token::LParen
            }
            ')' => {
                self.bump();
                Token::RParen
            }
            '"' => self.lex_string(start)?,
            '@' => self.lex_symbol_or_typed(start)?,
            ':' => self.lex_keyword(start)?,
            '-' | '0'..='9' => self.lex_number_or_timestamp(start)?,
            'a'..='z' | '_' => self.lex_bareword_or_reserved(start)?,
            _ => {
                let byte = c as u32;
                #[allow(clippy::cast_possible_truncation)]
                return Err(LexError::UnexpectedByte {
                    byte: byte as u8,
                    pos: start,
                });
            }
        };
        Ok(Some(Spanned {
            token,
            position: start,
        }))
    }

    fn lex_string(&mut self, start: Position) -> Result<Token, LexError> {
        self.bump(); // consume opening quote
        let mut buf = String::new();
        loop {
            let pos = self.pos;
            let Some(c) = self.bump() else {
                return Err(LexError::UnterminatedString { start });
            };
            match c {
                '"' => return Ok(Token::String(buf)),
                '\\' => {
                    let Some(esc) = self.bump() else {
                        return Err(LexError::UnterminatedString { start });
                    };
                    let resolved = match esc {
                        'n' => '\n',
                        'r' => '\r',
                        't' => '\t',
                        '\\' => '\\',
                        '"' => '"',
                        other => return Err(LexError::InvalidEscape { escape: other, pos }),
                    };
                    buf.push(resolved);
                }
                other => buf.push(other),
            }
        }
    }

    fn lex_symbol_or_typed(&mut self, start: Position) -> Result<Token, LexError> {
        self.bump(); // consume '@'
        let name_start = self.pos.offset;
        self.consume_identifier();
        let name_end = self.pos.offset;
        let name = self.input[name_start..name_end].to_string();
        if name.is_empty() || !is_valid_identifier_start(&name) {
            return Err(LexError::InvalidIdentifier {
                text: format!("@{name}"),
                pos: start,
            });
        }
        if self.peek() == Some(':') {
            self.bump();
            let kind_start = self.pos.offset;
            self.consume_kind_annotation();
            let kind_end = self.pos.offset;
            let kind = self.input[kind_start..kind_end].to_string();
            if kind.is_empty() || !is_valid_kind_annotation(&kind) {
                return Err(LexError::InvalidIdentifier {
                    text: format!("@{name}:{kind}"),
                    pos: start,
                });
            }
            Ok(Token::TypedSymbol { name, kind })
        } else {
            Ok(Token::Symbol(name))
        }
    }

    fn lex_keyword(&mut self, start: Position) -> Result<Token, LexError> {
        self.bump(); // consume ':'
        let name_start = self.pos.offset;
        self.consume_identifier();
        let name_end = self.pos.offset;
        let name = self.input[name_start..name_end].to_string();
        if name.is_empty() || !is_valid_identifier_start(&name) {
            return Err(LexError::InvalidIdentifier {
                text: format!(":{name}"),
                pos: start,
            });
        }
        Ok(Token::Keyword(name))
    }

    fn lex_number_or_timestamp(&mut self, start: Position) -> Result<Token, LexError> {
        let begin = self.pos.offset;
        // Consume the number / timestamp body — digits, `-`, `.`, `T`, `:`, `Z`.
        // The classifier below decides whether the result is an Integer,
        // Float, or Timestamp.
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() || matches!(c, '-' | '.' | ':' | 'T' | 'Z') {
                self.bump();
            } else {
                break;
            }
        }
        let end = self.pos.offset;
        let text = &self.input[begin..end];
        if looks_like_timestamp(text) {
            return Ok(Token::Timestamp(text.to_string()));
        }
        if text.contains('.') {
            text.parse::<f64>()
                .map(Token::Float)
                .map_err(|_| LexError::InvalidNumber {
                    text: text.to_string(),
                    pos: start,
                })
        } else {
            text.parse::<i64>()
                .map(Token::Integer)
                .map_err(|_| LexError::InvalidNumber {
                    text: text.to_string(),
                    pos: start,
                })
        }
    }

    fn lex_bareword_or_reserved(&mut self, start: Position) -> Result<Token, LexError> {
        let begin = self.pos.offset;
        self.consume_identifier();
        let end = self.pos.offset;
        let text = &self.input[begin..end];
        let token = match text {
            "true" => Token::Boolean(true),
            "false" => Token::Boolean(false),
            "nil" => Token::Nil,
            _ => {
                if is_valid_identifier_start(text) {
                    Token::Bareword(text.to_string())
                } else {
                    return Err(LexError::InvalidIdentifier {
                        text: text.to_string(),
                        pos: start,
                    });
                }
            }
        };
        Ok(token)
    }

    fn consume_identifier(&mut self) {
        while let Some(c) = self.peek() {
            if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' {
                self.bump();
            } else {
                break;
            }
        }
    }

    fn consume_kind_annotation(&mut self) {
        // Kind annotations start with an ASCII uppercase letter and
        // continue with alphanumeric characters.
        while let Some(c) = self.peek() {
            if c.is_ascii_alphabetic() || c.is_ascii_digit() {
                self.bump();
            } else {
                break;
            }
        }
    }
}

fn is_valid_identifier_start(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_lowercase() || c == '_' => {
            chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        }
        _ => false,
    }
}

fn is_valid_kind_annotation(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_uppercase() => chars.all(char::is_alphanumeric),
        _ => false,
    }
}

fn looks_like_timestamp(text: &str) -> bool {
    // Canonical formats per ir-write-surface.md § 3.1:
    //   YYYY-MM-DD
    //   YYYY-MM-DDTHH:MM:SS[Z|.<frac>Z]
    let bytes = text.as_bytes();
    if bytes.len() < 10 {
        return false;
    }
    // YYYY-MM-DD
    if !(bytes[..4].iter().all(u8::is_ascii_digit)
        && bytes[4] == b'-'
        && bytes[5..7].iter().all(u8::is_ascii_digit)
        && bytes[7] == b'-'
        && bytes[8..10].iter().all(u8::is_ascii_digit))
    {
        return false;
    }
    if bytes.len() == 10 {
        return true;
    }
    // Full date-time: must have 'T' after the date portion.
    if bytes[10] != b'T' {
        return false;
    }
    // Remainder is HH:MM:SS[Z|.frac Z]. Minimal sanity check — the
    // format is accepted here and later normalised by the binder.
    let rest = &bytes[11..];
    rest.contains(&b':')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn first(tokens: &[Spanned]) -> &Token {
        &tokens[0].token
    }

    #[test]
    fn empty_input_produces_no_tokens() {
        assert!(tokenize("").unwrap().is_empty());
        assert!(tokenize("   \t\n  ").unwrap().is_empty());
    }

    #[test]
    fn parens_are_tokens() {
        let t = tokenize("( )").unwrap();
        assert_eq!(t.len(), 2);
        assert_eq!(first(&t), &Token::LParen);
        assert_eq!(&t[1].token, &Token::RParen);
    }

    #[test]
    fn symbol_with_and_without_kind() {
        let t = tokenize("@alice @alice:Agent").unwrap();
        assert_eq!(first(&t), &Token::Symbol("alice".into()));
        assert_eq!(
            &t[1].token,
            &Token::TypedSymbol {
                name: "alice".into(),
                kind: "Agent".into(),
            }
        );
    }

    #[test]
    fn bareword_and_reserved_words() {
        let t = tokenize("email true false nil sem").unwrap();
        assert_eq!(first(&t), &Token::Bareword("email".into()));
        assert_eq!(&t[1].token, &Token::Boolean(true));
        assert_eq!(&t[2].token, &Token::Boolean(false));
        assert_eq!(&t[3].token, &Token::Nil);
        assert_eq!(&t[4].token, &Token::Bareword("sem".into()));
    }

    #[test]
    fn numbers_distinguish_int_and_float() {
        let t = tokenize("42 -17 3.14 -0.5").unwrap();
        assert_eq!(first(&t), &Token::Integer(42));
        assert_eq!(&t[1].token, &Token::Integer(-17));
        match &t[2].token {
            Token::Float(f) => assert!((f - 3.14).abs() < 1e-9),
            other => panic!("expected Float, got {other:?}"),
        }
        match &t[3].token {
            Token::Float(f) => assert!((f + 0.5).abs() < 1e-9),
            other => panic!("expected Float, got {other:?}"),
        }
    }

    #[test]
    fn timestamps_are_distinct_from_numbers() {
        let t = tokenize("2024-01-15 2026-04-17T10:00:00Z").unwrap();
        match first(&t) {
            Token::Timestamp(s) => assert_eq!(s, "2024-01-15"),
            other => panic!("expected Timestamp, got {other:?}"),
        }
        match &t[1].token {
            Token::Timestamp(s) => assert_eq!(s, "2026-04-17T10:00:00Z"),
            other => panic!("expected Timestamp, got {other:?}"),
        }
    }

    #[test]
    fn strings_resolve_escapes() {
        let t = tokenize(r#" "hello\nworld" "a\"b" "#).unwrap();
        assert_eq!(first(&t), &Token::String("hello\nworld".into()));
        assert_eq!(&t[1].token, &Token::String("a\"b".into()));
    }

    #[test]
    fn keyword_stripped_of_colon() {
        let t = tokenize(":src :confidence_threshold").unwrap();
        assert_eq!(first(&t), &Token::Keyword("src".into()));
        assert_eq!(&t[1].token, &Token::Keyword("confidence_threshold".into()));
    }

    #[test]
    fn line_comments_skipped() {
        let t = tokenize("; a comment\n@alice").unwrap();
        assert_eq!(t.len(), 1);
        assert_eq!(first(&t), &Token::Symbol("alice".into()));
    }

    #[test]
    fn unterminated_string_errors() {
        let result = tokenize(r#" "no close "#);
        assert!(matches!(result, Err(LexError::UnterminatedString { .. })));
    }

    #[test]
    fn invalid_escape_errors() {
        let result = tokenize(r#" "\q" "#);
        assert!(matches!(
            result,
            Err(LexError::InvalidEscape { escape: 'q', .. })
        ));
    }

    #[test]
    fn unexpected_byte_errors() {
        let result = tokenize("$");
        assert!(matches!(result, Err(LexError::UnexpectedByte { .. })));
    }

    #[test]
    fn positions_track_line_and_column() {
        let t = tokenize("(\n@alice").unwrap();
        assert_eq!(t[0].position.line, 1);
        assert_eq!(t[0].position.column, 1);
        assert_eq!(t[1].position.line, 2);
        assert_eq!(t[1].position.column, 1);
    }
}
