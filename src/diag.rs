//! Diagnostics: source spans, stable error codes, and rendering.
//!
//! Every failure in this crate is a [`Diag`] carrying a machine-readable
//! [`Code`] and, where a source is available, a span into it. The codes are
//! part of the public contract — `corpus/MANIFEST.tsv` names the exact code
//! each rejected image must produce, so renaming one breaks the conformance
//! suite on purpose.

use core::fmt;

/// Byte range into a source file, plus a precomputed line/column for rendering.
///
/// Spans survive assembly: `asm` attaches one to every emitted instruction and
/// `image` stores them in an optional debug section, which is what lets a
/// *verifier* failure point at `.cdx` source even though the verifier only ever
/// sees bytecode.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub struct Span {
    pub lo: u32,
    pub hi: u32,
    pub line: u32,
    pub col: u32,
}

impl Span {
    #[must_use]
    pub const fn new(lo: u32, hi: u32, line: u32, col: u32) -> Self {
        Self { lo, hi, line, col }
    }

    /// A span with no extent, used for synthesized instructions (branch padding,
    /// the implicit `halt` on a fallthrough at end of function).
    #[must_use]
    pub const fn synthetic() -> Self {
        Self {
            lo: 0,
            hi: 0,
            line: 0,
            col: 0,
        }
    }

    #[must_use]
    pub const fn is_synthetic(self) -> bool {
        self.line == 0
    }

    /// Smallest span covering both. Used when a diagnostic blames a region
    /// spanning several instructions, e.g. an unbalanced `fork`/`commit` pair.
    #[must_use]
    pub fn merge(self, other: Self) -> Self {
        if self.is_synthetic() {
            return other;
        }
        if other.is_synthetic() {
            return self;
        }
        let (lo, line, col) = if self.lo <= other.lo {
            (self.lo, self.line, self.col)
        } else {
            (other.lo, other.line, other.col)
        };
        Self {
            lo,
            hi: self.hi.max(other.hi),
            line,
            col,
        }
    }
}

impl fmt::Debug for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_synthetic() {
            f.write_str("<synthetic>")
        } else {
            write!(f, "{}:{}", self.line, self.col)
        }
    }
}

macro_rules! codes {
    ($( $variant:ident = $text:literal, $phase:ident ; $blurb:literal );* $(;)?) => {
        /// Stable error code. The string form is what appears in diagnostics and
        /// in `corpus/MANIFEST.tsv`.
        #[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
        pub enum Code { $( $variant ),* }

        impl Code {
            #[must_use]
            pub const fn as_str(self) -> &'static str {
                match self { $( Self::$variant => $text ),* }
            }

            /// Which stage can emit this code. Used by the corpus runner to
            /// check that a rejection happened at the stage it claims.
            #[must_use]
            pub const fn phase(self) -> Phase {
                match self { $( Self::$variant => Phase::$phase ),* }
            }

            /// One-line explanation, surfaced by `cinder explain <code>`.
            #[must_use]
            pub const fn blurb(self) -> &'static str {
                match self { $( Self::$variant => $blurb ),* }
            }

            /// Parse a code from its string form. The corpus manifest is text,
            /// so this is the inverse the runner needs.
            #[must_use]
            pub fn parse(s: &str) -> Option<Self> {
                match s { $( $text => Some(Self::$variant), )* _ => None }
            }

            /// Every code, for `cinder explain --list` and for the test that
            /// asserts the manifest only references codes that exist.
            #[must_use]
            pub const fn all() -> &'static [Self] {
                &[ $( Self::$variant ),* ]
            }
        }
    };
}

/// Pipeline stage a diagnostic originates from.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Phase {
    /// Tokenizing `.cdx`.
    Lex,
    /// Parsing, symbol resolution, encoding.
    Assemble,
    /// Container decode and sealing.
    Load,
    /// Abstract interpretation.
    Verify,
    /// Execution.
    Run,
    /// Snapshot restore.
    Restore,
    /// Journal integrity and replay divergence.
    Journal,
}

codes! {
    // ── lexer ────────────────────────────────────────────────────────────
    UnterminatedString = "E_UNTERMINATED",  Lex;      "String literal reaches end of line without a closing quote.";
    BadEscape          = "E_BAD_ESCAPE",    Lex;      "Unrecognized escape sequence in a string literal.";
    BadNumber          = "E_BAD_NUMBER",    Lex;      "Integer literal is malformed or does not fit in i64.";
    StrayChar          = "E_STRAY_CHAR",    Lex;      "Character cannot begin any token.";

    // ── assembler ────────────────────────────────────────────────────────
    UnknownMnemonic    = "E_UNKNOWN_OP",    Assemble; "No instruction by that name in this ISA version.";
    UnknownDirective   = "E_UNKNOWN_DIR",   Assemble; "Directive is not recognized.";
    BadOperand         = "E_BAD_OPERAND",   Assemble; "Operand form does not match what the instruction takes.";
    MissingOperand     = "E_MISSING_OPERAND", Assemble; "Instruction requires an operand and none was given.";
    UnexpectedOperand  = "E_UNEXPECTED_OPERAND", Assemble; "Instruction takes no operand.";
    UndefinedLabel     = "E_UNDEF_LABEL",   Assemble; "Branch target was never defined in this function.";
    DuplicateLabel     = "E_DUP_LABEL",     Assemble; "Label defined twice in the same function.";
    DuplicateSymbol    = "E_DUP_SYMBOL",    Assemble; "Function, tool, or constant name declared twice.";
    UnknownSymbol      = "E_UNKNOWN_SYMBOL", Assemble; "Reference to a function, tool, or constant that was never declared.";
    NoEntryPoint       = "E_NO_ENTRY",      Assemble; "Image declares no `main`.";
    MissingMaxstack    = "E_NO_MAXSTACK",   Assemble; "Function omits the required `.maxstack` declaration.";
    OperandOverflow    = "E_OPERAND_RANGE", Assemble; "Operand does not fit even with the wide prefix.";
    BranchTooFar       = "E_BRANCH_RANGE",  Assemble; "Branch target is outside the encodable range.";

    // ── loader ───────────────────────────────────────────────────────────
    BadMagic           = "E_BAD_MAGIC",     Load;     "Not a .cdxb container.";
    BadIsaVersion      = "E_ISA_VERSION",   Load;     "Image targets an ISA version this build does not implement.";
    TruncatedSection   = "E_TRUNCATED",     Load;     "A section extends past the end of the file.";
    BadChecksum        = "E_CHECKSUM",      Load;     "Image digest does not match its contents.";
    UnassignedOpcode   = "E_BAD_OPCODE",    Load;     "Code section contains a byte that is not an assigned opcode.";
    MisalignedInsn     = "E_MISALIGNED",    Load;     "Instruction stream does not decode cleanly to the end of a function.";
    DanglingIndex      = "E_DANGLING_INDEX", Load;    "Operand references a table entry that does not exist.";

    // ── verifier ─────────────────────────────────────────────────────────
    DepthMerge         = "E_DEPTH_MERGE",   Verify;   "Two paths reach the same instruction with different stack depths.";
    TypeMerge          = "E_TYPE_MERGE",    Verify;   "Two paths reach the same instruction with incompatible operand types.";
    TypeMismatch       = "E_TYPE",          Verify;   "Instruction requires an operand type the stack cannot supply.";
    StackUnderflow     = "E_UNDERFLOW",     Verify;   "Instruction pops more operands than the stack holds.";
    MaxstackExceeded   = "E_MAXSTACK",      Verify;   "Computed high-water mark exceeds the declared `.maxstack`.";
    PendingEscape      = "E_PENDING_ESCAPE", Verify;  "A pending effect is live at a return or snapshot boundary.";
    PendingMisuse      = "E_PENDING_MISUSE", Verify;  "A pending value is used by an instruction that does not consume effects.";
    ForkImbalance      = "E_FORK_IMBALANCE", Verify;  "fork/commit/abort nesting differs between paths, or is unbalanced at return.";
    UnmeteredLoop      = "E_UNMETERED_LOOP", Verify;  "A cycle in the control-flow graph contains no metering instruction.";
    BadTarget          = "E_BAD_TARGET",    Verify;   "Branch target is out of range or not an instruction boundary.";
    ReturnArity        = "E_RETURN_ARITY",  Verify;   "Return leaves a stack depth the function's signature does not declare.";
    Unreachable        = "E_UNREACHABLE",   Verify;   "Instruction is unreachable; images must not carry dead code.";
    SwitchNoDefault    = "E_SWITCH_DEFAULT", Verify;  "Jump table has no default arm.";

    // ── runtime ──────────────────────────────────────────────────────────
    DivideByZero       = "E_DIV0",          Run;      "Division or remainder by zero.";
    IndexRange         = "E_RANGE",         Run;      "List index outside its bounds.";
    ArityMismatch      = "E_ARITY",         Run;      "unpack count does not match the list's length.";
    ArenaExhausted     = "E_ARENA",         Run;      "Heap arena hit its quota.";
    BudgetExceeded     = "E_BUDGET",        Run;      "Reservation refused: the tenant's allowance is spent.";
    ToolFailed          = "E_TOOL",         Run;      "The host reported a terminal failure for a tool call.";
    Trapped            = "E_TRAP",          Run;      "Program executed an explicit trap.";
    TagMismatch        = "E_TAG",           Run;      "Arena value's tag does not match the operation.";

    // ── snapshots ────────────────────────────────────────────────────────
    ImageMismatch      = "E_IMAGE_MISMATCH", Restore; "Snapshot was taken against a different image.";
    SnapshotCorrupt    = "E_SNAP_CORRUPT",  Restore;  "Snapshot digest does not match its contents.";
    SnapshotVersion    = "E_SNAP_VERSION",  Restore;  "Snapshot format is newer than this build.";
    BadHandle          = "E_BAD_HANDLE",    Restore;  "Snapshot contains a handle outside the arena it ships with.";
    LivePending        = "E_LIVE_PENDING",  Restore;  "Snapshot contains an unresolved effect with no journal record.";

    // ── journal ──────────────────────────────────────────────────────────
    ChainBroken        = "E_CHAIN",         Journal;  "Journal hash chain does not link; the log was edited or truncated.";
    Diverged           = "E_DIVERGE",       Journal;  "Replay asked for something the journal does not record next.";
    JournalExhausted   = "E_JOURNAL_END",   Journal;  "Replay ran past the end of the journal.";
    RecordMalformed    = "E_BAD_RECORD",    Journal;  "Journal record does not decode.";
}

/// A secondary annotation on a diagnostic: a related location with a note.
#[derive(Clone, Debug)]
pub struct Label {
    pub span: Span,
    pub note: String,
