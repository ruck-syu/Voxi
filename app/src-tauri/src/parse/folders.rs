//! Creating folders and sorting a folder into subfolders.

use super::{Pattern, cap, pat, phrase::{bad_destination, clean_name, split_arguments, unquoted_text}};
use crate::model::Intent;
use regex::Regex;
use std::sync::LazyLock;

static LIST_SEP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\s*,\s*(?:and\s+)?|\s+and\s+").unwrap());

pub fn patterns() -> Vec<Pattern> {
    vec![
        pat(
            "create_folders_location_first",
            r"^(?:create|make|add|new)\s+(?:a\s+)?(?:new\s+)?folders?\s+(?:in|on|inside|under)\s+(.+)$",
            |c| {
                let rest = cap(c, 1)?;
                let (parent, names) = split_arguments(&rest, &["named", "called"])?;
                let parent = clean_name(parent);
                if bad_destination(&parent) { return None; }
                let unquoted = unquoted_text(names)?;
                let mut start = 0;
                let mut result = Vec::new();
                for separator in LIST_SEP.find_iter(&unquoted) {
                    result.push(clean_name(&names[start..separator.start()]));
                    start = separator.end();
                }
                result.push(clean_name(&names[start..]));
                let names = result;
                if names.is_empty() || names.iter().any(String::is_empty) { return None; }
                Some(Intent::CreateFolders { names, parent: Some(parent) })
            },
        ),
        pat(
            "create_folders",
            r"^(?:create|make|add|new)\s+(?:a\s+)?(?:new\s+)?folders?\s+(?:called\s+|named\s+|for\s+)?(.+?)(?:\s+(?:in|on|inside|under)\s+(.+))?$",
            |c| {
                let names: Vec<String> =
                    LIST_SEP.split(&cap(c, 1)?).map(clean_name).filter(|s| !s.is_empty()).collect();
                // "create a folder on the desktop" names no folder: "on the desktop" is a place.
                if names.is_empty() || names.iter().any(|n| ["on ", "in ", "inside ", "under "].iter().any(|p| n.to_lowercase().starts_with(p))) {
                    return None;
                }
                Some(Intent::CreateFolders { names, parent: cap(c, 2).map(|p| clean_name(&p)) })
            },
        ),
        pat(
            "sort",
            r"^(?:sort|organize|organise|group|tidy\s+up|clean\s*up)\s+(.+?)\s+(?:by|into)\s+(type|types|kind|file\s+type|extension|month|months|date)(?:\s+folders)?$",
            |c| {
                let folder = clean_name(&cap(c, 1)?);
                let by = cap(c, 2)?.to_ascii_lowercase();
                Some(if by.starts_with("month") || by == "date" {
                    Intent::SortByMonth { folder }
                } else {
                    Intent::SortByType { folder }
                })
            },
        ),
    ]
}
