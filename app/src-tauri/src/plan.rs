//! Turns a parsed intent into a concrete plan: resolves names against the
//! disk, expands filters into file lists, checks every path against the
//! allowed folders, and picks non-clobbering names for every output.

use crate::fsx::{self, Roots, file_name, human_size, split_name, unique_path};
use crate::history::{Entry, EntryState, History};
use crate::model::{
    ARCHIVE_EXTS, AUDIO_EXTS, Action, AmbiguityReason, DOC_EXTS, DateField, DayRef, FileType, Filter, IMAGE_EXTS, INSTALLER_EXTS,
    ImageFormat, InfoRow, Intent, Source, VIDEO_EXTS,
};
use crate::{pdf, search};
use chrono::{DateTime, Datelike, Local, NaiveDate};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, SystemTime};

#[cfg(test)]
#[path = "plan/model_audit.rs"]
mod model_audit;

/// Why planning stopped early.
#[derive(Debug)]
pub enum Stop {
    Ambiguous {
        name: String,
        candidates: Vec<PathBuf>,
        reason: AmbiguityReason,
        note: Option<String>,
    },
    MissingTransferItem {
        #[cfg_attr(not(feature = "tiny-model"), expect(dead_code))]
        name: String,
        message: String,
    },
    SearchIncomplete(String),
    NothingToChange(String),
    Problem(String),
}

fn problem<T>(msg: impl Into<String>) -> Result<T, Stop> {
    Err(Stop::Problem(msg.into()))
}

fn ambiguous(name: &str, candidates: Vec<PathBuf>, incomplete: bool) -> Stop {
    let reason = if incomplete {
        AmbiguityReason::IncompleteSearch
    } else if candidates.len() == 1 {
        AmbiguityReason::SuggestedMatch
    } else {
        AmbiguityReason::MultipleMatches
    };
    Stop::Ambiguous { name: name.into(), candidates, reason, note: None }
}

/// Conservative path overlap used by the chain/shortcut dependency guard.
fn overlaps(a: &Path, b: &Path) -> bool {
    // Conservative on case-sensitive volumes too: a preview must not
    // depend on two spellings denoting different filesystem objects.
    let a = PathBuf::from(fsx::canonical(a).to_string_lossy().to_lowercase());
    let b = PathBuf::from(fsx::canonical(b).to_string_lossy().to_lowercase());
    a.starts_with(&b) || b.starts_with(&a)
}

/// True when two paths denote the same folder, for folders that may only be
/// planned and not exist on disk yet. `overlaps` is symmetric (an ancestor
/// overlaps its descendant), so it can't answer "is this the created folder".
fn same_path(a: &Path, b: &Path) -> bool {
    fsx::canonical(a).to_string_lossy().to_lowercase() == fsx::canonical(b).to_string_lossy().to_lowercase()
}

/// What earlier steps of one plan (a chain or a shortcut) touched, so a
/// later step can be refused before it reads a folder those steps write.
#[derive(Default)]
struct Dependencies {
    files: Vec<PathBuf>,
    dirs: Vec<PathBuf>,
    /// Folders written into, and whether the write was a CreateFolder.
    writes: Vec<(PathBuf, bool)>,
}

/// One command's contribution: its title when it added steps to the shared
/// plan, or a finished result for read-only commands and undo.
enum Step {
    Changes(String),
    Finished(Planned),
}

pub enum Planned {
    Steps {
        title: String,
        actions: Vec<Action>,
        notes: Vec<Option<String>>,
        warnings: Vec<String>,
        undo_of: Option<u64>,
    },
    Info {
        title: String,
        summary: String,
        rows: Vec<InfoRow>,
        total: usize,
        listed: Option<Listed>,
    },
    /// Read-only, runs right away without a preview.
    Direct {
        action: Action,
        title: String,
    },
}

pub struct Listed {
    pub paths: Vec<PathBuf>,
    pub label: String,
    pub folder: Option<PathBuf>,
}

pub struct Planner<'a> {
    roots: &'a Roots,
    /// Name -> path the user picked when a name was ambiguous.
    choices: &'a HashMap<String, PathBuf>,
    reserved: HashSet<PathBuf>,
    actions: Vec<Action>,
    notes: Vec<Option<String>>,
    warnings: Vec<String>,
    planned_dirs: HashSet<PathBuf>,
    chain_input: Option<PathBuf>,
    now: DateTime<Local>,
    search: search::Search,
    scope: Option<crate::scope::Snapshot>,
    #[cfg(test)]
    index_hits: Option<IndexLookup>,
    #[cfg(test)]
    chain_action_ends: Option<&'a mut Vec<usize>>,
}

#[cfg(test)]
type IndexLookup = fn(&str, &[PathBuf], &search::Search) -> Vec<PathBuf>;

/// How many Spotlight answers to look at, and how long to wait for them.
const MAX_INDEXED: usize = 5_000;
const SPOTLIGHT_TIMEOUT: Duration = Duration::from_secs(2);
const SKIPPED_TREES_NOTE: &str = "Some folders were skipped.";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Expansion {
    Transfer,
    DirectChanges,
    RecursiveChanges,
    ReadOnly,
}

struct ExpandedSource {
    paths: Vec<PathBuf>,
    label: String,
    folder: Option<PathBuf>,
    nested: usize,
    skipped_note: String,
}

impl<'a> Planner<'a> {
    pub fn new(roots: &'a Roots, choices: &'a HashMap<String, PathBuf>) -> Self {
        Planner {
            roots,
            choices,
            reserved: HashSet::new(),
            actions: Vec::new(),
            notes: Vec::new(),
            warnings: Vec::new(),
            planned_dirs: HashSet::new(),
            chain_input: None,
            now: Local::now(),
            search: search::Search::default().with_roots(roots),
            scope: None,
            #[cfg(test)]
            index_hits: None,
            #[cfg(test)]
            chain_action_ends: None,
        }
    }

    pub fn with_control(mut self, control: search::Control) -> Self {
        self.search = search::Search::new(control).with_roots(self.roots);
        self
    }

    pub fn with_scope(mut self, scope: Option<crate::scope::Snapshot>) -> Self {
        self.scope = scope;
        self
    }

    pub fn plan(mut self, intent: &Intent, history: &History) -> Result<Planned, Stop> {
        if let Some(scope) = &self.scope {
            scope.validate().map_err(Stop::Problem)?;
        }
        let result = self.step(intent, history);
        if self.search.control.is_cancelled() {
            return problem("Search cancelled.");
        }
        let title = match result? {
            Step::Changes(title) => title,
            Step::Finished(Planned::Info { title, summary, rows, total, .. })
                if let search::Completeness::Partial(message) = self.search.completeness() =>
            {
                return Ok(Planned::Info { title, summary: format!("{message} Partial results. {summary}"), rows, total, listed: None });
            }
            Step::Finished(done) => return Ok(done),
        };
        if self.actions.is_empty() {
            let mut msg = "Nothing to change.".to_string();
            if let Some(w) = self.warnings.first() {
                msg = format!("{msg} {w}");
            }
            return Err(Stop::NothingToChange(msg));
        }
        Ok(Planned::Steps { title, actions: self.actions, notes: self.notes, warnings: self.warnings, undo_of: None })
    }

    /// Plans a saved shortcut: every command goes into one preview, confirmed
    /// once and undone as one change. A command with nothing to do becomes a
    /// warning. An idle chain aborts a shortcut with multiple commands, and
    /// any planning failure aborts the shortcut.
    pub fn plan_shortcut(mut self, name: &str, commands: &[(String, Intent)], history: &History) -> Result<Planned, Stop> {
        let mut skipped = Vec::new();
        let mut deps = Dependencies::default();
        for (index, (text, intent)) in commands.iter().enumerate() {
            // A checkpoint of the whole draft, so a dropped command leaves
            // nothing behind, its reserved output names included.
            let (actions, notes, warnings, dirs, reserved) =
                (self.actions.len(), self.notes.len(), self.warnings.len(), self.planned_dirs.clone(), self.reserved.clone());
            let result = self.step(intent, history);
            if self.search.control.is_cancelled() {
                return problem("Search cancelled.");
            }
            let why = match result {
                Ok(Step::Changes(_)) if self.actions.len() == actions => {
                    self.warnings.get(warnings).cloned().unwrap_or_else(|| "nothing to change".into())
                }
                Ok(Step::Changes(_)) => {
                    // A shortcut is one preview: give later commands the same
                    // dependency guard “then” chains get.
                    self.check_dependencies(actions, index + 1, &mut deps)?;
                    continue;
                }
                Ok(Step::Finished(_)) => {
                    return problem(format!("“{text}” only shows information, so it can't be part of “{name}”."));
                }
                Err(Stop::NothingToChange(why)) => why,
                Err(stop) => return Err(stop),
            };
            // A shortcut with other commands cannot proceed past an idle chain.
            if commands.len() > 1 && matches!(intent, Intent::Chain { .. }) {
                return problem(why);
            }
            self.actions.truncate(actions);
            self.notes.truncate(notes);
            self.warnings.truncate(warnings);
            self.planned_dirs = dirs;
            self.reserved = reserved;
            skipped.push(format!("Skipped “{text}”: {why}"));
        }
        if self.actions.is_empty() {
            return Err(Stop::NothingToChange(format!("Nothing to change for “{name}”. {}", skipped.join(" "))));
        }
        let n = commands.len();
        let count = if skipped.is_empty() { n.to_string() } else { format!("{} of {n}", n - skipped.len()) };
        self.warnings.extend(skipped);
        Ok(Planned::Steps {
            title: format!("{}: {count} {}", capitalize(name), plural(n, "command", "commands")),
            actions: self.actions,
            notes: self.notes,
            warnings: self.warnings,
            undo_of: None,
        })
    }

    fn step(&mut self, intent: &Intent, history: &History) -> Result<Step, Stop> {
        let start = self.actions.len();
        let title = match intent {
            Intent::Chain { commands } => self.chain(commands, history)?,
            Intent::Move { src, to } => self.transfer(src, to, false)?,
            Intent::Copy { src, to } => self.transfer(src, to, true)?,
            Intent::Rename { src, new_name } => self.rename_one(src, new_name)?,
            Intent::AddPrefix { src, prefix } => {
                let p = prefix.clone();
                self.batch_rename(src, "Add prefix", move |name, _| Some(format!("{p}{name}")))?
            }
            Intent::AddDatePrefix { src } => self.batch_rename(src, "Add date prefix", |name, path| {
                let date = created(path)?.format("%Y-%m-%d").to_string();
                let starts_dated = name.len() >= 10 && NaiveDate::parse_from_str(name.get(..10).unwrap_or(""), "%Y-%m-%d").is_ok();
                (!starts_dated).then(|| format!("{date} {name}"))
            })?,
            Intent::ReplaceInNames { src, find, replace } => {
                let (f, r) = (find.clone(), replace.clone());
                self.batch_rename(src, "Rename", move |name, _| replace_in_stem(name, &f, &r))?
            }
            Intent::Lowercase { src } => self.batch_rename(src, "Lowercase", |name, _| Some(name.to_lowercase()))?,
            Intent::Number { src, base, width } => self.number(src, base, *width)?,
            Intent::CreateFolders { names, parent } => self.create_folders(names, parent.as_deref())?.0,
            Intent::SortByType { folder, parent, include_subfolders } => self.sort(folder, parent.as_deref(), true, *include_subfolders)?,
            Intent::SortByMonth { folder, parent, include_subfolders } => {
                self.sort(folder, parent.as_deref(), false, *include_subfolders)?
            }
            Intent::ConvertImage { src, format } => self.convert(src, *format)?,
            Intent::ResizeImage { src, width } => self.resize(src, *width)?,
            Intent::MergePdf { src, output } => self.merge(src, output)?,
            Intent::SplitPdf { src } => self.split(src)?,
            Intent::ExtractPages { src, from, to, output } => self.extract(src, *from, *to, output.as_deref())?,
            Intent::ImagesToPdf { src, output } => self.images_to_pdf(src, output.as_deref())?,
            Intent::Zip { src, output } => self.zip(src, output.as_deref())?,
            Intent::Unzip { src, destination } => self.unzip(src, destination.as_deref())?,
            Intent::Trash { src } => self.trash(src)?,
            Intent::List { src } => return self.list(src).map(Step::Finished),
            Intent::Count { src } => return self.count(src).map(Step::Finished),
            Intent::Size { folder } => return self.size(folder).map(Step::Finished),
            Intent::Duplicates { folder } => return self.duplicates(folder).map(Step::Finished),
            Intent::Reveal { target } => {
                let path = self.resolve_any(target)?;
                let title = format!("Opened {} in Finder", file_name(&path));
                return Ok(Step::Finished(Planned::Direct { action: Action::OpenInFinder { reveal: !path.is_dir(), path }, title }));
            }
            Intent::RecentActions => return Ok(Step::Finished(recent(history, self.roots))),
            Intent::Undo => {
                let Some(entry) = history.last_undoable() else { return problem("Nothing to undo.") };
                return plan_undo(entry, self.roots).map(Step::Finished);
            }
        };
        if !matches!(intent, Intent::Chain { .. }) {
            for action in &self.actions[start..] {
                self.roots.check_whole_tree_exclusions(action).map_err(Stop::Problem)?;
            }
        }
        Ok(Step::Changes(title))
    }

    fn note_dates(&mut self, filter: &Filter) {
        if let Some(note) = date_note(filter)
            && !self.warnings.iter().any(|w| w == note)
        {
            self.warnings.push(note.into());
        }
    }

    fn push(&mut self, a: Action, note: Option<String>) {
        self.actions.push(a);
        self.notes.push(note);
    }

    fn check(&self, p: &Path) -> Result<(), Stop> {
        if self.roots.is_allowed_entry(p) {
            Ok(())
        } else if self.roots.configured_root_changed(p) {
            problem(format!("The folder {} moved or was remounted. Restart Voxi or update Settings → Folders.", self.roots.display(p)))
        } else {
            problem(format!("{} is outside the folders Voxi may change.", self.roots.display(p)))
        }
    }

    /// A non-clobbering path in `dir`, with a note when the name had to change.
    fn target(&mut self, dir: &Path, name: &str) -> (PathBuf, Option<String>) {
        let p = unique_path(dir, name, &mut self.reserved);
        let note = (file_name(&p) != name).then(|| format!("“{name}” already exists, so this becomes “{}”", file_name(&p)));
        (p, note)
    }

    /// Adds CreateFolder steps for `dir` and any missing parents.
    fn ensure_dir(&mut self, dir: &Path) -> Result<(), Stop> {
        self.check(dir)?;
        if dir.is_dir() || self.planned_dirs.contains(dir) {
            return Ok(());
        }
        if let Some(parent) = dir.parent() {
            self.ensure_dir(parent)?;
        }
        self.planned_dirs.insert(dir.to_path_buf());
        self.push(Action::CreateFolder { path: dir.to_path_buf() }, Some("New folder".into()));
        Ok(())
    }

    // ---- Name resolution -------------------------------------------------

    /// Resolves a spoken folder name. With `create_in`, a folder that doesn't
    /// exist is placed there (and created by the plan) instead of failing.
    fn resolve_folder(&self, name: &str, create_in: Option<&Path>) -> Result<PathBuf, Stop> {
        self.resolve_folder_with_lookup(name, create_in, false)
    }

    fn resolve_folder_with_lookup(&self, name: &str, create_in: Option<&Path>, read_only: bool) -> Result<PathBuf, Stop> {
        let name = name.trim().trim_end_matches('/');
        if fsx::has_parent_ref(Path::new(name)) {
            return problem("Paths with “..” aren't allowed.");
        }
        if let Some(p) = self.choices.get(name) {
            // A picked folder is a path like any other: a stale or hostile
            // choice must not skip the root/symlink check.
            self.check(p)?;
            return Ok(p.clone());
        }
        if name.starts_with('/') || name.starts_with('~') {
            let p = fsx::expand_tilde(name, &self.roots.home);
            self.check(&p)?;
            return Ok(p);
        }
        let (first, rest) = match name.split_once('/') {
            Some((a, b)) => (a, Some(b)),
            None => (name, None),
        };
        let base = if let Some(p) = self.choices.get(first) {
            // A picked prefix is checked itself, so a stale or hostile choice
            // can't smuggle an excluded or outside path into the plan.
            self.check(p)?;
            p.clone()
        } else if let Some(p) = self.roots.named(first) {
            p.clone()
        } else {
            self.lookup_folder(first, first, &self.roots.search_roots(), create_in, read_only)?
        };
        let full = match rest {
            Some(r) => base.join(r),
            None => base,
        };
        self.check(&full)?;
        if create_in.is_none() && !full.is_dir() {
            return problem(format!("Couldn't find a folder named “{name}”."));
        }
        Ok(full)
    }

    fn lookup_folder(
        &self,
        name: &str,
        label: &str,
        roots: &[PathBuf],
        create_in: Option<&Path>,
        read_only: bool,
    ) -> Result<PathBuf, Stop> {
        let disk = self.search.isolated(self.search.remaining_time());
        let (mut found, needs_confirmation) = match disk.nearest_dirs(roots, name) {
            search::NameLookup::Found { candidates, needs_confirmation } => (candidates, needs_confirmation),
            search::NameLookup::NotFound(_) => (Vec::new(), false),
            search::NameLookup::RetryPerRoot(mut candidates) => {
                candidates.extend(disk.scan_lookup_paths(roots, search::LookupKind::Container));
                (Self::name_candidates(name, candidates, search::LookupKind::Container), disk.status().is_some())
            }
            search::NameLookup::Incomplete(message) => {
                disk.record_failure();
                return Err(Stop::SearchIncomplete(message));
            }
        };
        if needs_confirmation {
            found.retain(|path| path.is_dir() && self.check(path).is_ok());
            return self.partial_lookup(label, found, &disk, disk.status());
        }
        if found.is_empty() && read_only && create_in.is_none() {
            self.confirm_skipped_folder(&disk, name, roots)?;
        }
        match found.len() {
            1 => Ok(found.remove(0)),
            0 => match create_in {
                Some(parent) => Ok(parent.join(name)),
                None => problem(format!("Couldn't find a folder named “{label}”.")),
            },
            _ => Err(ambiguous(label, found, false)),
        }
    }

    fn confirm_skipped_folder(&self, disk: &search::Search, name: &str, dirs: &[PathBuf]) -> Result<(), Stop> {
        let mut roots = disk.probe_roots();
        roots.retain(|p| !self.in_package(p));
        if roots.is_empty() {
            return Ok(());
        }
        let mut candidates = disk.lookup_inside_skipped(&roots, name);
        // A package found inside a skipped tree may match itself; nothing
        // inside a package ever does.
        let outside_packages = |p: &PathBuf| {
            [p.as_path(), &fsx::canonical(p)].into_iter().all(|path| path.parent().is_none_or(|parent| !self.in_package(parent)))
        };
        candidates.retain(|p| p.is_dir() && outside_packages(p) && self.check(p).is_ok());
        if candidates.is_empty() {
            // This probe can suggest a folder, and every suggestion already
            // needs confirmation because the normal lookup skipped the tree.
            let index = self.search.isolated(self.search.remaining_time());
            let hits = self.indexed_paths(name, dirs, &index);
            let hits = self.verified_hits(hits, &roots, true, &index);
            candidates = Self::name_candidates(
                name,
                hits.into_iter().filter(|p| p.is_dir() && outside_packages(p)).collect(),
                search::LookupKind::Container,
            );
        }
        if candidates.is_empty() { Ok(()) } else { Err(ambiguous(name, candidates, true)) }
    }

    /// Resolves a file or folder by name, searching the allowed folders.
    fn resolve_item(&self, name: &str, folder: Option<&str>) -> Result<PathBuf, Stop> {
        self.resolve_named_item(name, folder, search::LookupKind::Item, false)
    }

    fn resolve_named_item(
        &self,
        name: &str,
        folder: Option<&str>,
        kind: search::LookupKind,
        transfer_source: bool,
    ) -> Result<PathBuf, Stop> {
        let read_only = kind == search::LookupKind::ReadOnly;
        if name == "\0chain-output"
            && folder.is_none()
            && let Some(path) = &self.chain_input
        {
            return Ok(path.clone());
        }
        if name.trim().is_empty() {
            return problem("Name the file or folder to use.");
        }
        if let Some(scope) = &self.scope {
            let paths = scope.paths();
            if let Some(picked) = self.choices.get(name) {
                if !paths.contains(picked) {
                    return problem("That file isn't in the searched list.");
                }
                self.check(picked)?;
                return Ok(picked.clone());
            }
            let mut found = if Path::new(name).is_absolute() {
                paths.into_iter().filter(|p| p == Path::new(name)).collect()
            } else {
                closest(name, paths)
            };
            return match found.len() {
                1 if Path::new(name).is_absolute() || strong_name_match(name, &found[0]) => {
                    let path = found.remove(0);
                    self.check(&path)?;
                    Ok(path)
                }
                0 => problem("That file isn't in the searched list. Name another source folder to search again."),
                _ => Err(ambiguous(name, found, false)),
            };
        }
        if fsx::has_parent_ref(Path::new(name)) {
            return problem("Paths with “..” aren't allowed.");
        }
        if let Some(p) = self.choices.get(name) {
            self.check(p)?;
            return if p.exists() { Ok(p.clone()) } else { problem(format!("Couldn't find {}.", self.roots.display(p))) };
        }
        let typed = name.trim_start();
        if typed.starts_with(['/', '~']) {
            let p = fsx::expand_tilde(typed, &self.roots.home);
            self.check(&p)?;
            return if p.exists() { Ok(p) } else { problem(format!("Couldn't find {}.", self.roots.display(&p))) };
        }
        if let Some((dir, file)) = name.rsplit_once('/') {
            let d = self.resolve_folder_with_lookup(dir, None, read_only)?;
            let p = d.join(file);
            self.check(&p)?;
            return if p.exists() { Ok(p) } else { problem(format!("Couldn't find “{file}” in {}.", self.roots.display(&d))) };
        }
        if folder.is_none()
            && let Some(path) = self.roots.named(name)
        {
            self.check(path)?;
            return Ok(path.clone());
        }
        let dirs = match folder {
            Some(f) => vec![self.resolve_folder_with_lookup(f, None, read_only)?],
            None => self.roots.search_roots(),
        };
        // A lookup can ask the user to confirm a verified candidate even
        // when the walk is partial. Keep that state separate from file sets.
        let time = self.search.remaining_time();
        let reserve = if folder.is_none() { SPOTLIGHT_TIMEOUT.min(time / 4) } else { Duration::ZERO };
        let disk = self.search.isolated(time - reserve);
        let (mut paths, mut incomplete) = match disk.lookup_name(&dirs, name, kind) {
            search::NameLookup::Found { candidates, needs_confirmation: false } => return self.pick_exact(name, candidates),
            search::NameLookup::Found { candidates, needs_confirmation: true } => {
                (candidates, Some(disk.status().unwrap_or_else(|| "Search incomplete. Try a smaller folder.".into())))
            }
            search::NameLookup::NotFound(entries) => (entries, disk.status()),
            search::NameLookup::RetryPerRoot(mut candidates) => {
                candidates.extend(disk.scan_lookup_paths(&dirs, kind));
                (candidates, disk.status())
            }
            search::NameLookup::Incomplete(message) => (Vec::new(), Some(message)),
        };
        if incomplete.is_some() {
            paths.retain(|p| p.exists() && self.check(p).is_ok());
        }
        if incomplete.is_some() && folder.is_none() {
            let (hits, index) = self.search_index(name, &dirs);
            index.record_failure();
            paths.extend(hits);
        }
        let mut found = Self::name_candidates(name, paths, kind);
        if found.is_empty() && incomplete.is_none() {
            let (hits, index) = self.search_index(name, &dirs);
            found = Self::name_candidates(name, hits, kind);
            // An index miss leaves the complete disk result authoritative.
            if !found.is_empty() {
                incomplete = index.status();
                if incomplete.is_some() {
                    index.record_failure();
                }
            }
        }
        if self.search.control.is_cancelled() {
            return problem("Search cancelled.");
        }
        if found.is_empty() && read_only {
            self.confirm_skipped_folder(&disk, name, &dirs)?;
        }
        if incomplete.is_some() {
            return self.partial_lookup(name, found, &disk, incomplete);
        }
        match found.len() {
            1 if strong_name_match(name, &found[0]) => {
                let path = found.remove(0);
                self.check(&path)?;
                Ok(path)
            }
            0 => {
                let wher = match folder {
                    Some(f) => f.to_string(),
                    None => "Desktop, Downloads, Documents, or Pictures".into(),
                };
                let message = format!("Couldn't find “{name}” in {wher}.");
                if transfer_source { Err(Stop::MissingTransferItem { name: name.into(), message }) } else { problem(message) }
            }
            _ => Err(ambiguous(name, found, false)),
        }
    }

    fn partial_lookup<T>(&self, name: &str, candidates: Vec<PathBuf>, disk: &search::Search, note: Option<String>) -> Result<T, Stop> {
        if self.search.control.is_cancelled() {
            return problem("Search cancelled.");
        }
        disk.record_failure();
        if candidates.is_empty() {
            return Err(Stop::SearchIncomplete(note.unwrap_or_else(|| "Search incomplete. Try a smaller folder.".into())));
        }
        Err(Stop::Ambiguous { name: name.into(), candidates, reason: AmbiguityReason::IncompleteSearch, note })
    }

    fn name_candidates(name: &str, paths: Vec<PathBuf>, kind: search::LookupKind) -> Vec<PathBuf> {
        let mut exact = exact_paths(name, paths.iter().cloned());
        if exact.is_empty() && kind != search::LookupKind::Container {
            return closest(name, paths);
        }
        if kind != search::LookupKind::Item {
            let is_dir = |p: &Path| fs::symlink_metadata(p).is_ok_and(|m| m.is_dir());
            let matched_dirs: HashSet<_> = exact.iter().filter(|p| is_dir(p)).map(|p| entry_identity(p)).collect();
            exact.retain(|p| !entry_identity(p).ancestors().skip(if is_dir(p) { 1 } else { 2 }).any(|a| matched_dirs.contains(a)));
        }
        exact.sort_unstable();
        exact
    }

    fn pick_exact(&self, name: &str, mut candidates: Vec<PathBuf>) -> Result<PathBuf, Stop> {
        if candidates.len() != 1 {
            return Err(ambiguous(name, candidates, false));
        }
        let path = candidates.remove(0);
        self.check(&path)?;
        Ok(path)
    }

    /// A file, or a folder by its spoken name (for "open Downloads").
    fn resolve_any(&self, name: &str) -> Result<PathBuf, Stop> {
        let typed = name.trim_start();
        if typed.starts_with(['/', '~']) {
            return self.resolve_named_item(typed, None, search::LookupKind::ReadOnly, false);
        }
        if let Some(p) = self.roots.named(name) {
            self.check(p)?;
            return Ok(p.clone());
        }
        self.resolve_named_item(name, None, search::LookupKind::ReadOnly, false)
    }

    fn indexed_paths(&self, name: &str, dirs: &[PathBuf], search: &search::Search) -> Vec<PathBuf> {
        if search.stopped() {
            return Vec::new();
        }
        #[cfg(test)]
        {
            self.index_hits.map_or_else(Vec::new, |lookup| lookup(name, dirs, search))
        }
        #[cfg(not(test))]
        spotlight_checked(name, dirs, Some(search))
    }

    fn search_index(&self, name: &str, dirs: &[PathBuf]) -> (Vec<PathBuf>, search::Search) {
        let index = self.search.isolated(self.search.remaining_time());
        let hits = self.indexed_paths(name, dirs, &index);
        let hits = self.verified_hits(hits, dirs, false, &index);
        (hits, index)
    }

    /// Keeps the Spotlight answers Voxi could have reached itself. The index
    /// can lag behind the disk and knows nothing of the allowed folders, so
    /// each answer is checked like a typed path.
    fn verified_hits(&self, hits: Vec<PathBuf>, dirs: &[PathBuf], through_skipped: bool, search: &search::Search) -> Vec<PathBuf> {
        let roots: Vec<_> = dirs.iter().map(|d| (d, fsx::canonical(d))).collect();
        hits.into_iter()
            .take_while(|_| !search.stopped())
            .filter(|p| p.exists() && self.check(p).is_ok())
            .filter(|p| {
                let identity = entry_identity(p);
                roots.iter().any(|(root, canonical)| {
                    let Ok(relative) = p.strip_prefix(root).or_else(|_| p.strip_prefix(canonical)) else { return false };
                    let lexical = canonical.join(relative);
                    &identity != canonical
                        && search.walkable(canonical, &identity, through_skipped)
                        && (lexical == identity || search.walkable(canonical, &lexical, through_skipped))
                })
            })
            .collect()
    }

    /// The folders a set looks in, and a short label for titles.
    fn set_folders(&self, folder: Option<&str>, filter: &Filter, read_only: bool) -> Result<(Vec<PathBuf>, String), Stop> {
        match folder {
            Some(f) => {
                let p = self.resolve_folder_with_lookup(f, None, read_only)?;
                let label = file_name(&p);
                Ok((vec![p], label))
            }
            // Screenshots can be anywhere: search every allowed folder.
            None if filter.file_type == Some(FileType::Screenshots) => Ok((self.roots.search_roots(), "your folders".into())),
            // Deliberately narrow: widening an unscoped search would silently
            // change which files a command touches. Make the limit visible in
            // read-only answers instead of quietly skipping other roots.
            None => Ok((vec![self.roots.downloads(), self.roots.desktop()], "Downloads and Desktop".into())),
        }
    }

    /// Extra summary text for read-only answers to unscoped searches.
    fn search_scope_hint(src: &Source) -> &'static str {
        match src {
            Source::Set { folder: None, filter } if filter.file_type != Some(FileType::Screenshots) => {
                "Searched only Downloads and Desktop. Say “in Documents” or “in Pictures” to search there. "
            }
            _ => "",
        }
    }

    fn includes_nested_files(folder: Option<&str>, filter: &Filter) -> bool {
        filter.include_subfolders || filter.only_subfolders || (folder.is_none() && filter.file_type == Some(FileType::Screenshots))
    }

    fn expand_source(&mut self, src: &Source, expansion: Expansion) -> Result<ExpandedSource, Stop> {
        let read_only = expansion == Expansion::ReadOnly;
        let direct_by_default = matches!(expansion, Expansion::DirectChanges | Expansion::Transfer);
        match src {
            Source::Item { name, folder } => {
                let kind = if read_only { search::LookupKind::ReadOnly } else { search::LookupKind::Item };
                let p = self.resolve_named_item(name, folder.as_deref(), kind, expansion == Expansion::Transfer)?;
                let label = file_name(&p);
                Ok(ExpandedSource { paths: vec![p], label, folder: None, nested: 0, skipped_note: String::new() })
            }
            Source::Set { folder, filter } => {
                let contains = filter.name_contains.as_deref().map(NameNeedle::new).transpose()?;
                let starts = filter.name_starts_with.as_deref().map(NameNeedle::new).transpose()?;
                if filter.ext.as_ref().is_some_and(|exts| exts.is_empty() || exts.iter().any(|ext| ext.trim().is_empty())) {
                    return problem("Enter a file extension for the filter, such as .txt.");
                }
                if let Some(scope) = &self.scope {
                    if folder.is_some() {
                        return problem("An explicit source folder starts a new search.");
                    }
                    let boundary = filter
                        .only_subfolders
                        .then(|| crate::scope::subfolder_boundary(scope.folder.as_deref()))
                        .transpose()
                        .map_err(Stop::Problem)?;
                    let mut paths = Vec::new();
                    for path in scope.paths() {
                        self.check(&path)?;
                        if boundary.is_none_or(|folder| crate::scope::is_nested_in(&path, folder))
                            && matches_prepared(&path, filter, self.now, contains.as_ref(), starts.as_ref())
                        {
                            if !read_only && path.parent().is_some_and(|parent| self.in_package(parent)) {
                                return problem("File sets can't change items inside a macOS package.");
                            }
                            paths.push(path);
                        }
                    }
                    let folder = scope.folder.clone();
                    self.note_dates(filter);
                    return Ok(ExpandedSource {
                        paths,
                        label: "the previous results".into(),
                        folder,
                        nested: 0,
                        skipped_note: String::new(),
                    });
                }
                let (dirs, label) = self.set_folders(folder.as_deref(), filter, read_only)?;
                let direct = direct_by_default && !Self::includes_nested_files(folder.as_deref(), filter);
                let folder = folder.as_ref().and_then(|_| dirs.first()).map(|dir| fsx::canonical(dir));
                if !read_only
                    && !direct
                    && let Some(d) = dirs.iter().find(|d| self.in_hidden_tree(d) || self.in_package(d))
                {
                    return problem(format!(
                        "“Including subfolders” can't reach inside {}. Recursive changes are blocked in hidden folders, Voxi's {} folder, and macOS packages.",
                        file_name(d),
                        fsx::TRASH_DIR
                    ));
                }
                let top_level: HashSet<_> =
                    dirs.iter().filter(|_| direct_by_default && filter.only_subfolders).map(|d| fsx::canonical(d)).collect();
                let now = self.now;
                let inspect = filter.larger_than_mb.is_some() || filter.smaller_than_mb.is_some() || has_date_filter(filter);
                let matches_file = |p: &Path| {
                    if inspect && fs::metadata(p).is_err() {
                        return None;
                    }
                    Some(matches_prepared(p, filter, now, contains.as_ref(), starts.as_ref()))
                };
                let query = self.search.isolated(self.search.remaining_time());
                let mut out = Vec::new();
                let mut seen = HashSet::new();
                let mut subdirs = Vec::new();
                let mut skipped = Vec::new();
                for d in &dirs {
                    let entries = if direct {
                        query.direct_entries(d)
                    } else {
                        let scan = query.recursive_entries(d);
                        skipped.extend_from_slice(scan.skipped());
                        scan.entries.clone()
                    };
                    for e in entries.iter() {
                        let p = &e.path;
                        if e.is_dir {
                            if direct {
                                subdirs.push(e.clone());
                            }
                            continue;
                        }
                        if e.canon.parent().is_some_and(|parent| top_level.contains(parent)) || !seen.insert(p.clone()) {
                            continue;
                        }
                        match matches_file(p) {
                            Some(true) => out.push(p.clone()),
                            Some(false) => {}
                            None => query.unreadable(p.parent().unwrap_or(d)),
                        }
                    }
                }
                if read_only {
                    self.search.record_issues(&query);
                } else if let Some(message) = query.status() {
                    query.record_failure();
                    return Err(Stop::SearchIncomplete(message));
                }
                let nested = if direct { query.count_nested(&subdirs, matches_file).unwrap_or(0) } else { 0 };
                let skipped_note = self.skipped_trees_note(&skipped);
                if !read_only && !skipped_note.is_empty() {
                    self.warnings.push(skipped_note.clone());
                }
                out.sort_by_key(|p| file_name(p).to_lowercase());
                self.note_dates(filter);
                Ok(ExpandedSource { paths: out, label, folder, nested, skipped_note })
            }
        }
    }

    fn expand_nonempty(&mut self, src: &Source) -> Result<(Vec<PathBuf>, String), Stop> {
        let ExpandedSource { paths, label, skipped_note, .. } = self.expand_source(src, Expansion::RecursiveChanges)?;
        if paths.is_empty() {
            return Err(Self::no_matching_files(&label, 0, &skipped_note));
        }
        Ok((paths, label))
    }

    /// Keep recursive changes out of internal and hidden folders. An
    /// explicitly named dependency folder or virtual environment can be used.
    fn in_hidden_tree(&self, dir: &Path) -> bool {
        let hidden = |p: &Path| fsx::is_hidden_or_internal(p) && !search::is_virtual_env(p);
        [dir, &fsx::canonical(dir)].into_iter().any(|path| {
            self.roots.allowed.iter().any(|root| path.strip_prefix(root).is_ok_and(|rest| hidden(root) || rest.ancestors().any(hidden)))
        })
    }

    /// A configured root can start inside a package, so only inspect the
    /// deepest matching root and the path below it.
    fn in_package(&self, dir: &Path) -> bool {
        [dir, &fsx::canonical(dir)].into_iter().any(|path| {
            self.roots
                .allowed
                .iter()
                .chain(self.roots.named.iter().map(|(_, root)| root))
                .filter_map(|root| path.strip_prefix(root).ok().map(|rest| (root, rest)))
                .max_by_key(|(root, _)| root.as_os_str().len())
                .is_some_and(|(root, rest)| search::is_bundle(root) || rest.ancestors().any(search::is_bundle))
        })
    }

    fn skipped_trees_note(&self, skipped: &[PathBuf]) -> String {
        if skipped.is_empty() {
            return String::new();
        }
        let folders: BTreeSet<_> = skipped.iter().collect();
        let examples = folders.iter().take(3).map(|p| format!("“{}”", self.roots.display(p))).collect::<Vec<_>>().join(", ");
        let more = folders.len().saturating_sub(3);
        let more = if more > 0 { format!(" and {more} more") } else { String::new() };
        format!("{SKIPPED_TREES_NOTE} This includes {examples}{more}.")
    }

    /// The shared warning for matching files a move, copy or delete left in subfolders.
    fn note_nested(&mut self, nested: usize) {
        if nested > 0 {
            self.warnings.push(format!(
                "{nested} more matching {} left alone in subfolders. Add “including subfolders” at the end to include {}.",
                plural(nested, "file was", "files were"),
                plural(nested, "it", "them")
            ));
        }
    }

    /// Explains why a complete search found no inputs for a change.
    fn no_matching_files(label: &str, nested: usize, skipped_note: &str) -> Stop {
        if nested > 0 {
            Stop::NothingToChange(format!(
                "Nothing directly in {label}. Found {nested} matching {} in subfolders: add “including subfolders” at the end to include {}, or list them first and say what to do with them.",
                plural(nested, "file", "files"),
                plural(nested, "it", "them")
            ))
        } else {
            Stop::NothingToChange(format!("No files match in {label}.{}", skipped_trees_suffix(skipped_note)))
        }
    }

    // ---- Planners ----------------------------------------------------------

    /// "Trash", "the bin", "Recycle Bin": a destination that names the Trash
    /// itself rather than a folder path.
    fn names_the_trash(to: &str) -> bool {
        let to = to.trim().to_lowercase();
        let to = to.strip_prefix("the ").unwrap_or(&to).trim();
        !to.contains(['/', '~']) && matches!(to, "trash" | "bin" | "recycle bin")
    }

    fn transfer(&mut self, src: &Source, to: &str, copy: bool) -> Result<String, Stop> {
        let src_dir = match src {
            Source::Set { folder: Some(folder), .. } => Some(self.resolve_folder(folder, None)?),
            _ => None,
        };
        let parent_for_new = src_dir.unwrap_or_else(|| self.roots.desktop());
        let dest = self.resolve_folder(to, Some(&parent_for_new)).and_then(|dest| {
            if matches!(src, Source::Set { folder, filter } if Self::includes_nested_files(folder.as_deref(), filter))
                && (self.in_package(&dest) || self.in_hidden_tree(&dest))
            {
                return problem("Recursive transfers can't put files inside a macOS package or a hidden folder.");
            }
            Ok(dest)
        });
        // A filter after "to Trash" ("move files ... to Trash between 10 and
        // 50 MB") parses as a Move, not the delete phrasing. Never create a
        // folder called Trash: apply the same checks as a delete command.
        // Only a bare "Trash" means the Trash: "Documents/Trash" or "~/bin" is
        // an ordinary folder path, even when that folder doesn't exist yet.
        if let Ok(dest) = &dest
            && Self::names_the_trash(to)
            && !dest.exists()
            && !self.planned_dirs.contains(dest)
        {
            if copy {
                return problem("Copying to the Trash isn't supported. Use “delete” instead.");
            }
            return self.trash(src);
        }
        let ExpandedSource { paths, label, nested, skipped_note, .. } = self.expand_source(src, Expansion::Transfer)?;
        if paths.is_empty() {
            return Err(Self::no_matching_files(&label, nested, &skipped_note));
        }
        let dest = dest?;
        self.note_nested(nested);
        self.ensure_dir(&dest)?;
        // A destination inside the source may already hold previous results.
        let canonical_dest = fsx::canonical(&dest);
        let destination_in_source = match src {
            Source::Set { folder, filter } => self.set_folders(folder.as_deref(), filter, false)?.0.iter().any(|dir| {
                let canonical_dir = fsx::canonical(dir);
                canonical_dest != canonical_dir && canonical_dest.starts_with(canonical_dir)
            }),
            _ => false,
        };
        let mut already = 0;
        for p in &paths {
            self.check(p)?;
            if copy && p.parent().is_some_and(|parent| !self.roots.is_allowed(parent)) {
                return problem("Voxi won't copy an allowed root folder itself. Name a folder inside it.");
            }
            if !copy && self.roots.is_root(p) {
                return problem("Voxi won't move an allowed root folder itself. Name a folder inside it.");
            }
            let canonical_source = fsx::canonical(p);
            if (destination_in_source && canonical_source.starts_with(&canonical_dest))
                || (!copy && p.parent().is_some_and(|d| fsx::same_file(d, &dest)))
            {
                already += 1;
                continue;
            }
            if canonical_dest.starts_with(&canonical_source) {
                return problem(format!("Can't put {} inside itself.", file_name(p)));
            }
            let (to, note) = self.target(&dest, &file_name(p));
            self.check(&to)?;
            let a = if copy { Action::Copy { from: p.clone(), to } } else { Action::Move { from: p.clone(), to } };
            self.push(a, note);
        }
        if already > 0 {
            self.warnings.push(format!("{already} already in {}, left alone.", file_name(&dest)));
        }
        let verb = if copy { "Copy" } else { "Move" };
        Ok(format!("{verb} {} to {}", count_label(&paths), file_name(&dest)))
    }

    fn rename_one(&mut self, src: &Source, new_name: &str) -> Result<String, Stop> {
        let Source::Item { name, folder } = src else {
            return problem("Rename one file at a time, or use a rule like “replace X with Y”.");
        };
        let path = self.resolve_item(name, folder.as_deref())?;
        self.check(&path)?;
        if self.roots.is_root(&path) {
            return problem("Voxi won't rename an allowed root folder itself. Name a folder inside it.");
        }
        if new_name.contains('/') || new_name == "." || new_name == ".." {
            return problem("New names can't contain “/”.");
        }
        let old = file_name(&path);
        // "rename report.pdf to Q3 report" keeps the extension.
        let new_name = match (split_name(&old).1, split_name(new_name).1) {
            (Some(ext), None) if !path.is_dir() => format!("{new_name}.{ext}"),
            _ => new_name.to_string(),
        };
        self.push_rename(&path, &new_name);
        Ok(format!("Rename {old}"))
    }

    fn push_rename(&mut self, path: &Path, new_name: &str) {
        let dir = path.parent().unwrap_or(Path::new("/"));
        let wanted = dir.join(new_name);
        // A case-only rename targets the same file on a case-insensitive disk.
        let (target, note) = if fsx::same_file(path, &wanted) {
            self.reserved.insert(PathBuf::from(wanted.to_string_lossy().to_lowercase()));
            (wanted, None)
        } else {
            self.target(dir, new_name)
        };
        self.push(Action::Rename { path: path.to_path_buf(), new_name: file_name(&target) }, note);
    }

    fn batch_rename(&mut self, src: &Source, verb: &str, f: impl Fn(&str, &Path) -> Option<String>) -> Result<String, Stop> {
        let (paths, label) = self.expand_nonempty(src)?;
        let mut unchanged = 0;
        for p in &paths {
            self.check(p)?;
            if self.roots.is_root(p) {
                return problem("Voxi won't rename an allowed root folder itself. Name a folder inside it.");
            }
            let old = file_name(p);
            match f(&old, p) {
                Some(new) if new != old && !new.is_empty() && !new.contains('/') => self.push_rename(p, &new),
                _ => unchanged += 1,
            }
        }
        if unchanged > 0 {
            self.warnings.push(format!("{unchanged} {} already fine, left alone.", plural(unchanged, "name", "names")));
        }
        Ok(format!("{verb}: {} in {label}", count_label(&paths)))
    }

    fn number(&mut self, src: &Source, base: &str, width: usize) -> Result<String, Stop> {
        let (mut paths, label) = self.expand_nonempty(src)?;
        paths.sort_by_key(|p| (created(p), file_name(p)));
        for (i, p) in paths.iter().enumerate() {
            self.check(p)?;
            if self.roots.is_root(p) {
                return problem("Voxi won't rename an allowed root folder itself. Name a folder inside it.");
            }
            let ext = split_name(&file_name(p)).1.map(|e| format!(".{}", e.to_lowercase())).unwrap_or_default();
            let new = format!("{base}{:0width$}{ext}", i + 1);
            if new != file_name(p) {
                self.push_rename(p, &new);
            }
        }
        Ok(format!("Number {} in {label} as {base}{:0width$}…", count_label(&paths), 1))
    }

    /// Refuses a step whose inputs overlap files used earlier, or whose source
    /// folder is one an earlier step writes into. Every step's file list is
    /// read from disk *before* the plan runs, so a later step can never see
    /// what an earlier step creates or moves into a shared folder.
    fn check_dependencies(&self, start: usize, index: usize, deps: &mut Dependencies) -> Result<(), Stop> {
        let depends = || format!("Command {index} depends on files used earlier. Run those commands separately.");
        let mut files = Vec::new();
        let mut dirs = Vec::new();
        let mut writes = Vec::new();
        for action in &self.actions[start..] {
            let (inputs, outputs): (Vec<PathBuf>, Vec<PathBuf>) = match action {
                Action::CreateFolder { path } => {
                    if deps.files.iter().any(|p| overlaps(path, p)) {
                        return problem(depends());
                    }
                    dirs.push(path.clone());
                    writes.push((path.clone(), true));
                    continue;
                }
                Action::Move { from, to } | Action::Copy { from, to } => (vec![from.clone()], vec![to.clone()]),
                Action::Rename { path, new_name } => {
                    (vec![path.clone()], path.parent().map(|p| vec![p.join(new_name)]).unwrap_or_default())
                }
                Action::Trash { path } => (vec![path.clone()], Vec::new()),
                Action::ConvertImage { path, output, .. } => (vec![path.clone()], vec![output.clone()]),
                // The original is copied to `backup`, then `path` is resized in place.
                Action::ResizeImage { path, backup, .. } => (vec![path.clone()], vec![backup.clone()]),
                Action::ImagesToPdf { paths, output } | Action::MergePdf { paths, output } => (paths.clone(), vec![output.clone()]),
                Action::SplitPdf { path, output_dir } => (vec![path.clone()], vec![output_dir.clone()]),
                Action::ExtractPages { path, output, .. } => (vec![path.clone()], vec![output.clone()]),
                Action::Zip { path, output } => (vec![path.clone()], vec![output.clone()]),
                Action::Unzip { path, destination } => (vec![path.clone()], vec![destination.clone()]),
                // Read-only, undo-only and other actions don't touch files here.
                _ => continue,
            };
            // A later step reads its file list from the folder it draws from.
            // If an earlier step writes into that folder, this list is stale.
            // The one exception is "create X then move into X": there the
            // created folder is the later move's destination, not a source.
            for input in &inputs {
                if let Some(source_dir) = input.parent() {
                    let touched: Vec<_> = deps.writes.iter().filter(|(dir, _)| overlaps(source_dir, dir)).collect();
                    if !touched.is_empty() {
                        // A real move/copy destination inside this folder changes
                        // the direct file list a later step would read, so that
                        // preview is stale. Keep refusing those writes.
                        let stale_from_write = touched.iter().any(|(_, created)| !*created);
                        // A newly created empty folder can't change a direct-only
                        // listing (moves, copies and deletes all read direct
                        // children now). It only matters if this step reads the
                        // created folder itself.
                        let stale_from_create = touched.iter().any(|(dir, created)| *created && same_path(source_dir, dir));
                        if stale_from_write || stale_from_create {
                            return problem(depends());
                        }
                    }
                }
                if deps.files.iter().any(|p| overlaps(input, p) || outputs.iter().any(|out| overlaps(out, p)))
                    || deps.dirs.iter().any(|p| overlaps(input, p))
                {
                    return problem(depends());
                }
            }
            for out in &outputs {
                if let Some(parent) = out.parent() {
                    writes.push((parent.to_path_buf(), false));
                }
            }
            files.extend(inputs);
            files.extend(outputs);
        }
        deps.files.extend(files);
        deps.dirs.extend(dirs);
        deps.writes.extend(writes);
        Ok(())
    }

    /// Shares reservations and binds explicit output dependencies to concrete
    /// paths. Unbound disk searches still use the conservative stale-list guard.
    fn chain(&mut self, commands: &[Intent], history: &History) -> Result<String, Stop> {
        if self.scope.is_some() {
            return problem("Clear the previous result list before chaining commands.");
        }
        if !(2..=8).contains(&commands.len()) {
            return problem("Chain between 2 and 8 commands.");
        }
        let mut deps = Dependencies::default();
        let mut previous = None;
        let mut created_targets = HashSet::new();
        for (index, intent) in commands.iter().enumerate() {
            if !matches!(
                intent,
                Intent::CreateFolders { .. }
                    | Intent::Move { .. }
                    | Intent::Copy { .. }
                    | Intent::Rename { .. }
                    | Intent::Trash { .. }
                    | Intent::Zip { .. }
                    | Intent::Unzip { .. }
                    | Intent::ConvertImage { .. }
                    | Intent::ResizeImage { .. }
            ) {
                return problem("This command cannot be included in a file-changing chain.");
            }
            let mut bound = intent.clone();
            let explicit_it = matches!(intent.source(), Some(Source::Item { name, folder: None }) if name == "\0chain-output");
            // A named convert output is also an explicit dependency. Bind to
            // the actual reserved output, including its collision suffix.
            let converted = match (intent, self.actions.last()) {
                (
                    Intent::ResizeImage { src: Source::Item { name, folder: None }, .. },
                    Some(Action::ConvertImage { path, output, format }),
                ) => {
                    let wanted = path.with_extension(format.ext());
                    let requested = if Path::new(name).is_absolute() { PathBuf::from(name) } else { self.roots.home.join(name) };
                    same_path(&wanted, &requested).then(|| output.clone())
                }
                _ => None,
            };
            self.chain_input = if explicit_it { previous.clone() } else { converted };
            if explicit_it && self.chain_input.is_none() {
                return problem(format!(
                    "Command {} uses ‘it’, but the previous command has no single output. Name one file or folder, or run the commands separately.",
                    index + 1
                ));
            }
            let dependent = self.chain_input.is_some();
            if dependent {
                bound.replace_source(Source::item("\0chain-output"));
            }
            // A step that reads a folder this chain creates has nothing to do
            // while that folder is new: "create a folder named X and delete
            // its contents every day" creates X now and empties it on later runs.
            if let Some(Source::Set { folder: Some(folder), .. }) = bound.source()
                && let Ok(dir) = self.resolve_folder(folder, Some(&self.roots.desktop()))
                && !dir.exists()
                && created_targets.iter().any(|target: &PathBuf| same_path(target, &dir))
            {
                self.warnings.push(format!("“{}” is new, so command {} has nothing to do yet.", file_name(&dir), index + 1));
                #[cfg(test)]
                if let Some(ends) = &mut self.chain_action_ends {
                    ends.push(self.actions.len());
                }
                previous = None;
                continue;
            }
            let start = self.actions.len();
            if let Intent::CreateFolders { names, parent } = &bound {
                let (_, parent) = self.create_folders(names, parent.as_deref())?;
                created_targets.extend(names.iter().map(|name| parent.join(name)));
            } else {
                match self.step(&bound, history) {
                    // An idle step makes the whole chain idle if nothing has
                    // been drafted. Otherwise it aborts planning and discards
                    // the draft.
                    Err(stop @ Stop::NothingToChange(_)) if self.actions.is_empty() => return Err(stop),
                    Err(Stop::NothingToChange(message)) => return problem(message),
                    Err(stop) => return Err(stop),
                    Ok(Step::Finished(_)) => return problem("Every step in a chain must change files."),
                    Ok(Step::Changes(_)) => {}
                }
            }
            self.chain_input = None;
            if dependent {
                // This input is a concrete earlier output, never a fresh
                // directory search. Execution and inverse actions are ordered.
                let mut next = Dependencies::default();
                self.check_dependencies(start, index + 1, &mut next)?;
                deps.files.extend(next.files);
                deps.dirs.extend(next.dirs);
                deps.writes.extend(next.writes);
            } else {
                self.check_dependencies(start, index + 1, &mut deps)?;
            }
            let added = &self.actions[start..];
            #[cfg(test)]
            if let Some(ends) = &mut self.chain_action_ends {
                ends.push(start + added.len());
            }
            previous = match added.last() {
                Some(Action::CreateFolder { path }) => Some(path.clone()),
                Some(Action::Rename { path, new_name }) => Some(path.with_file_name(new_name)),
                Some(Action::Move { to, .. } | Action::Copy { to, .. }) => {
                    if to.parent().is_some_and(|p| created_targets.contains(p)) {
                        to.parent().map(Path::to_path_buf)
                    } else if added.iter().filter(|a| matches!(a, Action::Move { .. } | Action::Copy { .. })).count() == 1 {
                        Some(to.clone())
                    } else {
                        None
                    }
                }
                Some(Action::ConvertImage { output, .. } | Action::Zip { output, .. }) => Some(output.clone()),
                Some(Action::Unzip { destination, .. }) => Some(destination.clone()),
                Some(Action::ResizeImage { path, .. }) => Some(path.clone()),
                _ => None,
            };
        }
        Ok(format!("Run {} commands", commands.len()))
    }

    fn create_folders(&mut self, names: &[String], parent: Option<&str>) -> Result<(String, PathBuf), Stop> {
        let parent = match parent {
            Some(p) => self.resolve_folder(p, None)?,
            None => self.roots.desktop(),
        };
        for n in names {
            if n.contains('/') || n.starts_with('.') {
                return problem(format!("“{n}” isn't a valid folder name."));
            }
            let p = parent.join(n);
            // An existing folder still has to be one Voxi may use, so an
            // excluded folder is a real failure, not "nothing to change".
            self.check(&p)?;
            if p.exists() {
                if !p.is_dir() {
                    return problem(format!("A file named “{n}” already exists in {}. Choose another folder name.", file_name(&parent)));
                }
                self.warnings.push(format!("{n} already exists in {}.", file_name(&parent)));
            } else {
                self.ensure_dir(&p)?;
            }
        }
        let what = if names.len() == 1 { format!("folder {}", names[0]) } else { format!("{} folders", names.len()) };
        Ok((format!("Create {what} in {}", file_name(&parent)), parent))
    }

    fn resolve_sort_folder(&self, folder: &str, parent: Option<&str>) -> Result<PathBuf, Stop> {
        let Some(parent) = parent else { return self.resolve_folder(folder, None) };
        let root = self.resolve_folder(parent, None)?;
        let label = format!("{folder} in {parent}");
        if let Some(path) = self.choices.get(&label) {
            self.check(path)?;
            let within_parent =
                fsx::canonical(path).strip_prefix(fsx::canonical(&root)).is_ok_and(|relative| !relative.as_os_str().is_empty());
            if !path.is_dir() || !within_parent {
                return problem(format!("Choose a folder inside {}.", self.roots.display(&root)));
            }
            return Ok(path.clone());
        }
        let path = self.lookup_folder(folder, &label, &[root], None, false)?;
        self.check(&path)?;
        Ok(path)
    }

    fn sort(&mut self, folder: &str, parent: Option<&str>, by_type: bool, include_subfolders: bool) -> Result<String, Stop> {
        let dir = self.resolve_sort_folder(folder, parent)?;
        let filter = Filter { file_type: Some(FileType::Files), include_subfolders, ..Default::default() };
        let src = Source::Set { folder: Some(dir.to_string_lossy().into()), filter };
        let ExpandedSource { paths, nested, .. } = self.expand_source(&src, Expansion::DirectChanges)?;
        if nested > 0 {
            self.warnings.push(format!(
                "{nested} {} in subfolders left alone. Say “including subfolders” to sort them too.",
                plural(nested, "file was", "files were")
            ));
        }
        let sorted_name = file_name(&dir);
        let mut left = 0;
        let mut moved = 0;
        for p in &paths {
            self.check(p)?;
            let sub = if by_type {
                match type_folder(p, &sorted_name) {
                    Some(s) => s,
                    None => {
                        left += 1;
                        continue;
                    }
                }
            } else {
                match created(p) {
                    Some(t) => format!("{}-{:02}", t.year(), t.month()),
                    None => {
                        left += 1;
                        continue;
                    }
                }
            };
            let dest = dir.join(&sub);
            // A category folder must never lead (say, through a symlink) into
            // Voxi's trash, a hidden folder or a macOS package.
            if self.in_hidden_tree(&dest) || self.in_package(&dest) {
                return problem(format!(
                    "“{sub}” in {} leads into a hidden folder or a macOS package, so Voxi won't sort into it.",
                    file_name(&dir)
                ));
            }
            if p.parent().is_some_and(|parent| fsx::same_file(parent, &dest)) {
                continue;
            }
            self.ensure_dir(&dest)?;
            let (to, note) = self.target(&dest, &file_name(p));
            self.push(Action::Move { from: p.clone(), to }, note);
            moved += 1;
        }
        if left > 0 {
            self.warnings.push(format!("{left} {} with no matching group stay where they are.", plural(left, "file", "files")));
        }
        let how = if by_type { "type" } else { "month" };
        Ok(format!("Sort {moved} {} in {} by {how}", plural(moved, "file", "files"), file_name(&dir)))
    }

    fn convert(&mut self, src: &Source, format: ImageFormat) -> Result<String, Stop> {
        if format == ImageFormat::Webp {
            return problem("macOS can read WebP but can't write it. Try png or jpg.");
        }
        let (paths, label) = self.expand_nonempty(src)?;
        let mut skipped = 0;
        for p in &paths {
            self.check(p)?;
            let name = file_name(p);
            let (stem, ext) = split_name(&name);
            let ext = ext.unwrap_or("").to_lowercase();
            if !is_ext(&ext, IMAGE_EXTS) || ext == format.ext() || (ext == "jpeg" && format == ImageFormat::Jpg) {
                skipped += 1;
                continue;
            }
            let (out, note) = self.target(p.parent().unwrap(), &format!("{stem}.{}", format.ext()));
            self.push(Action::ConvertImage { path: p.clone(), format, output: out }, note);
        }
        if skipped > 0 {
            self.warnings.push(format!(
                "{skipped} {} skipped (not an image, or already {}).",
                plural(skipped, "file", "files"),
                format.ext()
            ));
        }
        Ok(format!("Convert {} in {label} to {}", count_label(&paths), format.ext().to_uppercase()))
    }

    fn resize(&mut self, src: &Source, width: u32) -> Result<String, Stop> {
        let (paths, label) = self.expand_nonempty(src)?;
        let stamp = self.now.format("%Y%m%d-%H%M%S").to_string();
        let mut small = 0;
        for p in &paths {
            self.check(p)?;
            let ext = split_name(&file_name(p)).1.unwrap_or("").to_lowercase();
            if !is_ext(&ext, IMAGE_EXTS) || ext == "svg" {
                continue;
            }
            if image_width(p).is_some_and(|w| w <= width) {
                small += 1;
                continue;
            }
            let backup_dir = self.roots.trash_dir_for(p).join(".voxi-originals").join(&stamp);
            self.check(&backup_dir)?;
            let backup = unique_path(&backup_dir, &file_name(p), &mut self.reserved);
            self.check(&backup)?;
            self.push(
                Action::ResizeImage { path: p.clone(), max_width_px: width, backup },
                Some("Original kept in _to_delete/.voxi-originals".into()),
            );
        }
        if small > 0 {
            self.warnings
                .push(format!("{small} {} already {width} px wide or smaller, left alone.", plural(small, "image is", "images are")));
        }
        Ok(format!("Resize {} in {label} to {width} px wide", count_label(&paths)))
    }

    /// Where a new output file goes: a bare name lands next to the inputs.
    fn output_path(&mut self, name: &str, next_to: &Path, ext: &str) -> Result<(PathBuf, Option<String>), Stop> {
        let name = if split_name(name).1.is_some_and(|e| e.eq_ignore_ascii_case(ext)) { name.to_string() } else { format!("{name}.{ext}") };
        let (dir, file) = match name.rsplit_once('/') {
            Some((d, f)) => (self.resolve_folder(d, Some(next_to))?, f.to_string()),
            None => (next_to.to_path_buf(), name),
        };
        self.check(&dir.join(&file))?;
        self.ensure_dir(&dir)?;
        Ok(self.target(&dir, &file))
    }

    fn merge(&mut self, src: &Source, output: &str) -> Result<String, Stop> {
        let (paths, label) = self.expand_nonempty(src)?;
        let pdfs: Vec<PathBuf> = paths.into_iter().filter(|p| ext_of(p) == "pdf").collect();
        if pdfs.len() < 2 {
            return Err(Stop::NothingToChange(format!("Found {} PDF in {label}; merging needs at least two.", pdfs.len())));
        }
        pdfs.iter().try_for_each(|p| self.check(p))?;
        let (out, note) = self.output_path(output, pdfs[0].parent().unwrap(), "pdf")?;
        let n = pdfs.len();
        self.push(Action::MergePdf { paths: pdfs, output: out.clone() }, note);
        Ok(format!("Merge {n} PDFs into {}", file_name(&out)))
    }

    fn single(&mut self, src: &Source, ext: &str, what: &str) -> Result<PathBuf, Stop> {
        let (paths, _) = self.expand_nonempty(src)?;
        let [p] = paths.as_slice() else { return problem(format!("Name one {what}.")) };
        if ext_of(p) != ext {
            return problem(format!("{} isn't a {what}.", file_name(p)));
        }
        self.check(p)?;
        Ok(p.clone())
    }

    fn split(&mut self, src: &Source) -> Result<String, Stop> {
        let p = self.single(src, "pdf", "PDF")?;
        let pages = pdf::page_count(&p).map_err(Stop::Problem)?;
        if pages < 2 {
            return problem(format!("{} has only one page.", file_name(&p)));
        }
        let stem = p.file_stem().unwrap().to_string_lossy().to_string();
        let (dir, note) = self.target(p.parent().unwrap(), &format!("{stem} pages"));
        self.push(Action::SplitPdf { path: p.clone(), output_dir: dir }, note);
        Ok(format!("Split {} into {pages} PDFs", file_name(&p)))
    }

    fn extract(&mut self, src: &Source, from: u32, to: u32, output: Option<&str>) -> Result<String, Stop> {
        let p = self.single(src, "pdf", "PDF")?;
        let pages = pdf::page_count(&p).map_err(Stop::Problem)? as u32;
        if from == 0 || from > to || to > pages {
            return problem(format!("{} has {pages} pages; pick a range from 1 to {pages}.", file_name(&p)));
        }
        let stem = p.file_stem().unwrap().to_string_lossy().to_string();
        let default = if from == to { format!("{stem} p{from}.pdf") } else { format!("{stem} p{from}-{to}.pdf") };
        let (out, note) = self.output_path(output.unwrap_or(&default), p.parent().unwrap(), "pdf")?;
        self.push(Action::ExtractPages { path: p.clone(), from, to, output: out }, note);
        let range = if from == to { format!("page {from}") } else { format!("pages {from}–{to}") };
        Ok(format!("Extract {range} of {}", file_name(&p)))
    }

    fn images_to_pdf(&mut self, src: &Source, output: Option<&str>) -> Result<String, Stop> {
        let (paths, label) = self.expand_nonempty(src)?;
        let images: Vec<PathBuf> = paths.into_iter().filter(|p| is_ext(&ext_of(p), IMAGE_EXTS)).collect();
        if images.is_empty() {
            return Err(Stop::NothingToChange(format!("No images match in {label}.")));
        }
        images.iter().try_for_each(|p| self.check(p))?;
        let default = match src {
            Source::Set { filter: Filter { name_contains: Some(n), .. }, .. } => n.clone(),
            _ => format!("{label} images"),
        };
        let (out, note) = self.output_path(output.unwrap_or(&default), images[0].parent().unwrap(), "pdf")?;
        let n = images.len();
        self.push(Action::ImagesToPdf { paths: images, output: out.clone() }, note);
        Ok(format!("Make {} from {n} {}", file_name(&out), plural(n, "image", "images")))
    }

    fn zip(&mut self, src: &Source, output: Option<&str>) -> Result<String, Stop> {
        let (paths, _) = self.expand_nonempty(src)?;
        let [p] = paths.as_slice() else {
            return problem("Name one file or folder to zip. To archive several files, put them in one folder and zip that folder.");
        };
        self.check(p)?;
        if self.roots.is_root(p) {
            return problem(format!("Zipping all of {} isn't supported; name a folder inside it.", file_name(p)));
        }
        let default = format!("{}.zip", file_name(p));
        let (out, note) = self.output_path(output.unwrap_or(&default), p.parent().unwrap(), "zip")?;
        self.push(Action::Zip { path: p.clone(), output: out.clone() }, note);
        Ok(format!("Zip {} into {}", file_name(p), file_name(&out)))
    }

    fn unzip(&mut self, src: &Source, destination: Option<&str>) -> Result<String, Stop> {
        let p = self.single(src, "zip", "zip file")?;
        let parent = p.parent().unwrap().to_path_buf();
        let stem = p.file_stem().unwrap();
        let wanted = match destination {
            Some(d) => {
                let resolved = self.resolve_folder(d, Some(&parent))?;
                if self.roots.is_root(&resolved) { resolved.join(stem) } else { resolved }
            }
            None => parent.join(stem),
        };
        let wanted_parent = wanted.parent().unwrap().to_path_buf();
        self.ensure_dir(&wanted_parent)?;
        // Never extract over existing files: use a fresh folder.
        let (dest, note) = self.target(&wanted_parent, &file_name(&wanted));
        self.check(&dest)?;
        self.push(Action::Unzip { path: p.clone(), destination: dest.clone() }, note);
        Ok(format!("Unzip {} into {}", file_name(&p), self.roots.display(&dest)))
    }

    fn trash(&mut self, src: &Source) -> Result<String, Stop> {
        if let Source::Set { folder, filter } = src
            && !filter.is_narrowing()
            && self.scope.is_none()
        {
            // A folder the user named inside Downloads, Desktop and the rest
            // may be emptied; those top-level folders themselves may not.
            let named_subfolder = match folder.as_deref() {
                Some(f) => !self.roots.is_root(&self.resolve_folder(f, None)?),
                None => false,
            };
            if !named_subfolder {
                return problem("To be safe, clean-up needs a filter, like “older than 90 days”, “installers”, or “larger than 500 MB”.");
            }
        }
        let ExpandedSource { paths, label, nested, skipped_note, .. } = self.expand_source(src, Expansion::DirectChanges)?;
        if paths.is_empty() {
            return Err(Self::no_matching_files(&label, nested, &skipped_note));
        }
        self.note_nested(nested);
        for p in &paths {
            self.check(p)?;
            if self.roots.is_root(p) {
                return problem(format!("Voxi won't remove {} itself.", file_name(p)));
            }
            self.push(Action::Trash { path: p.clone() }, None);
        }
        // A named item is its own label, so "from <file>" would only repeat it.
        if matches!(src, Source::Item { .. }) {
            return Ok(format!("Move {} to the Trash", count_label(&paths)));
        }
        Ok(format!("Move {} from {label} to the Trash", count_label(&paths)))
    }

    // ---- Read-only ---------------------------------------------------------

    /// `modified` shows the last-modified date, for a list filtered by it.
    fn file_row(&self, p: &Path, modified: bool) -> InfoRow {
        let meta = fs::metadata(p).map_err(|e| self.search.issue(format!("couldn't inspect {} ({e})", p.display()))).ok();
        let size = if p.is_dir() { fsx::tree_size(p).0 } else { meta.as_ref().map_or(0, |m| m.len()) };
        let date = row_date(p, modified).map(|d| d.format("%b %-d, %Y").to_string()).unwrap_or_default();
        let date = if modified && !date.is_empty() { format!("modified {date}") } else { date };
        let dir = p.parent().map(|d| self.roots.display(d)).unwrap_or_default();
        InfoRow { name: file_name(p), detail: format!("{} · {date} · {dir}", human_size(size)) }
    }

    fn list(&mut self, src: &Source) -> Result<Planned, Stop> {
        let ExpandedSource { mut paths, label, folder, skipped_note, .. } = self.expand_source(src, Expansion::ReadOnly)?;
        let by_size = matches!(src, Source::Set { filter, .. } if filter.larger_than_mb.is_some() || filter.smaller_than_mb.is_some());
        let modified = reads_modified(src);
        if by_size {
            paths.sort_by_key(|p| std::cmp::Reverse(fs::metadata(p).map_or(0, |m| m.len())));
        } else {
            paths.sort_by_key(|p| std::cmp::Reverse(row_date(p, modified)));
        }
        let total_bytes: u64 = paths.iter().map(|p| fs::metadata(p).map_or(0, |m| m.len())).sum();
        let rows = paths.iter().take(200).map(|p| self.file_row(p, modified)).collect();
        Ok(Planned::Info {
            listed: Some(Listed { paths: paths.clone(), label: label.clone(), folder }),
            // A single named file's label is its own name; title it by its folder instead.
            title: match paths.as_slice() {
                [one] if file_name(one) == label => format!("{label} in {}", one.parent().map_or_else(|| label.clone(), file_name)),
                _ => format!("{} in {label}", count_label(&paths)),
            },
            summary: format!(
                "{}{}",
                Self::search_scope_hint(src),
                if paths.is_empty() {
                    "Nothing matches.".into()
                } else {
                    format!("{} in total, including subfolders.", human_size(total_bytes))
                }
            ) + &summary_date_note(src)
                + &skipped_trees_suffix(&skipped_note),
            rows,
            total: paths.len(),
        })
    }

    fn count(&mut self, src: &Source) -> Result<Planned, Stop> {
        let ExpandedSource { paths, label, skipped_note, .. } = self.expand_source(src, Expansion::ReadOnly)?;
        let what = match src {
            Source::Set { filter, .. } => type_label(filter),
            _ => "files",
        };
        Ok(Planned::Info {
            listed: None,
            title: format!("{} {what} in {label}", paths.len()),
            summary: format!(
                "{}Including subfolders.{}{}",
                Self::search_scope_hint(src),
                summary_date_note(src),
                skipped_trees_suffix(&skipped_note)
            ),
            rows: paths.iter().take(20).map(|p| self.file_row(p, reads_modified(src))).collect(),
            total: paths.len(),
        })
    }

    fn size(&self, folder: &str) -> Result<Planned, Stop> {
        let p = self.resolve_any(folder)?;
        let mut bytes = 0;
        let mut files = 0;
        let mut skipped_note = String::new();
        let mut sizes: HashMap<PathBuf, u64> = HashMap::new();
        if p.is_dir() {
            let root = fsx::canonical(&p);
            let scan = self.search.recursive_entries(&p);
            skipped_note = self.skipped_trees_note(scan.skipped());
            for entry in scan.entries.iter() {
                let Some(child) = entry.canon.strip_prefix(&root).ok().and_then(|relative| relative.components().next()) else {
                    self.search.issue(format!("couldn't group {}", entry.path.display()));
                    continue;
                };
                let size = sizes.entry(root.join(child.as_os_str())).or_default();
                if !entry.is_dir {
                    match fs::symlink_metadata(&entry.path) {
                        Ok(meta) => {
                            bytes += meta.len();
                            files += 1;
                            *size += meta.len();
                        }
                        Err(e) => self.search.issue(format!("couldn't inspect {} ({e})", entry.path.display())),
                    }
                }
            }
        } else {
            match fs::symlink_metadata(&p) {
                Ok(meta) => {
                    bytes = meta.len();
                    files = 1;
                }
                Err(e) => self.search.issue(format!("couldn't inspect {} ({e})", p.display())),
            }
        }
        let mut children: Vec<(u64, PathBuf)> = sizes.into_iter().map(|(p, size)| (size, p)).collect();
        children.sort_by_key(|(b, _)| std::cmp::Reverse(*b));
        let rows = children
            .iter()
            .take(15)
            .map(|(b, c)| InfoRow {
                name: file_name(c),
                detail: format!("{}{}", human_size(*b), if c.is_dir() { " · folder" } else { "" }),
            })
            .collect();
        let (lower_bound, file_count_prefix) =
            if !skipped_note.is_empty() || matches!(self.search.completeness(), search::Completeness::Partial(_)) {
                ("at least ", "At least ")
            } else {
                ("", "")
            };
        Ok(Planned::Info {
            listed: None,
            title: format!("{} uses {lower_bound}{}", file_name(&p), human_size(bytes)),
            summary: format!(
                "{file_count_prefix}{files} {} in total.{} Largest items:",
                plural(files, "file", "files"),
                skipped_trees_suffix(&skipped_note)
            ),
            rows,
            total: children.len(),
        })
    }

    fn duplicates(&self, folder: &str) -> Result<Planned, Stop> {
        let dir = self.resolve_folder_with_lookup(folder, None, true)?;
        let mut groups: BTreeMap<(String, u64), Vec<PathBuf>> = BTreeMap::new();
        let scan = self.search.recursive_entries(&dir);
        for entry in scan.entries.iter() {
            let p = &entry.path;
            if !entry.is_dir {
                match fs::metadata(p) {
                    Ok(m) => groups.entry((base_name(&file_name(p)), m.len())).or_default().push(p.to_path_buf()),
                    Err(e) => self.search.issue(format!("couldn't inspect {} ({e})", p.display())),
                }
            }
        }
        let mut dupes: Vec<_> = groups.into_iter().filter(|(_, v)| v.len() > 1).collect();
        dupes.sort_by_key(|((_, size), v)| std::cmp::Reverse(size * (v.len() as u64 - 1)));
        let wasted: u64 = dupes.iter().map(|((_, s), v)| s * (v.len() as u64 - 1)).sum();
        let rows = dupes
            .iter()
            .take(100)
            .map(|((_, size), v)| InfoRow {
                name: v.iter().map(|p| file_name(p)).collect::<Vec<_>>().join(", "),
                detail: format!("{} copies · {} each · {}", v.len(), human_size(*size), self.roots.display(v[0].parent().unwrap())),
            })
            .collect();
        Ok(Planned::Info {
            listed: None,
            title: format!("{} {} in {}", dupes.len(), plural(dupes.len(), "set of duplicates", "sets of duplicates"), file_name(&dir)),
            summary: if dupes.is_empty() {
                "No duplicates by name and size.".into()
            } else {
                format!("Matched by name and size. Extra copies use {}.", human_size(wasted))
            } + &skipped_trees_suffix(&self.skipped_trees_note(scan.skipped())),
            rows,
            total: dupes.len(),
        })
    }
}

fn skipped_trees_suffix(note: &str) -> String {
    if note.is_empty() { String::new() } else { format!(" {note}") }
}

pub(crate) fn plan_undo(entry: &Entry, roots: &Roots) -> Result<Planned, Stop> {
    crate::history::validate_inverses(entry.inverses()).map_err(Stop::Problem)?;
    for inverse in entry.inverses() {
        roots.check_whole_tree_exclusions(&inverse.action).map_err(Stop::Problem)?;
    }
    let actions: Vec<_> = entry.inverses().iter().map(|inverse| inverse.action.clone()).collect();
    Ok(Planned::Steps {
        title: format!("Undo “{}”", entry.command),
        notes: vec![None; actions.len()],
        actions,
        warnings: vec![],
        undo_of: Some(entry.id),
    })
}

fn recent(history: &History, roots: &Roots) -> Planned {
    let rows: Vec<InfoRow> = history
        .entries
        .iter()
        .rev()
        .take(20)
        .map(|e| {
            let status = match &e.state {
                EntryState::Exhausted => " · undone",
                EntryState::PartiallyUndone(_) => " · partially undone",
                _ if e.undo_of.is_some() => " · undo",
                _ => "",
            };
            let first = e.done.first().map(|a| describe_path(a, roots)).unwrap_or_default();
            InfoRow {
                name: e.command.clone(),
                detail: format!(
                    "{} · {} {}{status}{first}",
                    e.time.format("%b %-d, %H:%M"),
                    e.done.len(),
                    plural(e.done.len(), "change", "changes")
                ),
            }
        })
        .collect();
    Planned::Info {
        listed: None,
        title: "Recent actions".into(),
        summary: if rows.is_empty() {
            "Nothing yet.".into()
        } else {
            "Newest first. “undo last” reverses the newest one that isn't undone.".into()
        },
        total: history.entries.len(),
        rows,
    }
}

fn describe_path(a: &Action, roots: &Roots) -> String {
    match a {
        Action::Move { from, .. } | Action::Copy { from, .. } => format!(" · {}", roots.display(from)),
        _ => String::new(),
    }
}

// ---- Filters and file facts -------------------------------------------------

fn is_ext(ext: &str, set: &[&str]) -> bool {
    set.contains(&ext)
}

fn ext_of(p: &Path) -> String {
    p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
}

fn is_screenshot(p: &Path) -> bool {
    let n = file_name(p).to_lowercase();
    is_ext(&ext_of(p), IMAGE_EXTS) && (n.starts_with("screenshot") || n.starts_with("screen shot") || n.starts_with("cleanshot"))
}

/// The category folder a file belongs in. If that name would match the folder
/// being sorted (case-insensitively) — sorting `~/Documents` would otherwise
/// create `~/Documents/Documents` — use a short, distinct name instead.
fn type_folder(p: &Path, sorted_name: &str) -> Option<String> {
    let e = ext_of(p);
    Some(match e.as_str() {
        "pdf" => category("PDFs", "PDF files", sorted_name),
        _ if is_ext(&e, IMAGE_EXTS) => category("Images", "Image files", sorted_name),
        _ if is_ext(&e, VIDEO_EXTS) => category("Videos", "Video files", sorted_name),
        _ if is_ext(&e, DOC_EXTS) => category("Documents", "Document files", sorted_name),
        _ if is_ext(&e, ARCHIVE_EXTS) => category("Archives", "Archive files", sorted_name),
        _ if is_ext(&e, INSTALLER_EXTS) => category("Installers", "Installer files", sorted_name),
        _ if is_ext(&e, AUDIO_EXTS) => category("Audio", "Audio files", sorted_name),
        _ => return None,
    })
}

fn category(name: &'static str, alternate: &'static str, sorted_name: &str) -> String {
    if sorted_name.eq_ignore_ascii_case(name) { alternate.to_string() } else { name.to_string() }
}

fn type_label(f: &Filter) -> &'static str {
    match f.file_type {
        Some(FileType::Screenshots) => "screenshots",
        Some(FileType::Images) => "images",
        Some(FileType::Pdfs) => "PDFs",
        Some(FileType::Videos) => "videos",
        Some(FileType::Documents) => "documents",
        Some(FileType::Archives) => "archives",
        Some(FileType::Installers) => "installers",
        Some(FileType::Audio) => "audio files",
        _ => "files",
    }
}

/// When the file was created (falls back to modified), in local time.
pub fn created(p: &Path) -> Option<DateTime<Local>> {
    let m = fs::metadata(p).ok()?;
    let t: SystemTime = m.created().or_else(|_| m.modified()).ok()?;
    Some(t.into())
}

/// The date `f`'s date filters read for `p`: the last change when the
/// command said "modified", otherwise the creation date.
fn filter_date(p: &Path, f: &Filter) -> Option<DateTime<Local>> {
    match f.date_field {
        Some(DateField::Modified) => Some(fs::metadata(p).ok()?.modified().ok()?.into()),
        Some(DateField::Created) | None => created(p),
    }
}

/// True when `src` is a set filtered by its last-modified date.
fn reads_modified(src: &Source) -> bool {
    matches!(src, Source::Set { filter, .. } if filter.date_field == Some(DateField::Modified) && has_date_filter(filter))
}

/// A list row's date: the last change for a list filtered by it, otherwise
/// the creation date.
fn row_date(p: &Path, modified: bool) -> Option<DateTime<Local>> {
    if modified { Some(fs::metadata(p).ok()?.modified().ok()?.into()) } else { created(p) }
}

/// The date note for a list or count summary, with a leading space.
fn summary_date_note(src: &Source) -> String {
    match src {
        Source::Set { filter, .. } => date_note(filter).map(|n| format!(" {n}")).unwrap_or_default(),
        _ => String::new(),
    }
}

/// Says in the preview which date a date filter read.
fn date_note(f: &Filter) -> Option<&'static str> {
    has_date_filter(f).then_some(match f.date_field {
        Some(DateField::Modified) => "Dates are when each file was last modified.",
        Some(DateField::Created) | None => "Dates are when each file was created (or last modified, where that's unknown).",
    })
}

enum NameNeedle<'a> {
    Literal(&'a str),
    Normalized(String),
}

impl<'a> NameNeedle<'a> {
    fn new(needle: &'a str) -> Result<Self, Stop> {
        if !needle.trim().is_empty() {
            // Punctuation alone is meaningful exactly as entered. For
            // example, "-." must not become "." and match every extension.
            if !needle.chars().any(char::is_alphanumeric) {
                return Ok(Self::Literal(needle));
            }
            let normalized = normalized_name(needle);
            if !normalized.is_empty() {
                return Ok(Self::Normalized(normalized));
            }
        }
        problem("Enter a filename filter with a letter, number, or punctuation mark.")
    }

    fn matches(&self, filename: &str, normalized: &str, check: impl FnOnce(&str, &str) -> bool) -> bool {
        match self {
            Self::Literal(needle) => check(filename, needle),
            Self::Normalized(needle) => check(normalized, needle),
        }
    }
}

fn matches_prepared(
    p: &Path,
    f: &Filter,
    now: DateTime<Local>,
    contains: Option<&NameNeedle<'_>>,
    starts: Option<&NameNeedle<'_>>,
) -> bool {
    let e = ext_of(p);
    let type_ok = match f.file_type {
        None => true,
        Some(FileType::Screenshots) => is_screenshot(p),
        Some(t) => t.accepts_ext(&e),
    };
    if !type_ok {
        return false;
    }
    if f.ext.as_ref().is_some_and(|exts| !exts.contains(&e)) {
        return false;
    }
    if contains.is_some() || starts.is_some() {
        let filename = file_name(p);
        let normalized = normalized_name(&filename);
        if contains.is_some_and(|needle| !needle.matches(&filename, &normalized, |name, n| name.contains(n)))
            || starts.is_some_and(|needle| !needle.matches(&filename, &normalized, |name, n| name.starts_with(n)))
        {
            return false;
        }
    }
    if f.larger_than_mb.is_some() || f.smaller_than_mb.is_some() {
        let mb = fs::metadata(p).map_or(0.0, |m| m.len() as f64 / (1024.0 * 1024.0));
        if f.larger_than_mb.is_some_and(|l| mb <= l) || f.smaller_than_mb.is_some_and(|s| mb >= s) {
            return false;
        }
    }
    if has_date_filter(f) {
        let Some(t) = filter_date(p, f) else { return false };
        return taken_matches(f, t, now);
    }
    true
}

/// Whether any filter looks at when the file was taken.
fn has_date_filter(f: &Filter) -> bool {
    f.since.is_some()
        || f.before.is_some()
        || f.older_than_days.is_some()
        || f.newer_than_days.is_some()
        || f.day.is_some()
        || f.month.is_some()
}

/// Whether a file taken at `t` passes the date filters. The caller reads the
/// creation date once per file, so every date check sees the same moment.
/// `now` is the planner's clock, so relative ages and calendar days agree.
fn taken_matches(f: &Filter, t: DateTime<Local>, now: DateTime<Local>) -> bool {
    let today = now.date_naive();
    let day = t.date_naive();
    if f.since.is_some_and(|s| day < s) || f.before.is_some_and(|b| day >= b) {
        return false;
    }
    let age_days = (now - t).num_seconds() as f64 / 86_400.0;
    if f.older_than_days.is_some_and(|d| age_days <= d as f64) || f.newer_than_days.is_some_and(|d| age_days > d as f64) {
        return false;
    }
    if f.month.is_some_and(|m| (t.year(), t.month()) != (month_year(m, f.year, today), m)) {
        return false;
    }
    match f.day {
        Some(DayRef::Today) => day == today,
        Some(DayRef::Yesterday) => day == today.pred_opt().unwrap_or(today),
        None => true,
    }
}

/// The year a month refers to: the one given, or else the most recent such
/// month ("August" in October means this August; in March, last August).
pub(crate) fn month_year(month: u32, year: Option<i32>, today: NaiveDate) -> i32 {
    year.unwrap_or(if month <= today.month() { today.year() } else { today.year() - 1 })
}

/// "report (1).pdf", "report 2.pdf", "report copy.pdf" -> "report.pdf".
fn base_name(name: &str) -> String {
    let lower = name.to_lowercase();
    let (stem, ext) = split_name(&lower);
    let mut s = stem.trim_end().to_string();
    for suffix in [" copy", "-copy", "_copy"] {
        if let Some(t) = s.strip_suffix(suffix) {
            s = t.to_string();
        }
    }
    if s.ends_with(')')
        && let Some(i) = s.rfind(" (").or_else(|| s.rfind('('))
        && s[i..].trim_start_matches([' ', '(']).trim_end_matches(')').chars().all(|c| c.is_ascii_digit())
    {
        s.truncate(i);
    } else if let Some((head, tail)) = s.rsplit_once(' ')
        && !tail.is_empty()
        && tail.len() <= 2
        && tail.chars().all(|c| c.is_ascii_digit())
    {
        s = head.to_string();
    }
    match ext {
        Some(e) => format!("{s}.{e}"),
        None => s,
    }
}

/// Lowercase letters and digits only: "Claude.dmg" and "claude dmg" agree.
fn loose(name: &str) -> String {
    name.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

fn replace_in_stem(name: &str, find: &str, rep: &str) -> Option<String> {
    if find.is_empty() {
        return None;
    }
    let (stem, ext) = split_name(name);
    let needle = find.to_lowercase();
    if needle.is_empty() {
        return None;
    }
    // Map each lowercased byte offset back to an original `stem` byte offset at
    // original char boundaries so chars whose lowercase changes byte length
    // (e.g. 'İ' -> "i\u{307}") never panic or slice mid-character.
    let mut lower = String::with_capacity(stem.len());
    let mut orig_at: Vec<Option<usize>> = Vec::with_capacity(stem.len() + 1);
    for (orig_byte, ch) in stem.char_indices() {
        orig_at.push(Some(orig_byte));
        let before = lower.len();
        for lc in ch.to_lowercase() {
            lower.push(lc);
        }
        for _ in 1..(lower.len() - before) {
            orig_at.push(None);
        }
    }
    orig_at.push(Some(stem.len()));

    // Case-insensitive replace that keeps the rest of the name's casing.
    let mut out = String::new();
    let mut orig_cursor = 0;
    let mut lower_cursor = 0;
    let mut matched = false;
    while lower_cursor <= lower.len() {
        let Some(rel) = lower[lower_cursor..].find(&needle) else {
            break;
        };
        let match_start = lower_cursor + rel;
        let match_end = match_start + needle.len();
        if let (Some(orig_start), Some(orig_end)) = (orig_at[match_start], orig_at[match_end]) {
            out.push_str(&stem[orig_cursor..orig_start]);
            out.push_str(rep);
            orig_cursor = orig_end;
            lower_cursor = match_end;
            matched = true;
        } else {
            let next_char_len = lower[match_start..].chars().next().map_or(1, char::len_utf8);
            lower_cursor = match_start + next_char_len;
        }
    }
    if !matched {
        return None;
    }
    out.push_str(&stem[orig_cursor..]);
    Some(match ext {
        Some(e) => format!("{out}.{e}"),
        None => out,
    })
}

fn image_width(p: &Path) -> Option<u32> {
    let out = Command::new("/usr/bin/sips").args(["-g", "pixelWidth"]).arg(p).output().ok()?;
    String::from_utf8_lossy(&out.stdout).lines().find_map(|l| l.trim().strip_prefix("pixelWidth: ")?.parse().ok())
}

fn entry_identity(path: &Path) -> PathBuf {
    // Two links can point to the same file and still be distinct rename or
    // delete targets. Resolve aliases to the parent, preserving the entry.
    match (path.parent(), path.file_name()) {
        (Some(parent), Some(name)) => fsx::canonical(parent).join(name),
        _ => fsx::canonical(path),
    }
}

fn exact_paths(name: &str, entries: impl IntoIterator<Item = PathBuf>) -> Vec<PathBuf> {
    let want = name.to_lowercase();
    let mut seen = HashSet::new();
    entries.into_iter().filter(|p| file_name(p).to_lowercase() == want && seen.insert(entry_identity(p))).collect()
}

/// Picks the entries with a name close to `name`: same stem ("claude" ->
/// Claude.dmg), same letters and digits ("claude dmg" -> Claude.dmg), containing
/// the words ("claude" -> Claude Setup.dmg), or holding every word of a
/// multi-word name in any order ("anigen android zip" -> Anigen-Android-1.zip,
/// which adds a version number). Closest first, then newest, so a numbered list
/// can be read out when there is more than one.
fn closest(name: &str, entries: Vec<PathBuf>) -> Vec<PathBuf> {
    const MAX_CANDIDATES: usize = 9;
    let want = loose(name);
    if want.len() < 3 {
        return Vec::new();
    }
    // The words of the name, skipping one-letter words, as they must appear in a file's name.
    let words: Vec<String> = name.split(|c: char| !c.is_alphanumeric()).map(loose).filter(|w| w.len() >= 2).collect();
    let mut scored: Vec<(u8, Option<DateTime<Local>>, PathBuf)> = Vec::new();
    let mut seen = HashSet::new();
    for p in &entries {
        let file = file_name(p);
        let stem = split_name(&file).0;
        let letters = loose(&file);
        let all_words = words.len() > 1 && words.iter().all(|w| letters.contains(w.as_str()));
        let score = if letters == want || loose(stem) == want {
            3
        } else if letters.contains(&want) || all_words {
            2
        } else {
            continue;
        };
        if seen.insert(entry_identity(p)) {
            scored.push((score, created(p), p.clone()));
        }
    }
    if scored.is_empty() && (5..=128).contains(&want.len()) {
        let query_ext = split_name(name).1;
        for p in entries {
            let file = file_name(&p);
            let (stem, ext) = split_name(&file);
            if query_ext.is_some_and(|q| !ext.is_some_and(|e| q.eq_ignore_ascii_case(e))) {
                continue;
            }
            let candidate = loose(if query_ext.is_some() { &file } else { stem });
            if near_typo(&want, &candidate) && seen.insert(entry_identity(&p)) {
                scored.push((1, created(&p), p));
            }
        }
    }
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)).then(a.2.cmp(&b.2)));
    scored.into_iter().take(MAX_CANDIDATES).map(|(_, _, p)| p).collect()
}

/// Spaces, underscores and hyphens are equivalent in filename filters.
fn normalized_name(name: &str) -> String {
    let mut result = String::with_capacity(name.len());
    let mut space = false;
    for ch in name.chars() {
        if ch.is_whitespace() || matches!(ch, '_' | '-') {
            space = !result.is_empty();
        } else {
            if space {
                result.push(' ');
                space = false;
            }
            result.extend(ch.to_lowercase());
        }
    }
    result
}

pub fn scope_matches(name: &str, paths: Vec<PathBuf>) -> bool {
    !closest(name, paths).is_empty()
}

fn strong_name_match(name: &str, path: &Path) -> bool {
    let file = file_name(path);
    file.eq_ignore_ascii_case(name) || loose(&file) == loose(name) || loose(split_name(&file).0) == loose(name)
}

/// Bounded Damerau-Levenshtein: one edit, or two for names of 12+ characters.
/// Adjacent transpositions count as one edit. Only used for suggestions.
fn near_typo(a: &str, b: &str) -> bool {
    let length = a.chars().count();
    let limit = if length >= 12 { 2 } else { 1 };
    if length.abs_diff(b.chars().count()) > limit || b.len() > 520 {
        return false;
    }
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    let mut older = previous.clone();
    for i in 1..=a.len() {
        let mut row = vec![limit + 1; b.len() + 1];
        row[0] = i;
        for j in i.saturating_sub(limit).max(1)..=b.len().min(i + limit) {
            row[j] = (previous[j] + 1).min(row[j - 1] + 1).min(previous[j - 1] + usize::from(a[i - 1] != b[j - 1]));
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                row[j] = row[j].min(older[j - 2] + 1);
            }
        }
        if row.iter().copied().min().unwrap_or(limit + 1) > limit {
            return false;
        }
        older = previous;
        previous = row;
    }
    previous[b.len()] <= limit
}

/// Asks Spotlight for entries below `dirs` with every word of `name` in their
/// file name, which finds what the walks are too shallow to reach. Answers
/// nothing if Spotlight is off or slow: searching just stops at the walks.
#[cfg(test)]
fn spotlight(name: &str, dirs: &[PathBuf]) -> Vec<PathBuf> {
    spotlight_checked(name, dirs, None)
}

fn spotlight_checked(name: &str, dirs: &[PathBuf], search: Option<&search::Search>) -> Vec<PathBuf> {
    // Letters and digits only, so a name can't break out of the quoted query.
    let terms: Vec<String> = name
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.chars().count() >= 2)
        .map(|t| format!("kMDItemFSName == \"*{}*\"c", t.to_lowercase()))
        .collect();
    if terms.is_empty() || search.is_some_and(search::Search::stopped) {
        return Vec::new();
    }
    let mut cmd = Command::new("/usr/bin/mdfind");
    for d in dirs {
        cmd.arg("-onlyin").arg(d);
    }
    let Ok(child) = cmd.arg(terms.join(" && ")).stdout(Stdio::piped()).stderr(Stdio::null()).spawn() else {
        if let Some(search) = search {
            search.issue("Spotlight could not start".into());
        }
        return Vec::new();
    };
    collect_spotlight(child, search, SPOTLIGHT_TIMEOUT)
}

fn collect_spotlight(mut child: std::process::Child, search: Option<&search::Search>, timeout: Duration) -> Vec<PathBuf> {
    let Some(out) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        if let Some(s) = search {
            s.issue("Spotlight output unavailable".into());
        }
        return Vec::new();
    };
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        use std::io::{BufRead, BufReader};
        let lines = BufReader::new(out).lines().take(MAX_INDEXED + 1).collect::<Result<Vec<_>, _>>();
        let _ = tx.send(lines);
    });
    let started = std::time::Instant::now();
    let mut received = false;
    let lines = loop {
        if search.is_some_and(search::Search::stopped) {
            break Vec::new();
        }
        if started.elapsed() >= timeout {
            if let Some(s) = search {
                s.spotlight_timeout();
            }
            break Vec::new();
        }
        let wait = Duration::from_millis(25)
            .min(timeout.saturating_sub(started.elapsed()))
            .min(search.map_or(Duration::MAX, search::Search::remaining_time));
        match rx.recv_timeout(wait) {
            Ok(Ok(lines)) => {
                received = true;
                break lines;
            }
            Ok(Err(_)) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                if let Some(s) = search {
                    s.issue("Spotlight results could not be read".into());
                }
                break Vec::new();
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    };
    if received && lines.len() <= MAX_INDEXED {
        loop {
            if search.is_some_and(search::Search::stopped) {
                break;
            }
            match child.try_wait() {
                Ok(Some(status)) => {
                    if !status.success()
                        && let Some(s) = search
                    {
                        s.issue("Spotlight exited unsuccessfully".into());
                    }
                    break;
                }
                Err(_) => {
                    if let Some(s) = search {
                        s.issue("Spotlight status unavailable".into());
                    }
                    break;
                }
                Ok(None) => {}
            }
            if started.elapsed() >= timeout {
                if let Some(s) = search {
                    s.spotlight_timeout();
                }
                break;
            }
            let wait = Duration::from_millis(1)
                .min(timeout.saturating_sub(started.elapsed()))
                .min(search.map_or(Duration::MAX, search::Search::remaining_time));
            std::thread::sleep(wait);
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    if lines.len() > MAX_INDEXED
        && let Some(s) = search
    {
        s.issue("Spotlight result limit reached".into());
    }
    lines.into_iter().take(MAX_INDEXED).map(PathBuf::from).collect()
}

fn plural<'s>(n: usize, one: &'s str, many: &'s str) -> &'s str {
    if n == 1 { one } else { many }
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

fn count_label(paths: &[PathBuf]) -> String {
    match paths {
        [one] => file_name(one),
        _ => format!("{} files", paths.len()),
    }
}

#[cfg(test)]
#[path = "plan/tests.rs"]
mod tests;
