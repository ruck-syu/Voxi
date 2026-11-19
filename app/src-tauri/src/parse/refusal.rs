//! Quote-aware refusal policy shared by normal parsing and model eligibility.
//! These are known hard refusals, not a complete natural-language classifier.
use super::{RefusalReason, phrase, rewrite};
use regex::Regex;
use std::sync::LazyLock;

static UNSUPPORTED_ACTION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
    r"(?i)^(?:upload(?:ing)?|download(?:ing)?|e-?mail(?:ing)?|mail(?:ing)?|encrypt(?:ing)?|decrypt(?:ing)?|chmod|chown|share|print|execute|run|edit|translate|summari[sz]e|crop|rotate|blur)\s+|^(?:search|look|browse)\s+(?:the\s+)?web\b|^replace\s+.+\s+with\s+.+\s+in\s+(?:the\s+)?(?:file\s+)?contents?\b"
).unwrap()
});
static PASSIVE_ACTION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\bbe\s+(?:uploaded|downloaded|e-?mailed|encrypted|decrypted|printed|translated|cropped|blurred)(?:\s|$)").unwrap()
});
static TRANSPORT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
    r"(?i)(?:\b(?:via|using|over|by)\s+(?:e-?mail|airdrop|bluetooth|whatsapp|slack)(?:\s|$)|\b[A-Z0-9._%+\-]+@[A-Z0-9.\-]+\.[A-Z]{2,}\b|\b(?:send|email|mail)\b[^.?!]{0,80}\b(?:to|at)\s+\S+@\S+)"
).unwrap()
});
/// A destination that names a person or device is a delivery request, not a
/// local folder. Quoted values are masked out by `outside` before this runs.
static DELIVERY_TARGET: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
    r"(?i)\bto\s+(?:(?:my|the|a|an|their|his|her)\s+)?(?:phone|mobile|cell|mom|mum|mother|dad|father|parent|friend|accountant|colleague|inbox|e-?mail|mail)\b"
).unwrap()
});
static PERMISSIONS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
    r"(?i)^(?:(?:make|set|mark)\s+.+\s+(?:read[-\s]only|write[-\s]protected|executable)(?:\s|$)|change\s+(?:the\s+)?permissions(?:\s|$))"
).unwrap()
});
static FOLDER_LABEL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^make\s+(?:(?:a|new)\s+)*folders?\s+(?:named|called)\s+").unwrap());
static EXCLUSION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(?:^|[\s,;])(?:exclude|excluding|except(?:\s+for)?|apart\s+from|other\s+than|but\s+not)\s+").unwrap()
});
static SECOND_ACTION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
    r"(?i)(?:\b(?:and|then|also)\s+|;\s*)(?:(?:then|also)\s+)?(?:delete|trash|remove|copy|move|send|rename|sort|organi[sz]e|encrypt|decrypt|upload|e-?mail|open|zip|unzip|resize|convert|merge|split|extract|crop|rotate|undo|empty|emty)(?:\s+|$)"
).unwrap()
});
/// A destructive command aimed at a superlative or vague reference is an
/// unresolved reference, not an ordinary grammar miss: a model fallback must
/// never be allowed to guess which file "the largest screenshot" means.
static VAGUE_DESTRUCTIVE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
    r"(?i)^(?:delete|trash|remove|erase|wipe|shred|clean\s+up)\b.*\b(?:largest|biggest|smallest|newest|latest|oldest|junk|important|rubbish|unimportant|unneeded|unwanted|unnecessary)\b"
).unwrap()
});
static BYPASS_CONFIRMATION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\b(?:without\s+(?:confirmation|confirming|asking)|(?:don't|do\s+not)\s+(?:ask|confirm))\b").unwrap());
static LEADING_LOCATION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?i)^(?:in|inside|within|under)\s+(?:"[^"]+"|'[^']+'|\S+)\s+(.+)$"#).unwrap());
static VAGUE_FOLDER_REQUEST: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^(?:make|create|add)\s+(?:a\s+)?(?:new\s+)?folder\s+somewhere\s+for\s+these\s+files$").unwrap());

pub(super) fn reason(normalized: &str) -> Option<RefusalReason> {
    if normalized.trim().is_empty() {
        return Some(RefusalReason::MalformedInput);
    }
    let Some(outside) = phrase::unquoted_text(normalized) else {
        return Some(RefusalReason::MalformedInput);
    };
    // Mask values after stripping openers: removing them first could turn
    // "copy 'would you mind ...'" into an apparent leading instruction.
    let stripped = rewrite::strip_fillers(normalized, false);
    let mut body = stripped.as_str();
    while let Some(caps) = LEADING_LOCATION.captures(body) {
        body = caps.get(1).unwrap().as_str();
    }
    let Some(command) = phrase::unquoted_text(body) else {
        return Some(RefusalReason::MalformedInput);
    };
    if UNSUPPORTED_ACTION.is_match(command.trim_start())
        || PASSIVE_ACTION.is_match(&command)
        || TRANSPORT.is_match(&outside)
        || DELIVERY_TARGET.is_match(&outside)
        || (PERMISSIONS.is_match(command.trim_start()) && !FOLDER_LABEL.is_match(command.trim_start()))
    {
        return Some(RefusalReason::UnsupportedAction);
    }
    if VAGUE_FOLDER_REQUEST.is_match(command.trim()) {
        return Some(RefusalReason::InvalidConstraints);
    }
    if EXCLUSION.is_match(&outside) {
        return Some(RefusalReason::UnsupportedFilter);
    }
    if SECOND_ACTION.is_match(&outside) {
        return Some(RefusalReason::MultipleActions);
    }
    if BYPASS_CONFIRMATION.is_match(&outside) {
        return Some(RefusalReason::RuleRestriction);
    }
    if rewrite::refused(normalized) || rewrite::refused(body) {
        return Some(RefusalReason::RuleRestriction);
    }
    if VAGUE_DESTRUCTIVE.is_match(&rewrite::without_filenames(&outside)) {
        return Some(RefusalReason::UnresolvedReference);
    }
    None
}

#[cfg(test)]
mod tests {
    use crate::parse::{ParseFailure, parse, parse_classified};

    #[test]
    fn hard_refusals_never_authorize_fallback() {
        for command in [
            "",
            "   ",
            "upload notes.pdf to Drive",
            "could you encrypt notes.pdf",
            "would you mind emailing notes.pdf",
            "could notes.pdf be uploaded to Drive",
            "send notes.pdf via email",
            "make notes.pdf read-only",
            "list files except drafts",
            "find files over 5 MB but exclude draft",
            "list files excluding \"draft\"",
            "list files except 'draft'",
            "move notes.pdf to Archive and rename it saved.pdf",
            "clean up Downloads and organize the files by month",
            "copy notes.pdf to Documents; resize photo.png to 800 px",
            "delete notes.pdf permanently",
            "delete notes.pdf without confirmation",
            "delete notes.pdf no confirmation",
            "zip notes.pdf as out.zip then delete notes.pdf",
            "list files larger than 10 MB and smaller than 5 MB",
            "list files from February 30 2026",
            "list files today yesterday",
            "copy \"unclosed name.pdf to Documents",
            "in Downloads upload notes.pdf to Drive",
        ] {
            let error = parse_classified(command).expect_err(command);
            assert!(matches!(error, ParseFailure::HardRefusal(_)), "{command}: {error:?}");
            assert!(!error.allows_fallback(), "{command}");
            assert_eq!(parse(command), Err(command.trim().to_string()));
        }
    }

    #[test]
    fn unfamiliar_supported_wording_remains_a_grammar_miss() {
        for command in ["Could notes.pdf be relocated into Documents", "I'd appreciate a tally of PDFs in Downloads"] {
            assert_eq!(parse_classified(command), Err(ParseFailure::GrammarMiss), "{command}");
            assert!(parse_classified(command).unwrap_err().allows_fallback());
        }
    }

    #[test]
    fn literal_names_and_local_destinations_are_not_blacklisted() {
        for command in [
            "copy \"upload instructions.txt\" to \"Drive\"",
            "move \"encrypt and delete.txt\" to Dropbox",
            "copy 'except drafts.pdf' to Documents",
            "copy King's upload.pdf to Documents",
            "copy upload.pdf to Documents",
            "move notes.pdf to Upload",
            "create a folder named \"read-only\" in Documents",
            "copy notes.pdf to \"via email\"",
            "rename notes.pdf to \"except drafts.pdf\"",
            "list files named \"exclude\" in Downloads",
            "list files 1 GB or more in Downloads",
            "list files on or before July 4 2025",
            "copy be uploaded.txt to Documents",
            "copy via email.txt to Documents",
            "make a folder named read-only",
        ] {
            assert!(parse_classified(command).is_ok(), "{command}: {:?}", parse_classified(command));
        }
    }
}
