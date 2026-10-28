//! Renaming: single files and batch rules (prefix, replace, number, lowercase).

use super::{Pattern, cap, pat, phrase::clean_name, src, typed_src};
use crate::model::{FileType, Intent, Source};
use regex::Regex;
use std::sync::LazyLock;

static NUMBERED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(.*?)(\d+)$").unwrap());

/// "spaces" -> " ", "dashes" -> "-", anything else unchanged.
fn spoken_chars(s: &str) -> String {
    match s.trim().to_ascii_lowercase().as_str() {
        "space" | "spaces" => " ".into(),
        "dash" | "dashes" | "hyphen" | "hyphens" | "minus" => "-".into(),
        "underscore" | "underscores" => "_".into(),
        "dot" | "dots" | "period" | "periods" => ".".into(),
        "nothing" | "empty" | "blank" => "".into(),
        _ => s.trim().trim_matches('"').to_string(),
    }
}

/// "Invoice dash" -> "Invoice-".
fn spoken_suffix(s: &str) -> String {
    let s = s.trim().trim_matches('"');
    for (word, ch) in [(" dash", "-"), (" hyphen", "-"), (" underscore", "_"), (" space", " ")] {
        if let Some(stem) = s.strip_suffix(word) {
            return format!("{stem}{ch}");
        }
    }
    s.to_string()
}

const NAMES_IN: &str = r"(?:(?:file\s+)?names?\s+(?:of\s+|in\s+|for\s+)?)?";

pub fn patterns() -> Vec<Pattern> {
    vec![
        pat(
            "add_date_prefix",
            r"^add\s+(?:a\s+)?(?:date|dates|today's\s+date|creation\s+date)\s+(?:as\s+(?:a\s+)?)?prefix(?:es)?\s+to\s+(.+)$",
            |c| Some(Intent::AddDatePrefix { src: src(c, 1)? }),
        ),
        pat("add_prefix", r"^add\s+(?:a\s+)?prefix\s+(.+?)\s+to\s+(.+)$", |c| {
            Some(Intent::AddPrefix { src: src(c, 2)?, prefix: spoken_suffix(&cap(c, 1)?) })
        }),
        pat("replace_in_names", &format!(r"^replace\s+(.+?)\s+with\s+(.+?)\s+in\s+{NAMES_IN}(.+)$"), |c| {
            Some(Intent::ReplaceInNames { src: src(c, 3)?, find: spoken_chars(&cap(c, 1)?), replace: spoken_chars(&cap(c, 2)?) })
        }),
        pat("remove_from_names", &format!(r"^(?:remove|strip)\s+(spaces|dashes|underscores|dots)\s+(?:from|in)\s+{NAMES_IN}(.+)$"), |c| {
            Some(Intent::ReplaceInNames { src: src(c, 2)?, find: spoken_chars(&cap(c, 1)?), replace: String::new() })
        }),
        pat("number", r"^(?:number|renumber|sequence)\s+(.+?)\s+(?:as|like|to)\s+(.+)$", |c| {
            let first = cap(c, 2)?.split(',').next()?.trim().to_string();
            let stem = match first.rsplit_once('.') {
                Some((stem, ext)) if ext.chars().all(|ch| ch.is_ascii_alphabetic()) => stem.to_string(),
                _ => first,
            };
            let (base, width) = match NUMBERED.captures(&stem) {
                Some(m) => (m[1].to_string(), m[2].len()),
                None => (format!("{stem}-"), 3),
            };
            Some(Intent::Number { src: typed_src(c, 1, FileType::Files)?, base, width })
        }),
        pat("lowercase", &format!(r"^(?:lowercase|lower\s*case|make\s+lowercase)\s+{NAMES_IN}(.+)$"), |c| {
            Some(Intent::Lowercase { src: src(c, 1)? })
        }),
        pat("make_lowercase", r"^make\s+(?:file\s+)?names?\s+(?:in\s+|of\s+)?(.+?)\s+lower\s*case$", |c| {
            Some(Intent::Lowercase { src: src(c, 1)? })
        }),
        pat("rename", r"^rename\s+(.+?)\s+(?:to|as)\s+(.+)$", |c| {
            let src = src(c, 1)?;
            if !matches!(src, Source::Item { .. }) {
                return None;
            }
            Some(Intent::Rename { src, new_name: clean_name(&cap(c, 2)?) })
        }),
    ]
}
