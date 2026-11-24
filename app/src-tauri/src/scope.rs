//! Session-local snapshots of complete, ordered list results.
use crate::{
    model::{Action, FileType, Filter, Intent, Source},
    parse,
};
use regex::Regex;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::LazyLock,
};

#[derive(Clone)]
struct Entry {
    path: PathBuf,
    identity: (u64, u64),
    removed: bool,
}
fn identity(path: &Path) -> Option<(u64, u64)> {
    let m = fs::symlink_metadata(path).ok()?;
    if !m.is_file() {
        return None;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Some((m.dev(), m.ino()))
    }
    #[cfg(not(unix))]
    {
        Some((m.len(), m.created().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_nanos() as u64))
    }
}

#[derive(Clone)]
pub struct Snapshot {
    entries: Vec<Entry>,
    pub label: String,
    pub folder: Option<PathBuf>,
}
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub count: usize,
    pub label: String,
    pub visible: usize,
}
#[derive(Default)]
pub struct State {
    pub generation: u64,
    pub current: Option<Snapshot>,
}
impl State {
    pub fn clear(&mut self) {
        self.generation += 1;
        self.current = None;
    }
    pub fn publish(&mut self, snapshot: Snapshot) {
        self.generation += 1;
        self.current = Some(snapshot);
    }
    pub fn summary(&self) -> Option<Summary> {
        self.current.as_ref().map(Snapshot::summary)
    }
}
impl Snapshot {
    pub fn capture(paths: Vec<PathBuf>, label: String) -> Option<Self> {
        Self::capture_list(paths, label, None)
    }
    pub fn capture_list(paths: Vec<PathBuf>, label: String, folder: Option<PathBuf>) -> Option<Self> {
        let entries: Option<Vec<_>> =
            paths.into_iter().map(|path| Some(Entry { identity: identity(&path)?, path, removed: false })).collect();
        Some(Self { entries: entries?, label, folder })
    }
    pub fn paths(&self) -> Vec<PathBuf> {
        self.entries.iter().filter(|e| !e.removed).map(|e| e.path.clone()).collect()
    }
    pub fn summary(&self) -> Summary {
        Summary {
            count: self.entries.iter().filter(|e| !e.removed).count(),
            visible: self.entries.iter().take(200).filter(|e| !e.removed).count(),
            label: self.label.clone(),
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.entries.iter().any(|e| !e.removed && identity(&e.path) != Some(e.identity)) {
            return Err("The listed files changed. Run the list command again before using these results.".into());
        }
        Ok(())
    }
    pub fn apply(&mut self, actions: &[Action]) {
        for action in actions {
            for entry in &mut self.entries {
                if entry.removed {
                    continue;
                }
                let next = match action {
                    Action::Move { from, to } if *from == entry.path => Some(to.clone()),
                    Action::Rename { path, new_name } if *path == entry.path => path.parent().map(|p| p.join(new_name)),
                    _ => None,
                };
                if let Some(next) = next {
                    entry.path = next;
                }
                // Only refresh identities for files this confirmed action changed.
                let touched = match action {
                    Action::Move { to, .. } => *to == entry.path,
                    Action::Rename { path, new_name } => path.parent().is_some_and(|p| p.join(new_name) == entry.path),
                    Action::ResizeImage { path, .. } | Action::Restore { to: path, .. } => *path == entry.path,
                    _ => false,
                };
                if touched && let Some(id) = identity(&entry.path) {
                    entry.identity = id;
                }
            }
            // Keep the original row slots so later numbers never shift to another file.
            if let Action::Trash { path } = action {
                for entry in &mut self.entries {
                    if entry.path == *path {
                        entry.removed = true;
                    }
                }
            }
        }
    }
}

pub(crate) fn subfolder_boundary(folder: Option<&Path>) -> Result<&Path, String> {
    folder.ok_or_else(|| "Name the folder whose subfolders to use, such as “delete PDFs in Downloads from subfolders”.".into())
}

pub(crate) fn is_nested_in(path: &Path, folder: &Path) -> bool {
    crate::fsx::canonical(path).parent().is_some_and(|parent| parent != folder && parent.starts_with(folder))
}

const MARKER: &str = "__voxi_result_reference__";
static REFERENCES: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
    r"(?i)\b(?:(?:first|second|third|fourth|fifth|sixth|seventh|eighth|ninth|tenth|last|\d+(?:st|nd|rd|th))\s+(?:files?|items?|ones?|1)|(?:file|item|number)\s+\d+|(?:these|those|listed|search)\s+(?:files|items|results)|them|these|those|results|everything|all|it|file|\d+)\b"
).unwrap()
});

/// Only source arguments are substituted. Quoted names and destinations remain literal.
pub fn parse_followup(text: &str, snapshot: &Snapshot) -> Result<Option<Intent>, String> {
    crate::parse::check_length(text).map_err(|_| "That command is too long.".to_string())?;
    let (cleaned, subfolders) = parse::split_subfolders(&parse::phrase::normalize(text));
    let Some(unquoted) = parse::phrase::unquoted_text(&cleaned) else { return Ok(None) };
    for matched in REFERENCES.find_iter(&unquoted) {
        let replacement = format!("{}\"{MARKER}\"{}", &cleaned[..matched.start()], &cleaned[matched.end()..]);
        let Ok(mut intent) = parse::parse(&replacement) else { continue };
        if !matches!(intent.source(), Some(Source::Item { name, folder: None }) if name == MARKER) {
            continue;
        }
        let only_subfolders = subfolders == parse::Subfolders::Only;
        let boundary = matches!(subfolders, parse::Subfolders::Only | parse::Subfolders::NeedsFolder)
            .then(|| subfolder_boundary(snapshot.folder.as_deref()))
            .transpose()?;
        let reference = matched.as_str().to_ascii_lowercase();
        let mut words = reference.split_whitespace();
        let first = words.next().unwrap_or("");
        let word = if matches!(first, "file" | "item" | "number") { words.next().unwrap_or(first) } else { first };
        let position = match word {
            "first" => Some(0),
            "second" => Some(1),
            "third" => Some(2),
            "fourth" => Some(3),
            "fifth" => Some(4),
            "sixth" => Some(5),
            "seventh" => Some(6),
            "eighth" => Some(7),
            "ninth" => Some(8),
            "tenth" => Some(9),
            "last" => snapshot.entries.iter().take(200).rposition(|e| !e.removed),
            "it" | "file" => {
                if snapshot.entries.iter().filter(|e| !e.removed).count() != 1 {
                    return Err("Name a file or its position in the displayed list, such as “the first file”.".into());
                }
                snapshot.entries.iter().position(|e| !e.removed)
            }
            n if n.chars().next().is_some_and(|c| c.is_ascii_digit()) => {
                let number: String = n.chars().take_while(char::is_ascii_digit).collect();
                number.parse::<usize>().ok().and_then(|n| n.checked_sub(1))
            }
            _ => None,
        };
        let is_position = position.is_some() || word == "last" || word.chars().next().is_some_and(|c| c.is_ascii_digit());
        let source = if is_position {
            let entry = position
                .filter(|p| *p < 200)
                .and_then(|p| snapshot.entries.get(p))
                .filter(|e| !e.removed)
                .ok_or("That position isn't in the displayed results. Name a listed file or run another list command.")?;
            if only_subfolders && boundary.is_some_and(|folder| !is_nested_in(&entry.path, folder)) {
                return Err("That file isn't in a subfolder of the listed folder.".into());
            }
            Source::item(&entry.path.to_string_lossy())
        } else {
            Source::Set {
                folder: None,
                filter: Filter {
                    file_type: Some(FileType::Files),
                    include_subfolders: subfolders != parse::Subfolders::None,
                    only_subfolders,
                    ..Default::default()
                },
            }
        };
        intent.replace_source(source);
        return Ok(Some(intent));
    }
    Ok(None)
}

/// Why `prepare` produced no intent.
#[derive(Debug, PartialEq, Eq)]
pub enum PrepareError {
    /// The parser didn't recognize the command; holds the trimmed text.
    NotUnderstood(String),
    /// A specific problem to show the user.
    Problem(String),
}

impl std::fmt::Display for PrepareError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotUnderstood(text) => write!(f, "Voxi doesn't understand “{text}”."),
            Self::Problem(message) => f.write_str(message),
        }
    }
}

pub fn prepare(text: &str, snapshot: Option<&Snapshot>) -> Result<(Intent, bool), PrepareError> {
    crate::parse::check_length(text).map_err(|_| PrepareError::Problem("That command is too long.".to_string()))?;
    if let Ok(intent @ Intent::Chain { .. }) = parse::parse(text) {
        if snapshot.is_some() {
            return Err(PrepareError::Problem("Clear the previous result list before chaining commands.".into()));
        }
        return Ok((intent, false));
    }
    if let Some(snapshot) = snapshot
        && let Some(intent) = parse_followup(text, snapshot).map_err(PrepareError::Problem)?
    {
        return Ok((intent, true));
    }
    // Same split as `parse::parse`, kept as a type instead of an echoed string.
    let intent = parse::parse_classified(text).map_err(|failure| match failure {
        parse::ParseFailure::HardRefusal(parse::RefusalReason::TooLong) => PrepareError::Problem("That command is too long.".to_string()),
        _ => PrepareError::NotUnderstood(text.trim().to_string()),
    })?;
    let scoped = snapshot.is_some_and(|snapshot| match intent.source() {
        Some(Source::Set { folder: None, .. }) => true,
        Some(Source::Item { name, folder: None }) if !name.contains('/') && !name.starts_with('~') => {
            crate::plan::scope_matches(name, snapshot.paths())
        }
        _ => false,
    });
    Ok((intent, scoped))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepare_separates_unknown_commands_from_problems() {
        assert_eq!(prepare("  wibble the frobnicator  ", None).unwrap_err(), PrepareError::NotUnderstood("wibble the frobnicator".into()));
        let long = "a".repeat(20_000);
        assert_eq!(prepare(&long, None).unwrap_err(), PrepareError::Problem("That command is too long.".into()));
        assert!(prepare("list files in Downloads", None).is_ok());
    }
}
