//! Allowed folders, path safety checks, and small filesystem helpers.

use std::collections::HashSet;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};

pub const TRASH_DIR: &str = "_to_delete";

#[derive(Debug, Clone)]
pub struct Roots {
    pub home: PathBuf,
    /// Lowercase spoken name -> folder. Includes the four standard folders,
    /// Screenshots when it exists, and any extra folders by their own name.
    pub named: Vec<(String, PathBuf)>,
    /// Canonical allowed roots. Nothing outside these is ever touched.
    pub allowed: Vec<PathBuf>,
    pub screenshots: Option<PathBuf>,
}

impl Roots {
    pub fn new(home: &Path, screenshots: Option<PathBuf>, extra: &[PathBuf]) -> Self {
        let std_dirs = [("desktop", "Desktop"), ("downloads", "Downloads"), ("documents", "Documents"), ("pictures", "Pictures")];
        let mut named = Vec::new();
        let mut allowed = Vec::new();
        for (key, dir) in std_dirs {
            let p = home.join(dir);
            named.push((key.to_string(), p.clone()));
            allowed.push(canonical(&p));
        }
        named.push(("docs".into(), home.join("Documents")));
        named.push(("photos".into(), home.join("Pictures")));
        for p in extra {
            if let Some(name) = p.file_name() {
                named.push((name.to_string_lossy().to_lowercase(), p.clone()));
            }
            allowed.push(canonical(p));
        }
        let mut roots = Roots { home: home.to_path_buf(), named, allowed, screenshots: None };
        let screenshots = screenshots
            .filter(|p| p.is_dir() && roots.is_allowed(p))
            .or_else(|| Some(home.join("Desktop/Screenshots")).filter(|p| p.is_dir()));
        if let Some(s) = &screenshots {
            roots.named.push(("screenshots".into(), s.clone()));
        }
        roots.screenshots = screenshots;
        roots
    }

    /// Looks up a spoken folder name. Singular and plural both match, so
    /// "download" finds Downloads.
    pub fn named(&self, key: &str) -> Option<&PathBuf> {
        let key = key.trim().to_lowercase();
        let find = |k: &str| self.named.iter().find(|(n, _)| n == k).map(|(_, p)| p);
        find(&key).or_else(|| find(&format!("{key}s"))).or_else(|| key.strip_suffix('s').and_then(find))
    }

    pub fn desktop(&self) -> PathBuf {
        self.home.join("Desktop")
    }

    pub fn downloads(&self) -> PathBuf {
        self.home.join("Downloads")
    }

    /// True when `p` is inside an allowed root. Symlinks are resolved, so a
    /// link cannot point Voxi outside the allowed folders.
    pub fn is_allowed(&self, p: &Path) -> bool {
        if has_parent_ref(p) || !p.is_absolute() {
            return false;
        }
        let c = canonical(p);
        self.allowed.iter().any(|root| c.starts_with(root))
    }

    /// True when `p` is one of the allowed roots itself.
    pub fn is_root(&self, p: &Path) -> bool {
        let c = canonical(p);
        self.allowed.iter().any(|root| *root == c) || self.screenshots.as_ref().is_some_and(|s| canonical(s) == c)
    }

    /// The `_to_delete/` folder for the root that holds `p`.
    pub fn trash_dir_for(&self, p: &Path) -> PathBuf {
        let c = canonical(p);
        let root = self.allowed.iter().filter(|r| c.starts_with(r)).max_by_key(|r| r.as_os_str().len());
        root.cloned().unwrap_or_else(|| canonical(&self.downloads())).join(TRASH_DIR)
    }

    /// "~/Documents/report.pdf" style path for display.
    pub fn display(&self, p: &Path) -> String {
        match p.strip_prefix(&self.home) {
            Ok(rest) => format!("~/{}", rest.display()),
            Err(_) => p.display().to_string(),
        }
    }

    /// The folders searched when a command names no folder.
    pub fn search_roots(&self) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = self.named.iter().map(|(_, p)| p.clone()).collect();
        let mut seen = HashSet::new();
        v.retain(|p| p.is_dir() && seen.insert(canonical(p)));
        v
    }
}

pub fn has_parent_ref(p: &Path) -> bool {
    p.components().any(|c| matches!(c, Component::ParentDir))
}

/// Canonicalizes the longest existing ancestor and re-appends the rest, so it
/// works for paths that don't exist yet.
pub fn canonical(p: &Path) -> PathBuf {
    if let Ok(c) = fs::canonicalize(p) {
        return c;
    }
    let mut tail = Vec::new();
    let mut cur = p;
    while let Some(parent) = cur.parent() {
        if let Some(name) = cur.file_name() {
            tail.push(name.to_owned());
        }
        if let Ok(c) = fs::canonicalize(parent) {
            let mut out = c;
            for t in tail.iter().rev() {
                out.push(t);
            }
            return out;
        }
        cur = parent;
    }
    p.to_path_buf()
}

pub fn expand_tilde(s: &str, home: &Path) -> PathBuf {
    if s == "~" {
        home.to_path_buf()
    } else if let Some(rest) = s.strip_prefix("~/") {
        home.join(rest)
    } else {
        PathBuf::from(s)
    }
}

pub fn same_file(a: &Path, b: &Path) -> bool {
    match (fs::metadata(a), fs::metadata(b)) {
        (Ok(x), Ok(y)) => x.dev() == y.dev() && x.ino() == y.ino(),
        _ => false,
    }
}

pub fn split_name(name: &str) -> (&str, Option<&str>) {
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && !ext.is_empty() && !ext.contains(' ') => (stem, Some(ext)),
        _ => (name, None),
    }
}

/// `dir/name`, or `dir/name 2`, `dir/name 3`, ... if that is taken on disk or
/// already claimed by an earlier step in the same plan.
pub fn unique_path(dir: &Path, name: &str, reserved: &mut HashSet<PathBuf>) -> PathBuf {
    let (stem, ext) = split_name(name);
    let mut n = 1;
    loop {
        let candidate = if n == 1 {
            name.to_string()
        } else {
            match ext {
                Some(e) => format!("{stem} {n}.{e}"),
                None => format!("{stem} {n}"),
            }
        };
        let p = dir.join(&candidate);
        let key = PathBuf::from(p.to_string_lossy().to_lowercase());
        if !p.exists() && !reserved.contains(&key) {
            reserved.insert(key);
            return p;
        }
        n += 1;
    }
}

pub fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

pub fn is_hidden_or_internal(p: &Path) -> bool {
    let n = file_name(p);
    n.starts_with('.') || n == TRASH_DIR || n == "Icon\r"
}

pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = bytes as f64;
    let mut i = 0;
    while v >= 1000.0 && i < UNITS.len() - 1 {
        v /= 1000.0;
        i += 1;
    }
    if i == 0 { format!("{bytes} B") } else if v < 10.0 { format!("{v:.1} {}", UNITS[i]) } else { format!("{v:.0} {}", UNITS[i]) }
}

/// Total size of a file or folder, following no symlinks.
pub fn tree_size(p: &Path) -> (u64, usize) {
    let Ok(meta) = fs::symlink_metadata(p) else { return (0, 0) };
    if !meta.is_dir() {
        return (meta.len(), 1);
    }
    let mut total = (0, 0);
    if let Ok(rd) = fs::read_dir(p) {
        for e in rd.flatten() {
            let (b, n) = tree_size(&e.path());
            total.0 += b;
            total.1 += n;
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unique_path_adds_suffix() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("report.pdf"), b"x").unwrap();
        let mut reserved = HashSet::new();
        assert_eq!(unique_path(dir.path(), "report.pdf", &mut reserved), dir.path().join("report 2.pdf"));
        assert_eq!(unique_path(dir.path(), "report.pdf", &mut reserved), dir.path().join("report 3.pdf"));
    }

    #[test]
    fn rejects_parent_refs_and_outside_paths() {
        let home = tempfile::tempdir().unwrap();
        for d in ["Desktop", "Downloads", "Documents", "Pictures"] {
            fs::create_dir(home.path().join(d)).unwrap();
        }
        let roots = Roots::new(home.path(), None, &[]);
        assert!(roots.is_allowed(&home.path().join("Downloads/a.txt")));
        assert!(!roots.is_allowed(&home.path().join("Downloads/../secret.txt")));
        assert!(!roots.is_allowed(&home.path().join("Library/a.txt")));
        assert!(!roots.is_allowed(Path::new("/etc/hosts")));
    }
}
