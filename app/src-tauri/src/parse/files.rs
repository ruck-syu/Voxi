//! Move and copy.

use super::{Pattern, cap, pat, phrase::{bad_destination, clean_name, parse_source, split_arguments}};
use crate::model::Intent;

fn destination(text: &str) -> Option<String> {
    if bad_destination(text) { return None; }
    let name = clean_name(text);
    // A delivery request must never silently become a local folder creation.
    // Quoted names and explicit paths still let users name a real email folder.
    if !text.starts_with(['"', '\'']) && !name.contains('/') {
        let lower = name.to_ascii_lowercase();
        if ["email", "e-mail", "mail", "inbox"].contains(&lower.as_str()) || lower.contains('@') {
            return None;
        }
    }
    Some(name)
}

pub fn patterns() -> Vec<Pattern> {
    vec![
        pat("copy", r"^(?:copy|duplicate)\s+(.+)$", |c| {
            let text = cap(c, 1)?;
            let (source, target) = split_arguments(&text, &["to", "into"])?;
            Some(Intent::Copy { src: parse_source(source)?, to: destination(target)? })
        }),
        pat("move", r"^(?:move|put|send|drop|transfer|shift|relocate|stash|store|file)\s+(.+)$", |c| {
            let text = cap(c, 1)?;
            let (source, target) = split_arguments(&text, &["to", "into"])?;
            Some(Intent::Move { src: parse_source(source)?, to: destination(target)? })
        }),
    ]
}
