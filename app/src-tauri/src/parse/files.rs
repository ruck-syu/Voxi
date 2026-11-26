//! Move and copy.

use super::{
    Pattern, RefusalReason, cap, checked_pat,
    phrase::{folder_target, parse_source, split_arguments, split_destination_filter},
};
use crate::model::{Destination, Intent, Source};

fn transfer(source: &str, target: &str, copy: bool) -> Option<Result<Intent, RefusalReason>> {
    let (target, filter_tail) = split_destination_filter(target).unwrap_or((target, ""));
    let to = match folder_target(target) {
        Ok(to) => to?,
        Err(reason) => return Some(Err(reason)),
    };
    let source = if filter_tail.is_empty() { source.to_string() } else { format!("{source} {filter_tail}") };
    let parsed = parse_source(&source)?;
    if !filter_tail.is_empty() && !matches!(parsed, Source::Set { .. }) {
        return None;
    }
    let to = transfer_target(target, to)?;
    Some(Ok(if copy { Intent::Copy { src: parsed, to } } else { Intent::Move { src: parsed, to } }))
}

fn transfer_target(text: &str, to: Destination) -> Option<Destination> {
    let Destination::Path(name) = &to else { return Some(to) };
    // A quoted "Trash" or "bin" names a folder, not the Trash: the trailing
    // slash keeps it a folder path, which the planner never reads as the Trash.
    if text.trim().starts_with(['"', '\'']) && matches!(name.to_lowercase().as_str(), "trash" | "bin" | "recycle bin") {
        return Some(format!("{name}/").into());
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
    Some(to)
}

pub fn patterns() -> Vec<Pattern> {
    vec![
        checked_pat("put_in", r"^put\s+(.+)$", |c| {
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
            transfer(source, target, false)
        }),
        checked_pat("copy", r"^(?:copy|duplicate)\s+(.+)$", |c| {
            let text = cap(c, 1)?;
            let (source, target) = split_arguments(&text, &["to", "into"])?;
            transfer(source, target, true)
        }),
        checked_pat("move", r"^(?:move|put|send|drop|transfer|shift|relocate|stash|store|file)\s+(.+)$", |c| {
            let text = cap(c, 1)?;
            let (source, target) = split_arguments(&text, &["to", "into"])?;
            transfer(source, target, false)
        }),
    ]
}
