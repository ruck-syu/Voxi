//! Undo, history, and opening things in Finder.

use super::{Pattern, cap, pat, phrase::clean_name};
use crate::model::{Intent, Source};
use regex::Regex;
use std::sync::LazyLock;

pub fn patterns() -> Vec<Pattern> {
    vec![
        pat(
            "undo",
            r"^(?:undo|revert|take\s+back|take\s+(?:that|it|this)\s+back)(?:\s+(?:that|it|this|the|last|previous|thing|one|change|action|command|step|what|you|just|did|i))*$",
            |_| Some(Intent::Undo),
        ),
        pat("history_done", r"^(?:show|display|list)\s+(?:me\s+)?(?:(?:log|record)\s+of\s+)?(?:what\s+(?:i've|i\s+have|i|you've|you\s+have|you)\s+(?:done|did)(?:\s+(?:so\s+far|up\s+till(?:\s+now)?|up\s+to\s+now))?|commands\s+i\s+used)$", |_| Some(Intent::RecentActions)),
        pat("history_phrases", r"^(?:show|display|list)\s+(?:me\s+)?(?:log\s+history(?:\s+to\s+me)?|history\s+of\s+what\s+(?:i|you)\s+did|everything\s+(?:i've|i\s+have|you've|you\s+have)\s+done(?:\s+so\s+far)?)$", |_| Some(Intent::RecentActions)),
        pat(
            "recent_actions",
            r"^(?:(?:show|list|view|see)\s+(?:me\s+)?(?:(?:recent|last|previous|past)\s+)?(?:actions|changes|commands|history)|history|recent\s+actions|what\s+changed(?:\s+(?:recently|lately))?|(?:show|display|view|list|open|see|pull\s+up|check)\s+(?:(?:to\s+)?me,?\s+)?(?:an?\s+)?(?:(?:(?:past|recent|earlier|previous|last)\s+)?(?:actions|changes|commands|operations)(?:\s+(?:history|log))?|history(?:\s+log)?|(?:list|log|record|history|rundown|timeline|summary)\s+of\s+(?:what\s+(?:i'?ve|i\s+have|you'?ve|you\s+have)\s+(?:been\s+)?(?:doing|done)|(?:(?:earlier|recent|past|previous|last)\s+)?(?:actions|changes|commands|operations|files?\s+operations))|(?:actions?|activity|command|operations?)\s+(?:history|log)))$",
            |_| Some(Intent::RecentActions),
        ),
        pat("reveal_in_finder", r"^(?:open|reveal|show)\s+(?:me\s+)?(.+?)\s+in\s+finder$", |c| {
            Some(Intent::Reveal { target: reveal_target(&cap(c, 1)?)? })
        }),
        pat("reveal", r"^reveal\s+(.+)$", |c| Some(Intent::Reveal { target: reveal_target(&cap(c, 1)?)? })),
        // "show me where report.pdf is". Plain "where is" stays a refusal.
        pat("where_is", r"^(?:show|tell)\s+me\s+where\s+(?:is\s+|are\s+)?(.+?)(?:\s+(?:is|are))?$", |c| {
            Some(Intent::Reveal { target: reveal_target(&cap(c, 1)?)? })
        }),
    ]
}

/// "open Downloads": tried last so it never shadows a more specific command.
pub fn open() -> Vec<Pattern> {
    vec![pat("open", r"^open\s+(?:up\s+)?(.+)$", |c| Some(Intent::Reveal { target: reveal_target(&cap(c, 1)?)? }))]
}

static FILE_NAMED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^(?:file|folder)\s+(?:named|called)\s+").unwrap());

/// "the file named notes.txt" -> "notes.txt".
fn reveal_target(s: &str) -> Option<String> {
    let source = FILE_NAMED.replace(s.trim(), "");
    match super::phrase::parse_source(&source)? {
        Source::Item { name, folder: None } => Some(name),
        // "report.pdf on my desktop" is one file in a named folder. Carry the
        // folder as a path so the planner resolves it instead of searching for
        // a file literally called "report.pdf on desktop".
        Source::Item { name, folder: Some(folder) } => Some(format!("{folder}/{name}")),
        _ => Some(clean_name(&source)),
    }
}
