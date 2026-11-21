//! Action log: every executed plan with its inverse, so `undo` can reverse it.

use crate::fsx::FileIdentity;
use crate::model::Action;
use crate::settings::{StorageWarning, load_store, save_store};
use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

const MAX_ENTRIES: usize = 200;
const MAX_SNAPSHOT_DESCENDANTS: usize = 500;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Inverse {
    pub(crate) action: Action,
    identities: Vec<IdentityCheck>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct IdentityCheck {
    root: FileIdentity,
    descendants: BTreeMap<PathBuf, FileIdentity>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    opaque: BTreeSet<PathBuf>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    truncated: bool,
}

impl IdentityCheck {
    fn subtree(&self, relative: &Path) -> Option<Self> {
        if relative.as_os_str().is_empty() {
            return Some(self.clone());
        }
        let root = self.descendants.get(relative)?.clone();
        let descendants = self
            .descendants
            .iter()
            .filter_map(|(path, identity)| {
                let child = path.strip_prefix(relative).ok()?;
                (!child.as_os_str().is_empty()).then(|| (child.to_path_buf(), identity.clone()))
            })
            .collect();
        let opaque = self.opaque.iter().filter_map(|path| path.strip_prefix(relative).ok().map(Path::to_path_buf)).collect();
        Some(Self { root, descendants, opaque, truncated: false })
    }

    fn contains(&self, relative: &Path) -> bool {
        relative.as_os_str().is_empty() || self.descendants.contains_key(relative)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct IdentityUpdates(BTreeMap<PathBuf, IdentityCheck>);

impl IdentityUpdates {
    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Later inverses may use a path only after an earlier inverse restores
    /// it. Carry the actual published identity, including across volumes, so
    /// a retry needs no fresh guesses about the new occupant of a path.
    pub(crate) fn rebind(&mut self, placed_at: &Path, evidence: &Inverse, remaining: &mut [Inverse]) {
        let Some(snapshot) = evidence.identities.first() else { return };
        for inverse in remaining {
            for (path, expected) in inverse.checks_mut() {
                if let Ok(relative) = path.strip_prefix(placed_at)
                    && let Some(actual) = snapshot.subtree(relative)
                {
                    *expected = actual.clone();
                    self.0.insert(path.to_path_buf(), actual);
                }
            }
        }
    }

    fn apply(&self, remaining: &mut [Inverse]) {
        for inverse in remaining {
            for (path, expected) in inverse.checks_mut() {
                if let Some(actual) = self.0.get(path) {
                    *expected = actual.clone();
                }
            }
        }
    }
}

fn snapshot(path: &Path, expected: &FileIdentity, empty_folder_only: bool, top_level_only: bool) -> Result<IdentityCheck, String> {
    enum Walk {
        Complete,
        Truncated,
    }
    fn descendants(
        root: &Path,
        directory: &Path,
        found: &mut BTreeMap<PathBuf, FileIdentity>,
        opaque: &mut BTreeSet<PathBuf>,
    ) -> std::io::Result<Walk> {
        let entries = match fs::read_dir(directory) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                let relative = directory.strip_prefix(root).expect("directory is at or below root");
                opaque.insert(relative.to_path_buf());
                return Ok(Walk::Complete);
            }
            Err(e) => return Err(e),
        };
        for child in entries {
            let path = match child {
                Ok(entry) => entry.path(),
                Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                    let relative = directory.strip_prefix(root).expect("directory is at or below root");
                    opaque.insert(relative.to_path_buf());
                    return Ok(Walk::Complete);
                }
                Err(e) => return Err(e),
            };
            let meta = match fs::symlink_metadata(&path) {
                Ok(meta) => meta,
                Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                    let relative = directory.strip_prefix(root).expect("directory is at or below root");
                    opaque.insert(relative.to_path_buf());
                    return Ok(Walk::Complete);
                }
                Err(e) => return Err(e),
            };
            if found.len() >= MAX_SNAPSHOT_DESCENDANTS {
                return Ok(Walk::Truncated);
            }
            let relative = path.strip_prefix(root).expect("read_dir yields paths below the root");
            found.insert(relative.to_path_buf(), FileIdentity::from_metadata(&meta));
            if meta.is_dir() && matches!(descendants(root, &path, found, opaque)?, Walk::Truncated) {
                return Ok(Walk::Truncated);
            }
        }
        Ok(Walk::Complete)
    }

    let changed = || format!("Voxi can't undo this change because {} is missing, was replaced, or has changed.", path.display());
    let meta = fs::symlink_metadata(path).map_err(|_| changed())?;
    let root = FileIdentity::from_metadata(&meta);
    let matches = if empty_folder_only { meta.is_dir() && root.same_object(expected) } else { root == *expected };
    if !matches {
        return Err(changed());
    }
    let mut children = BTreeMap::new();
    let mut opaque = BTreeSet::new();
    let mut truncated = top_level_only && !empty_folder_only;
    if meta.is_dir()
        && !empty_folder_only
        && !top_level_only
        && matches!(descendants(path, path, &mut children, &mut opaque).map_err(|_| changed())?, Walk::Truncated)
    {
        children.clear();
        opaque.clear();
        truncated = true;
    }
    Ok(IdentityCheck { root, descendants: children, opaque, truncated })
}

impl Inverse {
    fn paths(action: &Action) -> Result<Vec<&Path>, String> {
        match action {
            Action::Move { from, .. } | Action::RestoreFromTrash { from, .. } => Ok(vec![from]),
            Action::Rename { path, .. } | Action::Trash { path } | Action::RemoveEmptyFolder { path } => Ok(vec![path]),
            Action::Restore { from, to } => Ok(vec![from, to]),
            _ => Err("Unsupported undo action.".into()),
        }
    }

    pub(crate) fn capture(action: Action, expected: &[FileIdentity]) -> Result<Self, String> {
        let paths = Self::paths(&action)?;
        if paths.len() != expected.len() {
            return Err("Undo identities are incomplete.".into());
        }
        let empty_folder_only = matches!(action, Action::RemoveEmptyFolder { .. });
        let identities = paths
            .into_iter()
            .zip(expected)
            .map(|(path, id)| snapshot(path, id, empty_folder_only, false))
            .collect::<Result<_, _>>()?;
        Ok(Self { action, identities })
    }

    fn checks_mut(&mut self) -> impl Iterator<Item = (&Path, &mut IdentityCheck)> {
        Self::paths(&self.action).unwrap_or_default().into_iter().zip(&mut self.identities)
    }

    pub(crate) fn placement(&self) -> Option<PathBuf> {
        match &self.action {
            Action::Move { to, .. } | Action::RestoreFromTrash { to, .. } | Action::Restore { to, .. } => Some(to.clone()),
            Action::Rename { path, new_name } => Some(path.with_file_name(new_name)),
            _ => None,
        }
    }

    pub(crate) fn validate(&self) -> Result<(), String> {
        self.validate_except(|_| false)
    }

    fn validate_except(&self, supplied_by_earlier_step: impl Fn(&Path) -> bool) -> Result<(), String> {
        let paths = Self::paths(&self.action)?;
        if paths.len() != self.identities.len() {
            return Err("Voxi can't safely undo this older change because its file identities were never recorded.".into());
        }
        let empty_folder_only = matches!(self.action, Action::RemoveEmptyFolder { .. });
        for (path, expected) in paths.into_iter().zip(&self.identities) {
            if supplied_by_earlier_step(path) {
                continue;
            }
            let actual = snapshot(path, &expected.root, empty_folder_only, expected.truncated)?;
            // Our own steps can put files in a created folder and take them
            // back out. Its inverse uses rmdir, which only removes it empty.
            if !empty_folder_only && actual != *expected {
                return Err(format!("Voxi can't undo this change because the contents of {} have changed.", path.display()));
            }
        }
        Ok(())
    }
}

pub(crate) fn validate_inverses(inverses: &[Inverse]) -> Result<(), String> {
    let mut placements: Vec<(PathBuf, &IdentityCheck)> = Vec::new();
    for inverse in inverses {
        inverse.validate_except(|path| {
            placements.iter().rev().any(|(placed, identity)| path.strip_prefix(placed).is_ok_and(|relative| identity.contains(relative)))
        })?;
        if let Some(path) = inverse.placement() {
            placements.push((path, &inverse.identities[0]));
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(try_from = "Vec<Inverse>")]
pub(crate) struct Remaining(Vec<Inverse>);

impl TryFrom<Vec<Inverse>> for Remaining {
    type Error = &'static str;

    fn try_from(inverses: Vec<Inverse>) -> Result<Self, Self::Error> {
        if inverses.is_empty() { Err("Undo state requires at least one inverse") } else { Ok(Self(inverses)) }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum EntryState {
    Irreversible,
    Undoable(Remaining),
    PartiallyUndone(Remaining),
    Exhausted,
}

impl EntryState {
    pub(crate) fn with_inverses(inverses: Vec<Inverse>) -> Self {
        Remaining::try_from(inverses).map_or(Self::Irreversible, Self::Undoable)
    }

    fn remaining(&self) -> &[Inverse] {
        match self {
            Self::Undoable(remaining) | Self::PartiallyUndone(remaining) => &remaining.0,
            Self::Irreversible | Self::Exhausted => &[],
        }
    }

    fn reverse_prefix(&mut self, count: usize, updates: &IdentityUpdates) {
        let remaining = match self {
            Self::Undoable(remaining) | Self::PartiallyUndone(remaining) if count > 0 => remaining,
            _ => return,
        };
        let mut rest = remaining.0.split_off(count.min(remaining.0.len()));
        updates.apply(&mut rest);
        *self = Remaining::try_from(rest).map_or(Self::Exhausted, Self::PartiallyUndone);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(try_from = "EntryData")]
pub struct Entry {
    pub id: u64,
    pub time: DateTime<Local>,
    pub command: String,
    pub done: Vec<Action>,
    pub(crate) state: EntryState,
    /// The target of an undo attempt, and the number of its inverses completed.
    pub undo_of: Option<u64>,
    pub(crate) reversed: usize,
    #[serde(skip_serializing_if = "IdentityUpdates::is_empty")]
    pub(crate) identity_updates: IdentityUpdates,
}

#[derive(Deserialize)]
struct EntryData {
    id: u64,
    time: DateTime<Local>,
    command: String,
    done: Vec<Action>,
    state: Option<EntryState>,
    undo_of: Option<u64>,
    reversed: Option<usize>,
    #[serde(default)]
    identity_updates: IdentityUpdates,
    // Legacy entries lack identity evidence. Keep them visible in History.
    #[serde(default)]
    inverse: Vec<Action>,
    #[serde(default)]
    undone: bool,
}

impl TryFrom<EntryData> for Entry {
    type Error = &'static str;

    fn try_from(data: EntryData) -> Result<Self, Self::Error> {
        let legacy = data.state.is_none();
        let reversed = data.reversed.unwrap_or_else(|| if legacy && data.undo_of.is_some() { data.done.len() } else { 0 });
        if reversed > data.done.len()
            || (reversed > 0 && data.undo_of.is_none())
            || (!data.identity_updates.is_empty() && reversed == 0)
            || data.undo_of == Some(data.id)
            || (!legacy && (data.undone || !data.inverse.is_empty()))
        {
            return Err("Invalid undo attempt in history");
        }
        let state = data.state.unwrap_or_else(|| {
            if data.undone {
                EntryState::Exhausted
            } else {
                EntryState::with_inverses(data.inverse.into_iter().map(|action| Inverse { action, identities: vec![] }).collect())
            }
        });
        Ok(Self {
            id: data.id,
            time: data.time,
            command: data.command,
            done: data.done,
            state,
            undo_of: data.undo_of,
            reversed,
            identity_updates: data.identity_updates,
        })
    }
}

impl Entry {
    pub(crate) fn new(id: u64, command: String, undo_of: Option<u64>) -> Self {
        Self {
            id,
            time: Local::now(),
            command,
            done: vec![],
            state: EntryState::Irreversible,
            undo_of,
            reversed: 0,
            identity_updates: IdentityUpdates::default(),
        }
    }

    pub(crate) fn inverses(&self) -> &[Inverse] {
        self.state.remaining()
    }
}

pub struct History {
    pub(crate) notices: Vec<String>,
    path: PathBuf,
    pub entries: Vec<Entry>,
    warning: Option<StorageWarning>,
}

impl History {
    #[allow(dead_code)] // Compatibility helper for benchmarks and tests.
    pub fn load(path: PathBuf) -> Self {
        let (store, warning) = Self::load_with_warning(path);
        if let Some(warning) = warning {
            eprintln!("{}", warning.message);
        }
        store
    }

    pub fn load_with_warning(path: PathBuf) -> (Self, Option<StorageWarning>) {
        let (entries, warning) = load_store(&path, crate::settings::diagnostics::Store::History);
        (History { path, entries, notices: vec![], warning: warning.clone() }, warning)
    }

    pub(crate) fn journal_path(&self) -> PathBuf {
        self.path.with_file_name("operation-journal.json")
    }

    pub fn next_id(&self) -> u64 {
        self.entries.iter().map(|e| e.id).max().unwrap_or(0) + 1
    }

    #[cfg(test)]
    pub fn push(&mut self, entry: Entry) {
        if let Err(error) = self.try_push(entry) {
            eprintln!("{error}");
        }
    }

    /// Keeps the in-memory action evidence even if persistence fails. Callers
    /// must report the error instead of promising durable undo (T5).
    pub fn try_push(&mut self, mut entry: Entry) -> Result<(), String> {
        self.ensure_writable()?;
        if !self.entries.iter().any(|existing| existing.id == entry.id) {
            if let Some(target) = entry.undo_of
                && let Some(original) = self.entries.iter_mut().find(|e| e.id == target)
            {
                original.state.reverse_prefix(entry.reversed, &entry.identity_updates);
            }
            entry.identity_updates = IdentityUpdates::default();
            self.entries.push(entry);
            let excess = self.entries.len().saturating_sub(MAX_ENTRIES);
            self.entries.drain(..excess);
        }
        self.save()
    }

    /// The most recent change that hasn't been undone yet. An undo attempt
    /// counts too when it left a repair behind (a partial undo records the
    /// steps that put things back); a completed undo records none.
    pub fn last_undoable(&self) -> Option<&Entry> {
        self.entries.iter().rev().find(|e| !e.inverses().is_empty())
    }

    pub(crate) fn undoable(&self, id: u64) -> Result<&Entry, String> {
        self.entries
            .iter()
            .find(|e| e.id == id && !e.inverses().is_empty())
            .ok_or_else(|| "That history entry is no longer undoable.".into())
    }

    pub(crate) fn ensure_writable(&self) -> Result<(), String> {
        if let Some(warning) = &self.warning
            && warning.write_blocked
        {
            return Err(warning.message.clone());
        }
        Ok(())
    }

    pub fn save(&self) -> Result<(), String> {
        if let Err(error) = self.ensure_writable() {
            crate::settings::diagnostics::emit(
                crate::settings::diagnostics::Event::new(
                    crate::settings::diagnostics::Code::StoreSave,
                    crate::settings::diagnostics::Stage::Failed,
                )
                .store(crate::settings::diagnostics::Store::History),
            );
            return Err(error);
        }
        save_store(&self.path, &self.entries, crate::settings::diagnostics::Store::History)?.require_durable()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::WRITE_FAILURE;
    use std::fs;

    fn entry() -> Entry {
        Entry::new(7, "original command".into(), None)
    }

    #[test]
    fn legacy_history_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.json");
        let legacy = serde_json::to_vec(&serde_json::json!([{
            "id": 7, "time": Local::now(), "command": "original command",
            "done": [], "inverse": [{"kind":"trash", "path": "/tmp/old"}], "undone": false
        }]))
        .unwrap();
        fs::write(&path, &legacy).unwrap();
        let history = History::load(path.clone());
        assert_eq!(fs::read(&path).unwrap(), legacy);
        history.save().unwrap();
        let json: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(json["version"], 1);
        let loaded = History::load(path);
        let restored = loaded.last_undoable().unwrap();
        assert_eq!(restored.command, "original command");
        assert!(restored.inverses()[0].validate().unwrap_err().contains("identities were never recorded"));
    }

    #[test]
    fn corrupt_history_preserved_future_rejected_missing_and_unreadable_distinct() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.json");
        assert!(History::load_with_warning(path.clone()).1.is_none());
        assert!(!path.exists());
        for bytes in [b"[{".as_slice(), b"invalid JSON", br#"{"version":2,"data":[]}"#] {
            fs::write(&path, bytes).unwrap();
            let (mut history, warning) = History::load_with_warning(path.clone());
            let warning = warning.unwrap();
            assert_eq!(fs::read(warning.backup.unwrap()).unwrap(), bytes);
            assert_eq!(fs::read(&path).unwrap(), bytes);
            if warning.write_blocked {
                assert!(history.try_push(entry()).is_err());
                assert_eq!(fs::read(&path).unwrap(), bytes);
            } else {
                history.try_push(entry()).unwrap();
                assert_eq!(History::load(path.clone()).entries.len(), 1);
            }
        }
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();
        let (mut history, warning) = History::load_with_warning(path.clone());
        assert!(warning.unwrap().write_blocked);
        assert!(history.try_push(entry()).is_err());
        assert!(path.is_dir());
    }

    #[test]
    fn failed_history_writes_keep_disk_original_and_memory_evidence() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.json");
        let mut history = History::load(path.clone());
        history.try_push(entry()).unwrap();
        let original = fs::read(&path).unwrap();
        for stage in 1..=3 {
            WRITE_FAILURE.with(|f| f.set(stage));
            let result = history.try_push(Entry { id: 8, undo_of: Some(7), ..entry() });
            WRITE_FAILURE.with(|f| f.set(0));
            assert!(result.is_err());
            assert_eq!(fs::read(&path).unwrap(), original);
            assert!(history.entries.iter().any(|entry| entry.id == 8));
            assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
        }
    }

    #[test]
    fn snapshot_tolerates_unreadable_subfolder_as_opaque_and_validates_undo() {
        use std::os::unix::fs::PermissionsExt;
        struct RestoreMode(PathBuf);
        impl Drop for RestoreMode {
            fn drop(&mut self) {
                let _ = fs::set_permissions(&self.0, fs::Permissions::from_mode(0o755));
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("folder");
        let secret = folder.join("secret");
        fs::create_dir_all(&secret).unwrap();
        fs::write(secret.join("inside.txt"), b"hidden").unwrap();
        fs::write(folder.join("visible.txt"), b"visible").unwrap();
        fs::set_permissions(&secret, fs::Permissions::from_mode(0o000)).unwrap();
        let _restore = RestoreMode(secret.clone());

        let action = Action::Move { from: folder.clone(), to: dir.path().join("dest") };
        let id = FileIdentity::read(&folder).unwrap();
        let inverse = Inverse::capture(action, &[id]).expect("unreadable subfolder must not break snapshot capture");
        inverse.validate().expect("unchanged folder with unreadable subfolder must validate for undo");

        // Modifying a visible child still invalidates undo.
        fs::write(folder.join("visible.txt"), b"edited").unwrap();
        assert!(inverse.validate().is_err());
    }

    #[test]
    fn snapshot_degrades_to_top_level_identity_beyond_descendant_cap() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("huge");
        fs::create_dir(&folder).unwrap();
        for i in 0..=MAX_SNAPSHOT_DESCENDANTS {
            fs::write(folder.join(format!("f{i}.txt")), b"x").unwrap();
        }
        let action = Action::Move { from: folder.clone(), to: dir.path().join("dest") };
        let id = FileIdentity::read(&folder).unwrap();
        let inverse = Inverse::capture(action, &[id]).unwrap();
        assert!(inverse.identities[0].truncated);
        assert!(inverse.identities[0].descendants.is_empty());
        let encoded = serde_json::to_vec(&inverse).unwrap();
        assert!(encoded.len() < 1024, "truncated inverse should stay compact, got {} bytes", encoded.len());
        inverse.validate().unwrap();
        // Top-level changes still invalidate undo.
        fs::write(folder.join("added.txt"), b"new").unwrap();
        assert!(inverse.validate().is_err());
    }
}
