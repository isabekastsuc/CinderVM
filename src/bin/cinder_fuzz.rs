//! `cinder-fuzz` — a tiny structured fuzzer for the verifier and interpreter.

use std::process::ExitCode;

use cindervm::{asm, interp, isa, trap, verify, Value};

const ITERATIONS: usize = 200_000;

fn next_u64(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

fn random_insn(state: &mut u64, out: &mut Vec<u8>) {
    let ops = [
        isa::Op::Ldi,
        isa::Op::Ldc,
        isa::Op::Dup,
        isa::Op::Drop,
        isa::Op::Swap,
        isa::Op::Add,
        isa::Op::Sub,
        isa::Op::Mul,
        isa::Op::Div,
        isa::Op::Eq,
