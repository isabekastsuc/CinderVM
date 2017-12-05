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
                .unwrap_or_else(|| src.replace(".cdx", ".cdxb"));
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
