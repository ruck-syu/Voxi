//! What Voxi learns from the person using it: "this was wrong" reports kept
//! on this Mac, phrases taught after a miss, and folder nicknames.
//!
//! Reports never leave the Mac. They are JSON lines a person can read,
//! delete, or choose to send, and `scripts/feedback-to-corpus.mjs` turns them
//! into regression-corpus candidates.

use crate::settings::{nickname_key, Nickname, Settings, Shortcut};
use chrono::Local;
use serde::Serialize;
use serde_json::{json, Value};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Reports past this many are not written, so the file stays small.
const MAX_REPORTS: usize = 2_000;
/// Longest note or phrase kept, in characters.
const MAX_TEXT: usize = 500;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Reports {
    pub count: usize,
    pub path: String,
}

fn clip(s: &str) -> String {
    s.trim().chars().take(MAX_TEXT).collect()
}

pub fn count(path: &Path) -> usize {
    fs::read_to_string(path).map(|s| s.lines().filter(|l| !l.trim().is_empty()).count()).unwrap_or(0)
}

pub fn summary(path: &Path) -> Reports {
    Reports { count: count(path), path: path.to_string_lossy().into_owned() }
}

/// Appends one report: the command, what the parser made of it, what the
/// person saw, and what they say it should have done.
pub fn record(path: &Path, command: &str, outcome: Value, note: &str) -> Result<Reports, String> {
    let command = clip(command);
    if command.is_empty() {
        return Err("There's no command to report.".into());
    }
    if count(path) >= MAX_REPORTS {
        return Err("The report file is full. Clear it in Settings first.".into());
    }
    let intent = crate::parse::parse(&command).ok().and_then(|i| serde_json::to_value(i).ok());
    let row = json!({
        "time": Local::now().to_rfc3339(),
        "version": env!("CARGO_PKG_VERSION"),
        "command": command,
        "intent": intent,
        "outcome": outcome,
        "note": clip(note),
    });
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path).map_err(|e| format!("Couldn't save the report: {e}"))?;
    writeln!(file, "{row}").map_err(|e| format!("Couldn't save the report: {e}"))?;
    Ok(summary(path))
}

pub fn clear(path: &Path) -> Result<Reports, String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(summary(path)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(summary(path)),
        Err(e) => Err(format!("Couldn't clear the reports: {e}")),
    }
}

/// Settings with `phrase` taught to run `command`. The phrase must be one
/// Voxi doesn't already understand, so teaching never hides a built-in
/// command; the command must be one it does.
pub fn teach_phrase(settings: &Settings, phrase: &str, command: &str) -> Result<Settings, String> {
    let phrase = phrase.trim().trim_end_matches(['.', '!', '?']).trim().to_string();
    let command = command.trim().to_string();
    if phrase.is_empty() || phrase.chars().count() > 120 {
        return Err("That phrase can't be saved.".into());
    }
    if crate::parse::parse(&phrase).is_ok() {
        return Err(format!("Voxi already understands “{phrase}”."));
    }
    if settings.find_shortcut(&phrase).is_some() {
        return Err(format!("“{phrase}” is already a shortcut."));
    }
    let mut next = settings.clone();
    next.shortcuts.push(Shortcut { phrase, commands: vec![command] });
    next.validate_shortcuts()?;
    Ok(next)
}

/// Settings with `name` meaning the folder at `path`.
pub fn add_nickname(settings: &Settings, name: &str, path: &Path, home: &Path) -> Result<Settings, String> {
    let mut next = settings.clone();
    let key = nickname_key(name);
    next.nicknames.retain(|n| nickname_key(&n.name) != key);
    next.nicknames.push(Nickname { name: key, path: tilde(path, home) });
    next.validate_nicknames(home)?;
    Ok(next)
}

fn tilde(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

pub fn reports_path(data_dir: &Path) -> PathBuf {
    data_dir.join("reports.jsonl")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_append_with_the_parsed_intent() {
        let dir = tempfile::tempdir().unwrap();
        let path = reports_path(dir.path());
        assert_eq!(summary(&path).count, 0);
        record(&path, "move report.pdf to Documents", json!({"status": "plan"}), "should copy").unwrap();
        let r = record(&path, "make me a sandwich", json!({"status": "not_understood"}), "").unwrap();
        assert_eq!(r.count, 2);
        let rows: Vec<Value> = fs::read_to_string(&path).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(rows[0]["intent"]["intent"], "move");
        assert_eq!(rows[0]["note"], "should copy");
        assert!(rows[1]["intent"].is_null());
        assert!(record(&path, "   ", json!(null), "").is_err());
        assert_eq!(clear(&path).unwrap().count, 0);
    }

    #[test]
    fn teaching_a_phrase_only_covers_misses() {
        let s = Settings::default();
        let taught = teach_phrase(&s, "tidy my downloads.", "sort Downloads by type").unwrap();
        assert_eq!(taught.shortcuts[0].phrase, "tidy my downloads");
        assert!(taught.find_shortcut("Tidy my downloads").is_some());
        assert!(teach_phrase(&s, "sort Downloads by type", "sort Downloads by type").is_err(), "already a command");
        assert!(teach_phrase(&taught, "tidy my downloads", "sort Downloads by type").is_err(), "already taught");
        assert!(teach_phrase(&s, "tidy my downloads", "make me a sandwich").is_err(), "command must parse");
    }

    #[test]
    fn nicknames_replace_an_older_meaning() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        for d in ["Documents/Dev", "Desktop/Dev"] {
            fs::create_dir_all(home.join(d)).unwrap();
        }
        let s = add_nickname(&Settings::default(), "the dev folder", &home.join("Documents/Dev"), home).unwrap();
        assert_eq!(s.nicknames, vec![Nickname { name: "dev".into(), path: "~/Documents/Dev".into() }]);
        let s = add_nickname(&s, "Dev", &home.join("Desktop/Dev"), home).unwrap();
        assert_eq!(s.nicknames, vec![Nickname { name: "dev".into(), path: "~/Desktop/Dev".into() }]);
    }
}
