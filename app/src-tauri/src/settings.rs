//! User settings, stored as JSON in the app's config folder.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// Clear of Wispr Flow's defaults (Fn, Fn+Space) and of Spotlight (Cmd+Space).
pub const DEFAULT_HOTKEY: &str = "Control+Alt+Space";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub hotkey: String,
    /// Folders allowed beyond Desktop, Downloads, Documents, and Pictures.
    pub extra_folders: Vec<String>,
    pub start_at_login: bool,
    pub onboarded: bool,
    /// Saved phrases that run several commands at once ("run my usuals").
    pub shortcuts: Vec<Shortcut>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Shortcut {
    pub phrase: String,
    pub commands: Vec<String>,
}

/// Words a person might put around a shortcut's name when saying it.
const SHORTCUT_VERBS: &[&str] = &["run", "do", "start", "play", "execute", "launch"];
const SHORTCUT_NOUNS: &[&str] = &["shortcut", "routine", "commands"];

/// Compares shortcut phrases loosely, so "Run my usuals.", "my usuals" and
/// "do my usuals" all reach a shortcut saved as "run my usuals".
pub fn shortcut_key(text: &str) -> String {
    let normalized = crate::parse::phrase::normalize(text).to_lowercase();
    let mut words: Vec<&str> = normalized.split_whitespace().collect();
    while words.first().is_some_and(|w| SHORTCUT_VERBS.contains(w)) {
        words.remove(0);
    }
    while words.last().is_some_and(|w| SHORTCUT_NOUNS.contains(w)) {
        words.pop();
    }
    words.iter().map(|w| singular(w)).collect::<Vec<_>>().join(" ")
}

/// "usuals" -> "usual", "batches" -> "batch", "dailies" -> "daily", so a
/// dropped or added plural doesn't miss the shortcut.
fn singular(w: &str) -> String {
    if w.len() <= 3 || w.ends_with("ss") {
        w.to_string()
    } else if let Some(stem) = w.strip_suffix("ies") {
        format!("{stem}y")
    } else if let Some(stem) = w.strip_suffix("ches").or_else(|| w.strip_suffix("shes")).or_else(|| w.strip_suffix("xes")) {
        format!("{stem}{}", &w[stem.len()..w.len() - 2])
    } else if let Some(stem) = w.strip_suffix('s') {
        stem.to_string()
    } else {
        w.to_string()
    }
}

/// Letters that differ between two strings (insertions, deletions, swaps).
fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut prev = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cur = row[j + 1];
            row[j + 1] = (prev + usize::from(ca != *cb)).min(row[j] + 1).min(cur + 1);
            prev = cur;
        }
    }
    row[b.len()]
}

impl Default for Settings {
    fn default() -> Self {
        Settings { hotkey: DEFAULT_HOTKEY.into(), extra_folders: Vec::new(), start_at_login: false, onboarded: false, shortcuts: Vec::new() }
    }
}

impl Settings {
    pub fn load(path: &Path) -> Self {
        fs::read_to_string(path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        fs::write(path, serde_json::to_string_pretty(self).unwrap()).map_err(|e| e.to_string())
    }

    pub fn find_shortcut(&self, text: &str) -> Option<&Shortcut> {
        let key = shortcut_key(text);
        (!key.is_empty()).then(|| self.shortcuts.iter().find(|s| shortcut_key(&s.phrase) == key)).flatten()
    }

    /// A shortcut whose phrase is a near miss for `text` ("run my visuals"
    /// for "run my usuals"): a couple of letters off at most, and only one
    /// shortcut that close. Used only when the text isn't a command.
    pub fn find_close_shortcut(&self, text: &str) -> Option<&Shortcut> {
        let key = shortcut_key(text);
        let allowed = (key.chars().count() / 3).clamp(1, 3);
        let mut close: Vec<(usize, &Shortcut)> = self
            .shortcuts
            .iter()
            .map(|s| (edit_distance(&key, &shortcut_key(&s.phrase)), s))
            .filter(|(d, _)| *d <= allowed)
            .collect();
        close.sort_by_key(|(d, _)| *d);
        match close.as_slice() {
            [(_, only)] => Some(*only),
            [(a, best), (b, _), ..] if a < b => Some(*best),
            _ => None,
        }
    }

    /// Trims the shortcuts and checks that every command is one Voxi can
    /// run, so mistakes show up in Settings rather than when a shortcut runs.
    pub fn validate_shortcuts(&mut self) -> Result<(), String> {
        use crate::model::Intent;
        let mut seen = std::collections::HashSet::new();
        let mut kept = Vec::new();
        for s in &self.shortcuts {
            let phrase = s.phrase.trim().to_string();
            let commands: Vec<String> = s.commands.iter().map(|c| c.trim().to_string()).filter(|c| !c.is_empty()).collect();
            if phrase.is_empty() && commands.is_empty() {
                continue;
            }
            if phrase.is_empty() {
                return Err("Give each shortcut a phrase to say, like “run my usuals”.".into());
            }
            if commands.is_empty() {
                return Err(format!("“{phrase}” has no commands yet."));
            }
            if shortcut_key(&phrase).is_empty() || !seen.insert(shortcut_key(&phrase)) {
                return Err(format!("There is already a shortcut called “{phrase}”."));
            }
            for c in &commands {
                match crate::parse::parse(c) {
                    Err(_) => return Err(format!("In “{phrase}”, Voxi doesn't understand “{c}”.")),
                    Ok(
                        Intent::List { .. }
                        | Intent::Count { .. }
                        | Intent::Size { .. }
                        | Intent::Duplicates { .. }
                        | Intent::Reveal { .. }
                        | Intent::RecentActions
                        | Intent::Undo,
                    ) => return Err(format!("In “{phrase}”, “{c}” doesn't change files, so it can't be part of a shortcut.")),
                    Ok(_) => {}
                }
            }
            kept.push(Shortcut { phrase, commands });
        }
        self.shortcuts = kept;
        Ok(())
    }

    pub fn extra_paths(&self, home: &Path) -> Vec<PathBuf> {
        self.extra_folders
            .iter()
            .map(|f| crate::fsx::expand_tilde(f.trim(), home))
            .filter(|p| p.is_absolute() && p.is_dir() && !crate::fsx::has_parent_ref(p) && *p != home && p.parent().is_some())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with(phrase: &str, commands: &[&str]) -> Settings {
        Settings {
            shortcuts: vec![Shortcut { phrase: phrase.into(), commands: commands.iter().map(|c| c.to_string()).collect() }],
            ..Default::default()
        }
    }

    #[test]
    fn shortcut_matches_spoken_variations() {
        let s = with("run my usuals", &["move screenshots from today to Screenshots"]);
        for said in ["run my usuals", "Run my usuals.", "my usuals", "Do my usuals please", "run usuals shortcut"] {
            assert!(s.find_shortcut(said).is_some(), "{said}");
        }
        assert!(s.find_shortcut("run my usual").is_some(), "singular");
        assert!(s.find_shortcut("Run my usual.").is_some(), "singular, dictated");
        assert!(s.find_shortcut("run").is_none());
        assert!(s.find_shortcut("move my usuals to Documents").is_none());
    }

    #[test]
    fn near_miss_shortcut_phrases_are_caught() {
        let s = with("run my usuals", &["sort Downloads by type"]);
        assert!(s.find_close_shortcut("run my visuals").is_some());
        assert!(s.find_close_shortcut("run my usuels").is_some());
        assert!(s.find_close_shortcut("run my errands").is_none());
        assert!(s.find_close_shortcut("hello").is_none());
    }

    #[test]
    fn invalid_shortcut_commands_are_rejected_on_save() {
        assert!(with("tidy", &["make me a sandwich"]).validate_shortcuts().is_err());
        assert!(with("tidy", &["list files in Downloads"]).validate_shortcuts().is_err());
        assert!(with("", &["sort Downloads by type"]).validate_shortcuts().is_err());
        let mut ok = with(" tidy ", &["sort Downloads by type", "  "]);
        ok.validate_shortcuts().unwrap();
        assert_eq!(ok.shortcuts[0].phrase, "tidy");
        assert_eq!(ok.shortcuts[0].commands, vec!["sort Downloads by type"]);
    }
}
