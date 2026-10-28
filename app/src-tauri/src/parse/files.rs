//! Move and copy.

use super::{Pattern, cap, pat, phrase::{bad_destination, clean_name}, src};
use crate::model::Intent;

pub fn patterns() -> Vec<Pattern> {
    vec![
        pat("copy", r"^(?:copy|duplicate)\s+(.+?)\s+(?:to|into)\s+(.+)$", |c| {
            let to = clean_name(&cap(c, 2)?);
            if bad_destination(&to) {
                return None;
            }
            Some(Intent::Copy { src: src(c, 1)?, to })
        }),
        pat("move", r"^(?:move|put|send|drop|transfer|shift|relocate|stash|store|file)\s+(.+?)\s+(?:to|into)\s+(.+)$", |c| {
            let to = clean_name(&cap(c, 2)?);
            if bad_destination(&to) {
                return None;
            }
            Some(Intent::Move { src: src(c, 1)?, to })
        }),
    ]
}
