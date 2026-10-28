//! Action log: every executed plan with its inverse, so `undo` can reverse it.

use crate::model::Action;
use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use std::fs;
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
    path: PathBuf,
    pub entries: Vec<Entry>,
}

impl History {
    pub fn load(path: PathBuf) -> Self {
        let entries = fs::read_to_string(&path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        History { path, entries }
    }

    pub fn next_id(&self) -> u64 {
        self.entries.iter().map(|e| e.id).max().unwrap_or(0) + 1
    }

    pub fn push(&mut self, entry: Entry) {
        if let Some(target) = entry.undo_of {
            if let Some(e) = self.entries.iter_mut().find(|e| e.id == target) {
                e.undone = true;
            }
        }
        self.entries.push(entry);
        let excess = self.entries.len().saturating_sub(MAX_ENTRIES);
        self.entries.drain(..excess);
        self.save();
    }

    /// The most recent change that hasn't been undone yet.
    pub fn last_undoable(&self) -> Option<&Entry> {
        self.entries.iter().rev().find(|e| !e.undone && e.undo_of.is_none() && !e.inverse.is_empty())
    }

    fn save(&self) {
        if let Some(dir) = self.path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_string_pretty(&self.entries) {
            let tmp = self.path.with_extension("json.tmp");
            if fs::write(&tmp, json).is_ok() {
                let _ = fs::rename(&tmp, &self.path);
            }
        }
    }
}
