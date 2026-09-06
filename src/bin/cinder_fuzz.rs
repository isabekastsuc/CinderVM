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
        let n = (next_u64(&mut state) % 24 + 1) as usize;
        for _ in 0..n {
            random_insn(&mut state, &mut code);
        }
        let src = format!(
            ".fn main() -> i32\n.maxstack 8\nmain:\n    ldi 0\n{}\n    halt\n",
            code.chunks_exact(4)
                .map(|c| isa::decode(c, 0)
                    .map(|d| d.op.mnemonic().to_string())
                    .unwrap_or_else(|| "nop".into()))
                .map(|m| format!("    {m}"))
                .collect::<Vec<_>>()
                .join("\n"),
        );
        match verify::admit(asm::assemble("fuzz.cdx", &src).unwrap_or_default()) {
            Err(_) => {
                rejected += 1;
            }
            Ok(image) => {
                accepted += 1;
                let mut vm = interp::Vm::new(
                    &image,
                    interp::Limits {
                        max_steps: 1_000,
                        max_frames: 16,
                        max_arena: 1 << 20,
                    },
                );
                loop {
                    match vm.step(None) {
                        Ok(trap::Step::Halted(_)) => break,
                        Ok(trap::Step::Trap(_)) => {
                            if vm.step(Some(trap::Answer::Value(Value::int(0)))).is_err() {
                                crashed += 1;
                                break;
                            }
                        }
                        Err(_) => {
                            crashed += 1;
                            break;
                        }
                    }
                }
            }
        }
    }
    println!("accepted={accepted} rejected={rejected} crashes={crashed}");
    if crashed > 0 {
        println!("FINDINGS: {crashed} inputs produced interpreter errors");
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}
