//! The `pulse` command-line interface.
//!
//! Commands work on the project in the current directory by convention
//! (`src/App.tsx`, see `pulse_build::Project`); a project folder or an entry
//! file can be passed for the unusual cases.

mod scaffold;

use pulse_build::{BuildOptions, Project};
use pulse_dev::DevOptions;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str = "\
pulse — build web apps in TypeScript + JSX, compiled to WebAssembly

USAGE:
    pulse new <name>       Create a new app in folder <name>
    pulse dev              Start the dev server with hot reload
    pulse build            Build for production into dist/
    pulse serve            Preview the production build
    pulse check            Type-check without building
    pulse emit             Print the generated Rust code

    Commands use the project in the current directory, whose entry is
    src/App.tsx (or src/main.tsx, src/index.tsx). To use another project,
    pass its folder or entry file: `pulse dev path/to/app`.

OPTIONS:
    --port <n>          Port for `dev` and `serve` (default: 3000, next free if taken)
    --out <dir>         Output directory for `build` (default: dist)
    --debug             Unoptimized `build` (faster to compile, larger)

ENVIRONMENT:
    PULSE_RUNTIME_PATH  Path to the pulse-runtime crate (default: bundled with this repo)
";

fn main() -> ExitCode {
    match Args::parse(std::env::args().skip(1)).and_then(run) {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("error: {}", msg.trim_end());
            ExitCode::FAILURE
        }
    }
}

#[derive(Default)]
struct Args {
    command: String,
    path: Option<PathBuf>,
    out_dir: Option<PathBuf>,
    port: Option<u16>,
    debug: bool,
}

impl Args {
    fn parse(mut raw: impl Iterator<Item = String>) -> Result<Args, String> {
        let mut args = Args { command: raw.next().unwrap_or_else(|| "help".into()), ..Args::default() };
        while let Some(arg) = raw.next() {
            let mut value = |flag: &str| raw.next().ok_or_else(|| format!("{flag} needs a value"));
            match arg.as_str() {
                "--out" => args.out_dir = Some(value("--out")?.into()),
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

    fn project(&self) -> Result<Project, String> {
        Project::locate(self.path.as_deref())
    }

    fn port(&self) -> u16 {
        self.port.unwrap_or(3000)
    }
}

fn run(args: Args) -> Result<(), String> {
    match args.command.as_str() {
        "help" | "-h" | "--help" => {
            print!("{USAGE}");
            Ok(())
        }
        "new" => {
            let dir = args.path.ok_or("missing app name, e.g. `pulse new my-app`")?;
            scaffold::create(&dir)?;
            let name = dir.display();
            eprintln!("Created {name}. Next:\n\n    cd {name}\n    pulse dev\n");
            Ok(())
        }
        "dev" => pulse_dev::dev(DevOptions {
            project: args.project()?,
            port: args.port(),
            runtime_path: pulse_build::default_runtime_path(),
        }),
        "build" => {
            let project = args.project()?;
            let mut opts = BuildOptions::new(&project);
            opts.release = !args.debug;
            if let Some(dir) = args.out_dir {
                opts.out_dir = dir;
            }
            eprintln!("Building {} ...", project.entry.display());
            let report = pulse_build::build(&opts)?;
            let kib = |bytes: u64| format!("{:.1} KiB", bytes as f64 / 1024.0);
            let css = report.css_bytes.map(|b| format!(", app.css {}", kib(b))).unwrap_or_default();
            eprintln!("Built {} (app.wasm {}{css}). Preview it with `pulse serve`.", opts.out_dir.display(), kib(report.wasm_bytes));
            Ok(())
        }
        "serve" => pulse_dev::serve(serve_dir(args.path.as_deref())?, args.port()),
        "check" => {
            let project = args.project()?;
            pulse_build::check(&read(&project.entry)?, &file_name(&project.entry))?;
            eprintln!("{}: ok", project.entry.display());
            Ok(())
        }
        "emit" => {
            let entry = args.project()?.entry;
            let rust = pulse_build::compile_to_rust(&read(&entry)?, &file_name(&entry))?;
            print!("{}", pulse_build::format_rust(&rust));
            Ok(())
        }
        other => Err(format!("unknown command `{other}`\n\n{USAGE}")),
    }
}

/// `pulse serve` accepts a build directory, or a project whose `dist/` to
/// serve; without an argument it serves the current project's `dist/`.
fn serve_dir(path: Option<&Path>) -> Result<PathBuf, String> {
    if let Some(dir) = path {
        if dir.join("index.html").is_file() {
            return Ok(dir.to_path_buf());
        }
    }
    Ok(Project::locate(path)?.dist_dir())
}

fn read(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))
}

fn file_name(path: &Path) -> String {
    path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default()
}
