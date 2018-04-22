//! Per-function control-flow graph: basic blocks over instruction *indices*,
//! reverse postorder, Cooper-Harvey-Kennedy dominators, and dominator-based
//! back edges.
//!
//! # What this module guarantees
//!
//! A successfully built [`Cfg`] proves, for one function:
//!
//! - the code range decodes cleanly and lands exactly on the function's end
//!   boundary (otherwise `E_MISALIGNED` / `E_BAD_OPCODE`);
//! - every control-flow edge — branch target, `switch` arm, and implicit
//!   fallthrough alike — names an instruction inside the function (otherwise
//!   `E_BAD_TARGET`);
//! - the block partition covers `0..insn_count` exactly once, so
//!   [`Cfg::block_of`] is total over that range;
//! - [`Cfg::rpo`] lists exactly the blocks reachable from block 0, and
//!   [`Cfg::idom`] is defined for every one of them except the entry.
//!
//! # What it assumes the caller proved
//!
//! Nothing. `Cfg::build` is the *first* pass over a function's bytecode:
//! `verify.rs` builds one before it does anything else, so the CFG cannot
//! assume any verifier fact. It does assume `f.code_off`/`f.code_len` describe
//! a range the container's section layout already bounds-checked, and reports
//! `E_TRUNCATED` if not.
//!
//! # Indices, not byte offsets
//!
//! Everything public counts instructions, function-locally: index 0 is the
//! function's first instruction. Because [`Decoded::len`] is 4 or 8 — the wide
//! prefix doubles it — index↔offset is *not* a multiply, so `build` caches the
//! decoded stream and a parallel index→offset table once instead of rescanning
//! from the function start on every query. That makes [`Cfg::insn_at`] and
//! [`Cfg::byte_offset`] O(1) and, more importantly, keeps every consumer
//! (verifier, disassembler) from reimplementing the walk and disagreeing about
//! where instruction *n* begins.

use crate::diag::{Code, Diag, Result};
use crate::image::FuncMeta;
use crate::isa::{self, Decoded, Op};

/// Sentinel for "no immediate dominator yet" and "not in reverse postorder".
/// A real block id can never reach `u32::MAX`: a function's code section is
/// addressed by `u32` byte offsets and every instruction costs at least four
/// bytes.
const UNDEF: u32 = u32::MAX;

/// One basic block, as a half-open range of function-local instruction indices.
#[derive(Clone, Debug)]
pub struct Block {
    /// First instruction of the block; always a leader.
    pub start: u32,
    /// One past the last instruction: the block spans `start..end`, and its
    /// terminator is `end - 1`.
    pub end: u32,
    /// Successor block ids in ISA order: `[taken, fallthrough]` for a
    /// conditional branch, jump-table order (arms then default) for `switch`.
    /// Deduplicated, so a `switch` with repeated arms yields one edge each.
    pub succs: Vec<u32>,
    /// Predecessor block ids, ascending and deduplicated. Derived from
    /// `succs`; never a source of truth.
    pub preds: Vec<u32>,
}

/// The control-flow graph of a single function.
#[derive(Clone, Debug)]
pub struct Cfg {
    /// Decoded instruction stream, indexed by function-local index.
    insns: Vec<Decoded>,
    /// Byte offset of each instruction, *absolute* in the image code section,
    /// so `disas.rs` can print addresses that match the container.
    offsets: Vec<u32>,
    blocks: Vec<Block>,
    /// Instruction index → owning block id. Total over `0..insn_count`.
    owner: Vec<u32>,
    rpo: Vec<u32>,
    /// Block id → position in `rpo`, or [`UNDEF`] when unreachable. The
    /// dominator iteration needs this lookup in the inner loop.
    rpo_num: Vec<u32>,
    /// Immediate dominators; entry maps to itself, unreachable to [`UNDEF`].
    idoms: Vec<u32>,
    back_edges: Vec<(u32, u32)>,
}

fn bad(code: Code, message: String, pc: u32) -> Diag {
    Diag::new(code, message).at_pc(pc)
}

impl Cfg {
    /// Build the graph for `f` out of the image's code section.
    ///
    /// `jump_tables` is the image's jump-table section; `switch` operands index
    /// it. A table's last entry is its mandatory default arm and the preceding
    /// entries are the dense arms, so an empty table is `E_SWITCH_DEFAULT`.
    pub fn build(code: &[u8], f: &FuncMeta, jump_tables: &[Vec<u32>]) -> Result<Self> {
        let (insns, offsets) = decode_range(code, f)?;
        let n = insns.len() as u32;

        // Pass 1: validate every edge and mark leaders. Leaders are instruction
        // 0, every branch or `switch` target, and every instruction following a
        // terminator or a conditional branch.
        let mut leader = vec![false; insns.len()];
        if !leader.is_empty() {
            leader[0] = true;
        }
        for i in 0..n {
            let succs = raw_succs(&insns, i, jump_tables)?;
            // Plain fallthrough does not split a block. Terminals are excluded
            // even when their only successor happens to be `i + 1` (`br` to the
            // next instruction), so that the terminator of every block is always
            // its last instruction — pass 2 depends on that.
            if insns[i as usize].op.insn().falls_through && succs.len() == 1 && succs[0] == i + 1 {
                continue;
            }
            for &s in &succs {
                leader[s as usize] = true;
            }
            if i + 1 < n {
                leader[(i + 1) as usize] = true;
            }
        }

        let starts: Vec<u32> = leader
            .iter()
            .enumerate()
            .filter(|(_, &l)| l)
            .map(|(i, _)| i as u32)
            .collect();

        let mut owner = vec![0u32; insns.len()];
        let mut blocks = Vec::with_capacity(starts.len());
