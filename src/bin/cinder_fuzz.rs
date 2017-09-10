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
        isa::Op::Lt,
        isa::Op::Not,
        isa::Op::Brz,
        isa::Op::Brnz,
        isa::Op::Ret,
        isa::Op::Halt,
        isa::Op::Reserve,
        isa::Op::Release,
        isa::Op::Pack,
        isa::Op::Len,
        isa::Op::CtxCost,
        isa::Op::Nop,
    ];
    let op = ops[(next_u64(state) % ops.len() as u64) as usize];
    let a = (next_u64(state) & 0xFF) as u8;
    let b = (next_u64(state) & 0x7FFF) as u32;
    isa::encode(out, op, a, b);
}

fn main() -> ExitCode {
    println!("cinder-fuzz — {} iterations", ITERATIONS);
    let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut accepted = 0u64;
    let mut rejected = 0u64;
    let mut crashed = 0u64;
    for _ in 0..ITERATIONS {
        let mut code = Vec::new();
