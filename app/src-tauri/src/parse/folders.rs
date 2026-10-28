//! Creating folders and sorting a folder into subfolders.

use super::{Pattern, cap, pat, phrase::clean_name};
use crate::model::Intent;
use regex::Regex;
use std::sync::LazyLock;

static LIST_SEP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\s*,\s*(?:and\s+)?|\s+and\s+").unwrap());

pub fn patterns() -> Vec<Pattern> {
    vec![
        pat(
            "create_folders",
            r"^(?:create|make|add|new)\s+(?:a\s+)?(?:new\s+)?folders?\s+(?:called\s+|named\s+)?(.+?)(?:\s+(?:in|on|inside|under)\s+(.+))?$",
            |c| {
                let names: Vec<String> =
                    LIST_SEP.split(&cap(c, 1)?).map(clean_name).filter(|s| !s.is_empty()).collect();
                if names.is_empty() {
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
