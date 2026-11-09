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
//! a `src/` directory.

use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct Project {
    pub entry: PathBuf,
    pub root: PathBuf,
}

impl Project {
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
    fn root_is_entry_dir_otherwise() {
        assert_eq!(Project::new("app/App.tsx").root, PathBuf::from("app"));
        assert_eq!(Project::new("App.tsx").root, PathBuf::from("."));
        assert_eq!(Project::new("src/App.tsx").root, PathBuf::from("."));
    }
}
