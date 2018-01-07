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
