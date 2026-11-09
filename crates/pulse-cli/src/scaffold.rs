//! `pulse new`: creates a project with the conventional layout.
//!
//! The template lives in `crates/pulse-cli/templates/app` as ordinary files
//! and is embedded into the binary. `{{name}}` is replaced with the folder
//! name.

use std::fs;
use std::path::Path;

/// `(path in the new project, contents)`.
const TEMPLATE: &[(&str, &str)] = &[
    ("src/App.tsx", include_str!("../templates/app/src/App.tsx")),
    ("src/styles.css", include_str!("../templates/app/src/styles.css")),
    ("public/logo.svg", include_str!("../templates/app/public/logo.svg")),
    ("index.html", include_str!("../templates/app/index.html")),
    ("tsconfig.json", include_str!("../templates/app/tsconfig.json")),
    ("README.md", include_str!("../templates/app/README.md")),
    // Stored without the leading dot so tooling does not treat the template
    // folder itself as ignored.
    (".gitignore", include_str!("../templates/app/gitignore")),
    ("types/pulse.d.ts", include_str!("../../../types/pulse.d.ts")),
    ("types/assets.d.ts", include_str!("../../../types/assets.d.ts")),
];

/// Creates a new app in `dir`, which must not exist or be empty.
pub fn create(dir: &Path) -> Result<(), String> {
    if dir.exists() && fs::read_dir(dir).map(|mut d| d.next().is_some()).unwrap_or(true) {
        return Err(format!("{} already exists and is not empty", dir.display()));
    }
    let name = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|n| !n.is_empty() && n != "." && n != "..")
        .ok_or_else(|| format!("{} is not a valid app name", dir.display()))?;
    for (path, contents) in TEMPLATE {
        let target = dir.join(path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
        }
        fs::write(&target, contents.replace("{{name}}", &name))
            .map_err(|e| format!("cannot write {}: {e}", target.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_conventional_layout_and_refuses_to_overwrite() {
        let root = std::env::temp_dir().join(format!("pulse-new-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let app = root.join("my-app");
        create(&app).unwrap();

        for (path, _) in TEMPLATE {
            assert!(app.join(path).is_file(), "{path} missing");
        }
        let html = fs::read_to_string(app.join("index.html")).unwrap();
        assert!(html.contains("<title>my-app</title>"), "{html}");
        assert_eq!(pulse_build::Project::locate(Some(&app)).unwrap().entry, app.join("src/App.tsx"));

        assert!(create(&app).unwrap_err().contains("not empty"));
        fs::remove_dir_all(root).unwrap();
    }
}
