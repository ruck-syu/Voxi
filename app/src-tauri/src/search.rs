//! One command's filesystem snapshot, completeness, and cancellation state.
use crate::fsx;
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

pub const MAX_ENTRIES: usize = 50_000;
pub const MAX_DEPTH: usize = 12;

pub(crate) fn is_bundle(path: &Path) -> bool {
    const BUNDLE_EXTENSIONS: &[&str] = &[
        "app",
        "bundle",
        "framework",
        "photoslibrary",
        "xcodeproj",
        "xcworkspace",
        "xcarchive",
        "xcframework",
        "fcpbundle",
        "logicx",
        "vmwarevm",
        "sparsebundle",
        "pages",
        "key",
        "numbers",
    ];
    path.extension().and_then(|ext| ext.to_str()).is_some_and(|ext| BUNDLE_EXTENSIONS.iter().any(|bundle| ext.eq_ignore_ascii_case(bundle)))
}

pub(crate) fn is_virtual_env(path: &Path) -> bool {
    path.file_name().is_some_and(|name| name.eq_ignore_ascii_case(".venv"))
}

#[derive(Clone, Default)]
pub struct Control {
    pub cancelled: Arc<AtomicBool>,
    pub visited: Arc<AtomicUsize>,
    too_deep: Arc<AtomicUsize>,
    unreadable: Arc<AtomicUsize>,
    failures: Arc<AtomicU8>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearest_dirs_finds_a_shallow_folder_past_a_tree_too_big_to_walk() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        fs::create_dir_all(root.join("dev/repo")).unwrap();
        for i in 0..50 {
            fs::write(root.join("dev/repo").join(format!("f{i}")), b"").unwrap();
            fs::write(root.join(format!("f{i}")), b"").unwrap();
        }
        let search = Search { limit: 2, ..Default::default() };
        search.entries(&root);
        assert!(search.status().unwrap().contains("Entry limit"));
        let search = Search { limit: 2, ..Default::default() };
        let roots = std::slice::from_ref(&root);
        assert_eq!(search.nearest_dirs(roots, "DEV"), NameLookup::Found { candidates: vec![root.join("dev")], needs_confirmation: false });
        assert_eq!(search.control.visited.load(Ordering::Relaxed), 51);
        assert_eq!(search.nearest_dirs(roots, "missing"), NameLookup::NotFound(Vec::new()));
        assert!(search.status().is_none());
        assert!(matches!(Search { limit: 1, ..Default::default() }.nearest_dirs(roots, "missing"),
            NameLookup::Incomplete(message) if message.contains("Entry limit")));
        assert!(matches!(Search { depth: 0, ..Default::default() }.nearest_dirs(roots, "missing"),
            NameLookup::Incomplete(message) if message.contains("dev” because it was nested too deeply")));
    }

    #[test]
    fn nearest_dirs_uses_ancestry_across_overlapping_roots() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let extra = root.join("Work/Dev/project");
        fs::create_dir_all(extra.join("Dev")).unwrap();
        fs::create_dir_all(root.join("Else/team/Dev")).unwrap();
        assert_eq!(
            Search::default().nearest_dirs(&[root.to_path_buf(), extra], "dev"),
            NameLookup::Found { candidates: vec![root.join("Else/team/Dev"), root.join("Work/Dev")], needs_confirmation: false }
        );
    }

    #[test]
    fn nearest_dirs_counts_unreadable_branches_and_checks_other_folders() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let blocked = root.join("Blocked");
        let readable = root.join("Other");
        fs::create_dir_all(&blocked).unwrap();
        fs::create_dir_all(readable.join("Sub/Dev")).unwrap();
        fs::create_dir(root.join("Dev")).unwrap();
        fs::set_permissions(&blocked, fs::Permissions::from_mode(0o000)).unwrap();

        let search = Search::default();
        let deciding = search.nearest_dirs(&[blocked.clone(), readable], "dev");
        let visited = search.control.visited.load(Ordering::Relaxed);
        let search = Search::default();
        let deeper = search.nearest_dirs(std::slice::from_ref(&root), "dev");

        fs::set_permissions(&blocked, fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(deciding, NameLookup::Found { candidates: vec![root.join("Other/Sub/Dev")], needs_confirmation: true });
        assert_eq!(visited, 2);
        assert_eq!(deeper, NameLookup::Found { candidates: vec![root.join("Dev"), root.join("Other/Sub/Dev")], needs_confirmation: true });
        assert!(search.status().unwrap().contains("Blocked"));
    }

    #[test]
    fn name_lookup_and_uncached_walks_share_the_command_deadline() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let first = root.join("First");
        let second = root.join("Second");
        fs::create_dir_all(first.join("Dev")).unwrap();
        fs::create_dir_all(second.join("Dev")).unwrap();
        let mut search = Search::default();
        assert_eq!(search.entries(&first).len(), 1);
        assert!(search.status().is_none());
        search.started -= search.timeout;
        let roots = std::slice::from_ref(&second);
        assert!(matches!(search.nearest_dirs(roots, "dev"), NameLookup::Incomplete(message) if message.contains("time limit")));
        assert!(search.entries(&second).is_empty());
        assert!(search.status().unwrap().contains("time limit"));
        search.control.cancel();
        assert_eq!(search.nearest_dirs(roots, "dev"), NameLookup::Incomplete("Search cancelled.".into()));
        assert_eq!(search.status().as_deref(), Some("Search cancelled."));
    }

    #[test]
    fn mixed_case_bundles_can_match_but_are_not_descended_into() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let bundles = [
            "ApP",
            "BuNdLe",
            "FrAmEwOrK",
            "PhOtOsLiBrArY",
            "XcOdEpRoJ",
            "XcWoRkSpAcE",
            "XcArChIvE",
            "XcFrAmEwOrK",
            "FcPbUnDlE",
            "LoGiCx",
            "VmWaReVm",
            "SpArSeBuNdLe",
            "PaGeS",
            "KeY",
            "NuMbErS",
        ];
        for ext in bundles {
            fs::create_dir_all(root.join(format!("Project.{ext}/dev"))).unwrap();
        }
        let search = Search { depth: 0, ..Default::default() };
        let roots = std::slice::from_ref(&root);
        assert_eq!(search.nearest_dirs(roots, "dev"), NameLookup::NotFound(Vec::new()));
        for ext in bundles {
            let name = format!("Project.{ext}");
            let bundle = root.join(&name);
            assert_eq!(
                search.nearest_dirs(roots, &name.to_lowercase()),
                NameLookup::Found { candidates: vec![bundle], needs_confirmation: false }
            );
        }
        let entries = search.entries(&root);
        assert_eq!(entries.len(), bundles.len());
        assert!(search.status().is_none());
    }

    #[test]
    fn recursion_prunes_dependencies_and_project_outputs_but_can_start_inside_them() {
        let dir = tempfile::tempdir().unwrap();
        let marked = dir.path().join("Project");
        let ordinary = dir.path().join("Reference");
        let outputs = ["target", "build", "dist", "vendor"];
        let dependencies = ["node_modules", "Pods", "DerivedData", "__pycache__", "venv", ".venv"];
        for (parent, names) in [(&marked, [outputs.as_slice(), dependencies.as_slice()].concat()), (&ordinary, outputs.to_vec())] {
            for name in names {
                let folder = parent.join(name);
                fs::create_dir_all(&folder).unwrap();
                fs::write(folder.join("notes.pdf"), b"x").unwrap();
            }
        }
        for marker in ["go.mod", "Cargo.toml", "package.json", "Podfile.lock"] {
            fs::write(marked.join(marker), b"").unwrap();
        }

        let shallow = Search { depth: 0, ..Default::default() };
        assert_eq!(shallow.direct_entries(&ordinary).len(), 4);
        assert!(shallow.status().is_none());
        assert_eq!(shallow.recursive_entries(&marked).skipped().len(), outputs.len() + dependencies.len());
        assert!(shallow.status().is_none());
        assert!(matches!(shallow.lookup_name(std::slice::from_ref(&marked), "notes.pdf", LookupKind::Item),
            NameLookup::NotFound(candidates) if candidates.iter().all(|p| !p.ends_with("notes.pdf"))));
        assert!(shallow.status().is_none());

        let search = Search::default();
        let scan = search.recursive_entries(dir.path());
        assert_eq!(scan.skipped().len(), outputs.len() + dependencies.len());
        assert_eq!(scan.entries.iter().filter(|e| e.path.ends_with("notes.pdf")).count(), 4);
        assert!(search.recursive_entries(&ordinary).skipped().is_empty());
        for name in ["node_modules", "build", ".venv"] {
            let explicitly_named = search.recursive_entries(&marked.join(name));
            assert!(explicitly_named.skipped().is_empty());
            assert_eq!(explicitly_named.entries[0].path, marked.join(name).join("notes.pdf"));
        }
        assert!(search.status().is_none());
    }

    #[test]
    fn lookup_checks_directory_names_in_order_at_the_budget_edge() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        fs::create_dir_all(root.join("A/zzz")).unwrap();
        fs::create_dir_all(root.join("A/Dev")).unwrap();
        fs::create_dir(root.join("Dev")).unwrap();
        let search = Search { limit: 3, ..Default::default() };
        assert_eq!(
            search.nearest_dirs(std::slice::from_ref(&root), "dev"),
            NameLookup::Found { candidates: vec![root.join("A/Dev"), root.join("Dev")], needs_confirmation: true }
        );
    }

    #[test]
    fn lookup_skips_excluded_folders_and_finds_reincluded_roots() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let docs = home.join("Documents");
        let excluded = docs.join("Private");
        let reincluded = excluded.join("Public");
        let outright = docs.join("Hidden");
        fs::create_dir_all(excluded.join("Batch")).unwrap();
        fs::create_dir_all(reincluded.join("Batch")).unwrap();
        fs::create_dir_all(outright.join("Batch")).unwrap();
        let roots = fsx::Roots::new(home, None, std::slice::from_ref(&reincluded)).with_exclusions(&[excluded, outright]);
        let search = Search::default().with_roots(&roots);
        let dirs = roots.search_roots();
        assert_eq!(search.nearest_dirs(&dirs, "private"), NameLookup::NotFound(Vec::new()));
        assert_eq!(search.nearest_dirs(&dirs, "hidden"), NameLookup::NotFound(Vec::new()));
        assert_eq!(
            search.nearest_dirs(&dirs, "batch"),
            NameLookup::Found { candidates: vec![reincluded.join("Batch")], needs_confirmation: false }
        );
    }

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
    fn a_partial_ancestor_scan_cannot_supply_nested_results() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let nested = root.join("nested");
        fs::create_dir(&nested).unwrap();
        fs::write(nested.join("file.txt"), b"a").unwrap();
        let search = Search { depth: 0, ..Default::default() };
        assert_eq!(search.entries(root).len(), 1);
        assert!(search.status().unwrap().contains("nested”"));
        assert_eq!(search.entries(&nested)[0].path, nested.join("file.txt"));
    }

    #[test]
    fn ancestor_cache_matches_differently_cased_and_symlinked_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("Nested");
        fs::create_dir(&nested).unwrap();
        fs::write(nested.join("a.txt"), b"a").unwrap();
        let link_parent = tempfile::tempdir().unwrap();
        let link = link_parent.path().join("NestedLink");
        std::os::unix::fs::symlink(&nested, &link).unwrap();

        let search = Search::default();
        assert_eq!(search.entries(dir.path()).len(), 2);
        let visits = search.control.visited.load(Ordering::Relaxed);

        let via_link = search.entries(&link);
        assert_eq!(via_link.len(), 1);
        assert_eq!(search.control.visited.load(Ordering::Relaxed), visits);

        // On case-insensitive macOS filesystems, querying with different casing
        // must hit the ancestor cache rather than returning [].
        let lower_nested = dir.path().join("nested");
        if fsx::same_file(&nested, &lower_nested) {
            let search2 = Search::default();
            assert_eq!(search2.entries(dir.path()).len(), 2);
            let visits2 = search2.control.visited.load(Ordering::Relaxed);
            assert_eq!(search2.entries(&lower_nested).len(), 1);
            assert_eq!(search2.control.visited.load(Ordering::Relaxed), visits2);
        }
    }

    #[test]
    fn limits_and_cancellation_are_explicit() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("nested")).unwrap();
        fs::write(dir.path().join("nested/file.txt"), b"a").unwrap();
        let search = Search { depth: 0, ..Default::default() };
        search.entries(dir.path());
        let message = search.status().unwrap();
        assert!(message.contains("nested” because it was nested too deeply."), "{message}");
        assert!(!message.contains("including"), "{message}");
        fs::create_dir(dir.path().join("another")).unwrap();
        let search = Search { depth: 0, ..Default::default() };
        search.entries(dir.path());
        let message = search.status().unwrap();
        assert!(
            message.contains("Skipped 2 folders nested too deeply:") && message.contains("another”") && message.contains("nested”"),
            "{message}"
        );
        fs::remove_dir(dir.path().join("another")).unwrap();
        for n in 0..20 {
            fs::create_dir_all(dir.path().join(format!("deep{n}/inner"))).unwrap();
        }
        let search = Search { depth: 0, ..Default::default() };
        search.entries(dir.path());
        let message = search.status().unwrap();
        assert!(
            message.contains("Skipped 21 folders nested too deeply, including")
                && message.contains("deep0”")
                && message.contains("deep1”")
                && !message.contains("deep2”"),
            "{message}"
        );
        let search = Search { limit: 1, ..Default::default() };
        search.entries(dir.path());
        let message = search.status().unwrap();
        assert!(message.contains("Entry limit") && message.contains("deep0”") && message.contains("Settings → Folders"), "{message}");
        let search = Search { timeout: Duration::ZERO, ..Default::default() };
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
        assert!(search.status().unwrap().contains("Couldn't read"));
        let denied = dir.path().join("denied");
        fs::create_dir(&denied).unwrap();
        fs::set_permissions(&denied, fs::Permissions::from_mode(0o000)).unwrap();
        let search = Search::default();
        search.entries(&denied);
        fs::set_permissions(&denied, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(search.status().unwrap().contains("Couldn't read"));
    }

    #[test]
    fn unreadable_message_shows_that_the_list_is_truncated() {
        let dir = tempfile::tempdir().unwrap();
        let search = Search::default();
        for folder in ["first", "second", "third"] {
            search.direct_entries(&dir.path().join(folder));
        }
        let message = search.status().unwrap();
        assert!(message.contains("first”") && message.contains("second”") && !message.contains("third”"), "{message}");
        assert!(message.contains("3 read errors in total"), "{message}");
    }

    #[test]
    fn speculative_probes_do_not_record_depth_or_read_errors() {
        let dir = tempfile::tempdir().unwrap();
        let deep = dir.path().join("deep");
        let missing = dir.path().join("missing");
        fs::create_dir_all(deep.join("one/two/three/four")).unwrap();
        fs::create_dir(&missing).unwrap();
        let search = Search::default();
        let subdirs = search.direct_entries(dir.path());
        fs::remove_dir(&missing).unwrap();

        // Run each nested-count probe independently so both failure paths run.
        for subdir in subdirs.iter() {
            assert_eq!(search.count_nested(std::slice::from_ref(subdir), |_| Some(false)), None);
        }
        assert!(search.lookup_inside_skipped(&[deep, missing], "absent").is_empty());
        assert!(search.status().is_none());
        let stats = search.control.stats(Duration::ZERO);
        assert_eq!((stats.too_deep, stats.unreadable), (0, 0));
    }
}
#[derive(Clone, Copy)]
#[repr(u8)]
pub(crate) enum Failure {
    Limit = 1,
    Depth = 2,
    Timeout = 4,
    Unreadable = 8,
    SpotlightTimeout = 16,
    Incomplete = 32,
}

#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
pub(crate) struct Stats {
    pub visited: usize,
    pub too_deep: usize,
    pub unreadable: usize,
    pub elapsed_ms_bucket: crate::settings::diagnostics::ElapsedMsBucket,
}

impl Control {
    pub(crate) fn stats(&self, elapsed: Duration) -> Stats {
        Stats {
            visited: self.visited.load(Ordering::Relaxed),
            too_deep: self.too_deep.load(Ordering::Relaxed),
            unreadable: self.unreadable.load(Ordering::Relaxed),
            elapsed_ms_bucket: elapsed.into(),
        }
    }

    pub(crate) fn failed_with(&self, failure: Failure) -> bool {
        self.failures.load(Ordering::Relaxed) & failure as u8 != 0
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }
}

#[derive(Clone)]
pub struct Entry {
    pub path: PathBuf,
    pub is_dir: bool,
    pub(crate) canon: PathBuf,
}

/// Whether a search saw everything its query asked for.
#[derive(Debug, Clone, PartialEq)]
pub enum Completeness {
    Complete,
    /// The user cancelled; nothing from this search may be used.
    Cancelled,
    /// A limit, an unreadable folder or another issue cut it short; the
    /// message says what. Read-only answers may show partial results.
    Partial(String),
}

#[derive(Debug, PartialEq)]
pub enum NameLookup {
    Found {
        candidates: Vec<PathBuf>,
        needs_confirmation: bool,
    },
    /// An exact miss and the paths collected for fuzzy matching, if needed.
    NotFound(Vec<PathBuf>),
    RetryPerRoot(Vec<PathBuf>),
    Incomplete(String),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LookupKind {
    Container,
    ReadOnly,
    Item,
}

#[derive(Default)]
pub(crate) struct Scan {
    pub entries: Arc<Vec<Entry>>,
    skipped: Vec<PathBuf>,
}

impl Scan {
    pub fn skipped(&self) -> &[PathBuf] {
        &self.skipped
    }
}

#[derive(Default)]
struct Walk {
    entries: Vec<Entry>,
    skipped: Vec<PathBuf>,
    remaining: usize,
}

#[derive(Default)]
struct FolderIssues {
    count: usize,
    folders: Vec<PathBuf>,
}

impl FolderIssues {
    fn add(&mut self, folder: &Path) {
        self.count += 1;
        self.remember(folder);
    }

    fn remember(&mut self, folder: &Path) {
        const MAX_SHOWN: usize = 2;
        if self.folders.len() < MAX_SHOWN && !self.folders.iter().any(|path| path == folder) {
            self.folders.push(folder.to_path_buf());
        }
    }

    fn merge(&mut self, other: &Self) {
        self.count += other.count;
        for folder in &other.folders {
            self.remember(folder);
        }
    }
}

pub struct Search {
    cache: RefCell<HashMap<PathBuf, Arc<Scan>>>,
    skipped_folders: RefCell<Vec<PathBuf>>,
    issues: RefCell<Vec<String>>,
    entry_limit: RefCell<FolderIssues>,
    too_deep: RefCell<FolderIssues>,
    unreadable: RefCell<FolderIssues>,
    timed_out: std::cell::Cell<bool>,
    spotlight_timed_out: std::cell::Cell<bool>,
    track_issue_stats: bool,
    home: PathBuf,
    pub control: Control,
    pub depth: usize,
    pub limit: usize,
    pub timeout: Duration,
    started: Instant,
    allowed: Option<Vec<PathBuf>>,
    standard_roots: Vec<PathBuf>,
    /// Excluded folders are left out of results, and skipped entirely when
    /// nothing inside them is allowed.
    pub exclusions: fsx::Exclusions,
}

impl Default for Search {
    fn default() -> Self {
        Self::new(Control::default())
    }
}

impl Search {
    pub fn new(control: Control) -> Self {
        Self {
            cache: RefCell::new(HashMap::new()),
            skipped_folders: RefCell::new(Vec::new()),
            issues: RefCell::new(Vec::new()),
            entry_limit: Default::default(),
            too_deep: Default::default(),
            unreadable: Default::default(),
            timed_out: Default::default(),
            spotlight_timed_out: Default::default(),
            track_issue_stats: true,
            home: PathBuf::new(),
            control,
            depth: MAX_DEPTH,
            limit: MAX_ENTRIES,
            timeout: Duration::from_secs(10),
            started: Instant::now(),
            allowed: None,
            standard_roots: Vec::new(),
            exclusions: fsx::Exclusions::default(),
        }
    }

    pub fn with_roots(mut self, roots: &fsx::Roots) -> Self {
        self.home = roots.home.clone();
        self.allowed = Some(roots.allowed.clone());
        self.standard_roots = ["Desktop", "Downloads", "Documents", "Pictures"].map(|name| fsx::canonical(&roots.home.join(name))).into();
        self.exclusions = roots.exclusions.clone();
        self
    }

    /// Separate completeness and caches, with the command's cancellation,
    /// progress and deadline. A caller can reserve time for later work.
    pub fn isolated(&self, time_limit: Duration) -> Self {
        Self {
            depth: self.depth,
            limit: self.limit,
            started: self.started,
            timeout: self.timeout.min(self.started.elapsed() + time_limit),
            track_issue_stats: self.track_issue_stats,
            home: self.home.clone(),
            allowed: self.allowed.clone(),
            standard_roots: self.standard_roots.clone(),
            exclusions: self.exclusions.clone(),
            ..Self::new(self.control.clone())
        }
    }

    fn probe(&self, time_limit: Duration) -> Self {
        // Keep progress and cancellation shared while counting only required scan issues.
        Self { track_issue_stats: false, ..self.isolated(time_limit) }
    }

    pub fn issue(&self, message: String) {
        let mut issues = self.issues.borrow_mut();
        if issues.len() < 8 && !issues.contains(&message) {
            issues.push(message);
        }
    }

    pub fn record_issues(&self, other: &Self) {
        for issue in other.issues.borrow().iter() {
            self.issue(issue.clone());
        }
        self.entry_limit.borrow_mut().merge(&other.entry_limit.borrow());
        self.too_deep.borrow_mut().merge(&other.too_deep.borrow());
        self.unreadable.borrow_mut().merge(&other.unreadable.borrow());
        self.timed_out.set(self.timed_out.get() || other.timed_out.get());
        self.spotlight_timed_out.set(self.spotlight_timed_out.get() || other.spotlight_timed_out.get());
    }

    pub(crate) fn record_failure(&self) {
        let mut flags = 0;
        for (present, code) in [
            (self.entry_limit.borrow().count > 0, Failure::Limit),
            (self.too_deep.borrow().count > 0, Failure::Depth),
            (self.timed_out.get(), Failure::Timeout),
            (self.unreadable.borrow().count > 0, Failure::Unreadable),
            (self.spotlight_timed_out.get(), Failure::SpotlightTimeout),
        ] {
            if present {
                flags |= code as u8;
            }
        }
        if flags == 0 && !self.issues.borrow().is_empty() {
            flags = Failure::Incomplete as u8;
        }
        self.control.failures.fetch_or(flags, Ordering::Relaxed);
    }

    pub(crate) fn spotlight_timeout(&self) {
        self.spotlight_timed_out.set(true);
    }

    pub(crate) fn unreadable(&self, folder: &Path) {
        if self.track_issue_stats {
            self.control.unreadable.fetch_add(1, Ordering::Relaxed);
        }
        self.unreadable.borrow_mut().add(folder);
    }

    fn too_deep(&self, folder: &Path) {
        if self.track_issue_stats {
            self.control.too_deep.fetch_add(1, Ordering::Relaxed);
        }
        self.too_deep.borrow_mut().add(folder);
    }

    pub fn completeness(&self) -> Completeness {
        if self.control.is_cancelled() {
            return Completeness::Cancelled;
        }
        match self.incomplete_message() {
            Some(message) => Completeness::Partial(message),
            None => Completeness::Complete,
        }
    }

    /// The message for an incomplete or cancelled search; None when complete.
    pub fn status(&self) -> Option<String> {
        match self.completeness() {
            Completeness::Complete => None,
            Completeness::Cancelled => Some("Search cancelled.".into()),
            Completeness::Partial(message) => Some(message),
        }
    }

    fn shown_folders(&self, issue: &FolderIssues) -> String {
        issue
            .folders
            .iter()
            .map(|folder| {
                let shown = if let Ok(rest) = folder.strip_prefix(&self.home) {
                    format!("~/{}", rest.display())
                } else {
                    folder.display().to_string()
                };
                format!("“{shown}”")
            })
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn incomplete_message(&self) -> Option<String> {
        let mut parts = Vec::new();
        let limit = self.entry_limit.borrow();
        let depth = self.too_deep.borrow();
        let unreadable = self.unreadable.borrow();
        if limit.count > 0 {
            parts.push(format!("Entry limit reached in {}.", self.shown_folders(&limit)));
        }
        match depth.count {
            0 => {}
            1 => parts.push(format!("Skipped {} because it was nested too deeply.", self.shown_folders(&depth))),
            n if n == depth.folders.len() => {
                parts.push(format!("Skipped {n} folders nested too deeply: {}.", self.shown_folders(&depth)));
            }
            n => parts.push(format!("Skipped {n} folders nested too deeply, including {}.", self.shown_folders(&depth))),
        }
        if unreadable.count > 0 {
            parts.push(format!("Couldn't read {}.", self.shown_folders(&unreadable)));
            if unreadable.count > unreadable.folders.len() {
                parts.push(format!("{} read errors in total.", unreadable.count));
            }
        }
        if self.timed_out.get() {
            parts.push("Search time limit reached.".into());
        }
        if self.spotlight_timed_out.get() {
            parts.push("Spotlight timed out.".into());
        }
        parts.extend(self.issues.borrow().iter().map(|issue| format!("{issue}.")));
        if parts.is_empty() {
            return None;
        }
        let advice = if unreadable.count > 0 {
            "Check folder access or name a smaller folder."
        } else if limit.count > 0 || depth.count > 0 {
            "Name a smaller folder or use − in Settings → Folders to exclude a subfolder."
        } else {
            "Try a smaller folder."
        };
        Some(format!("Search incomplete. {} {advice}", parts.join(" ")))
    }

    pub fn remaining_time(&self) -> Duration {
        self.timeout.saturating_sub(self.started.elapsed())
    }

    pub fn stopped(&self) -> bool {
        if self.control.is_cancelled() {
            return true;
        }
        if self.remaining_time().is_zero() {
            self.timed_out.set(true);
            return true;
        }
        false
    }

    fn incomplete_lookup(&self) -> NameLookup {
        NameLookup::Incomplete(self.status().unwrap_or_else(|| "Search incomplete. Try a smaller folder.".into()))
    }

    pub fn nearest_dirs(&self, roots: &[PathBuf], name: &str) -> NameLookup {
        self.lookup_name(roots, name, LookupKind::Container)
    }

    fn skip_tree(&self, path: &Path) -> bool {
        if is_bundle(path) {
            return true;
        }
        let markers: &[&str] = match fsx::file_name(path).to_ascii_lowercase().as_str() {
            "node_modules" | "deriveddata" | "__pycache__" | "venv" | ".venv" => return true,
            "pods" => &["Podfile", "Podfile.lock"],
            "target" => &["Cargo.toml", "pom.xml"],
            "vendor" => &["go.mod", "composer.json", "Gemfile"],
            "dist" => &["package.json", "pyproject.toml"],
            "build" => &["package.json", "build.gradle", "build.gradle.kts", "pyproject.toml"],
            _ => return false,
        };
        path.parent().is_some_and(|parent| {
            !self.standard_roots.iter().any(|root| root == parent) && markers.iter().any(|m| parent.join(m).is_file())
        })
    }

    /// Whether a walk from the canonical `root` could yield `path`. An explicitly selected
    /// root is scanned. A read-only probe may cross dependency and build
    /// folders, but never walks into a package through a bare-name search.
    pub(crate) fn walkable(&self, root: &Path, path: &Path, through_skipped: bool) -> bool {
        let Ok(relative) = path.strip_prefix(root) else { return false };
        let mut current = root.to_path_buf();
        let mut parts = relative.iter().peekable();
        while let Some(part) = parts.next() {
            current.push(part);
            let leaf = parts.peek().is_none();
            if (fsx::is_hidden_or_internal(&current) && !((leaf || through_skipped) && is_virtual_env(&current) && current.is_dir()))
                || (!leaf && if through_skipped { is_bundle(&current) } else { self.skip_tree(&current) })
            {
                return false;
            }
        }
        true
    }

    /// The skipped dependency and build roots found in the last name lookup.
    pub fn probe_roots(&self) -> Vec<PathBuf> {
        let mut roots: Vec<_> = self.skipped_folders.borrow().iter().cloned().collect();
        roots.retain(|p| !fsx::is_hidden_or_internal(p) || is_virtual_env(p));
        roots.sort_unstable();
        roots.dedup();
        roots
    }

    pub fn lookup_inside_skipped(&self, roots: &[PathBuf], name: &str) -> Vec<PathBuf> {
        let mut probe = self.probe((self.remaining_time() / 4).min(Duration::from_millis(100)));
        probe.limit = probe.limit.min(1_024);
        probe.depth = probe.depth.min(3);
        // Don't start more folders than the budget can reasonably cover.
        let roots = &roots[..roots.len().min(probe.limit)];
        match probe.lookup(roots, name, LookupKind::Container, true) {
            NameLookup::Found { candidates, .. } => candidates,
            _ => Vec::new(),
        }
    }

    fn within_roots(&self, canon: &Path) -> bool {
        self.allowed.as_ref().is_none_or(|roots| roots.iter().any(|root| canon.starts_with(root)))
    }

    fn allowed_link(&self, path: &Path) -> bool {
        fs::canonicalize(path).is_ok_and(|target| self.within_roots(&target) && !self.exclusions.hides(&target))
    }

    /// Check each level before searching deeper. Container lookups skip
    /// matched trees, and Finder/size only check their immediate files for a
    /// name collision. Item lookups keep searching inside matched folders.
    pub fn lookup_name(&self, roots: &[PathBuf], name: &str, kind: LookupKind) -> NameLookup {
        self.lookup(roots, name, kind, false)
    }

    fn lookup(&self, roots: &[PathBuf], name: &str, kind: LookupKind, through_skipped: bool) -> NameLookup {
        self.skipped_folders.borrow_mut().clear();
        let stopped = || self.control.is_cancelled() || self.remaining_time().is_zero();
        if self.stopped() {
            return self.incomplete_lookup();
        }
        let want = name.to_lowercase();
        let mut queued = HashSet::new();
        let mut level: Vec<_> = roots
            .iter()
            .enumerate()
            .map(|(index, root)| (fsx::canonical(root), root.clone(), index))
            .filter(|(canon, _, _)| queued.insert(canon.clone()))
            .collect();
        if level.iter().any(|(canon, _, _)| !self.within_roots(canon)) {
            self.issue("folder is outside the allowed roots".into());
            return self.incomplete_lookup();
        }
        let root_count = level.len();
        level.sort();
        // An overlapping root is both a starting point and a named entry.
        // Match it before expanding it, even if its parent is several levels
        // away. A single scoped search still looks only inside its root.
        let mut found: Vec<_> = level
            .iter()
            .filter(|(canon, path, _)| {
                fsx::file_name(path).to_lowercase() == want
                    && (!fsx::is_hidden_or_internal(path) || is_virtual_env(path))
                    && !self.exclusions.hides(canon)
                    && canon.ancestors().skip(1).any(|ancestor| queued.contains(ancestor))
            })
            .map(|(canon, path, _)| Entry { path: path.clone(), is_dir: true, canon: canon.clone() })
            .collect();
        let mut matched_dirs: HashSet<_> = found.iter().map(|entry| entry.canon.clone()).collect();
        // Ordinary files don't compete with directories for exact lookup.
        // Keep the usual per-root entry limit for fuzzy matching.
        let mut dir_budget = self.limit;
        let mut match_budget = self.limit;
        let mut fuzzy_budget = vec![self.limit; roots.len()];
        let mut fuzzy_paths = Vec::new();
        let mut fuzzy_truncated = FolderIssues::default();
        let mut budget_exceeded = None;
        let mut decided_folder = false;
        let mut exhaustive = false;
        'levels: for depth in 0.. {
            let mut next = Vec::new();
            for (canon, dir, root_index) in &level {
                if stopped() {
                    break 'levels;
                }
                if depth == 0 && kind == LookupKind::Item && matched_dirs.contains(canon) {
                    next.push((canon.clone(), dir.clone(), *root_index));
                    continue;
                }
                let files_only = kind == LookupKind::ReadOnly && matched_dirs.contains(canon);
                if kind != LookupKind::Item
                    && ((kind == LookupKind::Container && matched_dirs.contains(canon))
                        || canon.ancestors().skip(1).any(|p| matched_dirs.contains(p)))
                {
                    continue;
                }
                if depth > self.depth && !files_only {
                    self.too_deep(dir);
                    continue;
                }
                let Ok(rd) = fs::read_dir(dir) else {
                    self.unreadable(dir);
                    continue;
                };
                let mut relevant = Vec::new();
                let mut truncated_here = false;
                let keep_for_fuzzy = kind != LookupKind::Container && found.is_empty();
                for entry in rd {
                    if stopped() {
                        break 'levels;
                    }
                    self.control.visited.fetch_add(1, Ordering::Relaxed);
                    let capture = keep_for_fuzzy && fuzzy_budget[*root_index] > 0;
                    if keep_for_fuzzy {
                        truncated_here |= !capture;
                        fuzzy_budget[*root_index] = fuzzy_budget[*root_index].saturating_sub(1);
                    }
                    let Ok(entry) = entry else {
                        self.unreadable(dir);
                        continue;
                    };
                    let path = entry.path();
                    let hidden_entry = fsx::is_hidden_or_internal(&path);
                    if hidden_entry && !is_virtual_env(&path) {
                        continue;
                    }
                    let Ok(file_type) = entry.file_type() else {
                        self.unreadable(dir);
                        continue;
                    };
                    let is_dir = file_type.is_dir();
                    if hidden_entry && !is_dir {
                        continue;
                    }
                    if (is_dir && files_only) || (!is_dir && kind == LookupKind::Container) {
                        continue;
                    }
                    let matches = fsx::file_name(&path).to_lowercase() == want;
                    if !is_dir && !matches && !capture {
                        continue;
                    }
                    let canon_path = canon.join(entry.file_name());
                    let hidden = self.exclusions.hides(&canon_path);
                    if hidden && (!is_dir || self.exclusions.prunes(&canon_path)) {
                        continue;
                    }
                    if file_type.is_symlink() && !self.allowed_link(&path) {
                        continue;
                    }
                    if capture && !hidden && !hidden_entry {
                        fuzzy_paths.push(path.clone());
                    }
                    if is_dir || matches {
                        relevant.push((path, canon_path, is_dir, matches && !hidden));
                    }
                }
                if truncated_here {
                    fuzzy_truncated.add(dir);
                }
                relevant.sort_unstable_by(|a, b| a.0.cmp(&b.0));
                for (path, canon_path, is_dir, matches) in relevant {
                    if stopped() {
                        break 'levels;
                    }
                    // A root already queued for expansion can still match as
                    // an entry of its parent.
                    let skipped = is_dir && if through_skipped { is_bundle(&canon_path) } else { self.skip_tree(&canon_path) };
                    if skipped && !self.exclusions.hides(&canon_path) {
                        self.skipped_folders.borrow_mut().push(canon_path.clone());
                    }
                    if is_dir && !skipped && queued.insert(canon_path.clone()) {
                        let Some(remaining) = dir_budget.checked_sub(1) else {
                            budget_exceeded = Some(dir.clone());
                            break 'levels;
                        };
                        dir_budget = remaining;
                        next.push((canon_path.clone(), path.clone(), *root_index));
                    }
                    if matches {
                        if is_dir {
                            matched_dirs.insert(canon_path.clone());
                        } else {
                            let Some(remaining) = match_budget.checked_sub(1) else {
                                budget_exceeded = Some(dir.clone());
                                break 'levels;
                            };
                            match_budget = remaining;
                        }
                        found.push(Entry { path, is_dir, canon: canon_path });
                    }
                }
            }
            if stopped() {
                break;
            }
            decided_folder |= !matched_dirs.is_empty();
            if next.is_empty() {
                exhaustive = self.unreadable.borrow().count == 0 && self.too_deep.borrow().count == 0;
                break;
            }
            next.sort();
            level = next;
        }
        if self.control.is_cancelled() {
            return self.incomplete_lookup();
        }
        let stopped = self.stopped();
        if let Some(folder) = budget_exceeded {
            if !through_skipped && !stopped && !decided_folder && self.status().is_none() && root_count > 1 {
                let candidates = found.into_iter().map(|entry| entry.path).collect();
                return NameLookup::RetryPerRoot(candidates);
            }
            self.entry_limit.borrow_mut().add(&folder);
        }
        if found.is_empty() {
            if !exhaustive {
                return self.incomplete_lookup();
            }
            self.entry_limit.borrow_mut().merge(&fuzzy_truncated);
            fuzzy_paths.sort_unstable();
            return NameLookup::NotFound(fuzzy_paths);
        }
        // An extra search root can expose a descendant before its ancestor.
        // Use canonical ancestry rather than the depth relative to each root.
        let mut seen_matches = HashSet::new();
        found.retain(|entry| {
            let skip = if entry.is_dir { 1 } else { 2 };
            (kind == LookupKind::Item || !entry.canon.ancestors().skip(skip).any(|p| matched_dirs.contains(p)))
                && seen_matches.insert(entry.canon.clone())
        });
        let mut candidates: Vec<_> = found.into_iter().map(|entry| entry.path).collect();
        candidates.sort();
        NameLookup::Found { candidates, needs_confirmation: !exhaustive }
    }

    #[cfg(test)]
    pub fn entries(&self, dir: &Path) -> Arc<Vec<Entry>> {
        self.recursive_entries(dir).entries.clone()
    }

    /// Retry a shared name budget using one walk per root. Keep the skipped
    /// roots so a read-only lookup can still offer to search inside them.
    pub fn scan_lookup_paths(&self, dirs: &[PathBuf], kind: LookupKind) -> Vec<PathBuf> {
        let mut paths = Vec::new();
        let mut skipped = self.skipped_folders.borrow_mut();
        for dir in dirs {
            let scan = self.recursive_entries(dir);
            skipped.extend_from_slice(scan.skipped());
            paths.extend(scan.entries.iter().filter(|e| e.is_dir || kind != LookupKind::Container).map(|e| e.path.clone()));
        }
        paths.sort_unstable();
        paths
    }

    pub(crate) fn recursive_entries(&self, dir: &Path) -> Arc<Scan> {
        let key = fsx::canonical(dir);
        if !self.within_roots(&key) {
            self.issue("folder is outside the allowed roots".into());
            return Arc::default();
        }
        if let Some(scan) = self.cache.borrow().get(&key) {
            return scan.clone();
        }
        // A scan starting at a skipped folder (or below one) must open it,
        // even if the cache already has a scan of its ancestor.
        let ancestor = self
            .cache
            .borrow()
            .iter()
            .filter(|(root, scan)| self.walkable(root, &key, false) && !scan.skipped.iter().any(|p| key.starts_with(p)))
            .max_by_key(|(root, _)| root.components().count())
            .map(|(_, scan)| scan.clone());
        let (scan, complete) = if let Some(ancestor) = ancestor {
            (
                Scan {
                    entries: Arc::new(ancestor.entries.iter().filter(|e| e.canon != key && e.canon.starts_with(&key)).cloned().collect()),
                    skipped: ancestor.skipped.iter().filter(|p| p.starts_with(&key)).cloned().collect(),
                },
                true,
            )
        } else {
            let query = self.isolated(self.remaining_time());
            let scan = query.scan(dir, &key, Some(self.depth));
            let complete = query.status().is_none();
            self.record_issues(&query);
            (scan, complete)
        };
        let scan = Arc::new(scan);
        if complete {
            self.cache.borrow_mut().insert(key, scan.clone());
        }
        scan
    }

    pub(crate) fn direct_entries(&self, dir: &Path) -> Arc<Vec<Entry>> {
        let key = fsx::canonical(dir);
        if !self.within_roots(&key) {
            self.issue("folder is outside the allowed roots".into());
            return Arc::default();
        }
        self.scan(dir, &key, None).entries
    }

    /// Optional hint for a direct-child command. Count only when this small
    /// scan finishes. Its time, depth and total entry budget are independent
    /// of the scan that selects the files to change.
    pub(crate) fn count_nested(&self, subdirs: &[Entry], mut matches: impl FnMut(&Path) -> Option<bool>) -> Option<usize> {
        if self.depth == 0 {
            return None;
        }
        let probe = self.probe((self.remaining_time() / 4).min(Duration::from_millis(50)));
        let mut walk = Walk { remaining: self.limit.min(512), ..Walk::default() };
        let mut seen = HashSet::new();
        let mut count = 0;
        for dir in subdirs {
            if probe.stopped() {
                return None;
            }
            if probe.skip_tree(&dir.canon) {
                continue;
            }
            // Charge the subfolder itself, including empty subfolders.
            if walk.remaining == 0 {
                return None;
            }
            walk.remaining -= 1;
            probe.walk(&dir.path, &dir.canon, Some(self.depth.min(3) - 1), &mut walk);
            if probe.status().is_some() {
                return None;
            }
            for entry in walk.entries.drain(..) {
                if !entry.is_dir && seen.insert(entry.path.clone()) {
                    if probe.stopped() {
                        return None;
                    }
                    count += usize::from(matches(&entry.path)?);
                }
            }
            walk.skipped.clear();
        }
        Some(count)
    }

    fn scan(&self, dir: &Path, canon: &Path, depth: Option<usize>) -> Scan {
        let mut walk = Walk { remaining: self.limit, ..Walk::default() };
        self.walk(dir, canon, depth, &mut walk);
        Scan { entries: Arc::new(walk.entries), skipped: walk.skipped }
    }

    /// `canon` is `dir` canonicalized; children extend it without another
    /// filesystem call per entry.
    fn walk(&self, dir: &Path, canon: &Path, depth: Option<usize>, walk: &mut Walk) {
        if self.stopped() {
            return;
        }
        let rd = match fs::read_dir(dir) {
            Ok(rd) => rd,
            Err(_) => {
                self.unreadable(dir);
                return;
            }
        };
        let mut entries = Vec::new();
        for entry in rd {
            if self.stopped() {
                return;
            }
            match entry {
                Ok(entry) => entries.push(entry),
                Err(_) => self.unreadable(dir),
            }
            if entries.len() > walk.remaining {
                break;
            }
        }
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            if self.stopped() {
                return;
            }
            if walk.remaining == 0 {
                self.entry_limit.borrow_mut().add(dir);
                return;
            }
            walk.remaining -= 1;
            self.control.visited.fetch_add(1, Ordering::Relaxed);
            let path = e.path();
            let hidden_entry = fsx::is_hidden_or_internal(&path);
            // Only recursive scans consider hidden virtual environment dirs.
            if hidden_entry && (depth.is_none() || !is_virtual_env(&path)) {
                continue;
            }
            let ft = match e.file_type() {
                Ok(ft) => ft,
                Err(_) => {
                    self.unreadable(dir);
                    continue;
                }
            };
            let is_dir = ft.is_dir();
            let canon_path = canon.join(e.file_name());
            let hidden = !self.exclusions.is_empty() && self.exclusions.hides(&canon_path);
            if !is_dir {
                if !hidden && !hidden_entry && (!ft.is_symlink() || self.allowed_link(&path)) {
                    walk.entries.push(Entry { path, is_dir, canon: canon_path });
                }
                continue;
            }
            if hidden && self.exclusions.prunes(&canon_path) {
                continue;
            }
            if !hidden {
                walk.entries.push(Entry { path: path.clone(), is_dir, canon: canon_path.clone() });
            }
            if let Some(depth) = depth {
                if self.skip_tree(&canon_path) {
                    if !hidden {
                        walk.skipped.push(canon_path);
                    }
                } else if depth == 0 {
                    self.too_deep(&path);
                } else {
                    self.walk(&path, &canon_path, Some(depth - 1), walk);
                }
            }
        }
    }
}
