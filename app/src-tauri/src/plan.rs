//! Turns a parsed intent into a concrete plan: resolves names against the
//! disk, expands filters into file lists, checks every path against the
//! allowed folders, and picks non-clobbering names for every output.

use crate::fsx::{self, Roots, file_name, human_size, split_name, unique_path};
use crate::history::History;
use crate::model::{
    Action, ARCHIVE_EXTS, AUDIO_EXTS, DayRef, DOC_EXTS, FileType, Filter, IMAGE_EXTS, ImageFormat, INSTALLER_EXTS, InfoRow,
    Intent, Source, VIDEO_EXTS,
};
use crate::pdf;
use chrono::{DateTime, Datelike, Local, NaiveDate};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

/// Why planning stopped early.
#[derive(Debug)]
pub enum Stop {
    Ambiguous { name: String, candidates: Vec<PathBuf> },
    Problem(String),
}

fn problem<T>(msg: impl Into<String>) -> Result<T, Stop> {
    Err(Stop::Problem(msg.into()))
}

/// One command's contribution: its title when it added steps to the shared
/// plan, or a finished result for read-only commands and undo.
enum Step {
    Changes(String),
    Finished(Planned),
}

pub enum Planned {
    Steps { title: String, actions: Vec<Action>, notes: Vec<Option<String>>, warnings: Vec<String>, undo_of: Option<u64> },
    Info { title: String, summary: String, rows: Vec<InfoRow>, total: usize },
    /// Read-only, runs right away without a preview.
    Direct { action: Action, title: String },
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
    now: DateTime<Local>,
}

const MAX_WALK: usize = 50_000;

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
            now: Local::now(),
        }
    }

    pub fn plan(mut self, intent: &Intent, history: &History) -> Result<Planned, Stop> {
        let title = match self.step(intent, history)? {
            Step::Changes(title) => title,
            Step::Finished(done) => return Ok(done),
        };
        if self.actions.is_empty() {
            let mut msg = "Nothing to change.".to_string();
            if let Some(w) = self.warnings.first() {
                msg = format!("{msg} {w}");
            }
            return problem(msg);
        }
        Ok(Planned::Steps { title, actions: self.actions, notes: self.notes, warnings: self.warnings, undo_of: None })
    }

    /// Plans a saved shortcut: every command goes into one preview, confirmed
    /// once and undone as one change. A command with nothing to do (no
    /// screenshots today, say) becomes a note instead of stopping the rest.
    pub fn plan_shortcut(mut self, name: &str, commands: &[(String, Intent)], history: &History) -> Result<Planned, Stop> {
        let mut notes_by_command = Vec::new();
        for (text, intent) in commands {
            let (actions, notes, warnings, dirs) =
                (self.actions.len(), self.notes.len(), self.warnings.len(), self.planned_dirs.clone());
            match self.step(intent, history) {
                Ok(Step::Changes(_)) if self.actions.len() == actions => {
                    let why = self.warnings.drain(warnings..).next().unwrap_or_else(|| "nothing to change".into());
                    notes_by_command.push(format!("“{text}”: {why}"));
                }
                Ok(Step::Changes(_)) => {}
                Ok(Step::Finished(_)) => {
                    return problem(format!("“{text}” only shows information, so it can't be part of “{name}”."));
                }
                Err(Stop::Problem(why)) => {
                    // Undo anything this command half-planned.
                    self.actions.truncate(actions);
                    self.notes.truncate(notes);
                    self.warnings.truncate(warnings);
                    self.planned_dirs = dirs;
                    notes_by_command.push(format!("“{text}”: {why}"));
                }
                Err(ambiguous) => return Err(ambiguous),
            }
        }
        if self.actions.is_empty() {
            return problem(format!("Nothing to change for “{name}”. {}", notes_by_command.join(" ")));
        }
        self.warnings.extend(notes_by_command);
        let n = commands.len();
        Ok(Planned::Steps {
            title: format!("{}: {n} {}", capitalize(name), plural(n, "command", "commands")),
            actions: self.actions,
            notes: self.notes,
            warnings: self.warnings,
            undo_of: None,
        })
    }

    fn step(&mut self, intent: &Intent, history: &History) -> Result<Step, Stop> {
        let title = match intent {
            Intent::Move { src, to } => self.transfer(src, to, false)?,
            Intent::Copy { src, to } => self.transfer(src, to, true)?,
            Intent::Rename { src, new_name } => self.rename_one(src, new_name)?,
            Intent::AddPrefix { src, prefix } => {
                let p = prefix.clone();
                self.batch_rename(src, "Add prefix", move |name, _| Some(format!("{p}{name}")))?
            }
            Intent::AddDatePrefix { src } => self.batch_rename(src, "Add date prefix", |name, path| {
                let date = created(path)?.format("%Y-%m-%d").to_string();
                let starts_dated = name.len() >= 10
                    && NaiveDate::parse_from_str(name.get(..10).unwrap_or(""), "%Y-%m-%d").is_ok();
                (!starts_dated).then(|| format!("{date} {name}"))
            })?,
            Intent::ReplaceInNames { src, find, replace } => {
                let (f, r) = (find.clone(), replace.clone());
                self.batch_rename(src, "Rename", move |name, _| replace_in_stem(name, &f, &r))?
            }
            Intent::Lowercase { src } => self.batch_rename(src, "Lowercase", |name, _| Some(name.to_lowercase()))?,
            Intent::Number { src, base, width } => self.number(src, base, *width)?,
            Intent::CreateFolders { names, parent } => self.create_folders(names, parent.as_deref())?,
            Intent::SortByType { folder } => self.sort(folder, true)?,
            Intent::SortByMonth { folder } => self.sort(folder, false)?,
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
                for a in &entry.inverse {
                    self.push(a.clone(), None);
                }
                return Ok(Step::Finished(Planned::Steps {
                    title: format!("Undo “{}”", entry.command),
                    actions: std::mem::take(&mut self.actions),
                    notes: std::mem::take(&mut self.notes),
                    warnings: std::mem::take(&mut self.warnings),
                    undo_of: Some(entry.id),
                }));
            }
        };
        Ok(Step::Changes(title))
    }

    fn push(&mut self, a: Action, note: Option<String>) {
        self.actions.push(a);
        self.notes.push(note);
    }

    fn check(&self, p: &Path) -> Result<(), Stop> {
        if self.roots.is_allowed(p) {
            Ok(())
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
        if dir.is_dir() || self.planned_dirs.contains(dir) {
            return Ok(());
        }
        self.check(dir)?;
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
        let name = name.trim().trim_end_matches('/');
        if fsx::has_parent_ref(Path::new(name)) {
            return problem("Paths with “..” aren't allowed.");
        }
        if let Some(p) = self.choices.get(name) {
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
        let base = if let Some(p) = self.roots.named(first) {
            p.clone()
        } else {
            let found = self.find_entries(first, true, &self.roots.search_roots(), 2);
            match found.len() {
                1 => found.into_iter().next().unwrap(),
                0 => match create_in {
                    Some(parent) => parent.join(first),
                    None => return problem(format!("Couldn't find a folder named “{first}”.")),
                },
                _ => return Err(Stop::Ambiguous { name: first.to_string(), candidates: found }),
            }
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

    /// Resolves a file or folder by name, searching the allowed folders.
    fn resolve_item(&self, name: &str, folder: Option<&str>) -> Result<PathBuf, Stop> {
        if fsx::has_parent_ref(Path::new(name)) {
            return problem("Paths with “..” aren't allowed.");
        }
        if let Some(p) = self.choices.get(name) {
            self.check(p)?;
            return Ok(p.clone());
        }
        if name.starts_with('/') || name.starts_with('~') {
            let p = fsx::expand_tilde(name, &self.roots.home);
            self.check(&p)?;
            return if p.exists() { Ok(p) } else { problem(format!("Couldn't find {}.", self.roots.display(&p))) };
        }
        if let Some((dir, file)) = name.rsplit_once('/') {
            let d = self.resolve_folder(dir, None)?;
            let p = d.join(file);
            return if p.exists() { Ok(p) } else { problem(format!("Couldn't find “{file}” in {}.", self.roots.display(&d))) };
        }
        let dirs = match folder {
            Some(f) => vec![self.resolve_folder(f, None)?],
            None => self.roots.search_roots(),
        };
        let mut found = self.find_entries(name, false, &dirs, 2);
        if found.is_empty() {
            found = self.find_similar(name, &dirs);
        }
        match found.len() {
            1 => Ok(found.into_iter().next().unwrap()),
            0 => {
                let wher = match folder {
                    Some(f) => f.to_string(),
                    None => "Desktop, Downloads, Documents, or Pictures".into(),
                };
                problem(format!("Couldn't find “{name}” in {wher}."))
            }
            _ => Err(Stop::Ambiguous { name: name.to_string(), candidates: found }),
        }
    }

    /// A file, or a folder by its spoken name (for "open Downloads").
    fn resolve_any(&self, name: &str) -> Result<PathBuf, Stop> {
        if let Some(p) = self.roots.named(name) {
            return Ok(p.clone());
        }
        self.resolve_item(name, None)
    }

    /// Entries named `name` (case-insensitive) up to `depth` levels below `dirs`.
    fn find_entries(&self, name: &str, dirs_only: bool, dirs: &[PathBuf], depth: usize) -> Vec<PathBuf> {
        let want = name.to_lowercase();
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        for d in dirs {
            walk(d, depth, &mut |p, is_dir| {
                if file_name(p).to_lowercase() == want && (is_dir || !dirs_only) && seen.insert(fsx::canonical(p)) {
                    out.push(p.to_path_buf());
                }
            });
        }
        out
    }

    /// Fallback for a vague or dictated name. Collects every entry whose name is
    /// close: same stem ("claude" -> Claude.dmg), same letters and digits
    /// ("claude dmg" -> Claude.dmg), containing the words ("claude" -> Claude
    /// Setup.dmg), or holding every word of a multi-word name in any order
    /// ("anigen android zip" -> Anigen-Android-1.zip, which adds a version
    /// number). Closest first, then newest, so a numbered list can be read out
    /// when there is more than one.
    fn find_similar(&self, name: &str, dirs: &[PathBuf]) -> Vec<PathBuf> {
        const MAX_CANDIDATES: usize = 9;
        let want = loose(name);
        if want.len() < 3 {
            return Vec::new();
        }
        // The words of the name, skipping one-letter words, as they must appear in a file's name.
        let words: Vec<String> = name.split_whitespace().map(loose).filter(|w| w.len() >= 2).collect();
        let mut scored: Vec<(u8, Option<DateTime<Local>>, PathBuf)> = Vec::new();
        let mut seen = HashSet::new();
        for d in dirs {
            walk(d, 2, &mut |p, _| {
                let file = file_name(p);
                let stem = split_name(&file).0;
                let letters = loose(&file);
                let all_words = words.len() > 1 && words.iter().all(|w| letters.contains(w.as_str()));
                let score = if letters == want || loose(stem) == want {
                    2
                } else if letters.contains(&want) || all_words {
                    1
                } else {
                    return;
                };
                if seen.insert(fsx::canonical(p)) {
                    scored.push((score, created(p), p.to_path_buf()));
                }
            });
        }
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
        scored.into_iter().take(MAX_CANDIDATES).map(|(_, _, p)| p).collect()
    }

    /// The folders a set looks in, and a short label for titles.
    fn set_folders(&self, folder: Option<&str>, filter: &Filter) -> Result<(Vec<PathBuf>, String), Stop> {
        match folder {
            Some(f) => {
                let p = self.resolve_folder(f, None)?;
                let label = file_name(&p);
                Ok((vec![p], label))
            }
            // Screenshots can be anywhere: every allowed folder, plus one
            // level of subfolders (see `expand`).
            None if filter.file_type == Some(FileType::Screenshots) => Ok((self.roots.search_roots(), "your folders".into())),
            None => Ok((vec![self.roots.downloads(), self.roots.desktop()], "Downloads and Desktop".into())),
        }
    }

    /// Expands a source into paths. Sets cover files directly in the folder
    /// (or every subfolder when `recursive`, for read-only queries).
    fn expand(&self, src: &Source, recursive: bool) -> Result<(Vec<PathBuf>, String), Stop> {
        match src {
            Source::Item { name, folder } => {
                let p = self.resolve_item(name, folder.as_deref())?;
                let label = file_name(&p);
                Ok((vec![p], label))
            }
            Source::Set { folder, filter } => {
                let (dirs, label) = self.set_folders(folder.as_deref(), filter)?;
                let today = self.now.date_naive();
                let mut out = Vec::new();
                let mut seen = HashSet::new();
                for d in &dirs {
                    let screenshots_anywhere = folder.is_none() && filter.file_type == Some(FileType::Screenshots);
                    let depth = if recursive { 12 } else if screenshots_anywhere { 1 } else { 0 };
                    walk(d, depth, &mut |p, is_dir| {
                        if !is_dir && seen.insert(p.to_path_buf()) && matches(p, filter, today) {
                            out.push(p.to_path_buf());
                        }
                    });
                }
                out.sort_by_key(|p| file_name(p).to_lowercase());
                Ok((out, label))
            }
        }
    }

    fn expand_nonempty(&self, src: &Source) -> Result<(Vec<PathBuf>, String), Stop> {
        let (paths, label) = self.expand(src, false)?;
        if paths.is_empty() {
            return problem(format!("No files match in {label}."));
        }
        Ok((paths, label))
    }

    // ---- Planners ----------------------------------------------------------

    fn transfer(&mut self, src: &Source, to: &str, copy: bool) -> Result<String, Stop> {
        let (paths, _) = self.expand_nonempty(src)?;
        let src_dir = match src {
            Source::Set { folder: Some(_), .. } => paths.first().and_then(|p| p.parent()).map(Path::to_path_buf),
            _ => None,
        };
        let parent_for_new = src_dir.unwrap_or_else(|| self.roots.desktop());
        let dest = self.resolve_folder(to, Some(&parent_for_new))?;
        self.ensure_dir(&dest)?;
        let mut already = 0;
        for p in &paths {
            self.check(p)?;
            if !copy && p.parent().is_some_and(|d| fsx::same_file(d, &dest)) {
                already += 1;
                continue;
            }
            if fsx::canonical(&dest).starts_with(fsx::canonical(p)) {
                return problem(format!("Can't put {} inside itself.", file_name(p)));
            }
            let (to, note) = self.target(&dest, &file_name(p));
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
        let Source::Item { name, folder } = src else { return problem("Rename one file at a time, or use a rule like “replace X with Y”.") };
        let path = self.resolve_item(name, folder.as_deref())?;
        self.check(&path)?;
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

    fn batch_rename(
        &mut self,
        src: &Source,
        verb: &str,
        f: impl Fn(&str, &Path) -> Option<String>,
    ) -> Result<String, Stop> {
        let (paths, label) = self.expand_nonempty(src)?;
        let mut unchanged = 0;
        for p in &paths {
            self.check(p)?;
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
            let ext = split_name(&file_name(p)).1.map(|e| format!(".{}", e.to_lowercase())).unwrap_or_default();
            let new = format!("{base}{:0width$}{ext}", i + 1);
            if new != file_name(p) {
                self.push_rename(p, &new);
            }
        }
        Ok(format!("Number {} in {label} as {base}{:0width$}…", count_label(&paths), 1))
    }

    fn create_folders(&mut self, names: &[String], parent: Option<&str>) -> Result<String, Stop> {
        let parent = match parent {
            Some(p) => self.resolve_folder(p, None)?,
            None => self.roots.desktop(),
        };
        for n in names {
            if n.contains('/') || n.starts_with('.') {
                return problem(format!("“{n}” isn't a valid folder name."));
            }
            let p = parent.join(n);
            if p.exists() {
                self.warnings.push(format!("{n} already exists in {}.", file_name(&parent)));
            } else {
                self.ensure_dir(&p)?;
            }
        }
        let what = if names.len() == 1 { format!("folder {}", names[0]) } else { format!("{} folders", names.len()) };
        Ok(format!("Create {what} in {}", file_name(&parent)))
    }

    fn sort(&mut self, folder: &str, by_type: bool) -> Result<String, Stop> {
        let dir = self.resolve_folder(folder, None)?;
        let filter = Filter { file_type: Some(FileType::Files), ..Default::default() };
        let (paths, _) = self.expand(&Source::Set { folder: Some(dir.to_string_lossy().into()), filter }, false)?;
        let mut left = 0;
        let mut moved = 0;
        for p in &paths {
            let sub = if by_type {
                match type_folder(p) {
                    Some(s) => s.to_string(),
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
            self.warnings.push(format!("{skipped} {} skipped (not an image, or already {}).", plural(skipped, "file", "files"), format.ext()));
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
            let backup = unique_path(&backup_dir, &file_name(p), &mut self.reserved);
            self.push(
                Action::ResizeImage { path: p.clone(), max_width_px: width, backup },
                Some("Original kept in _to_delete/.voxi-originals".into()),
            );
        }
        if small > 0 {
            self.warnings.push(format!("{small} {} already {width} px wide or smaller, left alone.", plural(small, "image is", "images are")));
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
            return problem(format!("Found {} PDF in {label}; merging needs at least two.", pdfs.len()));
        }
        pdfs.iter().try_for_each(|p| self.check(p))?;
        let (out, note) = self.output_path(output, pdfs[0].parent().unwrap(), "pdf")?;
        let n = pdfs.len();
        self.push(Action::MergePdf { paths: pdfs, output: out.clone() }, note);
        Ok(format!("Merge {n} PDFs into {}", file_name(&out)))
    }

    fn single(&self, src: &Source, ext: &str, what: &str) -> Result<PathBuf, Stop> {
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
            return problem(format!("No images match in {label}."));
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
        let [p] = paths.as_slice() else { return problem("Name one file or folder to zip.") };
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
        let wanted = match destination {
            Some(d) => self.resolve_folder(d, Some(&parent))?,
            None => parent.join(p.file_stem().unwrap()),
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
        if let Source::Set { filter, .. } = src {
            if !filter.is_narrowing() {
                return problem("To be safe, clean-up needs a filter, like “older than 90 days”, “installers”, or “larger than 500 MB”.");
            }
        }
        let (paths, label) = self.expand_nonempty(src)?;
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

    fn file_row(&self, p: &Path) -> InfoRow {
        let meta = fs::metadata(p).ok();
        let size = if p.is_dir() { fsx::tree_size(p).0 } else { meta.as_ref().map_or(0, |m| m.len()) };
        let date = created(p).map(|d| d.format("%b %-d, %Y").to_string()).unwrap_or_default();
        let dir = p.parent().map(|d| self.roots.display(d)).unwrap_or_default();
        InfoRow { name: file_name(p), detail: format!("{} · {date} · {dir}", human_size(size)) }
    }

    fn list(&self, src: &Source) -> Result<Planned, Stop> {
        let (mut paths, label) = self.expand(src, true)?;
        let by_size = matches!(src, Source::Set { filter, .. } if filter.larger_than_mb.is_some() || filter.smaller_than_mb.is_some());
        if by_size {
            paths.sort_by_key(|p| std::cmp::Reverse(fs::metadata(p).map_or(0, |m| m.len())));
        } else {
            paths.sort_by_key(|p| std::cmp::Reverse(created(p)));
        }
        let total_bytes: u64 = paths.iter().map(|p| fs::metadata(p).map_or(0, |m| m.len())).sum();
        let rows = paths.iter().take(200).map(|p| self.file_row(p)).collect();
        Ok(Planned::Info {
            title: format!("{} in {label}", count_label(&paths)),
            summary: if paths.is_empty() { "Nothing matches.".into() } else { format!("{} in total, including subfolders.", human_size(total_bytes)) },
            rows,
            total: paths.len(),
        })
    }

    fn count(&self, src: &Source) -> Result<Planned, Stop> {
        let (paths, label) = self.expand(src, true)?;
        let what = match src {
            Source::Set { filter, .. } => type_label(filter),
            _ => "files",
        };
        Ok(Planned::Info {
            title: format!("{} {what} in {label}", paths.len()),
            summary: "Including subfolders.".into(),
            rows: paths.iter().take(20).map(|p| self.file_row(p)).collect(),
            total: paths.len(),
        })
    }

    fn size(&self, folder: &str) -> Result<Planned, Stop> {
        let p = match self.roots.named(folder) {
            Some(p) => p.clone(),
            None => self.resolve_any(folder)?,
        };
        let (bytes, files) = fsx::tree_size(&p);
        let mut children: Vec<(u64, PathBuf)> = fs::read_dir(&p)
            .map(|rd| rd.flatten().map(|e| e.path()).filter(|c| !fsx::is_hidden_or_internal(c)).map(|c| (fsx::tree_size(&c).0, c)).collect())
            .unwrap_or_default();
        children.sort_by_key(|(b, _)| std::cmp::Reverse(*b));
        let rows = children
            .iter()
            .take(15)
            .map(|(b, c)| InfoRow { name: file_name(c), detail: format!("{}{}", human_size(*b), if c.is_dir() { " · folder" } else { "" }) })
            .collect();
        Ok(Planned::Info {
            title: format!("{} uses {}", file_name(&p), human_size(bytes)),
            summary: format!("{files} {} in total. Largest items:", plural(files, "file", "files")),
            rows,
            total: children.len(),
        })
    }

    fn duplicates(&self, folder: &str) -> Result<Planned, Stop> {
        let dir = self.resolve_folder(folder, None)?;
        let mut groups: BTreeMap<(String, u64), Vec<PathBuf>> = BTreeMap::new();
        walk(&dir, 12, &mut |p, is_dir| {
            if !is_dir {
                if let Ok(m) = fs::metadata(p) {
                    groups.entry((base_name(&file_name(p)), m.len())).or_default().push(p.to_path_buf());
                }
            }
        });
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
            title: format!("{} {} in {}", dupes.len(), plural(dupes.len(), "set of duplicates", "sets of duplicates"), file_name(&dir)),
            summary: if dupes.is_empty() { "No duplicates by name and size.".into() } else { format!("Matched by name and size. Extra copies use {}.", human_size(wasted)) },
            rows,
            total: dupes.len(),
        })
    }
}

fn recent(history: &History, roots: &Roots) -> Planned {
    let rows: Vec<InfoRow> = history
        .entries
        .iter()
        .rev()
        .take(20)
        .map(|e| {
            let status = if e.undone { " · undone" } else if e.undo_of.is_some() { " · undo" } else { "" };
            let first = e.done.first().map(|a| describe_path(a, roots)).unwrap_or_default();
            InfoRow {
                name: e.command.clone(),
                detail: format!("{} · {} {}{status}{first}", e.time.format("%b %-d, %H:%M"), e.done.len(), plural(e.done.len(), "change", "changes")),
            }
        })
        .collect();
    Planned::Info {
        title: "Recent actions".into(),
        summary: if rows.is_empty() { "Nothing yet.".into() } else { "Newest first. “undo last” reverses the newest one that isn't undone.".into() },
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

fn type_folder(p: &Path) -> Option<&'static str> {
    let e = ext_of(p);
    Some(match e.as_str() {
        "pdf" => "PDFs",
        _ if is_ext(&e, IMAGE_EXTS) => "Images",
        _ if is_ext(&e, VIDEO_EXTS) => "Videos",
        _ if is_ext(&e, DOC_EXTS) => "Documents",
        _ if is_ext(&e, ARCHIVE_EXTS) => "Archives",
        _ if is_ext(&e, INSTALLER_EXTS) => "Installers",
        _ if is_ext(&e, AUDIO_EXTS) => "Audio",
        _ => return None,
    })
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

pub fn matches(p: &Path, f: &Filter, today: NaiveDate) -> bool {
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
    if f.name_contains.as_ref().is_some_and(|n| !file_name(p).to_lowercase().contains(&n.to_lowercase())) {
        return false;
    }
    if f.name_starts_with.as_ref().is_some_and(|n| !file_name(p).to_lowercase().starts_with(&n.to_lowercase())) {
        return false;
    }
    if f.larger_than_mb.is_some() || f.smaller_than_mb.is_some() {
        let mb = fs::metadata(p).map_or(0.0, |m| m.len() as f64 / (1024.0 * 1024.0));
        if f.larger_than_mb.is_some_and(|l| mb <= l) || f.smaller_than_mb.is_some_and(|s| mb >= s) {
            return false;
        }
    }
    if has_date_filter(f) {
        let Some(t) = created(p) else { return false };
        return taken_matches(f, t, today);
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
fn taken_matches(f: &Filter, t: DateTime<Local>, today: NaiveDate) -> bool {
    let day = t.date_naive();
    if f.since.is_some_and(|s| day < s) || f.before.is_some_and(|b| day >= b) {
        return false;
    }
    let age_days = (Local::now() - t).num_seconds() as f64 / 86_400.0;
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
    if s.ends_with(')') {
        if let Some(i) = s.rfind(" (").or_else(|| s.rfind('(')) {
            if s[i..].trim_start_matches([' ', '(']).trim_end_matches(')').chars().all(|c| c.is_ascii_digit()) {
                s.truncate(i);
            }
        }
    } else if let Some((head, tail)) = s.rsplit_once(' ') {
        if !tail.is_empty() && tail.len() <= 2 && tail.chars().all(|c| c.is_ascii_digit()) {
            s = head.to_string();
        }
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
    let lower = stem.to_lowercase();
    let needle = find.to_lowercase();
    if !lower.contains(&needle) {
        return None;
    }
    // Case-insensitive replace that keeps the rest of the name's casing.
    let mut out = String::new();
    let mut i = 0;
    while let Some(pos) = lower[i..].find(&needle) {
        out.push_str(&stem[i..i + pos]);
        out.push_str(rep);
        i += pos + needle.len();
    }
    out.push_str(&stem[i..]);
    Some(match ext {
        Some(e) => format!("{out}.{e}"),
        None => out,
    })
}

fn image_width(p: &Path) -> Option<u32> {
    let out = Command::new("/usr/bin/sips").args(["-g", "pixelWidth"]).arg(p).output().ok()?;
    String::from_utf8_lossy(&out.stdout).lines().find_map(|l| l.trim().strip_prefix("pixelWidth: ")?.parse().ok())
}

/// Visits entries below `dir` up to `depth` extra levels, skipping hidden
/// files, `_to_delete/`, app bundles, and symlinked folders.
pub fn walk(dir: &Path, depth: usize, f: &mut dyn FnMut(&Path, bool)) {
    let mut budget = MAX_WALK;
    walk_inner(dir, depth, f, &mut budget);
}

fn walk_inner(dir: &Path, depth: usize, f: &mut dyn FnMut(&Path, bool), budget: &mut usize) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = rd.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        if *budget == 0 {
            return;
        }
        *budget -= 1;
        let p = e.path();
        if fsx::is_hidden_or_internal(&p) {
            continue;
        }
        let Ok(ft) = e.file_type() else { continue };
        let is_dir = ft.is_dir();
        f(&p, is_dir);
        let is_bundle = matches!(ext_of(&p).as_str(), "app" | "photoslibrary" | "bundle" | "framework");
        if is_dir && depth > 0 && !is_bundle {
            walk_inner(&p, depth - 1, f, budget);
        }
    }
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
mod tests {
    use super::*;
    use crate::parse::parse;
    use std::fs;

    struct Home {
        _dir: tempfile::TempDir,
        roots: Roots,
    }

    fn home() -> Home {
        let dir = tempfile::tempdir().unwrap();
        let h = fsx::canonical(dir.path());
        for d in ["Desktop", "Downloads", "Documents", "Pictures"] {
            fs::create_dir(h.join(d)).unwrap();
        }
        let roots = Roots::new(&h, None, &[]);
        Home { _dir: dir, roots }
    }

    fn plan(h: &Home, cmd: &str) -> Result<Planned, Stop> {
        let choices = HashMap::new();
        let history = History::load(h.roots.home.join("history.json"));
        Planner::new(&h.roots, &choices).plan(&parse(cmd).unwrap(), &history)
    }

    fn steps(h: &Home, cmd: &str) -> Vec<Action> {
        match plan(h, cmd) {
            Ok(Planned::Steps { actions, .. }) => actions,
            Ok(_) => panic!("expected steps"),
            Err(e) => panic!("{e:?}"),
        }
    }

    #[test]
    fn move_adds_overwrite_suffix() {
        let h = home();
        fs::write(h.roots.home.join("Downloads/report.pdf"), b"a").unwrap();
        fs::write(h.roots.home.join("Documents/report.pdf"), b"b").unwrap();
        let a = steps(&h, "move report.pdf from Downloads to Documents");
        assert_eq!(
            a,
            vec![Action::Move { from: h.roots.home.join("Downloads/report.pdf"), to: h.roots.home.join("Documents/report 2.pdf") }]
        );
    }

    #[test]
    fn singular_folder_name_resolves() {
        let h = home();
        fs::write(h.roots.home.join("Downloads/Claude.dmg"), b"a").unwrap();
        let a = steps(&h, "delete the claude.dmg file in download folder");
        assert_eq!(a.last(), Some(&Action::Trash { path: h.roots.home.join("Downloads/Claude.dmg") }));
    }

    #[test]
    fn dictated_name_without_dot_resolves() {
        let h = home();
        fs::write(h.roots.home.join("Downloads/Claude.dmg"), b"a").unwrap();
        let a = steps(&h, "Delete the Claude dmg file from the Downloads folder.");
        assert!(matches!(a.last(), Some(Action::Trash { path, .. }) if *path == h.roots.home.join("Downloads/Claude.dmg")));
    }

    #[test]
    fn vague_name_lists_similar_files_closest_first() {
        let h = home();
        let dl = h.roots.home.join("Downloads");
        fs::write(dl.join("Claude Setup.dmg"), b"a").unwrap();
        fs::write(dl.join("Claude.dmg"), b"a").unwrap();
        fs::write(dl.join("notes.txt"), b"a").unwrap();
        match plan(&h, "delete claude from downloads") {
            Err(Stop::Ambiguous { name, candidates }) => {
                assert_eq!(name, "claude");
                assert_eq!(candidates, vec![dl.join("Claude.dmg"), dl.join("Claude Setup.dmg")]);
            }
            _ => panic!("expected a question"),
        }
    }

    #[test]
    fn picked_candidate_is_used() {
        let h = home();
        let dl = h.roots.home.join("Downloads");
        fs::write(dl.join("Claude Setup.dmg"), b"a").unwrap();
        fs::write(dl.join("Claude.dmg"), b"a").unwrap();
        let choices = HashMap::from([("claude".to_string(), dl.join("Claude Setup.dmg"))]);
        let history = History::load(h.roots.home.join("history.json"));
        let Ok(Planned::Steps { actions, .. }) = Planner::new(&h.roots, &choices).plan(&parse("delete claude from downloads").unwrap(), &history) else {
            panic!("expected a plan")
        };
        assert!(matches!(actions.last(), Some(Action::Trash { path, .. }) if *path == dl.join("Claude Setup.dmg")));
    }

    #[test]
    fn shortcut_plans_all_commands_and_notes_the_idle_ones() {
        let h = home();
        let desk = h.roots.home.join("Desktop");
        fs::write(desk.join("Screenshot 2026-10-28 at 10.00.00.png"), b"a").unwrap();
        let commands: Vec<(String, Intent)> = ["move screenshots from today to Screenshots", "delete installers from Downloads"]
            .iter()
            .map(|c| (c.to_string(), parse(c).unwrap()))
            .collect();
        let choices = HashMap::new();
        let history = History::load(h.roots.home.join("history.json"));
        let Ok(Planned::Steps { title, actions, warnings, .. }) =
            Planner::new(&h.roots, &choices).plan_shortcut("run my usuals", &commands, &history)
        else {
            panic!("expected one combined plan")
        };
        assert_eq!(title, "Run my usuals: 2 commands");
        assert!(actions.iter().any(|a| matches!(a, Action::Move { to, .. } if to.ends_with("Screenshots/Screenshot 2026-10-28 at 10.00.00.png"))));
        assert!(warnings.iter().any(|w| w.starts_with("“delete installers from Downloads”")), "{warnings:?}");
    }

    #[test]
    fn screenshots_are_found_outside_the_desktop() {
        let h = home();
        let pics = h.roots.home.join("Pictures/Screenshots");
        fs::create_dir_all(&pics).unwrap();
        fs::write(pics.join("Screenshot 2026-10-28 at 10.00.00.png"), b"a").unwrap();
        fs::write(h.roots.home.join("Downloads/Screenshot 2026-10-27 at 09.00.00.png"), b"a").unwrap();
        let Ok(Planned::Info { total, .. }) = plan(&h, "list screenshots") else { panic!("expected a list") };
        assert_eq!(total, 2);
    }

    #[test]
    fn ambiguous_name_is_not_picked_silently() {
        let h = home();
        fs::write(h.roots.home.join("Downloads/notes.txt"), b"a").unwrap();
        fs::write(h.roots.home.join("Desktop/notes.txt"), b"b").unwrap();
        assert!(matches!(plan(&h, "copy notes.txt to Documents"), Err(Stop::Ambiguous { candidates, .. }) if candidates.len() == 2));
    }

    #[test]
    fn delete_becomes_move_to_trash() {
        let h = home();
        fs::write(h.roots.home.join("Downloads/setup.dmg"), b"a").unwrap();
        fs::write(h.roots.home.join("Downloads/keep.txt"), b"a").unwrap();
        let a = steps(&h, "delete installers from Downloads");
        assert_eq!(a, vec![Action::Trash { path: h.roots.home.join("Downloads/setup.dmg") }]);
    }

    #[test]
    fn unfiltered_clean_up_is_refused() {
        let h = home();
        fs::write(h.roots.home.join("Downloads/a.txt"), b"a").unwrap();
        assert!(matches!(plan(&h, "delete files in Downloads"), Err(Stop::Problem(_))));
    }

    #[test]
    fn paths_outside_allowed_folders_are_refused() {
        let h = home();
        assert!(matches!(plan(&h, "move ~/Library/x.txt to Documents"), Err(Stop::Problem(_))));
        assert!(matches!(plan(&h, "move ~/Downloads/../x.txt to Documents"), Err(Stop::Problem(_))));
    }

    #[test]
    fn missing_destination_is_created() {
        let h = home();
        fs::write(h.roots.home.join("Downloads/invoice-1.pdf"), b"a").unwrap();
        let a = steps(&h, "move files containing invoice from Downloads to Invoices");
        let dest = h.roots.home.join("Downloads/Invoices");
        assert_eq!(a[0], Action::CreateFolder { path: dest.clone() });
        assert_eq!(a[1], Action::Move { from: h.roots.home.join("Downloads/invoice-1.pdf"), to: dest.join("invoice-1.pdf") });
    }

    #[test]
    fn sort_by_type_groups_known_types() {
        let h = home();
        for f in ["a.pdf", "b.png", "c.weird"] {
            fs::write(h.roots.home.join("Downloads").join(f), b"x").unwrap();
        }
        let a = steps(&h, "sort Downloads by type");
        let d = h.roots.home.join("Downloads");
        assert!(a.contains(&Action::Move { from: d.join("a.pdf"), to: d.join("PDFs/a.pdf") }));
        assert!(a.contains(&Action::Move { from: d.join("b.png"), to: d.join("Images/b.png") }));
        assert_eq!(a.iter().filter(|x| matches!(x, Action::Move { .. })).count(), 2);
    }

    #[test]
    fn month_filter_matches_files_from_that_month() {
        let h = home();
        let pics = h.roots.home.join("Pictures");
        let august = pics.join("Screenshot 2026-08-15 at 10.00.00.png");
        fs::write(&august, b"a").unwrap();
        fs::write(pics.join("Screenshot today.png"), b"a").unwrap();
        let when = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_786_788_000); // 2026-08-15
        fs::File::options().write(true).open(&august).unwrap().set_modified(when).unwrap();
        let Ok(Planned::Info { total, rows, .. }) = plan(&h, "find screenshots taken in August 2026") else { panic!("expected a list") };
        assert_eq!(total, 1);
        assert_eq!(rows[0].name, "Screenshot 2026-08-15 at 10.00.00.png");
    }

    #[test]
    fn month_without_a_year_is_the_most_recent_one() {
        let oct = NaiveDate::from_ymd_opt(2026, 10, 28).unwrap();
        assert_eq!(month_year(8, None, oct), 2026);
        assert_eq!(month_year(12, None, oct), 2025);
        assert_eq!(month_year(10, None, oct), 2026);
        assert_eq!(month_year(3, Some(2024), oct), 2024);
    }

    /// Sets a file's date as `touch -t` would. On macOS an earlier modified
    /// time also moves the creation date, which is what the date filters read.
    fn set_taken(p: &Path, (y, m, d): (i32, u32, u32)) {
        let at = NaiveDate::from_ymd_opt(y, m, d).unwrap().and_hms_opt(12, 0, 0).unwrap().and_local_timezone(Local).unwrap();
        fs::File::options().write(true).open(p).unwrap().set_modified(at.into()).unwrap();
    }

    #[test]
    fn screenshots_before_and_after_a_month_are_split_by_date() {
        let h = home();
        let pics = h.roots.home.join("Pictures");
        let june = pics.join("Screenshot 2026-06-10 at 09.00.00.png");
        let august = pics.join("Screenshot 2026-08-02 at 09.00.00.png");
        fs::write(&june, b"a").unwrap();
        fs::write(&august, b"a").unwrap();
        set_taken(&june, (2026, 6, 10));
        set_taken(&august, (2026, 8, 2));
        let Ok(Planned::Info { total, rows, .. }) = plan(&h, "Show me the screenshots taken before July 2026") else {
            panic!("expected a list")
        };
        assert_eq!(total, 1);
        assert_eq!(rows[0].name, "Screenshot 2026-06-10 at 09.00.00.png");
        let Ok(Planned::Info { total, rows, .. }) = plan(&h, "list screenshots taken after July 2026") else { panic!("expected a list") };
        assert_eq!(total, 1);
        assert_eq!(rows[0].name, "Screenshot 2026-08-02 at 09.00.00.png");
    }

    #[test]
    fn name_prefix_filter_matches_only_the_start_of_the_name() {
        let h = home();
        let dl = h.roots.home.join("Downloads");
        fs::write(dl.join("invoice-june.pdf"), b"a").unwrap();
        fs::write(dl.join("old invoice.pdf"), b"a").unwrap();
        let a = steps(&h, "delete files that start with invoice from Downloads");
        assert_eq!(a, vec![Action::Trash { path: dl.join("invoice-june.pdf") }]);
    }

    #[test]
    fn a_name_matches_a_file_that_adds_a_version_number() {
        let h = home();
        let dl = h.roots.home.join("Downloads");
        fs::write(dl.join("Anigen-Android-1.zip"), b"a").unwrap();
        fs::write(dl.join("notes.txt"), b"a").unwrap();
        let a = steps(&h, "delete anigen android zip from downloads");
        assert_eq!(a, vec![Action::Trash { path: dl.join("Anigen-Android-1.zip") }]);
    }

    #[test]
    fn trashing_a_named_file_says_its_name_once() {
        let h = home();
        fs::write(h.roots.home.join("Downloads/setup.dmg"), b"a").unwrap();
        let Ok(Planned::Steps { title, .. }) = plan(&h, "delete setup.dmg from downloads") else { panic!("expected a plan") };
        assert_eq!(title, "Move setup.dmg to the Trash");
    }

    #[test]
    fn a_name_that_fits_two_files_asks_which_one() {
        let h = home();
        let dl = h.roots.home.join("Downloads");
        fs::write(dl.join("Anigen-Android-1.zip"), b"a").unwrap();
        fs::write(dl.join("Anigen-Android-2.zip"), b"a").unwrap();
        assert!(matches!(
            plan(&h, "delete anigen android zip from downloads"),
            Err(Stop::Ambiguous { candidates, .. }) if candidates.len() == 2
        ));
    }

    #[test]
    fn duplicate_base_names() {
        assert_eq!(base_name("Report (1).pdf"), "report.pdf");
        assert_eq!(base_name("report 2.pdf"), "report.pdf");
        assert_eq!(base_name("report copy.pdf"), "report.pdf");
        assert_eq!(base_name("Chapter 12.pdf"), "chapter.pdf");
    }

    #[test]
    fn replace_keeps_extension_and_case() {
        assert_eq!(replace_in_stem("My Draft v2.docx", "draft", "final").as_deref(), Some("My final v2.docx"));
        assert_eq!(replace_in_stem("a b c.txt", " ", "-").as_deref(), Some("a-b-c.txt"));
        assert_eq!(replace_in_stem("abc.txt", "z", "y"), None);
    }
}
