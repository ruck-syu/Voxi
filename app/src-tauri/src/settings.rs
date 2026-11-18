//! User settings, stored as JSON in the app's config folder.

use serde::{Deserialize, Serialize};
// Keep diagnostics with the shared store module so source-including benchmarks
// use the same sink without requiring another crate-root module.
#[path = "diagnostics.rs"]
pub(crate) mod diagnostics;
use diagnostics::{Event, Code, Stage, Store};
use std::fs;
use std::path::{Path, PathBuf};
/// Version shared by the three independent JSON stores.
pub(crate) const STORE_VERSION: u64 = 1;

/// A recoverable load failure. A blocked store must not be saved until the
/// original file has been repaired or explicitly moved aside by the user.
#[derive(Debug, Clone)]
pub struct StorageWarning {
    pub message: String,
    pub backup: Option<PathBuf>,
    pub write_blocked: bool,
}

fn parent_dir(path: &Path) -> &Path {
    path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or_else(|| Path::new("."))
}

fn exclusive_file(path: &Path) -> std::io::Result<fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(path)
}

fn unique_file(path: &Path, tag: &str) -> std::io::Result<(PathBuf, fs::File)> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    loop {
        let mut name = path.file_name().ok_or_else(|| std::io::Error::other("Missing store filename"))?.to_os_string();
        name.push(format!(".{tag}-{}-{}-{}", chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default(), std::process::id(), SEQUENCE.fetch_add(1, Ordering::Relaxed)));
        let candidate = parent_dir(path).join(name);
        match exclusive_file(&candidate) {
            Ok(file) => return Ok((candidate, file)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
}

pub(crate) fn preserve_store(path: &Path, bytes: &[u8]) -> Result<PathBuf, String> {
    use std::io::Write;
    let (backup, mut file) = unique_file(path, "corrupt").map_err(|e| e.to_string())?;
    let result = write_checkpoint(5).and_then(|_| file.write_all(bytes)).and_then(|_| file.sync_all())
        .and_then(|_| fs::File::open(parent_dir(path))?.sync_all());
    if let Err(error) = result {
        // A partial backup must never be mistaken for a successful preservation.
        let _ = fs::remove_file(&backup);
        return Err(error.to_string());
    }
    Ok(backup)
}

pub(crate) fn decode_store<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, String> {
    let mut value: serde_json::Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    if value.get("version").is_some() || value.get("data").is_some() {
        let version = value.get("version").and_then(|v| v.as_u64()).ok_or("Invalid store version")?;
        if version != STORE_VERSION {
            return Err(format!("Unsupported store version {version}; expected {STORE_VERSION}"));
        }
        value = value.get_mut("data").ok_or("Missing store data")?.take();
    }
    serde_json::from_value(value).map_err(|e| e.to_string())
}

pub(crate) fn load_store<T: serde::de::DeserializeOwned + Default>(path: &Path, store: Store) -> (T, Option<StorageWarning>) {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (T::default(), None),
        Err(e) => { diagnostics::emit(Event::new(Code::StoreLoad, Stage::Failed).store(store).io(&e)); return (T::default(), Some(StorageWarning { message: format!("Couldn't read store: {e}. Original kept."), backup: None, write_blocked: true })); },
    };
    match decode_store(&bytes) {
        Ok(data) => (data, None),
        Err(error) => {
            diagnostics::emit(Event::new(Code::StoreLoad, Stage::Failed).store(store));
            let backup = preserve_store(path, &bytes);
            diagnostics::emit(Event::new(Code::StoreBackup, if backup.is_ok() { Stage::Preserved } else { Stage::BackupFailed }).store(store));
            let blocked = backup.is_err() || serde_json::from_slice::<serde_json::Value>(&bytes).ok()
                .is_some_and(|v| v.get("version").is_some() && v.get("version").and_then(|n| n.as_u64()) != Some(STORE_VERSION));
            let message = match &backup {
                Ok(p) => format!("Store is unreadable ({error}); preserved at {}.", p.display()),
                Err(e) => format!("Store is unreadable ({error}); couldn't preserve it ({e}). Original kept."),
            };
            (T::default(), Some(StorageWarning { message, backup: backup.ok(), write_blocked: blocked }))
        }
    }
}

#[cfg(test)]
thread_local! { pub(crate) static WRITE_FAILURE: std::cell::Cell<u8> = const { std::cell::Cell::new(0) }; }

fn write_checkpoint(stage: u8) -> std::io::Result<()> {
    #[cfg(test)]
    if WRITE_FAILURE.with(|f| f.get() == stage) { return Err(std::io::Error::other("Injected storage failure")); }
    let _ = stage;
    Ok(())
}

/// `sync_all` doesn't flush a macOS drive's own write cache; `F_FULLFSYNC`
/// does. Issue it on the temp file before the rename makes it visible.
#[cfg(target_os = "macos")]
fn fullfsync(file: &fs::File) -> std::io::Result<()> {
    use std::os::unix::io::AsRawFd;
    if unsafe { libc::fcntl(file.as_raw_fd(), libc::F_FULLFSYNC) } == -1 {
        let err = std::io::Error::last_os_error();
        // Some filesystems don't implement it. `sync_all` already ran, so
        // those two are harmless; every other error must reach the caller.
        if !matches!(err.raw_os_error(), Some(libc::ENOTSUP | libc::EINVAL)) {
            return Err(err);
        }
    }
    Ok(())
}

/// On a directory-sync failure the rename has already happened: report the
/// failure, but never attempt a second rename that could destroy newer data.
pub(crate) fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let dir = parent_dir(path);
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let (tmp, mut file) = unique_file(path, "tmp").map_err(|e| e.to_string())?;
    let result = (|| -> std::io::Result<()> {
        write_checkpoint(1)?;
        file.write_all(bytes)?;
        write_checkpoint(2)?;
        file.sync_all()?;
        #[cfg(target_os = "macos")]
        fullfsync(&file)?;
        write_checkpoint(3)?;
        fs::rename(&tmp, path)?;
        write_checkpoint(4)?;
        fs::File::open(dir)?.sync_all()
    })();
    if result.is_err() { let _ = fs::remove_file(tmp); }
    result.map_err(|e| format!("Couldn't durably save store: {e}"))
}

/// Inspect the version without deserializing the complete store. Corrupt JSON
/// must be durably preserved before replacement, even if no load was attempted.
fn save_blocked_by_version(path: &Path, store: Store) -> Option<String> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => return Some(format!("Couldn't read store: {e}. Original kept.")),
    };
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        let result = preserve_store(path, &bytes);
        diagnostics::emit(Event::new(Code::StoreBackup, if result.is_ok() { Stage::Preserved } else { Stage::BackupFailed }).store(store));
        return result.err();
    };
    let version = value.get("version")?;
    match version.as_u64() {
        Some(v) if v == STORE_VERSION => None,
        Some(v) => Some(format!("Unsupported store version {v}; expected {STORE_VERSION}")),
        None => Some("Invalid store version".into()),
    }
}

pub(crate) fn save_store<T: Serialize>(path: &Path, data: &T, store: Store) -> Result<(), String> {
    let result = save_store_inner(path, data, store);
    if result.is_err() { diagnostics::emit(Event::new(Code::StoreSave, Stage::Failed).store(store)); }
    result
}

fn save_store_inner<T: Serialize>(path: &Path, data: &T, store: Store) -> Result<(), String> {
    // Recheck the on-disk schema even when the caller loaded defaults or is
    // saving an object received from the frontend rather than a loaded object.
    // A cheap peek replaces the full load here: history saves on every command.
    if let Some(reason) = save_blocked_by_version(path, store) { return Err(reason); }
    #[derive(Serialize)]
    struct Envelope<'a, T> { version: u64, data: &'a T }
    let bytes = serde_json::to_vec_pretty(&Envelope { version: STORE_VERSION, data }).map_err(|e| e.to_string())?;
    atomic_write(path, &bytes)
}

/// Roll back both sides on save failure, including failures after atomic rename.
pub(crate) fn login_transaction(settings: &Settings, path: &Path, os_before: bool,
    mut set_os: impl FnMut(bool) -> Result<(), String>) -> Result<(), String> {
    let original = match fs::read(path) {
        Ok(bytes) => Some(bytes),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(format!("Cannot read settings before changing login registration: {e}")),
    };
    if settings.start_at_login != os_before
        && let Err(e) = set_os(settings.start_at_login) {
            let rollback = set_os(os_before);
            return Err(format!("{e}; login rollback: {rollback:?}"));
        }
    if let Err(e) = settings.save(path) {
        let os_rollback = set_os(os_before);
        let disk_rollback = match original {
            Some(bytes) => atomic_write(path, &bytes),
            None => match fs::remove_file(path) {
                Ok(()) => fs::File::open(parent_dir(path)).and_then(|f| f.sync_all()).map_err(|e| e.to_string()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(e.to_string()),
            },
        };
        return Err(format!("{e}; login rollback: {os_rollback:?}; preference rollback: {disk_rollback:?}. Review Login Items in System Settings."));
    }
    Ok(())
}

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
    /// The user's own names for folders ("dev" for ~/Documents/Dev), learned
    /// from a folder they picked or added in Settings.
    pub nicknames: Vec<Nickname>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Nickname {
    pub name: String,
    pub path: String,
}

/// Names Voxi already gives the standard folders; a nickname can't take them.
const RESERVED_NAMES: &[&str] = &["desktop", "downloads", "download", "documents", "document", "docs", "doc", "pictures", "picture", "photos", "photo", "screenshots", "screenshot", "home", "trash"];

/// The part of a spoken folder name that identifies it: "the Dev folder" and
/// "my dev directory" both become "dev".
pub fn nickname_key(text: &str) -> String {
    let lower = text.trim().trim_matches(|c: char| "\"'“”‘’.,!?".contains(c)).to_lowercase();
    let mut words: Vec<&str> = lower.split_whitespace().collect();
    while words.first().is_some_and(|w| ["the", "my", "our"].contains(w)) {
        words.remove(0);
    }
    while words.len() > 1 && words.last().is_some_and(|w| ["folder", "directory", "dir"].contains(w)) {
        words.pop();
    }
    words.join(" ")
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Shortcut {
    pub phrase: String,
    pub commands: Vec<String>,
}

/// Bound raw expansion before allocating normalized commands or building a preview.
pub(crate) fn check_shortcut_commands(commands: &[String]) -> Result<(), String> {
    if commands.len() > 8 || commands.iter().try_fold(0usize, |n, c| n.checked_add(c.len()))
        .is_none_or(|n| n > crate::parse::MAX_COMMAND_BYTES) {
        return Err("That command is too long.".into());
    }
    for command in commands { crate::parse::check_length(command).map_err(|_| "That command is too long.".to_string())?; }
    Ok(())
}

/// Words a person might put around a shortcut's name when saying it.
const SHORTCUT_VERBS: &[&str] = &["run", "do", "start", "play", "execute", "launch"];
const SHORTCUT_NOUNS: &[&str] = &["shortcut", "routine", "commands"];

/// Compares shortcut phrases loosely, so "Run my usuals.", "my usuals" and
/// "do my usuals" all reach a shortcut saved as "run my usuals".
/// Read-only commands a shortcut may hold only as its sole command.
pub fn shortcut_read_only(intent: &crate::model::Intent) -> bool {
    use crate::model::Intent;
    matches!(intent, Intent::List { .. } | Intent::Count { .. } | Intent::Size { .. } | Intent::Duplicates { .. } | Intent::Reveal { .. })
}

pub fn shortcut_key(text: &str) -> String {
    if crate::parse::check_length(text).is_err() { return String::new(); }
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
        Settings { hotkey: DEFAULT_HOTKEY.into(), extra_folders: Vec::new(), start_at_login: false, onboarded: false, shortcuts: Vec::new(), nicknames: Vec::new() }
    }
}

impl Settings {
    #[cfg(test)]
    pub fn load(path: &Path) -> Self {
        let (data, warning) = Self::load_with_warning(path);
        if let Some(warning) = warning { eprintln!("{}", warning.message); }
        data
    }

    pub fn load_with_warning(path: &Path) -> (Self, Option<StorageWarning>) {
        load_store(path, Store::Settings)
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        save_store(path, self, Store::Settings)
    }

    pub fn find_shortcut(&self, text: &str) -> Option<&Shortcut> {
        if crate::parse::check_length(text).is_err() { return None; }
        let key = shortcut_key(text);
        (!key.is_empty()).then(|| self.shortcuts.iter().find(|s| shortcut_key(&s.phrase) == key)).flatten()
    }

    /// A shortcut whose phrase is a near miss for `text` ("run my visuals"
    /// for "run my usuals"): a couple of letters off at most, and only one
    /// shortcut that close. Used only when the text isn't a command.
    pub fn find_close_shortcut(&self, text: &str) -> Option<&Shortcut> {
        if crate::parse::check_length(text).is_err() { return None; }
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
            crate::parse::check_length(&s.phrase).map_err(|_| "That command is too long.".to_string())?;
            check_shortcut_commands(&s.commands)?;
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
                    Ok(Intent::RecentActions | Intent::Undo) => return Err(format!("In “{phrase}”, “{c}” can't be part of a shortcut.")),
                    // A lone read-only command runs as if typed. Mixed with
                    // others it can't join the single preview a shortcut makes.
                    Ok(ref intent) if shortcut_read_only(intent) && commands.len() > 1 => return Err(format!(
                        "In “{phrase}”, “{c}” only shows information. A shortcut can show information with one command on its own, or hold commands that change files.")),
                    Ok(_) => {}
                }
            }
            kept.push(Shortcut { phrase, commands });
        }
        self.shortcuts = kept;
        Ok(())
    }

    /// Trims nicknames, keeps one per name (the newest), and refuses names
    /// that would shadow a standard folder or paths that aren't folders.
    pub fn validate_nicknames(&mut self, home: &Path) -> Result<(), String> {
        let mut kept: Vec<Nickname> = Vec::new();
        for n in self.nicknames.iter().rev() {
            let name = nickname_key(&n.name);
            let path = n.path.trim().to_string();
            if name.is_empty() && path.is_empty() {
                continue;
            }
            if name.is_empty() {
                return Err("Give the folder a name to say, like “dev”.".into());
            }
            if name.contains('/') || name.chars().count() > 40 {
                return Err(format!("“{name}” can't be a folder name. Use a short word or two."));
            }
            if RESERVED_NAMES.contains(&name.as_str()) {
                return Err(format!("“{name}” already means a standard folder."));
            }
            let full = crate::fsx::expand_tilde(&path, home);
            if !full.is_absolute() || crate::fsx::has_parent_ref(&full) || !full.is_dir() {
                return Err(format!("“{path}” isn't a folder Voxi can find."));
            }
            if kept.iter().any(|k| k.name == name) {
                continue;
            }
            kept.push(Nickname { name, path });
        }
        kept.reverse();
        self.nicknames = kept;
        Ok(())
    }

    /// `(name, folder)` for each nickname whose folder still exists.
    pub fn nickname_paths(&self, home: &Path) -> Vec<(String, PathBuf)> {
        self.nicknames
            .iter()
            .map(|n| (nickname_key(&n.name), crate::fsx::expand_tilde(n.path.trim(), home)))
            .filter(|(name, p)| !name.is_empty() && p.is_absolute() && p.is_dir() && !crate::fsx::has_parent_ref(p))
            .collect()
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

    #[test]
    fn storage_legacy_migration_and_private_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let legacy = br#"{"hotkey":"Alt+Space","extraFolders":["/tmp"],"onboarded":true}"#;
        fs::write(&path, legacy).unwrap();
        let (settings, warning) = Settings::load_with_warning(&path);
        assert!(warning.is_none());
        assert_eq!(fs::read(&path).unwrap(), legacy);
        settings.save(&path).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(json["version"], 1);
        assert_eq!(Settings::load(&path).hotkey, "Alt+Space");
        assert_eq!(Settings::load(&path).extra_folders, vec!["/tmp"]);
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    }

    #[test]
    fn storage_corruption_backup_future_rejection_and_read_errors() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        assert!(Settings::load_with_warning(&path).1.is_none());
        assert!(!path.exists());
        for bytes in [b"{\"hotkey\":".as_slice(), b"invalid JSON", br#"{"version":99,"data":{"hotkey":"future"}}"#] {
            fs::write(&path, bytes).unwrap();
            let (settings, warning) = Settings::load_with_warning(&path);
            let warning = warning.unwrap();
            assert_eq!(fs::read(warning.backup.unwrap()).unwrap(), bytes);
            assert_eq!(fs::read(&path).unwrap(), bytes);
            if bytes.starts_with(b"{\"version") {
                assert!(settings.save(&path).is_err());
                assert_eq!(fs::read(&path).unwrap(), bytes);
            } else {
                settings.save(&path).unwrap();
                assert!(Settings::load_with_warning(&path).1.is_none());
            }
        }
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();
        assert!(Settings::load_with_warning(&path).1.unwrap().write_blocked);
        assert!(Settings::default().save(&path).is_err());
        assert!(path.is_dir());
    }

    #[test]
    fn save_preserves_corrupt_bytes_before_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        // A future version still blocks the write, and save leaves no backup.
        let future = br#"{"version":99,"data":{"hotkey":"future"}}"#;
        fs::write(&path, future).unwrap();
        let err = Settings::default().save(&path).unwrap_err();
        assert!(err.contains("Unsupported store version 99"), "{err}");
        assert_eq!(fs::read(&path).unwrap(), future);
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
        // A non-numeric version is rejected as well.
        fs::write(&path, br#"{"version":"soon","data":{}}"#).unwrap();
        assert!(Settings::default().save(&path).is_err());
        // Saving must preserve corrupt bytes even without a preceding load.
        fs::write(&path, b"invalid JSON").unwrap();
        Settings::default().save(&path).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(json["version"], STORE_VERSION);
        let backup = fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().path())
            .find(|p| p.to_string_lossy().contains(".corrupt-")).unwrap();
        assert_eq!(fs::read(backup).unwrap(), b"invalid JSON");
    }

    #[test]
    fn failed_backup_is_a_load_warning_and_does_not_block_save() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        fs::write(&path, b"truncated").unwrap();
        WRITE_FAILURE.with(|f| f.set(5));
        let (settings, warning) = Settings::load_with_warning(&path);
        WRITE_FAILURE.with(|f| f.set(0));
        let warning = warning.unwrap();
        assert!(warning.write_blocked);
        assert!(warning.backup.is_none());
        assert_eq!(fs::read(&path).unwrap(), b"truncated");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
        // A later save retries preservation before replacing corrupt bytes.
        settings.save(&path).unwrap();
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
        assert!(Settings::load_with_warning(&path).1.is_none());
    }

    #[test]
    fn storage_write_sync_rename_failures_preserve_original() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let settings = Settings::default();
        settings.save(&path).unwrap();
        let original = fs::read(&path).unwrap();
        let changed = Settings { hotkey: "new".into(), ..settings };
        for stage in 1..=3 {
            WRITE_FAILURE.with(|f| f.set(stage));
            let result = changed.save(&path);
            WRITE_FAILURE.with(|f| f.set(0));
            assert!(result.is_err());
            assert_eq!(fs::read(&path).unwrap(), original);
            assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
        }
        WRITE_FAILURE.with(|f| f.set(4));
        let result = changed.save(&path);
        WRITE_FAILURE.with(|f| f.set(0));
        assert!(result.is_err(), "directory sync failure must reach caller even after rename");
        assert_eq!(Settings::load(&path).hotkey, "new");
        // Actual rename failure: a target directory cannot be replaced by a file.
        let destination = dir.path().join("directory");
        fs::create_dir(&destination).unwrap();
        assert!(atomic_write(&destination, b"new").is_err());
        assert!(destination.is_dir());
    }

    #[test]
    fn removed_dictation_setting_does_not_break_existing_settings() {
        let old: Settings = serde_json::from_str(r#"{"hotkey":"Control+Alt+Space","onboarded":true,"startAtLogin":true,"extraFolders":["/tmp"],"appleDictationEnabled":true,"shortcuts":[{"phrase":"tidy","commands":["sort Downloads by type"]}]}"#).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        old.save(&path).unwrap();
        let restored = Settings::load(&path);
        assert!(restored.onboarded && restored.start_at_login);
        assert_eq!(restored.hotkey, "Control+Alt+Space");
        assert_eq!(restored.extra_folders, vec!["/tmp"]);
        assert_eq!(restored.shortcuts[0].phrase, "tidy");
        assert!(!fs::read_to_string(path).unwrap().contains("appleDictationEnabled"));
    }

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
    fn nickname_keys_ignore_articles_and_the_word_folder() {
        for said in ["dev", "the dev folder", "My Dev directory", "“Dev”", "the dev"] {
            assert_eq!(nickname_key(said), "dev", "{said}");
        }
        assert_eq!(nickname_key("client work folder"), "client work");
        assert_eq!(nickname_key("folder"), "folder");
    }

    #[test]
    fn nicknames_are_validated_and_deduplicated() {
        let dir = tempfile::tempdir().unwrap();
        let dev = dir.path().join("Dev");
        fs::create_dir(&dev).unwrap();
        let nick = |name: &str, path: &Path| Nickname { name: name.into(), path: path.to_string_lossy().into() };
        let mut s = Settings { nicknames: vec![nick("dev", dir.path()), nick("The Dev folder", &dev)], ..Default::default() };
        s.validate_nicknames(dir.path()).unwrap();
        assert_eq!(s.nicknames, vec![nick("dev", &dev)], "newest wins, name normalized");
        let mut reserved = Settings { nicknames: vec![nick("downloads", &dev)], ..Default::default() };
        assert!(reserved.validate_nicknames(dir.path()).is_err());
        let mut missing = Settings { nicknames: vec![nick("gone", &dir.path().join("nope"))], ..Default::default() };
        assert!(missing.validate_nicknames(dir.path()).is_err());
        assert_eq!(s.nickname_paths(dir.path()), vec![("dev".to_string(), dev)]);
    }

    #[test]
    fn invalid_shortcut_commands_are_rejected_on_save() {
        assert!(with("tidy", &["make me a sandwich"]).validate_shortcuts().is_err());
        // One read-only command on its own is a saved search; blank lines don't count.
        with("White", &["list downloads", "  "]).validate_shortcuts().unwrap();
        with("White", &["how many PDFs in Downloads"]).validate_shortcuts().unwrap();
        let mixed = with("tidy", &["list files in Downloads", "sort Downloads by type"]).validate_shortcuts().unwrap_err();
        assert!(mixed.contains("only shows information"), "{mixed}");
        assert!(with("tidy", &["list downloads", "list desktop"]).validate_shortcuts().is_err());
        assert!(with("tidy", &["undo"]).validate_shortcuts().is_err());
        assert!(with("", &["sort Downloads by type"]).validate_shortcuts().is_err());
        let mut ok = with(" tidy ", &["sort Downloads by type", "  "]);
        ok.validate_shortcuts().unwrap();
        assert_eq!(ok.shortcuts[0].phrase, "tidy");
        assert_eq!(ok.shortcuts[0].commands, vec!["sort Downloads by type"]);
    }
}

#[cfg(test)]
mod login_tests {
    use super::*;
    #[test]
    fn save_failure_restores_login_registration_and_preference() {
        for stage in 1..=4 {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("settings.json");
            Settings::default().save(&path).unwrap();
            let original = fs::read(&path).unwrap();
            let mut os_enabled = false;
            WRITE_FAILURE.with(|f| f.set(stage));
            let result = login_transaction(&Settings { start_at_login: true, ..Settings::default() }, &path, false, |enabled| { os_enabled = enabled; Ok(()) });
            WRITE_FAILURE.with(|f| f.set(0));
            assert!(result.is_err());
            assert!(!os_enabled);
            assert_eq!(fs::read(path).unwrap(), original);
        }
    }
}

#[cfg(test)]
mod raw_budget_tests {
    use super::*;
    #[test]
    fn raw_shortcuts_are_bounded_before_trimming_or_normalizing() {
        let long = " ".repeat(crate::parse::MAX_COMMAND_BYTES + 1);
        for shortcut in [
            Shortcut { phrase: long.clone(), commands: vec!["delete a.pdf".into()] },
            Shortcut { phrase: "run usuals".into(), commands: vec![long] },
            Shortcut { phrase: "run usuals".into(), commands: vec![" ".repeat(3000); 8] },
            Shortcut { phrase: "run usuals".into(), commands: vec!["".into(); 9] },
        ] {
            let mut settings = Settings { shortcuts: vec![shortcut], ..Settings::default() };
            assert_eq!(settings.validate_shortcuts().unwrap_err(), "That command is too long.");
        }
        let settings = Settings::default();
        let huge = "é".repeat(crate::parse::MAX_COMMAND_BYTES);
        assert!(shortcut_key(&huge).is_empty());
        assert!(settings.find_shortcut(&huge).is_none());
        assert!(settings.find_close_shortcut(&huge).is_none());
    }

    #[test]
    fn save_refuses_to_replace_corruption_when_backup_fails() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        fs::write(&path, b"corrupt").unwrap();
        WRITE_FAILURE.with(|f| f.set(5));
        let result = Settings::default().save(&path);
        WRITE_FAILURE.with(|f| f.set(0));
        assert!(result.is_err());
        assert_eq!(fs::read(path).unwrap(), b"corrupt");
    }
}
