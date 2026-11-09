//! Where things live in a Pulse app.
//!
//! ```text
//! my-app/              ← project root
//! ├── index.html       ← optional page template
//! ├── public/          ← optional static assets, copied as-is
//! ├── node_modules/    ← optional npm packages (CSS frameworks, Tailwind)
//! └── src/
//!     ├── App.tsx      ← entry
//!     └── styles.css
//! ```
//!
//! The root is the entry's directory, or its parent when the entry sits in
//! a `src/` directory. Commands run without arguments discover the project in
//! the current directory (see [`Project::locate`]); build output (`dist/`)
//! and the build cache (`.pulse/`) live in the project root.

use std::path::{Path, PathBuf};

/// Entry files looked for, in order, when only a directory is given.
pub const ENTRY_CANDIDATES: &[&str] = &["src/App.tsx", "src/main.tsx", "src/index.tsx"];

#[derive(Clone, Debug)]
pub struct Project {
    pub entry: PathBuf,
    pub root: PathBuf,
}

impl Project {
    /// Finds the project for a CLI argument: nothing (the current directory),
    /// a project directory, or an explicit entry file.
    pub fn locate(path: Option<&Path>) -> Result<Self, String> {
        let path = path.unwrap_or(Path::new("."));
        if path.is_file() {
            return Ok(Project::new(path));
        }
        if !path.is_dir() {
            return Err(format!("{} does not exist", path.display()));
        }
        let here = path == Path::new(".");
        ENTRY_CANDIDATES
            .iter()
            // Keep paths short in messages: `src/App.tsx`, not `./src/App.tsx`.
            .map(|candidate| if here { PathBuf::from(candidate) } else { path.join(candidate) })
            .find(|entry| entry.is_file())
            .map(Project::new)
            .ok_or_else(|| {
                let shown = if here { "the current directory".to_string() } else { path.display().to_string() };
                format!(
                    "no Pulse app found in {shown}: expected {}.\n\
                     hint: run the command inside your project, or pass its folder, e.g. `pulse dev my-app`",
                    ENTRY_CANDIDATES.join(", ")
                )
            })
    }

    pub fn new(entry: impl Into<PathBuf>) -> Self {
        let entry = entry.into();
        let entry_dir = parent(&entry);
        let root = if entry_dir.file_name().is_some_and(|n| n == "src") {
            parent(&entry_dir)
        } else {
            entry_dir
        };
        Project { entry, root }
    }

    /// Directory containing the entry file; relative imports start here.
    pub fn entry_dir(&self) -> PathBuf {
        parent(&self.entry)
    }

    /// The app's own page template, if it has one.
    pub fn index_html(&self) -> Option<PathBuf> {
        [self.root.join("index.html"), self.entry_dir().join("index.html")].into_iter().find(|p| p.is_file())
    }

    pub fn public_dir(&self) -> PathBuf {
        self.root.join("public")
    }

    /// Default output directory of production builds.
    pub fn dist_dir(&self) -> PathBuf {
        self.root.join("dist")
    }

    /// Generated code, compiled artifacts and other intermediate files.
    pub fn cache_dir(&self) -> PathBuf {
        self.root.join(".pulse")
    }

    /// Finds `relative` inside the nearest `node_modules`, searching from the
    /// project root upwards like Node.js does.
    pub fn find_in_node_modules(&self, relative: &str) -> Option<PathBuf> {
        let start = std::fs::canonicalize(&self.root).unwrap_or_else(|_| self.root.clone());
        start.ancestors().map(|dir| dir.join("node_modules").join(relative)).find(|p| p.exists())
    }
}

/// Parent directory, treating a bare file name as living in `.`.
fn parent(path: &Path) -> PathBuf {
    match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_is_above_src() {
        let p = Project::new("my-app/src/App.tsx");
        assert_eq!(p.root, PathBuf::from("my-app"));
        assert_eq!(p.entry_dir(), PathBuf::from("my-app/src"));
        assert_eq!(p.public_dir(), PathBuf::from("my-app/public"));
    }

    #[test]
    fn locates_projects_by_convention() {
        let root = std::env::temp_dir().join(format!("pulse-locate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("app/src")).unwrap();
        std::fs::write(root.join("app/src/main.tsx"), "").unwrap();

        let p = Project::locate(Some(&root.join("app"))).unwrap();
        assert_eq!(p.entry, root.join("app/src/main.tsx"));
        assert_eq!(p.root, root.join("app"));
        assert_eq!(p.dist_dir(), root.join("app/dist"));
        assert_eq!(p.cache_dir(), root.join("app/.pulse"));

        // `src/App.tsx` wins over the other candidates.
        std::fs::write(root.join("app/src/App.tsx"), "").unwrap();
        assert_eq!(Project::locate(Some(&root.join("app"))).unwrap().entry, root.join("app/src/App.tsx"));

        // An explicit file is used as-is.
        let explicit = root.join("app/src/main.tsx");
        assert_eq!(Project::locate(Some(&explicit)).unwrap().entry, explicit);

        let err = Project::locate(Some(&root)).unwrap_err();
        assert!(err.contains("no Pulse app found") && err.contains("src/App.tsx"), "{err}");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn root_is_entry_dir_otherwise() {
        assert_eq!(Project::new("app/App.tsx").root, PathBuf::from("app"));
        assert_eq!(Project::new("App.tsx").root, PathBuf::from("."));
        assert_eq!(Project::new("src/App.tsx").root, PathBuf::from("."));
    }
}
