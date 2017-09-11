//! The dispatch loop. The interpreter performs no I/O; effects surface as
//! [`Trap`]s and the host answers with [`Answer`]s.

use crate::budget::Budget;
use crate::ctx::ContextRing;
use crate::diag::{Code, Diag, Result};
use crate::frame::Frame;
use crate::heap::Arena;
use crate::image::Image;
use crate::isa::{self, Decoded, Op, Ty};
use crate::trap::{Answer, Step, Trap};
use crate::value::{EffectId, Handle, Tag, Value};

/// Runtime limits. Defaults are generous; the supervisor tightens them.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub max_steps: u64,
    pub max_frames: usize,
    pub max_arena: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_steps: 1_000_000,
            max_frames: 256,
            max_arena: 64 << 20,
        }
    }
}

/// The machine.
pub struct Vm {
    image: Image,
    frames: Vec<Frame>,
    arena: Arena,
    budget: Budget,
    ctx: ContextRing,
    steps: u64,
    limits: Limits,
    next_effect: u64,
    fork_depth: u32,
    staged_arena: Option<Arena>,
    pending: Option<Trap>,
    halt: Option<i64>,
}

impl Vm {
    #[must_use]
    pub fn new(image: &Image, limits: Limits) -> Self {
        let entry = image
            .funcs()
            .iter()
            .position(|f| f.name == "main")
            .unwrap_or(0);
        let f = &image.funcs()[entry];
        let frame = Frame::new(entry as u32, Vec::new(), 0, f.maxstack, f.returns);
        Self {
            image: image.clone(),
            frames: vec![frame],
            arena: Arena::new(),
            budget: Budget::default(),
            ctx: ContextRing::new(64),
            steps: 0,
            limits,
            next_effect: 1,
            fork_depth: 0,
            staged_arena: None,
            pending: None,
            halt: None,
        }
    }

    /// Run until the machine halts or traps. `answer` is the host's reply to
    /// the previous trap, when continuing after one.
    pub fn step(&mut self, answer: Option<Answer>) -> Result<Step> {
        if let Some(ans) = answer {
            match ans {
                Answer::Value(v) | Answer::Fail(v) => {
                    self.frames.last_mut().expect("frame").push(v)?
                }
                Answer::Shutdown => return Ok(Step::Halted(1)),
            }
        }
        loop {
            if let Some(t) = self.pending.take() {
                return Ok(Step::Trap(t));
            }
            if let Some(code) = self.halt.take() {
                return Ok(Step::Halted(code));
            }
            if self.frames.is_empty() {
                return Ok(Step::Halted(0));
            }
            self.steps += 1;
            if self.steps > self.limits.max_steps {
                return Err(Diag::new(Code::BudgetExceeded, "step limit exceeded"));
            }
            let fi = self.frames.len() - 1;
            let f = &self.image.funcs()[self.frames[fi].func as usize];
            let off = f.code_off as usize + (self.frames[fi].pc as usize) * isa::INSN_LEN;
            let d = isa::decode(self.image.code(), off)
                .ok_or_else(|| Diag::new(Code::UnassignedOpcode, "pc ran past the code section"))?;
            self.exec(d)?;
        }
    }

    fn exec(&mut self, d: Decoded) -> Result<()> {
        let fi = self.frames.len() - 1;
        let op = d.op;
        let mut advance = true;
        {
            let frame = &mut self.frames[fi];
            match op {
                Op::Nop => {}
                Op::Ldc => {
                    let bytes = self
                        .image
                        .constants()
                        .get(d.b as usize)
                        .cloned()
                        .unwrap_or_default();
                    let h = self.arena.alloc(&bytes)?;
                    let v = if std::str::from_utf8(&bytes).is_ok() {
                        Value::str(h)
                    } else {
                        Value::bytes(h)
                    };
                    frame.push(v)?;
                }
                Op::Ldi => frame.push(Value::int(isa::imm(d)))?,
                Op::Dup => {
                    let v = frame.peek(0)?;
                    frame.push(v)?;
                }
                Op::DupN => {
                    let v = frame.peek(d.b as usize)?;
                    frame.push(v)?;
                }
                Op::Drop => {
                    frame.pop()?;
                }
                Op::Swap => {
                    let a = frame.pop()?;
                    let b = frame.pop()?;
                    frame.push(a)?;
                    frame.push(b)?;
                }
                Op::Rot => {
                    let a = frame.pop()?;
                    let b = frame.pop()?;
