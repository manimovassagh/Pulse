//! The `pulse` command-line interface.

use pulse_compiler::driver::{self, BuildOptions};
use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "\
pulse — compile TypeScript/JSX apps to WebAssembly

USAGE:
    pulse build <entry.tsx> [--out <dir>] [--cache-dir <dir>] [--debug]
    pulse check <entry.tsx>
    pulse emit  <entry.tsx>

COMMANDS:
    build   Compile the app into <dir> (default: dist): index.html, app.wasm, pulse.js
    check   Parse and type-check only
    emit    Print the generated Rust code

OPTIONS:
    --out <dir>         Output directory (default: dist)
    --cache-dir <dir>   Where the generated crate is built (default: .pulse)
    --debug             Build unoptimized WebAssembly (faster compile)

ENVIRONMENT:
    PULSE_RUNTIME_PATH  Path to the pulse-runtime crate (default: bundled with this repo)
";

fn main() -> ExitCode {
    match run(std::env::args().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("{}", msg.trim_end());
            ExitCode::FAILURE
        }
    }
}

fn run(args: Vec<String>) -> Result<(), String> {
    let mut args = args.into_iter();
    let command = args.next().ok_or_else(|| USAGE.to_string())?;
    if matches!(command.as_str(), "-h" | "--help" | "help") {
        print!("{USAGE}");
        return Ok(());
    }

    let mut entry: Option<PathBuf> = None;
    let mut out_dir = None;
    let mut cache_dir = None;
    let mut release = true;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--out" => out_dir = Some(PathBuf::from(args.next().ok_or("--out needs a directory")?)),
            "--cache-dir" => cache_dir = Some(PathBuf::from(args.next().ok_or("--cache-dir needs a directory")?)),
            "--debug" => release = false,
            flag if flag.starts_with('-') => return Err(format!("unknown option `{flag}`\n\n{USAGE}")),
            path if entry.is_none() => entry = Some(PathBuf::from(path)),
            extra => return Err(format!("unexpected argument `{extra}`")),
        }
    }
    let entry = entry.ok_or_else(|| format!("missing entry file\n\n{USAGE}"))?;
    let read = || std::fs::read_to_string(&entry).map_err(|e| format!("cannot read {}: {e}", entry.display()));
    let file_name = entry.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();

    match command.as_str() {
        "check" => {
            driver::check(&read()?, &file_name)?;
            println!("{}: ok", entry.display());
            Ok(())
        }
        "emit" => {
            print!("{}", driver::format_rust(&driver::compile_to_rust(&read()?, &file_name)?));
            Ok(())
        }
        "build" => {
            let mut opts = BuildOptions::new(entry.clone());
            opts.release = release;
            if let Some(d) = out_dir {
                opts.out_dir = d;
            }
            if let Some(d) = cache_dir {
                opts.cache_dir = d;
            }
            eprintln!("Compiling {} ...", entry.display());
            let report = driver::build(&opts)?;
            eprintln!(
                "Built {} ({:.1} KiB). Serve `{}` with any static file server.",
                report.wasm_path.display(),
                report.wasm_bytes as f64 / 1024.0,
                opts.out_dir.display()
            );
            Ok(())
        }
        other => Err(format!("unknown command `{other}`\n\n{USAGE}")),
    }
}
