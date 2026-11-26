//! Parse each clause independently; a malformed clause never becomes a partial
//! plan or a model fallback. Separators inside quoted names stay literal.
use super::{ParseFailure, RefusalReason, parse_single_classified, phrase};
use crate::model::{Destination, Intent, Source};
use regex::Regex;
use std::sync::LazyLock;

const ACTIONS: &str = "create|make|add|new|move|copy|rename|delete|trash|remove|list|show|find|count|open|undo|zip|unzip|resize|convert|merge|split|extract|sort|organize|organise|upload|send|email|encrypt|decrypt";

static SEPARATOR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"(?i)(?:\b(?:and(?:\s+(?:then|also))?|then)\s+|;\s*)(?P<verb>{ACTIONS})(?:\s+|$)")).unwrap());
/// A comma before an action verb is a chain separator, but only when the
/// clause before it is itself a command. A leading scope or filler
/// ("from Downloads, move X", "real quick, copy X") keeps its comma.
static COMMA_SEPARATOR: LazyLock<Regex> = LazyLock::new(|| Regex::new(&format!(r"(?i),\s*(?P<verb>{ACTIONS})(?:\s+|$)")).unwrap());
static CLAUSE_IS_ACTION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"(?i)^\s*(?:(?:in|inside|within|under|on|to|into)\s+\S+\s+)?(?P<verb>{ACTIONS})\s+")).unwrap());
static DESTINATION_REFERENCE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\b(?:(?:to|into)\s+(?:it|that\s+folder|(?:the\s+)?(?:new|created)\s+folder)|there)\s*$").unwrap());
/// Bind content references after creating a folder to the files it contains.
static CONTENTS_REFERENCE: LazyLock<Regex> = LazyLock::new(|| {
    let it = r"(?:it|that\s+folder|(?:the\s+)?(?:new|created)\s+folder)";
    Regex::new(&format!(
        r"(?i)^(delete|remove|trash|move|copy)\s+(?:all\s+(?:of\s+)?)?(?:its\s+contents?|its\s+files|the\s+contents?\s+of\s+{it}|everything\s+(?:in|inside)\s+{it}|(?:the\s+)?files\s+(?:in|inside)\s+{it})$"
    ))
    .unwrap()
});

fn refused() -> ParseFailure {
    ParseFailure::HardRefusal(RefusalReason::MultipleActions)
}

/// All separators in reading order. Comma separators are only accepted when
/// the clause that precedes them starts with an action verb.
fn separators(outside: &str) -> Vec<regex::Captures<'_>> {
    let mut found: Vec<(usize, bool, regex::Captures)> = Vec::new();
    for caps in SEPARATOR.captures_iter(outside) {
        found.push((caps.get(0).unwrap().start(), false, caps));
    }
    for caps in COMMA_SEPARATOR.captures_iter(outside) {
        found.push((caps.get(0).unwrap().start(), true, caps));
    }
    found.sort_by_key(|(start, _, _)| *start);
    let mut out = Vec::new();
    let mut clause_start = 0usize;
    for (_, is_comma, caps) in found {
        if is_comma && !CLAUSE_IS_ACTION.is_match(&outside[clause_start..caps.get(0).unwrap().start()]) {
            continue;
        }
        clause_start = caps.name("verb").unwrap().start();
        out.push(caps);
    }
    out
}

fn outside_syntax(text: &str) -> Option<String> {
    let outside = phrase::unquoted_text(text)?;
    // A quote mask must be a barrier, not whitespace that a separator regex
    // can absorb (e.g. `and "B" then move`). Keep byte offsets unchanged.
    let masked: Vec<u8> =
        outside.bytes().zip(text.bytes()).map(|(mask, original)| if mask == b' ' && original != b' ' { b'_' } else { mask }).collect();
    Some(String::from_utf8(masked).expect("quote mask preserves UTF-8 outside quoted spans"))
}

pub(super) fn parse(text: &str) -> Option<Result<Intent, ParseFailure>> {
    if let Err(failure) = super::check_length(text) {
        return Some(Err(failure));
    }
    let outside = outside_syntax(text)?;
    let separators = separators(&outside);
    if separators.is_empty() {
        return None;
    }
    Some((|| {
        if separators.len() >= 8 {
            return Err(refused());
        }
        let mut clauses = Vec::new();
        let mut start = 0;
        for caps in separators {
            clauses.push(text[start..caps.get(0).unwrap().start()].trim().trim_end_matches(','));
            start = caps.name("verb").unwrap().start();
        }
        clauses.push(text[start..].trim());
        let mut commands = Vec::new();
        // The most recently created folder supplies destination references.
        // Unscoped sources always keep the single-command search roots.
        let mut created: Option<String> = None;
        let mut named_folders: Vec<String> = Vec::new();
        let mut expanded = Vec::new();
        // Only expand explicit filename lists; "and" in filters or literal
        // names must never manufacture a second command.
        let file_list = Regex::new(r"(?i)^(move|copy)\s+(\S+\.[a-z0-9]+)\s+and\s+(\S+\.[a-z0-9]+)(\s+from\s+.+)$").unwrap();
        for clause in clauses {
            if let Some(c) = file_list.captures(clause) {
                expanded.push(format!("{} {}{}", &c[1], &c[2], &c[4]));
                expanded.push(format!("{} {}{}", &c[1], &c[3], &c[4]));
            } else {
                expanded.push(clause.to_string());
            }
        }
        if expanded.len() > 8 {
            return Err(refused());
        }
        let source_it = Regex::new(r"(?i)^(move|copy|rename|zip|unzip|resize|convert)\s+it(\s|$)").unwrap();
        for clause in &expanded {
            // A trailing "including subfolders" belongs to the whole clause;
            // set it aside so "into it including subfolders" still binds.
            let (core, opt_in) = super::split_subfolders(clause);
            let clause = core.as_str();
            // Scheduling applies to one command at a time. Do not let a time
            // clause on one step silently become a literal destination/name.
            let (_, when) = crate::scheduler::extract_schedule(clause, chrono::Local::now()).map_err(|_| refused())?;
            if when.is_some() {
                return Err(refused());
            }
            let outside = outside_syntax(clause).ok_or_else(refused)?;
            let reference = DESTINATION_REFERENCE.find(&outside);
            let rewritten = if let Some(reference) = &reference {
                let folder = created.as_ref().ok_or_else(refused)?;
                // Quote the bound path as a literal; reject quote-bearing
                // paths rather than changing a source literal during binding.
                if folder.contains(['"', '\\']) {
                    return Err(refused());
                }
                format!("{}to \"{folder}\"", &clause[..reference.start()])
            } else {
                clause.to_string()
            };
            let (subject, destination) = phrase::split_arguments(&rewritten, &["to", "into"])
                .map_or((rewritten.as_str(), None), |(subject, to)| (subject, Some(to)));
            let subject_syntax = outside_syntax(subject).ok_or_else(refused)?;
            let rewritten = if let Some(c) = CONTENTS_REFERENCE.captures(subject_syntax.trim())
                && (c[1].eq_ignore_ascii_case("move") || c[1].eq_ignore_ascii_case("copy")) == destination.is_some()
            {
                let folder = created.as_ref().ok_or_else(refused)?;
                if folder.contains(['"', '\\']) {
                    return Err(refused());
                }
                let bound = format!("{} everything in \"{folder}\"", &c[1]);
                match destination {
                    Some(to) => format!("{bound} to {to}"),
                    None => bound,
                }
            } else {
                rewritten
            };
            let binds_output = source_it.is_match(&outside);
            let rewritten = if binds_output {
                if commands.is_empty() {
                    return Err(refused());
                }
                source_it.replace(&rewritten, "${1} voxi-chain-output.zip${2}").into_owned()
            } else {
                rewritten
            };
            let rewritten = match opt_in {
                super::Subfolders::None => rewritten,
                super::Subfolders::Any => format!("{rewritten} including subfolders"),
                super::Subfolders::NeedsFolder => format!("{rewritten} in its subfolders"),
                super::Subfolders::Only => format!("{rewritten} from subfolders"),
            };
            let mut intent = parse_single_classified(&rewritten).map_err(|failure| match failure {
                ParseFailure::HardRefusal(reason) => ParseFailure::HardRefusal(reason),
                // An unresolved "it"/"the original"/"the copy" in a clause is
                // a reference with nothing clean to bind to, not a soft miss.
                ParseFailure::GrammarMiss => ParseFailure::HardRefusal(RefusalReason::UnresolvedReference),
            })?;
            if binds_output {
                intent.replace_source(Source::item("\0chain-output"));
            }
            // Bare destinations matching a folder created in this chain bind
            // to its full planned path, regardless of the transfer's source.
            if let Intent::Move { to, .. } | Intent::Copy { to, .. } = &mut intent
                && let Destination::Path(name) | Destination::InFolder { literal: Some(name), .. } = &*to
            {
                let matches: Vec<_> =
                    named_folders.iter().filter(|folder| name.eq_ignore_ascii_case(folder.rsplit('/').next().unwrap())).collect();
                if let Some(folder) = matches.first() {
                    if matches.iter().any(|other| !other.eq_ignore_ascii_case(folder)) {
                        return Err(refused());
                    }
                    *to = Destination::Path((*folder).clone());
                }
            }
            if reference.is_some() && !matches!(intent, Intent::Move { .. } | Intent::Copy { .. }) {
                return Err(refused());
            }
            if !matches!(
                intent,
                Intent::CreateFolders { .. }
                    | Intent::Move { .. }
                    | Intent::Copy { .. }
                    | Intent::Rename { .. }
                    | Intent::Trash { .. }
                    | Intent::Zip { .. }
                    | Intent::Unzip { .. }
                    | Intent::ConvertImage { .. }
                    | Intent::ResizeImage { .. }
            ) {
                return Err(refused());
            }
            if let Intent::CreateFolders { names, parent } = &intent {
                let base = parent.as_deref().unwrap_or("Desktop").trim_end_matches('/');
                named_folders.extend(names.iter().map(|name| format!("{base}/{name}")));
                created = if names.len() == 1 {
                    let base = parent.as_deref().unwrap_or("Desktop").trim_end_matches('/');
                    Some(format!("{base}/{}", names[0]))
                } else {
                    None
                };
            }
            if matches!(intent, Intent::ResizeImage { .. }) && !matches!(commands.last(), Some(Intent::ConvertImage { .. })) {
                return Err(refused());
            }
            // Keep the pre-existing conservative refusal for zipping and then
            // deleting the original; this is not an output reference.
            if let Intent::Trash { src } = &intent
                && commands.iter().any(|c| matches!(c, Intent::Zip { src: zipped, .. } if zipped == src))
            {
                return Err(refused());
            }
            commands.push(intent);
        }
        Ok(Intent::Chain { commands })
    })())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{model::FileType, parse::parse};

    #[test]
    fn creates_folder_and_binds_transfer_destination_and_default_source() {
        for join in ["and", "and then", "then", ";"] {
            let text = format!("create a folder in downloads called PDFS {join} move pdfs into it");
            let Intent::Chain { commands } = parse(&text).unwrap() else { panic!() };
            assert_eq!(commands.len(), 2);
            assert!(matches!(&commands[1], Intent::Move { src: Source::Set { folder: None, filter }, to }
                if filter.file_type == Some(FileType::Pdfs) && to.path() == Some("downloads/PDFS")));
        }
    }

    #[test]
    fn quoted_separators_and_explicit_sources_stay_literal() {
        let Intent::Chain { commands } =
            parse("create a folder named \"R&D and move notes\" in Documents then copy PDFs from Downloads into that folder").unwrap()
        else {
            panic!()
        };
        assert!(matches!(&commands[0], Intent::CreateFolders { names, .. } if names == &["R&D and move notes"]));
        assert!(matches!(&commands[1], Intent::Copy { src: Source::Set { folder: Some(folder), .. }, to }
            if folder == "Downloads" && to.path() == Some("Documents/R&D and move notes")));
        assert!(!matches!(parse("create folders named \"Alpha and move Beta\" in Documents").unwrap(), Intent::Chain { .. }));
        // Mixed quoted folder lists are a pre-existing grammar miss. Chain
        // splitting must preserve that refusal instead of losing a member.
        assert!(parse("create folders Alpha and \"Beta\" in Downloads then copy images from Pictures to Documents").is_err());
        for clause in ["move a.pdf from Downloads to \"Archive and copy notes\"", "copy b.txt from Desktop to Documents"] {
            assert!(parse_single_classified(clause).is_ok(), "{clause}: {:?}", parse_single_classified(clause));
        }
        assert!(parse("move a.pdf from Downloads to \"Archive and copy notes\" then copy b.txt from Desktop to Documents").is_ok());
    }

    #[test]
    fn folder_outputs_and_earlier_named_destinations_bind_without_guessing() {
        let Intent::Chain { commands } = parse("create a folder named Backup and move it to Documents").unwrap() else { panic!() };
        assert!(matches!(&commands[1], Intent::Move { src: Source::Item { name, folder: None }, .. } if name == "\0chain-output"));
        let Intent::Chain { commands } = parse("create a folder called Atlas in Documents then create a folder called Boreal in Documents then copy a.pdf from Downloads into Atlas").unwrap() else { panic!() };
        assert!(matches!(&commands[2], Intent::Copy { to, .. } if to.path() == Some("Documents/Atlas")));
        assert!(parse("create a folder called Atlas in Documents then create a folder called Atlas in Downloads then copy a.pdf from Desktop into Atlas").is_err());
        // A quoted literal named "it" must never become an output reference.
        let Intent::Chain { commands } = parse("copy a.pdf from Downloads to Desktop then zip \"it\"").unwrap() else { panic!() };
        assert!(matches!(&commands[1], Intent::Zip { src: Source::Item { name, .. }, .. } if name == "it"));
    }

    #[test]
    fn invalid_steps_and_ambiguous_references_refuse_the_whole_chain() {
        for text in [
            "create folders A and B in Downloads and move PDFs into it",
            "move PDFs from Downloads to Documents and copy images into it",
            "create folder named Archive in Downloads and upload PDFs to Drive",
            "create folder named Archive in Downloads and move PDFs except invoices into it",
            "create folder named Archive in Downloads and list PDFs",
            "create folder named Archive in Downloads and move PDFs into it at 9 pm",
            "create folder named Archive at 9 pm and move PDFs into it",
            "create folder named Archive in Downloads and undo last",
            "create folder named Archive in Downloads and move PDFs into it without confirmation",
            "create folder named Archive in Downloads and move PDFs to it \"extra clause\"",
        ] {
            assert!(matches!(crate::parse::parse_classified(text), Err(ParseFailure::HardRefusal(_))), "{text}");
        }
        let eight = std::iter::repeat_n("create folder named Archive in Downloads", 8).collect::<Vec<_>>().join(" then ");
        assert!(parse(&eight).is_ok());
        assert!(parse(&format!("{eight} then create folder named More in Downloads")).is_err());
    }
}
