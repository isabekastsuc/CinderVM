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
    verified: bool,
}

impl Image {
    /// Image name as given at assembly time (the `.cdx` stem).
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The constant pool (raw little-endian bytes per constant).
    pub fn constants(&self) -> &[Vec<u8>] {
        &self.constants
    }

    /// The function table, in call-index order.
    pub fn funcs(&self) -> &[FuncMeta] {
        &self.funcs
    }

    /// External tool names addressable by the `TOOL` instruction.
    pub fn tools(&self) -> &[String] {
        &self.tools
    }

    /// The flat code section all functions share.
    pub fn code(&self) -> &[u8] {
        &self.code
    }

    /// Jump tables, indexed by the `SWITCH` operand.
    pub fn jump_tables(&self) -> &[Vec<u32>] {
        &self.jump_tables
    }

    /// Whether the verifier sealed this image.
    pub fn is_verified(&self) -> bool {
        self.verified
    }

    /// FNV-1a over every serialized byte except the digest itself.
    pub fn checksum(&self) -> u64 {
        hash::hash_bytes(&self.to_bytes()[..].to_vec())
    }

    /// Serialize to the `.cdxb` container layout.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&CDXB_MAGIC);
        out.push(CDXB_VERSION);
        put_str(&mut out, &self.name);

        put_u32(&mut out, self.constants.len() as u32);
        for c in &self.constants {
            put_bytes(&mut out, c);
        }

        put_u32(&mut out, self.funcs.len() as u32);
        for f in &self.funcs {
            put_str(&mut out, &f.name);
            put_u32(&mut out, f.code_off);
            put_u32(&mut out, f.code_len);
            put_u16(&mut out, f.maxstack);
            out.push(f.args);
            out.push(f.returns);
        }

        put_u32(&mut out, self.tools.len() as u32);
        for t in &self.tools {
            put_str(&mut out, t);
        }

        put_bytes(&mut out, &self.code);

        put_u32(&mut out, self.jump_tables.len() as u32);
        for t in &self.jump_tables {
            put_u32(&mut out, t.len() as u32);
            for &v in t {
                put_u32(&mut out, v);
            }
        }

        out.push(u8::from(self.verified));
        let digest = hash::hash_bytes(&out);
        out.extend_from_slice(&digest.to_le_bytes());
        out
    }

    /// Bytes that participate in the digest (everything but the trailing 8).
    pub fn payload_bytes(&self) -> Vec<u8> {
        let full = self.to_bytes();
        full[..full.len() - 8].to_vec()
    }

    /// Parse and check a `.cdxb` container: magic, version, section bounds,
    /// and digest. A container that survives this is structurally sound;
    /// semantic invariants are [`crate::verify`]'s job.
    pub fn from_bytes(bytes: &[u8]) -> Result<Image> {
        if bytes.len() < 5 {
            return Err(Diag::new(Code::BadMagic, "container too short"));
        }
        if bytes[0..4] != CDXB_MAGIC {
            return Err(Diag::new(Code::BadMagic, "not a .cdxb container"));
        }
        if bytes[4] != CDXB_VERSION {
            return Err(Diag::new(
                Code::BadMagic,
                format!("unsupported container version {}", bytes[4]),
            ));
        }
        let Some(body_end) = bytes.len().checked_sub(8) else {
        return Err(Diag::new(Code::BadMagic, "container too short"));
    };
    let body = &bytes[..body_end];
        let mut cur = Cursor {
            data: bytes,
            pos: 0,
        };
        cur.pos = 5;

        let name = cur.take_str()?;

        let n_consts = cur.take_u32()? as usize;
        let mut constants = Vec::with_capacity(n_consts.min(4096));
        for _ in 0..n_consts {
            constants.push(cur.take_byte_vec()?);
        }

        let n_funcs = cur.take_u32()? as usize;
        let mut funcs = Vec::with_capacity(n_funcs.min(4096));
        for _ in 0..n_funcs {
            let name = cur.take_str()?;
            let code_off = cur.take_u32()?;
            let code_len = cur.take_u32()?;
            let maxstack = cur.take_u16()?;
            let args = cur.take_u8()?;
            let returns = cur.take_u8()?;
            funcs.push(FuncMeta {
                name,
                code_off,
                code_len,
                maxstack,
                args,
                returns,
            });
        }

        let n_tools = cur.take_u32()? as usize;
        let mut tools = Vec::with_capacity(n_tools.min(4096));
        for _ in 0..n_tools {
            tools.push(cur.take_str()?);
        }

        let code = cur.take_byte_vec()?;

        let n_tables = cur.take_u32()? as usize;
        let mut jump_tables = Vec::with_capacity(n_tables.min(4096));
        for _ in 0..n_tables {
            let n = cur.take_u32()? as usize;
            if cur.pos + n * 4 > body.len() {
                return Err(Diag::new(
                    Code::TruncatedSection,
                    "jump table extends past end of file",
                ));
            }
            let mut t = Vec::with_capacity(n.min(65536));
            for _ in 0..n {
                t.push(cur.take_u32()?);
            }
            jump_tables.push(t);
        }

        let verified = cur.take_u8()? == 1;
        if cur.pos != body.len() {
            return Err(Diag::new(
                Code::TruncatedSection,
                "trailing bytes in container",
            ));
        }

        let stored = u64::from_le_bytes(
            bytes[bytes.len() - 8..]
                .try_into()
                .map_err(|_| Diag::new(Code::BadChecksum, "digest unreadable"))?,
        );
        if hash::hash_bytes(body) != stored {
            return Err(Diag::new(
                Code::BadChecksum,
                "image digest does not match its contents",
            ));
        }

        Ok(Image {
            name,
            constants,
            funcs,
            tools,
            code,
            jump_tables,
            verified,
        })
    }
}

/// Decode the entire code section in `INSN_LEN` strides. The assembler only
/// emits known opcodes, so a byte that fails to decode here means the object
/// was tampered with between assembly and sealing.
pub fn seal_decode_check(code: &[u8]) -> Result<()> {
    if code.len() % crate::isa::INSN_LEN != 0 {
        return Err(Diag::new(
            Code::TruncatedSection,
            "code section is not a multiple of the instruction length",
        ));
    }
    let mut off = 0;
    while off < code.len() {
        if crate::isa::decode(code, off).is_none() {
            return Err(Diag::new(
                Code::UnassignedOpcode,
                format!("opcode byte {:#04x} does not decode", code[off]),
            ));
        }
        off += crate::isa::INSN_LEN;
    }
    Ok(())
}

struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl Cursor<'_> {
    fn take_u8(&mut self) -> Result<u8> {
        let b = *self
            .data
            .get(self.pos)
            .ok_or_else(|| Diag::new(Code::TruncatedSection, "section extends past end of file"))?;
        self.pos += 1;
        Ok(b)
    }

    fn take_u16(&mut self) -> Result<u16> {
        let lo = self.take_u8()? as u16;
        let hi = self.take_u8()? as u16;
        Ok(lo | (hi << 8))
    }

    fn take_u32(&mut self) -> Result<u32> {
        let mut buf = [0u8; 4];
        for slot in &mut buf {
            *slot = self.take_u8()?;
        }
        Ok(u32::from_le_bytes(buf))
    }

    fn take_raw(&mut self, n: usize) -> Result<&[u8]> {
        if self.pos + n > self.data.len() {
            return Err(Diag::new(
                Code::TruncatedSection,
                "section extends past end of file",
            ));
        }
        let s = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    fn take_str(&mut self) -> Result<String> {
        let n = self.take_u32()? as usize;
        let raw = self.take_raw(n)?;
        String::from_utf8(raw.to_vec())
            .map_err(|_| Diag::new(Code::BadMagic, "section is not valid UTF-8"))
    }

    fn take_byte_vec(&mut self) -> Result<Vec<u8>> {
        let n = self.take_u32()? as usize;
        Ok(self.take_raw(n)?.to_vec())
    }
}

fn put_u16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn put_str(out: &mut Vec<u8>, s: &str) {
    put_u32(out, s.len() as u32);
    out.extend_from_slice(s.as_bytes());
}

fn put_bytes(out: &mut Vec<u8>, b: &[u8]) {
    put_u32(out, b.len() as u32);
    out.extend_from_slice(b);
}
