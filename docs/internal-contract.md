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
