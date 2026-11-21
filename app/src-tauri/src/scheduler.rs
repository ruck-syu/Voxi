//! Persisted schedules. A one-shot task runs only the exact confirmed actions.
//! A repeating task saves the command and plans it again at each run.
use crate::model::Action;
#[path = "scheduler/natural.rs"]
mod natural;
#[cfg(test)]
#[path = "scheduler/natural_tests.rs"]
mod natural_tests;
use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Timelike, Weekday};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::LazyLock,
};

static SUFFIX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^\s+(?:(after|in)\s+(.+?)\s+(hours?|hrs?|minutes?|mins?)|at\s+(.+?))\s*[.!]?\s*$").unwrap());
static CLOCK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(?:(today|tomorrow)\s+)?(\d{1,2})(?::(\d{2}))?\s*(a\.?m\.?|p\.?m\.?)?(?:\s+(today|tomorrow))?$").unwrap()
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
pub fn extract(text: &str, now: DateTime<Local>) -> Result<(String, Option<DateTime<Local>>), String> {
    crate::parse::check_length(text).map_err(|_| "That command is too long.".to_string())?;
    if let Some(result) = natural::one_time(text, now)? {
        return Ok(result);
    }
    let masked = crate::parse::phrase::unquoted_text(text).ok_or("Close the quotation marks first.")?;
    // Try clause boundaries from the end. Earlier “at” words can belong to names.
    let candidates: Vec<_> = CLAUSE.find_iter(&masked).collect();
    let matched = candidates.into_iter().rev().find_map(|marker| {
        if masked[marker.start()..] != text[marker.start()..] {
            return None;
        }
        let c = SUFFIX.captures(&masked[marker.start()..])?;
        if c.get(4).is_some_and(|clock| !TIME_LIKE.is_match(clock.as_str().trim())) {
            return None;
        }
        Some((marker.start(), c))
    });
    let Some((start, c)) = matched else {
        return Ok((text.into(), None));
    };
    let mut command = text[..start].trim().to_string();
    // "move X tomorrow at 9am": the day comes before "at", outside the clause.
    let mut lead_day = None;
    if c.get(4).is_some()
        && let Some((rest, day)) = command.rsplit_once(char::is_whitespace)
    {
        // "from today at 9 pm": "today" is a filter on the files, not the run day.
        let filter_word = rest
            .rsplit(char::is_whitespace)
            .next()
            .is_some_and(|w| ["from", "since", "until", "before", "after", "of", "on"].iter().any(|f| w.eq_ignore_ascii_case(f)));
        if !filter_word && (day.eq_ignore_ascii_case("today") || day.eq_ignore_ascii_case("tomorrow")) {
            lead_day = Some(day.to_ascii_lowercase());
            command = rest.trim_end().to_string();
        }
    }
    if command.is_empty() {
        return Err("Say what to do before the time.".into());
    }
    let due = if let Some(amount) = c.get(2) {
        let n =
            delay_number(amount.as_str()).ok_or("Use digits or a number up to ninety-nine in words, such as “after twenty minutes”.")?;
        let multiplier = if c[3].to_ascii_lowercase().starts_with('h') { 60 } else { 1 };
        let minutes =
            n.checked_mul(multiplier).filter(|n| *n > 0 && *n <= 525600).ok_or("Choose a delay between one minute and one year.")?;
        now.checked_add_signed(Duration::minutes(minutes)).ok_or("That time is too far away.")?
    } else {
        let value = c.get(4).unwrap().as_str().trim();
        let clock = CLOCK.captures(value).ok_or("Use a time such as “at 9:40 pm” or “at 21:40”.")?;
        if [clock.get(1).is_some(), clock.get(5).is_some(), lead_day.is_some()].into_iter().filter(|b| *b).count() > 1 {
            return Err("Mention today or tomorrow once.".into());
        }
        let mut hour: u32 = clock[2].parse().unwrap();
        let minute: u32 = clock.get(3).map_or("0", |m| m.as_str()).parse().unwrap();
        if let Some(period) = clock.get(4) {
            if !(1..=12).contains(&hour) {
                return Err("Use an hour from 1 to 12 with am or pm.".into());
            }
            hour = hour % 12 + if period.as_str().to_ascii_lowercase().starts_with('p') { 12 } else { 0 };
        } else if clock.get(3).is_none() {
            return Err("Include am or pm, or use a 24-hour time such as 21:40.".into());
        }
        let time = NaiveTime::from_hms_opt(hour, minute, 0).ok_or("That clock time isn't valid.")?;
        let day = clock.get(1).or(clock.get(5)).map(|m| m.as_str().to_ascii_lowercase()).or(lead_day);
        let mut date = now.date_naive();
        if day.as_deref() == Some("tomorrow") {
            date = date.succ_opt().ok_or("Invalid date.")?;
        }
        let local = |date: chrono::NaiveDate| {
            Local
                .from_local_datetime(&date.and_time(time))
                .single()
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
        r"(?i)\s+(?:at\s+(?P<t1>[^\s]+(?:\s*[ap]\.?m\.?)?)\s+)?(?:every\s+(?:(?P<n>[a-z0-9]+(?:[ -][a-z]+)?)\s+)?(?P<unit>minutes?|mins?|hours?|hrs?|days?|weeks?)|every\s+(?P<wd>monday|tuesday|wednesday|thursday|friday|saturday|sunday)s?|(?P<adv>hourly|daily|everyday|weekly))(?:\s+at\s+(?P<t2>.+?))?\s*[.!]?\s*$",
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
#[serde(rename_all = "camelCase", from = "SavedRule")]
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
    /// Read legacy model data for compatibility. Persist and run only rule text.
    #[serde(skip_serializing)]
    pub model_intent: Option<crate::model::Intent>,
    pub approved_version: u32,
    /// Older rules lack a record of whether a shortcut supplied the commands.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approved_source: Option<RuleSource>,
    /// The folder nicknames (name, path) in effect when the task was
    /// approved. A nickname its commands use that now means another folder
    /// sends the task to review instead of to that folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approved_nicknames: Option<Vec<(String, String)>>,
}

pub const APPROVED_RULE_VERSION: u32 = 2;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RuleSource {
    Command,
    Shortcut { phrase: String, commands: Vec<String> },
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SavedRule {
    every: u32,
    unit: Unit,
    time: Option<NaiveTime>,
    command: String,
    #[serde(default)]
    choices: HashMap<String, PathBuf>,
    model_intent: Option<crate::model::Intent>,
    #[serde(default)]
    modified_dates: bool,
    #[serde(default)]
    subfolder_opt_in: bool,
    approved_version: Option<u32>,
    approved_source: Option<RuleSource>,
    #[serde(default)]
    approved_nicknames: Option<Vec<(String, String)>>,
}

impl From<SavedRule> for Rule {
    fn from(saved: SavedRule) -> Self {
        Self {
            every: saved.every,
            unit: saved.unit,
            time: saved.time,
            command: saved.command,
            choices: saved.choices,
            model_intent: saved.model_intent,
            approved_version: saved.approved_version.unwrap_or(match (saved.modified_dates, saved.subfolder_opt_in) {
                (true, true) => 2,
                (true, false) => 1,
                (false, _) => 0,
            }),
            approved_source: saved.approved_source,
            approved_nicknames: saved.approved_nicknames,
        }
    }
}

/// The user's folder nicknames as (name, path) pairs, as a rule records them.
pub fn nickname_pairs(settings: &crate::settings::Settings) -> Vec<(String, String)> {
    settings.nicknames.iter().map(|n| (n.name.clone(), n.path.clone())).collect()
}

/// True when `text` uses `name` as whole words, ignoring case.
fn names_word(text: &str, name: &str) -> bool {
    let (text, name) = (text.to_lowercase(), name.to_lowercase());
    text.match_indices(&name).any(|(i, _)| {
        let before = text[..i].chars().next_back().is_none_or(|c| !c.is_alphanumeric());
        let after = text[i + name.len()..].chars().next().is_none_or(|c| !c.is_alphanumeric());
        before && after
    })
}

/// The shortest gap allowed between runs of a repeating task.
pub const MIN_REPEAT_MINUTES: i64 = 15;

fn local_at(date: NaiveDate, time: NaiveTime) -> Option<DateTime<Local>> {
    let at = NaiveDateTime::new(date, time);
    // A skipped daylight-saving hour moves to the first valid moment after it.
    Local.from_local_datetime(&at).earliest().or_else(|| Local.from_local_datetime(&(at + Duration::hours(1))).earliest())
}

/// True when any filter in `intent` reads the last-modified date.
fn reads_modified_date(intent: &crate::model::Intent) -> bool {
    fn walk(v: &serde_json::Value) -> bool {
        match v {
            serde_json::Value::Object(m) => m.get("dateField").is_some_and(|f| f == "modified") || m.values().any(walk),
            serde_json::Value::Array(a) => a.iter().any(walk),
            _ => false,
        }
    }
    serde_json::to_value(intent).is_ok_and(|v| walk(&v))
}

impl Rule {
    pub fn check_approval(&self, settings: &crate::settings::Settings, command: &str) -> Result<(), String> {
        let commands = match &self.approved_source {
            Some(RuleSource::Shortcut { commands, .. }) => commands.as_slice(),
            Some(RuleSource::Command) => std::slice::from_ref(&self.command),
            // Legacy shortcut definitions may explain a changed date or scope rule,
            // but they can never be treated as approved execution inputs.
            None => settings.find_shortcut(&self.command).map_or(std::slice::from_ref(&self.command), |s| s.commands.as_slice()),
        };
        if self.approved_version < 2 && commands.iter().any(|text| crate::parse::names_subfolders(text)) {
            return Err(format!("Skipped: Voxi now reads “subfolders” differently. Schedule “{command}” again to keep this task."));
        }
        if self.approved_version < 1
            && commands.iter().any(|text| crate::parse::parse(text).is_ok_and(|intent| reads_modified_date(&intent)))
        {
            return Err(format!(
                "Skipped: “modified” now means the date a file was last edited. Schedule “{command}” again to keep this task."
            ));
        }
        if let Some(approved) = &self.approved_nicknames {
            let current: Vec<(String, String)> = nickname_pairs(settings);
            let used = |name: &str| commands.iter().any(|text| names_word(text, name));
            let meaning =
                |list: &[(String, String)], name: &str| list.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, p)| p.clone());
            if let Some((name, _)) =
                approved.iter().chain(current.iter()).find(|(name, _)| used(name) && meaning(approved, name) != meaning(&current, name))
            {
                return Err(format!("The folder name “{name}” now means another folder. Schedule “{command}” again to keep this task."));
            }
        }
        if self.approved_version > APPROVED_RULE_VERSION {
            return Err("This task was approved in a newer version of Voxi. Schedule it again to keep this task.".into());
        }
        match &self.approved_source {
            Some(RuleSource::Command) => Ok(()),
            Some(RuleSource::Shortcut { phrase, commands })
                if settings.shortcuts.iter().any(|s| s.phrase == *phrase && s.commands == *commands) =>
            {
                Ok(())
            }
            Some(RuleSource::Shortcut { phrase, .. }) => {
                Err(format!("The shortcut “{phrase}” was changed or removed. Schedule it again to keep this task."))
            }
            None => Err("Voxi couldn't verify the original command or shortcut. Schedule it again to keep this task.".into()),
        }
    }

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
            if guard > 1_000_000 {
                return None;
            }
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
    if lower == "noon" {
        return Ok(NaiveTime::from_hms_opt(12, 0, 0).unwrap());
    }
    if lower == "midnight" {
        return Ok(NaiveTime::from_hms_opt(0, 0, 0).unwrap());
    }
    let clock = CLOCK
        .captures(value)
        .filter(|c| c.get(1).is_none() && c.get(5).is_none())
        .ok_or("Use a time such as “at 9:40 pm” or “at 21:40”.")?;
    let mut hour: u32 = clock[2].parse().map_err(|_| "That clock time isn't valid.")?;
    let minute: u32 = clock.get(3).map_or("0", |m| m.as_str()).parse().map_err(|_| "That clock time isn't valid.")?;
    if let Some(period) = clock.get(4) {
        if !(1..=12).contains(&hour) {
            return Err("Use an hour from 1 to 12 with am or pm.".into());
        }
        hour = hour % 12 + if period.as_str().to_ascii_lowercase().starts_with('p') { 12 } else { 0 };
    } else if clock.get(3).is_none() {
        return Err("Include am or pm, or use a 24-hour time such as 21:40.".into());
    }
    NaiveTime::from_hms_opt(hour, minute, 0).ok_or_else(|| "That clock time isn't valid.".into())
}

fn weekday(name: &str) -> Option<Weekday> {
    Some(match &name.to_ascii_lowercase()[..3] {
        "mon" => Weekday::Mon,
        "tue" => Weekday::Tue,
        "wed" => Weekday::Wed,
        "thu" => Weekday::Thu,
        "fri" => Weekday::Fri,
        "sat" => Weekday::Sat,
        "sun" => Weekday::Sun,
        _ => return None,
    })
}

/// What confirmation does with the approved preview.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum FirstRunPolicy {
    Immediate,
    OneTimeDeferred,
    RepeatLater,
    RunNowAndRepeat,
}

/// The first future occurrence and, for a repeat, its cadence.
#[derive(Clone, Debug, PartialEq)]
pub struct When {
    pub due: DateTime<Local>,
    pub rule: Option<Rule>,
    pub first_run: FirstRunPolicy,
    anchor: Option<RepeatAnchor>,
}

impl When {
    pub fn once(due: DateTime<Local>) -> Self {
        Self { due, rule: None, first_run: FirstRunPolicy::OneTimeDeferred, anchor: None }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
enum RepeatAnchor {
    Interval,
    Weekday(Weekday),
}

impl RepeatAnchor {
    fn next(self, rule: &mut Rule, now: DateTime<Local>) -> Option<DateTime<Local>> {
        match self {
            Self::Interval => {
                if matches!(rule.unit, Unit::Days | Unit::Weeks) {
                    rule.time = Some(now.time().with_nanosecond(0).unwrap_or(now.time()));
                }
                rule.step(now)
            }
            Self::Weekday(day) => next_weekday(day, rule.time?, now),
        }
    }
}

fn next_weekday(day: Weekday, time: NaiveTime, now: DateTime<Local>) -> Option<DateTime<Local>> {
    let ahead = (7 + i64::from(day.num_days_from_monday()) - i64::from(now.weekday().num_days_from_monday())) % 7;
    let date = now.date_naive().checked_add_signed(Duration::days(ahead))?;
    let due = local_at(date, time)?;
    if due > now { Some(due) } else { local_at(date.checked_add_signed(Duration::days(7))?, time) }
}

pub(crate) fn check_unscheduled(text: &str) -> Result<(), String> {
    natural::check_residual(text)
}

/// Finds “every 3 days”, “everyday”, “daily at 9 pm”, “every Monday at 8:30 am”, or a
/// one-time “at 9:40 pm” / “after 3 hours” at the end of a command.
pub fn extract_schedule(text: &str, now: DateTime<Local>) -> Result<(String, Option<When>), String> {
    crate::parse::check_length(text).map_err(|_| "That command is too long.".to_string())?;
    let normalized = natural::normalize(text)?;
    let text = normalized.as_str();
    if let Some((command, when)) = extract_repeat(text, now)? {
        natural::check_residual(&command)?;
        return Ok((command, Some(when)));
    }
    let (command, due) = extract(text, now)?;
    natural::check_residual(&command)?;
    Ok((command, due.map(When::once)))
}

fn extract_repeat(text: &str, now: DateTime<Local>) -> Result<Option<(String, When)>, String> {
    let masked = crate::parse::phrase::unquoted_text(text).ok_or("Close the quotation marks first.")?;
    let Some(c) = RECUR.captures(&masked) else { return Ok(None) };
    let whole = c.get(0).unwrap();
    if masked[whole.start()..] != text[whole.start()..] {
        return Ok(None);
    }
    let command = text[..whole.start()].trim().to_string();
    if command.is_empty() {
        return Err("Say what to do before how often.".into());
    }

    let (every, unit, day) = if let Some(adv) = c.name("adv") {
        let unit = match adv.as_str().to_ascii_lowercase().as_str() {
            "hourly" => Unit::Hours,
            "daily" | "everyday" => Unit::Days,
            _ => Unit::Weeks,
        };
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
        let unit = if raw.starts_with('m') {
            Unit::Minutes
        } else if raw.starts_with('h') {
            Unit::Hours
        } else if raw.starts_with('d') {
            Unit::Days
        } else {
            Unit::Weeks
        };
        (every, unit, None)
    };
    if !(1..=365).contains(&every) {
        return Err("Choose how often between once and 365.".into());
    }
    let every = every as u32;
    let minutes = i64::from(every)
        * match unit {
            Unit::Minutes => 1,
            Unit::Hours => 60,
            Unit::Days => 1440,
            Unit::Weeks => 10080,
        };
    if minutes < MIN_REPEAT_MINUTES {
        return Err(format!("Repeat at most every {MIN_REPEAT_MINUTES} minutes."));
    }

    let time = match (c.name("t1"), c.name("t2")) {
        (Some(_), Some(_)) => return Err("Mention the time once.".into()),
        (Some(t), None) | (None, Some(t)) => Some(clock_time(t.as_str())?),
        (None, None) => None,
    };
    if time.is_some() && matches!(unit, Unit::Minutes | Unit::Hours) {
        return Err("A time of day works with days or weeks, such as “every day at 9 pm”.".into());
    }

    let mut rule = Rule {
        every,
        unit,
        time,
        command: command.clone(),
        choices: HashMap::new(),
        model_intent: None,
        approved_version: APPROVED_RULE_VERSION,
        approved_source: Some(RuleSource::Command),
        approved_nicknames: None,
    };
    let anchor = time.is_none().then(|| day.map_or(RepeatAnchor::Interval, RepeatAnchor::Weekday));
    let due = match (day, time) {
        (Some(day), time) => {
            let time = time.unwrap_or_else(|| NaiveTime::from_hms_opt(9, 0, 0).unwrap());
            rule.time = Some(time);
            next_weekday(day, time, now)
        }
        (None, Some(time)) => {
            let date = now.date_naive();
            local_at(date, time).and_then(|due| if due > now { Some(due) } else { local_at(date.succ_opt()?, time) })
        }
        (None, None) => RepeatAnchor::Interval.next(&mut rule, now),
    }
    .ok_or("That time is too far away.")?;
    let first_run = if anchor.is_some() { FirstRunPolicy::RunNowAndRepeat } else { FirstRunPolicy::RepeatLater };
    Ok(Some((command, When { due, rule: Some(rule), first_run, anchor })))
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
        let m = fs::symlink_metadata(path).map_err(|e| format!("Couldn't check {}: {e}", path.display()))?;
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

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SeriesState {
    Active,
    Paused,
    Stopped,
    Completed,
    Failed,
    FirstRunPending,
    NeedsReview,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum OccurrenceState {
    Waiting,
    Running,
    Finished,
    ResultUnsaved,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RecordedResult {
    series: SeriesState,
    due: DateTime<Local>,
    summary: String,
}

pub enum Completion {
    OneTime { series: SeriesState, summary: String },
    Recurring(String),
    Stopped(String),
    NeedsReview(String),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", from = "SavedJob")]
pub struct Job {
    pub id: u64,
    pub command: String,
    pub due: DateTime<Local>,
    pub series: SeriesState,
    pub occurrence: OccurrenceState,
    pub summary: String,
    pub actions: Vec<Action>,
    stamps: Vec<Stamp>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repeat: Option<Rule>,
    pub first_run: FirstRunPolicy,
    #[serde(skip_serializing_if = "Option::is_none")]
    anchor: Option<RepeatAnchor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<RecordedResult>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SavedJob {
    id: u64,
    command: String,
    due: DateTime<Local>,
    #[serde(flatten)]
    state: SavedState,
    summary: String,
    actions: Vec<Action>,
    stamps: Vec<Stamp>,
    repeat: Option<Rule>,
    first_run: Option<FirstRunPolicy>,
    anchor: Option<RepeatAnchor>,
    result: Option<RecordedResult>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum SavedState {
    Current { series: SeriesState, occurrence: OccurrenceState },
    Legacy { status: LegacyStatus },
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
enum LegacyStatus {
    Pending,
    Running,
    Paused,
    Cancelled,
    Completed,
    Failed,
}

impl From<SavedJob> for Job {
    fn from(saved: SavedJob) -> Self {
        use OccurrenceState::{Finished, Running, Waiting};
        use SeriesState::{Active, Completed, Failed, Paused, Stopped};
        let (series, occurrence) = match saved.state {
            SavedState::Current { series, occurrence } => (series, occurrence),
            SavedState::Legacy { status } => match status {
                LegacyStatus::Pending => (Active, Waiting),
                LegacyStatus::Running => (Active, Running),
                LegacyStatus::Paused => (Paused, Waiting),
                LegacyStatus::Cancelled => (Stopped, Finished),
                LegacyStatus::Completed => (Completed, Finished),
                LegacyStatus::Failed => (Failed, Finished),
            },
        };
        Self {
            id: saved.id,
            command: saved.command,
            due: saved.due,
            series,
            occurrence,
            summary: saved.summary,
            actions: saved.actions,
            stamps: saved.stamps,
            first_run: saved.first_run.unwrap_or(if saved.repeat.is_some() {
                FirstRunPolicy::RepeatLater
            } else {
                FirstRunPolicy::OneTimeDeferred
            }),
            repeat: saved.repeat,
            anchor: saved.anchor,
            result: saved.result,
        }
    }
}

/// Actions a scheduled task may run. Undo steps and Finder commands may not.
pub fn check_actions(actions: &[Action]) -> Result<(), String> {
    for a in actions {
        if matches!(
            a,
            Action::OpenInFinder { .. } | Action::RemoveEmptyFolder { .. } | Action::RestoreFromTrash { .. } | Action::Restore { .. }
        ) {
            return Err("Undo and Finder commands cannot be scheduled.".into());
        }
    }
    Ok(())
}

impl Job {
    /// A repeating task keeps no file list. Each run uses what matches then.
    pub fn recurring(command: String, when: &When, rule: Rule) -> Self {
        let (series, occurrence) = if when.first_run == FirstRunPolicy::RunNowAndRepeat {
            (SeriesState::FirstRunPending, OccurrenceState::Running)
        } else {
            (SeriesState::Active, OccurrenceState::Waiting)
        };
        Self {
            id: 0,
            command,
            due: when.due,
            series,
            occurrence,
            summary: String::new(),
            actions: Vec::new(),
            stamps: Vec::new(),
            repeat: Some(rule),
            first_run: when.first_run,
            anchor: when.anchor,
            result: None,
        }
    }

    #[cfg(test)]
    pub fn recurring_later(command: String, due: DateTime<Local>, rule: Rule) -> Self {
        Self::recurring(command, &When { due, rule: None, first_run: FirstRunPolicy::RepeatLater, anchor: None }, rule)
    }

    pub fn anchor_first_run(&mut self, now: DateTime<Local>) -> Result<(), String> {
        let rule = self.repeat.as_mut().ok_or("That task doesn't repeat.")?;
        self.due = self.anchor.and_then(|anchor| anchor.next(rule, now)).ok_or("That time is too far away.")?;
        Ok(())
    }

    #[cfg(test)]
    pub fn new(command: String, due: DateTime<Local>, actions: Vec<Action>) -> Result<Self, String> {
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
                return Err("Checking scheduled files took too long. Choose a smaller group.".to_string());
            }
            Ok(())
        };
        let mut paths = Vec::new();
        // Track outputs created by earlier actions in the plan so later steps
        // that consume them are not stamped before they exist on disk.
        let mut produced: Vec<(PathBuf, bool)> = Vec::new();
        let is_produced = |path: &Path, produced: &[(PathBuf, bool)]| {
            let key = crate::fsx::canonical(path);
            produced.iter().any(|(out, includes_descendants)| key == *out || (*includes_descendants && key.starts_with(out)))
        };
        let mut push_input = |path: &Path, produced: &[(PathBuf, bool)]| {
            if !is_produced(path, produced) {
                paths.push(path.to_path_buf());
            }
        };
        for a in &actions {
            match a {
                Action::Move { from, to } | Action::Copy { from, to } => {
                    push_input(from, &produced);
                    produced.push((crate::fsx::canonical(to), true));
                }
                Action::Rename { path, new_name } => {
                    push_input(path, &produced);
                    produced.push((crate::fsx::canonical(&path.with_file_name(new_name)), true));
                }
                Action::Trash { path } => push_input(path, &produced),
                Action::ConvertImage { path, output, .. } | Action::ExtractPages { path, output, .. } | Action::Zip { path, output } => {
                    push_input(path, &produced);
                    produced.push((crate::fsx::canonical(output), false));
                }
                Action::ResizeImage { path, backup, .. } => {
                    push_input(path, &produced);
                    produced.push((crate::fsx::canonical(backup), false));
                }
                Action::SplitPdf { path, output_dir } => {
                    push_input(path, &produced);
                    produced.push((crate::fsx::canonical(output_dir), true));
                }
                Action::Unzip { path, destination } => {
                    push_input(path, &produced);
                    produced.push((crate::fsx::canonical(destination), true));
                }
                Action::ImagesToPdf { paths: inputs, output } | Action::MergePdf { paths: inputs, output } => {
                    for input in inputs {
                        push_input(input, &produced);
                    }
                    produced.push((crate::fsx::canonical(output), false));
                }
                Action::CreateFolder { path } => {
                    if path.symlink_metadata().is_err() {
                        produced.push((crate::fsx::canonical(path), false));
                    }
                }
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
                let children = fs::read_dir(&path).map_err(|e| format!("Couldn't check {}: {e}", path.display()))?;
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
            series: SeriesState::Active,
            occurrence: OccurrenceState::Waiting,
            summary: String::new(),
            actions,
            stamps,
            repeat: None,
            first_run: FirstRunPolicy::OneTimeDeferred,
            anchor: None,
            result: None,
        })
    }
    pub fn validate(&self) -> Result<(), String> {
        for stamp in &self.stamps {
            if Stamp::capture(&stamp.path).as_ref() != Ok(stamp) {
                return Err(format!("{} changed or disappeared since scheduling. Schedule it again.", stamp.path.display()));
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
    pub series: SeriesState,
    pub occurrence: OccurrenceState,
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
            series: self.series,
            occurrence: self.occurrence,
            summary: self.summary.clone(),
            repeat: self.repeat.as_ref().map(|rule| rule.describe(self.due)),
        }
    }
}

pub struct Store {
    path: PathBuf,
    pub jobs: Vec<Job>,
    dirty: bool,
    notice: Option<String>,
    /// A save that reached disk but couldn't be confirmed durable: the new
    /// state is kept, and this warning shown.
    durability: std::cell::RefCell<Option<String>>,
    unavailable: Option<String>,
}

impl Store {
    pub fn load(path: PathBuf) -> Self {
        let (jobs, warning) = crate::settings::load_store::<Vec<Job>>(&path, crate::settings::diagnostics::Store::Schedules);
        if let Some(warning) = warning {
            return Self {
                path,
                jobs: Vec::new(),
                dirty: false,
                notice: None,
                durability: Default::default(),
                unavailable: Some(format!("{} Scheduling is unavailable; original data has been kept.", warning.message)),
            };
        }
        let mut store = Self { path, jobs, dirty: true, notice: None, durability: Default::default(), unavailable: None };
        for job in &mut store.jobs {
            if job.occurrence == OccurrenceState::ResultUnsaved && job.result.is_some() {
                continue;
            }
            if job.series == SeriesState::FirstRunPending
                || matches!(job.occurrence, OccurrenceState::Running | OccurrenceState::ResultUnsaved)
            {
                crate::settings::diagnostics::emit(
                    crate::settings::diagnostics::Event::new(
                        crate::settings::diagnostics::Code::SchedulerSkip,
                        crate::settings::diagnostics::Stage::Interrupted,
                    )
                    .operation(job.id),
                );
                job.summary = if job.series == SeriesState::FirstRunPending {
                    "Voxi closed before recording the first run. Check your files and History before scheduling again.".into()
                } else {
                    "Voxi closed before recording the last run. Check your files and History before scheduling again.".into()
                };
                job.series = SeriesState::NeedsReview;
                job.occurrence = OccurrenceState::Finished;
                job.result = None;
            }
        }
        let _ = store.retry_results();
        store
    }

    pub fn notice(&self) -> Option<String> {
        self.unavailable.clone().or_else(|| self.notice.clone()).or_else(|| self.durability.borrow().clone())
    }

    fn save_jobs(&self, jobs: &[Job]) -> Result<(), String> {
        if let Some(error) = &self.unavailable {
            crate::settings::diagnostics::emit(
                crate::settings::diagnostics::Event::new(
                    crate::settings::diagnostics::Code::StoreSave,
                    crate::settings::diagnostics::Stage::Failed,
                )
                .store(crate::settings::diagnostics::Store::Schedules),
            );
            return Err(error.clone());
        }
        // A save the disk couldn't confirm keeps its warning until a later
        // save (retried each tick) is confirmed.
        *self.durability.borrow_mut() =
            match crate::settings::save_store(&self.path, &jobs, crate::settings::diagnostics::Store::Schedules)? {
                crate::settings::Durability::Durable => None,
                crate::settings::Durability::Uncertain(warning) => Some(warning),
            };
        Ok(())
    }

    /// Persist the result with the original occurrence before publishing its
    /// successor. After a restart either the staged or final result is safe.
    pub fn retry_results(&mut self) -> Result<(), String> {
        // Save again until the disk confirms the last change (a cancelled
        // task must stay cancelled after a crash).
        if self.durability.borrow().is_some() {
            self.dirty = true;
        }
        let result = self.flush_results();
        self.notice = result.as_ref().err().map(|e| format!("Couldn't save scheduled task state. Voxi will retry. {e}"));
        result
    }

    fn flush_results(&mut self) -> Result<(), String> {
        if self.dirty {
            self.save_jobs(&self.jobs)?;
            self.dirty = false;
        }
        if self.jobs.iter().any(|job| job.occurrence == OccurrenceState::ResultUnsaved) {
            let mut saved = self.jobs.clone();
            for job in &mut saved {
                if job.occurrence == OccurrenceState::ResultUnsaved {
                    let result = job.result.take().ok_or("Missing scheduled task result.")?;
                    job.series = result.series;
                    job.occurrence =
                        if result.series == SeriesState::Active { OccurrenceState::Waiting } else { OccurrenceState::Finished };
                    job.due = result.due;
                    job.summary = result.summary;
                }
            }
            self.save_jobs(&saved)?;
            self.jobs = saved;
        }
        Ok(())
    }

    pub fn add(&mut self, mut job: Job) -> Result<u64, String> {
        self.retry_results()?;
        let id = self.jobs.iter().map(|j| j.id).max().unwrap_or(0).checked_add(1).ok_or("Too many scheduled tasks.")?;
        job.id = id;
        self.jobs.push(job);
        if let Err(e) = self.save_jobs(&self.jobs) {
            self.jobs.pop();
            return Err(e);
        }
        Ok(id)
    }

    fn change(&mut self, id: u64, change: impl FnOnce(&mut Job) -> Result<(), String>) -> Result<(), String> {
        self.retry_results()?;
        let index = self.jobs.iter().position(|j| j.id == id).ok_or("That task no longer exists.")?;
        let mut next = self.jobs[index].clone();
        change(&mut next)?;
        let old = std::mem::replace(&mut self.jobs[index], next);
        if let Err(e) = self.save_jobs(&self.jobs) {
            self.jobs[index] = old;
            return Err(e);
        }
        Ok(())
    }

    pub fn cancel(&mut self, id: u64) -> Result<(), String> {
        self.change(id, |job| {
            // A task waiting for review can be retired too; it never resumes
            // without a fresh preview, so stopping it is its only way out.
            let reviewable = job.series == SeriesState::NeedsReview && job.occurrence != OccurrenceState::Running;
            if !reviewable
                && (job.occurrence != OccurrenceState::Waiting || !matches!(job.series, SeriesState::Active | SeriesState::Paused))
            {
                return Err("Only pending, paused or needs-review tasks can be cancelled.".into());
            }
            job.series = SeriesState::Stopped;
            job.occurrence = OccurrenceState::Finished;
            job.summary = if job.repeat.is_some() { "Stopped.".into() } else { "Cancelled before running.".into() };
            Ok(())
        })
    }

    pub fn pause(&mut self, id: u64) -> Result<(), String> {
        self.change(id, |job| {
            if job.series != SeriesState::Active || job.occurrence != OccurrenceState::Waiting || job.repeat.is_none() {
                return Err("Only waiting repeating tasks can be paused.".into());
            }
            job.series = SeriesState::Paused;
            Ok(())
        })
    }

    /// Runs missed while paused or stopped are skipped. A one-time task's
    /// approved files may have changed, so only repeat schedules can resume.
    pub fn resume(&mut self, id: u64, now: DateTime<Local>) -> Result<(), String> {
        self.change(id, |job| {
            if job.series == SeriesState::NeedsReview {
                return Err("Schedule it again to keep this task.".into());
            }
            if !matches!(job.series, SeriesState::Paused | SeriesState::Stopped | SeriesState::Failed) {
                return Err("Only paused, stopped or failed repeating tasks can be made active again.".into());
            }
            let rule = job.repeat.as_ref().ok_or("Only repeating tasks can be resumed.")?;
            if job.due <= now {
                job.due = rule.next_after(job.due, now).ok_or("That time is too far away.")?;
            }
            if job.series != SeriesState::Paused {
                job.summary = "Made active again.".into();
            }
            job.series = SeriesState::Active;
            job.occurrence = OccurrenceState::Waiting;
            Ok(())
        })
    }

    pub fn finish(&mut self, id: u64, completion: Completion, now: DateTime<Local>) -> Result<(), String> {
        let job = self.jobs.iter_mut().find(|j| j.id == id).ok_or("That task no longer exists.")?;
        let first = job.series == SeriesState::FirstRunPending;
        if job.occurrence != OccurrenceState::Running {
            return Err("That task has no run to finish.".into());
        }
        let mut due = job.due;
        let (series, summary) = match completion {
            Completion::OneTime { series, summary } => (series, summary),
            Completion::NeedsReview(summary) => (SeriesState::NeedsReview, summary),
            Completion::Stopped(summary) => (SeriesState::Stopped, summary),
            Completion::Recurring(mut summary) if first => {
                let series = if due <= now {
                    let rule = job.repeat.as_ref().ok_or("That task doesn't repeat.")?;
                    match rule.next_after(due, now) {
                        Some(next) => {
                            due = next;
                            SeriesState::Active
                        }
                        None => {
                            summary.push_str(" The next run is too far away.");
                            SeriesState::Failed
                        }
                    }
                } else {
                    SeriesState::Active
                };
                (series, summary)
            }
            Completion::Recurring(summary) => {
                let rule = job.repeat.as_ref().ok_or("That task doesn't repeat.")?;
                let mut summary = format!("Last run {}: {summary}", display(now));
                let series = match rule.next_after(due, now) {
                    Some(next) => {
                        due = next;
                        SeriesState::Active
                    }
                    None => {
                        summary.push_str(" The next run is too far away.");
                        SeriesState::Failed
                    }
                };
                (series, summary)
            }
        };
        job.summary = summary.clone();
        job.result = Some(RecordedResult { series, due, summary });
        job.occurrence = OccurrenceState::ResultUnsaved;
        self.dirty = true;
        self.retry_results()
    }

    pub fn claim(&mut self, now: DateTime<Local>) -> Result<Option<Job>, String> {
        self.retry_results()?;
        let job = self
            .jobs
            .iter()
            .filter(|j| j.series == SeriesState::Active && j.occurrence == OccurrenceState::Waiting && j.due <= now)
            .min_by_key(|j| j.due)
            .cloned();
        if let Some(job) = &job {
            self.change(job.id, |job| {
                job.occurrence = OccurrenceState::Running;
                job.summary = "Running…".into();
                Ok(())
            })?;
        }
        Ok(job)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_schedule_migration_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("schedules.json");
        let job = Job::new("legacy task".into(), now(), vec![]).unwrap();
        fs::write(&path, serde_json::to_vec(&vec![job]).unwrap()).unwrap();
        let store = Store::load(path.clone());
        assert!(store.notice().is_none());
        let json: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(json["version"], 1);
        assert_eq!(serde_json::to_value(&Store::load(path).jobs).unwrap(), serde_json::to_value(&store.jobs).unwrap());
    }

    #[test]
    fn future_schedules_preserved_and_disabled() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("schedules.json");
        let bytes = br#"{"version":99,"data":[]}"#;
        fs::write(&path, bytes).unwrap();
        let mut store = Store::load(path.clone());
        assert!(store.notice().unwrap().contains("Unsupported store version"));
        assert!(store.add(Job::new("x".into(), now(), vec![]).unwrap()).is_err());
        assert!(store.claim(now()).unwrap().is_none());
        assert_eq!(fs::read(&path).unwrap(), bytes);
        let backup = fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().path()).find(|p| p != &path).unwrap();
        assert_eq!(fs::read(backup).unwrap(), bytes);
    }

    #[test]
    fn missing_schedule_and_failed_writes_are_safe() {
        use crate::settings::WRITE_FAILURE;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("schedules.json");
        let mut store = Store::load(path.clone());
        assert!(store.notice().is_none());
        assert!(store.jobs.is_empty());
        let original = fs::read(&path).unwrap();
        for stage in 1..=3 {
            WRITE_FAILURE.with(|f| f.set(stage));
            let result = store.add(Job::new("x".into(), now(), vec![]).unwrap());
            WRITE_FAILURE.with(|f| f.set(0));
            assert!(result.is_err());
            assert!(store.jobs.is_empty());
            assert_eq!(fs::read(&path).unwrap(), original);
            assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
        }
    }

    fn now() -> DateTime<Local> {
        Local.with_ymd_and_hms(2026, 11, 12, 20, 0, 0).unwrap()
    }
    #[test]
    fn day_before_at_belongs_to_the_time() {
        let (cmd, due) = extract("move pdfs to Archive tomorrow at 9am", now()).unwrap();
        assert_eq!(cmd, "move pdfs to Archive");
        assert_eq!(due.unwrap(), Local.with_ymd_and_hms(2026, 11, 13, 9, 0, 0).unwrap());
        let (cmd, due) = extract("move pdfs to Archive today at 9 pm", now()).unwrap();
        assert_eq!(cmd, "move pdfs to Archive");
        assert_eq!(due.unwrap(), Local.with_ymd_and_hms(2026, 11, 12, 21, 0, 0).unwrap());
        assert!(extract("move pdfs to Archive tomorrow at 9am tomorrow", now()).is_err());
        let (cmd, _) = extract("delete files from today at 9 pm", now()).unwrap();
        assert_eq!(cmd, "delete files from today");
    }
    #[test]
    fn parses_clock_and_delays() {
        let (cmd, due) = extract("Move screenshots from desktop to Screenshots folder at 9:40 pm", now()).unwrap();
        assert_eq!(cmd, "Move screenshots from desktop to Screenshots folder");
        assert_eq!(due.unwrap().format("%H:%M").to_string(), "21:40");
        for (suffix, minutes) in [("after 3 hours", 180), ("in 20 minutes", 20), ("after three hours", 180), ("after 1 minute", 1)] {
            assert_eq!(
                extract(&format!("move a.pdf to Documents {suffix}"), now()).unwrap().1.unwrap(),
                now() + Duration::minutes(minutes)
            );
        }
        assert_eq!(
            extract("move a.pdf to Documents at 19:00", now()).unwrap().1.unwrap().date_naive(),
            now().date_naive().succ_opt().unwrap()
        );
        assert_eq!(extract("move a.pdf to Documents at 12 am tomorrow", now()).unwrap().1.unwrap().format("%H:%M").to_string(), "00:00");
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
            assert!(extract(&format!("move a.pdf to Documents {suffix}"), now()).is_err(), "{suffix}");
        }
        assert!(extract("rename a.pdf to \"report at 9:40 pm\"", now()).unwrap().1.is_none());
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
            assert_eq!(extract(command, now()).unwrap(), (command.into(), None), "{command}");
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
        for suffix in ["after twenty minutes", "in twenty-one minutes", "after ninety nine minutes"] {
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
            ("everyday", 1, Unit::Days, Some(at(20, 0))),
            ("everyday at 9 pm", 1, Unit::Days, Some(at(21, 0))),
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
        assert!(extract_schedule("rename a.pdf to \"everyday\"", now()).unwrap().1.is_none());
        assert!(extract_schedule("move a.pdf to Documents at 9:40 pm", now()).unwrap().1.unwrap().rule.is_none());
    }
    #[test]
    fn repeats_follow_the_schedule_and_collapse_missed_runs() {
        let rule = Rule {
            every: 3,
            unit: Unit::Days,
            time: Some(NaiveTime::from_hms_opt(21, 0, 0).unwrap()),
            command: "x".into(),
            choices: HashMap::new(),
            model_intent: None,
            approved_version: APPROVED_RULE_VERSION,
            approved_source: Some(RuleSource::Command),
            approved_nicknames: None,
        };
        let first = Local.with_ymd_and_hms(2026, 11, 1, 21, 0, 0).unwrap();
        // Voxi was closed for ten days: one run now, then back on the 3-day grid.
        let next = rule.next_after(first, now()).unwrap();
        assert_eq!(next, Local.with_ymd_and_hms(2026, 11, 13, 21, 0, 0).unwrap());
        assert_eq!(rule.describe(first), "every 3 days at 9:00 PM");
        let weekly = Rule { every: 1, unit: Unit::Weeks, ..rule.clone() };
        assert_eq!(weekly.describe(Local.with_ymd_and_hms(2026, 11, 16, 21, 0, 0).unwrap()), "every Monday at 9:00 PM");
    }
    #[test]
    fn interrupted_repeats_need_review_after_reload() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("jobs.json");
        let mut store = Store::load(path.clone());
        let rule = Rule {
            every: 1,
            unit: Unit::Hours,
            time: None,
            command: "move a.pdf to Documents".into(),
            choices: HashMap::new(),
            model_intent: None,
            approved_version: APPROVED_RULE_VERSION,
            approved_source: Some(RuleSource::Command),
            approved_nicknames: None,
        };
        store.add(Job::recurring_later("move a.pdf to Documents hourly".into(), now(), rule.clone())).unwrap();
        assert_eq!(store.claim(Local::now()).unwrap().unwrap().id, 1);
        let mut store = Store::load(path.clone());
        assert_eq!(store.jobs[0].series, SeriesState::NeedsReview);
        assert_eq!(store.jobs[0].occurrence, OccurrenceState::Finished);
        assert!(store.jobs[0].summary.contains("Check your files and History"));
        assert!(store.claim(Local::now()).unwrap().is_none());
        assert_eq!(store.resume(1, now()).unwrap_err(), "Schedule it again to keep this task.");
        assert_eq!(store.jobs[0].repeat, Some(rule));
        assert_eq!(store.jobs[0].summary().repeat.as_deref(), Some("every hour"));
        // A task waiting for review can be removed, and then never runs.
        store.cancel(1).unwrap();
        assert_eq!(store.jobs[0].series, SeriesState::Stopped);
        assert!(store.claim(Local::now()).unwrap().is_none());
    }
    #[test]
    fn unreadable_schedule_data_is_preserved_without_blocking_startup() {
        for bytes in [b"{".as_slice(), b"not JSON".as_slice(), b"[{\"oldSchema\":true}]".as_slice(), &[0xff]] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("schedules.json");
            fs::write(&path, bytes).unwrap();
            let mut store = Store::load(path.clone());
            assert!(store.jobs.is_empty());
            assert!(store.notice().unwrap().contains("unreadable"));
            let backup = fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().path()).find(|p| p != &path).unwrap();
            assert_eq!(fs::read(backup).unwrap(), bytes);
            assert!(store.add(Job::new("new task".into(), now(), vec![]).unwrap()).is_err());
            assert_eq!(fs::read(&path).unwrap(), bytes);
            assert!(Store::load(path).jobs.is_empty());
        }
    }
    #[test]
    fn schedule_io_failures_disable_scheduling_without_overwriting_data() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("schedules.json");
        fs::create_dir(&path).unwrap();
        let mut store = Store::load(path.clone());
        assert!(store.notice().unwrap().contains("Couldn't read"));
        assert!(store.add(Job::new("new task".into(), now(), vec![]).unwrap()).is_err());
        assert!(path.is_dir());
        assert!(store.jobs.is_empty());
    }
    #[test]
    fn scheduled_folder_checks_honor_cancellation() {
        let control = crate::search::Control::default();
        control.cancel();
        let dir = tempfile::tempdir().unwrap();
        let actions = vec![Action::Move { from: dir.path().into(), to: dir.path().join("destination") }];
        let error = Job::new_with_control("move folder".into(), now(), actions, &control).unwrap_err();
        assert_eq!(error, "Search cancelled.");
    }
    #[test]
    fn persistence_cancellation_and_claim_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("jobs.json");
        let mut store = Store::load(path.clone());
        store.add(Job::new("create folder".into(), now(), vec![]).unwrap()).unwrap();
        assert!(store.claim(now() - Duration::minutes(1)).unwrap().is_none());
        assert_eq!(store.claim(now()).unwrap().unwrap().id, 1);
        assert!(store.claim(now()).unwrap().is_none());
        let mut store = Store::load(path);
        assert_eq!(store.jobs[0].series, SeriesState::NeedsReview);
        store.add(Job::new("create folder".into(), now(), vec![]).unwrap()).unwrap();
        store.cancel(2).unwrap();
        assert!(store.claim(now()).unwrap().is_none());
    }
    #[test]
    fn failed_result_save_is_retried_without_reclaiming_the_run() {
        use crate::settings::WRITE_FAILURE;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("schedules.json");
        let mut store = Store::load(path.clone());
        let id = store.add(Job::new("create folder".into(), now(), vec![]).unwrap()).unwrap();
        assert!(store.claim(now()).unwrap().is_some());

        WRITE_FAILURE.with(|f| f.set(2));
        let failed = store.finish(id, Completion::OneTime { series: SeriesState::Completed, summary: "Done.".into() }, now());
        assert!(failed.is_err());
        assert_eq!(store.jobs[0].occurrence, OccurrenceState::ResultUnsaved);
        assert!(store.claim(now()).is_err());
        WRITE_FAILURE.with(|f| f.set(0));

        assert!(store.claim(now()).unwrap().is_none());
        let reloaded = Store::load(path);
        assert_eq!(reloaded.jobs[0].series, SeriesState::Completed);
        assert_eq!(reloaded.jobs[0].occurrence, OccurrenceState::Finished);
        assert_eq!(reloaded.jobs[0].summary, "Done.");
    }

    #[test]
    fn first_run_pending_requires_review_unless_the_result_was_staged() {
        let dir = tempfile::tempdir().unwrap();
        let (_, when) = extract_schedule("move PDFs to Documents every day", now()).unwrap();
        let when = when.unwrap();
        let job = Job::recurring("move PDFs to Documents every day".into(), &when, when.rule.clone().unwrap());
        for recovered_result in [false, true] {
            let path = dir.path().join(format!("schedules-{recovered_result}.json"));
            let mut store = Store::load(path.clone());
            store.add(job.clone()).unwrap();
            assert!(store.claim(when.due).unwrap().is_none());
            if recovered_result {
                // The process stopped between saving the result and promoting
                // the series: the first save lands, the second never does.
                use crate::settings::{WRITE_FAILURE, WRITE_FAILURE_AFTER};
                WRITE_FAILURE.with(|f| f.set(3));
                WRITE_FAILURE_AFTER.with(|n| n.set(1));
                let result = store.finish(1, Completion::Recurring("Done.".into()), now());
                WRITE_FAILURE.with(|f| f.set(0));
                WRITE_FAILURE_AFTER.with(|n| n.set(0));
                assert!(result.is_err());
                assert_eq!(store.jobs[0].occurrence, OccurrenceState::ResultUnsaved);
            }
            let store = Store::load(path);
            let expected = if recovered_result { SeriesState::Active } else { SeriesState::NeedsReview };
            assert_eq!(store.jobs[0].series, expected);
            if recovered_result {
                assert_eq!(store.jobs[0].occurrence, OccurrenceState::Waiting);
                assert_eq!(store.jobs[0].summary, "Done.");
            }
        }
    }

    #[test]
    fn legacy_status_and_approval_versions_migrate() {
        use OccurrenceState::{Finished, Running, Waiting};
        use SeriesState::{Active, Completed, Failed, Paused, Stopped};
        for (status, series, occurrence) in [
            ("pending", Active, Waiting),
            ("running", Active, Running),
            ("paused", Paused, Waiting),
            ("cancelled", Stopped, Finished),
            ("completed", Completed, Finished),
            ("failed", Failed, Finished),
        ] {
            let value = serde_json::json!({
                "id": 1, "command": "create folder", "due": now(), "status": status, "summary": "", "actions": [], "stamps": []
            });
            let job: Job = serde_json::from_value(value).unwrap();
            assert_eq!((job.series, job.occurrence), (series, occurrence));
            assert_eq!(job.first_run, FirstRunPolicy::OneTimeDeferred);
            assert!(serde_json::to_value(job).unwrap().get("status").is_none());
        }
        for (old_flags, version) in [
            (serde_json::json!({}), 0),
            (serde_json::json!({"modifiedDates": true, "subfolderOptIn": false}), 1),
            (serde_json::json!({"modifiedDates": true, "subfolderOptIn": true}), 2),
            (serde_json::json!({"approvedVersion": 2}), 2),
        ] {
            let mut value = serde_json::json!({
                "every": 1, "unit": "days", "command": "create folder",
                "modelIntent": crate::model::Intent::CreateFolders { names: vec!["folder".into()], parent: None },
            });
            value.as_object_mut().unwrap().extend(old_flags.as_object().unwrap().clone());
            let rule: Rule = serde_json::from_value(value).unwrap();
            assert_eq!(rule.approved_version, version);
            assert!(rule.model_intent.is_some());
            assert!(rule.approved_source.is_none());
            let saved = serde_json::to_value(rule).unwrap();
            assert_eq!(saved["approvedVersion"], version);
            assert!(saved.get("modifiedDates").is_none());
            assert!(saved.get("subfolderOptIn").is_none());
            assert!(saved.get("modelIntent").is_none());
        }
    }

    #[test]
    fn names_with_at_are_preserved() {
        assert_eq!(extract("rename a.txt to Notes at Work", now()).unwrap(), ("rename a.txt to Notes at Work".into(), None));
    }
    #[test]
    fn scheduled_folders_reject_new_or_modified_children() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("folder");
        fs::create_dir(&src).unwrap();
        let child = src.join("child.txt");
        fs::write(&child, "original").unwrap();
        let job =
            Job::new("move folder".into(), now(), vec![Action::Move { from: src.clone(), to: dir.path().join("destination") }]).unwrap();
        job.validate().unwrap();
        fs::write(child, "changed contents").unwrap();
        assert!(job.validate().is_err());
        let job =
            Job::new("move folder".into(), now(), vec![Action::Move { from: src.clone(), to: dir.path().join("destination") }]).unwrap();
        fs::write(src.join("new.txt"), "new").unwrap();
        assert!(job.validate().is_err());
    }
    #[test]
    fn changed_sources_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("a.txt");
        fs::write(&src, "original").unwrap();
        let job = Job::new("move".into(), now(), vec![Action::Move { from: src.clone(), to: dir.path().join("b.txt") }]).unwrap();
        job.validate().unwrap();
        fs::write(&src, "changed contents").unwrap();
        assert!(job.validate().is_err());
    }
    #[test]
    fn a_changed_folder_nickname_sends_a_repeat_to_review() {
        use crate::settings::{Nickname, Settings};
        let with =
            |path: &str| Settings { nicknames: vec![Nickname { name: "Receipts".into(), path: path.into() }], ..Settings::default() };
        let rule = |command: &str| Rule {
            every: 1,
            unit: Unit::Days,
            time: None,
            command: command.into(),
            choices: HashMap::new(),
            model_intent: None,
            approved_version: APPROVED_RULE_VERSION,
            approved_source: Some(RuleSource::Command),
            approved_nicknames: Some(nickname_pairs(&with("~/Documents/Receipts"))),
        };
        let trash = rule("trash PDFs from Receipts");
        assert!(trash.check_approval(&with("~/Documents/Receipts"), "x").is_ok());
        let err = trash.check_approval(&with("~/Desktop/Other"), "x").unwrap_err();
        assert!(err.contains("“Receipts” now means another folder"), "{err}");
        assert!(trash.check_approval(&Settings::default(), "x").is_err(), "a removed nickname changes the meaning too");
        // A nickname the command doesn't use can change freely.
        assert!(rule("trash PDFs from Downloads").check_approval(&with("~/Desktop/Other"), "x").is_ok());
        assert!(rule("trash PDFs from ReceiptsArchive").check_approval(&with("~/Desktop/Other"), "x").is_ok());
    }

    #[test]
    fn scheduled_chain_skips_stamping_paths_produced_by_earlier_steps() {
        let dir = tempfile::tempdir().unwrap();
        let photo = dir.path().join("photo.png");
        let converted = dir.path().join("photo.jpg");
        let moved = dir.path().join("Pictures/photo.jpg");
        fs::write(&photo, b"png").unwrap();

        let actions = vec![
            Action::ConvertImage { path: photo.clone(), format: crate::model::ImageFormat::Jpg, output: converted.clone() },
            Action::Move { from: converted.clone(), to: moved },
        ];
        let job = Job::new("convert photo.png to jpg then move it to Pictures at 9 pm".into(), now(), actions).unwrap();
        assert_eq!(job.stamps.len(), 1);
        assert_eq!(job.stamps[0].path, photo);
        job.validate().unwrap();
        fs::write(&photo, b"edited").unwrap();
        assert!(job.validate().is_err());

        // Chained folder creation + move + zip skips stamping the newly created folder.
        let note = dir.path().join("note.txt");
        let box_dir = dir.path().join("Box");
        fs::write(&note, b"hi").unwrap();
        let chain = vec![
            Action::CreateFolder { path: box_dir.clone() },
            Action::Move { from: note.clone(), to: box_dir.join("note.txt") },
            Action::Zip { path: box_dir.clone(), output: dir.path().join("Box.zip") },
        ];
        let job = Job::new("move note into Box then zip Box at 9 pm".into(), now(), chain).unwrap();
        assert_eq!(job.stamps.len(), 1);
        assert_eq!(job.stamps[0].path, note);

        // A missing source NOT produced by any earlier step still fails.
        let bad = vec![
            Action::CreateFolder { path: dir.path().join("Empty") },
            Action::Move { from: dir.path().join("Empty/missing.txt"), to: dir.path().join("dest.txt") },
        ];
        assert!(Job::new("bad chain".into(), now(), bad).is_err());
    }

    #[test]
    fn first_run_recurring_advances_due_when_run_overruns_interval() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("schedules.json");
        let mut store = Store::load(path);
        let t0 = now();
        let (_, when) = extract_schedule("move PDFs to Documents every 30 minutes", t0).unwrap();
        let when = when.unwrap();
        let rule = when.rule.clone().unwrap();
        let mut job = Job::recurring("move PDFs to Documents every 30 minutes".into(), &when, rule);
        job.anchor_first_run(t0).unwrap();
        let initial_due = job.due;
        assert_eq!(initial_due, t0 + Duration::minutes(30));
        let id = store.add(job).unwrap();

        // First run took 45 minutes (> 30 minute interval), so due <= finished_at.
        let finished_at = t0 + Duration::minutes(45);
        store.finish(id, Completion::Recurring("Done.".into()), finished_at).unwrap();
        let saved = &store.jobs[0];
        assert_eq!(saved.series, SeriesState::Active);
        assert_eq!(saved.occurrence, OccurrenceState::Waiting);
        assert!(saved.due > finished_at, "due {} must advance past finished_at {}", saved.due, finished_at);
        assert_eq!(saved.due, t0 + Duration::minutes(60));
        assert!(store.claim(finished_at).unwrap().is_none(), "must not immediately fire again");
    }
}
