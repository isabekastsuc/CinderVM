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
    pub fn insn_at(&self, idx: u32) -> Decoded {
        self.insns[idx as usize]
    }

    /// Number of instructions in the function.
    #[must_use]
    pub fn insn_count(&self) -> u32 {
        self.insns.len() as u32
    }

    /// The decoded instruction stream, function-local order.
    #[must_use]
    pub fn decoded(&self) -> &[Decoded] {
        &self.insns
    }

    /// Absolute byte offset of instruction `idx` in the image code section.
    /// Panics on an out-of-range index, for the reason given on
    /// [`Cfg::insn_at`].
    #[must_use]
    pub fn byte_offset(&self, idx: u32) -> u32 {
        self.offsets[idx as usize]
    }

    /// Inverse of [`Cfg::byte_offset`]: the instruction index that begins at
    /// absolute offset `off`, or `None` if `off` is outside the function or
    /// interior to an instruction. `O(log n)` — the offset table is ascending.
    #[must_use]
    pub fn index_at_byte(&self, off: u32) -> Option<u32> {
        self.offsets.binary_search(&off).ok().map(|i| i as u32)
    }

    /// Block ids not reachable from the entry, ascending. Non-empty means the
    /// function carries dead code, which `verify.rs` rejects with
    /// `E_UNREACHABLE`; the CFG itself merely reports it.
    #[must_use]
    pub fn unreachable(&self) -> Vec<u32> {
        self.rpo_num
            .iter()
            .enumerate()
            .filter(|(_, &pos)| pos == UNDEF)
            .map(|(id, _)| id as u32)
            .collect()
    }

    fn find_back_edges(&self) -> Vec<(u32, u32)> {
        let mut out = Vec::new();
        for (id, b) in self.blocks.iter().enumerate() {
            let tail = id as u32;
            if self.rpo_num[id] == UNDEF {
                continue;
            }
            for &h in &b.succs {
                if self.dominates(h, tail) {
                    out.push((tail, h));
                }
            }
        }
        out
    }
}

/// Decode `code[f.code_off .. f.code_off + f.code_len]` linearly, returning the
/// instruction stream and its absolute offset table.
fn decode_range(code: &[u8], f: &FuncMeta) -> Result<(Vec<Decoded>, Vec<u32>)> {
    let lo = f.code_off as usize;
    let hi = lo + f.code_len as usize;
    let Some(body) = code.get(lo..hi) else {
        return Err(Diag::new(
            Code::TruncatedSection,
            format!(
                "function `{}` claims code {lo}..{hi} but the section is {} bytes",
                f.name,
                code.len()
            ),
        ));
    };

    let mut insns = Vec::new();
    let mut offsets = Vec::new();
    let mut off = 0usize;
    while off < body.len() {
        let Some(d) = isa::decode(body, off) else {
            let pc = insns.len() as u32;
            let wide = body[off] == isa::WIDE_PREFIX;
            let need = if wide {
                2 * isa::INSN_LEN
            } else {
                isa::INSN_LEN
            };
            if off + need > body.len() {
                return Err(bad(
                    Code::MisalignedInsn,
                    format!(
                        "function `{}` ends mid-instruction: {} trailing byte(s) at offset {}",
                        f.name,
                        body.len() - off,
                        lo + off
                    ),
                    pc,
                ));
            }
            return Err(bad(
                Code::UnassignedOpcode,
                format!(
                    "byte {:#04x} at offset {} is not an opcode",
                    body[off],
                    lo + off
                ),
                pc,
            ));
        };
        offsets.push((lo + off) as u32);
        insns.push(d);
        off += usize::from(d.len);
    }
    debug_assert_eq!(off, body.len(), "decode never reads past the slice");
    Ok((insns, offsets))
}

/// Successors of instruction `i` as instruction indices, validating every edge.
///
/// Called once per instruction to find leaders and once per block to fill
/// `succs`; recomputing is cheaper than caching a `Vec` per instruction and
/// keeps the two passes from drifting.
fn raw_succs(insns: &[Decoded], i: u32, jump_tables: &[Vec<u32>]) -> Result<Vec<u32>> {
    let n = insns.len() as u32;
    let d = insns[i as usize];
    let next = i + 1;
    let check = |t: u32| -> Result<u32> {
        if t < n {
            Ok(t)
        } else {
            Err(bad(
                Code::BadTarget,
                format!(
                    "`{}` targets instruction {t}, past the function's {n}",
                    d.op.mnemonic()
                ),
                i,
            ))
        }
    };

    match d.op {
        Op::Br => Ok(vec![check(d.b)?]),
        Op::Brz | Op::Brnz => Ok(vec![check(d.b)?, check(next)?]),
        Op::Switch => {
            let Some(table) = jump_tables.get(d.b as usize) else {
                return Err(bad(
                    Code::DanglingIndex,
                    format!("`switch` names jump table {} of {}", d.b, jump_tables.len()),
                    i,
                ));
            };
            if table.is_empty() {
                return Err(bad(
                    Code::SwitchNoDefault,
                    format!("jump table {} is empty; the default arm is mandatory", d.b),
                    i,
                ));
            }
            table.iter().map(|&t| check(t)).collect()
        }
        // Terminators with no in-function successor: `ret`, `tail`, `trap`,
        // `halt`, `abort`. Taken from the table so a new terminal opcode needs
        // no change here.
        op if !op.insn().falls_through => Ok(Vec::new()),
        // Falling off the end of a function is an edge to a nonexistent
        // instruction, and reported as such rather than silently dropped.
        _ => Ok(vec![check(next)?]),
    }
}

/// Iterative depth-first postorder from block 0, reversed. Iterative rather
/// than recursive because block counts follow function size, and a deep
/// generated function must not overflow the stack during verification.
fn reverse_postorder(blocks: &[Block]) -> Vec<u32> {
    if blocks.is_empty() {
        return Vec::new();
    }
    let mut seen = vec![false; blocks.len()];
    let mut post = Vec::with_capacity(blocks.len());
    let mut stack = vec![(0u32, 0usize)];
    seen[0] = true;
    while let Some(top) = stack.last_mut() {
        let (b, k) = (top.0, top.1);
        top.1 += 1;
        match blocks[b as usize].succs.get(k) {
            Some(&s) => {
                if !seen[s as usize] {
                    seen[s as usize] = true;
                    stack.push((s, 0));
                }
            }
            None => {
                post.push(b);
                stack.pop();
            }
        }
    }
    post.reverse();
    post
}

/// Cooper-Harvey-Kennedy iterative dominators ("A Simple, Fast Dominance
/// Algorithm", Rice CS-TR-06-33870).
///
/// Sweeps blocks in reverse postorder, recomputing each one's idom as the
/// pairwise `intersect` of its already-processed predecessors, until nothing
/// changes. Worst case O(N·E) — the RPO ordering makes it converge in two
/// passes on reducible graphs, so it is effectively linear on real code, and it
/// beats Lengauer-Tarjan in practice at these sizes while being short enough to
/// audit. The result is the idom tree, not a dominator bitset: `dominates` is a
/// chain walk, which suits the verifier's few queries per back edge.
fn dominators(blocks: &[Block], rpo: &[u32], rpo_num: &[u32]) -> Vec<u32> {
    let mut idom = vec![UNDEF; blocks.len()];
    if rpo.is_empty() {
        return idom;
    }
    idom[rpo[0] as usize] = rpo[0]; // the entry dominates itself

    let mut changed = true;
    while changed {
        changed = false;
        for &b in rpo.iter().skip(1) {
            let mut new = UNDEF;
            for &p in &blocks[b as usize].preds {
                if rpo_num[p as usize] == UNDEF || idom[p as usize] == UNDEF {
                    continue; // unreachable, or not yet processed in this sweep
                }
                new = if new == UNDEF {
                    p
                } else {
                    intersect(&idom, rpo_num, p, new)
                };
            }
            if new != UNDEF && idom[b as usize] != new {
                idom[b as usize] = new;
                changed = true;
            }
        }
    }
    idom
}

/// Nearest common dominator of `a` and `b`: walk both idom chains upwards,
/// always advancing whichever sits deeper in reverse postorder, until they
/// meet. Terminates because the entry is every reachable block's ancestor and
/// its RPO number is 0.
fn intersect(idom: &[u32], rpo_num: &[u32], mut a: u32, mut b: u32) -> u32 {
    while a != b {
        while rpo_num[a as usize] > rpo_num[b as usize] {
            a = idom[a as usize];
        }
        while rpo_num[b as usize] > rpo_num[a as usize] {
            b = idom[b as usize];
        }
    }
    a
}

// CFG_TESTS_PLACEHOLDER
