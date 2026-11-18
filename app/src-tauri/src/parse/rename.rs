//! Renaming: single files and batch rules (prefix, replace, number, lowercase).

use super::{Pattern, cap, pat, phrase::{bad_destination, clean_name, parse_source, split_arguments}, typed_src};
use crate::model::{FileType, Intent, Source};
use regex::Regex;
use std::sync::LazyLock;

static NUMBERED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(.*?)(\d+)$").unwrap());

/// Turns a rename target into the base and width used by the existing
/// numbering planner. "holiday-1, holiday-2" -> ("holiday-", 1); a bare base
/// gets a separator and the default three-digit width.
fn number_template(target: &str) -> Option<(String, usize)> {
    let mut first = clean_name(target.split(',').next()?.trim());
    first = first.trim_end_matches(|c: char| c.is_ascii_whitespace() || matches!(c, '.' | '!' | '?')).to_string();
    for suffix in [" onwards", " onward"] {
        if let Some(stem) = first.strip_suffix(suffix) { first = stem.trim().to_string(); }
    }
    if first.is_empty() { return None; }
    let stem = match first.rsplit_once('.') {
        Some((stem, ext)) if ext.chars().all(|ch| ch.is_ascii_alphabetic()) => stem.to_string(),
        _ => first.to_string(),
    };
    if let Some(m) = NUMBERED.captures(&stem) {
        return Some((m[1].to_string(), m[2].len()));
    }
    // Avoid doubling an existing separator ("holiday-" must not become
    // "holiday--").
    let base = if stem.ends_with(['-', '_', '.', ' ']) { stem } else { format!("{stem}-") };
    Some((base, 3))
}

/// "spaces" -> " ", "dashes" -> "-", anything else unchanged.
fn spoken_chars(s: &str) -> String {
    let s = s.trim();
    if s.starts_with(['"', '\'']) { return clean_name(s); }
    let s = if s.get(..5).is_some_and(|p| p.eq_ignore_ascii_case("word ")) { &s[5..] } else { s };
    match s.to_ascii_lowercase().as_str() {
        "space" | "spaces" | "space characters" => " ".into(),
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

static NAMES_CONTEXT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^(?:file\s*)?names?\s+(?:(in|of|for)\s+)?(.+)$").unwrap());

/// An explicit "names in <folder>" must not turn Documents into a file type.
fn name_source(text: &str) -> Option<Source> {
    if let Some(c) = NAMES_CONTEXT.captures(text) {
        return if c.get(1).is_some_and(|m| m.as_str().eq_ignore_ascii_case("in")) {
            parse_source(&format!("files in {}", &c[2]))
        } else { parse_source(&c[2]) };
    }
    if ["name", "names", "file name", "file names"].iter().any(|n| text.eq_ignore_ascii_case(n)) {
        return parse_source("files");
    }
    // "PDF names in Pictures" is a batch source, not an item named "PDF names".
    static TYPE_NAMES: LazyLock<Regex> = LazyLock::new(|| Regex::new(
        r"(?i)^(.+?)(?:'s|s')?\s+names?(\s+(?:in|from|on|inside|within|under)\s+.+)?$"
    ).unwrap());
    if let Some(c) = TYPE_NAMES.captures(text)
        && super::phrase::type_word(&c[1]).is_some()
    {
        return parse_source(&format!("{}{}", &c[1], c.get(2).map_or("", |m| m.as_str())));
    }
    parse_source(text)
}

pub fn patterns() -> Vec<Pattern> {
    vec![
        pat(
            "add_date_prefix",
            r"^add\s+(?:a\s+)?(?:date|dates|today's\s+date|creation\s+date)\s+(?:as\s+(?:a\s+)?)?prefix(?:es)?\s+to\s+(.+)$",
            |c| Some(Intent::AddDatePrefix { src: name_source(&cap(c, 1)?)? }),
        ),
        pat("add_prefix", r"^add\s+(?:a\s+)?prefix\s+(.+?)\s+to\s+(.+)$", |c| {
            Some(Intent::AddPrefix { src: name_source(&cap(c, 2)?)?, prefix: spoken_suffix(&cap(c, 1)?) })
        }),
        pat("replace_in_names", r"^replace\s+(.+?)\s+with\s+(.+?)\s+in\s+(.+)$", |c| {
            Some(Intent::ReplaceInNames { src: name_source(&cap(c, 3)?)?, find: spoken_chars(&cap(c, 1)?), replace: spoken_chars(&cap(c, 2)?) })
        }),
        pat("remove_from_names", r"^(?:remove|strip)\s+(spaces|dashes|underscores|dots)\s+(?:from|in)\s+(.+)$", |c| {
            Some(Intent::ReplaceInNames { src: name_source(&cap(c, 2)?)?, find: spoken_chars(&cap(c, 1)?), replace: String::new() })
        }),
        pat("number", r"^(?:number|renumber|sequence)\s+(.+?)\s+(?:as|like|to)\s+(.+)$", |c| {
            // The first example is a format template. Quotes mark its literal
            // text, while its trailing digits supply the sequence width.
            let (base, width) = number_template(&cap(c, 2)?)?;
            Some(Intent::Number { src: typed_src(c, 1, FileType::Files)?, base, width })
        }),
        // "rename all screenshots to holiday-1, holiday-2" is a sequential
        // batch rename, not one file to one literal new name. It reuses the
        // same numbering planner as "number ... as ...".
        pat("rename_batch", r"^rename\s+(.+?)\s+to\s+(.+)$", |c| {
            let src = super::phrase::set_source(&cap(c, 1)?)?;
            // "the photo named sunset" is one file, not a set to renumber;
            // a single numbered example ("to Shot-1") stays ambiguous.
            if matches!(&src, Source::Set { filter, .. } if filter.name_contains.is_some() || filter.name_starts_with.is_some()) { return None; }
            let target = cap(c, 2)?;
            let first = clean_name(target.split(',').next()?.trim());
            if !target.contains(',') && NUMBERED.is_match(first.trim()) { return None; }
            let (base, width) = number_template(&target)?;
            Some(Intent::Number { src, base, width })
        }),
        pat("lowercase", r"^(?:lowercase|lower\s*case|make\s+lowercase)\s+(.+)$", |c| {
            Some(Intent::Lowercase { src: name_source(&cap(c, 1)?)? })
        }),
        pat("make_lowercase", r"^make\s+((?:file\s*)?names?\s+(?:(?:in|of)\s+)?.+?)\s+lower\s*case$", |c| {
            Some(Intent::Lowercase { src: name_source(&cap(c, 1)?)? })
        }),
        pat("rename_name_of", r"^(?:change|set)\s+(?:file\s+)?names?\s+of\s+(.+)$", |c| {
            let text = cap(c, 1)?;
            let (source, target) = split_arguments(&text, &["to"])?;
            let src = parse_source(source)?;
            if !matches!(src, Source::Item { .. }) {
                return None;
            }
            if bad_destination(target) { return None; }
            Some(Intent::Rename { src, new_name: clean_name(target) })
        }),
        pat("give_name", r"^give\s+(.+?)\s+name\s+(.+)$", |c| {
            let src = parse_source(&cap(c, 1)?)?;
            let target = cap(c, 2)?;
            if !matches!(src, Source::Item { .. }) || bad_destination(&target) { return None; }
            Some(Intent::Rename { src, new_name: clean_name(&target) })
        }),
        pat("set_to_name", r"^set\s+(.+?)\s+to\s+name\s+(.+)$", |c| {
            let src = parse_source(&cap(c, 1)?)?;
            let target = cap(c, 2)?;
            if !matches!(src, Source::Item { .. }) || bad_destination(&target) { return None; }
            Some(Intent::Rename { src, new_name: clean_name(&target) })
        }),
        pat("make_called", r"^make\s+(.+?)\s+called\s+(.+)$", |c| {
            let src = parse_source(&cap(c, 1)?)?;
            let target = cap(c, 2)?;
            if !matches!(src, Source::Item { .. }) || bad_destination(&target) { return None; }
            Some(Intent::Rename { src, new_name: clean_name(&target) })
        }),
        pat("change_possessive_name", r"^(?:change|set)\s+(.+?)'s\s+name\s+to\s+(.+)$", |c| {
            let source = parse_source(&cap(c, 1)?)?;
            let target = cap(c, 2)?;
            if !matches!(source, Source::Item { .. }) || bad_destination(&target) { return None; }
            Some(Intent::Rename { src: source, new_name: clean_name(&target) })
        }),
        pat("change_name", r"^change\s+(.+)$", |c| {
            let text = cap(c, 1)?;
            let (source, target) = split_arguments(&text, &["to"])?;
            let src = parse_source(source)?;
            if !matches!(src, Source::Item { .. }) || bad_destination(target) || !super::phrase::has_extension(target) { return None; }
            Some(Intent::Rename { src, new_name: clean_name(target) })
        }),
        pat("rename", r"^rename\s+(.+)$", |c| {
            let text = cap(c, 1)?;
            let (source, target) = split_arguments(&text, &["to", "as"])?;
            let src = parse_source(source)?;
            if !matches!(src, Source::Item { .. }) {
                return None;
            }
            if bad_destination(target) { return None; }
            Some(Intent::Rename { src, new_name: clean_name(target) })
        }),
    ]
}
