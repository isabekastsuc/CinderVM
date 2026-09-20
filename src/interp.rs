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
                    let c = frame.pop()?;
                    frame.push(a)?;
                    frame.push(c)?;
                    frame.push(b)?;
                }
                Op::Ldl => {
                    let v = frame.load_local(d.b as usize)?;
                    frame.push(v)?;
                }
                Op::Stl => {
                    let v = frame.pop()?;
                    frame.store_local(d.b as usize, v)?;
                }
                Op::Argv => {
                    let h = self.arena.alloc(&[0u8])?;
                    frame.push(Value::str(h))?;
                }
                Op::Pack => {
                    let items = frame.pop_n(d.b as usize)?;
                    let h = self.arena.alloc_list(&items)?;
                    frame.push(Value::list(h))?;
                }
                Op::Unpack => {
                    let list = frame.pop()?;
                    let h = list.as_handle_of(Tag::List)?;
                    for v in self.arena.get_list(h)? {
                        frame.push(v)?;
                    }
                }
                Op::Idx => {
                    let list = frame.pop()?;
                    let idx = frame.pop()?.as_int()?;
                    let h = list.as_handle_of(Tag::List)?;
                    let items = self.arena.get_list(h)?;
                    if idx < 0 || idx as usize >= items.len() {
                        return Err(Diag::new(Code::IndexRange, "index out of range"));
                    }
                    frame.push(items[idx as usize])?;
                }
                Op::Len => {
                    let v = frame.pop()?;
                    let h = v.as_handle()?;
                    let len = self.arena.get(h)?.len() as i64;
                    frame.push(Value::int(len))?;
                }
                Op::Cat => {
                    let b = frame.pop()?;
                    let a = frame.pop()?;
                    let ha = a.as_handle_of(Tag::Str)?;
                    let hb = b.as_handle_of(Tag::Str)?;
                    let mut buf = self.arena.get(ha)?.to_vec();
                    buf.extend_from_slice(self.arena.get(hb)?);
                    let h = self.arena.alloc(&buf)?;
                    frame.push(Value::str(h))?;
                }
                Op::Fmt => {
                    let list = frame.pop()?;
                    let h = list.as_handle_of(Tag::List)?;
                    let items = self.arena.get_list(h)?;
                    let template = String::from_utf8_lossy(
                        self.image
                            .constants()
                            .get(d.b as usize)
                            .map(Vec::as_slice)
                            .unwrap_or_default(),
                    );
                    let mut out = String::new();
                    let mut next = 0usize;
                    let mut last = false;
                    for part in template.split("{}") {
                        out.push_str(part);
                        if next < items.len() && !last {
                            out.push_str(&format_short(items[next], &self.arena));
                            next += 1;
                        }
                        last = next >= items.len();
                    }
                    let h = self.arena.alloc(out.as_bytes())?;
                    frame.push(Value::str(h))?;
                }
                Op::Slice => {
                    let v = frame.pop()?;
                    let hi = frame.pop()?.as_int()?;
                    let lo = frame.pop()?.as_int()?;
                    let h = v.as_handle()?;
                    let raw = self.arena.get(h)?.to_vec();
                    let lo = lo.clamp(0, raw.len() as i64) as usize;
                    let hi = hi.clamp(0, raw.len() as i64) as usize;
                    let h = self.arena.alloc(&raw[lo.min(hi)..hi.max(lo)])?;
                    frame.push(Value::bytes(h))?;
                }
                Op::Find => {
                    let needle = frame.pop()?;
                    let hay = frame.pop()?;
                    let hay = self.arena.get(hay.as_handle()?)?.to_vec();
                    let needle = self.arena.get(needle.as_handle()?)?.to_vec();
                    let pos = if needle.is_empty() {
            Some(0)
        } else {
            hay.windows(needle.len()).position(|w| w == needle)
        };
                    frame.push(Value::int(pos.map_or(-1, |p| p as i64)))?;
                }
                Op::Add => binop(&mut self.frames[fi], |a, b| a.wrapping_add(b))?,
                Op::Sub => binop(&mut self.frames[fi], |a, b| a.wrapping_sub(b))?,
                Op::Mul => binop(&mut self.frames[fi], |a, b| a.wrapping_mul(b))?,
                Op::Div => {
                    let b = frame.pop()?.as_int()?;
                    let a = frame.pop()?.as_int()?;
                    if b == 0 {
                        return Err(Diag::new(Code::DivideByZero, "divide by zero"));
                    }
                    frame.push(Value::int(a.wrapping_div(b)))?;
                }
                Op::Mod => {
                    let b = frame.pop()?.as_int()?;
                    let a = frame.pop()?.as_int()?;
                    if b == 0 {
                        return Err(Diag::new(Code::DivideByZero, "divide by zero"));
                    }
                    frame.push(Value::int(a.wrapping_rem(b)))?;
                }
                Op::Eq => {
                    let b = frame.pop()?;
                    let a = frame.pop()?;
                    let eq = if a.ty() == b.ty() && a.ty() != Ty::Int {
                        let ha = a.as_handle()?;
                        let hb = b.as_handle()?;
                        self.arena.get(ha)? == self.arena.get(hb)?
                    } else {
                        a.shallow_eq(b)
                    };
                    frame.push(Value::bool(eq))?;
                }
                Op::Lt => {
                    let b = frame.pop()?.as_int()?;
                    let a = frame.pop()?.as_int()?;
                    frame.push(Value::bool(a < b))?;
                }
                Op::Not => {
                    let a = frame.pop()?.as_int()?;
                    frame.push(Value::bool(a == 0))?;
                }
                Op::And => binop(&mut self.frames[fi], |a, b| a & b)?,
                Op::Or => binop(&mut self.frames[fi], |a, b| a | b)?,
                Op::Shl => {
                    let b = frame.pop()?.as_int()?;
                    let a = frame.pop()?.as_int()?;
                    frame.push(Value::int(a.wrapping_shl((b & 63) as u32)))?;
                }
                Op::Shr => {
                    let b = frame.pop()?.as_int()?;
                    let a = frame.pop()?.as_int()?;
                    frame.push(Value::int(a.wrapping_shr((b & 63) as u32)))?;
                }
                Op::Br => {
                    self.frames[fi].pc = d.b;
                    advance = false;
                }
                Op::Brz | Op::Brnz => {
                    let v = frame.pop()?.is_truthy();
                    let take = if op == Op::Brz { !v } else { v };
                    if take {
                        self.frames[fi].pc = d.b;
                        advance = false;
                    }
                }
                Op::Call => {
                    let target =
                        self.image
                            .funcs()
                            .get(d.b as usize)
                            .cloned()
                            .ok_or_else(|| {
                                Diag::new(Code::DanglingIndex, "call target out of range")
                            })?;
                    let args = frame.pop_n(target.args as usize)?;
                    if self.frames.len() >= self.limits.max_frames {
                        return Err(Diag::new(Code::BudgetExceeded, "frame limit exceeded"));
                    }
                    self.frames
                        .push(Frame::new(d.b, args, 0, target.maxstack, target.returns));
                    advance = false;
                }
                Op::Ret => {
                    let returns = frame.returns;
                    let values = frame.pop_n(returns as usize)?;
                    self.frames.pop();
                    if self.frames.is_empty() {
                        self.halt =
                            Some(values.first().map(|v| v.as_int().unwrap_or(0)).unwrap_or(0));
                    } else {
                        let caller = self.frames.len() - 1;
                        for v in values {
                            self.frames[caller].push(v)?;
                        }
                    }
                    advance = false;
                }
                Op::Tail => {
                    let target =
                        self.image
                            .funcs()
                            .get(d.b as usize)
                            .cloned()
                            .ok_or_else(|| {
                                Diag::new(Code::DanglingIndex, "tail target out of range")
                            })?;
                    let args = frame.pop_n(target.args as usize)?;
                    self.frames[fi] = Frame::new(d.b, args, 0, target.maxstack, target.returns);
                    advance = false;
                }
                Op::Switch => {
                    let v = frame.pop()?.as_int()?;
                    let table = self
                        .image
                        .jump_tables()
                        .get(d.b as usize)
                        .cloned()
                        .unwrap_or_default();
                    let target = if v >= 0 && (v as usize) < table.len().saturating_sub(1) {
                        table[v as usize]
                    } else {
                        table.last().copied().unwrap_or(0)
                    };
                    self.frames[fi].pc = target;
                    advance = false;
                }
                Op::Trap => {
                    let msg = self
                        .image
                        .constants()
                        .get(d.b as usize)
                        .cloned()
                        .unwrap_or_default();
                    return Err(Diag::new(
                        Code::Trapped,
                        String::from_utf8_lossy(&msg).into_owned(),
                    ));
                }
                Op::Halt => {
                    let code = frame.pop().map(|v| v.as_int().unwrap_or(0)).unwrap_or(0);
                    self.frames.clear();
                    self.halt = Some(code);
                    advance = false;
                }
                Op::CallTool | Op::Spawn => {
                    let list = frame.pop()?;
                    let h = list.as_handle_of(Tag::List)?;
                    let args = self.arena.get_list(h)?;
                    let id = EffectId(self.next_effect);
                    self.next_effect += 1;
                    self.pending = Some(Trap::Effect {
                        id,
                        tool: d.b,
                        args,
                    });
                    frame.push(Value::pending(id))?;
                    advance = false;
                }
                Op::Await | Op::Poll | Op::Join | Op::Select | Op::Cancel => {
                    self.pending = Some(Trap::Yield);
                    advance = false;
                }
                Op::Fork => {
                    if self.fork_depth > 0 {
                        return Err(Diag::new(Code::ForkImbalance, "nested forks unsupported"));
                    }
                    self.fork_depth = 1;
                    self.staged_arena = Some(self.arena.clone());
                    frame.push(Value::int(0))?;
                }
                Op::Commit => {
                    if let Some(staged) = self.staged_arena.take() {
                        self.arena = staged;
                    }
                    self.fork_depth = 0;
                }
                Op::Abort => {
                    self.staged_arena = None;
                    self.fork_depth = 0;
                }
                Op::Checkpoint => {
                    self.pending = Some(Trap::Yield);
                    advance = false;
                }
                Op::YieldCtx => {
                    self.pending = Some(Trap::Yield);
                    advance = false;
                }
                Op::Resume => {}
                Op::Reserve => {
                    self.budget.reserve(d.a, d.b as i64)?;
                }
                Op::Release => {
                    let amount = frame.pop()?.as_int()?;
                    self.budget.release(d.a, amount)?;
                }
                Op::Spend => {
                    let amount = frame.pop()?.as_int()?;
                    self.budget.spend(d.a, amount)?;
                }
                Op::QueryQ => {
                    let remaining = self.budget.remaining(d.a);
                    frame.push(Value::int(remaining))?;
                    self.pending = Some(Trap::Oracle(Op::QueryQ));
                    advance = false;
                }
                Op::CtxPush => {
                    let s = frame.pop()?.as_handle_of(Tag::Str)?;
                    let bytes = self.arena.get(s)?.to_vec();
                    self.ctx.push(d.a, &bytes)?;
                }
                Op::CtxPop => {
                    self.ctx.pop_oldest(d.b as usize)?;
                }
                Op::CtxWin => {
                    let h = self.arena.alloc_list(&self.ctx.window())?;
                    frame.push(Value::list(h))?;
                }
                Op::CtxCost => {
                    frame.push(Value::int(self.ctx.cost()))?;
                }
                Op::Now | Op::Rand | Op::Env | Op::Log => {
                    self.pending = Some(Trap::Oracle(op));
                    advance = false;
                }
            }
        }
        if advance && !self.frames.is_empty() {
            self.frames[fi].pc += 1;
        }
        Ok(())
    }

    /// The trap the machine wants serviced, if any.
    #[must_use]
    pub fn pending_trap(&self) -> Option<Trap> {
        self.pending.clone()
    }

    /// The exit code after `Step::Halted`.
    #[must_use]
    pub fn halted(&self) -> Option<i64> {
        self.halt
    }

    #[must_use]
    pub fn image(&self) -> &Image {
        &self.image
    }

    #[must_use]
    pub fn budget(&self) -> &Budget {
        &self.budget
    }

    #[must_use]
    pub fn arena_len(&self) -> usize {
        self.arena.len()
    }

    #[must_use]
    pub fn frame_depth(&self) -> usize {
        self.frames.len()
    }

    pub(crate) fn arena_bytes(&self) -> &[u8] {
        self.arena.as_bytes()
    }

    pub(crate) fn restore_arena(&mut self, bytes: &[u8]) {
        self.arena = Arena::from_bytes(bytes);
    }

    pub(crate) fn frames_snapshot(&self) -> Vec<Frame> {
        self.frames.clone()
    }

    pub(crate) fn restore_frames(&mut self, frames: Vec<Frame>) {
        self.frames = frames;
    }

    pub(crate) fn budget_snapshot(&self) -> Budget {
        self.budget.clone()
    }

    pub(crate) fn restore_budget(&mut self, b: Budget) {
        self.budget = b;
    }

    pub(crate) fn ctx_snapshot(&self) -> ContextRing {
        self.ctx.clone()
    }

    pub(crate) fn restore_ctx(&mut self, c: ContextRing) {
        self.ctx = c;
    }
}

fn format_short(v: Value, arena: &Arena) -> String {
    match v.ty() {
        Ty::Int => v.as_int().unwrap_or(0).to_string(),
        Ty::Str => arena
            .get_str(v.as_handle().unwrap_or(Handle::EMPTY))
            .unwrap_or("")
            .to_string(),
        _ => format!("{v:?}"),
    }
}

fn binop(frame: &mut Frame, f: impl Fn(i64, i64) -> i64) -> Result<()> {
    let b = frame.pop()?.as_int()?;
    let a = frame.pop()?.as_int()?;
    frame.push(Value::int(f(a, b)))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn find_with_empty_needle_returns_zero() {
        let src = ".const e \"\"\n.fn main() -> i32\n.maxstack 4\nmain:\n    ldc e\n    ldc e\n    find\n    halt\n";
        assert_eq!(run(src).unwrap(), 0);
    }

    use super::*;
    use crate::asm::assemble;
    use crate::verify::admit;

    fn run(src: &str) -> Result<i64> {
        let img = admit(assemble("t.cdx", src)?)?;
        let mut vm = Vm::new(&img, Limits::default());
        loop {
            match vm.step(None)? {
                Step::Halted(code) => return Ok(code),
                Step::Trap(_) => {
                    vm.step(Some(Answer::Value(Value::int(0))))?;
                }
            }
        }
    }

    #[test]
    fn arithmetic_program_halts_with_sum() {
        let src =
            ".fn main() -> i32\n.maxstack 4\nmain:\n    ldi 2\n    ldi 3\n    add\n    halt\n";
        assert_eq!(run(src).unwrap(), 5);
    }

    #[test]
    fn divide_by_zero_traps() {
        let src =
            ".fn main() -> i32\n.maxstack 3\nmain:\n    ldi 1\n    ldi 0\n    div\n    halt\n";
        assert_eq!(run(src).unwrap_err().code, Code::DivideByZero);
    }

    #[test]
    fn strings_and_length() {
        let src = ".const greeting \"hello\"\n.fn main() -> i32\n.maxstack 2\nmain:\n    ldc greeting\n    len\n    halt\n";
        assert_eq!(run(src).unwrap(), 5);
    }
}
