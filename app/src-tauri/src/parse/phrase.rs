//! Text cleanup and the shared "subject phrase" grammar: the part of a command
//! that names what to act on, e.g. "PDFs larger than 5 MB from Downloads".

use super::dates;
use crate::model::{DayRef, FileType, Filter, Source};
use regex::Regex;
use std::sync::LazyLock;

macro_rules! re {
    ($s:expr) => {
        LazyLock::new(|| Regex::new($s).unwrap())
    };
}

static LEAD_FILLER: LazyLock<Regex> = re!(
    r"(?i)^(?:(?:hey|ok|okay|so|well|alright|um|uh|erm|ah)[,]?\s+)*(?:voxi[,]?\s+)?(?:help\s+me\s+)?(?:(?:can|could|would|will)\s+you\s+|please[,]?\s+|kindly\s+|go\s+ahead\s+and\s+|i\s+want\s+(?:you\s+)?to\s+|i'?d\s+like\s+(?:you\s+)?to\s+|i\s+need\s+(?:you\s+)?to\s+|let's\s+|(?:um|uh|erm|ah)[,]?\s+)*"
);
static TRAIL_FILLER: LazyLock<Regex> = re!(r"(?i)(?:,?\s+(?:please|for\s+me|now|thanks|thank\s+you|period|full\s+stop))+$");
/// "move X in Downloads over to Documents": "over" and "across" before a destination.
static OVER_TO: LazyLock<Regex> = re!(r"(?i)\s+(?:over|across)\s+(to|into)\b");
static TRAIL_PUNCT: LazyLock<Regex> = re!(r"[.!?;:,\s]+$");
static SPOKEN_DOT: LazyLock<Regex> = re!(
    r"(?i)(\w)\s+dot\s+(pdf|png|jpe?g|heic|webp|gif|txt|md|zip|docx?|xlsx?|pptx?|csv|mov|mp4|dmg|pkg|key|pages|numbers)\b"
);
static FILLER_WORDS: LazyLock<Regex> = re!(r"(?i)\b(?:all\s+of\s+|all\s+|the\s+|my\s+|every\s+|whole\s+|please\s+)");
static COMMA_BEFORE_KEYWORD: LazyLock<Regex> = re!(r"(?i),\s+(to|into|in|from|as|on)\s+");
static SPACES: LazyLock<Regex> = re!(r"\s+");

/// Cleans dictated or typed text: drops politeness and filler words, trailing
/// punctuation that Wispr Flow adds, and turns spoken numbers into digits.
pub fn normalize(text: &str) -> String {
    // Dictation often produces curly quotes; they mean the same as straight ones.
    let straight = text.replace(['“', '”'], "\"").replace(['‘', '’'], "'");
    let mut s = SPACES.replace_all(straight.trim(), " ").to_string();
    s = OVER_TO.replace_all(&s, " $1").to_string();
    s = SPOKEN_DOT.replace_all(&s, "$1.$2").to_string();
    s = TRAIL_PUNCT.replace(&s, "").to_string();
    s = LEAD_FILLER.replace(&s, "").to_string();
    s = TRAIL_FILLER.replace(&s, "").to_string();
    s = COMMA_BEFORE_KEYWORD.replace_all(&s, " $1 ").to_string();
    // Filler words, but not inside quoted names.
    s = map_unquoted(&s, |part| FILLER_WORDS.replace_all(part, "").to_string());
    s = s
        .split(' ')
        .map(|tok| word_number(tok).map(|n| n.to_string()).unwrap_or_else(|| tok.to_string()))
        .collect::<Vec<_>>()
        .join(" ");
    s = SPACES.replace_all(s.trim(), " ").to_string();
    TRAIL_PUNCT.replace(&s, "").to_string()
}

fn map_unquoted(s: &str, f: impl Fn(&str) -> String) -> String {
    s.split('"')
        .enumerate()
        .map(|(i, part)| if i % 2 == 0 { f(part) } else { part.to_string() })
        .collect::<Vec<_>>()
        .join("\"")
}

fn word_number(tok: &str) -> Option<u32> {
    let n = match tok.to_ascii_lowercase().as_str() {
        "one" => 1, "two" => 2, "three" => 3, "four" => 4, "five" => 5,
        "six" => 6, "seven" => 7, "eight" => 8, "nine" => 9, "ten" => 10,
        "eleven" => 11, "twelve" => 12, "fifteen" => 15, "twenty" => 20,
        "thirty" => 30, "forty" => 40, "fifty" => 50, "sixty" => 60,
        "ninety" => 90, "hundred" => 100,
        _ => return None,
    };
    Some(n)
}

/// Strips matching quotes and a trailing " folder" from a name.
pub fn clean_name(s: &str) -> String {
    let s = s.trim().trim_matches(['"', '\'']).trim();
    let s = s.strip_suffix(" folder").or_else(|| s.strip_suffix(" Folder")).unwrap_or(s);
    let s = s.strip_prefix("folder ").or_else(|| s.strip_prefix("Folder ")).unwrap_or(s);
    s.trim().to_string()
}

/// Drops the spoken words around a name: "the file claude.dmg",
/// "claude.dmg file" -> "claude.dmg".
fn clean_item(s: &str) -> String {
    let mut s = clean_name(s);
    for word in ["file", "folder"] {
        let lower = s.to_lowercase();
        if lower.ends_with(&format!(" {word}")) {
            s.truncate(s.len() - word.len() - 1);
        } else if lower.starts_with(&format!("{word} ")) {
            s = s[word.len() + 1..].to_string();
        }
    }
    // "the file 'Lease.pdf'": the quotes are still there once the word is gone.
    clean_name(&s)
}

pub fn has_extension(name: &str) -> bool {
    let base = name.rsplit('/').next().unwrap_or(name);
    match base.rsplit_once('.') {
        Some((stem, ext)) => !stem.is_empty() && (1..=8).contains(&ext.len()) && ext.chars().all(|c| c.is_ascii_alphanumeric()),
        None => false,
    }
}

/// A size number: "500", "1.5", "a", "an", or "one" (for 1, as in "a gig").
const SIZE_NUMBER: &str = r"(\d+(?:\.\d+)?|an?|one)";
/// A size unit, with its spoken forms: gigs, megs, kilobytes.
const SIZE_UNIT: &str = r"(kbs?|kilobytes?|k|mbs?|megs?|megabytes?|m|gbs?|gigs?|gigabytes?|g|tbs?|terabytes?|t)";
static SIZE_LARGER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(r"(?i)\b(?:(?:larger|bigger|greater|more|heavier)\s+than|over|above|at\s+least)\s+{SIZE_NUMBER}\s*{SIZE_UNIT}?\b"))
        .unwrap()
});
static SIZE_SMALLER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(r"(?i)\b(?:(?:smaller|less|lighter)\s+than|under|below|at\s+most|up\s+to)\s+{SIZE_NUMBER}\s*{SIZE_UNIT}?\b"))
        .unwrap()
});
static BETWEEN_SIZE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(r"(?i)\bbetween\s+{SIZE_NUMBER}\s*{SIZE_UNIT}?\s+and\s+{SIZE_NUMBER}\s*{SIZE_UNIT}?\b")).unwrap()
});
/// "larger than 500 MB but smaller than 1 GB": "but" joins two size limits.
static BUT_SIZE: LazyLock<Regex> = re!(r"(?i)\bbut\s+((?:smaller|larger|bigger|less|more|under|over|at)\b)");
/// "1 GB or more", "500 MB or less": the size and the "or" phrase both go.
static SIZE_OR_MORE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(r"(?i)\b{SIZE_NUMBER}\s*{SIZE_UNIT}?\s+(?:or\s+more|or\s+larger|or\s+bigger|and\s+(?:up|more))\b")).unwrap()
});
static SIZE_OR_LESS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(r"(?i)\b{SIZE_NUMBER}\s*{SIZE_UNIT}?\s+(?:or\s+less|or\s+smaller|and\s+(?:under|less|below))\b")).unwrap()
});
/// "with spaces in their names" filters on the words, not on names: refused.
static NAME_VALUE_REFUSED: LazyLock<Regex> = re!(
    r"(?i)\b(?:with|containing|that\s+(?:has|have|contains?)|which\s+(?:has|have))\s+(?:spaces|numbers|digits|letters|capitals|capital\s+letters|dashes|underscores|symbols)\b"
);
/// Time words the parser cannot turn into dates: refused, not read as folder names.
static TIME_WORDS: LazyLock<Regex> = re!(
    r"(?i)\b(?:weekends?|mornings?|afternoons?|evenings?|tonight|last\s+night|last\s+(?:hour|quarter)|week\s+before|month\s+before|before\s+last|ago)\b"
);
/// "Desktop screenshots": a folder word, then a type.
static FOLDER_PREFIX: LazyLock<Regex> = re!(r"(?i)^(desktop|downloads|documents|pictures|screenshots)\s+(.+)$");
/// "and in Documents" once a filter before it is removed: the connector goes.
static CONNECTOR_BEFORE_PREP: LazyLock<Regex> = re!(r"(?i)\s+(?:and|or)\s+((?:in|from|on|inside|within)\b)");
static OLDER: LazyLock<Regex> = re!(
    r"(?i)\b(?:older\s+than|more\s+than|over|before|not\s+(?:opened|modified|touched|used)\s+in)\s+(\d+|a|an)\s*(days?|weeks?|months?|years?)(?:\s+old|\s+ago)?\b"
);
static NEWER: LazyLock<Regex> = re!(
    r"(?i)\b(?:(?:from|in|within|during|over)\s+)?(?:last|past)\s+(?:(\d+)\s*)?(days?|weeks?|months?|years?)\b"
);
/// "since 3 days ago": the same as "in the last 3 days".
static SINCE_AGO: LazyLock<Regex> = re!(r"(?i)\bsince\s+(\d+|an?)\s*(days?|weeks?|months?|years?)\s+ago\b");
static DAY: LazyLock<Regex> = re!(
    r"(?i)\b(?:(?:from|taken|created|made|modified|added|downloaded)\s+)?(today|yesterday)(?:'s)?\b"
);
static MONTH: LazyLock<Regex> = re!(
    r"(?i)\b(?:(?:taken|created|made|modified|added|downloaded|saved|captured|recorded)\s+)?(?:(?:from|in|during|of|on)\s+(?:month\s+of\s+)?(january|february|march|april|may|june|july|august|september|october|november|december|jan|feb|mar|apr|jun|jul|aug|sept|sep|oct|nov|dec)\b(?:\s*,?\s*(?:of\s+)?((?:19|20)\d{2}))?|(january|february|march|april|may|june|july|august|september|october|november|december|jan|feb|mar|apr|jun|jul|aug|sept|sep|oct|nov|dec)\b\s*,?\s*(?:of\s+)?((?:19|20)\d{2}))"
);
/// "taken", "downloaded", "that were saved": how files came to be says
/// nothing about which ones, once the dates are read.
static PARTICIPLE: LazyLock<Regex> = re!(
    r"(?i)\b(?:(?:that|which)\s+were\s+|were\s+)?(?:taken|created|made|downloaded|saved|captured|added|modified|recorded|grabbed|snapped|received)\b"
);
/// "named", "containing", "that start with": the keyword before a name.
static NAME_KEYWORD: LazyLock<Regex> = re!(
    r"(?i)\b(?:named\s+like|called\s+like|named|called|containing|that\s+contains?|whose\s+names?\s+contains?|matching|with|that\s+starts?\s+with|starting\s+with|beginning\s+with|that\s+begins?\s+with|(?:whose\s+)?names?\s+(?:start|starts|begin|begins)\s+with|that\s+(?:has|have)|which\s+(?:has|have))\s+"
);
/// After a name: "in the name", which is filler.
static NAME_SUFFIX: LazyLock<Regex> = re!(r"(?i)^\s+in\s+(?:the\s+|their\s+|its\s+)?names?\b");
static WORD: LazyLock<Regex> = re!(r"\S+");
/// Words that end an unquoted name: "files named budget from Downloads".
const NAME_STOP: &[&str] = &[
    "from", "in", "on", "inside", "within", "to", "into", "older", "newer", "larger", "bigger", "smaller", "less", "more",
    "over", "under", "taken", "created", "made", "before", "after", "since", "until", "between", "last", "past", "today",
    "yesterday", "that", "which",
];
static FOLDER: LazyLock<Regex> = re!(r"(?i)^(.*?)\s*\b(?:from|in|on|inside|within)\s+(.+)$");
/// "screenshots I took", "PDFs that I've downloaded", "files we saved":
/// who made the files says nothing about which ones, so it's dropped.
static WHO_CLAUSE: LazyLock<Regex> = re!(
    r"(?i)\b(?:that\s+|which\s+)?(?:i|i've|i\s+have|i'd|we|we've)\s+(?:have\s+|just\s+|recently\s+)?(?:took|taken|take|made|make|downloaded|download|saved|save|created|got|gotten|received|captured|grabbed|added|recorded|exported|scanned|snapped)\b"
);

fn month_number(name: &str) -> Option<u32> {
    const MONTHS: [&str; 12] = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];
    let n = name.to_ascii_lowercase();
    MONTHS.iter().position(|m| n.starts_with(m)).map(|i| i as u32 + 1)
}

fn to_mb(n: f64, unit: Option<&str>) -> f64 {
    match unit.map(|u| u.to_ascii_lowercase()) {
        Some(u) if u.starts_with('k') => n / 1024.0,
        Some(u) if u.starts_with('g') => n * 1024.0,
        Some(u) if u.starts_with('t') => n * 1024.0 * 1024.0,
        _ => n,
    }
}

/// Days in a count of days, weeks, months, or years. `None` when the count is
/// too large to hold, so "older than 99999999999 years" is refused, not read as
/// one year.
fn to_days(n: &str, unit: &str) -> Option<u32> {
    let n: u32 = match n.to_ascii_lowercase().as_str() {
        "a" | "an" | "" => 1,
        other => other.parse().ok()?,
    };
    let per = match unit.to_ascii_lowercase().trim_end_matches('s') {
        "week" => 7,
        "month" => 30,
        "year" => 365,
        _ => 1,
    };
    n.checked_mul(per)
}

/// Removes every match of `re` from `s`, handing each match's captures to `f`.
/// When `f` returns `None` the match is refused: `s` is left as it was, and the
/// caller refuses the whole command rather than drop the phrase unread.
pub(super) fn take(s: &mut String, re: &Regex, mut f: impl FnMut(&regex::Captures) -> Option<()>) -> Option<()> {
    let mut out = String::new();
    let mut last = 0;
    for caps in re.captures_iter(s) {
        let m = caps.get(0)?;
        f(&caps)?;
        out.push_str(&s[last..m.start()]);
        out.push(' ');
        last = m.end();
    }
    out.push_str(&s[last..]);
    *s = SPACES.replace_all(out.trim(), " ").to_string();
    Some(())
}

/// Reads a size number, where "a", "an", and "one" mean 1.
fn size_number(s: &str) -> f64 {
    match s.to_ascii_lowercase().as_str() {
        "a" | "an" | "one" => 1.0,
        n => n.parse().unwrap_or(0.0),
    }
}

/// Takes the name after "named", "containing", "that start with" and the like:
/// one quoted phrase, or the words up to the next stop word. Records it on `f`
/// and removes it from `s`. `None` when the keyword has no name after it, or
/// when a bare "with" names nothing.
fn take_name(s: &mut String, f: &mut Filter) -> Option<()> {
    let Some(m) = NAME_KEYWORD.find(s) else { return Some(()) };
    let (start, kw_end) = (m.start(), m.end());
    let keyword = m.as_str().trim().to_lowercase();
    let (value, used, in_name) = name_value(&s[kw_end..]);
    if value.is_empty() {
        return None;
    }
    if keyword == "with" && !in_name {
        // "Mom with Dad.pdf" is one file name; "with budget" alone names nothing.
        return has_extension(&value).then_some(());
    }
    if keyword.contains("start") || keyword.contains("begin") {
        f.name_starts_with = Some(value);
    } else {
        f.name_contains = Some(value);
    }
    s.replace_range(start..kw_end + used, " ");
    Some(())
}

/// The name at the start of `rest`: its text, how many bytes it covers
/// (including a trailing "in the name"), and whether "in the name" followed it.
fn name_value(rest: &str) -> (String, usize, bool) {
    let lead = rest.len() - rest.trim_start().len();
    let t = &rest[lead..];
    if let Some(q) = t.chars().next().filter(|c| *c == '"' || *c == '\'')
        && let Some(close) = t[1..].find(q)
    {
        let end = lead + close + 2;
        let suffix = name_suffix_len(&rest[end..]);
        return (t[1..close + 1].to_string(), end + suffix, suffix > 0);
    }
    let mut words: Vec<(&str, usize)> = Vec::new();
    for m in WORD.find_iter(rest) {
        if NAME_STOP.contains(&m.as_str().to_lowercase().as_str()) {
            break;
        }
        words.push((m.as_str(), m.end()));
    }
    // "and" and "or" end a name unless it is one file: "Terms and Conditions.pdf"
    // has an extension after the "and", while "notes.txt and todo.txt" already
    // ended one file before it, and "budget and invoice" names nothing.
    let mut cut = words.len();
    for (i, (word, _)) in words.iter().enumerate() {
        if matches!(word.to_ascii_lowercase().as_str(), "and" | "or") {
            let named_before = words[..i].iter().any(|(w, _)| has_extension(w));
            let named_after = words[i + 1..].iter().any(|(w, _)| has_extension(w));
            if named_before || !named_after {
                cut = i;
                break;
            }
        }
    }
    words.truncate(cut);
    let end = words.last().map_or(0, |(_, e)| *e);
    let value = words.iter().map(|(w, _)| *w).collect::<Vec<_>>().join(" ");
    let suffix = name_suffix_len(&rest[end..]);
    (value, end + suffix, suffix > 0)
}

fn name_suffix_len(rest: &str) -> usize {
    NAME_SUFFIX.find(rest).map_or(0, |m| m.end())
}

/// Pulls size, age, date, and name filters out of a phrase. Returns the filter
/// and what is left of the phrase, or `None` when the phrase names something
/// Voxi cannot read: a day that does not exist, an age too large to hold, or a
/// second "today" or "yesterday".
fn extract_filters(phrase: &str) -> Option<(Filter, String)> {
    let mut f = Filter::default();
    let mut s = phrase.trim().to_string();
    take(&mut s, &WHO_CLAUSE, |_| Some(()))?;
    take_name(&mut s, &mut f)?;
    // Ages before sizes: "over 30 days" must not read as a size.
    take(&mut s, &OLDER, |c| {
        f.older_than_days = Some(to_days(&c[1], &c[2])?);
        Some(())
    })?;
    take(&mut s, &SINCE_AGO, |c| {
        f.newer_than_days = Some(to_days(&c[1], &c[2])?);
        Some(())
    })?;
    take(&mut s, &NEWER, |c| {
        f.newer_than_days = Some(to_days(c.get(1).map_or("", |m| m.as_str()), &c[2])?);
        Some(())
    })?;
    dates::extract(&mut s, &mut f)?;
    take(&mut s, &DAY, |c| {
        // "from yesterday or today" names two days. Refuse it, rather than keep the last.
        if f.day.is_some() {
            return None;
        }
        f.day = Some(if c[1].eq_ignore_ascii_case("today") { DayRef::Today } else { DayRef::Yesterday });
        Some(())
    })?;
    take(&mut s, &MONTH, |c| {
        f.month = month_number(c.get(1).or(c.get(3)).map_or("", |m| m.as_str()));
        f.year = c.get(2).or(c.get(4)).and_then(|y| y.as_str().parse().ok());
        Some(())
    })?;
    take(&mut s, &PARTICIPLE, |_| Some(()))?;
    s = BUT_SIZE.replace_all(&s, "$1").to_string();
    take(&mut s, &SIZE_OR_MORE, |c| {
        f.larger_than_mb = Some(to_mb(size_number(&c[1]), c.get(2).map(|m| m.as_str())));
        Some(())
    })?;
    take(&mut s, &SIZE_OR_LESS, |c| {
        f.smaller_than_mb = Some(to_mb(size_number(&c[1]), c.get(2).map(|m| m.as_str())));
        Some(())
    })?;
    take(&mut s, &BETWEEN_SIZE, |c| {
        f.larger_than_mb = Some(to_mb(size_number(&c[1]), c.get(2).or(c.get(4)).map(|m| m.as_str())));
        f.smaller_than_mb = Some(to_mb(size_number(&c[3]), c.get(4).or(c.get(2)).map(|m| m.as_str())));
        Some(())
    })?;
    take(&mut s, &SIZE_LARGER, |c| {
        f.larger_than_mb = Some(to_mb(size_number(&c[1]), c.get(2).map(|m| m.as_str())));
        Some(())
    })?;
    take(&mut s, &SIZE_SMALLER, |c| {
        f.smaller_than_mb = Some(to_mb(size_number(&c[1]), c.get(2).map(|m| m.as_str())));
        Some(())
    })?;
    Some((f, s))
}

const EXT_WORDS: &[&str] = &[
    "heic", "heif", "png", "jpg", "jpeg", "gif", "webp", "tiff", "svg", "mov", "mp4", "csv", "txt",
    "docx", "doc", "xlsx", "pptx", "key", "pages", "numbers", "md", "rtf", "mp3", "wav", "dmg", "pkg", "mkv",
    "avi", "m4v", "webm", "psd", "flac", "m4a",
];

/// Maps the head of a subject phrase ("screenshots", "HEIC images", "files")
/// to a type and extension filter. None means it isn't a type word.
pub fn type_word(head: &str) -> Option<(FileType, Option<Vec<String>>)> {
    let h = head.trim().trim_start_matches('.').to_ascii_lowercase();
    let t = match h.as_str() {
        "" | "files" | "file" | "everything" | "anything" | "items" | "stuff" | "things" => FileType::Files,
        "screenshots" | "screenshot" | "screen shots" => FileType::Screenshots,
        "images" | "image" | "image files" | "pictures" | "picture" | "photos" | "photo" | "pics" | "pic" => FileType::Images,
        "pdfs" | "pdf" | "pdf files" | "pdf documents" => FileType::Pdfs,
        "videos" | "video" | "video files" | "movies" | "movie" | "screen recordings" => FileType::Videos,
        "documents" | "document" | "docs" | "document files" => FileType::Documents,
        "archives" | "archive" | "zips" | "zip" | "zip files" | "compressed files" => FileType::Archives,
        "installers" | "installer" | "installer files" | "dmgs" | "dmg files" | "disk images" => FileType::Installers,
        "audio" | "audio files" | "music" | "songs" | "song" => FileType::Audio,
        _ => {
            let word = h
                .trim_end_matches(" files")
                .trim_end_matches(" images")
                .trim_end_matches(" photos")
                .trim_end_matches(" pictures")
                .trim_end_matches(" documents")
                .trim_end_matches(" docs")
                .trim_end_matches(" videos")
                .trim_end_matches(" movies")
                .trim_end_matches(" clips")
                .trim_end_matches(" file");
            let word = word.strip_prefix('.').unwrap_or(word);
            if let Some((_, exts)) = WORD_TYPES.iter().find(|(w, _)| word == *w || word.strip_suffix('s') == Some(*w)) {
                return Some((FileType::Files, Some(exts.iter().map(|e| e.to_string()).collect())));
            }
            let single = EXT_WORDS
                .iter()
                .find(|e| word == **e || word.strip_suffix('s') == Some(**e) || word.strip_suffix("'s") == Some(**e))?;
            let exts = match *single {
                "jpg" | "jpeg" => vec!["jpg".into(), "jpeg".into()],
                "heic" | "heif" => vec!["heic".into(), "heif".into()],
                other => vec![other.to_string()],
            };
            return Some((FileType::Files, Some(exts)));
        }
    };
    Some((t, None))
}

/// Words for whole families of extensions: "spreadsheets", "text files".
const WORD_TYPES: &[(&str, &[&str])] = &[
    ("word", &["doc", "docx"]),
    ("spreadsheet", &["xlsx", "xls", "csv", "numbers"]),
    ("presentation", &["pptx", "ppt", "key"]),
    ("text", &["txt"]),
    ("raw", &["raw", "cr2", "nef", "arw", "dng"]),
    ("excel", &["xlsx", "xls"]),
    ("powerpoint", &["pptx", "ppt"]),
    ("photoshop", &["psd"]),
];

pub const KNOWN_FOLDERS: &[&str] = &[
    "desktop", "downloads", "download", "documents", "document", "pictures", "picture", "screenshots", "screenshot",
];

/// Parses a subject phrase into a single named item or a filtered set.
pub fn parse_source(phrase: &str) -> Option<Source> {
    let phrase = phrase.trim();
    if phrase.is_empty() {
        return None;
    }
    if phrase.starts_with('"') && phrase.ends_with('"') && phrase.len() > 1 {
        return Some(Source::item(&clean_name(phrase)));
    }
    if NAME_VALUE_REFUSED.is_match(phrase) {
        return None;
    }
    let (mut filter, rest) = extract_filters(phrase)?;
    if filter.name_contains.as_deref().is_some_and(|n| n.contains(['*', '?'])) {
        return None;
    }
    let rest = tidy_subject(&rest, filter.is_narrowing());
    if excludes(&rest) || (!has_extension(&rest) && TIME_WORDS.is_match(&rest)) {
        return None;
    }
    let (head, folder) = match FOLDER.captures(&rest) {
        Some(c) => (
            tidy_subject(&c[1], filter.is_narrowing()),
            Some(clean_name(&tidy_subject(&c[2], filter.is_narrowing()))),
        ),
        None => (rest.trim().to_string(), None),
    };
    // "the file called passport.pdf" is one file. A type word that contradicts
    // its extension ("photos named report.pdf") or another filter is refused.
    if let Some(name) = filter.name_contains.clone().filter(|n| has_extension(n)) {
        let other = Filter { name_contains: None, ..filter.clone() };
        return (type_fits(&head, &name) && !other.is_narrowing()).then_some(Source::Item { name, folder });
    }
    // "Pictures folder" names the folder, the same as "Pictures".
    if let Some(stem) = head.strip_suffix(" folder").or_else(|| head.strip_suffix(" Folder"))
        && folder.is_none()
        && KNOWN_FOLDERS.contains(&stem.trim().to_ascii_lowercase().as_str())
    {
        filter.file_type.get_or_insert(FileType::Files);
        return Some(Source::Set { folder: Some(stem.trim().to_string()), filter });
    }
    // "Desktop screenshots": a folder word, then a type.
    if let Some(c) = FOLDER_PREFIX.captures(&head)
        && folder.is_none()
        && let Some((t, ext)) = type_word(&c[2])
    {
        filter.file_type = Some(t);
        filter.ext = ext;
        return Some(Source::Set { folder: Some(c[1].to_string()), filter });
    }
    let narrowed = filter.is_narrowing();

    if !has_extension(&head) && !head.contains('/') {
        if let Some((t, ext)) = type_word(&head) {
            // "Documents" and "Pictures" are folders as well as types. With no
            // other folder given, a capitalized one names the folder.
            let is_folder_name = folder.is_none()
                && KNOWN_FOLDERS.contains(&head.to_ascii_lowercase().as_str())
                && head.starts_with(|c: char| c.is_uppercase());
            if !is_folder_name {
                filter.file_type = Some(t);
                filter.ext = ext;
                if head.is_empty() && folder.is_none() && !narrowed {
                    return None;
                }
                return Some(Source::Set { folder, filter });
            }
        }
        let known = KNOWN_FOLDERS.contains(&head.to_ascii_lowercase().as_str());
        if folder.is_none() && (narrowed || known) {
            // An unknown phrase with filters names a folder only when it is one word.
            if !known && head.contains(' ') {
                return None;
            }
            filter.file_type = Some(FileType::Files);
            return Some(Source::Set { folder: Some(clean_name(&head)), filter });
        }
    }
    // Filters with no known type or folder have nothing to apply to. Refuse
    // rather than drop them, so "mp4 videos older than a year" never becomes a
    // plain item named "mp4 videos".
    if narrowed {
        return None;
    }
    let name = clean_item(&head);
    if name.is_empty() || vague(&name) {
        return None;
    }
    Some(Source::Item { name, folder })
}

/// The subject of a command that needs a set of files, such as "how many PDFs"
/// or "what files did I download". A single named item is refused there.
pub fn set_source(phrase: &str) -> Option<Source> {
    match parse_source(phrase)? {
        source @ Source::Set { .. } => Some(source),
        Source::Item { .. } => None,
    }
}

/// Subjects that point at "it", "them", or "the newest one", or at a size
/// nobody can check ("big files"). Voxi refuses these rather than guess.
/// Names with an extension and paths are left alone.
fn vague(name: &str) -> bool {
    if has_extension(name) || name.contains('/') {
        return false;
    }
    const VAGUE: &[&str] = &[
        "it", "them", "they", "those", "these", "that", "this", "one", "ones", "stuff", "things", "something",
        "anything", "newest", "latest", "oldest", "biggest", "largest", "smallest", "recent", "recently", "duplicates",
        "duplicate",
    ];
    const SIZES: &[&str] = &["big", "bigger", "biggest", "large", "huge", "small", "tiny", "old", "older", "new", "recent"];
    const NOUNS: &[&str] = &[
        "files", "file", "photos", "pictures", "screenshots", "documents", "docs", "pdfs", "images", "videos", "downloads",
    ];
    let words: Vec<String> = name.to_lowercase().split_whitespace().map(str::to_string).collect();
    // A name made only of size words, or of "1" (which is "one" once normalized,
    // as in "old one"), says no particular file.
    let only_sizes = !words.is_empty() && words.iter().all(|w| SIZES.contains(&w.as_str()) || w.as_str() == "1");
    words.iter().any(|w| VAGUE.contains(&w.as_str()))
        || only_sizes
        || (words.iter().any(|w| SIZES.contains(&w.as_str())) && words.last().is_some_and(|w| NOUNS.contains(&w.as_str())))
}

/// Words a subject is left with once its dates, sizes, and names are cut out,
/// and that name nothing: "PDFs from" is "PDFs", "files that are" is "files",
/// and "did" in "files did yesterday" goes too. Only when there are filters, so
/// a plain name such as "Plan A" is left alone.
fn tidy_subject(s: &str, narrowed: bool) -> String {
    if !narrowed || s.split_whitespace().any(has_extension) {
        return s.trim().to_string();
    }
    // "PDFs and in Documents": a connector left between two removed filters.
    let s = CONNECTOR_BEFORE_PREP.replace_all(s.trim(), " $1");
    const AUX: &[&str] = &["did", "does", "do", "are", "were", "is"];
    const DANGLING: &[&str] = &[
        "from", "of", "on", "in", "to", "into", "onward", "onwards", "and", "or", "that", "which", "that's", "before",
        "after", "since", "until", "by", "with", "the", "a", "an", "up",
    ];
    const DANGLING_PAIRS: &[&str] = &[
        "that are", "that is", "that were", "which are", "which is", "that has", "that have", "start of",
        "beginning of", "end of", "and later", "or later", "and up",
    ];
    let mut words: Vec<&str> = s.split_whitespace().filter(|w| !AUX.contains(&w.to_ascii_lowercase().as_str())).collect();
    loop {
        let n = words.len();
        let pair = (n >= 2).then(|| format!("{} {}", words[n - 2], words[n - 1]).to_ascii_lowercase());
        if pair.is_some_and(|p| DANGLING_PAIRS.contains(&p.as_str())) {
            words.truncate(n - 2);
        } else if n >= 1 && DANGLING.contains(&words[n - 1].to_ascii_lowercase().as_str()) {
            words.truncate(n - 1);
        } else {
            break;
        }
    }
    words.join(" ")
}

/// Whether a file name's extension fits the type word beside it: "the video
/// called intro.mov" fits, "photos named report.pdf" does not. A word that names
/// no type fits any name.
fn type_fits(head: &str, name: &str) -> bool {
    let Some((t, exts)) = type_word(head) else { return true };
    let ext = name.rsplit_once('.').map_or("", |(_, e)| e).to_ascii_lowercase();
    match exts {
        Some(list) => list.contains(&ext),
        None => t.accepts_ext(&ext),
    }
}

/// A destination that is more than a folder name: "Documents and Pictures",
/// "folders by year", "a new folder called Shots", "Finance folder inside
/// Documents". Refused, not created as a folder with that name.
pub fn bad_destination(to: &str) -> bool {
    let lower = to.to_lowercase();
    excludes(to) || [" by ", " called ", " inside ", " within "].iter().any(|word| lower.contains(word))
}

/// Words that join two things or mean "all but": "PDFs and images",
/// "everything except PDFs". Voxi does one thing per command.
fn excludes(s: &str) -> bool {
    let lower = s.to_lowercase();
    lower.split_whitespace().any(|w| matches!(w, "and" | "or" | "but" | "except" | "not" | "without" | "then"))
        || lower.contains("other than")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_strips_dictation_noise() {
        assert_eq!(normalize("Please move the report.pdf to Documents."), "move report.pdf to Documents");
        assert_eq!(normalize("Hey Voxi, can you move all PDFs from Downloads to Documents?"), "move PDFs from Downloads to Documents");
        assert_eq!(normalize("convert photo dot heic to png"), "convert photo.heic to png");
        assert_eq!(normalize("list files from the last seven days"), "list files from last 7 days");
    }

    #[test]
    fn subject_with_filters_and_folder() {
        let s = parse_source("files larger than 100 MB from Downloads").unwrap();
        assert_eq!(
            s,
            Source::set(Some("Downloads"), Filter { file_type: Some(FileType::Files), larger_than_mb: Some(100.0), ..Default::default() })
        );
    }

    #[test]
    fn bare_folder_with_filter_is_a_folder() {
        let s = parse_source("Downloads older than 90 days").unwrap();
        assert_eq!(
            s,
            Source::set(Some("Downloads"), Filter { file_type: Some(FileType::Files), older_than_days: Some(90), ..Default::default() })
        );
    }

    #[test]
    fn single_file_with_extension() {
        assert_eq!(parse_source("report.pdf").unwrap(), Source::item("report.pdf"));
    }

    #[test]
    fn filters_leave_no_stray_words_in_the_subject() {
        assert_eq!(tidy_subject("PDFs from", true), "PDFs");
        assert_eq!(tidy_subject("files that are", true), "files");
        assert_eq!(tidy_subject("files did", true), "files");
        assert_eq!(tidy_subject("Downloads that's", true), "Downloads");
        assert_eq!(tidy_subject("Plan A", false), "Plan A", "a plain name is left alone");
        assert_eq!(tidy_subject("report.pdf from", true), "report.pdf from", "a file name is left alone");
    }

    #[test]
    fn a_name_keyword_needs_a_name() {
        assert_eq!(parse_source("files named in Downloads"), None);
        assert_eq!(parse_source("files with budget"), None, "a bare 'with' names nothing");
    }

    #[test]
    fn with_inside_a_file_name_stays_in_the_name() {
        assert_eq!(parse_source("Mom with Dad.pdf"), Some(Source::item("Mom with Dad.pdf")));
        assert_eq!(
            parse_source("files with budget in the name"),
            Some(Source::set(None, Filter { file_type: Some(FileType::Files), name_contains: Some("budget".into()), ..Default::default() }))
        );
    }

    #[test]
    fn a_type_word_is_not_dropped_for_a_file_name() {
        assert_eq!(parse_source("photos named report.pdf"), None);
        assert_eq!(parse_source("file called passport.pdf"), Some(Source::item("passport.pdf")));
    }

    #[test]
    fn an_age_too_large_to_hold_is_refused() {
        assert_eq!(to_days("99999999999", "years"), None);
        assert_eq!(to_days("2", "weeks"), Some(14));
        assert_eq!(to_days("a", "month"), Some(30));
    }

    #[test]
    fn a_second_day_word_is_refused() {
        assert_eq!(parse_source("screenshots from yesterday or today"), None);
    }

    #[test]
    fn vague_subjects_are_refused_but_real_names_are_not() {
        for vague in ["big", "old 1", "old one", "them", "recent files"] {
            assert_eq!(parse_source(vague), None, "{vague}");
        }
        for name in ["Report 1", "Plan A"] {
            assert_eq!(parse_source(name), Some(Source::item(name)), "{name}");
        }
    }

    #[test]
    fn a_type_word_must_fit_the_file_name() {
        assert_eq!(parse_source("video called intro.mov"), Some(Source::item("intro.mov")));
        assert_eq!(parse_source("photos named report.pdf"), None);
    }

    #[test]
    fn and_joins_a_name_only_when_it_is_one_file() {
        assert_eq!(parse_source("file called Terms and Conditions.pdf"), Some(Source::item("Terms and Conditions.pdf")));
        assert_eq!(parse_source("files named budget and invoice"), None);
        assert_eq!(parse_source("file called notes.txt and todo.txt"), None, "two files, not one name");
    }

    #[test]
    fn a_connector_before_a_folder_goes_once_a_filter_is_gone() {
        assert_eq!(tidy_subject("PDFs and in Documents", true), "PDFs in Documents");
        assert_eq!(tidy_subject("PDFs and in Documents", false), "PDFs and in Documents", "without filters nothing is removed");
    }

    #[test]
    fn named_like_names_what_follows_like() {
        assert_eq!(
            parse_source("files named like invoice"),
            Some(Source::set(None, Filter { file_type: Some(FileType::Files), name_contains: Some("invoice".into()), ..Default::default() }))
        );
    }

    #[test]
    fn a_question_needs_a_set_of_files() {
        assert_eq!(set_source("report.pdf"), None);
        assert!(set_source("PDFs").is_some());
    }
}
