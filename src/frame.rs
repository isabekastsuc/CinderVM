//! One execution frame: the operand stack plus frame-local slots, all in one
//! `Vec<Value>` with a movable stack pointer.

use crate::diag::{Code, Diag, Result};
use crate::isa::Ty;
use crate::value::Value;

/// A call frame.
#[derive(Clone, Debug)]
pub struct Frame {
    /// Function table index.
    pub func: u32,
    /// Instruction index within the function's code window.
    pub pc: u32,
    /// Stack and locals in one array; `sp` separates them.
    pub slots: Vec<Value>,
    /// Operand-stack pointer: `slots[..sp]` are operands, `slots[sp..]` are
    /// frame locals.
    pub sp: usize,
    /// Declared operand stack bound.
    pub maxstack: u16,
    /// Number of values to leave on the parent stack on return.
    pub returns: u8,
}

impl Frame {
    #[must_use]
    pub fn new(func: u32, args: Vec<Value>, locals: usize, maxstack: u16, returns: u8) -> Self {
        let mut slots = vec![Value::VOID; maxstack as usize + locals];
        let mut sp = 0usize;
        for v in args {
            if sp < maxstack as usize {
                slots[sp] = v;
                sp += 1;
            }
        }
        Self {
            func,
            pc: 0,
            sp,
            slots,
            maxstack,
            returns,
        }
    }

    #[must_use]
    pub fn depth(&self) -> usize {
        self.sp
    }

    pub fn push(&mut self, v: Value) -> Result<()> {
        if self.sp >= usize::from(self.maxstack) {
            return Err(Diag::new(Code::MaxstackExceeded, "operand stack overflow"));
        }
        self.slots[self.sp] = v;
        self.sp += 1;
        Ok(())
    }

    pub fn pop(&mut self) -> Result<Value> {
        if self.sp == 0 {
            return Err(Diag::new(Code::StackUnderflow, "operand stack underflow"));
        }
        self.sp -= 1;
        Ok(self.slots[self.sp])
    }

    pub fn peek(&self, n: usize) -> Result<Value> {
        self.slots.get(self.sp - 1 - n).copied().ok_or_else(|| {
            Diag::new(
                Code::StackUnderflow,
                format!("no operand {n} slots below the top"),
            )
        })
    }

    pub fn load_local(&self, idx: usize) -> Result<Value> {
        self.slots
            .get(usize::from(self.maxstack) + idx)
            .copied()
            .ok_or_else(|| Diag::new(Code::StackUnderflow, "frame-local slot out of range"))
    }

    pub fn store_local(&mut self, idx: usize, v: Value) -> Result<()> {
        let at = usize::from(self.maxstack) + idx;
        let slot = self
            .slots
            .get_mut(at)
            .ok_or_else(|| Diag::new(Code::StackUnderflow, "frame-local slot out of range"))?;
        *slot = v;
        Ok(())
    }

    /// Drop the top `n` operands.
    pub fn drop_n(&mut self, n: usize) -> Result<()> {
        if n > self.sp {
            return Err(Diag::new(
                Code::StackUnderflow,
                "cannot drop below the stack base",
            ));
        }
        self.sp -= n;
        Ok(())
    }

