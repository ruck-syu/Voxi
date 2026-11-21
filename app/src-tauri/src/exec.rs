//! Runs a confirmed plan step by step and records how to reverse each step.

use crate::fsx::{self, FileIdentity, Publication, PublishError, Roots, file_name};
use crate::history::Inverse;
use crate::model::{Action, RunOutcome};
use crate::{pdf, trash};
use std::fs;
use std::io;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

pub(crate) struct Outcome {
    pub done: Vec<Action>,
    /// Reverses known effects, newest step first.
    pub inverse: Vec<Inverse>,
    pub terminal: RunOutcome,
    pub entry_id: Option<u64>,
}

#[derive(Debug, PartialEq)]
pub(crate) struct Effect {
    pub action: Action,
    pub inverse: Vec<Inverse>,
}

impl Effect {
    fn reversible(action: Action, inverse: Action, identities: &[FileIdentity]) -> Result<Self, StepError> {
        let inverse = Inverse::capture(inverse, identities).map_err(StepError::Uncertain)?;
        Ok(Self { action, inverse: vec![inverse] })
    }

    fn irreversible(action: &Action) -> Self {
        Self { action: action.clone(), inverse: vec![] }
    }
}

/// Internal sentinel used by cancellable tools writing only to private staging.
const CANCELLED: &str = "Execution cancelled.";

#[derive(Debug, PartialEq)]
pub(crate) enum StepError {
    Cancelled,
    Failed(String),
    Partial { message: String, effect: Box<Effect> },
    Uncertain(String),
}

impl From<String> for StepError {
    fn from(message: String) -> Self {
        if message == CANCELLED { Self::Cancelled } else { Self::Failed(message) }
    }
}

fn check_cancel(is_cancelled: &dyn Fn() -> bool) -> Result<(), String> {
    if is_cancelled() { Err(CANCELLED.into()) } else { Ok(()) }
}

fn never_cancel() -> bool {
    false
}

pub(crate) fn run(actions: &[Action], roots: &Roots) -> Outcome {
    run_with_cancel(actions, roots, &never_cancel)
}

pub(crate) fn run_with_cancel(actions: &[Action], roots: &Roots, is_cancelled: &dyn Fn() -> bool) -> Outcome {
    let mut out = Outcome { done: vec![], inverse: vec![], terminal: RunOutcome::Completed, entry_id: None };
    for action in actions {
        let (effect, terminal) = match step(action, roots, is_cancelled) {
            Ok(effect) => (Some(effect), RunOutcome::Completed),
            Err(StepError::Cancelled) => (None, RunOutcome::Cancelled),
            Err(StepError::Failed(error)) => (None, RunOutcome::Failed { error, partial: !out.done.is_empty() }),
            Err(StepError::Partial { message, effect }) => (Some(*effect), RunOutcome::Failed { error: message, partial: true }),
            Err(StepError::Uncertain(error)) => (None, RunOutcome::NeedsReview { error }),
        };
        if let Some(effect) = effect {
            out.done.push(effect.action);
            out.inverse.splice(0..0, effect.inverse);
        }
        if !matches!(terminal, RunOutcome::Completed) {
            out.terminal = terminal;
            break;
        }
    }
    out
}

fn guard(roots: &Roots, paths: &[&Path]) -> Result<(), String> {
    for p in paths {
        if !roots.is_allowed(p) || p.parent().is_some_and(|parent| !roots.is_allowed(parent)) {
            return Err(format!("Stopped: {} is outside the allowed folders.", roots.display(p)));
        }
    }
    Ok(())
}

/// Opening a folder is harmless: an allowed root itself may be opened. Only
/// this read-only action gets the exemption; moves, renames and trash still
/// use `guard`, whose parent check keeps roots out of destructive actions.
fn guard_open(roots: &Roots, path: &Path) -> Result<(), String> {
    if !roots.is_allowed(path) || (!roots.is_root(path) && path.parent().is_some_and(|parent| !roots.is_allowed(parent))) {
        return Err(format!("Stopped: {} is outside the allowed folders.", roots.display(path)));
    }
    Ok(())
}

fn stopped(p: &Path) -> String {
    format!("Stopped: {} appeared since the preview. Nothing was overwritten.", p.display())
}

fn fresh(p: &Path) -> Result<(), String> {
    if p.symlink_metadata().is_ok() { Err(stopped(p)) } else { Ok(()) }
}

fn io_err(what: &str, p: &Path) -> impl Fn(io::Error) -> String {
    let msg = format!("Couldn't {what} {}", file_name(p));
    move |e| format!("{msg}: {e}")
}

/// Retained bytes per stdout/stderr stream; excess output is drained/discarded.
const TOOL_CAPTURE_BYTES: usize = 64 * 1024;
fn tool(cmd: &str, args: &[&std::ffi::OsStr]) -> Result<(), String> {
    bounded_tool(cmd, args, std::time::Duration::from_secs(120), || Ok(()))
}
fn bounded_tool(
    cmd: &str,
    args: &[&std::ffi::OsStr],
    deadline: std::time::Duration,
    mut check: impl FnMut() -> Result<(), String>,
) -> Result<(), String> {
    use std::io::Read;
    use std::os::fd::AsRawFd;
    use std::os::unix::process::CommandExt;
    use std::process::Stdio;
    check()?;
    let mut child = Command::new(cmd)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()
        .map_err(|e| format!("Couldn't run {cmd}: {e}"))?;
    let mut stdout = child.stdout.take().ok_or("Tool stdout pipe missing")?;
    let mut stderr = child.stderr.take().ok_or("Tool stderr pipe missing")?;
    // Nonblocking reads avoid
    // reader threads hanging on inherited pipes after the direct child exits.
    for fd in [stdout.as_raw_fd(), stderr.as_raw_fd()] {
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
            terminate_tool(&mut child);
            return Err("Cannot configure tool pipes".into());
        }
    }
    fn drain(reader: &mut dyn Read, kept: &mut Vec<u8>) -> io::Result<()> {
        let mut buf = [0; 8192];
        // Bound each polling iteration even if the writer never stops.
        for _ in 0..32 {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => kept.extend_from_slice(&buf[..n.min(TOOL_CAPTURE_BYTES - kept.len())]),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }
    let start = std::time::Instant::now();
    let mut out = Vec::new();
    let mut err = Vec::new();
    let result = loop {
        if let Err(e) = drain(&mut stdout, &mut out).and_then(|()| drain(&mut stderr, &mut err)) {
            break Err(e.to_string());
        }
        if start.elapsed() >= deadline {
            break Err(format!("{cmd} timed out after {} seconds", deadline.as_secs_f64()));
        }
        if let Err(e) = check() {
            break Err(e);
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                let _ = drain(&mut stdout, &mut out);
                let _ = drain(&mut stderr, &mut err);
                // try_wait reaped the leader: its pgid may now be reused.
                // Drop our pipes on return; descendants need not close theirs.
                return if status.success() { Ok(()) } else { Err(format!("{cmd} failed: {}", String::from_utf8_lossy(&err))) };
            }
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(10)),
            Err(e) => break Err(e.to_string()),
        }
    };
    // Only error paths with an un-reaped leader may signal the group.
    terminate_tool(&mut child);
    result
}

fn terminate_tool(child: &mut std::process::Child) {
    // The un-reaped leader reserves its pid/pgid until after this signal.
    unsafe {
        libc::kill(-(child.id() as libc::pid_t), libc::SIGKILL);
    }
    let _ = child.wait();
}

/// Maximum members, including directory entries, accepted from a ZIP.
const ARCHIVE_ENTRIES: usize = 10_000;
/// Maximum declared and observed bytes in any regular file (1 GiB).
const ARCHIVE_FILE_BYTES: u64 = 1024 * 1024 * 1024;
/// Maximum declared and observed aggregate output (5 GiB).
const ARCHIVE_TOTAL_BYTES: u64 = 5 * ARCHIVE_FILE_BYTES;
/// Maximum expansion per entry and across the archive.
const ARCHIVE_RATIO: u64 = 100;
/// Free space reserved beyond declared output and the private archive snapshot.
const ARCHIVE_DISK_MARGIN: u64 = 256 * 1024 * 1024;
#[derive(Clone, Copy)]
struct ArchiveLimits {
    entries: usize,
    file: u64,
    total: u64,
    ratio: u64,
    margin: u64,
}
impl Default for ArchiveLimits {
    fn default() -> Self {
        Self {
            entries: ARCHIVE_ENTRIES,
            file: ARCHIVE_FILE_BYTES,
            total: ARCHIVE_TOTAL_BYTES,
            ratio: ARCHIVE_RATIO,
            margin: ARCHIVE_DISK_MARGIN,
        }
    }
}

/// Strict classic-ZIP central/local header validation. ZIP64, encryption,
/// non-UTF8 names, special files and ambiguous separators fail closed.
/// Read headers individually so malicious metadata cannot allocate unbounded RAM.
fn inspect_archive(path: &Path, limits: ArchiveLimits, is_cancelled: &dyn Fn() -> bool) -> Result<u64, String> {
    use std::io::{Read, Seek, SeekFrom};
    let invalid = || "Unsafe, unsupported, or over-budget ZIP archive".to_string();
    let mut f = fs::File::open(path).map_err(|e| e.to_string())?;
    let len = f.metadata().map_err(|e| e.to_string())?.len();
    let tail_len = len.min(65557) as usize;
    f.seek(SeekFrom::End(-(tail_len as i64))).map_err(|e| e.to_string())?;
    let mut tail = vec![0; tail_len];
    f.read_exact(&mut tail).map_err(|e| e.to_string())?;
    let u16at = |b: &[u8], n| u16::from_le_bytes([b[n], b[n + 1]]) as u64;
    let u32at = |b: &[u8], n| u32::from_le_bytes([b[n], b[n + 1], b[n + 2], b[n + 3]]) as u64;
    let end = (0..tail.len().saturating_sub(21))
        .rev()
        .find(|&n| tail[n..].starts_with(b"PK\x05\x06") && n + 22 + u16at(&tail, n + 20) as usize == tail.len())
        .ok_or_else(invalid)?;
    let e = &tail[end..];
    let count = u16at(e, 10) as usize;
    let offset = u32at(e, 16);
    let size = u32at(e, 12);
    if u16at(e, 4) != 0
        || u16at(e, 6) != 0
        || u16at(e, 8) as usize != count
        || count == 65535
        || count > limits.entries
        || offset.checked_add(size) != Some(len - tail_len as u64 + end as u64)
    {
        return Err(invalid());
    }
    let mut pos = offset;
    let mut total = 0u64;
    let mut compressed = 0u64;
    let mut names = std::collections::HashSet::new();
    for _ in 0..count {
        check_cancel(is_cancelled)?;
        f.seek(SeekFrom::Start(pos)).map_err(|e| e.to_string())?;
        let mut h = [0; 46];
        f.read_exact(&mut h).map_err(|e| e.to_string())?;
        if &h[..4] != b"PK\x01\x02" {
            return Err(invalid());
        }
        let flags = u16at(&h, 8);
        let method = u16at(&h, 10);
        let packed = u32at(&h, 20);
        let bytes = u32at(&h, 24);
        let n = u16at(&h, 28) as usize;
        let local = u32at(&h, 42);
        let mode = (u32at(&h, 38) >> 16) & 0o170000;
        if n == 0
            || n > 4096
            || flags & !0x080e != 0
            || ![0, 8].contains(&method)
            || ![0, 0o100000, 0o040000].contains(&mode)
            || (method == 0 && bytes != packed)
            || bytes > limits.file
            || bytes > packed.saturating_mul(limits.ratio)
            || u16at(&h, 34) != 0
        {
            return Err(invalid());
        }
        let mut name = vec![0; n];
        f.read_exact(&mut name).map_err(|e| e.to_string())?;
        let name_str = std::str::from_utf8(&name).map_err(|_| invalid())?;
        if name_str.trim_end_matches('/').split('/').any(|c| c.is_empty())
            || name_str.starts_with('/')
            || name_str.contains(['\\', ':', '\0'])
            || name_str.split('/').any(|c| c == ".." || c == ".")
            || name_str.split('/').count() > 64
            || !names.insert(name_str.trim_end_matches('/').to_lowercase())
        {
            return Err(invalid());
        }
        fn safe_extra(f: &mut fs::File, size: usize) -> Result<(), String> {
            let mut extra = vec![0; size];
            f.read_exact(&mut extra).map_err(|e| e.to_string())?;
            let mut at = 0;
            while at < extra.len() {
                if at + 4 > extra.len() {
                    return Err("Malformed ZIP extra field".into());
                }
                let id = u16::from_le_bytes([extra[at], extra[at + 1]]);
                let n = u16::from_le_bytes([extra[at + 2], extra[at + 3]]) as usize;
                // These fields can override names or encode link targets.
                if [0x7075, 0x000d, 0x756e, 0x0001].contains(&id) {
                    return Err("Unsupported ZIP path/link/ZIP64 extra field".into());
                }
                at += 4 + n;
                if at > extra.len() {
                    return Err("Malformed ZIP extra field".into());
                }
            }
            Ok(())
        }
        safe_extra(&mut f, u16at(&h, 30) as usize)?;
        pos += 46 + n as u64 + u16at(&h, 30) + u16at(&h, 32);
        if pos > offset + size {
            return Err(invalid());
        }
        // The extractor must see the same member name and method locally.
        f.seek(SeekFrom::Start(local)).map_err(|e| e.to_string())?;
        let mut l = [0; 30];
        f.read_exact(&mut l).map_err(|e| e.to_string())?;
        if &l[..4] != b"PK\x03\x04"
            || u16at(&l, 6) != flags
            || u16at(&l, 8) != method
            || u16at(&l, 26) as usize != n
            || local + 30 + n as u64 + u16at(&l, 28) + packed > offset
        {
            return Err(invalid());
        }
        let mut lname = vec![0; n];
        f.read_exact(&mut lname).map_err(|e| e.to_string())?;
        if lname != name {
            return Err(invalid());
        }
        safe_extra(&mut f, u16at(&l, 28) as usize)?;
        if flags & 8 == 0 && (u32at(&l, 18) != packed || u32at(&l, 22) != bytes) {
            return Err(invalid());
        }
        total = total.checked_add(bytes).ok_or_else(invalid)?;
        compressed = compressed.checked_add(packed).ok_or_else(invalid)?;
        if total > limits.total {
            return Err(invalid());
        }
    }
    if pos != offset + size || total > compressed.saturating_mul(limits.ratio) {
        return Err(invalid());
    }
    Ok(total)
}

/// Polling detects byte/member overruns but cannot prevent transient writes
/// between polls or stop a blocked filesystem syscall. This is not a disk quota.
fn verify_extracted(root: &Path, limits: ArchiveLimits, extracting: bool, is_cancelled: &dyn Fn() -> bool) -> Result<(), String> {
    // ditto consumes and removes temporary AppleDouble files while restoring
    // metadata. A polling scan may race with those removals; the final scan
    // remains strict once the extractor has exited.
    fn observed<T>(result: io::Result<T>, extracting: bool) -> Result<Option<T>, String> {
        match result {
            Ok(value) => Ok(Some(value)),
            Err(e) if extracting && e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }
    let mut pending = vec![root.to_path_buf()];
    let mut entries = 0;
    let mut total = 0u64;
    while let Some(dir) = pending.pop() {
        let Some(children) = observed(fs::read_dir(dir), extracting)? else {
            continue;
        };
        for entry in children {
            check_cancel(is_cancelled)?;
            let Some(entry) = observed(entry, extracting)? else {
                continue;
            };
            let p = entry.path();
            let Some(m) = observed(fs::symlink_metadata(&p), extracting)? else {
                continue;
            };
            entries += 1;
            if entries > limits.entries || m.file_type().is_symlink() || (!m.is_file() && !m.is_dir()) {
                return Err("Unsafe extracted member or entry limit exceeded".into());
            }
            let Some(canonical) = observed(fs::canonicalize(&p), extracting)? else {
                continue;
            };
            if !canonical.starts_with(root) {
                return Err("Unsafe extracted member or entry limit exceeded".into());
            }
            if m.is_dir() {
                pending.push(p);
            } else {
                total = total.saturating_add(m.len());
                if m.len() > limits.file || total > limits.total {
                    return Err("Extracted byte limit exceeded".into());
                }
            }
        }
    }
    Ok(())
}
fn extract_archive(path: &Path, destination: &Path, limits: ArchiveLimits, is_cancelled: &dyn Fn() -> bool) -> Publication {
    use std::os::unix::fs::OpenOptionsExt;
    let stage = fsx::Staging::new(destination)?;
    let result = (|| {
        let snapshot = stage.dir.join("archive.zip");
        // O_NOFOLLOW on the input's final component; ancestors remain path based.
        let mut input = fs::OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW).open(path).map_err(|e| e.to_string())?;
        let meta = input.metadata().map_err(|e| e.to_string())?;
        let archive_bytes = meta.len();
        if !meta.is_file() || archive_bytes > limits.total.saturating_add(64 * 1024 * 1024) {
            return Err("ZIP input is not a regular file or exceeds snapshot budget".into());
        }
        let space = fsx::available_space(&stage.dir)?;
        if archive_bytes.saturating_add(limits.margin) > space {
            return Err("Insufficient free disk space for ZIP snapshot".into());
        }
        let mut output = fs::OpenOptions::new().write(true).create_new(true).open(&snapshot).map_err(|e| e.to_string())?;
        use std::io::Write;
        let snapshot_started = std::time::Instant::now();
        let mut remaining = archive_bytes;
        let mut buffer = [0; 65536];
        while remaining > 0 {
            check_cancel(is_cancelled)?;
            if snapshot_started.elapsed() > std::time::Duration::from_secs(60) {
                return Err("ZIP snapshot timed out".into());
            }
            let count = input.read(&mut buffer[..remaining.min(65536) as usize]).map_err(|e| e.to_string())?;
            if count == 0 {
                return Err("ZIP input changed while snapshotting".into());
            }
            output.write_all(&buffer[..count]).map_err(|e| e.to_string())?;
            remaining -= count as u64;
        }
        drop(output);
        let total = inspect_archive(&snapshot, limits, is_cancelled)?;
        // Available space already excludes the allocated archive snapshot. Thus
        // this reserves snapshot + extraction + margin across the two checks.
        if total.saturating_add(limits.margin) > fsx::available_space(&stage.dir)? {
            return Err("Insufficient free disk space for extraction".into());
        }
        // ditto restores AppleDouble metadata but can report success for a bad
        // member CRC. Test every member without writing files before extraction.
        // Both decompression passes share the extraction time budget.
        let extraction_started = std::time::Instant::now();
        let extraction_budget = std::time::Duration::from_secs(60);
        check_cancel(is_cancelled)?;
        // Both tools write only to private staging, so stopping them is safe.
        bounded_tool("/usr/bin/unzip", &["-tqq".as_ref(), snapshot.as_os_str()], extraction_budget, || check_cancel(is_cancelled))?;
        check_cancel(is_cancelled)?;
        fs::create_dir(&stage.path).map_err(|e| e.to_string())?;
        let observed = ArchiveLimits { total: total.min(limits.total), ..limits };
        bounded_tool(
            "/usr/bin/ditto",
            &["-x".as_ref(), "-k".as_ref(), snapshot.as_os_str(), stage.path.as_os_str()],
            extraction_budget.saturating_sub(extraction_started.elapsed()),
            || {
                check_cancel(is_cancelled)?;
                verify_extracted(&stage.path, observed, true, &never_cancel)?;
                if fsx::available_space(&stage.dir)? < limits.margin {
                    return Err("Extraction disk-space reserve exhausted".into());
                }
                Ok(())
            },
        )?;
        check_cancel(is_cancelled)?;
        verify_extracted(&stage.path, observed, false, is_cancelled)?;
        // Release the snapshot before making the extracted payload visible.
        fs::remove_file(&snapshot).map_err(|e| e.to_string())?;
        check_cancel(is_cancelled)?;
        Ok(())
    })();
    stage.finish(result.map_err(PublishError::Clean).and_then(|()| stage.publish(destination)))
}

/// The fallback tool may have copied or removed files before returning an
/// error. Its output must be reviewed whenever success cannot be confirmed.
fn mv_no_clobber(from: &Path, to: &Path) -> Publication {
    mv_no_clobber_with(from, to, || {})
}

fn mv_no_clobber_with(from: &Path, to: &Path, before_mv: impl FnOnce()) -> Publication {
    fresh(to)?;
    let source_meta = fs::symlink_metadata(from).map_err(|e| PublishError::Clean(io_err("move", from)(e)))?;
    let source_had_nested = source_meta.is_dir() && from.join(file_name(from)).symlink_metadata().is_ok();
    before_mv();
    tool("/bin/mv", &["-n".as_ref(), from.as_os_str(), to.as_os_str()]).map_err(PublishError::Uncertain)?;
    if !matches!(fs::symlink_metadata(from), Err(e) if e.kind() == io::ErrorKind::NotFound) {
        return Err(PublishError::Uncertain(format!("Couldn't confirm that {} moved to {}.", from.display(), to.display())));
    }
    let dest_meta = fs::symlink_metadata(to).map_err(|e| PublishError::Uncertain(io_err("check moved output", to)(e)))?;
    let matches_source = if source_meta.is_file() {
        dest_meta.is_file() && dest_meta.len() == source_meta.len()
    } else if source_meta.file_type().is_symlink() {
        dest_meta.file_type().is_symlink() && dest_meta.len() == source_meta.len()
    } else if source_meta.is_dir() {
        dest_meta.is_dir() && (source_had_nested || to.join(file_name(from)).symlink_metadata().is_err())
    } else {
        dest_meta.file_type() == source_meta.file_type()
    };
    if !matches_source {
        return Err(PublishError::Uncertain(format!(
            "Couldn't confirm that {} landed directly at {} rather than inside another directory.",
            from.display(),
            to.display()
        )));
    }
    Ok(FileIdentity::from_metadata(&dest_meta))
}

fn move_path(from: &Path, to: &Path) -> Publication {
    let source_parent = fs::canonicalize(from.parent().ok_or_else(|| "Missing source parent".to_string())?).map_err(|e| e.to_string())?;
    fsx::revalidate_parent(from, &source_parent)?;
    match fsx::rename_exclusive(from, to) {
        Err(PublishError::CrossDevice(_)) => mv_no_clobber(from, to),
        result => result,
    }
}

fn copy_path(from: &Path, to: &Path) -> Publication {
    fresh(to)?;
    fsx::staged_output(to, |out| {
        if fs::symlink_metadata(from).map_err(io_err("copy", from))?.is_symlink() {
            let target = fs::read_link(from).map_err(io_err("read link", from))?;
            std::os::unix::fs::symlink(target, out).map_err(io_err("copy link", from))
        } else {
            tool("/usr/bin/ditto", &[from.as_os_str(), out.as_os_str()])
        }
    })
}

/// Undoing something Voxi created puts it in the Trash.
fn trash_inverse(p: &Path) -> Action {
    Action::Trash { path: p.to_path_buf() }
}

fn partial(effect: Effect, message: String) -> StepError {
    StepError::Partial { message, effect: Box::new(effect) }
}

fn published_effect(
    result: Publication,
    complete: impl FnOnce(FileIdentity) -> Result<Effect, StepError>,
    incomplete: impl FnOnce(FileIdentity) -> Result<Effect, StepError>,
) -> Result<Effect, StepError> {
    match result {
        Ok(identity) => complete(identity),
        Err(PublishError::Published { identity, message }) => Err(partial(incomplete(identity)?, message)),
        Err(PublishError::Clean(message) | PublishError::CrossDevice(message)) => Err(message.into()),
        Err(PublishError::Uncertain(message)) => Err(StepError::Uncertain(message)),
    }
}

fn created(a: &Action, output: &Path, result: Publication) -> Result<Effect, StepError> {
    let effect = |identity| Effect::reversible(a.clone(), trash_inverse(output), &[identity]);
    published_effect(result, effect, effect)
}

fn moved(a: &Action, from: &Path, to: &Path, inverse: Action) -> Result<Effect, StepError> {
    published_effect(
        move_path(from, to),
        |identity| Effect::reversible(a.clone(), inverse, &[identity]),
        |identity| {
            let copy = Action::Copy { from: from.to_path_buf(), to: to.to_path_buf() };
            Effect::reversible(copy, trash_inverse(to), &[identity])
        },
    )
}

fn trash_with_identity(path: &Path, roots: &Roots) -> Result<(PathBuf, FileIdentity), StepError> {
    let original = FileIdentity::read(path).map_err(io_err("check before moving to Trash", path))?;
    let landed = trash::to_trash(path, &roots.home).map_err(StepError::Uncertain)?;
    let identity = FileIdentity::read(&landed).map_err(|error| StepError::Uncertain(io_err("check Trash", &landed)(error)))?;
    if identity != original {
        return Err(StepError::Uncertain(format!("Couldn't verify the file moved to {}.", landed.display())));
    }
    Ok((landed, identity))
}

fn resize(a: &Action, path: &Path, backup: &Path, width: u32, roots: &Roots) -> Result<Effect, StepError> {
    let original = FileIdentity::read(path).map_err(io_err("check before resizing", path))?;
    if let Some(dir) = backup.parent() {
        fs::create_dir_all(dir).map_err(io_err("create", dir))?;
    }
    let copied = Action::Copy { from: path.to_path_buf(), to: backup.to_path_buf() };
    let backup_id = match copy_path(path, backup) {
        Ok(identity) => identity,
        result => return created(&copied, backup, result),
    };
    let resize = (|| -> Publication {
        let stage = fsx::Staging::new(path)?;
        let prepared = (|| {
            tool("/usr/bin/ditto", &[backup.as_os_str(), stage.path.as_os_str()])?;
            tool("/usr/bin/sips", &["--resampleWidth".as_ref(), width.to_string().as_ref(), stage.path.as_os_str()])?;
            guard(roots, &[path])
        })();
        stage.finish(prepared.map_err(PublishError::Clean).and_then(|()| stage.replace(path, &original)))
    })();
    let resized = |identity| {
        Effect::reversible(
            a.clone(),
            Action::Restore { from: backup.to_path_buf(), to: path.to_path_buf() },
            &[backup_id.clone(), identity],
        )
    };
    match resize {
        Err(PublishError::Clean(message)) => {
            let effect = Effect::reversible(copied, trash_inverse(backup), &[backup_id])?;
            Err(partial(effect, format!("{message}; original backup retained at {}", backup.display())))
        }
        result => published_effect(result, resized, resized),
    }
}

fn restore(a: &Action, from: &Path, to: &Path, roots: &Roots) -> Result<Effect, StepError> {
    fs::symlink_metadata(from).map_err(io_err("restore", from))?;
    let trashed = match fs::symlink_metadata(to) {
        Ok(_) => {
            let (landed, identity) = trash_with_identity(to, roots)?;
            Some(
                Inverse::capture(Action::RestoreFromTrash { from: landed, to: to.to_path_buf() }, &[identity])
                    .map_err(StepError::Uncertain)?,
            )
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => return Err(io_err("check restore destination", to)(e).into()),
    };
    match move_path(from, to) {
        Ok(identity) => Effect::reversible(a.clone(), trash_inverse(to), &[identity]),
        Err(PublishError::Clean(message) | PublishError::CrossDevice(message)) => match trashed {
            Some(put_back) => Err(partial(Effect { action: trash_inverse(to), inverse: vec![put_back] }, message)),
            None => Err(message.into()),
        },
        Err(PublishError::Published { identity, message }) => {
            let mut effect =
                Effect::reversible(Action::Copy { from: from.to_path_buf(), to: to.to_path_buf() }, trash_inverse(to), &[identity])?;
            effect.inverse.extend(trashed);
            Err(partial(effect, message))
        }
        Err(PublishError::Uncertain(message)) => Err(StepError::Uncertain(message)),
    }
}

/// Runs one step and captures the identities of the files its inverse can touch.
pub(crate) fn step(a: &Action, roots: &Roots, is_cancelled: &dyn Fn() -> bool) -> Result<Effect, StepError> {
    check_cancel(is_cancelled)?;
    match a {
        Action::Move { from, to } => {
            guard(roots, &[from, to])?;
            moved(a, from, to, Action::Move { from: to.clone(), to: from.clone() })
        }
        Action::Copy { from, to } => {
            guard(roots, &[from, to])?;
            created(a, to, copy_path(from, to))
        }
        Action::Rename { path, new_name } => {
            let to = path.with_file_name(new_name);
            guard(roots, &[path, &to])?;
            if path == &to {
                return Ok(Effect::irreversible(a));
            }
            moved(a, path, &to, Action::Rename { path: to.clone(), new_name: file_name(path) })
        }
        Action::Trash { path } => {
            guard(roots, &[path])?;
            let (landed, identity) = trash_with_identity(path, roots)?;
            Effect::reversible(a.clone(), Action::RestoreFromTrash { from: landed, to: path.clone() }, &[identity])
        }
        Action::RestoreFromTrash { from, to } => {
            guard(roots, &[to])?;
            if !trash::is_in_trash(from) {
                return Err(format!("Stopped: {} isn't in the Trash.", from.display()).into());
            }
            moved(a, from, to, trash_inverse(to))
        }
        Action::CreateFolder { path } => {
            guard(roots, &[path])?;
            match fs::create_dir(path) {
                Ok(()) => {
                    let identity = FileIdentity::read(path).map_err(|e| StepError::Uncertain(io_err("check created folder", path)(e)))?;
                    Effect::reversible(a.clone(), Action::RemoveEmptyFolder { path: path.clone() }, &[identity])
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists && path.is_dir() => Ok(Effect::irreversible(a)),
                Err(e) => Err(io_err("create", path)(e).into()),
            }
        }
        Action::RemoveEmptyFolder { path } => {
            guard(roots, &[path])?;
            match fs::remove_dir(path) {
                Ok(()) => Ok(Effect::irreversible(a)),
                Err(e) if e.kind() == io::ErrorKind::DirectoryNotEmpty => Ok(Effect::irreversible(a)),
                Err(e) => Err(io_err("remove empty folder", path)(e).into()),
            }
        }
        Action::ConvertImage { path, format, output } => {
            guard(roots, &[path, output])?;
            created(
                a,
                output,
                fsx::staged_output(output, |out| {
                    tool(
                        "/usr/bin/sips",
                        &[
                            "-s".as_ref(),
                            "format".as_ref(),
                            format.sips_name().as_ref(),
                            path.as_os_str(),
                            "--out".as_ref(),
                            out.as_os_str(),
                        ],
                    )
                }),
            )
        }
        Action::ResizeImage { path, max_width_px, backup } => {
            guard(roots, &[path, backup])?;
            resize(a, path, backup, *max_width_px, roots)
        }
        Action::Restore { from, to } => {
            guard(roots, &[from, to])?;
            restore(a, from, to, roots)
        }
        Action::MergePdf { paths, output } => {
            guard(roots, &[output])?;
            guard(roots, &paths.iter().map(PathBuf::as_path).collect::<Vec<_>>())?;
            fresh(output)?;
            created(
                a,
                output,
                fsx::staged_output(output, |out| {
                    pdf::merge(paths, out, &|| check_cancel(is_cancelled))?;
                    check_cancel(is_cancelled)
                }),
            )
        }
        Action::ImagesToPdf { paths, output } => {
            guard(roots, &[output])?;
            guard(roots, &paths.iter().map(PathBuf::as_path).collect::<Vec<_>>())?;
            fresh(output)?;
            created(
                a,
                output,
                fsx::staged_output(output, |out| {
                    pdf::images_to_pdf(paths, out, &|| check_cancel(is_cancelled))?;
                    check_cancel(is_cancelled)
                }),
            )
        }
        Action::SplitPdf { path, output_dir } => {
            guard(roots, &[path, output_dir])?;
            fresh(output_dir)?;
            created(
                a,
                output_dir,
                fsx::staged_output(output_dir, |out| {
                    fs::create_dir(out).map_err(|e| e.to_string())?;
                    pdf::split(path, out, &|| check_cancel(is_cancelled)).map(|_| ())
                }),
            )
        }
        Action::ExtractPages { path, from, to, output } => {
            guard(roots, &[path, output])?;
            fresh(output)?;
            created(a, output, fsx::staged_output(output, |out| pdf::extract(path, *from, *to, out)))
        }
        Action::Zip { path, output } => {
            guard(roots, &[path, output])?;
            fresh(output)?;
            created(
                a,
                output,
                fsx::staged_output(output, |out| {
                    tool(
                        "/usr/bin/ditto",
                        &[
                            "-c".as_ref(),
                            "-k".as_ref(),
                            "--sequesterRsrc".as_ref(),
                            "--keepParent".as_ref(),
                            path.as_os_str(),
                            out.as_os_str(),
                        ],
                    )
                }),
            )
        }
        Action::Unzip { path, destination } => {
            guard(roots, &[path, destination])?;
            fresh(destination)?;
            created(a, destination, extract_archive(path, destination, ArchiveLimits::default(), is_cancelled))
        }
        Action::OpenInFinder { path, reveal } => {
            guard_open(roots, path)?;
            let mut args: Vec<&std::ffi::OsStr> = Vec::new();
            if *reveal {
                args.push("-R".as_ref());
            }
            args.push(path.as_os_str());
            tool("/usr/bin/open", &args)?;
            Ok(Effect::irreversible(a))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::History;
    use crate::parse::parse;
    use crate::plan::{Planned, Planner};
    use std::collections::HashMap;

    fn setup() -> (tempfile::TempDir, Roots) {
        let dir = tempfile::tempdir().unwrap();
        let h = fsx::canonical(dir.path());
        for d in ["Desktop", "Downloads", "Documents", "Pictures"] {
            fs::create_dir(h.join(d)).unwrap();
        }
        let roots = Roots::new(&h, None, &[]);
        (dir, roots)
    }

    fn plan_and_run(roots: &Roots, history: &History, cmd: &str) -> Outcome {
        let choices = HashMap::new();
        let Ok(Planned::Steps { actions, .. }) = Planner::new(roots, &choices).plan(&parse(cmd).unwrap(), history) else {
            panic!("no plan for {cmd}")
        };
        run(&actions, roots)
    }

    #[test]
    fn restore_missing_backup_preserves_current_file() {
        let (_dir, roots) = setup();
        let current = roots.home.join("Documents/current");
        fs::write(&current, b"modified version").unwrap();
        let out = run(&[Action::Restore { from: roots.home.join("Documents/missing"), to: current.clone() }], &roots);
        assert!(out.terminal.error().is_some());
        assert!(out.done.is_empty());
        assert_eq!(fs::read(current).unwrap(), b"modified version");
    }

    #[test]
    fn rename_and_restore_reject_destinations_created_after_preview() {
        let (_d, roots) = setup();
        let from = roots.home.join("Documents/source");
        let to = roots.home.join("Documents/target");
        for directory in [false, true] {
            if directory {
                fs::create_dir(&from).unwrap();
            } else {
                fs::write(&from, b"source").unwrap();
            }
            let action = Action::Rename { path: from.clone(), new_name: "target".into() };
            fs::write(&to, b"concurrent creator").unwrap();
            assert!(run(&[action], &roots).terminal.error().is_some());
            assert_eq!(fs::read(&to).unwrap(), b"concurrent creator");
            fs::remove_file(&to).unwrap();
            let landed = trash::to_trash(&from, &roots.home).unwrap();
            let action = Action::RestoreFromTrash { from: landed.clone(), to: to.clone() };
            fs::write(&to, b"concurrent creator").unwrap();
            assert!(run(&[action], &roots).terminal.error().is_some());
            assert!(landed.exists());
            assert_eq!(fs::read(&to).unwrap(), b"concurrent creator");
            fs::remove_file(&to).unwrap();
        }
    }

    #[test]
    fn staged_publication_cannot_clobber_and_cleans_failures() {
        let d = tempfile::tempdir().unwrap();
        let output = d.path().join("out");
        assert!(
            fsx::staged_output(&output, |stage| {
                fs::write(stage, b"generated").unwrap();
                fs::write(&output, b"winner").unwrap();
                Ok(())
            })
            .is_err()
        );
        assert_eq!(fs::read(&output).unwrap(), b"winner");
        assert_eq!(fs::read_dir(d.path()).unwrap().count(), 1);
        fs::remove_file(&output).unwrap();
        assert!(
            fsx::staged_output(&output, |stage| {
                fs::create_dir(stage).unwrap();
                fs::write(stage.join("partial"), b"x").unwrap();
                Err("injected extraction failure".into())
            })
            .is_err()
        );
        assert_eq!(fs::read_dir(d.path()).unwrap().count(), 0);
    }

    #[test]
    fn staged_publication_rejects_parent_swap() {
        let d = tempfile::tempdir().unwrap();
        let parent = d.path().join("parent");
        let moved = d.path().join("moved");
        fs::create_dir(&parent).unwrap();
        let stage = fsx::Staging::new(&parent.join("out")).unwrap();
        fs::write(&stage.path, b"generated").unwrap();
        fs::rename(&parent, &moved).unwrap();
        fs::create_dir(&parent).unwrap();
        // An identity check catches replacement even at the same spelling.
        assert!(stage.publish(&parent.join("out")).is_err());
        fs::remove_dir(&parent).unwrap();
        std::os::unix::fs::symlink(&moved, &parent).unwrap();
        assert!(stage.publish(&parent.join("out")).is_err());
        assert!(!moved.join("out").exists());
    }

    #[test]
    fn steps_never_touch_an_excluded_folder() {
        let d = tempfile::tempdir().unwrap();
        let h = fsx::canonical(d.path());
        fs::create_dir_all(h.join("Documents/Archive")).unwrap();
        fs::write(h.join("Documents/Archive/a.pdf"), b"a").unwrap();
        let roots = Roots::new(&h, None, &[]).with_exclusions(&[h.join("Documents/Archive")]);
        let trash = Action::Trash { path: h.join("Documents/Archive/a.pdf") };
        assert!(matches!(step(&trash, &roots, &never_cancel), Err(StepError::Failed(m)) if m.starts_with("Stopped:")));
        let into = Action::CreateFolder { path: h.join("Documents/Archive/new") };
        assert!(matches!(step(&into, &roots, &never_cancel), Err(StepError::Failed(_))));
        assert!(h.join("Documents/Archive/a.pdf").exists() && !h.join("Documents/Archive/new").exists());
    }

    #[test]
    fn step_reports_cancellation_as_a_variant_and_failures_with_their_message() {
        let d = tempfile::tempdir().unwrap();
        let roots = Roots::new(d.path(), None, &[]);
        let path = d.path().join("Documents/new");
        fs::create_dir(d.path().join("Documents")).unwrap();
        let create = Action::CreateFolder { path: path.clone() };
        assert_eq!(step(&create, &roots, &|| true), Err(StepError::Cancelled));
        assert!(!path.exists());
        let outside = Action::CreateFolder { path: PathBuf::from("/private/var/voxi-outside") };
        assert!(matches!(step(&outside, &roots, &never_cancel), Err(StepError::Failed(m)) if m.starts_with("Stopped:")));
    }

    #[test]
    fn tools_have_deadlines_cancellation_and_bounded_errors() {
        use std::time::{Duration, Instant};
        let start = Instant::now();
        let err = bounded_tool("/bin/sleep", &["2".as_ref()], Duration::from_millis(30), || Ok(())).unwrap_err();
        assert!(err.contains("timed out"));
        assert!(start.elapsed() < Duration::from_secs(1));
        let err = bounded_tool(
            "/bin/sh",
            &["-c".as_ref(), "head -c 200000 /dev/zero; head -c 200000 /dev/zero >&2; exit 1".as_ref()],
            Duration::from_secs(5),
            || Ok(()),
        )
        .unwrap_err();
        assert!(err.len() <= 65536 + 100, "{}", err.len());
        let check = || check_cancel(&|| true);
        assert_eq!(bounded_tool("/bin/sleep", &["2".as_ref()], Duration::from_secs(5), check).unwrap_err(), CANCELLED);
        let stop = std::sync::atomic::AtomicBool::new(false);
        let started = Instant::now();
        assert_eq!(
            bounded_tool("/bin/sleep", &["2".as_ref()], Duration::from_secs(5), || {
                let cancelled = stop.swap(true, std::sync::atomic::Ordering::Relaxed);
                check_cancel(&|| cancelled)
            })
            .unwrap_err(),
            CANCELLED
        );
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn normal_tool_exit_leaves_descendants_and_does_not_wait_for_pipe_eof() {
        use std::time::{Duration, Instant};
        let d = tempfile::tempdir().unwrap();
        let marker = d.path().join("descendant-finished");
        bounded_tool(
            "/bin/sh",
            &["-c".as_ref(), "(/bin/sleep 1; /usr/bin/touch \"$1\") & exit 0".as_ref(), "test".as_ref(), marker.as_os_str()],
            Duration::from_secs(5),
            || Ok(()),
        )
        .unwrap();
        assert!(!marker.exists(), "tool must return while the descendant retains its pipes");
        let start = Instant::now();
        while !marker.exists() && start.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(marker.exists(), "normal exit must not kill the reaped leader's process group");
    }

    // Tiny metadata-only ZIP fixtures: validation must reject before invoking
    // any decompressor. A real system-zip fixture below tests extraction.
    fn zip_fixture(path: &Path, name: &[u8], bytes: u32, packed: u32, mode: u32) {
        let mut local = vec![0u8; 30];
        local[..4].copy_from_slice(b"PK\x03\x04");
        if bytes != packed {
            local[8..10].copy_from_slice(&8u16.to_le_bytes());
        }
        local[18..22].copy_from_slice(&packed.to_le_bytes());
        local[22..26].copy_from_slice(&bytes.to_le_bytes());
        local[26..28].copy_from_slice(&(name.len() as u16).to_le_bytes());
        let mut data = local;
        data.extend_from_slice(name);
        data.resize(data.len() + packed as usize, 0);
        let offset = data.len() as u32;
        let mut central = vec![0u8; 46];
        central[..4].copy_from_slice(b"PK\x01\x02");
        if bytes != packed {
            central[10..12].copy_from_slice(&8u16.to_le_bytes());
        }
        central[20..24].copy_from_slice(&packed.to_le_bytes());
        central[24..28].copy_from_slice(&bytes.to_le_bytes());
        central[28..30].copy_from_slice(&(name.len() as u16).to_le_bytes());
        central[38..42].copy_from_slice(&(mode << 16).to_le_bytes());
        data.extend(central);
        data.extend_from_slice(name);
        let size = data.len() as u32 - offset;
        let mut end = vec![0u8; 22];
        end[..4].copy_from_slice(b"PK\x05\x06");
        end[8..10].copy_from_slice(&1u16.to_le_bytes());
        end[10..12].copy_from_slice(&1u16.to_le_bytes());
        end[12..16].copy_from_slice(&size.to_le_bytes());
        end[16..20].copy_from_slice(&offset.to_le_bytes());
        data.extend(end);
        fs::write(path, data).unwrap();
    }
    #[test]
    fn hostile_zip_metadata_is_rejected() {
        let d = tempfile::tempdir().unwrap();
        let zip = d.path().join("test.zip");
        let limits = ArchiveLimits::default();
        for name in [b"../escape".as_slice(), b"/absolute", b"a/../../x", b"a\\evil"] {
            zip_fixture(&zip, name, 1, 1, 0o100644);
            assert!(inspect_archive(&zip, limits, &|| false).is_err());
        }
        zip_fixture(&zip, b"link", 1, 1, 0o120777);
        assert!(inspect_archive(&zip, limits, &|| false).is_err());
        zip_fixture(&zip, b"ok", 101, 1, 0o100644);
        assert!(inspect_archive(&zip, limits, &|| false).is_err());
        zip_fixture(&zip, b"ok", 2, 2, 0o100644);
        assert_eq!(inspect_archive(&zip, limits, &|| false).unwrap(), 2);
        assert!(inspect_archive(&zip, ArchiveLimits { entries: 0, ..limits }, &|| false).is_err());
        assert!(inspect_archive(&zip, ArchiveLimits { file: 1, ..limits }, &|| false).is_err());
        assert!(inspect_archive(&zip, ArchiveLimits { total: 1, ..limits }, &|| false).is_err());
    }
    #[test]
    fn extraction_polling_tolerates_removed_directories_but_final_scan_is_strict() {
        let d = tempfile::tempdir().unwrap();
        let temporary = d.path().join("__MACOSX");
        fs::create_dir(&temporary).unwrap();
        fs::remove_dir(&temporary).unwrap();
        let limits = ArchiveLimits::default();
        // Model a metadata directory removed after it was queued for scanning.
        assert!(verify_extracted(&temporary, limits, true, &never_cancel).is_ok());
        assert!(verify_extracted(&temporary, limits, false, &never_cancel).is_err());
        // Polling still rejects unsafe members and enforces observed limits.
        let link = d.path().join("link");
        std::os::unix::fs::symlink(d.path(), &link).unwrap();
        assert!(verify_extracted(d.path(), limits, true, &never_cancel).is_err());
        fs::remove_file(link).unwrap();
        fs::write(d.path().join("file"), b"too large").unwrap();
        assert!(verify_extracted(d.path(), ArchiveLimits { file: 1, ..limits }, true, &never_cancel).is_err());
    }

    #[test]
    fn system_zip_limits_and_disk_margin() {
        let d = tempfile::tempdir().unwrap();
        let input = d.path().join("input");
        fs::write(&input, b"hello archive").unwrap();
        let zip = d.path().join("a.zip");
        tool("/usr/bin/zip", &["-j".as_ref(), zip.as_os_str(), input.as_os_str()]).unwrap();
        let destination = d.path().join("out");
        let zip_link = d.path().join("archive-link.zip");
        std::os::unix::fs::symlink(&zip, &zip_link).unwrap();
        assert!(extract_archive(&zip_link, &destination, ArchiveLimits::default(), &|| false).is_err());
        assert!(!destination.exists());
        assert!(extract_archive(&zip, &destination, ArchiveLimits { margin: u64::MAX, ..ArchiveLimits::default() }, &|| false).is_err());
        assert!(!destination.exists());
        extract_archive(&zip, &destination, ArchiveLimits::default(), &|| false).unwrap();
        assert_eq!(fs::read(destination.join("input")).unwrap(), b"hello archive");
        let link = d.path().join("link");
        std::os::unix::fs::symlink(&input, &link).unwrap();
        let hostile = d.path().join("links.zip");
        tool("/usr/bin/zip", &["-y".as_ref(), "-j".as_ref(), hostile.as_os_str(), link.as_os_str()]).unwrap();
        let rejected = d.path().join("rejected");
        assert!(extract_archive(&hostile, &rejected, ArchiveLimits::default(), &|| false).is_err());
        assert!(!rejected.exists());
        // ditto alone accepts this bad CRC: integrity validation must reject it.
        zip_fixture(&hostile, b"bad-crc", 2, 2, 0o100644);
        let Err(PublishError::Clean(err)) = extract_archive(&hostile, &rejected, ArchiveLimits::default(), &|| false) else {
            panic!("expected clean failure")
        };
        assert!(err.contains("/usr/bin/unzip failed"), "{err}");
        assert!(!rejected.exists());
        assert!(!fs::read_dir(d.path()).unwrap().any(|e| e.unwrap().file_name().to_string_lossy().starts_with(".voxi-stage")));
    }

    #[test]
    fn move_then_undo_restores_everything() {
        let (_d, roots) = setup();
        let h = &roots.home;
        fs::write(h.join("Downloads/a.pdf"), b"a").unwrap();
        fs::write(h.join("Downloads/b.pdf"), b"b").unwrap();
        fs::create_dir_all(h.join("Downloads/nested/deep")).unwrap();
        fs::write(h.join("Downloads/nested/deep/a.pdf"), b"nested").unwrap();
        let mut history = History::load(h.join("history.json"));

        let out = plan_and_run(&roots, &history, "move PDFs from Downloads to Archive");
        assert!(out.terminal.error().is_none());
        assert!(h.join("Downloads/Archive/a.pdf").exists());
        assert!(!h.join("Downloads/Archive/a 2.pdf").exists(), "files in subfolders stay put");
        history.push(crate::history::Entry {
            done: out.done,
            state: crate::history::EntryState::with_inverses(out.inverse),
            ..crate::history::Entry::new(1, "move".into(), None)
        });

        let undo = plan_and_run(&roots, &history, "undo last");
        assert!(undo.terminal.error().is_none(), "{:?}", undo.terminal.error());
        assert!(h.join("Downloads/a.pdf").exists());
        assert!(h.join("Downloads/b.pdf").exists());
        assert_eq!(fs::read(h.join("Downloads/nested/deep/a.pdf")).unwrap(), b"nested");
        assert!(!h.join("Downloads/Archive").exists(), "the folder Voxi created is removed once empty");
    }

    #[test]
    fn delete_never_removes_data() {
        let (_d, roots) = setup();
        let h = &roots.home;
        fs::write(h.join("Downloads/setup.dmg"), b"x").unwrap();
        let history = History::load(h.join("history.json"));
        let out = plan_and_run(&roots, &history, "delete installers from Downloads");
        assert!(out.terminal.error().is_none());
        assert!(!h.join("Downloads/setup.dmg").exists());
        assert_eq!(fs::read(h.join(".Trash/setup.dmg")).unwrap(), b"x", "in the (stand-in) Trash, not deleted");
        // And undo takes it back out.
        let back = run(&out.inverse.into_iter().map(|inverse| inverse.action).collect::<Vec<_>>(), &roots);
        assert!(back.terminal.error().is_none(), "{:?}", back.terminal.error());
        assert_eq!(fs::read(h.join("Downloads/setup.dmg")).unwrap(), b"x");
    }

    #[test]
    fn existing_target_is_never_overwritten() {
        let (_d, roots) = setup();
        let h = &roots.home;
        let to = h.join("Documents/a.txt");
        fs::write(h.join("Downloads/a.txt"), b"new").unwrap();
        fs::write(&to, b"old").unwrap();
        let out = run(&[Action::Move { from: h.join("Downloads/a.txt"), to: to.clone() }], &roots);
        assert!(out.terminal.error().is_some());
        assert_eq!(fs::read(&to).unwrap(), b"old");
    }

    #[test]
    fn copy_never_overwrites_an_existing_file() {
        let (_d, roots) = setup();
        let h = &roots.home;
        let to = h.join("Documents/a.txt");
        fs::write(h.join("Downloads/a.txt"), b"new").unwrap();
        fs::write(&to, b"old").unwrap();
        let out = run(&[Action::Copy { from: h.join("Downloads/a.txt"), to: to.clone() }], &roots);
        assert!(out.terminal.error().is_some());
        assert_eq!(fs::read(&to).unwrap(), b"old");
    }

    #[test]
    fn moving_a_folder_never_overwrites_an_existing_one() {
        let (_d, roots) = setup();
        let h = &roots.home;
        fs::create_dir_all(h.join("Downloads/Project/sub")).unwrap();
        fs::write(h.join("Downloads/Project/sub/a.txt"), b"new").unwrap();
        let from = h.join("Downloads/Project");
        let to = h.join("Documents/Project");
        let out = run(&[Action::Move { from: from.clone(), to: to.clone() }], &roots);
        assert!(out.terminal.error().is_none(), "{:?}", out.terminal.error());
        assert_eq!(fs::read(to.join("sub/a.txt")).unwrap(), b"new");
        assert!(!from.exists());

        fs::create_dir_all(h.join("Downloads/Other")).unwrap();
        fs::create_dir_all(h.join("Documents/Other")).unwrap();
        fs::write(h.join("Downloads/Other/b.txt"), b"old").unwrap();
        fs::write(h.join("Documents/Other/b.txt"), b"keep").unwrap();
        let out = run(&[Action::Move { from: h.join("Downloads/Other"), to: h.join("Documents/Other") }], &roots);
        assert!(out.terminal.error().is_some());
        assert_eq!(fs::read(h.join("Documents/Other/b.txt")).unwrap(), b"keep");
    }

    #[test]
    fn move_tool_fallback_moves_without_clobbering() {
        let (_d, roots) = setup();
        let h = &roots.home;
        let from = h.join("Downloads/move-me.txt");
        let to = h.join("Documents/move-me.txt");
        fs::write(&from, b"contents").unwrap();
        mv_no_clobber(&from, &to).unwrap();
        assert!(!from.exists());
        assert_eq!(fs::read(&to).unwrap(), b"contents");

        // The fallback still refuses to overwrite an existing destination.
        fs::write(&from, b"new").unwrap();
        let err = mv_no_clobber(&from, &to).unwrap_err();
        assert!(matches!(err, PublishError::Clean(message) if message.contains("Stopped")));
        assert_eq!(fs::read(&to).unwrap(), b"contents");
        assert!(from.exists());
    }

    #[test]
    fn cross_volume_directory_fallback_moves_without_clobbering() {
        // The EXDEV branch of move_path drops its claimed folder and uses
        // this helper for the directory, so it must preserve contents and
        // leave an existing destination alone.
        let (_d, roots) = setup();
        let h = &roots.home;
        let from = h.join("Downloads/folder");
        let to = h.join("Documents/folder");
        fs::create_dir(&from).unwrap();
        fs::write(from.join("a.txt"), b"a").unwrap();
        mv_no_clobber(&from, &to).unwrap();
        assert!(!from.exists());
        assert_eq!(fs::read(to.join("a.txt")).unwrap(), b"a");

        fs::create_dir(&from).unwrap();
        fs::write(from.join("b.txt"), b"b").unwrap();
        assert!(mv_no_clobber(&from, &to).is_err());
        assert!(!to.join("b.txt").exists(), "nothing was merged into the destination");
        assert_eq!(fs::read(to.join("a.txt")).unwrap(), b"a");
        assert!(from.exists());
    }

    #[test]
    fn moving_and_copying_a_symlink_to_a_folder_moves_the_link() {
        let (_d, roots) = setup();
        let h = &roots.home;
        let target = h.join("Documents/target");
        fs::create_dir(&target).unwrap();
        fs::write(target.join("keep.txt"), b"keep").unwrap();
        let link = h.join("Downloads/link");
        std::os::unix::fs::symlink(&target, &link).unwrap();

        // A symlink to a folder must take the file path, not create_dir +
        // rename, so the link itself is moved.
        let to = h.join("Documents/link");
        let out = run(&[Action::Move { from: link.clone(), to: to.clone() }], &roots);
        assert!(out.terminal.error().is_none(), "{:?}", out.terminal.error());
        assert!(!link.exists());
        assert_eq!(fs::read_link(&to).unwrap(), target, "the link itself moved, not the folder");
        assert_eq!(fs::read(to.join("keep.txt")).unwrap(), b"keep");

        // Copy the link itself, without letting ditto follow it.
        let copy = h.join("Documents/link-copy");
        let out = run(&[Action::Copy { from: to.clone(), to: copy.clone() }], &roots);
        assert!(out.terminal.error().is_none(), "{:?}", out.terminal);
        assert_eq!(fs::read_link(&copy).unwrap(), target);
    }

    #[test]
    fn zip_unzip_round_trip() {
        let (_d, roots) = setup();
        let h = &roots.home;
        fs::create_dir(h.join("Documents/Project")).unwrap();
        fs::write(h.join("Documents/Project/readme.md"), b"hi").unwrap();
        // Force ditto to create AppleDouble entries, even on a clean fixture.
        tool(
            "/usr/bin/xattr",
            &["-w".as_ref(), "com.voxi.roundtrip".as_ref(), "metadata".as_ref(), h.join("Documents/Project/readme.md").as_os_str()],
        )
        .unwrap();
        let history = History::load(h.join("history.json"));
        assert!(plan_and_run(&roots, &history, "zip Project into Project.zip").terminal.error().is_none());
        assert!(h.join("Documents/Project.zip").exists());
        let archive = h.join("Documents/Project.zip");
        let listing = Command::new("/usr/bin/unzip").args(["-Z1"]).arg(&archive).output().unwrap();
        assert!(listing.status.success());
        assert!(String::from_utf8_lossy(&listing.stdout).contains("__MACOSX/"));
        assert!(
            inspect_archive(&archive, ArchiveLimits { entries: 2, ..ArchiveLimits::default() }, &|| false).is_err(),
            "metadata entries count toward archive-wide limits"
        );
        let extracted = plan_and_run(&roots, &history, "unzip Project.zip into Documents/out");
        assert!(extracted.terminal.error().is_none(), "{:?}", extracted.terminal.error());
        assert_eq!(fs::read(h.join("Documents/out/Project/readme.md")).unwrap(), b"hi");
        assert!(!h.join("Documents/out/__MACOSX").exists());
        assert!(!h.join("Documents/out/archive.zip").exists());
        let attr = Command::new("/usr/bin/xattr")
            .args(["-p", "com.voxi.roundtrip"])
            .arg(h.join("Documents/out/Project/readme.md"))
            .output()
            .unwrap();
        assert!(attr.status.success());
        assert_eq!(String::from_utf8_lossy(&attr.stdout).trim(), "metadata");
        assert!(!fs::read_dir(h.join("Documents")).unwrap().any(|e| e.unwrap().file_name().to_string_lossy().starts_with(".voxi-stage")));
    }

    #[test]
    fn image_and_pdf_tools_on_sample_files() {
        let (_d, roots) = setup();
        let h = &roots.home;
        let pics = h.join("Pictures");
        // A 64x32 PNG made with sips from a generated TIFF-free source.
        tool(
            "/usr/bin/sips",
            &[
                "-s".as_ref(),
                "format".as_ref(),
                "png".as_ref(),
                "/System/Library/CoreServices/CoreTypes.bundle/Contents/Resources/GenericDocumentIcon.icns".as_ref(),
                "--out".as_ref(),
                pics.join("icon.png").as_os_str(),
            ],
        )
        .unwrap();
        let history = History::load(h.join("history.json"));

        assert!(plan_and_run(&roots, &history, "convert icon.png to jpg").terminal.error().is_none());
        assert!(pics.join("icon.jpg").exists());

        assert!(plan_and_run(&roots, &history, "resize icon.png to 16 pixels wide").terminal.error().is_none());
        let w = Command::new("/usr/bin/sips").args(["-g", "pixelWidth"]).arg(pics.join("icon.png")).output().unwrap();
        assert!(String::from_utf8_lossy(&w.stdout).contains("pixelWidth: 16"));

        assert!(plan_and_run(&roots, &history, "make a pdf from images in Pictures named icon").terminal.error().is_none());
        assert_eq!(pdf::page_count(&pics.join("icon.pdf")).unwrap(), 2);

        fs::copy(pics.join("icon.pdf"), pics.join("two.pdf")).unwrap();
        assert!(plan_and_run(&roots, &history, "merge PDFs in Pictures into all.pdf").terminal.error().is_none());
        assert_eq!(pdf::page_count(&pics.join("all.pdf")).unwrap(), 4);

        assert!(plan_and_run(&roots, &history, "extract pages 2 to 3 from all.pdf into mid.pdf").terminal.error().is_none());
        assert_eq!(pdf::page_count(&pics.join("mid.pdf")).unwrap(), 2);

        assert!(plan_and_run(&roots, &history, "split all.pdf into pages").terminal.error().is_none());
        assert_eq!(fs::read_dir(pics.join("all pages")).unwrap().count(), 4);
    }

    #[test]
    fn opening_an_allowed_root_is_allowed_but_destructive_guards_are_unchanged() {
        let (_dir, roots) = setup();
        let documents = roots.home.join("Documents");
        // A root can be opened or listed.
        assert!(guard_open(&roots, &documents).is_ok());
        // The general guard still requires a path strictly inside a root, so
        // moving, renaming or trashing the root itself stays blocked.
        assert!(guard(&roots, &[&documents]).is_err());
        assert!(guard(&roots, &[&roots.home]).is_err());
        assert!(guard_open(&roots, &roots.home).is_err());
        // Ordinary paths behave exactly as before.
        let inside = documents.join("note.txt");
        assert!(guard(&roots, &[&inside]).is_ok());
        assert!(guard_open(&roots, &inside).is_ok());
        assert!(guard_open(&roots, Path::new("/etc/passwd")).is_err());
    }

    #[test]
    fn case_only_rename_step_and_undo_preserve_exact_case() {
        let (_dir, roots) = setup();
        let docs = roots.home.join("Documents");
        let upper = docs.join("Report.pdf");
        fs::write(&upper, b"report").unwrap();
        let entry_names = || -> Vec<String> {
            let mut names: Vec<String> =
                fs::read_dir(&docs).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
            names.sort();
            names
        };

        let out = run(&[Action::Rename { path: upper.clone(), new_name: "report.pdf".into() }], &roots);
        assert!(out.terminal.error().is_none(), "{:?}", out.terminal);
        assert_eq!(entry_names(), vec!["report.pdf"]);
        assert_eq!(out.inverse.len(), 1);
        out.inverse[0].validate().unwrap();

        let undo = run(&[out.inverse[0].action.clone()], &roots);
        assert!(undo.terminal.error().is_none(), "{:?}", undo.terminal);
        assert_eq!(entry_names(), vec!["Report.pdf"]);
        assert_eq!(fs::read(&upper).unwrap(), b"report");
    }

    #[test]
    fn mv_no_clobber_returns_uncertain_when_directory_appears_at_destination() {
        let (_d, roots) = setup();
        let h = &roots.home;

        // File source: racing directory at `to` causes `/bin/mv -n` to move the file inside it.
        let from = h.join("Downloads/report.txt");
        let to = h.join("Documents/report.txt");
        fs::write(&from, b"important").unwrap();
        let result = mv_no_clobber_with(&from, &to, || {
            fs::create_dir(&to).unwrap();
            fs::write(to.join("existing.txt"), b"unrelated").unwrap();
        });
        assert!(matches!(result, Err(PublishError::Uncertain(_))), "expected Uncertain, got {result:?}");
        // Ensure published_effect turns Uncertain into StepError::Uncertain without recording an inverse.
        let action = Action::Move { from: from.clone(), to: to.clone() };
        let step_err = published_effect(
            result,
            |id| Effect::reversible(action.clone(), Action::Move { from: to.clone(), to: from.clone() }, &[id]),
            |_| unreachable!(),
        )
        .unwrap_err();
        assert!(matches!(step_err, StepError::Uncertain(_)), "{step_err:?}");

        // Directory source: racing directory at `to` moves `from` into `to/<name>`.
        let dir_from = h.join("Downloads/proj");
        let dir_to = h.join("Documents/proj");
        fs::create_dir(&dir_from).unwrap();
        fs::write(dir_from.join("a.txt"), b"a").unwrap();
        let dir_result = mv_no_clobber_with(&dir_from, &dir_to, || {
            fs::create_dir(&dir_to).unwrap();
            fs::write(dir_to.join("keep.txt"), b"keep").unwrap();
        });
        assert!(matches!(dir_result, Err(PublishError::Uncertain(_))), "expected Uncertain for dir race, got {dir_result:?}");
    }
}
