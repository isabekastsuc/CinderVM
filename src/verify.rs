//! Verification: the precondition for an [`Image`]. Nothing runs before
//! `admit` proves the invariants in the crate root's module docs.

use std::collections::HashMap;

use crate::asm::Object;
use crate::cfg;
use crate::diag::{Code, Diag, Result};
use crate::image::{self, FuncMeta, Image};
use crate::isa::{Op, Operand, Ty};

/// Prove the invariants and seal the image.
pub fn admit(object: Object) -> Result<Image> {
    image::seal_decode_check(&object.code)?;
    let mut funcs = object.funcs;
    if funcs.is_empty() {
        return Err(Diag::new(Code::NoEntryPoint, "image has no functions"));
    }
    let func_index: HashMap<String, u32> = funcs
        .iter()
        .enumerate()
        .map(|(i, f)| (f.name.clone(), i as u32))
        .collect();

    for f in &funcs {
        check_function(
            &object.code,
            f,
            &func_index,
            &object.jump_tables,
            object.constants.len(),
        )?;
    }

    let mut image = image::build(
        object.name,
        object.constants,
        funcs,
        object.tools,
        object.code,
        object.jump_tables,
    );
    image::mark_verified(&mut image);
    Ok(image)
}

fn check_function(
    code: &[u8],
    f: &FuncMeta,
    func_index: &HashMap<String, u32>,
    jump_tables: &[Vec<u32>],
    n_consts: usize,
) -> Result<()> {
    if f.maxstack == 0 {
        return Err(Diag::new(
            Code::MissingMaxstack,
            format!("`{}` declares maxstack 0", f.name),
        )
        .at_pc(0));
    }
    let g = cfg::Cfg::build(code, f, jump_tables)?;
    let insns = g.decoded();

    let mut depth: i32 = 0;
    let mut stack_types: Vec<Ty> = Vec::new();
    for (idx, d) in insns.iter().enumerate() {
        let insn = d.op.insn();
        let pc = idx as u32;

        if matches!(insn.operand, Operand::Const) && (d.b as usize) >= n_consts {
            return Err(Diag::new(Code::DanglingIndex, "constant index out of range").at_pc(pc));
        }
        if matches!(insn.operand, Operand::Func) && func_index.values().all(|i| *i != d.b) {
            return Err(Diag::new(Code::DanglingIndex, "function index out of range").at_pc(pc));
        }
        if matches!(insn.operand, Operand::JumpTable) && (d.b as usize) >= jump_tables.len() {
            return Err(Diag::new(Code::DanglingIndex, "jump table out of range").at_pc(pc));
        }

        if insn.variadic {
            depth = 0;
            stack_types.clear();
        } else {
            for want in insn.pops {
                match stack_types.last() {
                    Some(got) if got.satisfies(*want) => {
                        stack_types.pop();
                        depth -= 1;
                    }
                    Some(got) => {
                        return Err(Diag::new(
                            Code::TypeMismatch,
                            format!("expected {}, found {}", want.name(), got.name()),
                        )
                        .at_pc(pc));
                    }
