//! The context ring: an explicit, eviction-only window over the agent's
//! conversation segments.

use crate::diag::{Code, Diag, Result};
use crate::value::Value;
use std::collections::VecDeque;

/// A bounded ring of context segments.
#[derive(Clone, Debug)]
pub struct ContextRing {
    segments: VecDeque<Vec<u8>>,
    limit: usize,
}

impl ContextRing {
    #[must_use]
    pub fn new(limit: usize) -> Self {
        Self {
            segments: VecDeque::new(),
            limit: limit.max(1),
        }
    }

    /// Append a segment with an implicit role tag.
    pub fn push(&mut self, role: u8, bytes: &[u8]) -> Result<()> {
        let mut seg = Vec::with_capacity(bytes.len() + 1);
        seg.push(role);
        seg.extend_from_slice(bytes);
        if self.segments.len() >= self.limit {
            return Err(Diag::new(
                Code::ArenaExhausted,
                "context ring full; evict first",
            ));
        }
        self.segments.push_back(seg);
        Ok(())
    }

    /// Drop the `b` oldest segments.
    pub fn pop_oldest(&mut self, n: usize) -> Result<()> {
        if n > self.segments.len() {
            return Err(Diag::new(Code::IndexRange, "eviction exceeds ring size"));
        }
