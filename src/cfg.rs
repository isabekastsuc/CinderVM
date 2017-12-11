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
        for (id, &start) in starts.iter().enumerate() {
            let end = starts.get(id + 1).copied().unwrap_or(n);
            for slot in &mut owner[start as usize..end as usize] {
                *slot = id as u32;
            }
            blocks.push(Block {
                start,
                end,
                succs: Vec::new(),
                preds: Vec::new(),
            });
        }

        // Pass 2: lift the terminator's instruction-level successors to blocks.
        // Every successor is a leader by construction, so `owner` maps it to a
        // block whose `start` it equals.
        for id in 0..blocks.len() {
            let last = blocks[id].end - 1;
            let mut succs = Vec::new();
            for s in raw_succs(&insns, last, jump_tables)? {
                let b = owner[s as usize];
                if !succs.contains(&b) {
                    succs.push(b);
                }
            }
            blocks[id].succs = succs;
        }
        for id in 0..blocks.len() {
            for k in 0..blocks[id].succs.len() {
                let s = blocks[id].succs[k] as usize;
                let me = id as u32;
                if !blocks[s].preds.contains(&me) {
                    blocks[s].preds.push(me);
                }
            }
        }

        let rpo = reverse_postorder(&blocks);
        let mut rpo_num = vec![UNDEF; blocks.len()];
        for (pos, &b) in rpo.iter().enumerate() {
            rpo_num[b as usize] = pos as u32;
        }
        let idoms = dominators(&blocks, &rpo, &rpo_num);

        let mut cfg = Self {
            insns,
            offsets,
            blocks,
            owner,
            rpo,
            rpo_num,
            idoms,
            back_edges: Vec::new(),
        };
        cfg.back_edges = cfg.find_back_edges();
        Ok(cfg)
    }

    /// All basic blocks, ordered by `start`. Block 0 is the entry.
    #[must_use]
    pub fn blocks(&self) -> &[Block] {
        &self.blocks
    }

    /// Block containing instruction `insn`, or `None` if out of range.
    #[must_use]
    pub fn block_of(&self, insn: u32) -> Option<u32> {
        self.owner.get(insn as usize).copied()
    }

    /// Reverse postorder over blocks *reachable from the entry*. Unreachable
    /// blocks are absent, which is what makes it a valid dominator iteration
    /// order — see [`Cfg::unreachable`] for the rest.
    #[must_use]
    pub fn rpo(&self) -> &[u32] {
        &self.rpo
    }

    /// Immediate dominator of `b`. `None` for the entry block, for an
    /// unreachable block, and for an out-of-range id.
    #[must_use]
    pub fn idom(&self, b: u32) -> Option<u32> {
        match self.idoms.get(b as usize).copied() {
            Some(d) if d != UNDEF && d != b => Some(d),
            _ => None,
        }
    }

    /// Whether `a` dominates `b`: whether every path from the entry to `b`
    /// passes through `a`. Walks `b`'s idom chain, which is O(depth).
    #[must_use]
    pub fn dominates(&self, a: u32, b: u32) -> bool {
        if a as usize >= self.blocks.len() || b as usize >= self.blocks.len() {
            return false;
        }
        if self.rpo_num[a as usize] == UNDEF || self.rpo_num[b as usize] == UNDEF {
            return false; // domination is meaningless off the reachable subgraph
        }
        let mut cur = b;
        loop {
            if cur == a {
                return true;
            }
            match self.idom(cur) {
                Some(next) => cur = next,
                None => return false,
            }
        }
    }

    /// Back edges as `(tail, header)`, where the header *dominates* the tail.
    ///
    /// Dominance is the whole point. "Successor index is below source index" is
    /// the cheap approximation, and it is wrong in both directions: it flags the
    /// re-entry edge of an irreducible graph, where the target is merely earlier
    /// and not a loop header, and it misses nothing only by accident of layout.
    /// `verify.rs` turns each pair here into the claim "this cycle must contain
    /// a metering instruction dominated by the header" (`E_UNMETERED_LOOP`), so
    /// a false positive rejects a valid image and a false negative admits an
    /// unmeterable loop. Only dominance identifies a real natural loop.
    #[must_use]
    pub fn back_edges(&self) -> &[(u32, u32)] {
        &self.back_edges
    }

    /// The cached decoded instruction at `idx`.
    ///
    /// Panics if `idx >= insn_count()`. That is deliberate: `build` decoded the
    /// entire range and every public index-producing method is bounded by it, so
    /// an out-of-range index is a bug in the caller, not bad input, and a
    /// `Result` here would only push the `unwrap` outwards.
    #[must_use]
