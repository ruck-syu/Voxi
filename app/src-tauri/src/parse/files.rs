//! Move and copy.

use super::{
    Pattern, cap, pat,
    phrase::{bad_destination, clean_name, parse_source, split_arguments, split_destination_filter},
};
use crate::model::{Intent, Source};

fn transfer_source_and_target<'a>(source: &'a str, target: &'a str) -> Option<(Source, String)> {
    let (target, filter_tail) = split_destination_filter(target).unwrap_or((target, ""));
    if bad_destination(target) {
        return None;
    }
    let source = if filter_tail.is_empty() { source.to_string() } else { format!("{source} {filter_tail}") };
    let parsed = parse_source(&source)?;
    if !filter_tail.is_empty() && !matches!(parsed, Source::Set { .. }) {
        return None;
    }
    Some((parsed, target.to_string()))
}

fn destination(text: &str) -> Option<String> {
    if bad_destination(text) {
        return None;
    }
    let name = clean_name(text);
    // A quoted "Trash" or "bin" names a folder, not the Trash: the trailing
    // slash keeps it a folder path, which the planner never reads as the Trash.
    if text.trim().starts_with(['"', '\'']) && matches!(name.to_lowercase().as_str(), "trash" | "bin" | "recycle bin") {
        return Some(format!("{name}/"));
    }
    // A delivery request must never silently become a local folder creation.
    // Quoted names and explicit paths still let users name a real email folder.
    if !text.starts_with(['"', '\'']) && !name.contains('/') {
        let lower = name.trim_start_matches("my ").trim_start_matches("the ").to_ascii_lowercase();
        // "to my phone" / "to mom" is a delivery request, not a local folder.
        // Quoted names and explicit paths above stay literal on purpose.
        if ["email", "e-mail", "mail", "inbox", "phone", "mobile", "cell", "mom", "mum", "mother", "dad", "father", "parent", "accountant"]
            .contains(&lower.as_str())
            || lower.contains('@')
        {
            return None;
        }
    }
    Some(name)
}

pub fn patterns() -> Vec<Pattern> {
    vec![
        pat("put_in", r"^put\s+(.+)$", |c| {
            let text = cap(c, 1)?;
            // "put X in Downloads to Archive": "in" names where X is, so the
            // move pattern below reads it.
            if split_arguments(&text, &["to", "into"]).is_some() {
                return None;
            }
            let (source, target) = split_arguments(&text, &["in"])?;
            let syntax = super::phrase::unquoted_text(target)?.to_ascii_lowercase();
            if syntax.contains(" as ") || syntax.contains("together") {
                return None;
            }
            let (src, to) = transfer_source_and_target(source, target)?;
            Some(Intent::Move { src, to: destination(&to)? })
        }),
        pat("copy", r"^(?:copy|duplicate)\s+(.+)$", |c| {
            let text = cap(c, 1)?;
            let (source, target) = split_arguments(&text, &["to", "into"])?;
            let (src, to) = transfer_source_and_target(source, target)?;
            Some(Intent::Copy { src, to: destination(&to)? })
        }),
        pat("move", r"^(?:move|put|send|drop|transfer|shift|relocate|stash|store|file)\s+(.+)$", |c| {
            let text = cap(c, 1)?;
            let (source, target) = split_arguments(&text, &["to", "into"])?;
            let (src, to) = transfer_source_and_target(source, target)?;
            Some(Intent::Move { src, to: destination(&to)? })
        }),
    ]
}
