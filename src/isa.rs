//! The `cdx/4` instruction set: encoding, operand shapes, stack effects, and
//! type rules.
//!
//! This module is the single definition of the ISA. The assembler's mnemonic
//! table, the disassembler, the verifier's transfer functions, and
//! `docs/isa.md` are all derived from [`TABLE`] rather than restating it, so
//! adding an instruction is a one-line change plus a semantics arm in
//! `interp.rs` — and `isa::table_is_dense` fails the build if the two drift.
//!
//! # Encoding
//!
//! Fixed four bytes, little-endian:
//!
//! ```text
//!  byte 0     byte 1     bytes 2..4
//! ┌─────────┬──────────┬────────────────┐
//! │ opcode  │ operand  │ operand b      │
//! │   u8    │   a: u8  │      u16       │
//! └─────────┴──────────┴────────────────┘
//! ```
//!
//! Opcode `0xFF` is the wide prefix: it is followed by a full instruction whose
//! `b` field is extended by the prefix's own `a`/`b` operands to 32 bits. This
//! keeps the common case at four bytes while allowing constant pools larger
//! than 65 536 entries, which real prompt-heavy images hit.

use core::fmt;

/// Width of a non-prefixed instruction, in bytes.
pub const INSN_LEN: usize = 4;

/// Opcode reserved as the wide-operand prefix.
pub const WIDE_PREFIX: u8 = 0xFF;

/// ISA version encoded in the image header. Bumped only when the meaning of an
/// existing opcode changes; adding opcodes bumps the crate's minor version.
pub const ISA_VERSION: u16 = 4;

/// Static type lattice element, as seen by the verifier and asserted by the
/// assembler. `Bottom` is unreachable-code; `Top` is a join failure and is
/// illegal at every use site.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum Ty {
    Bottom,
    Int,
    Str,
    Bytes,
    List,
    Handle,
    /// Result of an effect instruction, consumable only by `AWAIT`, `POLL`,
    /// `CANCEL`, or `SELECT`. See `verify::check_pending_discipline`.
    Pending,
    Top,
}

impl Ty {
    /// Least upper bound. Distinct concrete types join to [`Ty::Top`], which the
    /// verifier reports at the merge point rather than at the eventual use —
    /// the merge is where the programmer's mistake actually is.
    #[must_use]
    pub fn join(self, other: Self) -> Self {
        match (self, other) {
            (Self::Bottom, t) | (t, Self::Bottom) => t,
            (a, b) if a == b => a,
            _ => Self::Top,
        }
    }

    /// Whether a value of type `self` is acceptable where `want` is required.
    /// `Handle` is accepted for `Str`/`Bytes`/`List` because those are all
    /// arena-resident and the tag check happens on deref; see `heap::classify`.
    #[must_use]
    pub fn satisfies(self, want: Self) -> bool {
        match (self, want) {
            (_, Self::Top) => true,
            (Self::Bottom, _) => true,
            (Self::Handle, Self::Str | Self::Bytes | Self::List) => true,
            (a, b) => a == b,
        }
    }

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Bottom => "⊥",
            Self::Int => "i64",
            Self::Str => "str",
            Self::Bytes => "bytes",
            Self::List => "list",
            Self::Handle => "handle",
            Self::Pending => "pending",
            Self::Top => "⊤",
        }
    }
}

impl fmt::Debug for Ty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// What the `b` operand of an instruction refers to. The assembler uses this to
/// decide how to resolve a symbol, and the verifier to decide what to
/// range-check `b` against.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Operand {
    /// No operand; `b` must be zero. Enforced, because a nonzero `b` here means
    /// the producer had a bug and we would rather fail loudly at load.
    None,
    /// Index into the constant pool.
    Const,
    /// Signed immediate, sign-extended from 16 (or 32, when wide) bits.
    Imm,
    /// Instruction index within the current function. Not a byte offset — the
    /// verifier needs indices for its CFG and byte offsets buy nothing at fixed
    /// width.
    Target,
    /// Index into the function table.
    Func,
    /// Index into the tool table.
    Tool,
    /// Index into the jump table section, used only by `SWITCH`.
    JumpTable,
    /// Operand-stack slot, relative to the current frame's base.
    Slot,
    /// Budget dimension: 0=tokens, 1=wall-ms, 2=tool-calls, 3=arena-bytes.
    Dimension,
}

/// Effect classification. Drives the verifier's snapshot-safety rule and the
/// supervisor's scheduling decisions: only `Suspend` instructions can yield a
/// VM, so only they need a serializable resume point.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Effect {
    /// Pure with respect to the host: state in, state out.
    Pure,
    /// Allocates in the arena; charged against the arena dimension.
    Alloc,
    /// Traps to the host and may not resume in this scheduling quantum.
    Suspend,
    /// Traps to the host but resumes immediately with a journalled answer
    /// (`NOW`, `RAND`, `QUERYQ`). Non-deterministic sources live here so that
    /// every one of them is on the record.
    Oracle,
    /// Alters the frame stack or program counter.
    Control,
    /// Produces or consumes a snapshot boundary.
    Durable,
}

/// One row of the instruction table.
///
/// `pops`/`pushes` are the *static* stack effect. Instructions whose effect
/// depends on an operand (`PACK`, `UNPACK`, `CALL`, `DUPN`) set the
/// [`Insn::variadic`] flag and are handled explicitly in
/// `verify::transfer`; every other instruction is checked generically from
/// these fields, which is why the verifier is short.
#[derive(Clone, Copy, Debug)]
pub struct Insn {
    pub op: Op,
    pub mnemonic: &'static str,
    pub operand: Operand,
    pub pops: &'static [Ty],
    pub pushes: &'static [Ty],
    pub effect: Effect,
    pub variadic: bool,
    /// Whether control can fall through to the next instruction. `false` for
    /// `BR`, `RET`, `TAIL`, `SWITCH`, `TRAP`, `HALT` — the CFG builder relies on
    /// this to terminate basic blocks.
    pub falls_through: bool,
    pub doc: &'static str,
}

macro_rules! ops {
    (
        $( $variant:ident = $code:literal, $mnem:literal, $operand:ident,
           [$($pop:ident),*] -> [$($push:ident),*],
           $effect:ident $(, $flag:ident)* ; $doc:literal );* $(;)?
    ) => {
        /// Opcode. The discriminant *is* the encoded byte.
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
        #[repr(u8)]
        pub enum Op { $( $variant = $code ),* }

        /// The instruction table, indexed by opcode. Dense over `0..=LAST`;
        /// see [`table_is_dense`].
        pub static TABLE: &[Insn] = &[
            $( Insn {
                op: Op::$variant,
                mnemonic: $mnem,
                operand: Operand::$operand,
                pops: &[$(Ty::$pop),*],
                pushes: &[$(Ty::$push),*],
                effect: Effect::$effect,
                variadic: has_flag(&[$(stringify!($flag)),*], "variadic"),
                falls_through: !has_flag(&[$(stringify!($flag)),*], "terminal"),
                doc: $doc,
            } ),*
        ];

        impl Op {
            /// Decode a byte into an opcode, or `None` if unassigned. This is
            /// the only place a `u8` becomes an `Op`; `image::seal` rejects
            /// unassigned bytes so the interpreter never sees one.
            #[must_use]
            pub fn from_byte(b: u8) -> Option<Self> {
                match b { $( $code => Some(Self::$variant), )* _ => None }
            }
        }
    };
}

/// `const`-evaluable string equality, so the table's flag lists can be checked
/// at compile time inside a `static` initializer.
const fn str_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

const fn has_flag(flags: &[&str], want: &str) -> bool {
    let mut i = 0;
    while i < flags.len() {
        if str_eq(flags[i], want) {
            return true;
        }
        i += 1;
    }
    false
}

ops! {
    // ── stack ────────────────────────────────────────────────────────────
    Nop      = 0x00, "nop",     None,      [] -> [],            Pure;     "No operation. Emitted as branch padding by `asm::layout`.";
    Ldc      = 0x01, "ldc",     Const,     [] -> [Handle],      Pure;     "Push constant-pool entry `b`. Interned; identical constants share a handle.";
    Ldi      = 0x02, "ldi",     Imm,       [] -> [Int],         Pure;     "Push sign-extended immediate `b`.";
    Dup      = 0x03, "dup",     None,      [Top] -> [Top, Top], Pure;     "Duplicate the top operand.";
    DupN     = 0x04, "dupn",    Slot,      [] -> [Top],         Pure,     variadic; "Duplicate the operand `b` slots below the top.";
    Drop     = 0x05, "drop",    None,      [Top] -> [],         Pure;     "Discard the top operand.";
    Swap     = 0x06, "swap",    None,      [Top, Top] -> [Top, Top], Pure; "Exchange the top two operands.";
    Rot      = 0x07, "rot",     None,      [Top, Top, Top] -> [Top, Top, Top], Pure; "Rotate the top three operands left.";
    Ldl      = 0x08, "ldl",     Slot,      [] -> [Top],         Pure,     variadic; "Load frame-local slot `b`.";
    Stl      = 0x09, "stl",     Slot,      [Top] -> [],         Pure,     variadic; "Store to frame-local slot `b`.";
    Argv     = 0x0A, "argv",    Slot,      [] -> [Str],         Pure;     "Push host-supplied argument `b`. Recorded in the image's arg arity.";

    // ── data ─────────────────────────────────────────────────────────────
    Pack     = 0x10, "pack",    Imm,       [] -> [List],        Alloc,    variadic; "Pop `b` operands into a list, first-pushed first.";
    Unpack   = 0x11, "unpack",  Imm,       [List] -> [],        Alloc,    variadic; "Push a list's `b` elements. Length mismatch traps `E_ARITY`.";
    Idx      = 0x12, "idx",     None,      [Int, List] -> [Top], Pure;    "Index a list. Out-of-range traps `E_RANGE`.";
    Len      = 0x13, "len",     None,      [Handle] -> [Int],   Pure;     "Length in elements for lists, bytes for str/bytes.";
    Cat      = 0x14, "cat",     None,      [Handle, Handle] -> [Handle], Alloc; "Concatenate two same-tag arena values.";
    Fmt      = 0x15, "fmt",     Const,     [List] -> [Str],     Alloc;    "Format a list into template `b`. Positional `{}` only — no locale, no float.";
    Slice    = 0x16, "slice",   None,      [Int, Int, Handle] -> [Handle], Alloc; "Half-open slice. Clamps rather than traps; matches `str` conventions.";
    Find     = 0x17, "find",    None,      [Handle, Handle] -> [Int], Pure; "Index of needle in haystack, or -1.";

    // ── arithmetic ───────────────────────────────────────────────────────
    Add      = 0x20, "add",     None,      [Int, Int] -> [Int], Pure;     "Wrapping i64 addition. Wrapping, not saturating, so replay is exact.";
    Sub      = 0x21, "sub",     None,      [Int, Int] -> [Int], Pure;     "Wrapping i64 subtraction.";
    Mul      = 0x22, "mul",     None,      [Int, Int] -> [Int], Pure;     "Wrapping i64 multiplication.";
    Div      = 0x23, "div",     None,      [Int, Int] -> [Int], Pure;     "Truncating division. Divide-by-zero traps `E_DIV0`.";
    Mod      = 0x24, "mod",     None,      [Int, Int] -> [Int], Pure;     "Remainder with the sign of the dividend.";
    Eq       = 0x25, "eq",      None,      [Top, Top] -> [Int], Pure;     "Structural equality; 1 or 0. Cross-tag comparison is 0, not an error.";
    Lt       = 0x26, "lt",      None,      [Int, Int] -> [Int], Pure;     "Signed less-than.";
    Not      = 0x27, "not",     None,      [Int] -> [Int],      Pure;     "Logical negation: 0 becomes 1, everything else 0.";
    And      = 0x28, "and",     None,      [Int, Int] -> [Int], Pure;     "Bitwise and.";
    Or       = 0x29, "or",      None,      [Int, Int] -> [Int], Pure;     "Bitwise or.";
    Shl      = 0x2A, "shl",     None,      [Int, Int] -> [Int], Pure;     "Left shift, masked to 0..64.";
    Shr      = 0x2B, "shr",     None,      [Int, Int] -> [Int], Pure;     "Arithmetic right shift, masked to 0..64.";

    // ── control ──────────────────────────────────────────────────────────
    Br       = 0x30, "br",      Target,    [] -> [],            Control,  terminal; "Unconditional branch. A back-edge requires a dominating metering insn.";
    Brz      = 0x31, "brz",     Target,    [Int] -> [],         Control;  "Branch if top is zero.";
    Brnz     = 0x32, "brnz",    Target,    [Int] -> [],         Control;  "Branch if top is nonzero.";
    Call     = 0x33, "call",    Func,      [] -> [],            Control,  variadic; "Call function `b`; arity and returns come from the function table.";
    Ret      = 0x34, "ret",     None,      [] -> [],            Control,  variadic, terminal; "Return. Verified against the function's declared return arity.";
    Tail     = 0x35, "tail",    Func,      [] -> [],            Control,  variadic, terminal; "Tail call: replaces the frame. Keeps recursive agents O(1) in frames.";
    Switch   = 0x36, "switch",  JumpTable, [Int] -> [],         Control,  terminal; "Dense jump table `b`, with a mandatory default arm.";
    Trap     = 0x37, "trap",    Const,     [] -> [],            Control,  terminal; "Abort the VM with diagnostic `b`. The assembler's `assert` lowers to this.";
    Halt     = 0x38, "halt",    None,      [Int] -> [],         Control,  terminal; "Stop with the top of stack as exit status.";

    // ── effects ──────────────────────────────────────────────────────────
    CallTool = 0x40, "calltool", Tool,     [List] -> [Pending], Suspend;  "Issue tool `b` with a packed argument list. Journalled before dispatch.";
    Await    = 0x41, "await",   None,      [Pending] -> [Top],  Suspend;  "Block this VM until the effect resolves. Descheduled, not spinning.";
    Poll     = 0x42, "poll",    None,      [Pending] -> [Pending, Int], Suspend; "Non-blocking check; pushes readiness without consuming the pending.";
    Cancel   = 0x43, "cancel",  None,      [Pending] -> [],     Suspend;  "Abandon an effect. Best-effort upstream; always resolves the pending.";
    Select   = 0x44, "select",  Imm,       [] -> [Int, Top],    Suspend,  variadic; "Await the first of `b` pendings. Winner index is journalled — ties are not racy on replay.";

    // ── concurrency ──────────────────────────────────────────────────────
    Spawn    = 0x50, "spawn",   Func,      [List] -> [Pending], Suspend;  "Start a child VM at function `b`. The child is a VM, not a thread.";
    Join     = 0x51, "join",    None,      [Pending] -> [Top],  Suspend;  "Await a child's result.";
    Fork     = 0x52, "fork",    None,      [] -> [Int],         Durable;  "Copy-on-write branch of this machine. Pushes 0 in the parent, 1 in the child.";
    Commit   = 0x53, "commit",  None,      [] -> [],            Durable;  "Accept the current fork's arena writes into the parent.";
    Abort    = 0x54, "abort",   None,      [] -> [],            Durable,  terminal; "Discard the current fork. Balanced with `fork` on every path.";

    // ── durability ───────────────────────────────────────────────────────
    Checkpoint = 0x60, "checkpoint", Const, [] -> [],           Durable;  "Snapshot to the object store under label `b`. Requires no live pendings.";
    YieldCtx = 0x61, "yieldctx", None,     [] -> [],            Durable;  "Voluntarily deschedule. The scheduler's cooperative preemption point.";
    Resume   = 0x62, "resume",  Const,     [] -> [],            Durable;  "Marker recorded on restore. Never executed; present for journal alignment.";

    // ── metering ─────────────────────────────────────────────────────────
    Reserve  = 0x70, "reserve", Dimension, [Int] -> [],         Oracle;   "Lease budget on dimension `b`. Refused at reserve time, never after the spend.";
    Release  = 0x71, "release", Dimension, [Int] -> [],         Pure;     "Return an unused reservation.";
    Spend    = 0x72, "spend",   Dimension, [Int] -> [],         Oracle;   "Settle a reservation with actual usage.";
    QueryQ   = 0x73, "queryq",  Dimension, [] -> [Int],         Oracle;   "Remaining allowance on dimension `b`. Journalled — it is host state.";

    // ── context ──────────────────────────────────────────────────────────
    CtxPush  = 0x80, "ctxpush", Imm,       [Str] -> [],         Alloc;    "Append a segment with role `b` to the context ring.";
    CtxPop   = 0x81, "ctxpop",  Imm,       [] -> [],            Pure;     "Drop the `b` oldest segments. Eviction is explicit, never implicit.";
    CtxWin   = 0x82, "ctxwin",  None,      [] -> [List],        Alloc;    "Materialize the current window as a list of segments.";
    CtxCost  = 0x83, "ctxcost", None,      [] -> [Int],         Pure;     "Token cost of the current window, from the image's tokenizer profile.";

    // ── oracles ──────────────────────────────────────────────────────────
    Now      = 0x90, "now",     None,      [] -> [Int],         Oracle;   "Milliseconds since the epoch, from the host. Journalled.";
    Rand     = 0x91, "rand",    None,      [] -> [Int],         Oracle;   "64 host-supplied random bits. Journalled.";
    Env      = 0x92, "env",     Const,     [] -> [Str],         Oracle;   "Host configuration value `b`. Journalled, and redacted in `cinder dis`.";
    Log      = 0x93, "log",     Const,     [Top] -> [],         Oracle;   "Structured log at level `b`. Journalled so replay reproduces output.";
}

/// Last assigned opcode. `TABLE` is dense over `0..=LAST` modulo the gaps that
/// `table_is_dense` whitelists as class boundaries.
pub const LAST: u8 = Op::Log as u8;

impl Op {
    /// Table row for this opcode. Infallible by construction: [`Op`] can only
    /// be produced by [`Op::from_byte`], which only returns assigned opcodes,
    /// and [`table_is_dense`] proves [`LOOKUP`] covers all of them.
    #[must_use]
    pub fn insn(self) -> &'static Insn {
        &TABLE[LOOKUP[self as usize] as usize]
    }

    #[must_use]
    pub fn mnemonic(self) -> &'static str {
        self.insn().mnemonic
    }

    #[must_use]
    pub fn effect(self) -> Effect {
        self.insn().effect
    }

    /// Whether this instruction ends a basic block.
    #[must_use]
    pub fn is_terminal(self) -> bool {
        !self.insn().falls_through
    }

    /// Whether this instruction can suspend the VM, and therefore needs the
    /// machine state to be snapshot-clean at its boundary.
    #[must_use]
    pub fn can_suspend(self) -> bool {
        matches!(self.effect(), Effect::Suspend | Effect::Durable)
    }
}

/// Opcode-indexed row table for the interpreter's hot path, resolved at compile
/// time. Stores an index into [`TABLE`] rather than a reference, because a
/// `static` initializer cannot take references into another `static`.
/// `u8::MAX` marks a class-boundary gap.
static LOOKUP: [u8; 256] = build_lookup();

const fn build_lookup() -> [u8; 256] {
    let mut out = [u8::MAX; 256];
    let mut i = 0;
    while i < TABLE.len() {
        out[TABLE[i].op as usize] = i as u8;
        i += 1;
    }
    out
}

/// A decoded instruction: opcode plus a widened operand pair.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Decoded {
    pub op: Op,
    pub a: u8,
    pub b: u32,
    /// Encoded length in bytes: 4, or 8 when wide-prefixed. The CFG builder and
    /// disassembler both need this to walk the code section.
    pub len: u8,
}

/// Decode at `off` in `code`.
///
/// Returns `None` for a truncated tail, an unassigned opcode, or a wide prefix
/// followed by another prefix. `image::seal` decodes the entire code section
/// eagerly, so a successfully loaded image is known to decode cleanly at every
/// instruction boundary — which is why `interp` can decode without checking.
#[must_use]
pub fn decode(code: &[u8], off: usize) -> Option<Decoded> {
    let raw = code.get(off..off + INSN_LEN)?;
    if raw[0] == WIDE_PREFIX {
        let ext = u32::from(u16::from_le_bytes([raw[2], raw[3]])) << 16;
        let inner = code.get(off + INSN_LEN..off + 2 * INSN_LEN)?;
        if inner[0] == WIDE_PREFIX {
            return None;
        }
        let op = Op::from_byte(inner[0])?;
        let lo = u32::from(u16::from_le_bytes([inner[2], inner[3]]));
        Some(Decoded {
            op,
            a: inner[1],
            b: ext | lo,
            len: 8,
        })
    } else {
        let op = Op::from_byte(raw[0])?;
        let b = u32::from(u16::from_le_bytes([raw[2], raw[3]]));
        Some(Decoded {
            op,
            a: raw[1],
            b,
            len: 4,
        })
    }
}

/// Encode into `out`, emitting a wide prefix when `b` exceeds 16 bits.
/// Returns bytes written.
pub fn encode(out: &mut Vec<u8>, op: Op, a: u8, b: u32) -> usize {
    if b > u32::from(u16::MAX) {
        let hi = (b >> 16) as u16;
        out.extend_from_slice(&[WIDE_PREFIX, 0]);
        out.extend_from_slice(&hi.to_le_bytes());
        out.extend_from_slice(&[op as u8, a]);
        out.extend_from_slice(&(b as u16).to_le_bytes());
        8
    } else {
        out.extend_from_slice(&[op as u8, a]);
        out.extend_from_slice(&(b as u16).to_le_bytes());
        4
    }
}

/// Sign-extend an operand `b` that [`Operand::Imm`] treats as signed. Width
/// depends on whether the instruction was wide-encoded.
#[must_use]
pub fn imm(d: Decoded) -> i64 {
    if d.len == 8 {
        i64::from(d.b as i32)
    } else {
        i64::from(d.b as u16 as i16)
    }
}

/// Emit `docs/isa.md`. Invoked by `cinderc --emit-isa-md`; `make verify` diffs
/// the result against the checked-in file so the reference cannot drift from the
/// implementation.
#[must_use]
pub fn emit_markdown() -> String {
    let mut s = String::with_capacity(16 * 1024);
    s.push_str("<!-- generated by `cinderc --emit-isa-md`; do not edit -->\n\n");
    s.push_str(&format!("# `cdx/{ISA_VERSION}` instruction reference\n\n"));
    s.push_str("| Op | Mnemonic | Operand | Stack | Effect | Notes |\n");
    s.push_str("|---:|---|---|---|---|---|\n");
    for i in TABLE {
        let pops: Vec<_> = i.pops.iter().map(|t| t.name()).collect();
        let pushes: Vec<_> = i.pushes.iter().map(|t| t.name()).collect();
        let stack = format!(
            "`[{}] → [{}]{}`",
            pops.join(", "),
            pushes.join(", "),
            if i.variadic { " *" } else { "" }
        );
        s.push_str(&format!(
            "| `0x{:02X}` | `{}` | {:?} | {} | {:?} | {} |\n",
            i.op as u8, i.mnemonic, i.operand, stack, i.effect, i.doc
        ));
    }
    s.push_str("\n`*` marks a variadic stack effect resolved from the operand; ");
    s.push_str("see `verify::transfer`.\n");
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Guards the invariant that makes `LOOKUP` and the disassembler correct:
    /// every table row is reachable from its opcode byte, and no two rows share
    /// one. Gaps are permitted only on class boundaries (multiples of 0x10).
    #[test]
    fn table_is_dense() {
        let mut seen = [false; 256];
        for i in TABLE {
            let b = i.op as u8;
            assert!(
                !seen[b as usize],
                "duplicate opcode 0x{b:02X} ({})",
                i.mnemonic
            );
            seen[b as usize] = true;
            assert_eq!(
                Op::from_byte(b),
                Some(i.op),
                "from_byte disagrees at 0x{b:02X}"
            );
            assert_ne!(LOOKUP[b as usize], u8::MAX, "LOOKUP missing 0x{b:02X}");
            assert_eq!(
                i.op.insn().mnemonic,
                i.mnemonic,
                "LOOKUP misroutes 0x{b:02X}"
            );
        }
        for b in 0..=LAST {
            if !seen[b as usize] {
                let cls = b / 16;
                let start = cls * 16;
                let end = (cls + 1) * 16;
                let max_seen = (start..end).filter(|&x| seen[x as usize]).max();
                match max_seen {
                    Some(m) => {
                        assert!(b > m, "gap at 0x{b:02X} below used byte 0x{m:02X} in class")
                    }
                    None => assert_eq!(b, start, "empty class must start on the boundary"),
                }
            }
        }
        assert!(
            Op::from_byte(WIDE_PREFIX).is_none(),
            "0xFF must stay reserved"
        );
    }

    #[test]
    fn mnemonics_are_unique_and_lowercase() {
        let mut names: Vec<_> = TABLE.iter().map(|i| i.mnemonic).collect();
        names.sort_unstable();
        let before = names.len();
        names.dedup();
        assert_eq!(before, names.len(), "duplicate mnemonic");
        assert!(TABLE
            .iter()
            .all(|i| i.mnemonic.chars().all(|c| c.is_ascii_lowercase())));
    }

    #[test]
    fn no_operand_implies_zero_b_roundtrip() {
        let mut buf = Vec::new();
        for i in TABLE.iter().filter(|i| i.operand == Operand::None) {
            buf.clear();
            encode(&mut buf, i.op, 0, 0);
            let d = decode(&buf, 0).expect("encoded insn decodes");
            assert_eq!(d.b, 0);
            assert_eq!(d.len, 4);
        }
    }

    #[test]
    fn wide_encoding_roundtrips() {
        let mut buf = Vec::new();
        let n = encode(&mut buf, Op::Ldc, 3, 0x0004_1234);
        assert_eq!(n, 8);
        let d = decode(&buf, 0).expect("wide insn decodes");
        assert_eq!((d.op, d.a, d.b, d.len), (Op::Ldc, 3, 0x0004_1234, 8));
    }

    #[test]
    fn double_wide_prefix_is_rejected() {
        let buf = [WIDE_PREFIX, 0, 0, 0, WIDE_PREFIX, 0, 0, 0];
        assert!(decode(&buf, 0).is_none());
    }

    #[test]
    fn imm_sign_extends_both_widths() {
        let mut buf = Vec::new();
        encode(&mut buf, Op::Ldi, 0, u32::from(u16::MAX)); // -1 narrow
        assert_eq!(imm(decode(&buf, 0).unwrap()), -1);
        buf.clear();
        encode(&mut buf, Op::Ldi, 0, u32::MAX); // -1 wide
        assert_eq!(imm(decode(&buf, 0).unwrap()), -1);
    }

    #[test]
    fn join_is_commutative_and_idempotent() {
        let all = [
            Ty::Bottom,
            Ty::Int,
            Ty::Str,
            Ty::Bytes,
            Ty::List,
            Ty::Handle,
            Ty::Pending,
            Ty::Top,
        ];
        for a in all {
            assert_eq!(a.join(a), a);
            for b in all {
                assert_eq!(a.join(b), b.join(a), "join not commutative on {a:?}/{b:?}");
            }
        }
        assert_eq!(Ty::Int.join(Ty::Str), Ty::Top);
        assert_eq!(Ty::Bottom.join(Ty::Pending), Ty::Pending);
    }

    /// The rule the snapshot safety argument rests on: exactly four
    /// instructions consume a `Pending`. If this count changes, `verify.rs`'s
    /// pending-discipline check needs a matching update.
    #[test]
    fn pending_consumers_are_exactly_the_effect_class() {
        let consumers: Vec<_> = TABLE
            .iter()
            .filter(|i| i.pops.contains(&Ty::Pending))
            .map(|i| i.mnemonic)
            .collect();
        assert_eq!(consumers, ["await", "poll", "cancel", "join"]);
    }

    #[test]
    fn terminals_do_not_fall_through() {
        for m in ["br", "ret", "tail", "switch", "trap", "halt", "abort"] {
            let i = TABLE
                .iter()
                .find(|i| i.mnemonic == m)
                .expect("mnemonic exists");
            assert!(!i.falls_through, "`{m}` must terminate its block");
        }
        let brz = TABLE.iter().find(|i| i.mnemonic == "brz").unwrap();
        assert!(
            brz.falls_through,
            "conditional branches have two successors"
        );
    }

    #[test]
    fn emitted_markdown_covers_every_instruction() {
        let md = emit_markdown();
        for i in TABLE {
            assert!(
                md.contains(&format!("`{}`", i.mnemonic)),
                "missing {}",
                i.mnemonic
            );
        }
    }
}
