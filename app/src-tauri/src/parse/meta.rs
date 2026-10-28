//! Undo, history, and opening things in Finder.

use super::{Pattern, cap, pat, phrase::clean_name};
use crate::model::Intent;
use regex::Regex;
use std::sync::LazyLock;

pub fn patterns() -> Vec<Pattern> {
    vec![
        pat(
            "undo",
            r"^(?:undo(?:\s+(?:last|that|it|this|previous|last\s+(?:change|action|one|command)))?|take\s+(?:that|it|this)\s+back|revert(?:\s+(?:that|it|this|last))?)$",
            |_| Some(Intent::Undo),
        ),
        pat(
            "recent_actions",
            r"^(?:(?:show|list|view|see)\s+(?:me\s+)?(?:(?:recent|last|previous|past)\s+)?(?:actions|changes|commands|history)|history|recent\s+actions)$",
            |_| Some(Intent::RecentActions),
        ),
        pat("reveal_in_finder", r"^(?:open|reveal|show)\s+(?:me\s+)?(.+?)\s+in\s+finder$", |c| {
            Some(Intent::Reveal { target: reveal_target(&cap(c, 1)?) })
        }),
        pat("reveal", r"^reveal\s+(.+)$", |c| Some(Intent::Reveal { target: reveal_target(&cap(c, 1)?) })),
    ]
}

/// "open Downloads": tried last so it never shadows a more specific command.
pub fn open() -> Vec<Pattern> {
    vec![pat("open", r"^open\s+(.+)$", |c| Some(Intent::Reveal { target: reveal_target(&cap(c, 1)?) }))]
}

static FILE_NAMED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^(?:file|folder)\s+(?:named|called)\s+").unwrap());

/// "the file named notes.txt" -> "notes.txt".
fn reveal_target(s: &str) -> String {
    clean_name(&FILE_NAMED.replace(s.trim(), "").to_string())
}
