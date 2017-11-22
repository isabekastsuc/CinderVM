//! The arena: one contiguous byte buffer holding every `Str`/`Bytes`/`List`
//! payload, addressed by arena-relative [`Handle`]s.

use crate::diag::{Code, Diag, Result};
use crate::isa::Ty;
use crate::value::{Handle, Tag, Value};

/// A grow-only byte arena with 8-byte alignment.
#[derive(Clone, Debug, Default)]
pub struct Arena {
    data: Vec<u8>,
}

impl Arena {
    #[must_use]
    pub fn new() -> Self {
        Self { data: Vec::new() }
    }

    /// Allocate `bytes`, returning a handle whose extent covers them.
    pub fn alloc(&mut self, bytes: &[u8]) -> Result<Handle> {
        let aligned = bytes.len().max(1).div_ceil(8) * 8;
        let off = self.data.len();
        if off + aligned > u32::MAX as usize {
            return Err(Diag::new(Code::ArenaExhausted, "arena exceeds 4 GiB"));
        }
        self.data.resize(off + aligned, 0);
        self.data[off..off + bytes.len()].copy_from_slice(bytes);
        Ok(Handle::new(off as u32, bytes.len() as u32))
    }

    /// Allocate from a `Value` list.
    pub fn alloc_list(&mut self, items: &[Value]) -> Result<Handle> {
        let mut buf = Vec::with_capacity(items.len() * crate::value::SLOT_LEN);
        for v in items {
            v.write(&mut buf);
        }
        self.alloc(&buf)
    }

    /// Read the bytes behind a handle, validating its extent.
    pub fn get(&self, h: Handle) -> Result<&[u8]> {
        let lo = h.off as usize;
        let hi = h.end() as usize;
        self.data
            .get(lo..hi)
            .ok_or_else(|| Diag::new(Code::BadHandle, format!("handle {h:?} outside arena")))
    }

    /// Decode a list handle into its elements.
    pub fn get_list(&self, h: Handle) -> Result<Vec<Value>> {
        let raw = self.get(h)?;
        if raw.len() % crate::value::SLOT_LEN != 0 {
            return Err(Diag::new(
                Code::BadHandle,
                "list payload is not slot-aligned",
            ));
        }
        raw.chunks_exact(crate::value::SLOT_LEN)
            .map(Value::read)
            .collect()
    }

    /// Read a handle as UTF-8.
    pub fn get_str(&self, h: Handle) -> Result<&str> {
        let raw = self.get(h)?;
        std::str::from_utf8(raw)
            .map_err(|_| Diag::new(Code::BadHandle, "str payload is not valid UTF-8"))
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// Snapshot the arena bytes for `cont::snapshot`.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
