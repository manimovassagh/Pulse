//! The `pulse` command-line interface.

use pulse_build::BuildOptions;
use pulse_dev::DevOptions;
use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "\
pulse — build web apps in TypeScript + JSX, compiled to WebAssembly

USAGE:
    pulse dev   <entry.tsx> [--port <n>]       Develop with hot reload
    pulse build <entry.tsx> [--out <dir>]      Production build
    pulse serve [dir] [--port <n>]             Preview a build (default dir: dist)
    pulse check <entry.tsx>                    Type-check only
    pulse emit  <entry.tsx>                    Print the generated Rust

OPTIONS:
    --out <dir>         Output directory for `build` (default: dist)
    --port <n>          Port for `dev`/`serve` (default: 3000, next free if taken)
    --cache-dir <dir>   Where generated code is compiled (default: .pulse)
    --debug             `build` unoptimized WebAssembly

ENVIRONMENT:
    PULSE_RUNTIME_PATH  Path to the pulse-runtime crate (default: bundled with this repo)
";

fn main() -> ExitCode {
    match Args::parse(std::env::args().skip(1)).and_then(run) {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("{}", msg.trim_end());
            ExitCode::FAILURE
        }
    }
}

#[derive(Default)]
struct Args {
    command: String,
    path: Option<PathBuf>,
    out_dir: Option<PathBuf>,
    cache_dir: Option<PathBuf>,
    port: Option<u16>,
    debug: bool,
}

impl Args {
    fn parse(mut raw: impl Iterator<Item = String>) -> Result<Args, String> {
        let mut args = Args { command: raw.next().ok_or(USAGE)?, ..Args::default() };
        while let Some(arg) = raw.next() {
            let mut value = |flag: &str| raw.next().ok_or_else(|| format!("{flag} needs a value"));
            match arg.as_str() {
                "--out" => args.out_dir = Some(value("--out")?.into()),
                "--cache-dir" => args.cache_dir = Some(value("--cache-dir")?.into()),
                "--port" => {
                    let v = value("--port")?;
                    args.port = Some(v.parse().map_err(|_| format!("invalid port `{v}`"))?);
                }
                "--debug" => args.debug = true,
                "-h" | "--help" => args.command = "help".into(),
                flag if flag.starts_with('-') => return Err(format!("unknown option `{flag}`\n\n{USAGE}")),
                path if args.path.is_none() => args.path = Some(path.into()),
                extra => return Err(format!("unexpected argument `{extra}`")),
            }
        }
        Ok(args)
    }

    fn entry(&self) -> Result<PathBuf, String> {
        self.path.clone().ok_or_else(|| format!("missing entry file, e.g. `pulse {} src/App.tsx`", self.command))
    }

    fn cache_dir(&self) -> PathBuf {
        self.cache_dir.clone().unwrap_or_else(|| PathBuf::from(".pulse"))
    }
}

fn run(args: Args) -> Result<(), String> {
    match args.command.as_str() {
        "help" | "-h" | "--help" => {
            print!("{USAGE}");
            Ok(())
        }
        "dev" => pulse_dev::dev(DevOptions {
            entry: args.entry()?,
            port: args.port.unwrap_or(3000),
            cache_dir: args.cache_dir(),
            runtime_path: pulse_build::default_runtime_path(),
        }),
        "build" => {
            let entry = args.entry()?;
            let mut opts = BuildOptions::new(entry.clone());
            opts.release = !args.debug;
            opts.cache_dir = args.cache_dir();
            if let Some(dir) = args.out_dir {
                opts.out_dir = dir;
            }
            eprintln!("Compiling {} ...", entry.display());
            let report = pulse_build::build(&opts)?;
            let kib = |bytes: u64| format!("{:.1} KiB", bytes as f64 / 1024.0);
            let css = report.css_bytes.map(|b| format!(", app.css {}", kib(b))).unwrap_or_default();
            eprintln!(
                "Built {} (app.wasm {}{css}). Preview with `pulse serve {}`.",
                opts.out_dir.display(),
                kib(report.wasm_bytes),
                opts.out_dir.display()
            );
            Ok(())
        }
        "serve" => pulse_dev::serve(args.path.unwrap_or_else(|| PathBuf::from("dist")), args.port.unwrap_or(3000)),
        "check" => {
            let entry = args.entry()?;
            pulse_build::check(&read(&entry)?, &file_name(&entry))?;
            println!("{}: ok", entry.display());
            Ok(())
        }
        "emit" => {
            let entry = args.entry()?;
            print!("{}", pulse_build::format_rust(&pulse_build::compile_to_rust(&read(&entry)?, &file_name(&entry))?));
            Ok(())
        }
        other => Err(format!("unknown command `{other}`\n\n{USAGE}")),
    }
}

fn read(path: &PathBuf) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))
}

fn file_name(path: &std::path::Path) -> String {
    path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default()
}
