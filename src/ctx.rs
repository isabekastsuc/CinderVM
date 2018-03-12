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
