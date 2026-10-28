//! Undo, history, and opening things in Finder.

use super::{Pattern, cap, pat, phrase::clean_name};
use crate::model::Intent;

pub fn patterns() -> Vec<Pattern> {
    vec![
        pat("undo", r"^undo(?:\s+(?:last|that|it|this|previous|last\s+(?:change|action|one|command)))?$", |_| Some(Intent::Undo)),
        pat(
            "recent_actions",
            r"^(?:(?:show|list|view|see)\s+(?:me\s+)?(?:recent|last|previous|past)\s+(?:actions|changes|commands)|history|recent\s+actions)$",
            |_| Some(Intent::RecentActions),
        ),
        pat("reveal_in_finder", r"^(?:open|reveal|show)\s+(?:me\s+)?(.+?)\s+in\s+finder$", |c| {
            Some(Intent::Reveal { target: clean_name(&cap(c, 1)?) })
        }),
    ]
}

/// "open Downloads": tried last so it never shadows a more specific command.
pub fn open() -> Vec<Pattern> {
    vec![pat("open", r"^open\s+(.+)$", |c| Some(Intent::Reveal { target: clean_name(&cap(c, 1)?) }))]
}
