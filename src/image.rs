//! The `.cdxb` container: an assembled, sealed, verifiable image.
//!
//! [`Image`] is the whole program: constant pool, function table, tool
//! names, machine code, and jump tables. It has no public constructor -
//! [`crate::verify::admit`] is the only way to obtain a sealed one, which is
//! what lets the rest of the crate assume the invariants proven there.
//! Serialization is a flat little-endian layout with an FNV-1a digest over
//! everything that precedes it, so [`Image::from_bytes`] rejects corruption
//! before the VM ever sees a byte of code.

use crate::diag::{Code, Diag, Result};
use crate::hash;

/// Magic bytes every `.cdxb` container starts with.
pub const CDXB_MAGIC: [u8; 4] = *b"CDXB";

/// Container format version written by [`Image::to_bytes`].
pub const CDXB_VERSION: u8 = 1;

/// Assemble an image from already-verified parts. Callers must be
/// `crate::verify::admit` - everything else goes through it.
pub fn build(
    name: String,
    constants: Vec<Vec<u8>>,
    funcs: Vec<FuncMeta>,
    tools: Vec<String>,
    code: Vec<u8>,
    jump_tables: Vec<Vec<u32>>,
) -> Image {
    Image {
        name,
        constants,
        funcs,
        tools,
        code,
        jump_tables,
        verified: false,
    }
}

/// Flip the seal once the verifier has proven its invariants.
pub fn mark_verified(image: &mut Image) {
    image.verified = true;
}

/// Static description of one function: where its code lives and its
/// stack contract. Produced by the assembler, consumed by the verifier,
/// the CFG builder, and the interpreter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FuncMeta {
    /// Symbol name (`main` is the entry point).
    pub name: String,
    /// Offset of the first instruction inside [`Image::code`].
    pub code_off: u32,
    /// Length of the function body in bytes (multiple of `INSN_LEN`).
    pub code_len: u32,
    /// Maximum operand-stack depth the verifier proved for this function.
    pub maxstack: u16,
    /// Number of arguments the function pops on call.
    pub args: u8,
    /// Number of values the function leaves behind on return (0 or 1).
    pub returns: u8,
}

/// A sealed, verified program image.
#[derive(Clone, Debug)]
pub struct Image {
    name: String,
    constants: Vec<Vec<u8>>,
    funcs: Vec<FuncMeta>,
    tools: Vec<String>,
    code: Vec<u8>,
    jump_tables: Vec<Vec<u32>>,
