//! Write-ahead execution evidence. An interrupted step is never replayed:
//! tools can fail after publishing output, and a missing acknowledgement does
//! not prove that the filesystem was unchanged. Keep the intent and any known
//! inverses for manual reconciliation rather than guessing or deleting files.
use crate::{exec, fsx::Roots, history::{Entry, History}, model::Action, settings};
use serde::{Deserialize, Serialize};
use crate::settings::diagnostics::{self, Event, Code, Stage, Store};
use std::path::Path;

#[derive(Default, Serialize, Deserialize)]
struct Record {
    #[serde(default)]
    unknown: bool,
    entry: Option<Entry>,
    actions: Vec<Action>,
    /// Persisted before entering a tool; cleared when the tool returns.
    in_flight: Option<usize>,
    finished: bool,
    #[serde(default)]
    error: Option<String>,
}

fn sync(path: &Path, record: &Record) -> Result<(), String> {
    checkpoint("before_sync");
    settings::save_store(path, record, Store::Journal)?;
    checkpoint("after_sync");
    Ok(())
}

fn read(path: &Path) -> Result<Record, String> {
    // Serialize corrupt-file conversion so concurrent status polls preserve it once.
    static READ: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _read = READ.lock().map_err(|_| "Journal state unavailable")?;
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Record::default()),
        Err(e) => return Err(e.to_string()),
    };
    match settings::decode_store(&bytes) {
        Ok(record) => Ok(record),
        Err(_) => {
            // Reuse evidence if a previous attempt preserved it but failed to
            // publish the marker (including a crash between those two writes).
            let parent = path.parent().ok_or("Journal directory unavailable")?;
            let prefix = format!("{}.corrupt-", path.file_name().ok_or("Missing journal filename")?.to_string_lossy());
            let mut preserved = false;
            for candidate in std::fs::read_dir(parent).map_err(|e| e.to_string())? {
                let candidate = candidate.map_err(|e| e.to_string())?.path();
                if candidate.file_name().is_some_and(|name| name.to_string_lossy().starts_with(&prefix))
                    && std::fs::read(&candidate).is_ok_and(|saved| saved == bytes) {
                    std::fs::File::open(&candidate).and_then(|file| file.sync_all()).map_err(|e| e.to_string())?;
                    std::fs::File::open(parent).and_then(|file| file.sync_all()).map_err(|e| e.to_string())?;
                    preserved = true;
                    break;
                }
            }
            if !preserved { settings::preserve_store(path, &bytes)?; }
            let record = Record { unknown: true, ..Record::default() };
            // Already preserved above: bypass save_store's second corruption backup.
            let marker = serde_json::json!({ "version": settings::STORE_VERSION, "data": &record });
            settings::atomic_write(path, &serde_json::to_vec(&marker).map_err(|e| e.to_string())?)?;
            Ok(record)
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RecoveryStatus {
    command: String,
    acknowledged: usize,
    in_flight: Option<Action>,
}

fn interrupted(record: &Record) -> bool {
    record.unknown || (record.entry.is_some() && (!record.finished || record.in_flight.is_some()))
}

pub(crate) fn status(path: &Path) -> Result<Option<RecoveryStatus>, String> {
    let record = read(path)?;
    Ok(if interrupted(&record) {
        Some(record.entry.map_or_else(|| RecoveryStatus {
            command: "Unknown interrupted operation (corrupt journal preserved in backup)".into(),
            acknowledged: 0, in_flight: None,
        }, |entry| RecoveryStatus {
            command: entry.command, acknowledged: entry.done.len(),
            in_flight: record.in_flight.and_then(|i| record.actions.get(i).cloned()),
        }))
    } else { None })
}

fn import(record: &Record, history: &mut History) -> Result<(), String> {
    if let Some(entry) = &record.entry {
        if !entry.done.is_empty() && !history.entries.iter().any(|e| e.id == entry.id) {
            history.try_push(entry.clone())?;
        }
        // Retry persistence even if an earlier try_push only updated memory.
        if !entry.done.is_empty() { history.save()?; }
    }
    Ok(())
}

/// Import known steps without replaying anything. Only the user can resolve
/// an interruption, after inspecting the paths shown in Settings → Scheduled.
pub(crate) fn recover(path: &Path, history: &mut History) -> Result<(), String> {
    let record = read(path)?;
    import(&record, history)?;
    if record.entry.is_some() && !interrupted(&record) { diagnostics::emit(Event::new(Code::JournalRecovered, Stage::Recovered).operation(record.entry.as_ref().map_or(0, |e| e.id)).count(record.entry.as_ref().map_or(0, |e| e.done.len()))); }
    if interrupted(&record) {
        diagnostics::emit(Event::new(Code::JournalInterrupted, Stage::Interrupted).operation(record.entry.as_ref().map_or(0, |e| e.id)).count(record.entry.as_ref().map_or(0, |e| e.done.len())).step(record.in_flight));
        return Err("Interrupted operation. Open Settings → Scheduled, check the files in the recovery notice, then choose Mark as reviewed. No action was retried.".into());
    }
    Ok(())
}

/// Preserve the exact journal bytes before removing the active name. A hard
/// link reserves a unique archive name without ever overwriting old evidence.
pub(crate) fn acknowledge(path: &Path, history: &mut History) -> Result<(), String> {
    let record = read(path)?;
    if !interrupted(&record) { return Ok(()); }
    import(&record, history)?;
    let parent = path.parent().ok_or("Journal directory unavailable")?;
    let stamp = chrono::Utc::now().timestamp_micros();
    for suffix in 0..1000 {
        let archive = parent.join(format!("operation-journal.interrupted-{stamp}-{suffix}.json"));
        match std::fs::hard_link(path, &archive) {
            Ok(()) => {
                let directory = std::fs::File::open(parent).map_err(|e| e.to_string())?;
                directory.sync_all().map_err(|e| e.to_string())?;
                std::fs::remove_file(path).map_err(|e| e.to_string())?;
                let result = directory.sync_all().map_err(|e| e.to_string());
                if result.is_ok() { diagnostics::emit(Event::new(Code::JournalRecovered, Stage::Recovered).count(record.entry.as_ref().map_or(0, |e| e.done.len()))); }
                return result;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("Cannot archive recovery evidence: {e}. Try Mark as reviewed again.")),
        }
    }
    Err("Cannot reserve a recovery archive. Try Mark as reviewed again.".into())
}

pub(crate) fn run(path: &Path, actions: &[Action], roots: &Roots, mut entry: Entry, history: &mut History) -> Result<exec::Outcome, String> {
    // Finder navigation has no filesystem mutation or undo to persist.
    if actions.iter().all(|a| matches!(a, Action::OpenInFinder { .. })) {
        return Ok(exec::run(actions, roots));
    }
    history.ensure_writable()?;
    recover(path, history)?;
    let mut record = Record { entry: Some(entry.clone()), actions: actions.to_vec(), ..Record::default() };
    sync(path, &record)?;
    let mut error = None;
    for (index, action) in actions.iter().enumerate() {
        record.in_flight = Some(index);
        sync(path, &record)?;
        checkpoint("before_mutation");
        let result = exec::step(action, roots);
        checkpoint("after_mutation");
        match result {
            Ok(mut inverse) => {
                entry.done.push(action.clone());
                inverse.reverse();
                entry.inverse.splice(0..0, inverse);
                record.entry = Some(entry.clone());
                record.in_flight = None;
                sync(path, &record)?;
            }
            Err(e) => {
                // The call returned: its error is evidence, not an interruption.
                // Executor publication is staged/no-clobber; retained backups
                // and cleanup errors are described by the returned error.
                record.error = Some(e.clone());
                record.in_flight = None;
                error = Some(e);
                sync(path, &record)?;
                break;
            }
        }
    }
    // Keep the journal until history is durable; a failure must not advertise undo.
    if !entry.done.is_empty() { history.try_push(entry.clone())?; }
    record.finished = true;
    sync(path, &record)?;
    Ok(exec::Outcome { done: entry.done, inverse: entry.inverse, error })
}

// Only test binaries honor crash injection; production environment variables
// cannot crash the application.
fn checkpoint(_point: &str) {
    #[cfg(test)]
    {
        thread_local! { static HITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }
        if std::env::var("VOXI_JOURNAL_CRASH").ok().as_deref() == Some(_point) {
            let hit = HITS.with(|hits| { let hit = hits.get() + 1; hits.set(hit); hit });
            let nth = std::env::var("VOXI_JOURNAL_NTH").ok().and_then(|n| n.parse().ok()).unwrap_or(1);
            if hit == nth { std::process::exit(86); }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn entry() -> Entry {
        Entry { id: 1, time: chrono::Local::now(), command: "move files".into(), done: vec![], inverse: vec![], undone: false, undo_of: None }
    }

    #[test]
    fn corrupt_journal_preserved_once_and_review_unblocks_execution() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("operation-journal.json");
        std::fs::write(&path, b"{broken journal").unwrap();
        let roots = Roots::new(dir.path(), None, &[]);
        let mut history = History::load(dir.path().join("history.json"));
        settings::WRITE_FAILURE.with(|failure| failure.set(5));
        assert!(status(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"{broken journal");
        settings::WRITE_FAILURE.with(|failure| failure.set(1));
        assert!(status(&path).is_err());
        settings::WRITE_FAILURE.with(|failure| failure.set(0));
        for _ in 0..3 {
            assert!(status(&path).unwrap().is_some());
            assert!(run(&path, &[], &roots, entry(), &mut history).unwrap().error.is_none());
            assert!(recover(&path, &mut history).is_err());
            assert!(run(&path, &[Action::CreateFolder { path: dir.path().join("Documents/new") }], &roots, entry(), &mut history).is_err());
        }
        let backups: Vec<_> = std::fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().path())
            .filter(|p| p.to_string_lossy().contains(".corrupt-")).collect();
        assert_eq!(backups.len(), 1);
        assert_eq!(std::fs::read(&backups[0]).unwrap(), b"{broken journal");
        acknowledge(&path, &mut history).unwrap();
        assert!(!path.exists());
        assert!(status(&path).unwrap().is_none());
        std::fs::create_dir(dir.path().join("Documents")).unwrap();
        assert!(run(&path, &[Action::CreateFolder { path: dir.path().join("Documents/new") }], &roots, entry(), &mut history).unwrap().error.is_none());
        assert_eq!(std::fs::read(&backups[0]).unwrap(), b"{broken journal");
    }

    #[test]
    fn partial_undo_retries_only_remaining_steps_after_reload() {
        let dir = tempfile::tempdir().unwrap();
        let roots = Roots::new(dir.path(), None, &[]);
        std::fs::create_dir(dir.path().join("Documents")).unwrap();
        let a = dir.path().join("Documents/a");
        let b = dir.path().join("Documents/b");
        let c = dir.path().join("Documents/c");
        std::fs::write(&a, b"original").unwrap();
        let first = Action::Move { from: a.clone(), to: b.clone() };
        let second = Action::Move { from: dir.path().join("Documents/missing"), to: c.clone() };
        let history_path = dir.path().join("history.json");
        let mut history = History::load(history_path.clone());
        history.try_push(Entry { inverse: vec![first.clone(), second.clone()], ..entry() }).unwrap();
        let path = dir.path().join("operation-journal.json");
        let undo = Entry { id: 2, undo_of: Some(1), ..entry() };
        let out = run(&path, &[first, second.clone()], &roots, undo, &mut history).unwrap();
        assert_eq!(out.done.len(), 1); assert!(out.error.is_some());
        let mut history = History::load(history_path.clone());
        let remaining = history.last_undoable().unwrap();
        assert_eq!(remaining.id, 1); assert!(!remaining.undone);
        assert_eq!(remaining.inverse, vec![second.clone()]);
        std::fs::write(dir.path().join("Documents/missing"), b"remaining").unwrap();
        let undo = Entry { id: 3, undo_of: Some(1), ..entry() };
        assert!(run(&path, &[second], &roots, undo, &mut history).unwrap().error.is_none());
        let history = History::load(history_path);
        assert!(history.entries[0].undone);
        assert!(history.entries[0].inverse.is_empty());
        assert!(b.exists() && c.exists() && !a.exists());
    }

    #[test]
    fn returned_failures_finish_and_allow_the_next_operation() {
        for collision in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let roots = Roots::new(dir.path(), None, &[]);
            for folder in ["Downloads", "Documents"] { std::fs::create_dir(dir.path().join(folder)).unwrap(); }
            let source = dir.path().join("Downloads/a");
            let target = dir.path().join("Documents/a");
            if collision { std::fs::write(&source, "source").unwrap(); std::fs::write(&target, "existing").unwrap(); }
            let path = dir.path().join("operation-journal.json");
            let mut history = History::load(dir.path().join("history.json"));
            let out = run(&path, &[Action::Move { from: source, to: target.clone() }], &roots, entry(), &mut history).unwrap();
            assert!(out.error.is_some());
            let record = read(&path).unwrap();
            assert!(record.finished && record.in_flight.is_none());
            assert_eq!(record.error, out.error);
            assert!(status(&path).unwrap().is_none());
            recover(&path, &mut history).unwrap();
            run(&path, &[Action::CreateFolder { path: dir.path().join("Documents/next") }], &roots, entry(), &mut history).unwrap();
            if collision { assert_eq!(std::fs::read(target).unwrap(), b"existing"); }
        }
    }

    #[test]
    fn interruption_requires_review_and_archives_exact_evidence() {
        for in_flight in [None, Some(0)] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("operation-journal.json");
            let mut history = History::load(dir.path().join("history.json"));
            let action = Action::Move { from: dir.path().join("Downloads/a"), to: dir.path().join("Documents/a") };
            let record = Record { entry: Some(Entry { done: vec![action.clone()], ..entry() }), actions: vec![action.clone()], in_flight, ..Record::default() };
            sync(&path, &record).unwrap();
            let original = std::fs::read(&path).unwrap();
            let status = status(&path).unwrap().unwrap();
            assert_eq!(status.command, "move files"); assert_eq!(status.acknowledged, 1);
            assert_eq!(status.in_flight, in_flight.map(|_| action));
            assert!(recover(&path, &mut history).unwrap_err().contains("Mark as reviewed"));
            assert_eq!(std::fs::read(&path).unwrap(), original);
            acknowledge(&path, &mut history).unwrap();
            assert!(!path.exists());
            let archives: Vec<_> = std::fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().path())
                .filter(|p| p.file_name().unwrap().to_string_lossy().starts_with("operation-journal.interrupted-")).collect();
            assert_eq!(archives.len(), 1);
            assert_eq!(std::fs::read(&archives[0]).unwrap(), original);
            recover(&path, &mut history).unwrap();
            assert_eq!(history.entries.len(), 1);
            acknowledge(&path, &mut history).unwrap();
            assert!(archives[0].exists());
        }
    }

    #[test]
    fn review_keeps_journal_when_history_cannot_be_saved() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("journal.json");
        let action = Action::CreateFolder { path: dir.path().join("Documents/a") };
        sync(&path, &Record { entry: Some(Entry { done: vec![action], ..entry() }), ..Record::default() }).unwrap();
        let original = std::fs::read(&path).unwrap();
        std::fs::create_dir(dir.path().join("history.json")).unwrap();
        let mut history = History::load(dir.path().join("history.json"));
        assert!(acknowledge(&path, &mut history).is_err());
        assert_eq!(std::fs::read(path).unwrap(), original);
    }

    #[test]
    fn crash_child() {
        let Ok(dir) = std::env::var("VOXI_JOURNAL_CHILD") else { return };
        let home = std::path::PathBuf::from(dir);
        let roots = Roots::new(&home, None, &[]);
        let mut history = History::load(home.join("history.json"));
        run(&home.join("journal.json"), &[Action::Move { from: home.join("Downloads/a"), to: home.join("Documents/a") }, Action::Move { from: home.join("Downloads/b"), to: home.join("Documents/b") }], &roots,
            Entry { id: 1, time: chrono::Local::now(), command: "move".into(), done: vec![], inverse: vec![], undone: false, undo_of: None }, &mut history).unwrap();
    }

    #[test]
    fn crash_boundaries_never_replay_mutations() {
        for point in ["before_sync", "after_sync", "before_mutation", "after_mutation"] {
            let count = if point.ends_with("sync") { 6 } else { 2 };
            for nth in 1..=count {
                let dir = tempfile::tempdir().unwrap();
                for folder in ["Downloads", "Documents"] { std::fs::create_dir(dir.path().join(folder)).unwrap(); }
                for name in ["a", "b"] { std::fs::write(dir.path().join("Downloads").join(name), "original").unwrap(); }
                let status = std::process::Command::new(std::env::current_exe().unwrap())
                    .args(["--exact", "operation_journal::tests::crash_child"])
                    .env("VOXI_JOURNAL_CHILD", dir.path()).env("VOXI_JOURNAL_CRASH", point)
                    .env("VOXI_JOURNAL_NTH", nth.to_string()).status().unwrap();
                assert_eq!(status.code(), Some(86), "{point} {nth}");
                let mut history = History::load(dir.path().join("history.json"));
                for _ in 0..2 { let _ = recover(&dir.path().join("journal.json"), &mut history); }
                assert!(history.entries.len() <= 1);
                let moved = match point {
                    "before_mutation" => nth - 1,
                    "after_mutation" => nth,
                    _ => ((nth - 1) / 2).min(2),
                };
                for (index, name) in ["a", "b"].iter().enumerate() {
                    let from = dir.path().join("Downloads").join(name);
                    let to = dir.path().join("Documents").join(name);
                    assert_eq!(to.exists(), index < moved, "{point} {nth}");
                    assert_eq!(from.exists(), index >= moved, "{point} {nth}");
                    assert_eq!(std::fs::read(if to.exists() { to } else { from }).unwrap(), b"original");
                }
            }
        }
    }
}
