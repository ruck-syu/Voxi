//! Conservative semantic certificate for command-rewrite model fallback.
//!
//! This module is deliberately independent of Tauri. Call [`certify`] only
//! after inference; it reparses both strings and returns the certified parsed
//! candidate. The caller still owns scheduling, planning, previews and user
//! confirmation. No model output is executable by this API.

use crate::model::{FileType, Intent, Source};
use crate::parse::{self, ParseFailure};
use regex::Regex;
use std::sync::LazyLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rejection {
    OriginalAccepted,
    OriginalHardRefusal,
    UnsupportedOriginalSemantics,
    CandidateRefusal,
    UnsupportedCandidateSemantics,
    MeaningChanged,
}

/// The only approved candidate intents. The returned intent must still go
/// through the normal scheduler and planner. Exact supported input domains are
/// folder creation, explicit pixel-width image resizing, and explicit bounded
/// size-range moves/copies.
pub fn certify(original: &str, candidate: &str) -> Result<Intent, Rejection> {
    match parse::parse_classified(original) {
        Ok(_) => return Err(Rejection::OriginalAccepted),
        Err(ParseFailure::HardRefusal(_)) => return Err(Rejection::OriginalHardRefusal),
        Err(ParseFailure::GrammarMiss) => {}
    };
    certify_bound(original, candidate)
}

/// Complete a request already prepared by the host from an original grammar
/// miss. Routing priority was checked before preparation; all remaining
/// semantic certificates are unchanged.
pub(crate) fn certify_bound(original: &str, candidate: &str) -> Result<Intent, Rejection> {
    if has_unquoted_unsupported_clause(original) {
        return Err(Rejection::UnsupportedOriginalSemantics);
    }
    let parsed = parse::parse_classified(candidate).map_err(|_| Rejection::CandidateRefusal)?;
    if has_unquoted_unsupported_clause(candidate) {
        return Err(Rejection::UnsupportedCandidateSemantics);
    }
    let contract = folder_creation(original).or_else(|| resize(original)).or_else(|| bounded_transfer(original));
    let Some(contract) = contract else {
        return Err(Rejection::UnsupportedOriginalSemantics);
    };
    if contract != parsed {
        return Err(Rejection::MeaningChanged);
    }
    Ok(parsed)
}

static BOUNDED_TRANSFER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^\s*(move|copy)\s+files\s+from\s+(.+?)\s+to\s+(.+?)\s+between\s+(\d+(?:\.\d+)?)\s*(?:MB)?\s+and\s+(\d+(?:\.\d+)?)\s*MB\s*[.!?]?\s*$").unwrap()
});

/// Certify only a directly stated transfer of files from one named folder to
/// another with an explicit upper MB unit. The documented "between 10 and 50 MB"
/// shorthand applies that shared unit to both endpoints; the lower MB may repeat.
/// Compare the existing strict size bounds and the independently extracted roles.
fn bounded_transfer(text: &str) -> Option<Intent> {
    let caps = BOUNDED_TRANSFER.captures(text)?;
    let operation = caps.get(1)?.as_str().to_ascii_lowercase();
    let source_name = literal(caps.get(2)?.as_str())?;
    let destination = literal(caps.get(3)?.as_str())?;
    if [source_name.as_str(), destination.as_str()].iter().any(|name| name.contains('/') || name.contains('\\')) {
        return None;
    }
    let lower = caps.get(4)?.as_str().parse::<f64>().ok()?;
    let upper = caps.get(5)?.as_str().parse::<f64>().ok()?;
    if !lower.is_finite() || !upper.is_finite() || lower < 0.0 || upper <= lower {
        return None;
    }
    let src = Source::Set {
        folder: Some(source_name),
        filter: crate::model::Filter {
            file_type: Some(FileType::Files),
            larger_than_mb: Some(lower),
            smaller_than_mb: Some(upper),
            ..Default::default()
        },
    };
    Some(if operation == "move" { Intent::Move { src, to: destination } } else { Intent::Copy { src, to: destination } })
}

static CREATE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
    r"(?i)^\s*(?:(?:please\s+)|(?:(?:could|would|can)\s+you\s+))?(?:make|create)\s+(?:(?:(?:a|the)\s+)?folders?\s+(?:(?:named|called)\s+)?(.+?)|(.+?)\s+folders?)\s+(?:in|on)\s+(.+?)\s*$"
).unwrap()
});

pub(crate) fn folder_creation(text: &str) -> Option<Intent> {
    let caps = CREATE.captures(text)?;
    let alt_two = caps.get(1).is_none();
    let name_text = caps.get(1).or_else(|| caps.get(2))?.as_str().trim();
    let parent_text = caps.get(3)?.as_str().trim();
    // The second alternative accepts arbitrary prose before "folder". An
    // article there ("a folder", "the folder") requests one folder and is not
    // a literal name; quoted names keep their exact spelling.
    if alt_two && !starts_with_quote(name_text) && starts_with_article(name_text) {
        return None;
    }
    let names = split_names(name_text)?;
    // Use the same destination policy as the main parser so that unquoted
    // parents such as "folder" or "latest" never become created folder names.
    if parse::phrase::bad_destination(parent_text) {
        return None;
    }
    let parent = literal(parent_text)?;
    if names.iter().any(|n| n.is_empty() || n.contains('/') || n.contains('\\')) {
        return None;
    }
    Some(Intent::CreateFolders { names, parent: Some(parent) })
}

fn starts_with_quote(value: &str) -> bool {
    matches!(value.chars().next(), Some('"' | '\'' | '“' | '‘'))
}

fn starts_with_article(value: &str) -> bool {
    value.split_whitespace().next().is_some_and(|word| matches!(word.to_ascii_lowercase().as_str(), "a" | "an" | "the"))
}

fn split_names(value: &str) -> Option<Vec<String>> {
    // Require explicit conjunctions: do not split commas or infer names from
    // arbitrary prose. Quoted values may contain the word "and".
    let mut spans = Vec::new();
    let mut start = 0;
    let mut quote = None;
    let chars: Vec<(usize, char)> = value.char_indices().collect();
    let mut i = 0;
    while i < chars.len() {
        let (byte, ch) = chars[i];
        if byte < start {
            i += 1;
            continue;
        }
        if let Some(q) = quote {
            if ch
                == match q {
                    '“' => '”',
                    '‘' => '’',
                    other => other,
                }
            {
                quote = None;
            }
        } else if ch == '"' || ch == '\'' || ch == '“' || ch == '‘' {
            quote = Some(ch);
        } else if value[byte..].get(..5).is_some_and(|suffix| suffix.eq_ignore_ascii_case(" and ")) {
            spans.push(&value[start..byte]);
            start = byte + 5;
        } else if ch == ',' {
            // A comma is a name separator only when every member is an explicit
            // quoted literal. This prevents prose from becoming a name list.
            let before = value[start..byte].trim();
            if !is_quoted_literal(before) {
                return None;
            }
            let mut end = byte + ch.len_utf8();
            if value[end..].starts_with(' ') {
                end += 1;
            }
            spans.push(&value[start..byte]);
            start = end;
        }
        i += 1;
    }
    if quote.is_some() {
        return None;
    }
    spans.push(&value[start..]);
    let result: Vec<String> = spans.into_iter().map(literal).collect::<Option<_>>()?;
    (!result.is_empty()).then_some(result)
}

fn literal(value: &str) -> Option<String> {
    let v = value.trim();
    if v.is_empty() || v.chars().any(char::is_control) {
        return None;
    }
    if v.starts_with('"') || v.starts_with('\'') || v.starts_with('“') || v.starts_with('‘') {
        let delimiter = v.chars().next()?;
        let closing = match delimiter {
            '“' => '”',
            '‘' => '’',
            other => other,
        };
        if v.len() < 2 || !v.ends_with(closing) {
            return None;
        }
        let inner = &v[delimiter.len_utf8()..v.len() - closing.len_utf8()];
        if inner.is_empty() || inner.contains(delimiter) || (closing != delimiter && inner.contains(closing)) {
            return None;
        }
        return Some(inner.to_string());
    }
    // Articles, quantifiers, vague words and bare nouns are not literal
    // values. This mirrors the parser's vague/NOT_A_NAME vocabulary where it
    // is exposed; the same spellings remain valid when explicitly quoted.
    let lower = v.to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "a" | "an"
            | "the"
            | "another"
            | "more"
            | "many"
            | "few"
            | "some"
            | "other"
            | "others"
            | "couple"
            | "rest"
            | "stuff"
            | "extra"
            | "latest"
            | "recent"
            | "junk"
            | "file"
            | "files"
            | "folder"
            | "folders"
            | "without"
            | "never"
            | "also"
            | "new"
            | "it"
            | "them"
            | "this"
            | "that"
            | "these"
            | "those"
            | "not"
            | "no"
            | "none"
            | "here"
            | "there"
            | "several"
            | "multiple"
            | "any"
            | "all"
            | "everything"
            | "one"
            | "two"
            | "three"
            | "four"
            | "five"
            | "six"
            | "seven"
            | "eight"
            | "nine"
            | "ten"
            | "first"
            | "last"
            | "next"
            | "previous"
    ) || v.chars().all(|ch| ch.is_ascii_digit())
    {
        return None;
    }
    // Ambiguous unquoted prose cannot be silently treated as a literal name.
    // Do not strip punctuation from values to force agreement with a rewrite.
    if !v.chars().all(|ch| ch.is_alphanumeric() || "_-./~".contains(ch)) || matches!(v, "." | "..") {
        return None;
    }
    Some(v.to_string())
}

fn is_quoted_literal(value: &str) -> bool {
    literal(value).is_some_and(|_| value.trim().chars().next().is_some_and(|c| matches!(c, '"' | '\'' | '“' | '‘')))
}

static RESIZE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
    r"(?i)^\s*(?:(?:resize|scale|shrink)\s+(.+?)\s+to\s+(\d+)\s*(?:px|pixels?)\s*(?:wide|width|across|in width)?|make\s+(.+?)\s+(\d+)\s*(?:px|pixels?)\s+wide)\s*[.!?]?\s*$"
).unwrap()
});

fn resize(text: &str) -> Option<Intent> {
    let caps = RESIZE.captures(text)?;
    let subject = caps.get(1).or_else(|| caps.get(3))?.as_str().trim();
    let width = caps.get(2).or_else(|| caps.get(4))?.as_str().parse::<u32>().ok().filter(|w| *w > 0)?;
    let exact_subject = literal(subject)?;
    let src = if subject.starts_with('"') || subject.starts_with('\'') {
        Source::item(&exact_subject)
    } else {
        let source = parse::phrase::parse_source(subject)?;
        match &source {
            Source::Item { name, folder: None } if name == &exact_subject => {}
            Source::Set { filter, .. } if *filter == (crate::model::Filter { file_type: filter.file_type, ..Default::default() }) => {}
            _ => return None,
        }
        source
    };
    Some(Intent::ResizeImage { src: image_source(src)?, width })
}

fn image_source(source: Source) -> Option<Source> {
    match source {
        Source::Item { .. } => Some(source),
        Source::Set { folder, mut filter } => {
            if filter.file_type.is_some() && filter.file_type != Some(FileType::Images) {
                return None;
            }
            filter.file_type = Some(FileType::Images);
            Some(Source::Set { folder, filter })
        }
    }
}

/// Strip quoted literals before looking for unsupported action/schedule
/// language. This catches clauses the model might erase, including S029 where
/// both extracted scheduler values were null.
pub(crate) fn has_unquoted_unsupported_clause(text: &str) -> bool {
    let mut clean = String::with_capacity(text.len());
    let mut quote = None;
    for ch in text.chars() {
        if let Some(q) = quote {
            if ch == q {
                quote = None;
            }
            clean.push(' ');
        } else if ch == '"' || ch == '\'' {
            quote = Some(ch);
            clean.push(' ');
        } else {
            clean.push(ch.to_ascii_lowercase());
        }
    }
    static UNSUPPORTED: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
        r"\b(?:every|daily|weekly|monthly|tomorrow|tonight|at\s+\d|and\s+then|then\s+(?:convert|resize|sharpen|delete|move|copy|rename)|overwrite|replace\s+(?:the\s+)?original|replace\s+it|sharpen(?:ing)?|encrypt(?:ion)?|excluding|except|unless|without\s+the\s+original|quality|percent|percentage|\d+\s*%)\b"
    ).unwrap()
    });
    quote.is_some()
        || UNSUPPORTED.is_match(&clean)
        || clean.contains(" and ") && !has_name_list_separator(text) && BOUNDED_TRANSFER.captures(text).is_none()
}

/// Whether the " and " in a command separates folder names inside a
/// creation name group, rather than starting a second action or clause.
/// Checking the name group (instead of the substring "folders") treats
/// singular "folder" and plural "folders" lists consistently.
fn has_name_list_separator(text: &str) -> bool {
    let Some(caps) = CREATE.captures(text) else {
        return false;
    };
    let Some(group) = caps.get(1).or_else(|| caps.get(2)) else {
        return false;
    };
    split_names(group.as_str()).is_some_and(|names| names.len() > 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_folder_paraphrase_certifies_exact_names_and_parent() {
        let candidate = "create folders named Archive and Backup in Desktop";
        let actual = certify("Make Archive and Backup folders on Desktop", candidate).unwrap();
        assert_eq!(actual, Intent::CreateFolders { names: vec!["Archive".into(), "Backup".into()], parent: Some("Desktop".into()) });
    }

    #[test]
    fn resize_requires_same_actual_pixel_width_and_source() {
        assert!(certify("Make photo.jpg 1200 pixels wide", "resize photo.jpg to 1200 px").is_ok());
        assert_eq!(
            certify("Resize Pictures to 50 percent quality", "resize Pictures to 50 px"),
            Err(Rejection::UnsupportedOriginalSemantics)
        );
        assert_eq!(
            certify("Resize photo.jpg to 1200 pixels wide and sharpen it", "resize photo.jpg to 1200 px"),
            Err(Rejection::UnsupportedOriginalSemantics)
        );
        assert_eq!(certify("Make photo.jpg 1200 pixels wide", "resize photo.jpg to 1024 px"), Err(Rejection::MeaningChanged));
    }

    #[test]
    fn bounded_transfers_preserve_operation_roles_and_both_size_limits() {
        let original = "move files from Downloads to Desktop between 10 and 50 MB";
        assert!(parse::parse_classified(original).is_ok());
        assert_eq!(
            certify(original, "move files larger than 10 MB smaller than 50 MB in Downloads to Desktop"),
            Err(Rejection::OriginalAccepted)
        );
        assert!(bounded_transfer("move files from Downloads to Desktop between 10 and 50").is_none());
        assert_eq!(
            certify_bound(original, "move files larger than 10 MB smaller than 50 MB in Downloads to Desktop"),
            Ok(Intent::Move {
                src: Source::Set {
                    folder: Some("Downloads".into()),
                    filter: crate::model::Filter {
                        file_type: Some(FileType::Files),
                        larger_than_mb: Some(10.0),
                        smaller_than_mb: Some(50.0),
                        ..Default::default()
                    },
                },
                to: "Desktop".into(),
            })
        );
        assert!(
            certify_bound(
                "copy files from Downloads to Archive between 1.5 MB and 8 MB",
                "copy files larger than 1.5 MB smaller than 8 MB in Downloads to Archive"
            )
            .is_ok()
        );
        assert!(
            certify_bound(
                "copy files from \"Folder and Then\" to \"Archive\" between 2 and 9 MB",
                "copy files larger than 2 MB smaller than 9 MB in \"Folder and Then\" to \"Archive\""
            )
            .is_ok()
        );
        for (input, output) in [
            (original, "copy files larger than 10 MB smaller than 50 MB in Downloads to Desktop"),
            (original, "move files larger than 10 MB in Downloads to Desktop"),
            (original, "move files larger than 10 MB smaller than 49 MB in Downloads to Desktop"),
            (original, "move files larger than 10 MB smaller than 50 MB in Desktop to Downloads"),
            (
                "move files from Downloads to Desktop between 50 and 10 MB",
                "move files larger than 10 MB smaller than 50 MB in Downloads to Desktop",
            ),
            (
                "move files from Downloads to Desktop between 10 and 50 MB and delete the originals",
                "move files larger than 10 MB smaller than 50 MB in Downloads to Desktop",
            ),
            (
                "move files from Downloads to Desktop between 10 and 50 MB every week",
                "move files larger than 10 MB smaller than 50 MB in Downloads to Desktop",
            ),
        ] {
            assert!(certify_bound(input, output).is_err(), "{input} => {output}");
        }
    }

    #[test]
    fn known_dangerous_additions_are_rejected() {
        for (input, output) in [
            ("Rename the file named \"Archive\" to Archive-old", "rename \"Archive\" to Archive"),
            ("Remove the word draft from file names in Documents", "delete files named \"draft\" in Documents"),
            ("Sort Downloads by type and then by month", "sort Downloads by month"),
            ("Convert photo.jpg to png and replace it", "convert \"photo.jpg\" to png"),
            ("Zip Downloads and overwrite Downloads.zip", "zip Downloads into Downloads.zip"),
            ("Unzip Backup.zip over existing files in Documents", "unzip Backup.zip into Documents"),
            ("Sort Pictures by type every month", "sort Pictures by month"),
            ("Delete that file", "delete files"),
        ] {
            assert!(certify(input, output).is_err(), "{input}");
        }
        assert_eq!(
            certify("Create a nested folder Archive/2026 in Documents", "create a folder named Archive/2026 in Documents"),
            Err(Rejection::UnsupportedOriginalSemantics)
        );
    }

    #[test]
    fn all_twelve_reviewed_additions_have_expected_certificate_outcome() {
        let rows = [
            ("Rename the file named \"Archive\" to Archive-old", "rename \"Archive\" to Archive", false),
            ("Remove the word draft from file names in Documents", "delete files named \"draft\" in Documents", false),
            ("Make Archive and Backup folders on Desktop", "create folders named Archive and Backup in Desktop", true),
            ("Create a nested folder Archive/2026 in Documents", "create a folder named Archive/2026 in Documents", false),
            ("Sort Downloads by type and then by month", "sort Downloads by month", false),
            ("Resize Pictures to 50 percent quality", "resize Pictures to 50 px", false),
            ("Convert photo.jpg to png and replace it", "convert \"photo.jpg\" to png", false),
            ("Zip Downloads and overwrite Downloads.zip", "zip Downloads into Downloads.zip", false),
            ("Unzip Backup.zip over existing files in Documents", "unzip Backup.zip into Documents", false),
            ("Resize photo.jpg to 1200 pixels wide and sharpen it", "resize \"photo.jpg\" to 1200 px", false),
            ("Sort Pictures by type every month", "sort Pictures by month", false),
            ("Delete that file", "delete files", false),
        ];
        for (input, output, accepted) in rows {
            assert_eq!(certify(input, output).is_ok(), accepted, "{input}");
        }
    }

    #[test]
    fn exact_transfer_contract_does_not_accept_extra_filter_or_ambiguous_roles() {
        for text in [
            "move files from Downloads to Desktop between 10 and 50 MB older than 3 days",
            "move files from Downloads to Desktop between 10 and 50 MB, excluding PDFs",
            "move files between 10 and 50 MB from Downloads to Desktop",
            "move files from Downloads/Archive to Desktop between 10 and 50 MB",
        ] {
            assert!(bounded_transfer(text).is_none(), "{text}");
        }
        assert!(bounded_transfer("copy files from \"Folder and Then\" to \"Archive\" between 2 and 9 MB").is_some());
    }

    #[test]
    fn literal_boundaries_do_not_swallow_extra_instructions() {
        for command in [
            "Make Archive and delete files folders on Desktop",
            "Make Archive and Backup folders on Desktop before noon",
            "Make move.pdf on Desktop",
        ] {
            assert!(folder_creation(command).is_none(), "{command}");
        }
        assert!(literal("\"").is_none());
        assert!(literal("'unclosed").is_none());
        assert_eq!(literal("Archive-old."), Some("Archive-old.".into()));
        assert!(resize("Make images older than 30 days 1200 pixels wide").is_none());
        assert_eq!(
            folder_creation("Make \"Report and Delete\" and \"Résumé.\" folders on Desktop"),
            Some(Intent::CreateFolders { names: vec!["Report and Delete".into(), "Résumé.".into()], parent: Some("Desktop".into()) })
        );
    }

    #[test]
    fn unspecified_names_and_references_are_not_literal_certificates() {
        assert!(certify("Make some folders on Desktop", "create folders named some in Desktop").is_err());
        for value in ["some", "two", "that", "there", "2"] {
            assert!(literal(value).is_none());
            assert_eq!(literal(&format!("\"{value}\"")), Some(value.to_string()));
        }
    }

    #[test]
    fn range_unit_inheritance_matches_documented_shorthand() {
        let shared = bounded_transfer("move files from Downloads to Desktop between 10 and 50 MB").unwrap();
        let repeated = bounded_transfer("move files from Downloads to Desktop between 10 MB and 50 MB").unwrap();
        assert_eq!(shared, repeated);
        assert!(bounded_transfer("move files from Downloads to Desktop between 10 and 50").is_none());
        assert!(bounded_transfer("move files from Downloads to Desktop between 10 GB and 50 MB").is_none());
        assert!(bounded_transfer("move files from Downloads to Desktop between 10 MB and 50 GB").is_none());
    }

    #[test]
    fn routing_preserves_parser_priority() {
        assert_eq!(certify("move report.pdf to Documents", "move report.pdf to Documents"), Err(Rejection::OriginalAccepted));
        assert_eq!(certify("delete everything permanently", "delete files"), Err(Rejection::OriginalHardRefusal));
    }

    #[test]
    fn articles_and_vague_words_are_not_unquoted_folder_names() {
        for reserved in [
            "a", "an", "the", "another", "more", "many", "few", "some", "other", "others", "couple", "rest", "stuff", "extra", "latest",
            "recent", "junk", "file", "files", "folder", "folders", "without", "never", "also",
        ] {
            let command = format!("create folder named {reserved} in Documents");
            assert!(folder_creation(&command).is_none(), "{command}");
            let quoted = format!("create folder named \"{reserved}\" in Documents");
            assert!(folder_creation(&quoted).is_some(), "{quoted}");
        }
        assert!(folder_creation("Make the Archive folder on Desktop").is_none());
        assert!(certify("Create a folder in Documents", "create folder named a in Documents").is_err());
    }

    #[test]
    fn bad_destinations_are_not_certified_as_parents() {
        for parent in ["folder", "latest", "newest", "recently", "duplicates", "into", "size", "before"] {
            let command = format!("create folder named Archive in {parent}");
            assert!(folder_creation(&command).is_none(), "{command}");
        }
        assert!(folder_creation("create folder named Archive in Documents").is_some());
    }

    #[test]
    fn singular_folder_with_a_name_list_is_certified_consistently() {
        let original = "Make \"Atlas\" and \"Birch\" folder in Desktop";
        assert!(matches!(parse::parse_classified(original), Err(ParseFailure::GrammarMiss)));
        assert_eq!(
            certify(original, "create folder named Atlas and Birch in Desktop").unwrap(),
            Intent::CreateFolders { names: vec!["Atlas".into(), "Birch".into()], parent: Some("Desktop".into()) }
        );
        assert!(certify("Make \"Atlas\" and \"Birch\" folders in Desktop", "create folders named Atlas and Birch in Desktop").is_ok());
        assert!(has_unquoted_unsupported_clause("Make \"Atlas\" and \"Birch\" folder in Desktop and then delete files"));
        assert_eq!(
            certify_bound(
                "Make \"Atlas\" and \"Birch\" folder in Desktop and then delete files",
                "create folder named Atlas and Birch in Desktop"
            ),
            Err(Rejection::UnsupportedOriginalSemantics)
        );
    }

    #[test]
    fn long_and_separated_name_lists_parse_correctly() {
        let names: Vec<String> = (0..500).map(|i| format!("Folder{i}")).collect();
        let text = format!("create folders named {} in Documents", names.join(" and "));
        let Intent::CreateFolders { names: got, parent } = folder_creation(&text).unwrap() else { panic!() };
        assert_eq!(got, names);
        assert_eq!(parent.as_deref(), Some("Documents"));
    }
}
