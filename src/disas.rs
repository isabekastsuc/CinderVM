//! Disassembly: instruction-level listing of a sealed image.

use crate::image::Image;
use crate::isa::{self, Operand};

/// Render the whole image as text.
pub fn emit(image: &Image) -> String {
    let mut out = String::new();
    out.push_str(&format!("; {} (cdx/{})\n", image.name(), isa::ISA_VERSION));
    for (fi, f) in image.funcs().iter().enumerate() {
        out.push_str(&format!(
            "\n.fn {}() -> {} (maxstack {})\n",
            f.name,
            if f.returns == 0 { "void" } else { "i32" },
            f.maxstack
        ));
        let lo = f.code_off as usize;
        let hi = lo + f.code_len as usize;
        let mut off = lo;
        while off < hi {
            let d = isa::decode(image.code(), off).expect("sealed image decodes");
            let rel = (off - lo) as u32 / isa::INSN_LEN as u32;
            out.push_str(&format!("  {rel:>4}:  {}\n", render(image, &d)));
            off += d.len as usize;
        }
        let _ = fi;
    }
    if !image.tools().is_empty() {
        out.push_str("\n.tools:");
        for t in image.tools() {
            out.push_str(&format!(" {t}"));
        }
        out.push('\n');
    }
    out
}

fn render(image: &Image, d: &isa::Decoded) -> String {
    let insn = d.op.insn();
    let opname = d.op.mnemonic();
    let operand = match insn.operand {
        Operand::None => String::new(),
        Operand::Const => format!(
            " [{}]",
            image
                .constants()
                .get(d.b as usize)
                .map(|c| String::from_utf8_lossy(c).into_owned())
                .unwrap_or_default()
        ),
        Operand::Imm => format!(" {}", isa::imm(*d)),
        Operand::Target => format!(" {}", d.b),
        Operand::Func => {
            let name = image
                .funcs()
                .get(d.b as usize)
                .map(|f| f.name.clone())
                .unwrap_or_default();
            format!(" {name}")
        }
        Operand::Tool => {
            let name = image.tools().get(d.b as usize).cloned().unwrap_or_default();
            format!(" {name}")
        }
        Operand::JumpTable => format!(" table:{}", d.b),
        Operand::Slot => format!(" slot:{}", d.b),
        Operand::Dimension => format!(" dim:{}", d.b),
