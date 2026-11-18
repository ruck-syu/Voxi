//! Creating folders and sorting a folder into subfolders.

use super::{Pattern, cap, pat, phrase::{bad_destination, clean_name, split_arguments, unquoted_text}};
use crate::model::Intent;
use regex::Regex;
use std::sync::LazyLock;

static LIST_SEP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\s*,\s*(?:and\s+)?|\s+and\s+").unwrap());

fn list_name(text: &str) -> String {
    let text = text.trim();
    let text = if text.get(..4).is_some_and(|s| s.eq_ignore_ascii_case("for ")) { &text[4..] } else { text };
    clean_name(text)
}

fn folder_names(text: &str) -> Option<Vec<String>> {
    let unquoted = unquoted_text(text)?;
    static INSTRUCTION: LazyLock<Regex> = LazyLock::new(|| Regex::new(
        r"(?i)(?:,|\band)\s*(?:put|move|copy|goes|rename|delete)\b"
    ).unwrap());
    if INSTRUCTION.is_match(&unquoted) { return None; }
    let mut start = 0;
    let mut names = Vec::new();
    for separator in LIST_SEP.find_iter(&unquoted) {
        names.push(list_name(&text[start..separator.start()]));
        start = separator.end();
    }
    names.push(list_name(&text[start..]));
    (!names.iter().any(String::is_empty)).then_some(names)
}

pub fn patterns() -> Vec<Pattern> {
    vec![
        pat(
            "create_folders_location_first",
            r"^(?:create|make|add|new)\s+(?:a\s+)?(?:new\s+)?folders?\s+(?:in|on|inside|within|under|to)\s+(.+)$",
            |c| {
                let rest = cap(c, 1)?;
                let (parent, names) = split_arguments(&rest, &["named", "called", "labeled", "labelled"])?;
                if bad_destination(parent) { return None; }
                let parent = clean_name(parent);
                let names = folder_names(names)?;
                Some(Intent::CreateFolders { names, parent: Some(parent) })
            },
        ),
        pat(
            "create_folders",
            r"^(?:create|make|add|new)\s+(?:a\s+)?(?:new\s+)?folders?\s+(?:called\s+|named\s+|labelled\s+|labeled\s+|titled\s+|for\s+)?(.+)$",
            |c| {
                let raw = cap(c, 1)?;
                let outside = unquoted_text(&raw)?;
                let separators = ["in", "on", "inside", "within", "under", "to"];
                let has_location = outside.split_whitespace().any(|word| separators.iter().any(|p| word.eq_ignore_ascii_case(p)));
                let (names, parent) = if has_location {
                    let (names, parent) = split_arguments(&raw, &separators)?;
                    if bad_destination(parent) { return None; }
                    (folder_names(names)?, Some(clean_name(parent)))
                } else {
                    (folder_names(&raw)?, None)
                };
                Some(Intent::CreateFolders { names, parent })
            },
        ),
        pat(
            "sort",
            r"^(?:sort|organize|organise|group|tidy\s+up|clean\s*up)\s+(.+?)\s+(?:by|into)\s+(type|types|kind|file\s+type|extension|month|months|date)(?:\s+folders?)?(?:\s+(?:including|with|and|plus)\s+(?:the\s+)?(?:sub-?folders?|nested\s+folders?|subdirectories|sub\s+folders))?$",
            |c| {
                let raw = cap(c, 1)?;
                if bad_destination(&raw) { return None; }
                let folder = clean_name(&raw);
                let by = cap(c, 2)?.to_ascii_lowercase();
                let include_subfolders = c[0].to_ascii_lowercase().contains("subfolders")
                    || c[0].to_ascii_lowercase().contains("sub-folders")
                    || c[0].to_ascii_lowercase().contains("sub folders")
                    || c[0].to_ascii_lowercase().contains("nested folders")
                    || c[0].to_ascii_lowercase().contains("subdirectories");
                Some(if by.starts_with("month") || by == "date" {
                    Intent::SortByMonth { folder, include_subfolders }
                } else {
                    Intent::SortByType { folder, include_subfolders }
                })
            },
        ),
    ]
}
