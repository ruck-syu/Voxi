//! Calendar phrases: "before July 2026", "between March and May", "in 2025",
//! "on July 4th", "since last Monday", "this month". Each becomes a `since`
//! bound (first day, inclusive) and/or a `before` bound (first day after, not
//! included). Plain "today" and "yesterday" stay with the `day` field.
//!
//! A phrase naming a day that does not exist ("February 29" in 2026) makes
//! `extract` return `None`, so the command is refused rather than read without
//! its date.

use super::phrase::take;
use crate::model::Filter;
use chrono::{Datelike, Duration, Local, NaiveDate, Weekday};
use regex::Regex;
use std::sync::LazyLock;

#[cfg(test)]
thread_local! {
    /// Lets tests pin "today", so years inferred from "July 4" are stable.
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
const ISO: &str = r"\d{4}-\d{1,2}-\d{1,2}";

/// "4th of July 2026", "4 July": the year is optional.
fn day_first_pattern() -> String {
    format!(r"\d{{1,2}}(?:st|nd|rd|th)?\s+(?:of\s+)?{MONTH}\b(?:,?\s*(?:of\s+)?{YEAR}\b)?")
}

/// "July 4th, 2026": the year is optional.
fn month_day_pattern() -> String {
    format!(r"{MONTH}\s+\d{{1,2}}(?:st|nd|rd|th)?\b(?:,?\s*(?:of\s+)?{YEAR}\b)?")
}

/// "July 2026", or "July" alone.
fn month_year_pattern() -> String {
    format!(r"{MONTH}\b(?:,?\s*(?:of\s+)?{YEAR}\b)?")
}

/// "Monday", or "last Monday".
fn weekday_pattern() -> String {
    format!(r"(?:last\s+)?{WEEKDAY}\b")
}

/// Dates that name one day or a whole year: "in 2025", "on July 4th". A bare
/// month ("in July") is left to the month field.
fn day_or_year_pattern() -> String {
    format!(r"(?:{ISO}|{}|{}|{}|{YEAR}\b)", weekday_pattern(), day_first_pattern(), month_day_pattern())
}

/// Any date phrase, including a bare month, "today", and "yesterday".
fn any_date_pattern() -> String {
    format!(
        r"(?:{ISO}|{}|{}|{}|{}|{YEAR}\b|today\b|yesterday\b)",
        weekday_pattern(),
        day_first_pattern(),
        month_day_pattern(),
        month_year_pattern()
    )
}

static RANGE: LazyLock<Regex> = LazyLock::new(|| {
    let date = any_date_pattern();
    Regex::new(&format!(r"(?i)\b(?:between|from)\s+({date})\s+(?:and|to|through|thru|till|until)\s+({date})")).unwrap()
});

static BOUND: LazyLock<Regex> = LazyLock::new(|| {
    let date = any_date_pattern();
    Regex::new(&format!(
        r"(?i)\b(before|prior\s+to|earlier\s+than|after|later\s+than|since|until|till|through|thru|up\s+to|on\s+or\s+before|on\s+or\s+after)\s+({date})"
    ))
    .unwrap()
});

static ON: LazyLock<Regex> = LazyLock::new(|| Regex::new(&format!(r"(?i)\b(?:on|in|during|from)\s+({})", day_or_year_pattern())).unwrap());

/// "from 2024 onward": from that day on, with no end.
static ONWARD: LazyLock<Regex> = LazyLock::new(|| {
    let date = day_or_year_pattern();
    Regex::new(&format!(r"(?i)\bfrom\s+({date})\s+(?:onward|onwards|and\s+(?:later|after|up)|or\s+(?:later|after))\b")).unwrap()
});

/// "before this year", "after this month": the bounds of the current period.
static PERIOD_BOUND: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(before|prior\s+to|after|since|until|till|through)\s+this\s+(week|month|year)\b").unwrap()
});

/// "last Tuesday" with no "on" in front: the most recent one before today.
static LAST_WEEKDAY: LazyLock<Regex> = LazyLock::new(|| Regex::new(&format!(r"(?i)\blast\s+({WEEKDAY})\b")).unwrap());

/// "this week", "from this month": the current period.
static THIS_PERIOD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\b(?:(?:from|since)\s+)?this\s+(week|month|year)\b").unwrap());

static ISO_DATE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(\d{4})-(\d{1,2})-(\d{1,2})$").unwrap());
static WEEKDAY_NAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(&format!(r"^(last\s+)?({WEEKDAY})$")).unwrap());
static YEAR_NAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(&format!(r"^({YEAR})$")).unwrap());
static CALENDAR: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"^(?:(\d{{1,2}})(?:st|nd|rd|th)?\s+(?:of\s+)?)?({MONTH})(?:\s+(\d{{1,2}})(?:st|nd|rd|th)?)?(?:,?\s*(?:of\s+)?({YEAR}))?$"
    ))
    .unwrap()
});

/// One parsed date phrase. Parts the phrase leaves out ("March", "July 4") are
/// filled in from context.
#[derive(Debug, Clone, Copy)]
enum When {
    Calendar { year: Option<i32>, month: Option<u32>, day: Option<u32> },
    Weekday { wd: Weekday, last: bool },
    /// Days from today: 0 is today, -1 is yesterday.
    Relative(i64),
}

impl When {
    /// The year the phrase names, if it names one.
    fn year(self) -> Option<i32> {
        match self {
            When::Calendar { year, .. } => year,
            _ => None,
        }
    }

    /// A date with no year takes `year`: "March" in "March to May 2026".
    fn or_year(self, year: Option<i32>) -> When {
        match self {
            When::Calendar { year: None, month, day } => When::Calendar { year, month, day },
            other => other,
        }
    }
}

fn parse_when(text: &str) -> Option<When> {
    let t = text.trim().to_lowercase();
    match t.as_str() {
        "today" => return Some(When::Relative(0)),
        "yesterday" => return Some(When::Relative(-1)),
        _ => {}
    }
    if let Some(c) = ISO_DATE.captures(&t) {
        return Some(When::Calendar { year: c[1].parse().ok(), month: c[2].parse().ok(), day: c[3].parse().ok() });
    }
    if let Some(c) = WEEKDAY_NAME.captures(&t) {
        return Some(When::Weekday { wd: weekday_of(&c[2])?, last: c.get(1).is_some() });
    }
    if let Some(c) = YEAR_NAME.captures(&t) {
        return Some(When::Calendar { year: c[1].parse().ok(), month: None, day: None });
    }
    let c = CALENDAR.captures(&t)?;
    let day = c.get(1).or(c.get(3)).and_then(|d| d.as_str().parse().ok());
    let year = c.get(4).and_then(|y| y.as_str().parse().ok());
    Some(When::Calendar { year, month: month_number(&c[2]), day })
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

/// The first day of a month, and the first day of the month after it.
fn month_bounds(year: i32, month: u32) -> Option<(NaiveDate, NaiveDate)> {
    let next = if month == 12 { first_of(year + 1, 1)? } else { first_of(year, month + 1)? };
    Some((first_of(year, month)?, next))
}

/// The calendar week (from Monday), month, or year that holds `today`.
fn this_period(unit: &str, today: NaiveDate) -> Option<(NaiveDate, NaiveDate)> {
    match unit {
        "week" => {
            let start = today - Duration::days(i64::from(today.weekday().num_days_from_monday()));
            Some((start, start + Duration::days(7)))
        }
        "month" => month_bounds(today.year(), today.month()),
        _ => Some((first_of(today.year(), 1)?, first_of(today.year() + 1, 1)?)),
    }
}

/// The days a phrase covers, as (first day, first day after). `None` when the
/// phrase names a day that does not exist.
fn span(when: When, today: NaiveDate) -> Option<(NaiveDate, NaiveDate)> {
    match when {
        When::Relative(n) => {
            let day = today + Duration::days(n);
            Some((day, day + Duration::days(1)))
        }
        When::Weekday { wd, last } => {
            // "last Monday" starts from yesterday; "on Monday" includes today.
            let back = i64::from(last);
            (back..back + 7)
                .map(|i| today - Duration::days(i))
                .find(|d| d.weekday() == wd)
                .map(|d| (d, d + Duration::days(1)))
        }
        When::Calendar { year, month: Some(m), day: Some(d) } => {
            let date = NaiveDate::from_ymd_opt(year.unwrap_or_else(|| day_year_for(m, d, today)), m, d)?;
            Some((date, date + Duration::days(1)))
        }
        When::Calendar { year, month: Some(m), day: None } => month_bounds(crate::plan::month_year(m, year, today), m),
        When::Calendar { year: Some(y), month: None, day: None } => Some((first_of(y, 1)?, first_of(y + 1, 1)?)),
        When::Calendar { .. } => None,
    }
}

/// Pulls calendar phrases out of `s` and records them on `f`. Runs after
/// "older than" and "newer than", and before the single-day and month fields.
/// Returns `None` when a phrase names a day that does not exist.
pub fn extract(s: &mut String, f: &mut Filter) -> Option<()> {
    let today = today();
    take(s, &RANGE, |c| {
        let (a, b) = (parse_when(&c[1])?, parse_when(&c[2])?);
        // "March to May 2026": the first date takes the year of the second.
        f.since = Some(span(a.or_year(b.year()), today)?.0);
        f.before = Some(span(b, today)?.1);
        Some(())
    })?;
    take(s, &ONWARD, |c| {
        f.since = Some(span(parse_when(&c[1])?, today)?.0);
        Some(())
    })?;
    take(s, &PERIOD_BOUND, |c| {
        let (start, end) = this_period(&c[2].to_lowercase(), today)?;
        match c[1].to_lowercase().as_str() {
            "before" | "prior to" => f.before = Some(start),
            "after" => f.since = Some(end),
            "since" => f.since = Some(start),
            _ => f.before = Some(end), // until, till, through
        }
        Some(())
    })?;
    take(s, &BOUND, |c| {
        let (start, end) = span(parse_when(&c[2])?, today)?;
        match c[1].to_lowercase().as_str() {
            "before" | "prior to" | "earlier than" => f.before = Some(start),
            "until" | "till" | "through" | "thru" | "up to" | "on or before" => f.before = Some(end),
            "after" | "later than" => f.since = Some(end),
            _ => f.since = Some(start), // since, on or after
        }
        Some(())
    })?;
    take(s, &ON, |c| {
        let (start, end) = span(parse_when(&c[1])?, today)?;
        f.since = Some(start);
        f.before = Some(end);
        Some(())
    })?;
    take(s, &LAST_WEEKDAY, |c| {
        let wd = weekday_of(&c[1].to_lowercase())?;
        let (start, end) = span(When::Weekday { wd, last: true }, today)?;
        f.since = Some(start);
        f.before = Some(end);
        Some(())
    })?;
    take(s, &THIS_PERIOD, |c| {
        f.since = Some(this_period(&c[1].to_lowercase(), today)?.0);
        Some(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    /// The span of a phrase, with today fixed at Wednesday 28 October 2026.
    fn span_of(phrase: &str) -> Option<(NaiveDate, NaiveDate)> {
        span(parse_when(phrase)?, on(2026, 10, 28))
    }

    #[test]
    fn a_day_spans_that_day() {
        assert_eq!(span_of("July 4"), Some((on(2026, 7, 4), on(2026, 7, 5))));
        assert_eq!(span_of("4th of July 2025"), Some((on(2025, 7, 4), on(2025, 7, 5))));
    }

    #[test]
    fn a_day_or_month_without_a_year_is_the_most_recent_one() {
        assert_eq!(span_of("December 25"), Some((on(2025, 12, 25), on(2025, 12, 26))));
        assert_eq!(span_of("August"), Some((on(2026, 8, 1), on(2026, 9, 1))));
        assert_eq!(span_of("December"), Some((on(2025, 12, 1), on(2026, 1, 1))));
    }

    #[test]
    fn a_year_spans_the_whole_year() {
        assert_eq!(span_of("2025"), Some((on(2025, 1, 1), on(2026, 1, 1))));
    }

    #[test]
    fn a_day_that_does_not_exist_has_no_span() {
        assert_eq!(span_of("February 29 2026"), None);
        assert_eq!(span_of("February 29"), None, "2026 is not a leap year");
        assert_eq!(span_of("February 29 2028"), Some((on(2028, 2, 29), on(2028, 3, 1))));
    }

    #[test]
    fn weekdays_count_back_from_today() {
        assert_eq!(span_of("last Tuesday"), Some((on(2026, 10, 27), on(2026, 10, 28))));
        assert_eq!(span_of("Wednesday"), Some((on(2026, 10, 28), on(2026, 10, 29))), "on Wednesday includes today");
        assert_eq!(span_of("last Wednesday"), Some((on(2026, 10, 21), on(2026, 10, 22))));
    }

    #[test]
    fn calendar_periods_start_on_the_right_day() {
        assert_eq!(this_period("week", on(2026, 10, 28)), Some((on(2026, 10, 26), on(2026, 11, 2))));
        assert_eq!(this_period("year", on(2026, 10, 28)), Some((on(2026, 1, 1), on(2027, 1, 1))));
    }

    #[test]
    fn a_range_takes_the_year_of_its_second_date() {
        let mut s = "photos from March to May 2026".to_string();
        let mut f = Filter::default();
        extract(&mut s, &mut f).unwrap();
        assert_eq!((f.since, f.before), (Some(on(2026, 3, 1)), Some(on(2026, 6, 1))));
    }

    #[test]
    fn a_phrase_naming_a_missing_day_refuses_the_whole_extraction() {
        let mut s = "photos from February 29 2026".to_string();
        assert_eq!(extract(&mut s, &mut Filter::default()), None);
    }
}
