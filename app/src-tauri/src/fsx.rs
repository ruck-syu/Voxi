//! Allowed folders, path safety checks, and small filesystem helpers.

use std::collections::HashSet;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};

pub const TRASH_DIR: &str = "_to_delete";

/// Folders the user excluded, checked against the allowed roots. The deepest
/// matching folder wins, so an allowed folder inside an excluded one is
/// included again. All paths are canonical.
#[derive(Clone, Debug, Default)]
pub struct Exclusions {
    excluded: Vec<PathBuf>,
    included: Vec<PathBuf>,
}

impl Exclusions {
    pub fn is_empty(&self) -> bool {
        self.excluded.is_empty()
    }

    /// True when an exclusion covers canonical path `c` and no deeper allowed
    /// folder includes it again. A folder both added and excluded is excluded.
    pub fn hides(&self, c: &Path) -> bool {
        let deepest = |rules: &[PathBuf]| rules.iter().filter(|r| c.starts_with(r)).map(|r| r.components().count()).max();
        deepest(&self.excluded).is_some_and(|excluded| deepest(&self.included).is_none_or(|included| included <= excluded))
    }

    /// True when nothing at or below canonical `c` is allowed, so a folder
    /// walk can skip the whole tree.
    pub fn prunes(&self, c: &Path) -> bool {
        self.hides(c) && !self.included.iter().any(|r| r != c && r.starts_with(c))
    }
}

#[derive(Debug, Clone)]
pub struct Roots {
    pub home: PathBuf,
    /// Lowercase spoken name -> folder. Includes the four standard folders,
    /// Screenshots when it exists, and any extra folders by their own name.
    pub named: Vec<(String, PathBuf)>,
    /// Canonical allowed roots. Nothing outside these is ever touched.
    pub allowed: Vec<PathBuf>,
    pub screenshots: Option<PathBuf>,
    /// The user's own folder names, looked up before the standard ones but
    /// never searched as extra roots (they already sit inside one).
    pub nicknames: Vec<(String, PathBuf)>,
    /// Folders inside the allowed roots that Voxi must not touch.
    pub exclusions: Exclusions,
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
        let mut roots =
            Roots { home: home.to_path_buf(), named, allowed, screenshots: None, nicknames: Vec::new(), exclusions: Exclusions::default() };
        // Where screenshots land: the system setting if there is one, else
        // the usual Screenshots folders. A custom location inside the home
        // folder is allowed even outside the four standard folders.
        let screenshots = screenshots.filter(|p| p.is_dir() && p.starts_with(home) && p != home).or_else(|| {
            ["Desktop/Screenshots", "Pictures/Screenshots", "Documents/Screenshots"].iter().map(|d| home.join(d)).find(|p| p.is_dir())
        });
        if let Some(s) = &screenshots {
            if !roots.is_allowed(s) {
                roots.allowed.push(canonical(s));
            }
            roots.named.push(("screenshots".into(), s.clone()));
        }
        roots.screenshots = screenshots;
        roots
    }

    /// Adds the folders the user excluded. Call before `with_nicknames`, so a
    /// nickname for an excluded folder is dropped.
    pub fn with_exclusions(mut self, excluded: &[PathBuf]) -> Self {
        self.exclusions = Exclusions { excluded: excluded.iter().map(|p| canonical(p)).collect(), included: self.allowed.clone() };
        self
    }

    /// Adds the user's folder nicknames. Only folders inside the allowed
    /// roots count, so a nickname can never widen what Voxi may touch.
    pub fn with_nicknames(mut self, nicknames: Vec<(String, PathBuf)>) -> Self {
        self.nicknames = nicknames.into_iter().filter(|(_, p)| p.is_dir() && self.is_allowed(p)).collect();
        self
    }

    /// Looks up a spoken folder name. Singular and plural both match, so
    /// "download" finds Downloads. A nickname wins over a standard name.
    pub fn named(&self, key: &str) -> Option<&PathBuf> {
        let nick = crate::settings::nickname_key(key);
        if let Some((_, p)) = self.nicknames.iter().find(|(n, _)| *n == nick) {
            return Some(p);
        }
        let key = key.trim().to_lowercase();
        let find = |k: &str| self.named.iter().find(|(n, _)| n == k).map(|(_, p)| p);
        find(&key)
            .or_else(|| find(&format!("{key}s")))
            .or_else(|| key.strip_suffix('s').and_then(find))
            // Dictation typos in the four standard folders only, never in a
            // user's own folder name.
            .or_else(|| crate::parse::phrase::known_folder(&key).and_then(find))
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
        self.allowed.iter().any(|root| c.starts_with(root)) && !self.exclusions.hides(&c)
    }

    /// True when `p` is one of the allowed roots itself.
    pub fn is_root(&self, p: &Path) -> bool {
        let c = canonical(p);
        self.allowed.contains(&c) || self.screenshots.as_ref().is_some_and(|s| canonical(s) == c)
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
        v.retain(|p| {
            let c = canonical(p);
            p.is_dir() && !self.exclusions.hides(&c) && seen.insert(c)
        });
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
    if i == 0 {
        format!("{bytes} B")
    } else if v < 10.0 {
        format!("{v:.1} {}", UNITS[i])
    } else {
        format!("{v:.0} {}", UNITS[i])
    }
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

/// Rechecks the canonical parent immediately before path-based mutation.
/// This narrows but does not close the TOCTOU window: an ancestor can still
/// be swapped after this check. Full confinement requires openat-relative IO.
pub fn revalidate_parent(path: &Path, expected: &Path) -> Result<(), String> {
    let parent = path.parent().ok_or("Missing parent")?;
    if fs::canonicalize(parent).map_err(|e| e.to_string())? != expected {
        return Err(format!("Stopped: parent of {} changed", path.display()));
    }
    Ok(())
}

/// No-replace publication using the same claims as executor move/copy.
/// Directories are claimed before rename; files are linked before unlink.
pub fn rename_exclusive(from: &Path, to: &Path) -> Result<(), String> {
    let parent = fs::canonicalize(to.parent().ok_or("Missing parent")?).map_err(|e| e.to_string())?;
    revalidate_parent(to, &parent)?;
    if fs::symlink_metadata(from).map_err(|e| e.to_string())?.is_dir() {
        fs::create_dir(to).map_err(|e| format!("Cannot claim {}: {e}", to.display()))?;
        if let Err(e) = fs::rename(from, to) {
            let _ = fs::remove_dir(to);
            return Err(format!("Cannot publish {}: {e}", to.display()));
        }
    } else {
        if let Err(e) = fs::hard_link(from, to) {
            return rename_after_link_failure(from, to, e);
        }
        fs::remove_file(from).map_err(|e| format!("Published {}; source remains at {}: {e}", to.display(), from.display()))?;
    }
    Ok(())
}

// Preserve the original hard-link error if the platform fallback cannot publish.
fn rename_after_link_failure(from: &Path, to: &Path, error: std::io::Error) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    if matches!(error.raw_os_error(), Some(libc::ENOTSUP | libc::EPERM | libc::EMLINK)) {
        use std::{ffi::CString, os::unix::ffi::OsStrExt};
        if let (Ok(from), Ok(to)) = (CString::new(from.as_os_str().as_bytes()), CString::new(to.as_os_str().as_bytes())) {
            // SAFETY: both C strings are live, NUL-terminated paths. EXCL forbids replacement.
            if unsafe { libc::renamex_np(from.as_ptr(), to.as_ptr(), libc::RENAME_EXCL) } == 0 {
                return Ok(());
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = from;
    Err(format!("Cannot claim {}: {error}", to.display()))
}

/// An exclusive mode-0700 directory in the output's parent. Writers may
/// replace files inside it without exposing partially generated output.
pub struct Staging {
    pub dir: PathBuf,
    pub path: PathBuf,
    parent: PathBuf,
    parent_handle: fs::File,
    dir_handle: fs::File,
}
impl Staging {
    pub fn new(output: &Path) -> Result<Self, String> {
        use std::os::unix::fs::DirBuilderExt;
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let parent = fs::canonicalize(output.parent().ok_or("Missing parent")?).map_err(|e| e.to_string())?;
        for _ in 0..100 {
            let dir = parent.join(format!(".voxi-stage-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
            match fs::DirBuilder::new().mode(0o700).create(&dir) {
                Ok(()) => {
                    let parent_handle = fs::File::open(&parent).map_err(|e| e.to_string())?;
                    let dir_handle = fs::File::open(&dir).map_err(|e| e.to_string())?;
                    return Ok(Self { path: dir.join("payload"), dir, parent, parent_handle, dir_handle });
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e.to_string()),
            }
        }
        Err("Cannot claim staging directory".into())
    }
    /// Remove only our private tree, reporting any evidence that could not be cleaned.
    pub fn finish(&self, result: Result<(), String>) -> Result<(), String> {
        let cleanup = (|| {
            let before = self.dir_handle.metadata().map_err(|e| e.to_string())?;
            let now = fs::symlink_metadata(&self.dir).map_err(|e| e.to_string())?;
            if before.dev() != now.dev() || before.ino() != now.ino() {
                return Err("staging directory moved or replaced".into());
            }
            fs::remove_dir_all(&self.dir).map_err(|e| e.to_string())
        })();
        match (result, cleanup) {
            (result, Ok(())) => result,
            (result, Err(e)) => Err(format!(
                "{}; staging cleanup failed at {}: {e}",
                result.err().unwrap_or_else(|| "Output published successfully".into()),
                self.dir.display()
            )),
        }
    }
    pub fn publish(&self, output: &Path) -> Result<(), String> {
        revalidate_parent(output, &self.parent)?;
        let before = self.parent_handle.metadata().map_err(|e| e.to_string())?;
        let now = fs::metadata(&self.parent).map_err(|e| e.to_string())?;
        if before.dev() != now.dev() || before.ino() != now.ino() {
            return Err("Stopped: output parent was replaced".into());
        }
        rename_exclusive(&self.path, output)
    }
}
impl Drop for Staging {
    fn drop(&mut self) {
        // Never recursively remove a path that now names someone else's tree.
        if let (Ok(before), Ok(now)) = (self.dir_handle.metadata(), fs::symlink_metadata(&self.dir))
            && before.dev() == now.dev()
            && before.ino() == now.ino()
        {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }
}

/// Generate privately, then publish with a no-replace claim.
pub fn staged_output(output: &Path, write: impl FnOnce(&Path) -> Result<(), String>) -> Result<(), String> {
    let stage = Staging::new(output)?;
    let result = write(&stage.path).and_then(|()| stage.publish(output));
    stage.finish(result)
}

/// Available bytes for a non-root caller, using the platform's statvfs ABI.
pub fn available_space(path: &Path) -> Result<u64, String> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let name = CString::new(path.as_os_str().as_bytes()).map_err(|e| e.to_string())?;
    let mut buf = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    if unsafe { libc::statvfs(name.as_ptr(), buf.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    // statvfs initialized the platform struct on success.
    let buf = unsafe { buf.assume_init() };
    Ok((buf.f_bavail as u64).saturating_mul(buf.f_frsize))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ~/Documents/Archive excluded, ~/Documents/Archive/Keep added back,
    /// ~/Downloads excluded outright.
    fn excluded_home() -> (tempfile::TempDir, PathBuf, Roots) {
        let dir = tempfile::tempdir().unwrap();
        let h = canonical(dir.path());
        for d in ["Desktop", "Downloads", "Documents/Archive/Keep", "Documents/Archive/Old", "Pictures"] {
            fs::create_dir_all(h.join(d)).unwrap();
        }
        let keep = h.join("Documents/Archive/Keep");
        let roots = Roots::new(&h, None, std::slice::from_ref(&keep)).with_exclusions(&[
            h.join("Documents/Archive"),
            h.join("Downloads"),
            h.join("Pictures/Missing"),
        ]);
        (dir, h, roots)
    }

    #[test]
    fn the_deepest_include_or_exclude_rule_wins() {
        let (_dir, h, roots) = excluded_home();
        assert!(roots.is_allowed(&h.join("Documents/report.pdf")));
        assert!(!roots.is_allowed(&h.join("Documents/Archive")));
        assert!(!roots.is_allowed(&h.join("Documents/Archive/Old/a.pdf")));
        assert!(roots.is_allowed(&h.join("Documents/Archive/Keep/b.pdf")));
        assert!(!roots.is_allowed(&h.join("Downloads/c.dmg")));
        // A missing excluded folder still applies if it is created later.
        assert!(!roots.is_allowed(&h.join("Pictures/Missing/d.png")));
        assert!(roots.is_allowed(&h.join("Pictures/e.png")));
        let searched = roots.search_roots();
        assert!(!searched.contains(&h.join("Downloads")), "{searched:?}");
        assert!(searched.contains(&h.join("Documents/Archive/Keep")), "{searched:?}");
        assert!(!roots.exclusions.prunes(&h.join("Documents/Archive")), "Keep inside it is allowed");
        assert!(roots.exclusions.prunes(&h.join("Documents/Archive/Old")));
    }

    #[test]
    fn a_folder_both_added_and_excluded_is_excluded() {
        let dir = tempfile::tempdir().unwrap();
        let h = canonical(dir.path());
        let extra = h.join("Projects");
        fs::create_dir_all(&extra).unwrap();
        let roots = Roots::new(&h, None, std::slice::from_ref(&extra)).with_exclusions(std::slice::from_ref(&extra));
        assert!(!roots.is_allowed(&extra.join("a.txt")));
    }

    #[test]
    fn folder_walks_leave_out_excluded_folders() {
        let (_dir, h, roots) = excluded_home();
        for f in ["Documents/a.pdf", "Documents/Archive/Old/b.pdf", "Documents/Archive/c.pdf", "Documents/Archive/Keep/d.pdf"] {
            fs::write(h.join(f), b"x").unwrap();
        }
        let search = crate::search::Search::default().with_exclusions(roots.exclusions.clone());
        let mut found: Vec<String> =
            search.entries(&h.join("Documents")).iter().map(|e| e.path.strip_prefix(&h).unwrap().display().to_string()).collect();
        found.sort();
        assert_eq!(found, ["Documents/Archive/Keep", "Documents/Archive/Keep/d.pdf", "Documents/a.pdf"]);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn unsupported_hard_links_fall_back_without_clobbering() {
        let dir = tempfile::tempdir().unwrap();
        let from = dir.path().join("source");
        let to = dir.path().join("target");
        for code in [libc::ENOTSUP, libc::EPERM, libc::EMLINK] {
            fs::write(&from, b"source").unwrap();
            rename_after_link_failure(&from, &to, std::io::Error::from_raw_os_error(code)).unwrap();
            assert!(!from.exists());
            assert_eq!(fs::read(&to).unwrap(), b"source");
            fs::write(&from, b"new source").unwrap();
            assert!(rename_after_link_failure(&from, &to, std::io::Error::from_raw_os_error(code)).is_err());
            assert_eq!(fs::read(&to).unwrap(), b"source");
            assert_eq!(fs::read(&from).unwrap(), b"new source");
            fs::remove_file(&to).unwrap();
            fs::remove_file(&from).unwrap();
        }
        fs::write(&from, b"source").unwrap();
        for code in [libc::EEXIST, libc::EXDEV, libc::EIO] {
            assert!(rename_after_link_failure(&from, &to, std::io::Error::from_raw_os_error(code)).is_err());
            assert!(from.exists() && !to.exists());
        }
    }

    #[test]
    fn available_space_uses_platform_layout_and_reports_errors() {
        let d = tempfile::tempdir().unwrap();
        assert!(available_space(d.path()).unwrap() > 0);
        assert!(available_space(&d.path().join("missing")).is_err());
    }

    #[test]
    fn unique_path_adds_suffix() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("report.pdf"), b"x").unwrap();
        let mut reserved = HashSet::new();
        assert_eq!(unique_path(dir.path(), "report.pdf", &mut reserved), dir.path().join("report 2.pdf"));
        assert_eq!(unique_path(dir.path(), "report.pdf", &mut reserved), dir.path().join("report 3.pdf"));
    }

    #[test]
    fn finds_screenshots_folder_in_pictures() {
        let home = tempfile::tempdir().unwrap();
        for d in ["Desktop", "Downloads", "Documents", "Pictures", "Pictures/Screenshots"] {
            fs::create_dir(home.path().join(d)).unwrap();
        }
        let roots = Roots::new(home.path(), None, &[]);
        assert_eq!(roots.named("screenshots"), Some(&home.path().join("Pictures/Screenshots")));
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
