//! Move and copy.

use super::{Pattern, cap, pat, phrase::clean_name, src};
use crate::model::Intent;

pub fn patterns() -> Vec<Pattern> {
    vec![
        pat("copy", r"^(?:copy|duplicate)\s+(.+?)\s+(?:to|into)\s+(.+)$", |c| {
            Some(Intent::Copy { src: src(c, 1)?, to: clean_name(&cap(c, 2)?) })
        }),
        pat("move", r"^(?:move|put|send|drop|transfer|file)\s+(.+?)\s+(?:to|into)\s+(.+)$", |c| {
            Some(Intent::Move { src: src(c, 1)?, to: clean_name(&cap(c, 2)?) })
        }),
    ]
}
