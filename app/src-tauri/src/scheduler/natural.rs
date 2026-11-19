//! Quote-aware schedule normalization and calendar clauses. Never reinterpret a
//! failed time phrase as a destination name.
use super::*;

const WD: &str = "monday|tuesday|wednesday|thursday|friday|saturday|sunday";
const MONTH: &str =
    "january|february|march|april|may|june|july|august|september|october|november|december|jan|feb|mar|apr|jun|jul|aug|sep|oct|nov|dec";
const UNCLEAR: &str = "I couldn't understand the time in that command.";

pub(super) fn normalize(text: &str) -> Result<String, String> {
    let masked = crate::parse::phrase::unquoted_text(text).ok_or("Close the quotation marks first.")?;
    static ALIASES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\b(tmrw|evryday|evry|each)\b").unwrap());
    let mut result = text.to_string();
    for m in ALIASES.find_iter(&masked).collect::<Vec<_>>().into_iter().rev() {
        let replacement = match m.as_str().to_ascii_lowercase().as_str() {
            "tmrw" => "tomorrow",
            "evryday" => "everyday",
            _ => "every",
        };
        result.replace_range(m.range(), replacement);
    }
    let masked = crate::parse::phrase::unquoted_text(&result).ok_or("Close the quotation marks first.")?;
    // A schedule prefix ends at the first action verb, outside quoted names.
    static ACTION: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)\b(move|copy|create|make|rename|delete|trash|list|find|sort|organize|zip|unzip)\b").unwrap());
    static PREFIX: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)^(?:in\s+\d|after\s+|at\s+|tomorrow\b|today\b|tonight\b|this\s+(?:morning|evening|afternoon)\b|every\b|everyday\b|daily\b|weekly\b|monthly\b|on\s+|next\s+)").unwrap()
    });
    if PREFIX.is_match(masked.trim_start())
        && let Some(action) = ACTION.find(&masked)
    {
        let prefix = result[..action.start()].trim().trim_end_matches(',').trim();
        result = format!("{} {}", result[action.start()..].trim(), prefix);
    }
    let masked = crate::parse::phrase::unquoted_text(&result).ok_or("Close the quotation marks first.")?;
    static WEEKLY: LazyLock<Regex> = LazyLock::new(|| Regex::new(&format!(r"(?i)\bweekly\s+on\s+({WD})\b")).unwrap());
    for c in WEEKLY.captures_iter(&masked).collect::<Vec<_>>().into_iter().rev() {
        result.replace_range(c.get(0).unwrap().range(), &format!("every {}", &c[1]));
    }
    let masked = crate::parse::phrase::unquoted_text(&result).ok_or("Close the quotation marks first.")?;
    static PERIOD: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)\bevery\s+(morning|evening|afternoon)(?:\s+at\s+(\d{1,2})(?::(\d{2}))?\s*(am|pm)?)?\s*[.!]?$").unwrap()
    });
    if let Some(c) = PERIOD.captures(&masked) {
        let hour = c.get(2).ok_or("Include a time, such as every morning at 8 am.")?.as_str();
        let minutes = c.get(3).map(|m| format!(":{}", m.as_str())).unwrap_or_default();
        let period = c.get(4).map(|m| m.as_str()).unwrap_or(if c[1].eq_ignore_ascii_case("morning") { "am" } else { "pm" });
        result.replace_range(c.get(0).unwrap().range(), &format!("every day at {hour}{minutes}{period}"));
    }
    Ok(result)
}

pub(super) fn check_residual(text: &str) -> Result<(), String> {
    let masked = crate::parse::phrase::unquoted_text(text).ok_or("Close the quotation marks first.")?;
    static MONTHLY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\b(?:monthly|(?:every|each)\s+months?)\b").unwrap());
    if MONTHLY.is_match(&masked) {
        return Err("Monthly repeats are not supported. Use days or weeks.".into());
    }
    static REPEAT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\b(?:every|each|daily|weekly|monthly|hourly|everyday)\b").unwrap());
    static QUANTIFIER: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r"(?i)^(?:every|each)\s+(?:files?|pdfs?|images?|photos?|pictures?|screenshots?|documents?|videos?|archives?|folders?|items?)\b",
        )
        .unwrap()
    });
    for m in REPEAT.find_iter(&masked) {
        if !QUANTIFIER.is_match(&masked[m.start()..]) {
            return Err(UNCLEAR.into());
        }
    }
    // Only inspect destination/name clauses: source dates remain file filters.
    static DEST: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)(?:\b(?:to|into|called|named)\s+|\b(?:create|make|add)\s+(?:a\s+)?(?:new\s+)?folders?\s+)(.+)").unwrap()
    });
    static TIME: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(&format!(r"(?i)\b(?:{WD}|today|tomorrow|yesterday|tonight|morning|evening|afternoon|(?:next|last)\s+\w+|(?:in|after)\s+-?\d+\s+(?:min\w*|hour\w*|hr\w*)|\d{{4}}-\d{{1,2}}-\d{{1,2}}|on\s+\w*day|at\s+\d{{1,2}}(?:\s|:|[ap]|$))\b")).unwrap()
    });
    if DEST.captures(&masked).is_some_and(|c| TIME.is_match(&c[1])) {
        return Err(UNCLEAR.into());
    }
    Ok(())
}

type OneTime = Option<(String, Option<DateTime<Local>>)>;
pub(super) fn one_time(text: &str, now: DateTime<Local>) -> Result<OneTime, String> {
    let masked = crate::parse::phrase::unquoted_text(text).ok_or("Close the quotation marks first.")?;
    static COMPOUND: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)\s+(?:in|after)\s+(\d+)\s+(?:hours?|hrs?)\s+and\s+(\d+)\s+(?:minutes?|mins?)\s*[.!]?$").unwrap());
    if let Some(c) = COMPOUND.captures(&masked) {
        let hours: i64 = c[1].parse().map_err(|_| UNCLEAR)?;
        let mins: i64 = c[2].parse().map_err(|_| UNCLEAR)?;
        let total = hours
            .checked_mul(60)
            .and_then(|h| h.checked_add(mins))
            .filter(|n| *n > 0 && *n <= 525600)
            .ok_or("Choose a delay between one minute and one year.")?;
        return Ok(Some((text[..c.get(0).unwrap().start()].trim().into(), Some(now + Duration::minutes(total)))));
    }
    static CALENDAR: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(&format!(r"(?i)\s+(?P<date>today|tomorrow|yesterday|tonight|this\s+(?:morning|evening|afternoon)|(?:on|next)\s+(?:{WD})|on\s+(?:\d{{4}}-\d{{1,2}}-\d{{1,2}}|(?:{MONTH})\s+\d{{1,2}}(?:st|nd|rd|th)?(?:,?\s+\d{{4}})?))(?:\s+at\s+(?P<time>.+?))?\s*[.!]?$" )).unwrap()
    });
    let Some(c) = CALENDAR.captures(&masked) else { return Ok(None) };
    let whole = c.get(0).unwrap();
    let command = text[..whole.start()].trim();
    // Preserve source date filters, e.g. "delete files from today at 9pm".
    if command.split_whitespace().last().is_some_and(|s| ["from", "since", "before", "after", "of"].contains(&s)) {
        return Ok(None);
    }
    let date_phrase = c["date"].to_ascii_lowercase();
    // The legacy clock grammar also accepts "at 12 am tomorrow".
    if c.name("time").is_none() && matches!(date_phrase.as_str(), "today" | "tomorrow") {
        static TRAILING_CLOCK: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r"(?i)\bat\s+\d{1,2}(?::\d{2})?\s*(?:[ap]\.?m\.?)?$").unwrap());
        if TRAILING_CLOCK.is_match(command) {
            return Ok(None);
        }
    }
    let today = now.date_naive();
    let date = if date_phrase == "tomorrow" {
        today + Duration::days(1)
    } else if date_phrase == "yesterday" {
        today - Duration::days(1)
    } else if date_phrase.starts_with("next ") || date_phrase.starts_with("on ") {
        let value = date_phrase.split_once(' ').unwrap().1;
        if let Some(day) = weekday(value).filter(|_| value.chars().all(char::is_alphabetic)) {
            let ahead = (7 + i64::from(day.num_days_from_monday()) - i64::from(now.weekday().num_days_from_monday())) % 7;
            today + Duration::days(if ahead == 0 && date_phrase.starts_with("next ") { 7 } else { ahead })
        } else {
            parse_date(value, today.year()).ok_or("That date isn't valid.")?
        }
    } else {
        today
    };
    if date < today {
        return Err("That time has already passed.".into());
    }
    let value = c.name("time").ok_or("Include a time, such as on Friday at 9 am.")?.as_str();
    static BARE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\d{1,2}(?::\d{2})?$").unwrap());
    let implied = if date_phrase.contains("morning") {
        Some("am")
    } else if date_phrase.contains("evening") || date_phrase.contains("afternoon") || date_phrase == "tonight" {
        Some("pm")
    } else {
        None
    };
    let clock = if BARE.is_match(value)
        && let Some(period) = implied
    {
        format!("{value}{period}")
    } else {
        value.into()
    };
    let time = clock_time(&clock)?;
    let due = Local
        .from_local_datetime(&date.and_time(time))
        .single()
        .ok_or("That time is skipped or repeated by a daylight-saving change. Choose another time.")?;
    if due <= now {
        return Err("That time has already passed.".into());
    }
    if command.is_empty() {
        return Err("Say what to do before the time.".into());
    }
    Ok(Some((command.into(), Some(due))))
}

fn parse_date(value: &str, year: i32) -> Option<NaiveDate> {
    if let Ok(date) = NaiveDate::parse_from_str(value, "%Y-%m-%d") {
        return Some(date);
    }
    static DATE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(\w+)\s+(\d{1,2})(?:st|nd|rd|th)?(?:,?\s+(\d{4}))?$").unwrap());
    let c = DATE.captures(value)?;
    let month = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"]
        .iter()
        .position(|m| c[1].starts_with(m))? as u32
        + 1;
    NaiveDate::from_ymd_opt(c.get(3).map_or(Some(year), |m| m.as_str().parse().ok())?, month, c[2].parse().ok()?)
}
