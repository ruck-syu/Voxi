//! Lossless lexical binding for the tiny rewrite model.
//!
//! This is deliberately limited to a directly supported folder-creation
//! contract. It changes only literal spans and list separators in a verified
//! names list. The model must still emit every alias in the correct role.

use crate::fallback_guard::{self, Rejection};
use crate::model::Intent;
use crate::parse::{self, ParseFailure};
use regex::Regex;
use std::sync::LazyLock;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrepareError {
    OriginalAccepted,
    OriginalHardRefusal,
    UnsupportedOriginal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompletionError {
    ModelError(String),
    Rejected(Rejection),
    AliasMismatch,
}

#[derive(Debug, Clone)]
struct Binding {
    alias: String,
    value: String,
    role: Role,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    Name(usize),
    Parent,
}

/// Prepared native-model request. Keep and finish this value with the output
/// of the same inference call; aliases are per-request and virtual.
#[derive(Debug, Clone)]
pub struct PreparedRewrite {
    pub model_input: String,
    bound_original: String,
    expected: Intent,
    bindings: Vec<Binding>,
}

impl PreparedRewrite {
    /// Complete an inference result. Native errors and noncertifying rewrites
    /// remain refusals; no source-derived intent is returned as a fallback.
    pub fn finish(
        &self,
        raw_model_rewrite: Result<String, String>,
    ) -> Result<Intent, CompletionError> {
        let candidate = raw_model_rewrite.map_err(CompletionError::ModelError)?;
        let parsed = fallback_guard::certify_bound(&self.bound_original, &candidate)
            .map_err(CompletionError::Rejected)?;
        restore(parsed, &self.expected, &self.bindings)
    }
}

/// Prepare only true grammar misses in the existing folder-creation domain.
/// Accepted commands and typed refusals retain parser priority.
pub fn prepare(original: &str) -> Result<PreparedRewrite, PrepareError> {
    // Match plan_model's input ceiling before any expensive parsing or binding.
    if original.len() > 16_384 {
        return Err(PrepareError::UnsupportedOriginal);
    }
    match parse::parse_classified(original) {
        Ok(_) => return Err(PrepareError::OriginalAccepted),
        Err(ParseFailure::HardRefusal(_)) => return Err(PrepareError::OriginalHardRefusal),
        Err(ParseFailure::GrammarMiss) => {}
    }
    let expected =
        fallback_guard::folder_creation(original).ok_or(PrepareError::UnsupportedOriginal)?;
    if fallback_guard::has_unquoted_unsupported_clause(original) {
        return Err(PrepareError::UnsupportedOriginal);
    }
    let Intent::CreateFolders { names, parent } = &expected else {
        return Err(PrepareError::UnsupportedOriginal);
    };
    let captures = FOLDER_CREATE
        .captures(original)
        .ok_or(PrepareError::UnsupportedOriginal)?;
    let name_match = captures
        .get(1)
        .or_else(|| captures.get(2))
        .ok_or(PrepareError::UnsupportedOriginal)?;
    let parent_match = captures.get(3).ok_or(PrepareError::UnsupportedOriginal)?;
    let raw_names = name_match.as_str();
    let values = split_literal_members(raw_names).ok_or(PrepareError::UnsupportedOriginal)?;
    if values != *names {
        return Err(PrepareError::UnsupportedOriginal);
    }
    let parent_value = parent.clone().ok_or(PrepareError::UnsupportedOriginal)?;

    let mut bindings = Vec::with_capacity(names.len() + 1);
    for (index, value) in names.iter().enumerate() {
        let alias = choose_alias(original, &bindings).ok_or(PrepareError::UnsupportedOriginal)?;
        bindings.push(Binding {
            alias,
            value: value.clone(),
            role: Role::Name(index),
        });
    }
    let parent_alias =
        choose_alias(original, &bindings).ok_or(PrepareError::UnsupportedOriginal)?;
    bindings.push(Binding {
        alias: parent_alias,
        value: parent_value,
        role: Role::Parent,
    });

    // Replace capture spans from right to left, preserving all other wording,
    // punctuation, casing and clauses byte-for-byte. Normalize commas only
    // between independently parsed quoted members.
    let mut model_input = original.to_string();
    let mut bound_original = original.to_string();
    let name_aliases: Vec<String> = bindings
        .iter()
        .filter(|b| matches!(b.role, Role::Name(_)))
        .map(|b| b.alias.clone())
        .collect();
    let bound_names =
        join_like_list(raw_names, &name_aliases).ok_or(PrepareError::UnsupportedOriginal)?;
    let parent_binding = bindings.last().unwrap();
    let parent_alias = parent_binding.alias.clone();
    let mut edits = vec![
        (name_match.start(), name_match.end(), bound_names),
        (parent_match.start(), parent_match.end(), parent_alias),
    ];
    edits.sort_by_key(|(start, _, _)| std::cmp::Reverse(*start));
    for (start, end, replacement) in edits {
        replace_span(&mut model_input, start, end, &replacement);
        replace_span(&mut bound_original, start, end, &replacement);
    }

    Ok(PreparedRewrite {
        model_input,
        bound_original,
        expected,
        bindings,
    })
}

static FOLDER_CREATE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
    r"(?i)^\s*(?:(?:please\s+)|(?:(?:could|would|can)\s+you\s+))?(?:make|create)\s+(?:(?:(?:a|the)\s+)?folders?\s+(?:(?:named|called)\s+)?(.+?)|(.+?)\s+folders?)\s+(?:in|on)\s+(.+?)\s*$"
).unwrap()
});

fn choose_alias(original: &str, used: &[Binding]) -> Option<String> {
    // Familiar ordinary vocabulary works with the trained tokenizer. Aliases
    // are case-insensitively unique within the request and avoid every source
    // literal, so they cannot collide with user values.
    let pool = [
        "Archive",
        "Backup",
        "Documents",
        "Desktop",
        "Downloads",
        "Pictures",
        "Projects",
        "Reports",
        "Invoices",
        "Receipts",
        "Music",
        "Videos",
        "Library",
        "Workspace",
        "Collection",
    ];
    pool.iter()
        .find(|candidate| {
            !original.to_lowercase().contains(&candidate.to_lowercase())
                && used
                    .iter()
                    .all(|b| !b.alias.eq_ignore_ascii_case(candidate))
        })
        .map(|word| (*word).to_string())
}

fn split_literal_members(s: &str) -> Option<Vec<String>> {
    let mut result = Vec::new();
    let mut start = 0;
    let mut quote: Option<char> = None;
    for (i, ch) in s.char_indices() {
        if i < start {
            continue;
        }
        if let Some(open) = quote {
            if ch == closing(open) {
                quote = None;
            }
        } else if matches!(ch, '"' | '\'' | '“' | '‘') {
            quote = Some(ch);
        } else if ch == ','
            || s[i..]
                .get(..5)
                .is_some_and(|suffix| suffix.eq_ignore_ascii_case(" and "))
        {
            let member = s[start..i].trim();
            result.push(unquote(member)?);
            start = if ch == ',' { i + 1 } else { i + 5 };
            while s[start..].starts_with(' ') {
                start += 1;
            }
        }
    }
    if quote.is_some() {
        return None;
    }
    result.push(unquote(s[start..].trim())?);
    (!result.is_empty()).then_some(result)
}

fn unquote(s: &str) -> Option<String> {
    let first = s.chars().next()?;
    let last = s.chars().last()?;
    if !matches!(first, '"' | '\'' | '“' | '‘') || last != closing(first) {
        return None;
    }
    let value = s[first.len_utf8()..s.len() - last.len_utf8()].to_string();
    (!value.is_empty() && !value.chars().any(char::is_control)).then_some(value)
}
fn closing(ch: char) -> char {
    match ch {
        '“' => '”',
        '‘' => '’',
        x => x,
    }
}

fn join_like_list(raw: &str, aliases: &[String]) -> Option<String> {
    let mut pieces = Vec::new();
    let mut start = 0;
    let mut quote: Option<char> = None;
    let mut separators = Vec::new();
    for (i, ch) in raw.char_indices() {
        if i < start {
            continue;
        }
        if let Some(open) = quote {
            if ch == closing(open) {
                quote = None;
            }
        } else if matches!(ch, '"' | '\'' | '“' | '‘') {
            quote = Some(ch);
        } else if ch == ','
            || raw[i..]
                .get(..5)
                .is_some_and(|suffix| suffix.eq_ignore_ascii_case(" and "))
        {
            pieces.push((start, i));
            let end = if ch == ',' { i + 1 } else { i + 5 };
            separators.push((i, end));
            start = end;
        }
    }
    if quote.is_some() {
        return None;
    }
    pieces.push((start, raw.len()));
    if pieces.len() != aliases.len() {
        return None;
    }
    let mut out = String::new();
    for (idx, (a, b)) in pieces.iter().enumerate() {
        if idx > 0 {
            out.push_str(" and ");
        }
        let _ = (a, b);
        out.push_str(&aliases[idx]);
    }
    let _ = separators;
    Some(out)
}

fn replace_span(s: &mut String, start: usize, end: usize, with: &str) {
    s.replace_range(start..end, with);
}

fn restore(
    mut parsed: Intent,
    expected: &Intent,
    bindings: &[Binding],
) -> Result<Intent, CompletionError> {
    let (
        Intent::CreateFolders {
            names: parsed_names,
            parent: parsed_parent,
        },
        Intent::CreateFolders {
            names: expected_names,
            parent: expected_parent,
        },
    ) = (&mut parsed, expected)
    else {
        return Err(CompletionError::AliasMismatch);
    };
    if parsed_names.len() != expected_names.len() || bindings.len() != expected_names.len() + 1 {
        return Err(CompletionError::AliasMismatch);
    }
    let mut restored = vec![None; parsed_names.len()];
    for binding in bindings {
        match binding.role {
            Role::Name(index) => {
                if index >= restored.len() || restored[index].is_some() {
                    return Err(CompletionError::AliasMismatch);
                }
                let positions: Vec<usize> = parsed_names
                    .iter()
                    .enumerate()
                    .filter_map(|(i, n)| n.eq_ignore_ascii_case(&binding.alias).then_some(i))
                    .collect();
                if positions.len() != 1 {
                    return Err(CompletionError::AliasMismatch);
                }
                restored[index] = Some(binding.value.clone());
            }
            Role::Parent => {
                if parsed_parent.as_deref() != Some(binding.alias.as_str())
                    || expected_parent.as_deref() != Some(binding.value.as_str())
                {
                    return Err(CompletionError::AliasMismatch);
                }
                *parsed_parent = Some(binding.value.clone());
            }
        }
    }
    // Preserve the model's ordering; exact list membership and multiplicity
    // are required, with no aliases repeated or substituted.
    let mut final_names = Vec::new();
    for alias in parsed_names.iter() {
        let binding = bindings
            .iter()
            .find(|b| matches!(b.role, Role::Name(_)) && b.alias.eq_ignore_ascii_case(alias))
            .ok_or(CompletionError::AliasMismatch)?;
        final_names.push(binding.value.clone());
    }
    if final_names.len() != expected_names.len() {
        return Err(CompletionError::AliasMismatch);
    }
    *parsed_names = final_names;
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binds_quoted_unicode_list_and_restores_exact_values() {
        let p = prepare("Create folders ‘A, B’, \"RÉSUMÉ's\" in Documents").unwrap();
        let Intent::CreateFolders {
            names: aliases,
            parent: alias_parent,
        } = fallback_guard::folder_creation(&p.bound_original).unwrap()
        else {
            panic!()
        };
        assert!(p.model_input.contains(" and "));
        assert!(!p.model_input.contains("A, B"));
        let rewrite = format!(
            "create folders named {} and {} in {}",
            aliases[0],
            aliases[1],
            alias_parent.unwrap()
        );
        let Intent::CreateFolders { names, parent } = p.finish(Ok(rewrite)).unwrap() else {
            panic!()
        };
        assert_eq!(names, vec!["A, B", "RÉSUMÉ's"]);
        assert_eq!(parent.as_deref(), Some("Documents"));
    }

    #[test]
    fn missing_extra_duplicate_and_role_swaps_fail_closed() {
        let p = prepare("Create folders \"Xylophone\", \"Quasar\" in Documents").unwrap();
        for bad in [
            "create a folder named Archive in Downloads",
            "create folders Archive and Archive in Downloads",
            "create folders Archive and Backup in Archive",
            "create folders Archive and Backup in Downloads and Projects",
        ] {
            assert!(p.finish(Ok(bad.to_string())).is_err(), "{bad}");
        }
    }

    #[test]
    fn accepted_and_hard_refusal_keep_priority_and_model_errors_propagate() {
        assert!(matches!(
            prepare("Create folders X and Y in Documents"),
            Err(PrepareError::OriginalAccepted)
        ));
        assert!(matches!(
            prepare("delete everything permanently"),
            Err(PrepareError::OriginalHardRefusal)
        ));
        let p = prepare("Create folders \"Quasar\", \"Xylophone\" in Documents").unwrap();
        assert_eq!(
            p.finish(Err("NoEos".into())),
            Err(CompletionError::ModelError("NoEos".into()))
        );
    }

    #[test]
    fn clauses_counts_paths_and_unresolved_references_never_become_slots() {
        for command in [
            "Unzip “backup.zip” into Documents/backup",
            "Create folders \"Atlas\", \"Birch\" in Documents every week",
            "Create folders \"Atlas\", \"Birch\" in Documents and delete the originals",
            "Create some folders in Documents",
            "Create folders \"Atlas/2026\", \"Birch\" in Documents",
            "Create folders \"Atlas\", \"Birch\" in there",
        ] {
            assert!(prepare(command).is_err(), "{command}");
        }
    }

    #[test]
    fn quoted_reserved_words_are_literal_values_and_aliases_do_not_collide() {
        let p = prepare("Create folders \"some\", \"two\" in \"Archive\"").unwrap();
        let aliases = fallback_guard::folder_creation(&p.bound_original).unwrap();
        let Intent::CreateFolders {
            names: alias_names,
            parent: alias_parent,
        } = aliases
        else {
            panic!()
        };
        assert_ne!(alias_names[0], alias_names[1]);
        assert!(!alias_names.contains(&alias_parent.unwrap()));
    }

    #[test]
    fn malformed_or_unsupported_list_separators_refuse_without_panicking() {
        for names in [
            "\"A\", and \"B\"",
            "\"A\",  and \"B\"",
            "\"A\",, \"B\"",
            "\"A\" and and \"B\"",
            "\"A\",",
            ", \"B\"",
        ] {
            let original = format!("Create folders {names} in Documents");
            assert!(
                std::panic::catch_unwind(|| prepare(&original)).is_ok(),
                "{original}"
            );
        }
    }

    #[test]
    fn polite_source_wording_is_preserved_while_literals_are_bound() {
        let prepared =
            prepare("Please create folders \"Quasar\", \"Zephyr\" in Documents").unwrap();
        assert!(prepared.model_input.starts_with("Please create folders "));
        assert!(
            prepared
                .finish(Ok(
                    "create folders named Archive and Backup in Desktop".into()
                ))
                .is_ok()
        );
        assert!(
            prepare("Please do not create folders \"Quasar\", \"Zephyr\" in Documents").is_err()
        );
    }

    #[test]
    fn native_q8_development_rewrites_recover_the_three_literal_list_cases() {
        for (original, model_input, rewrite, names, parent) in [
            (
                "Create folders “Atlas”, “Birch” in Documents",
                "Create folders Archive and Backup in Desktop",
                "create folders named Archive and Backup in Desktop",
                vec!["Atlas", "Birch"],
                "Documents",
            ),
            (
                "Create folders “Delta”, “Echo” in Desktop",
                "Create folders Archive and Backup in Documents",
                "create folders named Archive and Backup in Documents",
                vec!["Delta", "Echo"],
                "Desktop",
            ),
            (
                "Create folders “Fable”, “Grove” in Documents",
                "Create folders Archive and Backup in Desktop",
                "create folders named Archive and Backup in Desktop",
                vec!["Fable", "Grove"],
                "Documents",
            ),
        ] {
            let prepared = prepare(original).unwrap();
            assert_eq!(prepared.model_input, model_input);
            let Intent::CreateFolders {
                names: got_names,
                parent: got_parent,
            } = prepared.finish(Ok(rewrite.to_string())).unwrap()
            else {
                panic!()
            };
            assert_eq!(got_names, names);
            assert_eq!(got_parent.as_deref(), Some(parent));
        }
    }

    #[test]
    fn oversized_input_is_refused_before_preparation() {
        let base = "Create folders \"A\", \"B\" in ";
        let mut at_limit = base.to_string();
        at_limit.push_str(&"x".repeat(16_384 - base.len()));
        assert_eq!(at_limit.len(), 16_384);
        assert!(prepare(&at_limit).is_ok());
        let too_long = format!("{at_limit}x");
        assert_eq!(too_long.len(), 16_385);
        assert!(matches!(prepare(&too_long), Err(PrepareError::UnsupportedOriginal)));
    }
}
