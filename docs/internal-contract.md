# Internal implementation contract

Not user documentation. This is the frozen interface every module compiles
against, so that modules written independently link. If you need to change a
signature here, change it here first.

Foundation modules (already written, do not modify): `isa.rs`, `value.rs`,
`diag.rs`, `lib.rs`.

## Conventions

- `#![deny(unsafe_code)]` crate-wide. No `unsafe`, ever.
- Zero external dependencies in the library. No `serde`, no `anyhow`, no
  `blake3` crate — `hash.rs` provides hashing, `std` provides the rest.
- Every fallible function returns `crate::diag::Result<T>` = `Result<T, Diag>`.
- Errors use an existing `diag::Code`. Do not invent codes; if one is genuinely
  missing, note it rather than adding it.
- Tests live in a `#[cfg(test)] mod tests` at the bottom of each module. Test
  the invariant the module claims, not the syntax.
- Doc comment at the top of each module: what it guarantees, what it assumes the
  caller already proved.
- Rust 2021, `rustfmt` defaults, no `clippy::pedantic` warnings.

## Available from foundation

```rust
// isa.rs
pub const INSN_LEN: usize = 4;
pub const WIDE_PREFIX: u8 = 0xFF;
pub const ISA_VERSION: u16 = 4;
pub enum Ty { Bottom, Int, Str, Bytes, List, Handle, Pending, Top }
impl Ty { fn join(self, Self) -> Self; fn satisfies(self, want: Self) -> bool;
          const fn name(self) -> &'static str }
pub enum Operand { None, Const, Imm, Target, Func, Tool, JumpTable, Slot, Dimension }
pub enum Effect { Pure, Alloc, Suspend, Oracle, Control, Durable }
pub struct Insn { pub op: Op, pub mnemonic: &'static str, pub operand: Operand,
                  pub pops: &'static [Ty], pub pushes: &'static [Ty],
                  pub effect: Effect, pub variadic: bool,
                  pub falls_through: bool, pub doc: &'static str }
pub enum Op { Nop, Ldc, Ldi, Dup, DupN, Drop, Swap, Rot, Ldl, Stl, Argv,
              Pack, Unpack, Idx, Len, Cat, Fmt, Slice, Find,
              Add, Sub, Mul, Div, Mod, Eq, Lt, Not, And, Or, Shl, Shr,
              Br, Brz, Brnz, Call, Ret, Tail, Switch, Trap, Halt,
              CallTool, Await, Poll, Cancel, Select,
              Spawn, Join, Fork, Commit, Abort,
              Checkpoint, YieldCtx, Resume,
              Reserve, Release, Spend, QueryQ,
              CtxPush, CtxPop, CtxWin, CtxCost,
              Now, Rand, Env, Log }
pub static TABLE: &[Insn];
impl Op { fn from_byte(u8) -> Option<Self>; fn insn(self) -> &'static Insn;
          fn mnemonic(self) -> &'static str; fn effect(self) -> Effect;
          fn is_terminal(self) -> bool; fn can_suspend(self) -> bool }
pub struct Decoded { pub op: Op, pub a: u8, pub b: u32, pub len: u8 }
pub fn decode(code: &[u8], off: usize) -> Option<Decoded>;
pub fn encode(out: &mut Vec<u8>, op: Op, a: u8, b: u32) -> usize;
pub fn imm(d: Decoded) -> i64;
pub fn emit_markdown() -> String;

// value.rs
pub enum Tag { Void=0, Int=1, Str=2, Bytes=3, List=4, Pending=5 }
impl Tag { fn from_byte(u8) -> Option<Self>; const fn ty(self) -> Ty;
           const fn is_arena(self) -> bool; const fn name(self) -> &'static str }
pub struct Handle { pub off: u32, pub len: u32 }
impl Handle { const EMPTY: Self; const fn new(u32,u32) -> Self;
              const fn end(self) -> u32; const fn to_bits(self) -> u64;
              const fn from_bits(u64) -> Self }
pub struct EffectId(pub u64);
pub const SLOT_LEN: usize = 16;
pub struct Value; // Copy, 16 bytes
impl Value { const VOID: Self;
  const fn int(i64)->Self; const fn str(Handle)->Self; const fn bytes(Handle)->Self;
  const fn list(Handle)->Self; const fn pending(EffectId)->Self; const fn bool(bool)->Self;
  const fn tag(self)->Tag; const fn ty(self)->Ty; const fn is_void(self)->bool;
  fn is_truthy(self)->bool; fn as_int(self)->Result<i64>;
  fn as_handle(self)->Result<Handle>; fn as_handle_of(self,Tag)->Result<Handle>;
  fn as_pending(self)->Result<EffectId>; fn shallow_eq(self,Self)->bool;
  fn write(self, &mut Vec<u8>); fn read(&[u8])->Result<Self> }

// diag.rs
pub struct Span { pub lo: u32, pub hi: u32, pub line: u32, pub col: u32 }
impl Span { const fn new(u32,u32,u32,u32)->Self; const fn synthetic()->Self;
            const fn is_synthetic(self)->bool; fn merge(self,Self)->Self }
pub enum Phase { Lex, Assemble, Load, Verify, Run, Restore, Journal }
pub enum Code { /* see diag.rs; use as_str()/parse() */ }
impl Code { const fn as_str(self)->&'static str; const fn phase(self)->Phase;
            const fn blurb(self)->&'static str; fn parse(&str)->Option<Self>;
            const fn all()->&'static [Self] }
pub struct Diag { pub code: Code, pub message: String, pub span: Span,
                  pub labels: Vec<Label>, pub notes: Vec<String>, pub pc: Option<u32> }
impl Diag { fn new(Code, impl Into<String>)->Self; fn at(self,Span)->Self;
            fn at_pc(self,u32)->Self; fn label(self,Span,impl Into<String>)->Self;
            fn note(self,impl Into<String>)->Self;
            fn render(&self,name:&str,source:Option<&str>)->String }
pub type Result<T> = core::result::Result<T, Diag>;
pub fn err<T>(Code, impl Into<String>) -> Result<T>;
```

## Module contracts to implement

Signatures are binding. Add private helpers freely; do not change these.

### `hash.rs`
```rust
pub struct Hasher;
impl Hasher {
    pub fn new() -> Self;
    pub fn update(&mut self, bytes: &[u8]);
    pub fn finish(self) -> Digest;
}
pub struct Digest(pub [u8; 32]);
impl Digest {
    pub fn of(bytes: &[u8]) -> Self;
    pub fn short(&self) -> String;         // first 4 bytes as hex, e.g. "6f2ac1d3"
    pub fn to_hex(&self) -> String;
    pub fn from_hex(s: &str) -> Result<Self>;
}
```
A keyed sponge over a 64-bit ARX permutation is fine — this is integrity, not
cryptography. Document that clearly. Must be deterministic across platforms:
fixed endianness, no `usize`, no address-dependent behaviour.

### `lex.rs`
```rust
pub enum Tok<'a> {
    Ident(&'a str), Directive(&'a str), Label(&'a str),
    ConstRef(&'a str), ToolRef(&'a str),      // $name, %name
    Int(i64), Str(String), Newline, Eof,
    Comma, Colon, Arrow, LParen, RParen, Eq_,
}
pub struct Token<'a> { pub tok: Tok<'a>, pub span: Span }
pub struct Lexer<'a>;
impl<'a> Lexer<'a> {
    pub fn new(src: &'a str) -> Self;
    pub fn next_token(&mut self) -> Result<Token<'a>>;
    pub fn tokenize(src: &'a str) -> Result<Vec<Token<'a>>>;
}
```
`;` starts a comment to end of line. Newlines are significant (one insn per
line). Track line/col for spans. Escapes: `\n \t \r \\ \" \0 \xNN`.

### `image.rs`
```rust
pub struct FuncMeta { pub name: String, pub params: u8, pub returns: u8,
                      pub maxstack: u16, pub code_off: u32, pub code_len: u32 }
pub struct ToolMeta { pub name: String, pub returns: Ty }
pub struct Budgets { pub tokens: u64, pub wall_ms: u64, pub tools: u32, pub arena: u32 }
pub struct Object {          // assembler output: unverified
    pub name: String, pub isa: u16, pub code: Vec<u8>,
    pub consts: Vec<Vec<u8>>, pub const_tags: Vec<Tag>,
    pub funcs: Vec<FuncMeta>, pub tools: Vec<ToolMeta>,
    pub jump_tables: Vec<Vec<u32>>, pub budgets: Budgets,
    pub spans: Vec<Span>,    // parallel to instruction indices, may be empty
    pub source: Option<String>,
}
pub struct Image;            // verified; only verify::admit constructs it
impl Image {
    pub fn is_verified(&self) -> bool;   // always true; documents the invariant
    pub fn digest(&self) -> Digest;
    pub fn name(&self) -> &str;
    pub fn entry(&self) -> u32;                       // index of `main`
    pub fn func(&self, id: u32) -> &FuncMeta;
    pub fn funcs(&self) -> &[FuncMeta];
    pub fn tool(&self, id: u32) -> &ToolMeta;
    pub fn tools(&self) -> &[ToolMeta];
    pub fn code(&self) -> &[u8];
    pub fn const_bytes(&self, id: u32) -> &[u8];
    pub fn const_tag(&self, id: u32) -> Tag;
    pub fn consts(&self) -> usize;
    pub fn jump_table(&self, id: u32) -> &[u32];
    pub fn budgets(&self) -> &Budgets;
    pub fn span_at(&self, pc: u32) -> Span;
    pub fn source(&self) -> Option<&str>;
    pub fn encode(&self) -> Vec<u8>;                  // .cdxb bytes
    pub fn decode(bytes: &[u8]) -> Result<Object>;    // container -> unverified
}
pub(crate) fn seal(obj: Object) -> Result<Image>;     // verify.rs calls this last
```
Container: magic `CDXB`, u16 format version, u16 isa, u32 flags, then
length-prefixed sections (code, consts, funcs, tools, jumptables, budgets,
debug), then a 32-byte digest. `decode` must reject truncation, unassigned
opcodes, dangling table indices, misaligned instruction streams.

### `cfg.rs`
```rust
pub struct Block { pub start: u32, pub end: u32,        // instruction indices
                   pub succs: Vec<u32>, pub preds: Vec<u32> }
pub struct Cfg { /* blocks for one function */ }
impl Cfg {
    pub fn build(image_code: &[u8], f: &FuncMeta) -> Result<Self>;
    pub fn blocks(&self) -> &[Block];
    pub fn block_of(&self, insn: u32) -> Option<u32>;
    pub fn rpo(&self) -> &[u32];                    // reverse postorder
    pub fn idom(&self, b: u32) -> Option<u32>;      // immediate dominator
    pub fn dominates(&self, a: u32, b: u32) -> bool;
    pub fn back_edges(&self) -> &[(u32, u32)];      // (tail, header)
    pub fn insn_at(&self, idx: u32) -> Decoded;     // decoded, cached
    pub fn insn_count(&self) -> u32;
    pub fn unreachable(&self) -> Vec<u32>;          // block ids
}
```
Cooper-Harvey-Kennedy iterative dominators. Instruction *indices*, not byte
offsets, everywhere in the public surface.

### `verify.rs`
```rust
pub fn admit(obj: Object) -> Result<Image>;
pub fn check_function(code: &[u8], f: &FuncMeta, ctx: &VerifyCtx) -> Result<FnFacts>;
pub struct VerifyCtx<'a> { pub funcs: &'a [FuncMeta], pub tools: &'a [ToolMeta],
                           pub consts: usize, pub jump_tables: &'a [Vec<u32>],
                           pub spans: &'a [Span] }
pub struct FnFacts { pub high_water: u16, pub passes: u32 }
pub struct AbsFrame { pub depth: u16, pub stack: Vec<Ty>, pub fork_depth: u8 }
```
Implements the seven proofs from the README. `admit` runs every function, then
calls `image::seal`. Diagnostics must attach spans from `ctx.spans` and use the
exact codes in `diag.rs`.

### `heap.rs`
```rust
pub struct Arena;
impl Arena {
    pub fn new(quota: u32) -> Self;
    pub fn alloc(&mut self, tag: Tag, bytes: &[u8]) -> Result<Handle>;
    pub fn alloc_list(&mut self, vals: &[Value]) -> Result<Handle>;
    pub fn bytes(&self, h: Handle) -> Result<&[u8]>;
    pub fn str(&self, h: Handle) -> Result<&str>;
    pub fn list(&self, h: Handle) -> Result<Vec<Value>>;
    pub fn len(&self) -> u32;
    pub fn used(&self) -> u32;
    pub fn quota(&self) -> u32;
    pub fn raw(&self) -> &[u8];                       // for snapshot
    pub fn from_raw(bytes: Vec<u8>, quota: u32) -> Self;
    pub fn fork(&mut self) -> ForkToken;              // COW
    pub fn commit(&mut self, t: ForkToken) -> Result<()>;
    pub fn abort(&mut self, t: ForkToken) -> Result<()>;
    pub fn fork_depth(&self) -> u8;
    #[cfg(debug_assertions)] pub fn check_invariants(&self);
}
pub struct ForkToken { /* opaque */ }
```
Bump allocator. Offset 0 reserved so `Handle::EMPTY` is a real empty value.
COW: copy the arena on fork; `commit` keeps the child, `abort` restores the
parent. Document that page-granular COW is the intended optimization and that
this is the whole-arena version.

### `frame.rs`
```rust
pub struct Frame { pub func: u32, pub pc: u32, pub base: u32,
                   pub depth: u16, pub fork_depth: u8 }
pub struct Stack;
impl Stack {
    pub fn new() -> Self;
    pub fn push_frame(&mut self, func: u32, pc: u32, maxstack: u16) -> Result<()>;
    pub fn pop_frame(&mut self) -> Option<Frame>;
    pub fn frame(&self) -> &Frame;
    pub fn frame_mut(&mut self) -> &mut Frame;
    pub fn frames(&self) -> &[Frame];
    pub fn depth(&self) -> usize;                     // frame count
    pub fn push(&mut self, v: Value);                 // verified: cannot overflow
    pub fn pop(&mut self) -> Value;                   // verified: cannot underflow
    pub fn peek(&self, back: u16) -> Value;
    pub fn operands(&self) -> &[Value];               // for snapshot
    pub fn local(&self, slot: u16) -> Value;
    pub fn set_local(&mut self, slot: u16, v: Value);
    pub fn from_parts(frames: Vec<Frame>, operands: Vec<Value>) -> Self;
}
```
`push`/`pop` do not return `Result` — the verifier proved they cannot fail. Say
so in the doc comment and debug-assert it.

### `budget.rs`
```rust
pub enum Dim { Tokens = 0, WallMs = 1, Tools = 2, Arena = 3 }
impl Dim { pub fn from_byte(u8) -> Option<Self>; pub fn name(self) -> &'static str }
pub struct Ledger;
impl Ledger {
    pub fn new(b: &Budgets) -> Self;
    pub fn reserve(&mut self, d: Dim, amount: u64) -> Result<()>;   // E_BUDGET
    pub fn release(&mut self, d: Dim, amount: u64);
    pub fn spend(&mut self, d: Dim, amount: u64) -> Result<()>;
    pub fn remaining(&self, d: Dim) -> u64;
    pub fn spent(&self, d: Dim) -> u64;
    pub fn reserved(&self, d: Dim) -> u64;
    pub fn write(&self, out: &mut Vec<u8>);
    pub fn read(buf: &[u8], b: &Budgets) -> Result<Self>;
}
```
Two-phase. `reserve` fails if `spent + reserved + amount > limit`. `spend`
settles against a reservation, releasing any excess.

### `ctx.rs`
```rust
pub struct Segment { pub role: u8, pub text: Handle, pub tokens: u32 }
pub struct Ring;
impl Ring {
    pub fn new() -> Self;
    pub fn push(&mut self, role: u8, text: Handle, tokens: u32);
    pub fn pop_oldest(&mut self, n: u32) -> u32;         // returns count dropped
    pub fn window(&self) -> &[Segment];
    pub fn cost(&self) -> u32;
    pub fn write(&self, out: &mut Vec<u8>);
    pub fn read(buf: &[u8]) -> Result<(Self, usize)>;    // value + bytes consumed
}
pub fn estimate_tokens(text: &str) -> u32;
```
Explicit eviction only — never drop a segment implicitly. `estimate_tokens` is a
documented heuristic (bytes/4 with whitespace correction), not a real tokenizer;
say so.

### `trap.rs`
```rust
pub enum Trap {
    Tool { id: u32, name: String, args: Vec<u8>, effect: EffectId },
    Spawn { func: u32, args: Vec<u8>, effect: EffectId },
    Await { effect: EffectId },
    Poll { effect: EffectId },
    Cancel { effect: EffectId },
    Select { effects: Vec<EffectId> },
    Checkpoint { label: String },
    Yield,
    Now, Rand,
    Env { key: String },
    Log { level: u8, message: String },
    Reserve { dim: u8, amount: u64 },
    Spend { dim: u8, amount: u64 },
    QueryQuota { dim: u8 },
}
pub enum Answer {
    Value(Value), Bytes { tag: Tag, data: Vec<u8> },
    Effect(EffectId), Ready { effect: EffectId, ready: bool },
    Selected { index: u32, value: Value },
    Int(i64), Ack, Failed { code: Code, message: String },
}
pub enum Step { Trap(Trap), Halted(i64) }
```
This is the whole interpreter/host boundary. Nothing else crosses it.

### `interp.rs`
```rust
pub struct Limits { pub arena: u32, pub max_frames: u16, pub max_steps: u64 }
impl Default for Limits;
pub struct Vm<'i>;
impl<'i> Vm<'i> {
    pub fn new(image: &'i Image, limits: Limits) -> Self;
    pub fn with_args(image: &'i Image, limits: Limits, args: Vec<String>) -> Self;
    pub fn step(&mut self, answer: Option<Answer>) -> Result<Step>;
    pub fn run<H: Host>(&mut self, host: &mut H) -> Result<i64>;
    pub fn pc(&self) -> u32;
    pub fn steps(&self) -> u64;
    pub fn stack(&self) -> &Stack;
    pub fn arena(&self) -> &Arena;
    pub fn ledger(&self) -> &Ledger;
    pub fn ctx(&self) -> &Ring;
    pub fn state_digest(&self) -> Digest;      // for --verify-replay
}
pub trait Host {
    fn perform(&mut self, trap: &Trap) -> Result<Answer>;
}
```
`step` executes until it needs the host, then returns a `Trap`. Never performs
I/O. No bounds/depth/type checks that the verifier already proved — comment the
inner loop where clarity was traded for speed.

### `cont.rs`
```rust
pub const SNAP_MAGIC: &[u8; 4] = b"CDXC";
pub const SNAP_VERSION: u16 = 1;
pub struct Snapshot { /* owns the bytes */ }
impl Snapshot {
    pub fn bytes(&self) -> &[u8];
    pub fn digest(&self) -> Digest;
    pub fn image_digest(&self) -> Digest;
    pub fn journal_seq(&self) -> u64;
    pub fn summary(&self) -> String;           // `cinder snap inspect` output
}
pub fn snapshot(vm: &Vm<'_>, journal_seq: u64) -> Snapshot;
pub fn restore<'i>(image: &'i Image, snap: &Snapshot, limits: Limits) -> Result<Vm<'i>>;
```
Layout is in the README. `restore` validates digest, image binding, frame and
operand bounds, and every arena handle's extent *before* constructing anything.

### `journal.rs`
```rust
pub enum Kind { ToolIssue=1, ToolAnswer=2, ToolChunk=3, SpawnIssue=4, SpawnAnswer=5,
                Now=6, Rand=7, Env=8, Log=9, Quota=10, Checkpoint=11,
                Resume=12, Select=13, Cancel=14, Halt=15 }
pub struct Record { pub seq: u64, pub kind: Kind, pub refs: u64,
                    pub payload: Vec<u8>, pub prev: Digest, pub hash: Digest }
pub struct Journal;
impl Journal {
    pub fn new() -> Self;
    pub fn append(&mut self, kind: Kind, refs: u64, payload: Vec<u8>) -> &Record;
    pub fn records(&self) -> &[Record];
    pub fn head(&self) -> Digest;
    pub fn seq(&self) -> u64;
    pub fn encode(&self) -> Vec<u8>;
    pub fn decode(bytes: &[u8]) -> Result<Self>;     // verifies the chain
    pub fn verify_chain(&self) -> Result<()>;
    pub fn cursor(&self) -> Cursor<'_>;
}
pub struct Cursor<'j>;
impl<'j> Cursor<'j> {
    pub fn next(&mut self) -> Result<&'j Record>;
    pub fn expect(&mut self, kind: Kind) -> Result<&'j Record>;   // E_DIVERGE
    pub fn seek(&mut self, seq: u64) -> Result<()>;
    pub fn last_checkpoint_before(&self, seq: u64) -> Option<u64>;
    pub fn pos(&self) -> u64;
}
```
`hash = Digest::of(prev || seq || kind || refs || payload)`. `decode` must
reject a broken chain with `E_CHAIN`.

### `replay.rs`
```rust
pub struct ReplayHost<'j> { /* wraps Cursor */ }
impl<'j> ReplayHost<'j> {
    pub fn new(j: &'j Journal) -> Self;
    pub fn seek(&mut self, seq: u64) -> Result<()>;
    pub fn pos(&self) -> u64;
}
impl Host for ReplayHost<'_>;                       // serves answers from records
pub struct RecordingHost<'a, H: Host> { /* wraps a real host + journal */ }
impl<'a, H: Host> RecordingHost<'a, H> {
    pub fn new(inner: &'a mut H, j: &'a mut Journal) -> Self;
}
impl<H: Host> Host for RecordingHost<'_, H>;
pub fn verify_replay(image: &Image, j: &Journal) -> Result<VerifyReport>;
pub struct VerifyReport { pub records: u64, pub matched: u64, pub head: Digest }
```
Divergence — the interpreter asking for a kind the journal does not have next —
is `E_DIVERGE` naming the record index, expected kind, and requested kind.
`RecordingHost` seals the record *before* the answer reaches the interpreter.

### `disas.rs`
```rust
pub fn disassemble(image: &Image) -> String;
pub fn disassemble_function(image: &Image, id: u32) -> String;
pub fn format_insn(image: &Image, pc: u32, d: Decoded) -> String;
pub fn annotate(image: &Image, pc: u32) -> Option<String>;   // resolved operand
```
Output resolves symbols: `ldc $sys ; "You triage..."`. Redact `env` values.
