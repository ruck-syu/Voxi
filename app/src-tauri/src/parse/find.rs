//! Read-only queries: list, size, count, duplicates.

use super::{Pattern, cap, pat, phrase::{bad_destination, clean_name}, src};
use crate::model::Intent;

pub fn specific() -> Vec<Pattern> {
    vec![
        pat("what_named_file", r"^what\s+files?\s+(?:are\s+)?(?:named|called)\s+(.+)$", |c| {
            Some(Intent::List { src: super::phrase::parse_source(&format!("file named {}", cap(c, 1)?))? })
        }),
        pat("count_reworded", r"^(?:tell\s+me\s+(?:number|count)\s+of\s+(?:files|items)\s+in|give\s+me\s+(?:a\s+)?file\s+count\s+for|number\s+of\s+files\s+under|what\s+is\s+file\s+count\s+in)\s+(.+)$", |c| {
            Some(Intent::Count { src: super::phrase::parse_source(&format!("files in {}", cap(c, 1)?))? })
        }),
        pat("how_many_contained", r"^how\s+many\s+(?:files|items)\s+does\s+(.+?)\s+(?:contain|have)$", |c| {
            Some(Intent::Count { src: super::phrase::parse_source(&format!("files in {}", cap(c, 1)?))? })
        }),
        pat(
            "size_of",
            r"^(?:(?:show|what\s+is|what's|get|check|calculate|tell\s+me)\s+(?:me\s+)?)?(?:total\s+)?size\s+of\s+(.+)$",
            |c| Some(Intent::Size { folder: folder(c, 1)? }),
        ),
        pat("how_big", r"^how\s+big\s+is\s+(.+)$", |c| Some(Intent::Size { folder: folder(c, 1)? })),
        pat(
            "duplicates",
            r"^(?:list|find|show|search\s+for)\s+(?:me\s+)?duplicates?(?:\s+files)?\s+(?:in|from|on)\s+(.+)$",
            |c| Some(Intent::Duplicates { folder: folder(c, 1)? }),
        ),
        pat("count", r"^count\s+(.+)$", |c| Some(Intent::Count { src: src(c, 1)? })),
        pat("how_many", r"^how\s+many\s+(.+?)\s+(?:are\s+there\s+|are\s+|do\s+i\s+have\s+)?(in|on|from)\s+(.+)$", |c| {
            let phrase = format!("{} in {}", cap(c, 1)?, cap(c, 3)?);
            Some(Intent::Count { src: super::phrase::parse_source(&phrase)? })
        }),
        // "how many screenshots did I take today": a count of a set with a type or filter.
        pat("how_many_any", r"^how\s+many\s+(.+)$", |c| Some(Intent::Count { src: super::phrase::set_source(&cap(c, 1)?)? })),
        pat(
            "how_much_space",
            r"^how\s+much\s+(?:space|storage|room)\s+(?:is\s+|does\s+)?(.+?)\s+(?:is\s+)?(?:using|takes?(?:\s+up)?|uses?)$",
            |c| Some(Intent::Size { folder: folder(c, 1)? }),
        ),
    ]
}

pub fn general() -> Vec<Pattern> {
    vec![
        pat("list", r"^(?:list|find|show|display|search\s+for|search|look\s+for)\s+(?:me\s+)?(?:what\s+)?(?:contents?\s+of\s+|what'?s\s+(?:on|in)\s+)?(.+)$", |c| {
            Some(Intent::List { src: src(c, 1)? })
        }),
        pat("whats_in", r"^(?:what'?s|what\s+is)\s+(?:on|in)\s+(.+)$", |c| Some(Intent::List { src: src(c, 1)? })),
        // "what files did I download yesterday": a question lists only a set of files.
        pat("what_list", r"^what\s+(?:are\s+|is\s+)?(.+)$", |c| Some(Intent::List { src: super::phrase::set_source(&cap(c, 1)?)? })),
    ]
}

fn folder(c: &regex::Captures, i: usize) -> Option<String> {
    let raw = cap(c, i)?;
    (!bad_destination(&raw)).then(|| clean_name(&raw))
}
