//! Write-ahead execution evidence. An interrupted step is never replayed:
//! tools can fail after publishing output, and a missing acknowledgement does
//! not prove that the filesystem was unchanged. Keep the intent and any known
//! inverses for manual reconciliation rather than guessing or deleting files.
use crate::settings::diagnostics::{self, Code, Event, Stage, Store};
use crate::{
    exec,
    fsx::Roots,
    history::{Entry, EntryState, History, Inverse, validate_inverses},
    model::{Action, RunOutcome},
    settings,
};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    actions: Vec<Action>,
    state: State,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
enum State {
    Prepared(Entry),
    StepInFlight { entry: Entry, index: usize },
    Acknowledged { progress: Progress, count: usize },
    HistorySaved(Progress),
    Finished(Progress),
    NeedsReview { entry: Option<Entry>, index: Option<usize>, reason: String },
}

#[derive(Serialize, Deserialize)]
struct Progress {
    entry: Entry,
    error: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyRecord {
    #[serde(default)]
    unknown: bool,
    entry: Option<Entry>,
    actions: Vec<Action>,
    in_flight: Option<usize>,
    finished: bool,
    #[serde(default)]
    error: Option<String>,
}

impl Record {
    fn entry(&self) -> Option<&Entry> {
        match &self.state {
            State::Prepared(entry) | State::StepInFlight { entry, .. } => Some(entry),
            State::Acknowledged { progress, .. } | State::HistorySaved(progress) | State::Finished(progress) => Some(&progress.entry),
            State::NeedsReview { entry, .. } => entry.as_ref(),
        }
    }

    fn in_flight(&self) -> Option<usize> {
        match &self.state {
            State::StepInFlight { index, .. } => Some(*index),
            State::NeedsReview { index, .. } => *index,
            _ => None,
        }
    }

    fn interrupted(&self) -> bool {
        !matches!(self.state, State::HistorySaved(_) | State::Finished(_))
    }

    fn valid(&self) -> bool {
        let done = self.entry().map_or(0, |entry| entry.done.len());
        done <= self.actions.len()
            && match &self.state {
                State::Prepared(entry) => done == 0 && matches!(entry.state, EntryState::Irreversible),
                State::StepInFlight { index, .. } => *index == done && *index < self.actions.len(),
                State::Acknowledged { count, .. } => *count == done,
                State::NeedsReview { entry: Some(_), index: Some(index), .. } => *index == done && *index < self.actions.len(),
                _ => true,
            }
    }
}

fn decode(bytes: &[u8]) -> Result<Option<Record>, String> {
    if let Ok(record) = settings::decode_store::<Record>(bytes) {
        return if record.valid() { Ok(Some(record)) } else { Err("Invalid journal state".into()) };
    }
    let LegacyRecord { unknown, entry, actions, in_flight, finished, error } = settings::decode_store(bytes)?;
    let state = match entry {
        Some(entry) if finished && !unknown && in_flight.is_none() => State::Finished(Progress { entry, error }),
        None if !unknown && actions.is_empty() && in_flight.is_none() && !finished && error.is_none() => return Ok(None),
        entry => State::NeedsReview {
            reason: if unknown || entry.is_none() { "Journal evidence is incomplete.".into() } else { error.unwrap_or_default() },
            entry,
            index: in_flight,
        },
    };
    let record = Record { actions, state };
    if record.valid() { Ok(Some(record)) } else { Err("Invalid legacy journal state".into()) }
}

fn sync(path: &Path, record: &Record) -> Result<(), String> {
    if !record.valid() {
        return Err("Cannot save an invalid journal state.".into());
    }
    checkpoint("before_sync");
    settings::save_store(path, record, Store::Journal)?.require_durable()?;
    checkpoint("after_sync");
    Ok(())
}

fn read(path: &Path) -> Result<Option<Record>, String> {
    // Serialize corrupt-file conversion so concurrent status polls preserve it once.
    static READ: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _read = READ.lock().map_err(|_| "Journal state unavailable")?;
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    if let Some(reason) = settings::unsupported_store_version(&bytes) {
        return Err(format!("Cannot read operation journal: {reason}. Original file kept; update Voxi before running commands."));
    }
    match decode(&bytes) {
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
                    && std::fs::read(&candidate).is_ok_and(|saved| saved == bytes)
                {
                    std::fs::File::open(&candidate).and_then(|file| file.sync_all()).map_err(|e| e.to_string())?;
                    std::fs::File::open(parent).and_then(|file| file.sync_all()).map_err(|e| e.to_string())?;
                    preserved = true;
                    break;
                }
            }
            if !preserved {
                settings::preserve_store(path, &bytes)?;
            }
            let record = Record {
                actions: vec![],
                state: State::NeedsReview { entry: None, index: None, reason: "Unreadable journal preserved in backup.".into() },
            };
            // Already preserved above: bypass save_store's second corruption backup.
            let marker = serde_json::json!({ "version": settings::STORE_VERSION, "data": &record });
            settings::atomic_write(path, &serde_json::to_vec(&marker).map_err(|e| e.to_string())?)?.require_durable()?;
            Ok(Some(record))
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RecoveryStatus {
    command: String,
    acknowledged: usize,
    in_flight: Option<Action>,
    reason: Option<String>,
}

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum RecoveryPoll {
    Clear,
    Busy,
    NeedsReview {
        #[serde(flatten)]
        details: RecoveryStatus,
    },
}

pub(crate) fn status(path: &Path) -> Result<Option<RecoveryStatus>, String> {
    Ok(read(path)?.filter(Record::interrupted).map(|record| RecoveryStatus {
        command: record.entry().map_or_else(|| "Unknown interrupted operation".into(), |entry| entry.command.clone()),
        acknowledged: record.entry().map_or(0, |entry| entry.done.len()),
        in_flight: record.in_flight().and_then(|index| record.actions.get(index).cloned()),
        reason: match record.state {
            State::NeedsReview { reason, .. } if !reason.is_empty() => Some(reason),
            _ => None,
        },
    }))
}

fn import(record: &Record, history: &mut History) -> Result<(), String> {
    if let Some(entry) = record.entry()
        && !entry.done.is_empty()
        && (record.interrupted() || !history.entries.iter().any(|existing| existing.id == entry.id))
    {
        history.try_push(entry.clone())?;
    }
    Ok(())
}

const REVIEW: &str =
    "Open Settings → Scheduled, check the files in the recovery notice, then choose Mark as reviewed. No action was retried.";

/// Import known steps without replaying anything. The user resolves uncertain effects.
pub(crate) fn recover(path: &Path, history: &mut History) -> Result<(), String> {
    let Some(record) = read(path)? else { return Ok(()) };
    import(&record, history)?;
    diagnostics::emit(
        Event::new(
            if record.interrupted() { Code::JournalInterrupted } else { Code::JournalRecovered },
            if record.interrupted() { Stage::Interrupted } else { Stage::Recovered },
        )
        .operation(record.entry().map_or(0, |e| e.id))
        .count(record.entry().map_or(0, |e| e.done.len()))
        .step(record.in_flight()),
    );
    if record.interrupted() {
        return Err(format!("Interrupted operation. {REVIEW}"));
    }
    Ok(())
}

/// Preserve the exact journal bytes before removing the active name. A hard
/// link reserves a unique archive name without ever overwriting old evidence.
pub(crate) fn acknowledge(path: &Path, history: &mut History) -> Result<(), String> {
    let Some(record) = read(path)?.filter(Record::interrupted) else { return Ok(()) };
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
                if result.is_ok() {
                    diagnostics::emit(
                        Event::new(Code::JournalRecovered, Stage::Recovered).count(record.entry().map_or(0, |e| e.done.len())),
                    );
                }
                return result;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("Cannot archive recovery evidence: {e}. Try Mark as reviewed again.")),
        }
    }
    Err("Cannot reserve a recovery archive. Try Mark as reviewed again.".into())
}

pub(crate) fn run(
    path: &Path,
    actions: &[Action],
    roots: &Roots,
    command: String,
    undo_of: Option<u64>,
    history: &mut History,
    is_cancelled: &dyn Fn() -> bool,
) -> Result<exec::Outcome, String> {
    // Finder navigation has no filesystem mutation or undo to persist.
    if actions.iter().all(|a| matches!(a, Action::OpenInFinder { .. })) {
        return Ok(exec::run_with_cancel(actions, roots, is_cancelled));
    }
    history.ensure_writable()?;
    recover(path, history)?;
    let mut entry = Entry::new(history.next_id(), command, undo_of);
    let mut out = exec::Outcome { done: vec![], inverse: vec![], terminal: RunOutcome::Completed, entry_id: None };
    let mut undo = match undo_of.map(|id| approved_inverses(history, id, actions)).transpose() {
        Ok(undo) => undo,
        Err(error) => {
            out.terminal = RunOutcome::Failed { error, partial: false };
            return Ok(out);
        }
    };
    let mut record = Record { actions: actions.to_vec(), state: State::Prepared(entry.clone()) };
    sync(path, &record)?;
    for (index, action) in actions.iter().enumerate() {
        if is_cancelled() {
            out.terminal = RunOutcome::Cancelled;
            break;
        }
        record.state = State::StepInFlight { entry: entry.clone(), index };
        sync(path, &record)?;
        checkpoint("before_mutation");
        let result = undo
            .as_ref()
            .map_or(Ok(()), |undo| undo[index].validate())
            .map_err(exec::StepError::Failed)
            .and_then(|()| exec::step(action, roots, is_cancelled));
        checkpoint("after_mutation");
        let effect = match result {
            Ok(effect) => {
                if let Some(undo) = &mut undo {
                    entry.reversed += 1;
                    if let (Some(placed), Some(evidence)) = (undo[index].placement(), effect.inverse.first()) {
                        entry.identity_updates.rebind(&placed, evidence, &mut undo[index + 1..]);
                    }
                    Some(exec::Effect { inverse: vec![], ..effect })
                } else {
                    Some(effect)
                }
            }
            Err(exec::StepError::Cancelled) => {
                out.terminal = RunOutcome::Cancelled;
                None
            }
            Err(exec::StepError::Failed(error)) => {
                out.terminal = RunOutcome::Failed { error, partial: !entry.done.is_empty() };
                None
            }
            Err(exec::StepError::Partial { message, effect }) => {
                out.terminal = RunOutcome::Failed { error: message, partial: true };
                Some(*effect)
            }
            Err(exec::StepError::Uncertain(error)) => {
                record.state = State::NeedsReview { entry: Some(entry.clone()), index: Some(index), reason: error.clone() };
                sync(path, &record)?;
                out.done = entry.done;
                out.terminal = RunOutcome::NeedsReview { error: format!("{error} {REVIEW}") };
                return Ok(out);
            }
        };
        if let Some(effect) = effect {
            entry.done.push(effect.action);
            out.inverse.splice(0..0, effect.inverse);
            entry.state = EntryState::with_inverses(out.inverse.clone());
        }
        record.state = State::Acknowledged {
            count: entry.done.len(),
            progress: Progress { entry: entry.clone(), error: out.terminal.error().map(str::to_owned) },
        };
        if let Err(error) = sync(path, &record) {
            // The step already happened. Keep its undo in History (memory
            // first, disk if it can be saved) rather than drop it with the
            // journal write; recovery skips an entry History already has.
            let kept = if entry.done.is_empty() {
                String::new()
            } else {
                match history.try_push(entry.clone()) {
                    Ok(()) => " The change was made and can be undone from History.".into(),
                    Err(_) => " The change was made; its undo is kept until Voxi can save History.".into(),
                }
            };
            return Err(format!("{error}.{kept} {REVIEW}"));
        }
        if !matches!(out.terminal, RunOutcome::Completed) {
            break;
        }
    }
    // Only advertise durable history after it and the journal checkpoint are synced.
    if !entry.done.is_empty() {
        history.try_push(entry.clone())?;
        out.entry_id = Some(entry.id);
    }
    record.state = State::HistorySaved(Progress { entry: entry.clone(), error: out.terminal.error().map(str::to_owned) });
    sync(path, &record)?;
    let State::HistorySaved(progress) = record.state else { unreachable!() };
    record.state = State::Finished(progress);
    // HistorySaved is already recoverable and has no unresolved filesystem step.
    if let Err(warning) = sync(path, &record) {
        history.notices.push(warning);
    }
    out.done = entry.done;
    Ok(out)
}

fn approved_inverses(history: &History, id: u64, actions: &[Action]) -> Result<Vec<Inverse>, String> {
    let inverses = history.undoable(id)?.inverses();
    if !inverses.iter().map(|inverse| &inverse.action).eq(actions) {
        return Err("This undo preview changed. Plan the undo again.".into());
    }
    validate_inverses(inverses)?;
    Ok(inverses.to_vec())
}

// Only test binaries honor crash injection; production environment variables
// cannot crash the application.
fn checkpoint(_point: &str) {
    #[cfg(test)]
    {
        thread_local! { static HITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }
        if std::env::var("VOXI_JOURNAL_CRASH").ok().as_deref() == Some(_point) {
            let hit = HITS.with(|hits| {
                let hit = hits.get() + 1;
                hits.set(hit);
                hit
            });
            let nth = std::env::var("VOXI_JOURNAL_NTH").ok().and_then(|n| n.parse().ok()).unwrap_or(1);
            if hit == nth {
                std::process::exit(86);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn run(path: &Path, actions: &[Action], roots: &Roots, entry: Entry, history: &mut History) -> Result<exec::Outcome, String> {
        super::run(path, actions, roots, entry.command, entry.undo_of, history, &|| false)
    }
    fn entry() -> Entry {
        Entry::new(1, "move files".into(), None)
    }

    fn undo_actions(entry: &Entry) -> Vec<Action> {
        entry.inverses().iter().map(|inverse| inverse.action.clone()).collect()
    }

    fn entry_with_inverses(actions: &[Action]) -> Entry {
        let inverses = actions
            .iter()
            .map(|action| {
                let Action::Move { from, .. } = action else { panic!("expected move") };
                crate::history::Inverse::capture(action.clone(), &[crate::fsx::FileIdentity::read(from).unwrap()]).unwrap()
            })
            .collect();
        Entry { state: EntryState::with_inverses(inverses), ..entry() }
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
            assert!(run(&path, &[], &roots, entry(), &mut history).unwrap().terminal.error().is_none());
            assert!(recover(&path, &mut history).is_err());
            assert!(run(&path, &[Action::CreateFolder { path: dir.path().join("Documents/new") }], &roots, entry(), &mut history).is_err());
        }
        let backups: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.to_string_lossy().contains(".corrupt-"))
            .collect();
        assert_eq!(backups.len(), 1);
        assert_eq!(std::fs::read(&backups[0]).unwrap(), b"{broken journal");
        acknowledge(&path, &mut history).unwrap();
        assert!(!path.exists());
        assert!(status(&path).unwrap().is_none());
        std::fs::create_dir(dir.path().join("Documents")).unwrap();
        assert!(
            run(&path, &[Action::CreateFolder { path: dir.path().join("Documents/new") }], &roots, entry(), &mut history)
                .unwrap()
                .terminal
                .error()
                .is_none()
        );
        assert_eq!(std::fs::read(&backups[0]).unwrap(), b"{broken journal");
    }

    #[test]
    fn legacy_entryless_in_flight_requires_review() {
        let dir = tempfile::tempdir().unwrap();
        let documents = dir.path().join("Documents");
        std::fs::create_dir(&documents).unwrap();
        let path = dir.path().join("operation-journal.json");
        let action = Action::CreateFolder { path: documents.join("interrupted") };
        let bytes = serde_json::to_vec(&serde_json::json!({
            "entry": null, "actions": [&action], "in_flight": 0, "finished": true
        }))
        .unwrap();
        std::fs::write(&path, &bytes).unwrap();
        let poll = status(&path).unwrap().unwrap();
        assert_eq!(poll.in_flight, Some(action.clone()));
        assert_eq!(poll.acknowledged, 0);
        let mut history = History::load(dir.path().join("history.json"));
        let roots = Roots::new(dir.path(), None, &[]);
        assert!(run(&path, &[action], &roots, entry(), &mut history).is_err());
        assert!(!documents.join("interrupted").exists());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        acknowledge(&path, &mut history).unwrap();
        assert!(status(&path).unwrap().is_none());
    }

    #[test]
    fn contradictory_persisted_index_requires_review() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("operation-journal.json");
        let action = Action::CreateFolder { path: dir.path().join("Documents/unused") };
        let record = Record { actions: vec![action], state: State::StepInFlight { entry: entry(), index: 1 } };
        assert!(sync(&path, &record).is_err());
        settings::save_store(&path, &record, Store::Journal).unwrap();
        let poll = status(&path).unwrap().unwrap();
        assert!(poll.reason.unwrap().contains("preserved"));
    }

    #[test]
    fn a_failed_acknowledgement_keeps_the_undo_of_a_step_that_ran() {
        use crate::settings::{WRITE_FAILURE, WRITE_FAILURE_AFTER, WRITE_FAILURE_ONCE};
        let dir = tempfile::tempdir().unwrap();
        let documents = dir.path().join("Documents");
        std::fs::create_dir(&documents).unwrap();
        let roots = Roots::new(dir.path(), None, &[]);
        let (source, target) = (documents.join("a"), documents.join("b"));
        std::fs::write(&source, b"original").unwrap();
        let history_path = dir.path().join("history.json");
        let mut history = History::load(history_path.clone());
        let path = history.journal_path();
        // Prepared and StepInFlight save; the Acknowledged write after the move fails.
        WRITE_FAILURE.with(|f| f.set(3));
        WRITE_FAILURE_AFTER.with(|n| n.set(2));
        WRITE_FAILURE_ONCE.with(|o| o.set(true));
        let result = run(&path, &[Action::Move { from: source.clone(), to: target.clone() }], &roots, entry(), &mut history);
        WRITE_FAILURE.with(|f| f.set(0));
        WRITE_FAILURE_AFTER.with(|n| n.set(0));
        let Err(error) = result else { panic!("the failed acknowledgement must be reported") };
        assert!(error.contains("can be undone from History"), "{error}");
        assert!(target.exists() && !source.exists());
        // History kept the undo on disk, and review doesn't duplicate or drop it.
        let mut history = History::load(history_path);
        assert_eq!(undo_actions(history.last_undoable().unwrap()), vec![Action::Move { from: target.clone(), to: source.clone() }]);
        acknowledge(&path, &mut history).unwrap();
        assert_eq!(history.entries.len(), 1);
        assert!(history.last_undoable().is_some());
    }

    #[test]
    fn published_move_and_staging_cleanup_failures_are_undoable() {
        use crate::fsx::{STAGE_CLEANUP_FAILURE, UNLINK_FAILURE};
        for (injection, move_file) in [(&UNLINK_FAILURE, true), (&STAGE_CLEANUP_FAILURE, false)] {
            let dir = tempfile::tempdir().unwrap();
            let documents = dir.path().join("Documents");
            std::fs::create_dir(&documents).unwrap();
            let roots = Roots::new(dir.path(), None, &[]);
            let source = documents.join("a");
            let target = documents.join("b");
            let next = documents.join("never-ran");
            std::fs::write(&source, b"original").unwrap();
            let copy = Action::Copy { from: source.clone(), to: target.clone() };
            let action = if move_file { Action::Move { from: source.clone(), to: target.clone() } } else { copy.clone() };
            let history_path = dir.path().join("history.json");
            let mut history = History::load(history_path.clone());
            let path = history.journal_path();
            injection.with(|failure| failure.set(true));
            let result = run(&path, &[action, Action::CreateFolder { path: next.clone() }], &roots, entry(), &mut history);
            injection.with(|failure| failure.set(false));
            let out = result.unwrap();
            assert!(matches!(out.terminal, RunOutcome::Failed { partial: true, .. }));
            assert_eq!(out.done, vec![copy]);
            assert!(status(&path).unwrap().is_none());
            assert!(!next.exists());
            assert_eq!(std::fs::read(&source).unwrap(), b"original");
            assert_eq!(std::fs::read(&target).unwrap(), b"original");

            let mut history = History::load(history_path);
            let undo = undo_actions(history.last_undoable().unwrap());
            assert_eq!(undo, vec![Action::Trash { path: target.clone() }]);
            let attempt = Entry::new(2, "undo last".into(), Some(1));
            assert!(matches!(run(&path, &undo, &roots, attempt, &mut history).unwrap().terminal, RunOutcome::Completed));
            assert!(!target.exists());
            assert_eq!(std::fs::read(&source).unwrap(), b"original");
            assert_eq!(std::fs::read(dir.path().join(".Trash/b")).unwrap(), b"original");
        }
    }

    #[test]
    fn uncertain_step_blocks_mutations_until_review() {
        let dir = tempfile::tempdir().unwrap();
        let documents = dir.path().join("Documents");
        std::fs::create_dir(&documents).unwrap();
        let roots = Roots::new(dir.path(), None, &[]);
        let mut history = History::load(dir.path().join("history.json"));
        let path = history.journal_path();
        let source = documents.join("a");
        std::fs::write(&source, b"original").unwrap();
        std::fs::write(dir.path().join(".Trash"), b"blocked").unwrap();
        let trash = Action::Trash { path: source.clone() };
        let next = Action::CreateFolder { path: documents.join("next") };
        let out = run(&path, &[trash.clone(), next.clone()], &roots, entry(), &mut history).unwrap();
        assert!(matches!(out.terminal, RunOutcome::NeedsReview { .. }));
        assert!(out.done.is_empty());
        assert_eq!(status(&path).unwrap().unwrap().in_flight, Some(trash));
        assert!(run(&path, std::slice::from_ref(&next), &roots, entry(), &mut history).is_err());
        assert!(!documents.join("next").exists());
        acknowledge(&path, &mut history).unwrap();
        assert!(matches!(run(&path, &[next], &roots, entry(), &mut history).unwrap().terminal, RunOutcome::Completed));
        assert_eq!(std::fs::read(source).unwrap(), b"original");
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
        let remaining_path = dir.path().join("Documents/remaining");
        std::fs::write(&remaining_path, b"remaining").unwrap();
        std::fs::write(&c, b"collision").unwrap();
        let second = Action::Move { from: remaining_path, to: c.clone() };
        let history_path = dir.path().join("history.json");
        let mut history = History::load(history_path.clone());
        history.try_push(entry_with_inverses(&[first.clone(), second.clone()])).unwrap();
        let path = dir.path().join("operation-journal.json");
        let undo = Entry { id: 2, undo_of: Some(1), ..entry() };
        let out = run(&path, &[first, second.clone()], &roots, undo, &mut history).unwrap();
        assert_eq!(out.done.len(), 1);
        assert!(out.terminal.error().is_some());
        let mut history = History::load(history_path.clone());
        let remaining = history.last_undoable().unwrap();
        assert_eq!(remaining.id, 1);
        assert!(matches!(remaining.state, EntryState::PartiallyUndone(_)));
        assert_eq!(undo_actions(remaining), vec![second.clone()]);
        std::fs::remove_file(&c).unwrap();
        let undo = Entry { id: 3, undo_of: Some(1), ..entry() };
        assert!(run(&path, &[second], &roots, undo, &mut history).unwrap().terminal.error().is_none());
        let history = History::load(history_path);
        assert!(matches!(history.entries[0].state, EntryState::Exhausted));
        assert!(history.entries[0].inverses().is_empty());
        assert!(b.exists() && c.exists() && !a.exists());
    }

    #[test]
    fn cancelled_undo_keeps_only_remaining_steps() {
        let dir = tempfile::tempdir().unwrap();
        let roots = Roots::new(dir.path(), None, &[]);
        let documents = dir.path().join("Documents");
        std::fs::create_dir(&documents).unwrap();
        let first = Action::Move { from: documents.join("a"), to: documents.join("a-restored") };
        let second = Action::Move { from: documents.join("b"), to: documents.join("b-restored") };
        std::fs::write(documents.join("a"), b"first").unwrap();
        std::fs::write(documents.join("b"), b"second").unwrap();
        let history_path = dir.path().join("history.json");
        let mut history = History::load(history_path.clone());
        history.try_push(entry_with_inverses(&[first.clone(), second.clone()])).unwrap();
        let path = history.journal_path();
        let undo = Entry { id: 2, undo_of: Some(1), ..entry() };
        let out = super::run(&path, &[first.clone(), second.clone()], &roots, undo.command, undo.undo_of, &mut history, &|| {
            documents.join("a-restored").exists()
        })
        .unwrap();
        assert!(matches!(out.terminal, RunOutcome::Cancelled));
        assert_eq!(out.done, vec![first]);
        let record = read(&path).unwrap().unwrap();
        assert!(matches!(record.state, State::Finished(_)));
        assert!(status(&path).unwrap().is_none());

        let mut history = History::load(history_path.clone());
        assert_eq!(undo_actions(history.last_undoable().unwrap()), vec![second.clone()]);
        let undo = Entry { id: 3, undo_of: Some(1), ..entry() };
        assert!(run(&path, &[second], &roots, undo, &mut history).unwrap().terminal.error().is_none());
        assert!(History::load(history_path).last_undoable().is_none());
        assert_eq!(std::fs::read(documents.join("a-restored")).unwrap(), b"first");
        assert_eq!(std::fs::read(documents.join("b-restored")).unwrap(), b"second");
    }

    #[test]
    fn cancelled_undo_keeps_the_identity_restored_by_a_completed_inverse() {
        use crate::fsx::FileIdentity;
        let dir = tempfile::tempdir().unwrap();
        let documents = dir.path().join("Documents");
        std::fs::create_dir(&documents).unwrap();
        let original = documents.join("converted.png");
        let backup = documents.join("backup.png");
        std::fs::write(&original, b"converted").unwrap();
        let remove = Action::Trash { path: original.clone() };
        let remove_original = Inverse::capture(remove.clone(), &[FileIdentity::read(&original).unwrap()]).unwrap();
        std::fs::copy(&original, &backup).unwrap();
        std::fs::write(&original, b"resized").unwrap();
        let restore = Action::Restore { from: backup.clone(), to: original.clone() };
        let restore_backup =
            Inverse::capture(restore.clone(), &[FileIdentity::read(&backup).unwrap(), FileIdentity::read(&original).unwrap()]).unwrap();
        let history_path = dir.path().join("history.json");
        let mut history = History::load(history_path.clone());
        history.try_push(Entry { state: EntryState::with_inverses(vec![restore_backup, remove_original]), ..entry() }).unwrap();
        let path = history.journal_path();
        let cancelled = super::run(
            &path,
            &[restore, remove.clone()],
            &Roots::new(dir.path(), None, &[]),
            "undo last".into(),
            Some(1),
            &mut history,
            &|| !backup.exists(),
        )
        .unwrap();
        assert!(matches!(cancelled.terminal, RunOutcome::Cancelled));

        let mut history = History::load(history_path);
        let remaining = history.last_undoable().unwrap();
        assert!(matches!(remaining.state, EntryState::PartiallyUndone(_)));
        assert_eq!(undo_actions(remaining), vec![remove.clone()]);
        validate_inverses(remaining.inverses()).unwrap();
        let roots = Roots::new(dir.path(), None, &[]);
        let attempt = Entry::new(3, "undo last".into(), Some(1));
        assert!(matches!(run(&path, &[remove], &roots, attempt, &mut history).unwrap().terminal, RunOutcome::Completed));
        assert!(!original.exists());
        assert_eq!(std::fs::read(dir.path().join(".Trash/converted.png")).unwrap(), b"resized");
    }

    #[test]
    fn returned_failures_finish_and_allow_the_next_operation() {
        for collision in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let roots = Roots::new(dir.path(), None, &[]);
            for folder in ["Downloads", "Documents"] {
                std::fs::create_dir(dir.path().join(folder)).unwrap();
            }
            let source = dir.path().join("Downloads/a");
            let target = dir.path().join("Documents/a");
            if collision {
                std::fs::write(&source, "source").unwrap();
                std::fs::write(&target, "existing").unwrap();
            }
            let path = dir.path().join("operation-journal.json");
            let mut history = History::load(dir.path().join("history.json"));
            let out = run(&path, &[Action::Move { from: source, to: target.clone() }], &roots, entry(), &mut history).unwrap();
            assert!(out.terminal.error().is_some());
            let record = read(&path).unwrap().unwrap();
            assert!(matches!(record.state, State::Finished(_)));
            let State::Finished(progress) = record.state else { unreachable!() };
            assert_eq!(progress.error.as_deref(), out.terminal.error());
            assert!(status(&path).unwrap().is_none());
            recover(&path, &mut history).unwrap();
            run(&path, &[Action::CreateFolder { path: dir.path().join("Documents/next") }], &roots, entry(), &mut history).unwrap();
            if collision {
                assert_eq!(std::fs::read(target).unwrap(), b"existing");
            }
        }
    }

    #[test]
    fn interruption_requires_review_and_archives_exact_evidence() {
        for in_flight in [None, Some(1)] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("operation-journal.json");
            let mut history = History::load(dir.path().join("history.json"));
            let action = Action::Move { from: dir.path().join("Downloads/a"), to: dir.path().join("Documents/a") };
            let entry = Entry { done: vec![action.clone()], ..entry() };
            let state = match in_flight {
                Some(index) => State::StepInFlight { entry, index },
                None => State::Acknowledged { count: 1, progress: Progress { entry, error: None } },
            };
            let record = Record { actions: vec![action.clone(), action.clone()], state };
            sync(&path, &record).unwrap();
            let original = std::fs::read(&path).unwrap();
            let status = status(&path).unwrap().unwrap();
            assert_eq!(status.command, "move files");
            assert_eq!(status.acknowledged, 1);
            assert_eq!(status.in_flight, in_flight.map(|_| action));
            assert!(recover(&path, &mut history).unwrap_err().contains("Mark as reviewed"));
            assert_eq!(std::fs::read(&path).unwrap(), original);
            acknowledge(&path, &mut history).unwrap();
            assert!(!path.exists());
            let archives: Vec<_> = std::fs::read_dir(dir.path())
                .unwrap()
                .map(|e| e.unwrap().path())
                .filter(|p| p.file_name().unwrap().to_string_lossy().starts_with("operation-journal.interrupted-"))
                .collect();
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
        let progress = Progress { entry: Entry { done: vec![action.clone()], ..entry() }, error: None };
        sync(&path, &Record { actions: vec![action], state: State::Acknowledged { progress, count: 1 } }).unwrap();
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
        run(
            &home.join("journal.json"),
            &[
                Action::Move { from: home.join("Downloads/a"), to: home.join("Documents/a") },
                Action::Move { from: home.join("Downloads/b"), to: home.join("Documents/b") },
            ],
            &roots,
            Entry::new(1, "move".into(), None),
            &mut history,
        )
        .unwrap();
    }

    #[test]
    fn crash_boundaries_never_replay_mutations() {
        for point in ["before_sync", "after_sync", "before_mutation", "after_mutation"] {
            let count = if point.ends_with("sync") { 7 } else { 2 };
            for nth in 1..=count {
                let dir = tempfile::tempdir().unwrap();
                for folder in ["Downloads", "Documents"] {
                    std::fs::create_dir(dir.path().join(folder)).unwrap();
                }
                for name in ["a", "b"] {
                    std::fs::write(dir.path().join("Downloads").join(name), "original").unwrap();
                }
                let status = std::process::Command::new(std::env::current_exe().unwrap())
                    .args(["--exact", "operation_journal::tests::crash_child"])
                    .env("VOXI_JOURNAL_CHILD", dir.path())
                    .env("VOXI_JOURNAL_CRASH", point)
                    .env("VOXI_JOURNAL_NTH", nth.to_string())
                    .status()
                    .unwrap();
                assert_eq!(status.code(), Some(86), "{point} {nth}");
                let mut history = History::load(dir.path().join("history.json"));
                for _ in 0..2 {
                    let _ = recover(&dir.path().join("journal.json"), &mut history);
                }
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

    #[test]
    fn future_version_journal_is_untouched_and_blocks_execution() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("Documents")).unwrap();
        let path = dir.path().join("operation-journal.json");
        let future = br#"{"version":2,"data":{"futureField":true}}"#;
        std::fs::write(&path, future).unwrap();
        let roots = Roots::new(dir.path(), None, &[]);
        let mut history = History::load(dir.path().join("history.json"));

        let Err(err) = status(&path) else { panic!("expected unsupported version error from status") };
        assert!(err.contains("Unsupported store version 2"), "{err}");
        let err = recover(&path, &mut history).unwrap_err();
        assert!(err.contains("Unsupported store version 2"), "{err}");
        let action = Action::CreateFolder { path: dir.path().join("Documents/new") };
        let Err(err) = run(&path, std::slice::from_ref(&action), &roots, entry(), &mut history) else {
            panic!("expected unsupported version error from run")
        };
        assert!(err.contains("Unsupported store version 2"), "{err}");
        assert!(!dir.path().join("Documents/new").exists());
        let err = acknowledge(&path, &mut history).unwrap_err();
        assert!(err.contains("Unsupported store version 2"), "{err}");

        assert_eq!(std::fs::read(&path).unwrap(), future);
        let backups: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.to_string_lossy().contains(".corrupt-") || p.to_string_lossy().contains(".interrupted-"))
            .collect();
        assert!(backups.is_empty(), "future version must not be treated as corruption: {backups:?}");
    }
}
