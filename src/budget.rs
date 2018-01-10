//! Metering: reservations against four budget dimensions.

use crate::diag::{Code, Diag, Result};

/// Budget dimension indices, matching the `Dimension` operand class.
pub const DIM_TOKENS: u8 = 0;
pub const DIM_WALL_MS: u8 = 1;
pub const DIM_TOOL_CALLS: u8 = 2;
pub const DIM_ARENA_BYTES: u8 = 3;
pub const DIM_COUNT: usize = 4;

/// Four-dimensional allowance ledger.
#[derive(Clone, Debug)]
pub struct Budget {
    /// Remaining allowance per dimension.
    remaining: [i64; DIM_COUNT],
    /// Reservations held by `reserve`.
    reserved: [i64; DIM_COUNT],
}

impl Default for Budget {
    fn default() -> Self {
        Self {
            remaining: [i64::MAX, i64::MAX, i64::MAX, i64::MAX],
            reserved: [0; DIM_COUNT],
