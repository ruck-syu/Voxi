//! Action log: every executed plan with its inverse, so `undo` can reverse it.

use crate::model::Action;
use crate::settings::{StorageWarning, load_store, save_store};
use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const MAX_ENTRIES: usize = 200;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub id: u64,
    pub time: DateTime<Local>,
    pub command: String,
    pub done: Vec<Action>,
    /// Steps that reverse `done`, already in the order they should run.
    pub inverse: Vec<Action>,
    #[serde(default)]
    pub undone: bool,
    /// Set when this entry is itself an undo of entry `undo_of`.
    #[serde(default)]
    pub undo_of: Option<u64>,
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
    pub fn try_push(&mut self, entry: Entry) -> Result<(), String> {
        if let Some(target) = entry.undo_of
            && let Some(e) = self.entries.iter_mut().find(|e| e.id == target)
        {
            let completed = entry.done.len().min(e.inverse.len());
            e.inverse.drain(..completed);
            e.undone = e.inverse.is_empty();
        }
        self.entries.push(entry);
        let excess = self.entries.len().saturating_sub(MAX_ENTRIES);
        self.entries.drain(..excess);
        self.save()
    }

    /// The most recent change that hasn't been undone yet.
    pub fn last_undoable(&self) -> Option<&Entry> {
        self.entries.iter().rev().find(|e| !e.undone && e.undo_of.is_none() && !e.inverse.is_empty())
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
        save_store(&self.path, &self.entries, crate::settings::diagnostics::Store::History)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::WRITE_FAILURE;
    use std::fs;

    fn entry() -> Entry {
        Entry { id: 7, time: Local::now(), command: "original command".into(), done: vec![], inverse: vec![], undone: false, undo_of: None }
    }

    #[test]
    fn legacy_history_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.json");
        let legacy = serde_json::to_vec(&vec![entry()]).unwrap();
        fs::write(&path, &legacy).unwrap();
        let history = History::load(path.clone());
        assert_eq!(fs::read(&path).unwrap(), legacy);
        history.save().unwrap();
        let json: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(json["version"], 1);
        assert_eq!(History::load(path).entries[0].command, "original command");
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
            assert!(history.entries[0].undone);
            assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
        }
    }
}
