//! # pulse-build
//!
//! Turns a Pulse app into a deployable bundle of static files:
//!
//! ```text
//! src/App.tsx ─ pulse-compiler ─► Rust ─ cargo (wasm32) ─► app.wasm ┐
//! imported .css ─ (Tailwind) ──────────────────────────► app.css  ├─► out dir
//! index.html, public/, loader ─────────────────────────► ...      ┘
//! ```
//!
//! Stylesheets are processed in parallel with the cargo build.

mod assets;
mod cargo;
mod css;
mod project;

pub use assets::{insert_before, CSS_LINK, DEFAULT_INDEX_HTML, LOADER_JS};
pub use cargo::{format_rust, write_app_crate};
pub use project::Project;
pub use pulse_compiler::check;

use pulse_compiler::CodegenOptions;
use std::fs;
use std::path::{Path, PathBuf};

pub struct BuildOptions {
    pub entry: PathBuf,
    pub out_dir: PathBuf,
    /// Where the generated crate, its `target/` directory and other
    /// intermediate files live.
    pub cache_dir: PathBuf,
    /// Path to the `pulse-runtime` crate.
    pub runtime_path: PathBuf,
    /// Optimized WebAssembly and minified CSS.
    pub release: bool,
    /// Development build: include hot-reload hooks.
    pub hot: bool,
}

impl BuildOptions {
    /// Defaults: output to `dist/`, cache in `.pulse/`, runtime from
    /// `$PULSE_RUNTIME_PATH` or from this repository.
    pub fn new(entry: PathBuf) -> Self {
        BuildOptions {
            entry,
            out_dir: PathBuf::from("dist"),
            cache_dir: PathBuf::from(".pulse"),
            runtime_path: default_runtime_path(),
            release: true,
            hot: false,
        }
    }
}

pub fn default_runtime_path() -> PathBuf {
    std::env::var_os("PULSE_RUNTIME_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../pulse-runtime"))
}

/// What a successful build produced.
pub struct BuildReport {
    pub wasm_path: PathBuf,
    pub wasm_bytes: u64,
    /// Size of `app.css`, if the app has styles.
    pub css_bytes: Option<u64>,
}

/// Compiles TSX source to Rust source (production settings).
pub fn compile_to_rust(source: &str, file_name: &str) -> Result<String, String> {
    let opts = CodegenOptions { source_name: file_name.to_string(), ..Default::default() };
    pulse_compiler::compile(source, &opts).map(|c| c.rust)
}

/// Runs the full pipeline and writes the bundle to the output directory.
pub fn build(opts: &BuildOptions) -> Result<BuildReport, String> {
    let project = Project::new(&opts.entry);
    if opts.entry.extension().and_then(|e| e.to_str()) != Some("tsx") {
        return Err(format!(
            "{}: Pulse apps are written in TypeScript; the entry file must be a `.tsx` file",
            opts.entry.display()
        ));
    }
    let source = fs::read_to_string(&opts.entry).map_err(|e| format!("cannot read {}: {e}", opts.entry.display()))?;
    let file_name = opts.entry.file_name().unwrap_or_default().to_string_lossy().into_owned();
    let codegen = CodegenOptions { source_name: file_name, hot: opts.hot, ..Default::default() };
    let compiled = pulse_compiler::compile(&source, &codegen)?;

    let crate_dir = opts.cache_dir.join("app");
    cargo::write_app_crate(&crate_dir, &cargo::format_rust(&compiled.rust), &opts.runtime_path)?;

    // CSS tools (Tailwind) and cargo are independent; run them side by side.
    let css_work_dir = opts.cache_dir.join("css");
    let css_opts = css::CssOptions {
        work_dir: &css_work_dir,
        exclude: &[&opts.cache_dir, &opts.out_dir],
        minify: opts.release && !opts.hot,
    };
    let (wasm, css) = std::thread::scope(|s| {
        let css = s.spawn(|| css::bundle(&project, &compiled.stylesheets, &css_opts));
        let wasm = cargo::build_wasm(&crate_dir, &opts.cache_dir.join("target"), opts.release);
        (wasm, css.join().unwrap_or_else(|_| Err("stylesheet bundling panicked".into())))
    });
    let (built_wasm, css) = (wasm?, css?);

    let out = &opts.out_dir;
    fs::create_dir_all(out).map_err(|e| format!("cannot create {}: {e}", out.display()))?;
    assets::copy_public(&project, out)?;
    let css_path = out.join("app.css");
    let css_bytes = match &css {
        Some(text) => {
            write_atomic(&css_path, text.as_bytes())?;
            Some(text.len() as u64)
        }
        None => {
            let _ = fs::remove_file(&css_path);
            None
        }
    };
    let wasm_bytes = fs::read(&built_wasm).map_err(|e| format!("cannot read {}: {e}", built_wasm.display()))?;
    let wasm_path = out.join("app.wasm");
    write_atomic(&wasm_path, &wasm_bytes)?;
    write_atomic(&out.join("pulse.js"), LOADER_JS.as_bytes())?;
    write_atomic(&out.join("index.html"), assets::index_html(&project, css.is_some())?.as_bytes())?;

    Ok(BuildReport { wasm_path, wasm_bytes: wasm_bytes.len() as u64, css_bytes })
}

/// Writes via a temporary file and a rename, so a server never sees a
/// half-written file.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes).map_err(|e| format!("cannot write {}: {e}", tmp.display()))?;
    fs::rename(&tmp, path).map_err(|e| format!("cannot write {}: {e}", path.display()))
}
