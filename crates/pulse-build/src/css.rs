//! Stylesheet bundling.
//!
//! Every `import "<spec>.css"` in the app becomes part of one `app.css`, in
//! import order:
//!
//! * `./x.css`, `../x.css` resolve relative to the entry file;
//! * anything else (`bootstrap/dist/css/bootstrap.min.css`) resolves from
//!   `node_modules`, so CSS frameworks installed with npm just work;
//! * a stylesheet that pulls in Tailwind (`@import "tailwindcss"`, or v3's
//!   `@tailwind` directives) is compiled with the project's Tailwind CLI,
//!   which scans the app's `.tsx` files for the classes in use. Pulse's own
//!   directories (cache, output) are excluded from that scan.
//!
//! Plain CSS is copied verbatim; `@import` and `url(...)` references inside it
//! are resolved by the browser relative to `app.css`, so assets they point to
//! belong in `public/`.

use crate::project::Project;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Inputs of a stylesheet build besides the import list.
pub struct CssOptions<'a> {
    /// Holds intermediate files (Tailwind input/output).
    pub work_dir: &'a Path,
    /// Directories Tailwind must not scan for class names.
    pub exclude: &'a [&'a Path],
    pub minify: bool,
}

/// Builds the app stylesheet. Returns `None` when the app imports no CSS.
pub fn bundle(project: &Project, imports: &[String], opts: &CssOptions) -> Result<Option<String>, String> {
    if imports.is_empty() {
        return Ok(None);
    }
    let mut out = String::new();
    for (i, spec) in imports.iter().enumerate() {
        let path = resolve(project, spec)?;
        let source = fs::read_to_string(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let css = if uses_tailwind(&source) { run_tailwind(project, &path, i, opts)? } else { source };
        if !opts.minify {
            out.push_str(&format!("/* {spec} */\n"));
        }
        out.push_str(css.trim_end());
        out.push('\n');
    }
    Ok(Some(out))
}

/// Maps an import specifier to a file.
fn resolve(project: &Project, spec: &str) -> Result<PathBuf, String> {
    if spec.starts_with("./") || spec.starts_with("../") {
        let path = project.entry_dir().join(spec);
        return if path.is_file() {
            Ok(path)
        } else {
            Err(format!("stylesheet `{spec}` not found (looked for {})", path.display()))
        };
    }
    project.find_in_node_modules(spec).filter(|p| p.is_file()).ok_or_else(|| {
        format!(
            "stylesheet `{spec}` not found in node_modules.\nhint: install the package that provides it, e.g. `npm install {}`",
            package_name(spec)
        )
    })
}

/// `@scope/pkg/dist/x.css` → `@scope/pkg`, `pkg/x.css` → `pkg`.
fn package_name(spec: &str) -> String {
    let mut parts = spec.split('/');
    match (parts.next(), parts.next()) {
        (Some(scope), Some(name)) if scope.starts_with('@') => format!("{scope}/{name}"),
        (Some(name), _) => name.to_string(),
        _ => spec.to_string(),
    }
}

/// True for stylesheets that need the Tailwind compiler.
pub fn uses_tailwind(css: &str) -> bool {
    css.lines().map(str::trim_start).any(|line| {
        line.starts_with("@import \"tailwindcss")
            || line.starts_with("@import 'tailwindcss")
            || line.starts_with("@tailwind ")
    })
}

fn run_tailwind(project: &Project, stylesheet: &Path, index: usize, opts: &CssOptions) -> Result<String, String> {
    let bin = project.find_in_node_modules(".bin/tailwindcss").ok_or_else(|| {
        format!(
            "{} uses Tailwind, but the Tailwind CLI is not installed.\n\
             hint: npm install -D tailwindcss @tailwindcss/cli",
            stylesheet.display()
        )
    })?;
    fs::create_dir_all(opts.work_dir).map_err(|e| format!("cannot create {}: {e}", opts.work_dir.display()))?;
    // A wrapper around the user's stylesheet that keeps Tailwind out of
    // Pulse's own directories (generated code, build output).
    let mut wrapper = format!("@import {};\n", css_string(&abs(stylesheet)));
    for dir in opts.exclude {
        wrapper.push_str(&format!("@source not {};\n", css_string(&abs(dir))));
    }
    let input = opts.work_dir.join(format!("tailwind-{index}.input.css"));
    let output = opts.work_dir.join(format!("tailwind-{index}.css"));
    fs::write(&input, wrapper).map_err(|e| format!("cannot write {}: {e}", input.display()))?;

    let mut cmd = Command::new(&bin);
    cmd.arg("--input").arg(abs(&input)).arg("--output").arg(abs(&output));
    if opts.minify {
        cmd.arg("--minify");
    }
    // Tailwind discovers class names in the files under its working
    // directory, so run it from the project root.
    let result = cmd.current_dir(&project.root).output().map_err(|e| format!("failed to run {}: {e}", bin.display()))?;
    if !result.status.success() {
        return Err(format!(
            "Tailwind failed on {}:\n{}",
            stylesheet.display(),
            String::from_utf8_lossy(&result.stderr).trim_end()
        ));
    }
    fs::read_to_string(&output).map_err(|e| format!("cannot read Tailwind output {}: {e}", output.display()))
}

/// A path as a CSS string literal (forward slashes, quotes escaped).
fn css_string(path: &Path) -> String {
    format!("\"{}\"", path.to_string_lossy().replace('\\', "/").replace('"', "\\\""))
}

fn abs(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(root: &Path) -> CssOptions<'_> {
        CssOptions { work_dir: root, exclude: &[], minify: false }
    }

    fn temp_project(name: &str) -> (PathBuf, Project) {
        let root = std::env::temp_dir().join(format!("pulse-css-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/App.tsx"), "").unwrap();
        let project = Project::new(root.join("src/App.tsx"));
        (root, project)
    }

    #[test]
    fn bundles_relative_and_node_modules_imports_in_order() {
        let (root, project) = temp_project("bundle");
        fs::write(root.join("src/app.css"), "h1 { color: red; }\n").unwrap();
        fs::create_dir_all(root.join("node_modules/@acme/ui/dist")).unwrap();
        fs::write(root.join("node_modules/@acme/ui/dist/ui.css"), ".btn { padding: 0; }").unwrap();

        let imports = vec!["@acme/ui/dist/ui.css".to_string(), "./app.css".to_string()];
        let css = bundle(&project, &imports, &opts(&root)).unwrap().unwrap();
        assert_eq!(css, "/* @acme/ui/dist/ui.css */\n.btn { padding: 0; }\n/* ./app.css */\nh1 { color: red; }\n");
        assert_eq!(bundle(&project, &[], &opts(&root)).unwrap(), None);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn missing_files_explain_how_to_fix() {
        let (root, project) = temp_project("missing");
        let err = bundle(&project, &["@scope/pkg/x.css".into()], &opts(&root)).unwrap_err();
        assert!(err.contains("npm install @scope/pkg"), "{err}");
        let err = bundle(&project, &["./nope.css".into()], &opts(&root)).unwrap_err();
        assert!(err.contains("stylesheet `./nope.css` not found"), "{err}");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn detects_tailwind() {
        assert!(uses_tailwind("@import \"tailwindcss\";\n@theme { }"));
        assert!(uses_tailwind("  @import 'tailwindcss' source(none);"));
        assert!(uses_tailwind("@tailwind base;\n@tailwind utilities;"));
        assert!(!uses_tailwind("/* tailwind-like */ .a { }"));
    }

    #[test]
    fn tailwind_without_cli_has_install_hint() {
        let (root, project) = temp_project("tw-missing");
        fs::write(root.join("src/styles.css"), "@import \"tailwindcss\";").unwrap();
        // Only meaningful when no Tailwind CLI is reachable from the temp dir.
        if project.find_in_node_modules(".bin/tailwindcss").is_none() {
            let err = bundle(&project, &["./styles.css".into()], &opts(&root)).unwrap_err();
            assert!(err.contains("npm install -D tailwindcss @tailwindcss/cli"), "{err}");
        }
        fs::remove_dir_all(root).unwrap();
    }
}
