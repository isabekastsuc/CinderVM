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
