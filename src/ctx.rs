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
        for _ in 0..n {
            self.segments.pop_front();
        }
        Ok(())
    }

    /// Materialize the current window as a list of segment values.
    #[must_use]
    pub fn window(&self) -> Vec<Value> {
        self.segments
            .iter()
            .map(|s| Value::int(s[0] as i64))
            .collect()
    }

    /// Approximate token cost: bytes in the window.
    #[must_use]
    pub fn cost(&self) -> i64 {
        self.segments.iter().map(|s| s.len() as i64).sum()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.segments.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_evicts_oldest_first() {
        let mut r = ContextRing::new(3);
        r.push(0, b"a").unwrap();
        r.push(0, b"b").unwrap();
        r.push(0, b"c").unwrap();
        assert!(r.push(0, b"d").is_err());
        r.pop_oldest(1).unwrap();
        r.push(0, b"d").unwrap();
        assert_eq!(r.len(), 3);
        assert_eq!(r.cost(), 6);
    }
}
