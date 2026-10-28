//! Calendar phrases: "before July 2026", "between March and May", "in 2025",
//! "on July 4th", "since last Monday", "this month". Each becomes a `since`
//! bound (first day, inclusive) and/or a `before` bound (first day after, not
//! included). Plain "today" and "yesterday" stay with the `day` field.

use super::phrase::take;
use crate::model::Filter;
use chrono::{Datelike, Duration, Local, NaiveDate, Weekday};
use regex::Regex;
use std::sync::LazyLock;

#[cfg(test)]
thread_local! {
    /// Lets the corpus pin "today", so years inferred from "July 4" are stable.
    pub static TEST_TODAY: std::cell::Cell<Option<NaiveDate>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
pub fn today() -> NaiveDate {
    TEST_TODAY.with(|t| t.get()).unwrap_or_else(|| Local::now().date_naive())
}

#[cfg(not(test))]
pub fn today() -> NaiveDate {
    Local::now().date_naive()
}

const MONTH: &str = "(?:january|february|march|april|may|june|july|august|september|october|november|december|jan|feb|mar|apr|jun|jul|aug|sept|sep|oct|nov|dec)";
const WEEKDAY: &str = "(?:monday|tuesday|wednesday|thursday|friday|saturday|sunday)";
const YEAR: &str = r"(?:19|20)\d{2}";

fn iso() -> String {
    r"\d{4}-\d{1,2}-\d{1,2}".to_string()
}

fn day_first() -> String {
    format!(r"\d{{1,2}}(?:st|nd|rd|th)?\s+(?:of\s+)?{MONTH}\b(?:,?\s*(?:of\s+)?{YEAR}\b)?")
}

fn month_day() -> String {
    format!(r"{MONTH}\s+\d{{1,2}}(?:st|nd|rd|th)?\b(?:,?\s*(?:of\s+)?{YEAR}\b)?")
}

fn month_year() -> String {
    format!(r"{MONTH}\b(?:,?\s*(?:of\s+)?{YEAR}\b)?")
}

fn weekday() -> String {
    format!(r"(?:last\s+)?{WEEKDAY}\b")
}

/// Dates that name one day or a whole year: "in 2025", "on July 4th". A bare
/// month ("in July") is left to the month field.
fn on_when() -> String {
    format!("(?:{}|{}|{}|{}|{YEAR}\\b)", iso(), weekday(), day_first(), month_day())
}

static RANGE: LazyLock<Regex> = LazyLock::new(|| {
    let when = when_all_with_month();
    Regex::new(&format!(r"(?i)\b(?:between|from)\s+({when})\s+(?:and|to|through|thru|till|until)\s+({when})")).unwrap()
});

static BOUND: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"(?i)\b(before|prior\s+to|earlier\s+than|after|later\s+than|since|until|till|through|thru|up\s+to|on\s+or\s+before|on\s+or\s+after)\s+({})",
        when_all_with_month()
    ))
    .unwrap()
});

static ON: LazyLock<Regex> = LazyLock::new(|| Regex::new(&format!(r"(?i)\b(?:on|in|during|from)\s+({})", on_when())).unwrap());

static THIS_PERIOD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\b(?:(?:from|since)\s+)?this\s+(week|month|year)\b").unwrap());

static ISO_DATE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(\d{4})-(\d{1,2})-(\d{1,2})$").unwrap());
static WEEKDAY_NAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(&format!(r"^(last\s+)?({WEEKDAY})$")).unwrap());
static YEAR_NAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(&format!(r"^({YEAR})$")).unwrap());
static CALENDAR: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(r"^(?:(\d{{1,2}})(?:st|nd|rd|th)?\s+(?:of\s+)?)?({MONTH})(?:\s+(\d{{1,2}})(?:st|nd|rd|th)?)?(?:,?\s*(?:of\s+)?({YEAR}))?$")).unwrap()
});

/// Every date form, with a bare month included (for "before July").
fn when_all_with_month() -> String {
    format!(r"(?:{}|{}|{}|{}|{}|{YEAR}\b|today\b|yesterday\b)", iso(), weekday(), day_first(), month_day(), month_year())
}

/// One parsed date phrase. Missing parts are filled in from context.
#[derive(Debug, Clone, Copy)]
enum When {
    Cal { year: Option<i32>, month: Option<u32>, day: Option<u32> },
    Weekday { wd: Weekday, last: bool },
    /// Days from today: 0 is today, -1 is yesterday.
    Relative(i64),
}

fn parse_when(text: &str) -> Option<When> {
    let t = text.trim().to_lowercase();
    match t.as_str() {
        "today" => return Some(When::Relative(0)),
        "yesterday" => return Some(When::Relative(-1)),
        _ => {}
    }
    if let Some(c) = ISO_DATE.captures(&t) {
        return Some(When::Cal { year: c[1].parse().ok(), month: c[2].parse().ok(), day: c[3].parse().ok() });
    }
    if let Some(c) = WEEKDAY_NAME.captures(&t) {
        return Some(When::Weekday { wd: weekday_of(&c[2])?, last: c.get(1).is_some() });
    }
    if let Some(c) = YEAR_NAME.captures(&t) {
        return Some(When::Cal { year: c[1].parse().ok(), month: None, day: None });
    }
    let c = CALENDAR.captures(&t)?;
    let day = c.get(1).or(c.get(3)).and_then(|d| d.as_str().parse().ok());
    let year = c.get(4).and_then(|y| y.as_str().parse().ok());
    Some(When::Cal { year, month: month_number(&c[2]), day })
}

fn month_number(name: &str) -> Option<u32> {
    const MONTHS: [&str; 12] = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];
    let key: String = name.chars().take(3).collect();
    MONTHS.iter().position(|m| *m == key).map(|i| i as u32 + 1)
}

fn weekday_of(name: &str) -> Option<Weekday> {
    Some(match name.get(..3)? {
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

fn first_of(year: i32, month: u32) -> Option<NaiveDate> {
    NaiveDate::from_ymd_opt(year, month, 1)
}

/// A month and day without a year: the most recent one not after today.
fn day_year_for(month: u32, day: u32, today: NaiveDate) -> i32 {
    if (month, day) <= (today.month(), today.day()) { today.year() } else { today.year() - 1 }
}

/// The days a phrase covers, as (first day, first day after).
fn span(when: When, today: NaiveDate) -> Option<(NaiveDate, NaiveDate)> {
    match when {
        When::Relative(n) => {
            let d = today + Duration::days(n);
            Some((d, d + Duration::days(1)))
        }
        When::Weekday { wd, last } => {
            // "last Monday" starts from yesterday; "on Monday" includes today.
            let back: i64 = if last { 1 } else { 0 };
            (back..back + 7).map(|i| today - Duration::days(i)).find(|d| d.weekday() == wd).map(|d| (d, d + Duration::days(1)))
        }
        When::Cal { year, month, day } => match (year, month, day) {
            (year, Some(m), Some(d)) => {
                let y = year.unwrap_or_else(|| day_year_for(m, d, today));
                let date = NaiveDate::from_ymd_opt(y, m, d)?;
                Some((date, date + Duration::days(1)))
            }
            (year, Some(m), None) => {
                let y = crate::plan::month_year(m, year, today);
                let next = if m == 12 { first_of(y + 1, 1)? } else { first_of(y, m + 1)? };
                Some((first_of(y, m)?, next))
            }
            (Some(y), None, None) => Some((first_of(y, 1)?, first_of(y + 1, 1)?)),
            _ => None,
        },
    }
}

/// Pulls calendar phrases out of `s` and records them on `f`. Runs after
/// "older than" and "newer than", and before the single-day and month fields.
pub fn extract(s: &mut String, f: &mut Filter) {
    let today = today();
    take(s, &RANGE, |c| {
        let (Some(mut a), Some(b)) = (parse_when(&c[1]), parse_when(&c[2])) else { return };
        // "March to May 2026": the first date takes the year of the second.
        if let When::Cal { year, .. } = &mut a {
            if let (None, When::Cal { year: Some(y), .. }) = (*year, b) {
                *year = Some(y);
            }
        }
        let (Some((start, _)), Some((_, end))) = (span(a, today), span(b, today)) else { return };
        f.since = Some(start);
        f.before = Some(end);
    });
    take(s, &BOUND, |c| {
        let Some((start, end)) = parse_when(&c[2]).and_then(|w| span(w, today)) else { return };
        match c[1].to_lowercase().as_str() {
            "before" | "prior to" | "earlier than" => f.before = Some(start),
            "until" | "till" | "through" | "thru" | "up to" | "on or before" => f.before = Some(end),
            "after" | "later than" => f.since = Some(end),
            _ => f.since = Some(start), // since, on or after
        }
    });
    take(s, &ON, |c| {
        let Some((start, end)) = parse_when(&c[1]).and_then(|w| span(w, today)) else { return };
        f.since = Some(start);
        f.before = Some(end);
    });
    take(s, &THIS_PERIOD, |c| {
        f.since = Some(match c[1].to_lowercase().as_str() {
            "week" => today - Duration::days(today.weekday().num_days_from_monday() as i64),
            "month" => today.with_day(1).unwrap_or(today),
            _ => NaiveDate::from_ymd_opt(today.year(), 1, 1).unwrap_or(today),
        });
    });
}
