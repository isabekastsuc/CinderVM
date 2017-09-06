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
        }
    }
}

impl Budget {
    #[must_use]
    pub fn new(tokens: i64, wall_ms: i64, tool_calls: i64, arena_bytes: i64) -> Self {
        Self {
            remaining: [tokens, wall_ms, tool_calls, arena_bytes],
            reserved: [0; DIM_COUNT],
        }
    }

    fn dim(d: u8) -> Result<usize> {
        if (d as usize) < DIM_COUNT {
            Ok(d as usize)
        } else {
            Err(Diag::new(
                Code::BudgetExceeded,
                format!("unknown budget dimension {d}"),
            ))
        }
    }

    /// Lease `amount` on `dim`. Refused at reserve time, never after the spend.
    pub fn reserve(&mut self, dim: u8, amount: i64) -> Result<()> {
        let d = Self::dim(dim)?;
        if amount < 0 || self.remaining[d] < amount {
            return Err(Diag::new(
                Code::BudgetExceeded,
                format!("reserve refused on dimension {d}"),
            ));
        }
        self.remaining[d] -= amount;
        self.reserved[d] += amount;
        Ok(())
    }

    /// Return an unused reservation.
    pub fn release(&mut self, dim: u8, amount: i64) -> Result<()> {
        let d = Self::dim(dim)?;
        if amount < 0 || self.reserved[d] < amount {
            return Err(Diag::new(
                Code::BudgetExceeded,
                "release exceeds reservation",
            ));
        }
        self.reserved[d] -= amount;
        self.remaining[d] += amount;
        Ok(())
    }

    /// Settle a reservation with actual usage; the difference stays spent.
    pub fn spend(&mut self, dim: u8, amount: i64) -> Result<()> {
        let d = Self::dim(dim)?;
        if amount < 0 || self.reserved[d] < amount {
            return Err(Diag::new(Code::BudgetExceeded, "spend exceeds reservation"));
        }
        self.reserved[d] -= amount;
        Ok(())
    }

    #[must_use]
    pub fn remaining(&self, dim: u8) -> i64 {
        self.remaining.get(dim as usize).copied().unwrap_or(0)
    }

    /// Account arena growth.
    pub fn charge_arena(&mut self, bytes: usize) -> Result<()> {
        let d = DIM_ARENA_BYTES as usize;
        if self.remaining[d] < bytes as i64 {
            return Err(Diag::new(Code::ArenaExhausted, "arena budget exhausted"));
        }
        self.remaining[d] -= bytes as i64;
        Ok(())
