//! Move and copy.

use super::{Pattern, cap, pat, phrase::{bad_destination, clean_name, parse_source, split_arguments}};
use crate::model::Intent;

pub fn patterns() -> Vec<Pattern> {
    vec![
        pat("copy", r"^(?:copy|duplicate)\s+(.+)$", |c| {
            let text = cap(c, 1)?;
            let (source, target) = split_arguments(&text, &["to", "into"])?;
            if bad_destination(target) {
                return None;
            }
            Some(Intent::Copy { src: parse_source(source)?, to: clean_name(target) })
        }),
        pat("move", r"^(?:move|put|send|drop|transfer|shift|relocate|stash|store|file)\s+(.+)$", |c| {
            let text = cap(c, 1)?;
            let (source, target) = split_arguments(&text, &["to", "into"])?;
            if bad_destination(target) {
                return None;
            }
            Some(Intent::Move { src: parse_source(source)?, to: clean_name(target) })
        }),
    ]
}
