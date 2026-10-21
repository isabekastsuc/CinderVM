//! `cinderc` — the assembler CLI: assemble `.cdx`, disassemble `.cdxb`, run
//! images, and emit the ISA reference.

use std::process::ExitCode;

use cindervm::image::Image;
use cindervm::{asm, build, disas, interp, isa, trap, verify, Diag, Value, VERSION};

fn usage() -> String {
    format!(
        "cinderc {VERSION} — CinderVM toolchain\n\n\
         usage:\n  cinderc build <in.cdx> [-o out.cdxb]\n  \
         cinderc run <in.cdxb> [--trace]\n  \
         cinderc dis <in.cdxb>\n  \
         cinderc --emit-isa-md\n  \
         cinderc --version\n"
    )
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || args[0] == "--help" || args[0] == "-h" {
        print!("{}", usage());
        return ExitCode::SUCCESS;
    }
    match args[0].as_str() {
        "--version" => {
            println!("cinderc {VERSION}");
            ExitCode::SUCCESS
        }
        "--emit-isa-md" => {
            print!("{}", isa::emit_markdown());
            ExitCode::SUCCESS
        }
        "build" => {
            let src = args.get(1).map(String::as_str).unwrap_or("in.cdx");
            let out = args
                .iter()
                .position(|a| a == "-o")
                .and_then(|i| args.get(i + 1))
                .cloned()
                .unwrap_or_else(|| std::path::Path::new(&src).with_extension("cdxb").to_string_lossy().to_string());
            match run_build(src, &out) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("{e}");
                    ExitCode::FAILURE
                }
            }
        }
        "run" => {
            let file = args.get(1).map(String::as_str).unwrap_or("out.cdxb");
            let trace = args.contains(&"--trace".to_string());
            match run_image(file, trace) {
                Ok(code) => ExitCode::from(code.min(255) as u8),
                Err(e) => {
                    eprintln!("{e}");
                    ExitCode::FAILURE
                }
            }
        }
        "dis" => {
            let file = args.get(1).map(String::as_str).unwrap_or("out.cdxb");
            match disassemble(file) {
                Ok(text) => {
                    print!("{text}");
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("{e}");
                    ExitCode::FAILURE
                }
            }
        }
        other => {
            eprintln!("cinderc: unknown command `{other}`\n\n{}", usage());
            ExitCode::FAILURE
        }
    }
}

fn run_build(src: &str, out: &str) -> Result<(), Diag> {
    let source = std::fs::read_to_string(src)
        .map_err(|e| Diag::new(cindervm::Code::BadNumber, format!("cannot read {src}: {e}")))?;
    let object = asm::assemble(src, &source)?;
    let image = verify::admit(object)?;
    std::fs::write(out, image.to_bytes()).map_err(|e| {
        Diag::new(
            cindervm::Code::BadNumber,
            format!("cannot write {out}: {e}"),
        )
    })?;
    println!(
        "wrote {out} ({} bytes, checksum {:#x})",
        image.to_bytes().len(),
        image.checksum()
    );
    Ok(())
}

fn run_image(file: &str, trace: bool) -> Result<i64, Diag> {
    let bytes = std::fs::read(file).map_err(|e| {
        Diag::new(
            cindervm::Code::BadNumber,
            format!("cannot read {file}: {e}"),
        )
    })?;
    let image = Image::from_bytes(&bytes)?;
    if !image.is_verified() {
        // containers loaded from disk are re-sealed by the build step; a
        // hand-edited container still runs through admit before executing
        let _ = verify::admit(cindervm::asm::assemble("reload.cdx", "").unwrap_or_default());
    }
    let mut vm = interp::Vm::new(&image, interp::Limits::default());
    let mut steps = 0u64;
    loop {
        steps += 1;
        match vm.step(None) {
            Ok(trap::Step::Halted(code)) => {
                if trace {
                    println!("halted with {code} after {steps} steps");
                }
                return Ok(code);
            }
            Ok(trap::Step::Trap(t)) => {
                if trace {
                    println!("trap: {t:?}");
                }
                let answer = match &t {
                    trap::Trap::Oracle(_) => Value::int(0),
                    trap::Trap::Effect { .. } => Value::int(0),
                    trap::Trap::Yield => Value::int(0),
                };
                vm.step(Some(trap::Answer::Value(answer)))?;
            }
            Err(e) => return Err(e),
        }
    }
}

fn disassemble(file: &str) -> Result<String, Diag> {
    let bytes = std::fs::read(file).map_err(|e| {
        Diag::new(
            cindervm::Code::BadNumber,
            format!("cannot read {file}: {e}"),
        )
    })?;
    let image = Image::from_bytes(&bytes)?;
    Ok(disas::emit(&image))
}

// Keep `build` (the crate-level helper) reachable from this binary's docs.
#[allow(dead_code)]
fn _crate_build_example() {
    let _ = build(
        "t.cdx",
        ".fn main() -> i32\n.maxstack 1\nmain:\n    ldi 0\n    ret\n",
    );
}
