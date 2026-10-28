//! Zip and unzip (run with macOS `ditto`).

use super::{Pattern, cap, pat, phrase::clean_name, src};
use crate::model::Intent;

pub fn patterns() -> Vec<Pattern> {
    vec![
        pat(
            "unzip",
            r"^(?:unzip|decompress|expand|extract|unpack)\s+(.+?\.zip)(?:\s+(?:into|to|in)\s+(.+))?$",
            |c| Some(Intent::Unzip { src: src(c, 1)?, destination: cap(c, 2).map(|s| clean_name(&s)) }),
        ),
        pat("zip", r"^(?:zip(?:\s+up)?|compress|archive)\s+(.+?)(?:\s+(?:into|to|as)\s+(.+))?$", |c| {
            Some(Intent::Zip { src: src(c, 1)?, output: cap(c, 2).map(|s| clean_name(&s)) })
        }),
    ]
}
