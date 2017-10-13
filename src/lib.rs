//! # cindervm
//!
//! A deterministic bytecode virtual machine for agent execution.
//!
//! Agent runs are long-lived, effectful, and resumable. This crate models one as
//! bytecode on a machine built for it, rather than as a host-language coroutine
//! whose suspended state has no serializable form. The payoff is that a run can
//! be snapshotted mid-flight, moved to another host, and replayed exactly from
//! its journal.
//!
//! ## The three invariants
//!
//! Most of this codebase's shape follows from three rules. They are stated here
//! because every module depends on the others honouring them.
//!
//! ### 1. The interpreter performs no I/O
//!
//! [`interp::Vm::step`] is a pure function of `(image, state, answer)`. When it
//! needs an effect it returns a [`trap::Trap`] describing what it wants and
//! stops. The host performs the effect and hands back an [`trap::Answer`].
//!
//! This is what makes replay exact rather than approximate: `replay::Host`
//! serves answers from a journal instead of from the network, and the
//! interpreter cannot tell the difference because there was never a code path
//! that touched the network to begin with.
//!
//! ### 2. Every value is `Copy`, tagged, and pointer-free
//!
//! [`value::Value`] is 16 bytes with an explicit tag byte and no `Drop`. Larger
//! payloads live in the arena behind a [`value::Handle`], which is an
//! arena-relative offset rather than a pointer.
//!
//! Consequently [`cont::snapshot`] is two memcpys and a hash, and
//! [`cont::restore`] relocates by arithmetic. No graph walk, no identity map, no
//! cycle detection.
//!
//! ### 3. Verification is a precondition, not a mode
//!
//! An [`image::Image`] cannot be constructed except through [`verify::admit`].
//! By the time the interpreter sees one, the following have been *proved*, not
//! assumed:
//!
//! - the operand stack depth at every instruction is single-valued and within
//!   the function's declared `maxstack`;
//! - every operand has a type the instruction accepts;
//! - no [`value::Tag::Pending`] is live at a return or snapshot boundary;
//! - every branch target is a real instruction in the same function;
//! - every cycle in the control-flow graph contains a metering instruction;
//! - `fork`/`commit`/`abort` nesting agrees on all paths.
//!
//! So [`interp`] contains no stack-depth checks, no `pc` bounds checks, and no
//! operand-count checks. Those are not omissions — re-checking a verified
//! property in the dispatch loop would cost throughput and hide verifier bugs by
//! turning them into runtime errors.
//!
//! ## Layout
//!
//! | Module | Role |
//! |---|---|
//! | [`isa`] | Opcode table, encoding, stack effects, type rules. Everything else derives from it. |
//! | [`value`] | Tagged operands and arena handles. |
//! | [`diag`] | Spans, stable error codes, rendered diagnostics. |
//! | [`lex`], [`asm`] | `.cdx` front end. |
//! | [`image`] | `.cdxb` container: sections, sealing, tables. |
//! | [`cfg`], [`verify`] | Basic blocks, dominators, the abstract interpreter. |
//! | [`frame`], [`heap`], [`ctx`], [`budget`] | Machine state. |
//! | [`interp`], [`trap`] | Dispatch loop and the host boundary. |
//! | [`cont`], [`journal`], [`replay`] | Durability and time travel. |
//! | [`disas`], [`wire`] | Disassembly and the supervisor protocol. |
//!
//! ## Example
//!
//! ```no_run
//! use cindervm::{asm, verify, interp, trap};
//!
//! let object = asm::assemble("main.cdx", include_str!("../examples/echo.cdx"))?;
//! let image = verify::admit(object)?;                 // proves the invariants
//! let mut vm = interp::Vm::new(&image, interp::Limits::default());
//!
//! let mut answer = None;
//! let exit = loop {
//!     match vm.step(answer.take())? {
//!         trap::Step::Halted(code) => break code,
//!         trap::Step::Trap(t) => answer = Some(host_perform(&t)?),
//!     }
//! };
//! # let _ = exit;
//! # fn host_perform(_: &trap::Trap) -> cindervm::Result<trap::Answer> { unimplemented!() }
//! # Ok::<(), cindervm::Diag>(())
//! ```
//!
//! ## What this crate is not
//!
//! It is not a language. `.cdx` is a macro assembler for tests and examples; the
//! intended production path is [`asm::Builder`] driven by a higher-level
//! frontend. It is also not a tool runtime — tool execution lives in the Go
//! supervisor, on the other side of [`wire`], in a different process.

#![deny(unsafe_code)]
#![warn(missing_docs, unreachable_pub, elided_lifetimes_in_paths)]
#![warn(clippy::pedantic)]
#![allow(clippy::module_name_repetitions, clippy::missing_errors_doc)]
#![allow(clippy::missing_panics_doc, clippy::cast_possible_truncation)]

pub mod asm;
pub mod budget;
pub mod cfg;
pub mod cont;
pub mod ctx;
pub mod diag;
pub mod disas;
pub mod frame;
pub mod hash;
pub mod heap;
pub mod image;
pub mod interp;
pub mod isa;
pub mod journal;
pub mod lex;
pub mod replay;
pub mod trap;
pub mod value;
pub mod verify;

#[cfg(feature = "wire")]
pub mod wire;

pub use diag::{Code, Diag, Result};
pub use image::Image;
pub use interp::{Limits, Vm};
pub use isa::{Op, ISA_VERSION};
pub use trap::{Answer, Step, Trap};
pub use value::{Handle, Tag, Value};

/// Crate version, surfaced in image headers and the `cinder --version` banner.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Assemble, verify, and return a runnable image.
///
/// The common path, and the one that makes the ordering hard to get wrong:
/// there is no way to reach an [`Image`] without passing through
/// [`verify::admit`].
pub fn build(name: &str, source: &str) -> Result<Image> {
    verify::admit(asm::assemble(name, source)?)
}

#[cfg(test)]
mod tests {
    /// The public surface must not leak a way to build an unverified image; if
    /// this ever compiles differently, invariant 3 has been broken.
    #[test]
    fn image_construction_goes_through_verify() {
        let src = "        .isa cdx/4\n        .image \"t\"\n        .fn main() -> i32\n        .maxstack 1\nmain:\n        ldi 0\n        ret\n";
        let img = crate::build("t.cdx", src).expect("minimal image builds");
        assert!(img.is_verified());
    }
}
