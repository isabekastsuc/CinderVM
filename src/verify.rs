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
                    None => {
                        return Err(Diag::new(
                            Code::StackUnderflow,
                            format!("{} underflows", d.op.mnemonic()),
                        )
                        .at_pc(pc));
                    }
                }
            }
            for push in insn.pushes {
                stack_types.push(*push);
                depth += 1;
                if depth > i32::from(f.maxstack) {
                    return Err(Diag::new(
                        Code::MaxstackExceeded,
                        format!("`{}` exceeds maxstack", f.name),
                    )
                    .at_pc(pc));
                }
            }
        }

        if matches!(d.op, Op::Ret | Op::Tail) && stack_types.contains(&Ty::Pending) {
            return Err(Diag::new(Code::PendingEscape, "pending value live at return").at_pc(pc));
        }
        if matches!(d.op, Op::Checkpoint | Op::Fork | Op::Commit)
            && stack_types.contains(&Ty::Pending)
        {
            return Err(
                Diag::new(Code::LivePending, "pending value live at snapshot boundary").at_pc(pc),
            );
        }
    }

    if !insns
        .iter()
        .any(|d| matches!(d.op, Op::Ret | Op::Tail | Op::Halt | Op::Trap))
    {
        return Err(Diag::new(Code::BadTarget, format!("`{}` has no exit", f.name)).at_pc(0));
    }

    let metering: [Op; 4] = [Op::Reserve, Op::Spend, Op::Release, Op::QueryQ];
    for (a, b) in g.back_edges() {
        let window: Vec<Op> = insns[*a as usize..=*b as usize]
            .iter()
            .map(|d| d.op)
            .collect();
        if !window.iter().any(|op| metering.contains(op)) {
            return Err(
                Diag::new(Code::UnmeteredLoop, "loop has no metering instruction").at_pc(*a),
            );
        }
    }

    let balance = insns.iter().fold(0i32, |acc, d| {
        acc + match d.op {
            Op::Fork => 1,
            Op::Commit | Op::Abort => -1,
            _ => 0,
        }
    });
    if balance != 0 {
        return Err(Diag::new(Code::ForkImbalance, "unbalanced fork/commit/abort").at_pc(0));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asm::assemble;

    fn admit_src(src: &str) -> Result<Image> {
        admit(assemble("t.cdx", src)?)
    }

    #[test]
    fn minimal_image_verifies() {
        let img = admit_src(
            ".isa cdx/4\n.image \"t\"\n.fn main() -> i32\n.maxstack 1\nmain:\n    ldi 0\n    ret\n",
        )
        .expect("minimal image builds");
        assert!(img.is_verified());
    }

    #[test]
    fn maxstack_violation_is_rejected() {
        let src = ".fn main() -> i32\n.maxstack 1\nmain:\n    ldi 1\n    ldi 2\n    ret\n";
        assert_eq!(admit_src(src).unwrap_err().code, Code::MaxstackExceeded);
    }

    #[test]
    fn loop_without_metering_is_rejected() {
        let src = ".fn main() -> i32\n.maxstack 1\nmain:\n    ldi 0\nloop:\n    brz done\n    br loop\ndone:\n    ret\n";
        assert_eq!(admit_src(src).unwrap_err().code, Code::UnmeteredLoop);
    }
}
