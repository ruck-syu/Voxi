//! One command's filesystem snapshot, completeness, and cancellation state.
use std::{cell::RefCell, collections::HashMap, fs, path::{Path, PathBuf},
    sync::{Arc, atomic::{AtomicBool, AtomicUsize, Ordering}}, time::{Duration, Instant}};
use crate::fsx;

pub const MAX_ENTRIES: usize = 50_000;
pub const MAX_DEPTH: usize = 12;

#[derive(Clone, Default)]
pub struct Control {
    pub cancelled: Arc<AtomicBool>,
    pub visited: Arc<AtomicUsize>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cached_scans_reuse_ancestor_results_and_a_new_command_sees_changes() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("nested");
        fs::create_dir(&nested).unwrap();
        fs::write(nested.join("a.txt"), b"a").unwrap();
        let search = Search::default();
        assert_eq!(search.entries(dir.path()).len(), 2);
        let visits = search.control.visited.load(Ordering::Relaxed);
        fs::write(nested.join("b.txt"), b"b").unwrap();
        assert_eq!(search.entries(&nested).len(), 1);
        assert_eq!(search.control.visited.load(Ordering::Relaxed), visits);
        assert_eq!(Search::default().entries(&nested).len(), 2);
        assert!(search.status().is_none());
    }

    #[test]
    fn limits_and_cancellation_are_explicit() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("nested")).unwrap();
        fs::write(dir.path().join("nested/file.txt"), b"a").unwrap();
        let mut search = Search::default();
        search.depth = 0;
        search.entries(dir.path());
        assert!(search.status().unwrap().contains("depth limit"));
        let mut search = Search::default();
        search.limit = 1;
        search.entries(dir.path());
        assert!(search.status().unwrap().contains("entry limit"));
        let mut search = Search::default();
        search.timeout = Duration::ZERO;
        search.entries(dir.path());
        assert!(search.status().unwrap().contains("time limit"));
        let search = Search::default();
        search.control.cancel();
        assert!(search.entries(dir.path()).is_empty());
        assert_eq!(search.status().as_deref(), Some("Search cancelled."));
    }

    #[test]
    fn denied_or_missing_folders_are_not_empty_successes() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let search = Search::default();
        search.entries(&dir.path().join("missing"));
        assert!(search.status().unwrap().contains("couldn't read"));
        let denied = dir.path().join("denied");
        fs::create_dir(&denied).unwrap();
        fs::set_permissions(&denied, fs::Permissions::from_mode(0o000)).unwrap();
        let search = Search::default();
        search.entries(&denied);
        fs::set_permissions(&denied, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(search.status().unwrap().contains("couldn't read"));
    }
}
impl Control {
    pub fn cancel(&self) { self.cancelled.store(true, Ordering::Relaxed); }
    pub fn is_cancelled(&self) -> bool { self.cancelled.load(Ordering::Relaxed) }
}

#[derive(Clone)]
pub struct Entry { pub path: PathBuf, pub is_dir: bool }

pub struct Search {
    cache: RefCell<HashMap<PathBuf, Arc<Vec<Entry>>>>,
    issues: RefCell<Vec<String>>,
    pub control: Control,
    started: Instant,
    pub depth: usize,
    pub limit: usize,
    pub timeout: Duration,
}

impl Default for Search {
    fn default() -> Self { Self::new(Control::default()) }
}

impl Search {
    pub fn new(control: Control) -> Self {
        Self { cache: RefCell::new(HashMap::new()), issues: RefCell::new(Vec::new()), control,
            started: Instant::now(), depth: MAX_DEPTH, limit: MAX_ENTRIES, timeout: Duration::from_secs(10) }
    }

    pub fn issue(&self, message: String) {
        let mut issues = self.issues.borrow_mut();
        if issues.len() < 8 && !issues.contains(&message) { issues.push(message); }
    }

    pub fn status(&self) -> Option<String> {
        if self.control.is_cancelled() { return Some("Search cancelled.".into()); }
        let issues = self.issues.borrow();
        (!issues.is_empty()).then(|| format!("Search incomplete: {}. Try a smaller folder or check folder access.", issues.join("; ")))
    }

    pub fn stopped(&self) -> bool {
        if self.control.is_cancelled() { return true; }
        if self.started.elapsed() >= self.timeout {
            self.issue("search time limit reached".into());
            return true;
        }
        false
    }

    pub fn entries(&self, dir: &Path) -> Arc<Vec<Entry>> {
        let key = fsx::canonical(dir);
        if let Some(entries) = self.cache.borrow().get(&key) { return entries.clone(); }
        // Folder resolution may already have scanned an ancestor.
        let ancestor = self.cache.borrow().iter().find(|(root, _)| key.starts_with(root))
            .map(|(_, entries)| entries.clone());
        let entries = if let Some(ancestor) = ancestor {
            ancestor.iter().filter(|e| e.path != dir && e.path.starts_with(dir)).cloned().collect()
        } else {
            let mut out = Vec::new();
            let mut remaining = self.limit;
            self.walk(dir, self.depth, &mut remaining, &mut out);
            out
        };
        let entries = Arc::new(entries);
        self.cache.borrow_mut().insert(key, entries.clone());
        entries
    }

    fn walk(&self, dir: &Path, depth: usize, remaining: &mut usize, out: &mut Vec<Entry>) {
        if self.stopped() { return; }
        let rd = match fs::read_dir(dir) {
            Ok(rd) => rd,
            Err(e) => { self.issue(format!("couldn't read {} ({e})", dir.display())); return; }
        };
        let mut entries = Vec::new();
        for entry in rd {
            if self.stopped() { return; }
            match entry {
                Ok(entry) => entries.push(entry),
                Err(e) => self.issue(format!("couldn't enumerate {} ({e})", dir.display())),
            }
            if entries.len() > *remaining { break; }
        }
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            if self.stopped() { return; }
            if *remaining == 0 { self.issue(format!("entry limit reached in {}", dir.display())); return; }
            *remaining -= 1;
            self.control.visited.fetch_add(1, Ordering::Relaxed);
            let path = e.path();
            if fsx::is_hidden_or_internal(&path) { continue; }
            let ft = match e.file_type() {
                Ok(ft) => ft,
                Err(e) => { self.issue(format!("couldn't inspect {} ({e})", path.display())); continue; }
            };
            let is_dir = ft.is_dir();
            if !is_dir { out.push(Entry { path, is_dir }); continue; }
            out.push(Entry { path: path.clone(), is_dir });
            let bundle = path.extension().is_some_and(|e| matches!(e.to_str(), Some("app" | "photoslibrary" | "bundle" | "framework")));
            if is_dir && !bundle {
                if depth == 0 { self.issue(format!("depth limit reached at {}", path.display())); }
                else { self.walk(&path, depth - 1, remaining, out); }
            }
        }
    }
}
