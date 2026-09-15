//! `cinder` — the runner and explain tool.

use std::process::ExitCode;

use cindervm::{Code, VERSION};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None | Some("--help") | Some("-h") => {
            println!("cinder {VERSION} — CinderVM runtime\n\nusage:\n  cinder --version\n  cinder explain <CODE>\n  cinder run <image.cdxb>");
            ExitCode::SUCCESS
        }
        Some("--version") => {
            println!("cinder {VERSION}");
            ExitCode::SUCCESS
        }
        Some("explain") => {
            let code = args.get(1).map(String::as_str).unwrap_or("");
            match Code::parse(code) {
                Some(c) => {
                    println!("{}: {} (phase {:?})", c.as_str(), c.blurb(), c.phase());
                    ExitCode::SUCCESS
                }
                None => {
                    eprintln!("unknown code `{code}`; known codes:");
                    for c in Code::all() {
                        eprintln!("  {} — {}", c.as_str(), c.blurb());
                    }
                    ExitCode::FAILURE
                }
            }
        }
        Some("run") => {
            let file = args.get(1).map(String::as_str).unwrap_or("out.cdxb");
            let mut argv = std::env::args();
            argv.next();
            match std::process::Command::new("cinderc")
                .args(["run", file])
                .status()
            {
                Ok(status) => ExitCode::from(status.code().unwrap_or(1) as u8),
                Err(e) => {
                    eprintln!("cinder: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        Some(other) => {
            eprintln!("cinder: unknown command `{other}`");
            ExitCode::FAILURE
        }
    }
}
