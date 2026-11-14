//! Persisted schedules. A one-shot task runs only the exact confirmed actions.
//! A repeating task saves the command and plans it again at each run.
use crate::model::Action;
use chrono::{DateTime, Datelike, Timelike, Duration, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Weekday};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    collections::HashMap,
    sync::LazyLock,
};

static SUFFIX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)^\s+(?:(after|in)\s+(.+?)\s+(hours?|hrs?|minutes?|mins?)|at\s+(.+?))\s*[.!]?\s*$",
    )
    .unwrap()
});
static CLOCK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
    r"(?i)^(?:(today|tomorrow)\s+)?(\d{1,2})(?::(\d{2}))?\s*(a\.?m\.?|p\.?m\.?)?(?:\s+(today|tomorrow))?$"
).unwrap()
});

static CLAUSE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\s+(?:at|after|in)\b").unwrap());
// Recognize a complete time-shaped suffix, including invalid clock values, without
// claiming screenshot filenames or names such as “Archive at 2024”.
static TIME_LIKE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
    r"(?i)^(?:(?:today|tomorrow)\s+)?(?:\d{1,2}|\d+:\d+|\d+\s*[ap]\.?m\.?|\d+:\d+\s*[ap]\.?m\.?|noon|midnight)(?:\s+(?:today|tomorrow))?[.!]?$"
).unwrap()
});

fn delay_number(value: &str) -> Option<i64> {
    let value = value.trim().to_ascii_lowercase();
    if let Ok(n) = value.parse() {
        return Some(n);
    }
    fn word(value: &str) -> Option<i64> {
        Some(match value {
            "zero" => 0,
            "one" | "a" | "an" => 1,
            "two" => 2,
            "three" => 3,
            "four" => 4,
            "five" => 5,
            "six" => 6,
            "seven" => 7,
            "eight" => 8,
            "nine" => 9,
            "ten" => 10,
            "eleven" => 11,
            "twelve" => 12,
            "thirteen" => 13,
            "fourteen" => 14,
            "fifteen" => 15,
            "sixteen" => 16,
            "seventeen" => 17,
            "eighteen" => 18,
            "nineteen" => 19,
            "twenty" => 20,
            "thirty" => 30,
            "forty" => 40,
            "fifty" => 50,
            "sixty" => 60,
            "seventy" => 70,
            "eighty" => 80,
            "ninety" => 90,
            _ => return None,
        })
    }
    if let Some(n) = word(&value) {
        return Some(n);
    }
    let parts: Vec<_> = value.split([' ', '-']).filter(|s| !s.is_empty()).collect();
    match parts.as_slice() {
        [tens, ones] => {
            let (tens, ones) = (word(tens)?, word(ones)?);
            (tens >= 20 && tens % 10 == 0 && (1..=9).contains(&ones)).then_some(tens + ones)
        }
        _ => None,
    }
}

/// Scheduling clauses must be outside quotes. Clock times use the Mac's timezone.
pub fn extract(
    text: &str,
    now: DateTime<Local>,
) -> Result<(String, Option<DateTime<Local>>), String> {
    let masked =
        crate::parse::phrase::unquoted_text(text).ok_or("Close the quotation marks first.")?;
    // Try clause boundaries from the end. Earlier “at” words can belong to names.
    let candidates: Vec<_> = CLAUSE.find_iter(&masked).collect();
    let matched = candidates.into_iter().rev().find_map(|marker| {
        if masked[marker.start()..] != text[marker.start()..] {
            return None;
        }
        let c = SUFFIX.captures(&masked[marker.start()..])?;
        if c.get(4)
            .is_some_and(|clock| !TIME_LIKE.is_match(clock.as_str().trim()))
        {
            return None;
        }
        Some((marker.start(), c))
    });
    let Some((start, c)) = matched else {
        return Ok((text.into(), None));
    };
    let command = text[..start].trim().to_string();
    if command.is_empty() {
        return Err("Say what to do before the time.".into());
    }
    let due = if let Some(amount) = c.get(2) {
        let n = delay_number(amount.as_str()).ok_or(
            "Use digits or a number up to ninety-nine in words, such as “after twenty minutes”.",
        )?;
        let multiplier = if c[3].to_ascii_lowercase().starts_with('h') {
            60
        } else {
            1
        };
        let minutes = n
            .checked_mul(multiplier)
            .filter(|n| *n > 0 && *n <= 525600)
            .ok_or("Choose a delay between one minute and one year.")?;
        now.checked_add_signed(Duration::minutes(minutes))
            .ok_or("That time is too far away.")?
    } else {
        let value = c.get(4).unwrap().as_str().trim();
        let clock = CLOCK
            .captures(value)
            .ok_or("Use a time such as “at 9:40 pm” or “at 21:40”.")?;
        if clock.get(1).is_some() && clock.get(5).is_some() {
            return Err("Mention today or tomorrow once.".into());
        }
        let mut hour: u32 = clock[2].parse().unwrap();
        let minute: u32 = clock.get(3).map_or("0", |m| m.as_str()).parse().unwrap();
        if let Some(period) = clock.get(4) {
            if !(1..=12).contains(&hour) {
                return Err("Use an hour from 1 to 12 with am or pm.".into());
            }
            hour = hour % 12
                + if period.as_str().to_ascii_lowercase().starts_with('p') {
                    12
                } else {
                    0
                };
        } else if clock.get(3).is_none() {
            return Err("Include am or pm, or use a 24-hour time such as 21:40.".into());
        }
        let time =
            NaiveTime::from_hms_opt(hour, minute, 0).ok_or("That clock time isn't valid.")?;
        let day = clock
            .get(1)
            .or(clock.get(5))
            .map(|m| m.as_str().to_ascii_lowercase());
        let mut date = now.date_naive();
        if day.as_deref() == Some("tomorrow") {
            date = date.succ_opt().ok_or("Invalid date.")?;
        }
        let local = |date: chrono::NaiveDate| {
            Local.from_local_datetime(&date.and_time(time)).single()
            .ok_or_else(|| "That time is skipped or repeated by a daylight-saving change. Choose another time.".to_string())
        };
        let mut due = local(date)?;
        if due <= now {
            if day.as_deref() == Some("today") {
                return Err("That time has already passed today.".into());
            }
            due = local(date.succ_opt().ok_or("Invalid date.")?)?;
        }
        due
    };
    Ok((command, Some(due)))
}

static RECUR: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\s+(?:at\s+(?P<t1>[^\s]+(?:\s*[ap]\.?m\.?)?)\s+)?(?:every\s+(?:(?P<n>[a-z0-9]+(?:[ -][a-z]+)?)\s+)?(?P<unit>minutes?|mins?|hours?|hrs?|days?|weeks?)|every\s+(?P<wd>monday|tuesday|wednesday|thursday|friday|saturday|sunday)s?|(?P<adv>hourly|daily|weekly))(?:\s+at\s+(?P<t2>.+?))?\s*[.!]?\s*$",
    )
    .unwrap()
});

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Unit {
    Minutes,
    Hours,
    Days,
    Weeks,
}

/// How a repeating task repeats, and what it runs each time.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Rule {
    pub every: u32,
    pub unit: Unit,
    /// Wall-clock time for day and week repeats, kept across daylight-saving changes.
    pub time: Option<NaiveTime>,
    /// The command without its schedule, planned again at each run.
    pub command: String,
    /// Folder and file choices made when the task was scheduled.
    #[serde(default)]
    pub choices: HashMap<String, PathBuf>,
}

/// The shortest gap allowed between runs of a repeating task.
pub const MIN_REPEAT_MINUTES: i64 = 15;

fn local_at(date: NaiveDate, time: NaiveTime) -> Option<DateTime<Local>> {
    let at = NaiveDateTime::new(date, time);
    // A skipped daylight-saving hour moves to the first valid moment after it.
    Local.from_local_datetime(&at).earliest()
        .or_else(|| Local.from_local_datetime(&(at + Duration::hours(1))).earliest())
}

impl Rule {
    fn step(&self, due: DateTime<Local>) -> Option<DateTime<Local>> {
        let n = i64::from(self.every);
        match self.unit {
            Unit::Minutes => due.checked_add_signed(Duration::minutes(n)),
            Unit::Hours => due.checked_add_signed(Duration::hours(n)),
            Unit::Days | Unit::Weeks => {
                let days = if self.unit == Unit::Weeks { n * 7 } else { n };
                let date = due.date_naive().checked_add_signed(Duration::days(days))?;
                local_at(date, self.time.unwrap_or_else(|| due.time()))
            }
        }
    }
    /// The first run after `now`, counted from the schedule rather than from
    /// when the last run finished. Runs missed while asleep collapse into one.
    pub fn next_after(&self, due: DateTime<Local>, now: DateTime<Local>) -> Option<DateTime<Local>> {
        let mut next = self.step(due)?;
        let mut guard = 0;
        while next <= now {
            next = self.step(next)?;
            guard += 1;
            if guard > 1_000_000 { return None; }
        }
        Some(next)
    }
    pub fn describe(&self, first: DateTime<Local>) -> String {
        let n = self.every;
        let unit = |one: &str, many: &str| if n == 1 { format!("every {one}") } else { format!("every {n} {many}") };
        let time = self.time.map(|t| format!(" at {}", t.format("%-I:%M %p"))).unwrap_or_default();
        match self.unit {
            Unit::Minutes => unit("minute", "minutes"),
            Unit::Hours => unit("hour", "hours"),
            Unit::Days => format!("{}{time}", unit("day", "days")),
            Unit::Weeks if n == 1 => format!("every {}{time}", first.format("%A")),
            Unit::Weeks => format!("{} on {}{time}", unit("week", "weeks"), first.format("%A")),
        }
    }
}

/// A clock time such as “9:40 pm”, “9 pm” or “21:40”.
fn clock_time(value: &str) -> Result<NaiveTime, String> {
    let value = value.trim().trim_end_matches(['.', '!']).trim();
    let lower = value.to_ascii_lowercase();
    if lower == "noon" { return Ok(NaiveTime::from_hms_opt(12, 0, 0).unwrap()); }
    if lower == "midnight" { return Ok(NaiveTime::from_hms_opt(0, 0, 0).unwrap()); }
    let clock = CLOCK.captures(value).filter(|c| c.get(1).is_none() && c.get(5).is_none())
        .ok_or("Use a time such as “at 9:40 pm” or “at 21:40”.")?;
    let mut hour: u32 = clock[2].parse().map_err(|_| "That clock time isn't valid.")?;
    let minute: u32 = clock.get(3).map_or("0", |m| m.as_str()).parse().map_err(|_| "That clock time isn't valid.")?;
    if let Some(period) = clock.get(4) {
        if !(1..=12).contains(&hour) { return Err("Use an hour from 1 to 12 with am or pm.".into()); }
        hour = hour % 12 + if period.as_str().to_ascii_lowercase().starts_with('p') { 12 } else { 0 };
    } else if clock.get(3).is_none() {
        return Err("Include am or pm, or use a 24-hour time such as 21:40.".into());
    }
    NaiveTime::from_hms_opt(hour, minute, 0).ok_or_else(|| "That clock time isn't valid.".into())
}

fn weekday(name: &str) -> Option<Weekday> {
    Some(match &name.to_ascii_lowercase()[..3] {
        "mon" => Weekday::Mon, "tue" => Weekday::Tue, "wed" => Weekday::Wed, "thu" => Weekday::Thu,
        "fri" => Weekday::Fri, "sat" => Weekday::Sat, "sun" => Weekday::Sun, _ => return None,
    })
}

/// When a command should run: once, or first at `due` and then by `rule`.
#[derive(Clone, Debug, PartialEq)]
pub struct When {
    pub due: DateTime<Local>,
    pub rule: Option<Rule>,
}

/// Finds “every 3 days”, “daily at 9 pm”, “every Monday at 8:30 am”, or a
/// one-time “at 9:40 pm” / “after 3 hours” at the end of a command.
pub fn extract_schedule(text: &str, now: DateTime<Local>) -> Result<(String, Option<When>), String> {
    if let Some((command, when)) = extract_repeat(text, now)? {
        return Ok((command, Some(when)));
    }
    let (command, due) = extract(text, now)?;
    Ok((command, due.map(|due| When { due, rule: None })))
}

fn extract_repeat(text: &str, now: DateTime<Local>) -> Result<Option<(String, When)>, String> {
    let masked = crate::parse::phrase::unquoted_text(text).ok_or("Close the quotation marks first.")?;
    let Some(c) = RECUR.captures(&masked) else { return Ok(None) };
    let whole = c.get(0).unwrap();
    if masked[whole.start()..] != text[whole.start()..] { return Ok(None); }
    let command = text[..whole.start()].trim().to_string();
    if command.is_empty() { return Err("Say what to do before how often.".into()); }

    let (every, unit, day) = if let Some(adv) = c.name("adv") {
        let unit = match adv.as_str().to_ascii_lowercase().as_str() { "hourly" => Unit::Hours, "daily" => Unit::Days, _ => Unit::Weeks };
        (1, unit, None)
    } else if let Some(wd) = c.name("wd") {
        (1, Unit::Weeks, weekday(wd.as_str()))
    } else {
        let every = match c.name("n").map(|m| m.as_str().to_ascii_lowercase()) {
            None => 1,
            Some(n) if n == "other" => 2,
            Some(n) => delay_number(&n).ok_or("Use digits or a number up to ninety-nine in words, such as “every 3 days”.")?,
        };
        let raw = c["unit"].to_ascii_lowercase();
        let unit = if raw.starts_with('m') { Unit::Minutes } else if raw.starts_with('h') { Unit::Hours }
            else if raw.starts_with('d') { Unit::Days } else { Unit::Weeks };
        (every, unit, None)
    };
    if !(1..=365).contains(&every) { return Err("Choose how often between once and 365.".into()); }
    let every = every as u32;
    let minutes = i64::from(every) * match unit { Unit::Minutes => 1, Unit::Hours => 60, Unit::Days => 1440, Unit::Weeks => 10080 };
    if minutes < MIN_REPEAT_MINUTES { return Err(format!("Repeat at most every {MIN_REPEAT_MINUTES} minutes.")); }

    let time = match (c.name("t1"), c.name("t2")) {
        (Some(_), Some(_)) => return Err("Mention the time once.".into()),
        (Some(t), None) | (None, Some(t)) => Some(clock_time(t.as_str())?),
        (None, None) => None,
    };
    if time.is_some() && matches!(unit, Unit::Minutes | Unit::Hours) {
        return Err("A time of day works with days or weeks, such as “every day at 9 pm”.".into());
    }

    let (time, due) = match (unit, time, day) {
        (Unit::Minutes | Unit::Hours, _, _) => {
            let rule = Rule { every, unit, time: None, command: String::new(), choices: HashMap::new() };
            (None, rule.step(now).ok_or("That time is too far away.")?)
        }
        // A weekday without a time runs at 9 am.
        (_, time, Some(day)) => {
            let time = time.unwrap_or_else(|| NaiveTime::from_hms_opt(9, 0, 0).unwrap());
            let ahead = (7 + day.num_days_from_monday() as i64 - now.weekday().num_days_from_monday() as i64) % 7;
            let mut date = now.date_naive() + Duration::days(ahead);
            let mut due = local_at(date, time).ok_or("Invalid date.")?;
            if due <= now { date += Duration::days(7); due = local_at(date, time).ok_or("Invalid date.")?; }
            (Some(time), due)
        }
        // With a time: the next time it comes round. Without: one interval from now.
        (_, Some(time), None) => {
            let mut due = local_at(now.date_naive(), time).ok_or("Invalid date.")?;
            if due <= now { due = local_at(now.date_naive() + Duration::days(1), time).ok_or("Invalid date.")?; }
            (Some(time), due)
        }
        (_, None, None) => {
            let time = now.time().with_nanosecond(0).unwrap_or(now.time());
            let rule = Rule { every, unit, time: Some(time), command: String::new(), choices: HashMap::new() };
            (Some(time), rule.step(now).ok_or("That time is too far away.")?)
        }
    };
    let rule = Rule { every, unit, time, command: command.clone(), choices: HashMap::new() };
    Ok(Some((command, When { due, rule: Some(rule) })))
}

pub fn display(due: DateTime<Local>) -> String {
    due.format("%a, %b %-d at %-I:%M %p %Z").to_string()
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
struct Stamp {
    path: PathBuf,
    len: u64,
    modified: Option<std::time::SystemTime>,
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
}
impl Stamp {
    fn capture(path: &Path) -> Result<Self, String> {
        let m = fs::symlink_metadata(path)
            .map_err(|e| format!("Couldn't check {}: {e}", path.display()))?;
        if m.file_type().is_symlink() {
            return Err("Scheduled tasks cannot use symbolic links.".into());
        }
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        Ok(Self {
            path: path.into(),
            len: m.len(),
            modified: m.modified().ok(),
            #[cfg(unix)]
            device: m.dev(),
            #[cfg(unix)]
            inode: m.ino(),
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    pub id: u64,
    pub command: String,
    pub due: DateTime<Local>,
    pub status: String,
    pub summary: String,
    pub actions: Vec<Action>,
    stamps: Vec<Stamp>,
    /// Present for repeating tasks, which plan their command again at each run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repeat: Option<Rule>,
}

/// Actions a scheduled task may run. Undo steps and Finder commands may not.
pub fn check_actions(actions: &[Action]) -> Result<(), String> {
    for a in actions {
        if matches!(a, Action::OpenInFinder { .. } | Action::RemoveEmptyFolder { .. } | Action::RestoreFromTrash { .. } | Action::Restore { .. }) {
            return Err("Undo and Finder commands cannot be scheduled.".into());
        }
    }
    Ok(())
}

impl Job {
    /// A repeating task. It keeps no file list: each run uses what matches then.
    pub fn recurring(command: String, due: DateTime<Local>, rule: Rule) -> Self {
        Self { id: 0, command, due, status: "pending".into(), summary: String::new(), actions: Vec::new(), stamps: Vec::new(), repeat: Some(rule) }
    }
    #[cfg(test)]
    pub fn new(
        command: String,
        due: DateTime<Local>,
        actions: Vec<Action>,
    ) -> Result<Self, String> {
        Self::new_with_control(command, due, actions, &crate::search::Control::default())
    }
    pub fn new_with_control(
        command: String,
        due: DateTime<Local>,
        actions: Vec<Action>,
        control: &crate::search::Control,
    ) -> Result<Self, String> {
        let started = std::time::Instant::now();
        let check = || {
            if control.is_cancelled() {
                return Err("Search cancelled.".to_string());
            }
            if started.elapsed() >= std::time::Duration::from_secs(10) {
                return Err(
                    "Checking scheduled files took too long. Choose a smaller group.".to_string(),
                );
            }
            Ok(())
        };
        let mut paths = Vec::new();
        for a in &actions {
            match a {
                Action::Move { from, .. } | Action::Copy { from, .. } => paths.push(from.clone()),
                Action::Rename { path, .. }
                | Action::Trash { path }
                | Action::ConvertImage { path, .. }
                | Action::ResizeImage { path, .. }
                | Action::SplitPdf { path, .. }
                | Action::ExtractPages { path, .. }
                | Action::Zip { path, .. }
                | Action::Unzip { path, .. } => paths.push(path.clone()),
                Action::ImagesToPdf { paths: inputs, .. }
                | Action::MergePdf { paths: inputs, .. } => paths.extend(inputs.clone()),
                Action::CreateFolder { .. } => {}
                _ => return Err("Undo and Finder commands cannot be scheduled.".into()),
            }
        }
        paths.sort();
        paths.dedup();
        // A scheduled folder must not silently include children added after approval.
        let mut stamps = Vec::new();
        while let Some(path) = paths.pop() {
            check()?;
            if stamps.len() >= 50_000 {
                return Err("Too many items to schedule. Choose a smaller group.".into());
            }
            let stamp = Stamp::capture(&path)?;
            if path.is_dir() {
                let children = fs::read_dir(&path)
                    .map_err(|e| format!("Couldn't check {}: {e}", path.display()))?;
                for child in children {
                    check()?;
                    paths.push(child.map_err(|e| e.to_string())?.path());
                    if paths.len() + stamps.len() > 50_000 {
                        return Err("Too many items to schedule. Choose a smaller group.".into());
                    }
                }
            }
            stamps.push(stamp);
        }
        Ok(Self {
            id: 0,
            command,
            due,
            status: "pending".into(),
            summary: String::new(),
            actions,
            stamps,
            repeat: None,
        })
    }
    pub fn validate(&self) -> Result<(), String> {
        for stamp in &self.stamps {
            if Stamp::capture(&stamp.path).as_ref() != Ok(stamp) {
                return Err(format!(
                    "{} changed or disappeared since scheduling. Schedule it again.",
                    stamp.path.display()
                ));
            }
        }
        Ok(())
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub id: u64,
    pub command: String,
    pub due: DateTime<Local>,
    pub status: String,
    pub summary: String,
    /// How often a repeating task runs, such as “every 3 days at 9:00 PM”.
    pub repeat: Option<String>,
}
impl Job {
    pub fn summary(&self) -> Summary {
        Summary {
            id: self.id,
            command: self.command.clone(),
            due: self.due,
            status: self.status.clone(),
            summary: self.summary.clone(),
            repeat: self.repeat.as_ref().map(|rule| rule.describe(self.due)),
        }
    }
}

pub struct Store {
    path: PathBuf,
    pub jobs: Vec<Job>,
    notice: Option<String>,
    unavailable: Option<String>,
}
impl Store {
    pub fn load(path: PathBuf) -> Self {
        let mut notice = None;
        let jobs = match fs::read(&path) {
            Ok(json) => match serde_json::from_slice::<Vec<Job>>(&json) {
                Ok(jobs) => jobs,
                Err(error) => {
                    let backup = path.with_extension(format!(
                        "unreadable-{}.json",
                        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
                    ));
                    let preserved = fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(&backup)
                        .and_then(|mut file| {
                            file.write_all(&json)?;
                            file.sync_all()
                        });
                    if let Err(e) = preserved {
                        return Self::unavailable(
                            path,
                            format!(
                                "Couldn't read schedules ({error}) or preserve them ({e}). Scheduling is unavailable; the original file has been kept."
                            ),
                        );
                    }
                    notice = Some(format!(
                        "Saved unreadable schedules to {}. No saved tasks were restored. You can schedule new tasks.",
                        backup.display()
                    ));
                    Vec::new()
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => {
                return Self::unavailable(
                    path,
                    format!(
                        "Couldn't read schedules: {e}. Scheduling is unavailable; existing data has been kept."
                    ),
                );
            }
        };
        let mut store = Self {
            path,
            jobs,
            notice,
            unavailable: None,
        };
        // Never rerun a task whose process stopped partway through execution.
        // A repeating task skips to its next run instead.
        let now = Local::now();
        for job in &mut store.jobs {
            if job.status == "running" && let Some(next) = job.repeat.as_ref().and_then(|rule| rule.next_after(job.due, now)) {
                job.status = "pending".into();
                job.due = next;
                job.summary = "Voxi stopped during the last run. Check History; the next run is scheduled.".into();
            } else if job.status == "running" {
                job.status = "failed".into();
                job.summary = "Voxi stopped during this task. Check files and History before scheduling again.".into();
            }
        }
        if let Err(error) = store.save() {
            store.jobs.clear();
            store.unavailable = Some(error);
        }
        store
    }
    fn unavailable(path: PathBuf, error: String) -> Self {
        Self {
            path,
            jobs: Vec::new(),
            notice: None,
            unavailable: Some(error),
        }
    }
    pub fn notice(&self) -> Option<String> {
        self.unavailable.clone().or_else(|| self.notice.clone())
    }
    fn save(&self) -> Result<(), String> {
        if let Some(error) = &self.unavailable {
            return Err(error.clone());
        }
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let json = serde_json::to_vec_pretty(&self.jobs).map_err(|e| e.to_string())?;
        let tmp = self.path.with_extension("json.tmp");
        fs::write(&tmp, json)
            .and_then(|_| fs::rename(tmp, &self.path))
            .map_err(|e| format!("Couldn't save schedules: {e}"))
    }
    pub fn add(&mut self, mut job: Job) -> Result<(), String> {
        job.id = self.jobs.iter().map(|j| j.id).max().unwrap_or(0) + 1;
        self.jobs.push(job);
        if let Err(e) = self.save() {
            self.jobs.pop();
            return Err(e);
        }
        Ok(())
    }
    pub fn update(&mut self, id: u64, status: &str, summary: String) -> Result<(), String> {
        let job = self
            .jobs
            .iter_mut()
            .find(|j| j.id == id)
            .ok_or("That task no longer exists.")?;
        let old = job.clone();
        job.status = status.into();
        job.summary = summary;
        if let Err(e) = self.save() {
            *self.jobs.iter_mut().find(|j| j.id == id).unwrap() = old;
            return Err(e);
        }
        Ok(())
    }
    pub fn cancel(&mut self, id: u64) -> Result<(), String> {
        let job = self.jobs.iter().find(|j| j.id == id && matches!(j.status.as_str(), "pending" | "paused"))
            .ok_or("Only pending or paused tasks can be cancelled.")?;
        let summary = if job.repeat.is_some() { "Stopped.".into() } else { "Cancelled before running.".into() };
        self.update(id, "cancelled", summary)
    }
    /// Stops a repeating task from running until it is resumed.
    pub fn pause(&mut self, id: u64) -> Result<(), String> {
        let job = self.jobs.iter().find(|j| j.id == id && j.status == "pending" && j.repeat.is_some())
            .ok_or("Only waiting repeating tasks can be paused.")?;
        let summary = job.summary.clone();
        self.update(id, "paused", summary)
    }
    /// Makes a paused, stopped, or failed repeating task active again.
    /// Runs missed in the meantime are skipped. One-time tasks can't come
    /// back: the files they were checked against may have changed.
    pub fn resume(&mut self, id: u64, now: DateTime<Local>) -> Result<(), String> {
        let index = self.jobs.iter().position(|j| j.id == id && j.repeat.is_some() && matches!(j.status.as_str(), "paused" | "cancelled" | "failed"))
            .ok_or("Only repeating tasks that aren't running can be made active again.")?;
        let old = self.jobs[index].clone();
        let job = &mut self.jobs[index];
        let rule = job.repeat.as_ref().ok_or("Only repeating tasks can be resumed.")?;
        if job.due <= now { job.due = rule.next_after(job.due, now).ok_or("That time is too far away.")?; }
        if job.status != "paused" { job.summary = "Made active again.".into(); }
        job.status = "pending".into();
        if let Err(e) = self.save() { self.jobs[index] = old; return Err(e); }
        Ok(())
    }
    /// Records a repeating task's run and schedules the next one.
    pub fn finish_recurring(&mut self, id: u64, summary: String, now: DateTime<Local>) -> Result<(), String> {
        let index = self.jobs.iter().position(|j| j.id == id).ok_or("That task no longer exists.")?;
        let old = self.jobs[index].clone();
        let job = &mut self.jobs[index];
        let rule = job.repeat.as_ref().ok_or("That task doesn't repeat.")?;
        job.summary = format!("Last run {}: {summary}", display(now));
        match rule.next_after(job.due, now) {
            Some(next) => { job.due = next; job.status = "pending".into(); }
            None => { job.status = "failed".into(); job.summary.push_str(" The next run is too far away."); }
        }
        if let Err(e) = self.save() { self.jobs[index] = old; return Err(e); }
        Ok(())
    }
    pub fn claim(&mut self, now: DateTime<Local>) -> Result<Option<Job>, String> {
        let job = self
            .jobs
            .iter()
            .filter(|j| j.status == "pending" && j.due <= now)
            .min_by_key(|j| j.due)
            .cloned();
        if let Some(job) = &job {
            self.update(job.id, "running", "Running…".into())?;
        }
        Ok(job)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn now() -> DateTime<Local> {
        Local.with_ymd_and_hms(2026, 11, 12, 20, 0, 0).unwrap()
    }
    #[test]
    fn parses_clock_and_delays() {
        let (cmd, due) = extract(
            "Move screenshots from desktop to Screenshots folder at 9:40 pm",
            now(),
        )
        .unwrap();
        assert_eq!(cmd, "Move screenshots from desktop to Screenshots folder");
        assert_eq!(due.unwrap().format("%H:%M").to_string(), "21:40");
        for (suffix, minutes) in [
            ("after 3 hours", 180),
            ("in 20 minutes", 20),
            ("after three hours", 180),
            ("after 1 minute", 1),
        ] {
            assert_eq!(
                extract(&format!("move a.pdf to Documents {suffix}"), now())
                    .unwrap()
                    .1
                    .unwrap(),
                now() + Duration::minutes(minutes)
            );
        }
        assert_eq!(
            extract("move a.pdf to Documents at 19:00", now())
                .unwrap()
                .1
                .unwrap()
                .date_naive(),
            now().date_naive().succ_opt().unwrap()
        );
        assert_eq!(
            extract("move a.pdf to Documents at 12 am tomorrow", now())
                .unwrap()
                .1
                .unwrap()
                .format("%H:%M")
                .to_string(),
            "00:00"
        );
    }
    #[test]
    fn invalid_times_and_quoted_names() {
        for suffix in [
            "after 0 minutes",
            "after -3 hours",
            "after 999999999999999999999 hours",
            "at 25:00",
            "at 13 pm",
            "at 9",
            "at 7 pm today",
            "at noon",
        ] {
            assert!(
                extract(&format!("move a.pdf to Documents {suffix}"), now()).is_err(),
                "{suffix}"
            );
        }
        assert!(
            extract("rename a.pdf to \"report at 9:40 pm\"", now())
                .unwrap()
                .1
                .is_none()
        );
    }
    #[test]
    fn filenames_and_final_time_clauses_do_not_conflict() {
        for command in [
            "move Screenshot 2026-11-12 at 09.40.00.png to Screenshots",
            "move a.pdf to Archive at 2024",
            "move a.pdf to Archive at Tomorrowland",
            "rename a.pdf to Notes at midnight sun",
            "rename a.pdf to \"report at 9:40 pm\"",
            "rename a.pdf to \"Archive after twenty minutes\"",
        ] {
            assert_eq!(
                extract(command, now()).unwrap(),
                (command.into(), None),
                "{command}"
            );
        }
        for command in [
            "move Trip at Home to Archive at 9:40 pm",
            "move Screenshot 2026-11-12 at 09.40.00.png to Screenshots at 9:40 pm",
            "move \"Trip at Home\" to Archive at 9:40 pm",
        ] {
            let (base, due) = extract(command, now()).unwrap();
            assert_eq!(base, command.strip_suffix(" at 9:40 pm").unwrap());
            assert_eq!(due.unwrap().format("%H:%M").to_string(), "21:40");
        }
        for suffix in [
            "after twenty minutes",
            "in twenty-one minutes",
            "after ninety nine minutes",
        ] {
            let command = format!("move Trip at Home to Archive {suffix}");
            let (base, due) = extract(&command, now()).unwrap();
            assert_eq!(base, "move Trip at Home to Archive");
            let minutes = if suffix.contains("ninety") {
                99
            } else if suffix.contains("one") {
                21
            } else {
                20
            };
            assert_eq!(due.unwrap(), now() + Duration::minutes(minutes));
        }
    }
    #[test]
    fn parses_repeats() {
        let at = |h, m| NaiveTime::from_hms_opt(h, m, 0).unwrap();
        let cases: &[(&str, u32, Unit, Option<NaiveTime>)] = &[
            ("every 3 days", 3, Unit::Days, Some(at(20, 0))),
            ("every day at 9 pm", 1, Unit::Days, Some(at(21, 0))),
            ("at 7:30 am every day", 1, Unit::Days, Some(at(7, 30))),
            ("daily at 21:40", 1, Unit::Days, Some(at(21, 40))),
            ("every other week", 2, Unit::Weeks, Some(at(20, 0))),
            ("every two hours", 2, Unit::Hours, None),
            ("hourly", 1, Unit::Hours, None),
            ("every 30 minutes", 30, Unit::Minutes, None),
            ("every Monday at 8 am", 1, Unit::Weeks, Some(at(8, 0))),
            ("weekly", 1, Unit::Weeks, Some(at(20, 0))),
        ];
        for (suffix, every, unit, time) in cases {
            let (cmd, when) = extract_schedule(&format!("move a.pdf to Documents {suffix}"), now()).unwrap();
            assert_eq!(cmd, "move a.pdf to Documents", "{suffix}");
            let when = when.unwrap();
            let rule = when.rule.clone().unwrap();
            assert_eq!((rule.every, rule.unit, rule.time), (*every, *unit, *time), "{suffix}");
            assert!(when.due > now(), "{suffix}");
            assert_eq!(rule.command, "move a.pdf to Documents");
        }
        // 2026-11-12 is a Thursday, 8 pm.
        let due = |s: &str| extract_schedule(&format!("move a.pdf to Documents {s}"), now()).unwrap().1.unwrap().due;
        assert_eq!(due("every 3 days"), now() + Duration::days(3));
        assert_eq!(due("every day at 9 pm"), Local.with_ymd_and_hms(2026, 11, 12, 21, 0, 0).unwrap());
        assert_eq!(due("every day at 7 pm"), Local.with_ymd_and_hms(2026, 11, 13, 19, 0, 0).unwrap());
        assert_eq!(due("every Monday"), Local.with_ymd_and_hms(2026, 11, 16, 9, 0, 0).unwrap());
        assert_eq!(due("every thursday at 9 pm"), Local.with_ymd_and_hms(2026, 11, 12, 21, 0, 0).unwrap());
        assert_eq!(due("every thursday at 7 pm"), Local.with_ymd_and_hms(2026, 11, 19, 19, 0, 0).unwrap());
        for bad in ["every 5 minutes", "every 2 hours at 9 pm", "at 9 pm every day at 10 pm", "every 0 days", "every 400 days"] {
            assert!(extract_schedule(&format!("move a.pdf to Documents {bad}"), now()).is_err(), "{bad}");
        }
        // Quoted names and one-time schedules are untouched.
        assert!(extract_schedule("rename a.pdf to \"every day\"", now()).unwrap().1.is_none());
        assert!(extract_schedule("move a.pdf to Documents at 9:40 pm", now()).unwrap().1.unwrap().rule.is_none());
    }
    #[test]
    fn repeats_follow_the_schedule_and_collapse_missed_runs() {
        let rule = Rule { every: 3, unit: Unit::Days, time: Some(NaiveTime::from_hms_opt(21, 0, 0).unwrap()), command: "x".into(), choices: HashMap::new() };
        let first = Local.with_ymd_and_hms(2026, 11, 1, 21, 0, 0).unwrap();
        // Voxi was closed for ten days: one run now, then back on the 3-day grid.
        let next = rule.next_after(first, now()).unwrap();
        assert_eq!(next, Local.with_ymd_and_hms(2026, 11, 13, 21, 0, 0).unwrap());
        assert_eq!(rule.describe(first), "every 3 days at 9:00 PM");
        let weekly = Rule { every: 1, unit: Unit::Weeks, ..rule.clone() };
        assert_eq!(weekly.describe(Local.with_ymd_and_hms(2026, 11, 16, 21, 0, 0).unwrap()), "every Monday at 9:00 PM");
    }
    #[test]
    fn repeating_tasks_survive_reload_and_interrupted_runs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("jobs.json");
        let mut store = Store::load(path.clone());
        let rule = Rule { every: 1, unit: Unit::Hours, time: None, command: "move a.pdf to Documents".into(), choices: HashMap::new() };
        store.add(Job::recurring("move a.pdf to Documents hourly".into(), Local::now() - Duration::minutes(1), rule.clone())).unwrap();
        assert_eq!(store.claim(Local::now()).unwrap().unwrap().id, 1);
        // Voxi quit mid-run: the task skips ahead instead of failing or rerunning.
        let store = Store::load(path.clone());
        assert_eq!(store.jobs[0].status, "pending");
        assert!(store.jobs[0].due > Local::now());
        assert_eq!(store.jobs[0].repeat, Some(rule));
        assert_eq!(store.jobs[0].summary().repeat.as_deref(), Some("every hour"));
    }
    #[test]
    fn unreadable_schedule_data_is_preserved_without_blocking_startup() {
        for bytes in [
            b"{".as_slice(),
            b"[{\"oldSchema\":true}]".as_slice(),
            &[0xff],
        ] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("schedules.json");
            fs::write(&path, bytes).unwrap();
            let mut store = Store::load(path.clone());
            assert!(store.jobs.is_empty());
            assert!(store.notice().unwrap().contains("unreadable"));
            let backup = fs::read_dir(dir.path())
                .unwrap()
                .map(|e| e.unwrap().path())
                .find(|p| p != &path)
                .unwrap();
            assert_eq!(fs::read(backup).unwrap(), bytes);
            store
                .add(Job::new("new task".into(), now(), vec![]).unwrap())
                .unwrap();
            assert_eq!(Store::load(path).jobs.len(), 1);
        }
    }
    #[test]
    fn schedule_io_failures_disable_scheduling_without_overwriting_data() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("schedules.json");
        fs::create_dir(&path).unwrap();
        let mut store = Store::load(path.clone());
        assert!(store.notice().unwrap().contains("Couldn't read"));
        assert!(
            store
                .add(Job::new("new task".into(), now(), vec![]).unwrap())
                .is_err()
        );
        assert!(path.is_dir());
        assert!(store.jobs.is_empty());
    }
    #[test]
    fn scheduled_folder_checks_honor_cancellation() {
        let control = crate::search::Control::default();
        control.cancel();
        let dir = tempfile::tempdir().unwrap();
        let actions = vec![Action::Move {
            from: dir.path().into(),
            to: dir.path().join("destination"),
        }];
        let error =
            Job::new_with_control("move folder".into(), now(), actions, &control).unwrap_err();
        assert_eq!(error, "Search cancelled.");
    }
    #[test]
    fn persistence_cancellation_and_claim_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("jobs.json");
        let mut store = Store::load(path.clone());
        store
            .add(Job::new("create folder".into(), now(), vec![]).unwrap())
            .unwrap();
        assert!(store.claim(now() - Duration::minutes(1)).unwrap().is_none());
        assert_eq!(store.claim(now()).unwrap().unwrap().id, 1);
        assert!(store.claim(now()).unwrap().is_none());
        let mut store = Store::load(path);
        assert_eq!(store.jobs[0].status, "failed");
        store
            .add(Job::new("create folder".into(), now(), vec![]).unwrap())
            .unwrap();
        store.cancel(2).unwrap();
        assert!(store.claim(now()).unwrap().is_none());
    }
    #[test]
    fn names_with_at_are_preserved() {
        assert_eq!(
            extract("rename a.txt to Notes at Work", now()).unwrap(),
            ("rename a.txt to Notes at Work".into(), None)
        );
    }
    #[test]
    fn scheduled_folders_reject_new_or_modified_children() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("folder");
        fs::create_dir(&src).unwrap();
        let child = src.join("child.txt");
        fs::write(&child, "original").unwrap();
        let job = Job::new(
            "move folder".into(),
            now(),
            vec![Action::Move {
                from: src.clone(),
                to: dir.path().join("destination"),
            }],
        )
        .unwrap();
        job.validate().unwrap();
        fs::write(child, "changed contents").unwrap();
        assert!(job.validate().is_err());
        let job = Job::new(
            "move folder".into(),
            now(),
            vec![Action::Move {
                from: src.clone(),
                to: dir.path().join("destination"),
            }],
        )
        .unwrap();
        fs::write(src.join("new.txt"), "new").unwrap();
        assert!(job.validate().is_err());
    }
    #[test]
    fn changed_sources_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("a.txt");
        fs::write(&src, "original").unwrap();
        let job = Job::new(
            "move".into(),
            now(),
            vec![Action::Move {
                from: src.clone(),
                to: dir.path().join("b.txt"),
            }],
        )
        .unwrap();
        job.validate().unwrap();
        fs::write(&src, "changed contents").unwrap();
        assert!(job.validate().is_err());
    }
}
