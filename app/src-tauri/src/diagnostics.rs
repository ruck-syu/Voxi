//! Private operational diagnostics: three files (`diagnostics.0.log` through
//! `diagnostics.2.log`), each at most 1 MiB, oldest discarded on rotation.
//! Retention is size-based, with no age guarantee. No telemetry. History and
//! opt-in feedback are separate stores with their existing retention policies.
//! Producers never perform disk IO or wait for the writer: a full queue drops
//! events. IO failure disables this session's sink with at most one fixed stderr
//! note. Queued events may be lost on crash/exit; this is not a recovery journal.

use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{
        OnceLock,
        atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
};

const CAP: u64 = 1024 * 1024;
const NAMES: [&str; 3] = ["diagnostics.0.log", "diagnostics.1.log", "diagnostics.2.log"];
static SINK: OnceLock<Sink> = OnceLock::new();
static NOTE: AtomicBool = AtomicBool::new(false);
static SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug)]
pub(crate) enum Store {
    Settings,
    History,
    Schedules,
    Journal,
}
#[derive(Clone, Copy, Debug)]
pub(crate) enum Code {
    Startup,
    StoreLoad,
    StoreSave,
    StoreBackup,
    SchedulerRun,
    SchedulerSkip,
    JournalInterrupted,
    JournalRecovered,
    LoginItem,
    Update,
}
impl Code {
    fn label(self) -> &'static str {
        match self {
            Self::Startup => "E_STARTUP",
            Self::StoreLoad => "E_STORE_LOAD",
            Self::StoreSave => "E_STORE_SAVE",
            Self::StoreBackup => "E_STORE_BACKUP",
            Self::SchedulerRun => "E_SCHED_RUN",
            Self::SchedulerSkip => "E_SCHED_SKIP",
            Self::JournalInterrupted => "E_JOURNAL_INTERRUPTED",
            Self::JournalRecovered => "E_JOURNAL_RECOVERED",
            Self::LoginItem => "E_LOGIN_ITEM",
            Self::Update => "E_UPDATE",
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub(crate) enum Stage {
    Begin,
    Failed,
    Preserved,
    BackupFailed,
    Interrupted,
    Recovered,
    Skipped,
}

/// No string/path/payload field exists. Even IO errors are reduced immediately
/// to ErrorKind and an optional OS integer, never Display/Debug of the error.
#[derive(Clone, Copy)]
pub(crate) struct Event {
    code: Code,
    stage: Stage,
    operation: u64,
    store: Option<Store>,
    count: usize,
    step: Option<usize>,
    io: Option<(io::ErrorKind, Option<i32>)>,
}
impl Event {
    pub(crate) fn new(code: Code, stage: Stage) -> Self {
        Self { code, stage, operation: SEQUENCE.fetch_add(1, Ordering::Relaxed), store: None, count: 0, step: None, io: None }
    }
    pub(crate) fn operation(mut self, id: u64) -> Self {
        self.operation = id;
        self
    }
    pub(crate) fn store(mut self, store: Store) -> Self {
        self.store = Some(store);
        self
    }
    pub(crate) fn count(mut self, count: usize) -> Self {
        self.count = count;
        self
    }
    pub(crate) fn step(mut self, step: Option<usize>) -> Self {
        self.step = step;
        self
    }
    pub(crate) fn io(mut self, error: &io::Error) -> Self {
        self.io = Some((error.kind(), error.raw_os_error()));
        self
    }
    fn line(self) -> String {
        let level = if matches!(self.stage, Stage::Begin | Stage::Preserved | Stage::Recovered) { "INFO" } else { "WARN" };
        format!(
            "{} level={} code={} op={}-{}-{} stage={:?} store={:?} count={} step={:?} io={:?}\n",
            chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            level,
            self.code.label(),
            std::process::id(),
            SESSION.get_or_init(|| chrono::Utc::now().timestamp_micros()),
            self.operation,
            self.stage,
            self.store,
            self.count,
            self.step,
            self.io
        )
    }
}
static SESSION: OnceLock<i64> = OnceLock::new();
fn unavailable() {
    if !NOTE.swap(true, Ordering::Relaxed) {
        eprintln!("Voxi diagnostics unavailable");
    }
}
#[repr(u8)]
enum Health {
    Live,
    Failed,
}

struct Sink {
    tx: SyncSender<Event>,
    health: AtomicU8,
}

impl Sink {
    fn new(tx: SyncSender<Event>) -> Self {
        Self { tx, health: AtomicU8::new(Health::Live as u8) }
    }

    fn fail(&self) {
        self.health.store(Health::Failed as u8, Ordering::Relaxed);
        unavailable();
    }

    fn emit(&self, event: Event) {
        if self.health.load(Ordering::Relaxed) == Health::Live as u8
            && matches!(self.tx.try_send(event), Err(mpsc::TrySendError::Disconnected(_)))
        {
            self.fail();
        }
    }

    fn consume(&self, dir: &Path, rx: &Receiver<Event>) {
        let result = (|| -> io::Result<()> {
            let mut writer = Writer::open(dir)?;
            for event in rx {
                writer.write(event)?;
            }
            Ok(())
        })();
        if result.is_err() {
            self.fail();
        }
    }
}

pub(crate) fn emit(event: Event) {
    if let Some(sink) = SINK.get() {
        sink.emit(event);
    }
}

/// Start before loading stores. Directory creation and opening run on the worker.
pub(crate) fn init(dir: PathBuf) {
    let (tx, rx) = mpsc::sync_channel(256);
    if SINK.set(Sink::new(tx)).is_err() {
        return;
    }
    let sink = SINK.get().expect("sink was registered");
    if std::thread::Builder::new().name("voxi-diagnostics".into()).spawn(move || sink.consume(&dir, &rx)).is_err() {
        sink.fail();
    }
}

struct Writer {
    dir: File,
    file: File,
    size: u64,
}
impl Writer {
    fn open(path: &Path) -> io::Result<Self> {
        fs::DirBuilder::new().recursive(true).mode(0o700).create(path)?;
        let dir = OpenOptions::new().read(true).custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC).open(path)?;
        if dir.metadata()?.uid() != unsafe { libc::geteuid() } {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
        dir.set_permissions(fs::Permissions::from_mode(0o700))?;
        // Advisory lock excludes another Voxi process from rotating this sink.
        use std::os::fd::AsRawFd;
        if unsafe { libc::flock(dir.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let file = Self::file(&dir)?;
        let size = file.metadata()?.len();
        let mut writer = Self { dir, file, size };
        // Reject oversized or unsafe pre-existing archives, rather than keep
        // unbounded files or follow links introduced outside the logger.
        for name in &NAMES[1..] {
            let meta = writer.at_metadata(name)?;
            if let Some(meta) = meta
                && (!meta.is_file() || meta.nlink() != 1 || meta.len() > CAP || meta.permissions().mode() & 0o777 != 0o600)
            {
                return Err(io::ErrorKind::PermissionDenied.into());
            }
        }
        if size > CAP {
            writer.file.set_len(0)?;
            writer.size = 0;
        }
        Ok(writer)
    }
    fn at_metadata(&self, name: &str) -> io::Result<Option<fs::Metadata>> {
        match self.open_at(name, libc::O_RDONLY | libc::O_NONBLOCK, 0) {
            Ok(file) => Ok(Some(file.metadata()?)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }
    fn open_at(&self, name: &str, flags: i32, mode: libc::mode_t) -> io::Result<File> {
        open_at(&self.dir, name, flags, mode)
    }
    fn file(dir: &File) -> io::Result<File> {
        let file = open_at(dir, NAMES[0], libc::O_WRONLY | libc::O_APPEND | libc::O_CREAT | libc::O_NONBLOCK, 0o600)?;
        let meta = file.metadata()?;
        if !meta.is_file() || meta.nlink() != 1 || meta.uid() != unsafe { libc::geteuid() } {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
        Ok(file)
    }
    fn write(&mut self, event: Event) -> io::Result<()> {
        let line = event.line();
        if self.size + line.len() as u64 > CAP {
            use std::{ffi::CString, os::fd::AsRawFd};
            for (from, to) in [(NAMES[1], NAMES[2]), (NAMES[0], NAMES[1])] {
                let from = CString::new(from).map_err(io::Error::other)?;
                let to = CString::new(to).map_err(io::Error::other)?;
                if unsafe { libc::renameat(self.dir.as_raw_fd(), from.as_ptr(), self.dir.as_raw_fd(), to.as_ptr()) } != 0 {
                    let e = io::Error::last_os_error();
                    if e.kind() != io::ErrorKind::NotFound {
                        return Err(e);
                    }
                }
            }
            self.file = Self::file(&self.dir)?;
            self.size = 0;
        }
        self.file.write_all(line.as_bytes())?;
        self.size += line.len() as u64;
        // Unbuffered writes survive GUI launches without a stdout/stderr sink.
        Ok(())
    }
}
fn open_at(dir: &File, name: &str, flags: i32, mode: libc::mode_t) -> io::Result<File> {
    use std::{
        ffi::CString,
        os::fd::{AsRawFd, FromRawFd},
    };
    let name = CString::new(name).map_err(io::Error::other)?;
    let fd = unsafe { libc::openat(dir.as_raw_fd(), name.as_ptr(), flags | libc::O_NOFOLLOW | libc::O_CLOEXEC, mode as libc::c_uint) };
    if fd < 0 { Err(io::Error::last_os_error()) } else { Ok(unsafe { File::from_raw_fd(fd) }) }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn contents(dir: &Path) -> String {
        NAMES.iter().filter_map(|name| fs::read_to_string(dir.join(name)).ok()).collect()
    }

    #[test]
    fn rotation_is_bounded_and_private() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("logs");
        let mut writer = Writer::open(&dir).unwrap();
        let event = Event::new(Code::StoreSave, Stage::Failed).store(Store::Settings);
        // More than four caps ensures the oldest archive is replaced too.
        for _ in 0..(CAP * 5 / event.line().len() as u64) {
            writer.write(event).unwrap();
        }
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 3);
        assert_eq!(fs::metadata(&dir).unwrap().permissions().mode() & 0o777, 0o700);
        for name in NAMES {
            let meta = fs::metadata(dir.join(name)).unwrap();
            assert!(meta.len() > 0 && meta.len() <= CAP);
            assert_eq!(meta.permissions().mode() & 0o777, 0o600);
        }
    }

    #[test]
    fn every_entry_point_reduces_sensitive_errors_before_rotation() {
        let temp = tempfile::tempdir().unwrap();
        let secret = "move SECRET-command /Users/alice/private SECRET-filename.pdf run my SECRET-shortcut notes SECRET-model";
        let error = io::Error::other(secret);
        let codes = [
            Code::Startup,
            Code::StoreLoad,
            Code::StoreSave,
            Code::StoreBackup,
            Code::SchedulerRun,
            Code::SchedulerSkip,
            Code::JournalInterrupted,
            Code::JournalRecovered,
            Code::LoginItem,
            Code::Update,
        ];
        let stages =
            [Stage::Begin, Stage::Failed, Stage::Preserved, Stage::BackupFailed, Stage::Interrupted, Stage::Recovered, Stage::Skipped];
        let mut writer = Writer::open(temp.path()).unwrap();
        for _ in 0..100 {
            for code in codes {
                for stage in stages {
                    for store in [Store::Settings, Store::History, Store::Schedules, Store::Journal] {
                        let event = Event::new(code, stage).io(&error).store(store).operation(42).count(3).step(Some(2));
                        // Exercise the same structured event at both producer and sink.
                        emit(event);
                        writer.write(event).unwrap();
                    }
                }
            }
        }
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 3);
        let logs = contents(temp.path());
        for sentinel in [secret, "SECRET-command", "/Users/", "SECRET-filename.pdf", "run my SECRET-shortcut", "SECRET-model"] {
            assert!(!logs.contains(sentinel));
        }
        assert!(logs.contains("io=Some((Other, None))"));
    }

    #[test]
    fn startup_and_scheduler_failure_persist_structured_context() {
        let temp = tempfile::tempdir().unwrap();
        {
            let mut writer = Writer::open(temp.path()).unwrap();
            writer.write(Event::new(Code::Startup, Stage::Begin).operation(1)).unwrap();
            writer.write(Event::new(Code::Startup, Stage::Failed).operation(1)).unwrap();
            writer.write(Event::new(Code::SchedulerRun, Stage::Failed).operation(99).count(4).step(Some(2))).unwrap();
        }
        let logs = contents(temp.path());
        assert!(logs.contains("level=INFO code=E_STARTUP"));
        assert!(logs.contains("level=WARN code=E_STARTUP"));
        assert!(logs.contains("code=E_SCHED_RUN"));
        assert!(logs.contains("-99 stage=Failed store=None count=4 step=Some(2)"));
    }

    #[test]
    fn unwritable_sink_and_full_queue_never_fail_the_operation() {
        let temp = tempfile::tempdir().unwrap();
        let blocked = temp.path().join("file");
        fs::write(&blocked, "keep").unwrap();
        let operation = || {
            assert!(Writer::open(&blocked.join("logs")).is_err());
            let (tx, _rx) = mpsc::sync_channel(1);
            let event = Event::new(Code::Startup, Stage::Failed);
            let _ = tx.try_send(event);
            assert!(tx.try_send(event).is_err());
            emit(event);
            Ok::<_, io::Error>(7)
        };
        assert_eq!(operation().unwrap(), 7);
        assert_eq!(fs::read(blocked).unwrap(), b"keep");
    }

    #[test]
    fn failed_worker_disables_sends() {
        let dir = tempfile::tempdir().unwrap();
        let blocked = dir.path().join("file");
        fs::write(&blocked, b"keep").unwrap();
        let (tx, rx) = mpsc::sync_channel(1);
        let sink = Sink::new(tx);
        sink.consume(&blocked.join("logs"), &rx);
        assert_eq!(sink.health.load(Ordering::Relaxed), Health::Failed as u8);
        sink.emit(Event::new(Code::Startup, Stage::Failed));
        assert!(matches!(rx.try_recv(), Err(mpsc::TryRecvError::Empty)));
    }

    #[test]
    fn refuses_symlinks_and_a_second_writer() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("logs");
        let writer = Writer::open(&dir).unwrap();
        assert!(Writer::open(&dir).is_err());
        drop(writer);
        let target = temp.path().join("secret");
        fs::write(&target, "untouched").unwrap();
        fs::remove_file(dir.join(NAMES[0])).unwrap();
        std::os::unix::fs::symlink(&target, dir.join(NAMES[0])).unwrap();
        assert!(Writer::open(&dir).is_err());
        assert_eq!(fs::read(target).unwrap(), b"untouched");
    }
}
