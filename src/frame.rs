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

    /// Pop `n` operands, first-pushed first.
    pub fn pop_n(&mut self, n: usize) -> Result<Vec<Value>> {
        if n > self.sp {
            return Err(Diag::new(
                Code::StackUnderflow,
                "cannot pop below the stack base",
            ));
        }
        let out = self.slots[self.sp - n..self.sp].to_vec();
        self.sp -= n;
        Ok(out)
    }

    /// Verify a static type assertion at the top of the stack.
    pub fn check_top(&self, want: Ty) -> Result<()> {
        let v = self.peek(0)?;
        if v.ty().satisfies(want) {
            Ok(())
        } else {
            Err(Diag::new(
                Code::TagMismatch,
                format!("expected {}, found {}", want.name(), v.ty().name()),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_pop_and_locals_are_separated() {
        let mut f = Frame::new(0, vec![Value::int(7)], 2, 8, 1);
        f.push(Value::int(1)).unwrap();
        f.store_local(0, Value::int(9)).unwrap();
        assert_eq!(f.pop().unwrap().as_int().unwrap(), 1);
        assert_eq!(f.load_local(0).unwrap().as_int().unwrap(), 9);
        assert_eq!(f.pop().unwrap().as_int().unwrap(), 7);
        assert!(f.pop().is_err(), "base underflow must error");
    }

    #[test]
    fn maxstack_is_enforced() {
        let mut f = Frame::new(0, Vec::new(), 0, 2, 0);
        f.push(Value::int(1)).unwrap();
        f.push(Value::int(2)).unwrap();
        assert_eq!(
            f.push(Value::int(3)).unwrap_err().code,
            Code::MaxstackExceeded
        );
    }
}
