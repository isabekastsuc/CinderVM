//! The `.cdx` tokenizer. Line-oriented: directives, mnemonics, labels,
//! integers, and strings.

use crate::diag::{Code, Diag, Result, Span};

/// Token kinds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Tok {
    /// `.directive`
    Directive(String),
    /// `name:`
    Label(String),
    /// Bare identifier (mnemonic, symbol, function name).
    Ident(String),
    /// Integer literal.
    Int(i64),
    /// String literal with escapes resolved.
    Str(String),
    /// End of a logical line.
    Newline,
    End,
}

#[derive(Clone, Debug)]
pub struct Token {
    pub tok: Tok,
    pub span: Span,
}

/// Tokenize `.cdx` source.
pub fn lex(source: &str) -> Result<Vec<Token>> {
    let mut out = Vec::new();
    let bytes = source.as_bytes();
    let mut i = 0;
    let mut line = 1u32;
    let mut col = 1u32;
    while i < bytes.len() {
        let b = bytes[i];
        match b {
            b' ' | b'\t' | b'\r' => {
                i += 1;
                col += 1;
            }
            b'\n' => {
                out.push(Token {
                    tok: Tok::Newline,
                    span: Span::new(i as u32, i as u32 + 1, line, col),
                });
                i += 1;
                line += 1;
                col = 1;
            }
            b'#' => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                    col += 1;
                }
            }
            b'"' => {
                let (s, next, ncol) = lex_string(bytes, i, line, col)?;
                out.push(Token {
                    tok: Tok::Str(s),
                    span: Span::new(i as u32, next as u32, line, col),
                });
                i = next;
                col = ncol;
            }
            b'.' => {
                let start = i;
                i += 1;
                col += 1;
                let mut name = String::new();
                while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                    name.push(bytes[i] as char);
                    i += 1;
                    col += 1;
                }
                if name.is_empty() {
                    return Err(Diag::new(Code::StrayChar, "directive with no name")
                        .at(Span::new(start as u32, i as u32, line, col)));
                }
                out.push(Token {
                    tok: Tok::Directive(name),
                    span: Span::new(start as u32, i as u32, line, col),
                });
            }
            b'0'..=b'9' => {
                let start = i;

                let mut value: i64 = 0;
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    value = value
                        .saturating_mul(10)
                        .saturating_add(i64::from(bytes[i] - b'0'));
                    i += 1;
                    col += 1;
                }

                let _ = start;
                out.push(Token {
                    tok: Tok::Int(value),
                    span: Span::new(start as u32, i as u32, line, col),
                });
            }
            b'-' => {
                let start = i;
                i += 1;
                col += 1;
                if i < bytes.len() && bytes[i] == b'>' {
                    out.push(Token {
                        tok: Tok::Ident("->".into()),
                        span: Span::new(start as u32, i as u32 + 1, line, col),
                    });
                    i += 1;
                    col += 1;
                } else if i < bytes.len() && bytes[i].is_ascii_digit() {
                    let mut value: i64 = 0;
                    while i < bytes.len() && bytes[i].is_ascii_digit() {
                        value = value
                            .saturating_mul(10)
                            .saturating_add(i64::from(bytes[i] - b'0'));
                        i += 1;
                        col += 1;
                    }
                    out.push(Token {
                        tok: Tok::Int(-value),
                        span: Span::new(start as u32, i as u32, line, col),
                    });
                } else {
                    return Err(Diag::new(Code::BadNumber, "expected digits after '-'")
                        .at(Span::new(start as u32, i as u32, line, col)));
                }
            }
            b':' => {
                return Err(
                    Diag::new(Code::StrayChar, "':' without a label").at(Span::new(
                        i as u32,
                        i as u32 + 1,
                        line,
                        col,
