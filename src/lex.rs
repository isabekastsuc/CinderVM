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
