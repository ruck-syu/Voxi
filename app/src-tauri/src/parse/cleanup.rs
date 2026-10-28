//! Delete and clean-up commands. These never hard-delete: the planner turns
//! them into moves into `_to_delete/`.

use super::{Pattern, pat, src};
use crate::model::Intent;

pub fn patterns() -> Vec<Pattern> {
    vec![
        pat(
            "move_to_trash",
            r"^(?:move|put|send|throw|transfer|drop|shift|stash|store)\s+(.+?)\s+(?:to|into|in)\s+(?:trash|bin|_to_delete|to\s+delete)(?:\s+folder)?$",
            |c| Some(Intent::Trash { src: src(c, 1)? }),
        ),
        pat("delete", r"^(?:delete|trash|remove|bin|erase|throw\s+away|get\s+rid\s+of)\s+(.+)$", |c| {
            Some(Intent::Trash { src: src(c, 1)? })
        }),
        pat("clean_up", r"^clean\s*up\s+(.+)$", |c| Some(Intent::Trash { src: src(c, 1)? })),
    ]
}
