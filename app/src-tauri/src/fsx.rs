//! Allowed folders, path safety checks, and small filesystem helpers.

use crate::model::Action;
use std::collections::{HashMap, HashSet};
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct ConfiguredRoot {
    parent: PathBuf,
    dev: u64,
    ino: u64,
    target: PathBuf,
}

impl ConfiguredRoot {
    fn read(path: &Path) -> Option<Self> {
        let parent = fs::canonicalize(path.parent()?).ok()?;
        let path = parent.join(path.file_name()?);
        let entry = fs::symlink_metadata(&path).ok()?;
        if entry.ino() == 0 {
            return None;
        }
        let target = fs::canonicalize(&path).ok()?;
        Some(Self { parent, dev: entry.dev(), ino: entry.ino(), target })
    }

    fn register(self, path: &Path, configured: &mut HashMap<PathBuf, Self>) {
        if self.target != path
            && let Some(target) = Self::read(&self.target)
        {
            configured.entry(self.target.clone()).or_insert(target);
        }
        configured.insert(path.to_path_buf(), self);
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
    configured: HashMap<PathBuf, ConfiguredRoot>,
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
        let mut configured = HashMap::new();
        let mut allow_root = |path: &Path| {
            let entry = ConfiguredRoot::read(path);
            allowed.push(entry.as_ref().map_or_else(|| canonical(path), |entry| entry.target.clone()));
            if let Some(entry) = entry {
                entry.register(path, &mut configured);
            }
        };
        for (key, dir) in std_dirs {
            let p = home.join(dir);
            allow_root(&p);
            named.push((key.to_string(), p));
        }
        named.push(("docs".into(), home.join("Documents")));
        named.push(("photos".into(), home.join("Pictures")));
        for p in extra {
            if let Some(name) = p.file_name() {
                named.push((name.to_string_lossy().to_lowercase(), p.clone()));
            }
            allow_root(p);
        }
        let mut roots = Roots {
            home: home.to_path_buf(),
            named,
            allowed,
            configured,
            screenshots: None,
            nicknames: Vec::new(),
            exclusions: Exclusions::default(),
        };
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
            if !roots.configured.contains_key(s)
                && let Some(entry) = ConfiguredRoot::read(s)
            {
                entry.register(s, &mut roots.configured);
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
        self.is_allowed_canonical(&canonical(p))
    }

    /// Checks a path the caller has already canonicalized.
    pub(crate) fn is_allowed_canonical(&self, p: &Path) -> bool {
        self.allowed.iter().any(|root| p.starts_with(root)) && !self.exclusions.hides(p)
    }

    /// True when `p` is one of the allowed roots itself.
    pub fn is_root(&self, p: &Path) -> bool {
        let c = canonical(p);
        self.allowed.contains(&c) || self.screenshots.as_ref().is_some_and(|s| canonical(s) == c)
    }

    /// Resolves a path for planning or Finder, including configured roots
    /// whose parent is outside the allowed folders.
    pub(crate) fn is_allowed_entry(&self, p: &Path) -> bool {
        self.is_allowed(p) && (p.parent().is_none_or(|parent| self.is_allowed(parent)) || self.is_configured_root(p))
    }

    pub(crate) fn check_whole_tree_exclusions(&self, action: &Action) -> Result<(), String> {
        if self.exclusions.is_empty() {
            return Ok(());
        }
        let renamed;
        let (source, destination) = match action {
            Action::Move { from, to } | Action::Copy { from, to } | Action::RestoreFromTrash { from, to } => (from, Some(to)),
            Action::Rename { path, new_name } => {
                renamed = path.with_file_name(new_name);
                if *path == renamed {
                    return Ok(());
                }
                (path, Some(&renamed))
            }
            Action::Trash { path } => (path, None),
            Action::Zip { path, .. } => {
                // ditto follows a link supplied as a source argument.
                return self.check_excluded_descendants(path, Some(path));
            }
            Action::ResizeImage { path, backup, .. } => (path, Some(backup)),
            Action::Restore { from, to } => {
                if fs::symlink_metadata(to).is_ok_and(|meta| meta.is_dir()) {
                    self.check_excluded_descendants(to, Some(to))?;
                }
                (from, Some(to))
            }
            Action::Unzip { destination, .. } => return self.check_excluded_descendants(destination, None),
            _ => return Ok(()),
        };
        // Moving or copying a symlink operates on the link itself.
        if fs::symlink_metadata(source).is_ok_and(|meta| meta.is_dir()) {
            let checked = self
                .check_excluded_descendants(source, Some(source))
                .and_then(|()| destination.map_or(Ok(()), |destination| self.check_excluded_descendants(destination, Some(source))));
            // Exclusions are saved by path, so a rename would quietly drop one.
            if matches!(action, Action::Rename { .. }) {
                return checked.map_err(|message| {
                    message.replace(
                        "Name the files to include, or remove the exclusion.",
                        "Renaming it would stop that exclusion from working. Remove the exclusion first, then rename it and exclude the folder again.",
                    )
                });
            }
            checked?;
        }
        Ok(())
    }

    fn check_excluded_descendants(&self, folder: &Path, contents: Option<&Path>) -> Result<(), String> {
        let canonical_folder = canonical(folder);
        for excluded in &self.exclusions.excluded {
            let Some(relative) = strip_prefix_ignoring_case(excluded, &canonical_folder) else { continue };
            if relative.as_os_str().is_empty() {
                continue;
            }
            // Check configured paths directly, including inside trees skipped
            // by searches. Missing paths do not count, and unreadable ones
            // may still contain excluded entries.
            if let Some(contents) = contents
                && matches!(fs::symlink_metadata(contents.join(&relative)),
                    Err(e) if matches!(e.kind(), std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory))
            {
                continue;
            }
            let relation = if contents == Some(folder) { "contains" } else { "would include" };
            return Err(format!(
                "“{}” {relation} “{}”, which is excluded in Settings → Folders. Name the files to include, or remove the exclusion.",
                file_name(folder),
                relative.display()
            ));
        }
        Ok(())
    }

    /// Match the original directory entry, resolving case via the filesystem
    /// and checking that a configured symlink still resolves to its original target.
    pub fn is_configured_root(&self, p: &Path) -> bool {
        p.is_absolute()
            && !has_parent_ref(p)
            && ConfiguredRoot::read(p).is_some_and(|entry| self.configured.values().any(|root| *root == entry))
    }

    /// True when a recorded root spelling no longer points to its original entry.
    pub(crate) fn configured_root_changed(&self, p: &Path) -> bool {
        self.configured.get(p).is_some_and(|entry| ConfiguredRoot::read(p).as_ref() != Some(entry))
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

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct FileIdentity {
    dev: u64,
    ino: u64,
    size: u64,
    mtime: i64,
    mtime_nsec: i64,
}

impl FileIdentity {
    pub(crate) fn read(path: &Path) -> std::io::Result<Self> {
        fs::symlink_metadata(path).map(|meta| Self::from_metadata(&meta))
    }

    pub(crate) fn from_metadata(meta: &fs::Metadata) -> Self {
        Self { dev: meta.dev(), ino: meta.ino(), size: meta.len(), mtime: meta.mtime(), mtime_nsec: meta.mtime_nsec() }
    }

    pub(crate) fn same_object(&self, other: &Self) -> bool {
        self.dev == other.dev && self.ino == other.ino
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum PublishError {
    Clean(String),
    Published { identity: FileIdentity, message: String },
    Uncertain(String),
    CrossDevice(String),
}

impl From<String> for PublishError {
    fn from(message: String) -> Self {
        Self::Clean(message)
    }
}

pub(crate) type Publication = Result<FileIdentity, PublishError>;

/// Capture the identity before publication, so a replacement at the output
/// cannot be mistaken for the result of this operation.
pub(crate) fn rename_exclusive(from: &Path, to: &Path) -> Publication {
    let parent = fs::canonicalize(to.parent().ok_or_else(|| "Missing parent".to_string())?).map_err(|e| e.to_string())?;
    revalidate_parent(to, &parent)?;
    let meta = fs::symlink_metadata(from).map_err(|e| e.to_string())?;
    let identity = FileIdentity::from_metadata(&meta);
    #[cfg(test)]
    run_hook(&BEFORE_PUBLISH);
    if meta.is_dir() || same_entry_rename(from, to) {
        rename_without_replacing(from, to).map_err(|error| rename_error(to, error))?;
        return Ok(identity);
    }
    // One no-replace rename moves whatever file is at `from` and never
    // deletes anything. Linking and then unlinking the source by name would
    // delete a newer version saved over it in between.
    #[cfg(test)]
    let renamed = if FORCE_LINK_FALLBACK.with(|force| force.get()) {
        Err(std::io::Error::from_raw_os_error(libc::ENOTSUP))
    } else {
        rename_no_replace(from, to)
    };
    #[cfg(not(test))]
    let renamed = rename_no_replace(from, to);
    match renamed {
        Ok(()) => {
            return if FileIdentity::read(to).is_ok_and(|current| current == identity) {
                Ok(identity)
            } else {
                Err(PublishError::Uncertain(format!(
                    "{} changed while it was being moved. The current version is at {}.",
                    from.display(),
                    to.display()
                )))
            };
        }
        Err(error) if !rename_flag_unsupported(&error) => return Err(rename_error(to, error)),
        Err(_) => {}
    }
    if let Err(error) = fs::hard_link(from, to) {
        rename_after_link_failure(from, to, error)?;
    } else {
        // A source replaced after publication must never be unlinked.
        if !FileIdentity::read(from).is_ok_and(|current| current == identity)
            || !FileIdentity::read(to).is_ok_and(|current| current == identity)
        {
            return Err(PublishError::Uncertain(format!(
                "The source {} or published destination {} changed during the move.",
                from.display(),
                to.display()
            )));
        }
        if let Err(e) = unlink_published_source(from) {
            let message = format!("Published {}; couldn't remove source {}: {e}", to.display(), from.display());
            return Err(
                if FileIdentity::read(from).is_ok_and(|current| current == identity)
                    && FileIdentity::read(to).is_ok_and(|current| current == identity)
                {
                    PublishError::Published { identity, message }
                } else {
                    PublishError::Uncertain(message)
                },
            );
        }
    }
    Ok(identity)
}

/// Some volumes (older formats, some network shares) don't support the
/// rename flags; the caller then falls back to a checked alternative.
fn rename_flag_unsupported(error: &std::io::Error) -> bool {
    matches!(error.raw_os_error(), Some(libc::ENOTSUP | libc::EINVAL))
}

fn unlink_published_source(from: &Path) -> std::io::Result<()> {
    #[cfg(test)]
    if UNLINK_FAILURE.with(|failure| failure.get()) {
        return Err(std::io::Error::other("Injected source cleanup failure"));
    }
    fs::remove_file(from)
}

#[cfg(test)]
type Hook = std::cell::RefCell<Option<Box<dyn FnOnce()>>>;

/// Runs a test's simulated concurrent change, such as an editor saving.
#[cfg(test)]
fn run_hook(hook: &'static std::thread::LocalKey<Hook>) {
    if let Some(change) = hook.with(|hook| hook.borrow_mut().take()) {
        change();
    }
}

#[cfg(test)]
thread_local! {
    pub(crate) static BEFORE_PUBLISH: Hook = const { std::cell::RefCell::new(None) };
    pub(crate) static BEFORE_REPLACE: Hook = const { std::cell::RefCell::new(None) };
    pub(crate) static UNLINK_FAILURE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Act as if the volume lacks no-replace renames, to test the fallback.
    pub(crate) static FORCE_LINK_FALLBACK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    pub(crate) static STAGE_CLEANUP_FAILURE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn rename_error(to: &Path, error: std::io::Error) -> PublishError {
    let message = format!("Cannot publish {}: {error}", to.display());
    if error.raw_os_error() == Some(libc::EXDEV) { PublishError::CrossDevice(message) } else { PublishError::Clean(message) }
}

fn same_entry_rename(from: &Path, to: &Path) -> bool {
    let (Some(from_parent), Some(to_parent)) = (from.parent(), to.parent()) else { return false };
    let (Some(from_name), Some(to_name)) = (from.file_name(), to.file_name()) else { return false };
    if from_name == to_name {
        return false;
    }
    let Ok(fp) = fs::canonicalize(from_parent) else { return false };
    let Ok(tp) = fs::canonicalize(to_parent) else { return false };
    if fp != tp {
        return false;
    }
    match (FileIdentity::read(from), FileIdentity::read(to)) {
        (Ok(a), Ok(b)) => a.same_object(&b),
        _ => false,
    }
}

fn rename_after_link_failure(from: &Path, to: &Path, error: std::io::Error) -> Result<(), PublishError> {
    if error.kind() == std::io::ErrorKind::Unsupported
        || matches!(error.raw_os_error(), Some(libc::ENOTSUP | libc::EPERM | libc::EMLINK))
        || (error.kind() == std::io::ErrorKind::AlreadyExists && same_entry_rename(from, to))
    {
        return rename_without_replacing(from, to).map_err(|error| rename_error(to, error));
    }
    Err(rename_error(to, error))
}

/// A no-replace rename. Drives without that option (ExFAT, FAT32) get a
/// plain rename after checking the destination is free; a plain rename
/// still can't replace a folder that has anything in it.
fn rename_without_replacing(from: &Path, to: &Path) -> std::io::Result<()> {
    match rename_no_replace(from, to) {
        Err(error) if rename_flag_unsupported(&error) => {
            if to.symlink_metadata().is_ok() {
                return Err(std::io::Error::from(std::io::ErrorKind::AlreadyExists));
            }
            fs::rename(from, to)
        }
        result => result,
    }
}

fn rename_no_replace(from: &Path, to: &Path) -> std::io::Result<()> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    let from = CString::new(from.as_os_str().as_bytes())?;
    let to = CString::new(to.as_os_str().as_bytes())?;
    // SAFETY: both C strings are live and NUL-terminated. The flags forbid replacement.
    #[cfg(target_os = "macos")]
    let result = unsafe { libc::renamex_np(from.as_ptr(), to.as_ptr(), libc::RENAME_EXCL) };
    #[cfg(target_os = "linux")]
    let result = unsafe { libc::renameat2(libc::AT_FDCWD, from.as_ptr(), libc::AT_FDCWD, to.as_ptr(), libc::RENAME_NOREPLACE) };
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    let result = -1;
    if result == 0 { Ok(()) } else { Err(std::io::Error::last_os_error()) }
}

/// Atomically exchanges two directory entries.
fn rename_swap(a: &Path, b: &Path) -> std::io::Result<()> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    let a = CString::new(a.as_os_str().as_bytes())?;
    let b = CString::new(b.as_os_str().as_bytes())?;
    // SAFETY: both C strings are live and NUL-terminated.
    #[cfg(target_os = "macos")]
    let result = unsafe { libc::renamex_np(a.as_ptr(), b.as_ptr(), libc::RENAME_SWAP) };
    #[cfg(target_os = "linux")]
    let result = unsafe { libc::renameat2(libc::AT_FDCWD, a.as_ptr(), libc::AT_FDCWD, b.as_ptr(), libc::RENAME_EXCHANGE) };
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    let result = {
        let _ = (a, b);
        -1
    };
    if result == 0 { Ok(()) } else { Err(std::io::Error::last_os_error()) }
}

/// A free name beside `path`, such as "photo (edited during resize).png".
fn sibling_name(path: &Path, label: &str, n: usize) -> PathBuf {
    let stem = path.file_stem().map_or_else(String::new, |s| s.to_string_lossy().into_owned());
    let ext = path.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    let number = if n == 0 { String::new() } else { format!(" {}", n + 1) };
    path.with_file_name(format!("{stem} ({label}{number}){ext}"))
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
    pub(crate) fn finish(&self, result: Publication) -> Publication {
        let cleanup = (|| {
            #[cfg(test)]
            if STAGE_CLEANUP_FAILURE.with(|failure| failure.get()) {
                return Err("Injected stage cleanup failure".into());
            }
            let before = self.dir_handle.metadata().map_err(|e| e.to_string())?;
            let now = fs::symlink_metadata(&self.dir).map_err(|e| e.to_string())?;
            if before.dev() != now.dev() || before.ino() != now.ino() {
                return Err("staging directory moved or replaced".into());
            }
            remove_private_tree(&self.dir).map_err(|e| e.to_string())
        })();
        let Err(error) = cleanup else { return result };
        let append = |message: &str| format!("{message}; staging cleanup failed at {}: {error}", self.dir.display());
        Err(match result {
            Ok(identity) => PublishError::Published { identity, message: append("Output published successfully") },
            Err(PublishError::Published { identity, message }) => PublishError::Published { identity, message: append(&message) },
            Err(PublishError::Uncertain(message)) => PublishError::Uncertain(append(&message)),
            Err(PublishError::Clean(message) | PublishError::CrossDevice(message)) => PublishError::Clean(append(&message)),
        })
    }
    pub(crate) fn publish(&self, output: &Path) -> Publication {
        self.validate_parent(output)?;
        rename_exclusive(&self.path, output)
    }

    pub(crate) fn replace(&self, output: &Path, expected: &FileIdentity) -> Publication {
        self.validate_parent(output)?;
        if !FileIdentity::read(output).is_ok_and(|current| current == *expected) {
            return Err(format!("Stopped: {} changed while preparing the replacement.", output.display()).into());
        }
        let identity = FileIdentity::read(&self.path).map_err(|e| e.to_string())?;
        #[cfg(test)]
        run_hook(&BEFORE_REPLACE);
        // Swap rather than rename over the output, so the file that was there
        // is never discarded unseen: if it isn't the one checked above (an app
        // saved over it), keep it beside the result.
        match rename_swap(&self.path, output) {
            Ok(()) if FileIdentity::read(&self.path).is_ok_and(|displaced| displaced == *expected) => Ok(identity),
            Ok(()) => self.keep_displaced(output, identity),
            Err(error) if rename_flag_unsupported(&error) => {
                fs::rename(&self.path, output).map_err(|e| format!("Cannot replace {}: {e}", output.display()))?;
                Ok(identity)
            }
            Err(error) => Err(format!("Cannot replace {}: {error}", output.display()).into()),
        }
    }

    /// After a swap, the staging path holds a newer version someone saved
    /// over the output. Move it out of staging (which is deleted) next to it.
    fn keep_displaced(&self, output: &Path, identity: FileIdentity) -> Publication {
        for n in 0..1000 {
            let kept = sibling_name(output, "edited during resize", n);
            match rename_no_replace(&self.path, &kept) {
                Ok(()) => {
                    return Err(PublishError::Published {
                        identity,
                        message: format!(
                            "{} changed while Voxi was resizing it. Your newer version was kept as “{}”.",
                            file_name(output),
                            file_name(&kept)
                        ),
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(_) => break,
            }
        }
        // Nowhere to keep it: swap back, leaving the newer version in place.
        rename_swap(&self.path, output).map_err(|e| {
            PublishError::Uncertain(format!("{} changed while Voxi was resizing it, and Voxi couldn't keep both: {e}", output.display()))
        })?;
        Err(format!("Stopped: {} changed while Voxi was resizing it. It was left as it is.", file_name(output)).into())
    }

    fn validate_parent(&self, output: &Path) -> Result<(), String> {
        revalidate_parent(output, &self.parent)?;
        let before = self.parent_handle.metadata().map_err(|e| e.to_string())?;
        let now = fs::metadata(&self.parent).map_err(|e| e.to_string())?;
        if before.dev() != now.dev() || before.ino() != now.ino() {
            return Err("Stopped: output parent was replaced".into());
        }
        Ok(())
    }
}
impl Drop for Staging {
    fn drop(&mut self) {
        // Never recursively remove a path that now names someone else's tree.
        if let (Ok(before), Ok(now)) = (self.dir_handle.metadata(), fs::symlink_metadata(&self.dir))
            && before.dev() == now.dev()
            && before.ino() == now.ino()
        {
            let _ = remove_private_tree(&self.dir);
        }
    }
}

/// `path` relative to `prefix`, comparing names without regard to case: a
/// folder that doesn't exist yet keeps the spelling typed, while the drive
/// usually treats "project" and "Project" as one folder.
fn strip_prefix_ignoring_case(path: &Path, prefix: &Path) -> Option<PathBuf> {
    let mut rest = path.components();
    for wanted in prefix.components() {
        let got = rest.next()?;
        if got.as_os_str().to_string_lossy().to_lowercase() != wanted.as_os_str().to_string_lossy().to_lowercase() {
            return None;
        }
    }
    Some(rest.as_path().to_path_buf())
}

/// Removes Voxi's own staging tree. A copied read-only folder (mode 0555)
/// can't have its contents deleted, so make folders writable first.
fn remove_private_tree(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut pending = vec![dir.to_path_buf()];
    while let Some(folder) = pending.pop() {
        if let Ok(meta) = fs::symlink_metadata(&folder)
            && meta.is_dir()
        {
            let _ = fs::set_permissions(&folder, fs::Permissions::from_mode(meta.permissions().mode() | 0o700));
            if let Ok(entries) = fs::read_dir(&folder) {
                pending.extend(entries.flatten().map(|entry| entry.path()));
            }
        }
    }
    fs::remove_dir_all(dir)
}

/// Generate privately, then publish with a no-replace claim.
pub(crate) fn staged_output(output: &Path, write: impl FnOnce(&Path) -> Result<(), String>) -> Publication {
    let stage = Staging::new(output)?;
    let result = write(&stage.path).map_err(PublishError::Clean).and_then(|()| stage.publish(output));
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

    /// An editor's atomic save: write a new file, then rename it over `path`.
    fn save_over(path: &Path, bytes: &'static [u8]) -> Box<dyn FnOnce()> {
        let path = path.to_path_buf();
        Box::new(move || {
            let temp = path.with_file_name(".editor-save.tmp");
            fs::write(&temp, bytes).unwrap();
            fs::rename(&temp, &path).unwrap();
        })
    }

    #[test]
    fn a_save_during_a_move_is_moved_not_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let base = canonical(dir.path());
        let (from, to) = (base.join("notes.txt"), base.join("moved.txt"));
        fs::write(&from, b"old").unwrap();
        BEFORE_PUBLISH.with(|hook| *hook.borrow_mut() = Some(save_over(&from, b"newer")));
        let result = rename_exclusive(&from, &to);
        assert!(matches!(&result, Err(PublishError::Uncertain(m)) if m.contains("changed while it was being moved")), "{result:?}");
        assert_eq!(fs::read(&to).unwrap(), b"newer", "the newer save survives at the destination");
        assert!(from.symlink_metadata().is_err());

        // Without a concurrent save the move is clean.
        let other = base.join("other.txt");
        fs::write(&other, b"data").unwrap();
        rename_exclusive(&other, &base.join("other moved.txt")).unwrap();
        assert_eq!(fs::read(base.join("other moved.txt")).unwrap(), b"data");
    }

    #[test]
    fn a_save_during_a_replace_is_kept_beside_the_result() {
        let dir = tempfile::tempdir().unwrap();
        let base = canonical(dir.path());
        let output = base.join("photo.png");
        fs::write(&output, b"original").unwrap();
        let expected = FileIdentity::read(&output).unwrap();
        let stage = Staging::new(&output).unwrap();
        fs::write(&stage.path, b"resized").unwrap();
        BEFORE_REPLACE.with(|hook| *hook.borrow_mut() = Some(save_over(&output, b"newer edit")));
        let result = stage.finish(stage.replace(&output, &expected));
        assert!(
            matches!(&result, Err(PublishError::Published { message, .. }) if message.contains("newer version was kept")),
            "{result:?}"
        );
        assert_eq!(fs::read(&output).unwrap(), b"resized");
        assert_eq!(fs::read(base.join("photo (edited during resize).png")).unwrap(), b"newer edit");

        // Without a concurrent save, the replacement leaves nothing beside it.
        let clean = base.join("clean.png");
        fs::write(&clean, b"original").unwrap();
        let expected = FileIdentity::read(&clean).unwrap();
        let stage = Staging::new(&clean).unwrap();
        fs::write(&stage.path, b"resized").unwrap();
        stage.finish(stage.replace(&clean, &expected)).unwrap();
        assert_eq!(fs::read(&clean).unwrap(), b"resized");
        assert!(base.join("clean (edited during resize).png").symlink_metadata().is_err());
    }

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
        let search = crate::search::Search::default().with_roots(&roots);
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
        assert!(!roots.is_configured_root(&home.path().join("Documents/../Downloads")));
        assert!(!roots.is_configured_root(Path::new("Downloads")));
        assert!(!roots.is_allowed(&home.path().join("Library/a.txt")));
        assert!(!roots.is_allowed(Path::new("/etc/hosts")));
    }

    #[test]
    fn case_only_rename_succeeds_without_clobbering_distinct_files() {
        let dir = tempfile::tempdir().unwrap();
        let upper = dir.path().join("Report.pdf");
        let lower = dir.path().join("report.pdf");
        fs::write(&upper, b"report").unwrap();
        rename_exclusive(&upper, &lower).unwrap();
        let names = || -> Vec<String> {
            let mut v: Vec<String> =
                fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
            v.sort();
            v
        };
        assert_eq!(names(), vec!["report.pdf"]);
        assert_eq!(fs::read(&lower).unwrap(), b"report");

        // Must still refuse clobbering a genuinely different existing file.
        let other = dir.path().join("other.pdf");
        fs::write(&other, b"keep").unwrap();
        assert!(rename_exclusive(&lower, &other).is_err());
        assert_eq!(fs::read(&lower).unwrap(), b"report");
        assert_eq!(fs::read(&other).unwrap(), b"keep");

        // Must still refuse renaming one hard link onto a distinct hard link of the same inode.
        let alias = dir.path().join("alias.pdf");
        fs::hard_link(&lower, &alias).unwrap();
        assert!(rename_exclusive(&lower, &alias).is_err());
        assert!(lower.exists() && alias.exists());
    }
}
