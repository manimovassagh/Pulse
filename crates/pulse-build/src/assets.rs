//! Non-code output: the HTML page, the JavaScript loader and `public/` files.

use crate::project::Project;
use std::fs;
use std::path::Path;

/// The browser loader, embedded so the CLI is a single self-contained binary.
pub const LOADER_JS: &str = include_str!("../../../runtime/pulse.js");

/// Files Pulse writes itself; `public/` must not contain them.
pub const GENERATED_FILES: &[&str] = &["index.html", "app.wasm", "app.css", "pulse.js"];

/// Attribute marking the stylesheet link Pulse manages (the dev client uses
/// it to hot-swap CSS).
pub const CSS_LINK: &str = r#"<link rel="stylesheet" href="app.css" data-pulse-css />"#;

pub const DEFAULT_INDEX_HTML: &str = r#"<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>Pulse app</title>
  </head>
  <body>
    <div id="app"></div>
    <script type="module">
      import { start } from "./pulse.js";
      start("./app.wasm");
    </script>
  </body>
</html>
"#;

/// The page to serve: the app's `index.html` (or the default one), with the
/// stylesheet link added when the app has CSS.
pub fn index_html(project: &Project, has_css: bool) -> Result<String, String> {
    let html = match project.index_html() {
        Some(path) => fs::read_to_string(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?,
        None => DEFAULT_INDEX_HTML.to_string(),
    };
    Ok(if has_css { insert_before(&html, "</head>", CSS_LINK) } else { html })
}

/// Inserts `tag` before the last `marker` (or appends it).
pub fn insert_before(html: &str, marker: &str, tag: &str) -> String {
    match html.rfind(marker) {
        Some(i) => format!("{}  {tag}\n  {}", &html[..i], &html[i..]),
        None => format!("{html}\n{tag}\n"),
    }
}

/// Copies `public/` into the output directory, recursively and verbatim.
pub fn copy_public(project: &Project, out_dir: &Path) -> Result<(), String> {
    let public = project.public_dir();
    if !public.is_dir() {
        return Ok(());
    }
    for name in GENERATED_FILES {
        if public.join(name).exists() {
            return Err(format!("public/{name} conflicts with a file Pulse generates; rename it"));
        }
    }
    copy_dir(&public, out_dir)
}

fn copy_dir(from: &Path, to: &Path) -> Result<(), String> {
    fs::create_dir_all(to).map_err(|e| format!("cannot create {}: {e}", to.display()))?;
    let entries = fs::read_dir(from).map_err(|e| format!("cannot read {}: {e}", from.display()))?;
    for entry in entries.flatten() {
        let src = entry.path();
        let dst = to.join(entry.file_name());
        if src.is_dir() {
            copy_dir(&src, &dst)?;
        } else {
            fs::copy(&src, &dst).map_err(|e| format!("cannot copy {}: {e}", src.display()))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn css_link_goes_into_head() {
        let project = Project::new("/nonexistent/src/App.tsx");
        let html = index_html(&project, true).unwrap();
        let link = html.find(CSS_LINK).expect("link");
        assert!(link < html.find("</head>").unwrap());
        assert!(!index_html(&project, false).unwrap().contains("app.css"));
    }

    #[test]
    fn public_files_are_copied_and_conflicts_rejected() {
        let root = std::env::temp_dir().join(format!("pulse-assets-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("public/img")).unwrap();
        fs::write(root.join("public/img/logo.svg"), "<svg/>").unwrap();
        let project = Project::new(root.join("src/App.tsx"));
        copy_public(&project, &root.join("out")).unwrap();
        assert_eq!(fs::read_to_string(root.join("out/img/logo.svg")).unwrap(), "<svg/>");

        fs::write(root.join("public/app.css"), "").unwrap();
        assert!(copy_public(&project, &root.join("out")).unwrap_err().contains("conflicts"));
        fs::remove_dir_all(root).unwrap();
    }
}
