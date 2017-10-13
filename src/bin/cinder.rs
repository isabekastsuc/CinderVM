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
