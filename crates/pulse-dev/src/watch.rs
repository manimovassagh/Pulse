//! Polling file watcher.
//!
//! Polling every couple of hundred milliseconds is plenty for a source tree
//! and works the same on every OS and filesystem (including containers and
//! network mounts), without platform-specific dependencies.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// File extensions that can affect a build.
const WATCHED_EXTENSIONS: &[&str] = &["tsx", "ts", "css", "html"];

/// Directory names never worth scanning.
const IGNORED_DIRS: &[&str] = &["node_modules", "target", "dist", ".pulse", ".git"];

pub struct Watcher {
    roots: Vec<PathBuf>,
    files: HashMap<PathBuf, SystemTime>,
}

impl Watcher {
    /// Watches the given files and directories (recursively).
    pub fn new(roots: Vec<PathBuf>) -> Self {
        let mut w = Watcher { roots, files: HashMap::new() };
        w.files = w.scan();
        w
    }

    /// Returns files that were added, modified or removed since the last call.
    pub fn changes(&mut self) -> Vec<PathBuf> {
        let now = self.scan();
        let mut changed: Vec<PathBuf> = now
            .iter()
            .filter(|(path, time)| self.files.get(*path) != Some(*time))
            .map(|(path, _)| path.clone())
            .collect();
        changed.extend(self.files.keys().filter(|p| !now.contains_key(*p)).cloned());
        changed.sort();
        self.files = now;
        changed
    }

    fn scan(&self) -> HashMap<PathBuf, SystemTime> {
        let mut out = HashMap::new();
        for root in &self.roots {
            if root.is_dir() {
                scan_dir(root, &mut out);
            } else {
                record(root, &mut out);
            }
        }
        out
    }
}

fn scan_dir(dir: &Path, out: &mut HashMap<PathBuf, SystemTime>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if path.is_dir() {
            if !name.starts_with('.') && !IGNORED_DIRS.contains(&name.as_ref()) {
                scan_dir(&path, out);
            }
        } else if path.extension().and_then(|e| e.to_str()).is_some_and(|e| WATCHED_EXTENSIONS.contains(&e)) {
            record(&path, out);
        }
    }
}

fn record(path: &Path, out: &mut HashMap<PathBuf, SystemTime>) {
    if let Ok(modified) = fs::metadata(path).and_then(|m| m.modified()) {
        out.insert(path.to_path_buf(), modified);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_add_modify_remove_and_ignores_noise() {
        let dir = std::env::temp_dir().join(format!("pulse-watch-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("node_modules")).unwrap();
        fs::write(dir.join("App.tsx"), "a").unwrap();
        let mut w = Watcher::new(vec![dir.clone()]);
        assert!(w.changes().is_empty());

        fs::write(dir.join("styles.css"), "b").unwrap();
        fs::write(dir.join("notes.md"), "ignored extension").unwrap();
        fs::write(dir.join("node_modules/x.ts"), "ignored dir").unwrap();
        assert_eq!(w.changes(), vec![dir.join("styles.css")]);

        // Make sure the timestamp actually moves on coarse filesystems.
        let later = SystemTime::now() + std::time::Duration::from_secs(5);
        fs::File::options().write(true).open(dir.join("App.tsx")).unwrap().set_modified(later).unwrap();
        assert_eq!(w.changes(), vec![dir.join("App.tsx")]);

        fs::remove_file(dir.join("styles.css")).unwrap();
        assert_eq!(w.changes(), vec![dir.join("styles.css")]);
        fs::remove_dir_all(&dir).unwrap();
    }
}
