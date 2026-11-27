//! Rule-based command parser. Each category file contributes named regex
//! patterns; they are tried in order, most specific first, and the first one
//! that matches and builds an intent wins.
//!
//! T5 ingress handoff (checks must precede trimming, allocation and regexes):
//! - `lib::plan_input`: check raw text before `scheduler::extract_schedule`,
//!   `schedule_of`, scope handling or shortcut lookup.
//! - `lib::schedule_of`: check text before its `known` probe.
//! - `lib::plan_input` / `run_recurring`: validate saved shortcut expansion
//!   before iterating its commands or passing it to `plan_shortcut`.
//! - `scope::parse_followup` / `scope::prepare`: check raw text before
//!   normalization or reference rewriting, including direct callers.
//! - `scheduler::extract` / `extract_schedule`: check raw text before quote
//!   masking or schedule regexes, including direct callers.
//! - `settings::validate_shortcuts`: check raw phrase and each raw command,
//!   reject more than eight commands and a checked sum over MAX_COMMAND_BYTES
//!   before trimming or `shortcut_key`. Repeat expansion validation when
//!   executing loaded shortcuts in both lib paths; reject the whole expansion.
//! - `settings::shortcut_key` / `find_shortcut` / `find_close_shortcut`: bound
//!   lookup text and stored phrases before normalization or edit distance.
//!
//! Remaining algorithmic debt: `rewrite::osa` allocates a full O(m*n) typo
//! distance matrix. The byte cap bounds its inputs, but does not replace a
//! length-difference precheck or a bounded/rolling-row distance implementation.

mod archives;
mod chain;
mod cleanup;
mod dates;
mod files;
mod find;
mod folders;
mod images;
mod meta;
mod pdf;
pub mod phrase;
mod refusal;
mod rename;
pub(crate) use rename::spoken_chars;
mod rewrite;

use crate::model::{FileType, Intent, Source};
use regex::{Captures, Regex};
use std::sync::LazyLock;

/// Raw UTF-8 command budget, measured before trimming or normalization.
/// Shortcut callers must also cap the sum of command bytes at this budget
/// and the expansion at eight commands before parsing any command (T5).
pub const MAX_COMMAND_BYTES: usize = 16_384;

type Build = dyn Fn(&Captures) -> Option<Result<Intent, RefusalReason>> + Send + Sync;

pub struct Pattern {
    pub re: Regex,
    build: Box<Build>,
}

pub fn pat(name: &'static str, re: &str, build: fn(&Captures) -> Option<Intent>) -> Pattern {
    checked_pat(name, re, move |caps| build(caps).map(Ok))
}

fn checked_pat(
    name: &str,
    re: &str,
    build: impl Fn(&Captures) -> Option<Result<Intent, RefusalReason>> + Send + Sync + 'static,
) -> Pattern {
    Pattern { re: Regex::new(&format!("(?i){re}")).expect(name), build: Box::new(build) }
}

static PATTERNS: LazyLock<Vec<Pattern>> = LazyLock::new(|| {
    let mut v = Vec::new();
    v.extend(meta::patterns());
    v.extend(find::specific());
    v.extend(folders::patterns());
    v.extend(pdf::patterns());
    v.extend(images::patterns());
    v.extend(archives::patterns());
    v.extend(rename::patterns());
    v.extend(cleanup::patterns());
    v.extend(files::patterns());
    v.extend(find::general());
    v.extend(meta::open());
    v
});

static LEADING_SCOPE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
    r#"(?i)^(?:in|inside|within|under)\s+("[^"]+"|'[^']+'|\S+)\s+((?:list|show|find|display|move|copy|rename|trash|delete|remove)\s+.+)$"#
).unwrap()
});

/// Deliberate refusals are terminal. Only a grammar miss may reach a model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RefusalReason {
    TooLong,
    MalformedInput,
    RuleRestriction,
    UnsupportedAction,
    UnsupportedFilter,
    MultipleActions,
    InvalidConstraints,
    InvalidFolderLocation,
    /// A clause points at "it", "the original", "the copy" and the like
    /// without a clean earlier item to bind to.
    UnresolvedReference,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseFailure {
    HardRefusal(RefusalReason),
    GrammarMiss,
}

impl ParseFailure {
    pub(crate) fn message(self) -> Option<&'static str> {
        match self {
            Self::HardRefusal(RefusalReason::TooLong) => Some("That command is too long."),
            Self::HardRefusal(RefusalReason::InvalidFolderLocation) => Some(
                "Use one folder name before “in”. Quote names that contain “in”, or use a single path such as Downloads/2026/Receipts.",
            ),
            _ => None,
        }
    }

    #[cfg(test)]
    pub fn allows_fallback(self) -> bool {
        matches!(self, Self::GrammarMiss)
    }
}

/// Reject oversized input in constant time, without slicing UTF-8, allocating,
/// normalizing, or initializing regexes. Ingress callers must use this before
/// shortcut lookup, scope rewriting, or schedule extraction as well (T5).
pub fn check_length(text: &str) -> Result<(), ParseFailure> {
    if text.len() > MAX_COMMAND_BYTES { Err(ParseFailure::HardRefusal(RefusalReason::TooLong)) } else { Ok(()) }
}

/// Parses a command. `Err` carries a specific refusal message when available
/// or the raw text for "Didn't understand".
/// A misspelled verb is corrected only when the command can't be read as
/// typed, so a real word ("take that back") is never turned into another.
pub fn parse(text: &str) -> Result<Intent, String> {
    parse_classified(text).map_err(|failure| failure.message().unwrap_or(text.trim()).to_string())
}

/// The policy-aware parser entry point for fallback callers. `parse` retains
/// the existing UI error contract; its string error must not authorize fallback.
pub fn parse_classified(text: &str) -> Result<Intent, ParseFailure> {
    parse_resolved(text).map(|(intent, _)| intent)
}

/// Returns the built-in text of a vocabulary expansion alongside its intent.
/// Callers that persist commands must keep the expansion used for the preview.
pub(crate) fn parse_resolved(text: &str) -> Result<(Intent, Option<String>), ParseFailure> {
    match parse_builtin(text) {
        Err(ParseFailure::GrammarMiss) => {
            let (intent, expansion) = crate::vocab::rewrite(&phrase::normalize(text)).ok_or(ParseFailure::GrammarMiss)?;
            Ok((intent, Some(expansion)))
        }
        result => result.map(|intent| (intent, None)),
    }
}

pub(super) fn parse_builtin(text: &str) -> Result<Intent, ParseFailure> {
    check_length(text)?;
    let cleaned = phrase::normalize(text);
    if let Some(chain) = chain::parse(&cleaned) {
        return chain;
    }
    parse_single_classified(text)
}

pub(super) fn parse_vocab_output(text: &str) -> Option<Intent> {
    match parse_builtin(text).ok()? {
        Intent::Chain { .. } => None,
        intent => Some(intent),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Subfolders {
    None,
    /// "including subfolders", "recursively", "and subfolders": valid on any set.
    Any,
    /// "in its subfolders", "and its subfolders", "in subfolders" (normalized
    /// from "in all/the subfolders"): requires an explicit named folder.
    NeedsFolder,
    /// "from subfolders", "only in subfolders", "in subfolders only": only the
    /// named folder's subfolders, not the files directly in it. Requires a
    /// named folder.
    Only,
}

/// "Including subfolders", "and its subfolders", "recursively": opts a move,
/// copy or delete into the named folder's subfolders. Read only as the last
/// words of a command; anywhere else the words are refused (`mentions_subfolders`).
static SUBFOLDERS_TAIL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)(?:\s*,)?\s+(?:(?:including|incl\.?|and|with|plus)\s+(?:all\s+|any\s+)?(?:of\s+)?(?:(?P<pronoun>its|their)\s+|the\s+)?sub-?folders(?:\s+too)?|(?P<in_branch>in\s+(?:all\s+(?:of\s+)?(?:its\s+|their\s+|the\s+)?|(?:its|their|the)\s+)?sub-?folders(?:\s+too)?)|(?P<only_branch>(?:from|only\s+(?:in|from))\s+(?:its\s+|their\s+)?sub-?folders(?:\s+only)?|in\s+(?:its\s+|their\s+)?sub-?folders\s+only)|recursively)\s*$",
    )
    .unwrap()
});
static SUBFOLDER_WORD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\bsub-?folders?\b|\brecursive(?:ly)?\b").unwrap());

/// Splits a trailing subfolder opt-in off normalized text: the rest, and
/// whether (and how) it was there.
pub(crate) fn split_subfolders(normalized: &str) -> (String, Subfolders) {
    match SUBFOLDERS_TAIL.captures(normalized) {
        Some(c) => {
            let m = c.get(0).unwrap();
            let kind = if c.name("only_branch").is_some() {
                Subfolders::Only
            } else if c.name("pronoun").is_some() || c.name("in_branch").is_some() {
                Subfolders::NeedsFolder
            } else {
                Subfolders::Any
            };
            (normalized[..m.start()].to_string(), kind)
        }
        None => (normalized.to_string(), Subfolders::None),
    }
}

/// True when a command's unquoted words name subfolders or recursion, which
/// rules saved before the subfolder opt-in read differently.
pub(crate) fn names_subfolders(text: &str) -> bool {
    mentions_subfolders(&phrase::normalize(text))
}

/// True when unquoted text still names subfolders or recursion.
fn mentions_subfolders(normalized: &str) -> bool {
    phrase::unquoted_text(normalized).is_none_or(|u| SUBFOLDER_WORD.is_match(&u))
}

fn parse_single_classified(text: &str) -> Result<Intent, ParseFailure> {
    // Also bound synthesized chain clauses before their normalization.
    check_length(text)?;
    let cleaned = phrase::normalize(text);
    let (core, opt_in) = split_subfolders(&cleaned);
    if opt_in == Subfolders::None && !mentions_subfolders(&core) {
        return parse_plain(text);
    }
    // Sorting reads "including subfolders" itself.
    let original = parse_plain(text);
    if matches!(original, Ok(Intent::SortByType { .. } | Intent::SortByMonth { .. })) {
        return original;
    }
    // Anywhere but the end, the words could be part of a name ("Work with
    // Subfolders") or qualify something else ("and subfolders named draft"):
    // refuse rather than guess. Quoting a name keeps it literal.
    if mentions_subfolders(&core) {
        return Err(ParseFailure::HardRefusal(RefusalReason::UnsupportedFilter));
    }
    let mut intent = match parse_plain(&core) {
        Ok(intent) => intent,
        // A recognized modifier on an unreadable command is terminal: the
        // structured fallback can't carry it.
        Err(ParseFailure::GrammarMiss) => return Err(ParseFailure::HardRefusal(RefusalReason::UnsupportedFilter)),
        Err(e) => return Err(e),
    };
    match &mut intent {
        Intent::Move { src: Source::Set { folder, filter }, .. }
        | Intent::Copy { src: Source::Set { folder, filter }, .. }
        | Intent::Trash { src: Source::Set { folder, filter } } => {
            if matches!(opt_in, Subfolders::NeedsFolder | Subfolders::Only) && folder.is_none() {
                return Err(ParseFailure::HardRefusal(RefusalReason::UnresolvedReference));
            }
            filter.include_subfolders = true;
            filter.only_subfolders = opt_in == Subfolders::Only;
            Ok(intent)
        }
        // Lists and counts already look in subfolders.
        Intent::List { src: Source::Set { folder, .. } } | Intent::Count { src: Source::Set { folder, .. } } => {
            // Lists and counts don't separate a folder's own files from its
            // subfolders' yet: refuse rather than show both.
            if opt_in == Subfolders::Only {
                return Err(ParseFailure::HardRefusal(RefusalReason::UnsupportedFilter));
            }
            if opt_in == Subfolders::NeedsFolder && folder.is_none() {
                return Err(ParseFailure::HardRefusal(RefusalReason::UnresolvedReference));
            }
            Ok(intent)
        }
        _ => Err(ParseFailure::HardRefusal(RefusalReason::UnsupportedFilter)),
    }
}

fn parse_plain(text: &str) -> Result<Intent, ParseFailure> {
    let cleaned = phrase::normalize(text);
    if let Some(reason) = refusal::reason(&cleaned) {
        return Err(ParseFailure::HardRefusal(reason));
    }
    let as_typed = rewrite::canonical(&cleaned, false);
    if let Some(reason) = refusal::reason(&as_typed) {
        return Err(ParseFailure::HardRefusal(reason));
    }
    if let Some(intent) = parse_canonical(&as_typed)? {
        return Ok(intent);
    }
    let corrected = rewrite::canonical(&cleaned, true);
    if corrected != as_typed {
        if let Some(reason) = refusal::reason(&corrected) {
            return Err(ParseFailure::HardRefusal(reason));
        }
        if let Some(intent) = parse_canonical(&corrected)? {
            return Ok(intent);
        }
    }
    if phrase::has_filter_after_destination(&as_typed) || phrase::has_filter_after_destination(&corrected) {
        return Err(ParseFailure::HardRefusal(RefusalReason::InvalidConstraints));
    }
    if phrase::invalid_filters(&as_typed) {
        return Err(ParseFailure::HardRefusal(RefusalReason::InvalidConstraints));
    }
    Err(ParseFailure::GrammarMiss)
}

fn parse_canonical(cleaned: &str) -> Result<Option<Intent>, ParseFailure> {
    for p in PATTERNS.iter() {
        if let Some(caps) = p.re.captures(cleaned)
            && let Some(intent) = (p.build)(&caps)
        {
            return intent.map(Some).map_err(ParseFailure::HardRefusal);
        }
    }
    if let Some(c) = LEADING_SCOPE.captures(cleaned)
        && !phrase::bad_destination(&c[1])
    {
        let mut intent = match parse_builtin(&c[2]) {
            Ok(intent) => intent,
            Err(error @ ParseFailure::HardRefusal(_)) => return Err(error),
            Err(ParseFailure::GrammarMiss) => return Ok(None),
        };
        let folder = Some(phrase::clean_name(&c[1]));
        let source = match intent.source() {
            Some(Source::Item { name, folder: None }) => Some(Source::Item { name: name.clone(), folder }),
            Some(Source::Set { filter, folder: None }) => Some(Source::Set { filter: filter.clone(), folder }),
            _ => None,
        };
        if let Some(source) = source {
            intent.replace_source(source);
            return Ok(Some(intent));
        }
    }
    Ok(None)
}

/// Capture group `i` as a trimmed string, if present and non-empty.
pub(crate) fn cap(c: &Captures, i: usize) -> Option<String> {
    c.get(i).map(|m| m.as_str().trim().to_string()).filter(|s| !s.is_empty())
}

pub(crate) fn src(c: &Captures, i: usize) -> Option<Source> {
    phrase::parse_source(&cap(c, i)?)
}

/// Like `src`, but a generic "files" set is narrowed to `t`
/// ("merge everything in Downloads" means its PDFs).
pub(crate) fn typed_src(c: &Captures, i: usize, t: FileType) -> Option<Source> {
    match src(c, i)? {
        Source::Set { folder, mut filter } => {
            if matches!(filter.file_type, None | Some(FileType::Files)) && filter.ext.is_none() {
                filter.file_type = Some(t);
            }
            Some(Source::Set { folder, filter })
        }
        item => Some(item),
    }
}

#[cfg(test)]
pub(crate) mod corpus_tests;

#[cfg(test)]
mod rule_regressions;

#[cfg(test)]
mod catalog_tests {
    //! One test per row of the command catalog in BUILD.md, section 4.
    use super::parse;
    use crate::model::{DayRef, FileType::*, Filter, ImageFormat, Intent::*, Source};

    fn set(folder: Option<&str>, f: Filter) -> Source {
        Source::set(folder, f)
    }
    fn of(t: crate::model::FileType) -> Filter {
        Filter { file_type: Some(t), ..Default::default() }
    }
    fn item(n: &str) -> Source {
        Source::item(n)
    }
    fn p(s: &str) -> crate::model::Intent {
        parse(s).unwrap_or_else(|e| panic!("didn't understand: {e}"))
    }

    #[test]
    fn r01_move_file() {
        assert_eq!(p("move report.pdf to Documents"), Move { src: item("report.pdf"), to: "Documents".into() });
    }
    #[test]
    fn r02_move_screenshots_today() {
        assert_eq!(
            p("move screenshots from today to Screenshots"),
            Move { src: set(None, Filter { day: Some(DayRef::Today), ..of(Screenshots) }), to: "Screenshots".into() }
        );
    }
    #[test]
    fn r03_move_pdfs() {
        assert_eq!(p("move all PDFs from Downloads to Documents"), Move { src: set(Some("Downloads"), of(Pdfs)), to: "Documents".into() });
    }
    #[test]
    fn r04_move_large_files() {
        assert_eq!(
            p("move files larger than 100 MB from Downloads to External"),
            Move { src: set(Some("Downloads"), Filter { larger_than_mb: Some(100.0), ..of(Files) }), to: "External".into() }
        );
    }
    #[test]
    fn r05_move_containing() {
        assert_eq!(
            p("move files containing invoice from Downloads to Invoices"),
            Move { src: set(Some("Downloads"), Filter { name_contains: Some("invoice".into()), ..of(Files) }), to: "Invoices".into() }
        );
    }
    #[test]
    fn r06_copy_file() {
        assert_eq!(p("copy notes.txt to Desktop"), Copy { src: item("notes.txt"), to: "Desktop".into() });
    }
    #[test]
    fn r07_copy_images() {
        assert_eq!(p("copy all images from Pictures to Backup"), Copy { src: set(Some("Pictures"), of(Images)), to: "Backup".into() });
    }
    #[test]
    fn r08_rename() {
        assert_eq!(p("rename report.pdf to Q3-report.pdf"), Rename { src: item("report.pdf"), new_name: "Q3-report.pdf".into() });
    }
    #[test]
    fn r09_add_prefix() {
        assert_eq!(
            p("add prefix Invoice- to files named bill in Downloads"),
            AddPrefix {
                src: set(Some("Downloads"), Filter { name_contains: Some("bill".into()), ..of(Files) }),
                prefix: "Invoice-".into()
            }
        );
    }
    #[test]
    fn r10_date_prefix() {
        assert_eq!(p("add date prefix to files in Screenshots"), AddDatePrefix { src: set(Some("Screenshots"), of(Files)) });
    }
    #[test]
    fn r11_replace_spaces() {
        assert_eq!(
            p("replace spaces with dashes in files in Downloads"),
            ReplaceInNames { src: set(Some("Downloads"), of(Files)), find: " ".into(), replace: "-".into() }
        );
    }
    #[test]
    fn r12_replace_words() {
        assert_eq!(
            p("replace draft with final in names in Documents"),
            ReplaceInNames { src: set(Some("Documents"), of(Files)), find: "draft".into(), replace: "final".into() }
        );
    }
    #[test]
    fn r13_number() {
        assert_eq!(
            p("number images in Trip as trip-001, trip-002"),
            Number { src: set(Some("Trip"), of(Images)), base: "trip-".into(), width: 3 }
        );
    }
    #[test]
    fn r14_lowercase() {
        assert_eq!(p("lowercase file names in Downloads"), Lowercase { src: set(Some("Downloads"), of(Files)) });
    }
    #[test]
    fn r15_create_folder() {
        assert_eq!(
            p("create folder Invoices on Desktop"),
            CreateFolders { names: vec!["Invoices".into()], parent: Some("Desktop".into()) }
        );
    }
    #[test]
    fn create_folder_accepts_location_before_name() {
        for command in [
            "create a folder in downloads named PDFS",
            "Create a new folder inside Downloads called PDFS.",
            "make a folder on Downloads named PDFS",
            "add folder under Downloads called PDFS",
            "create a folder named PDFS in downloads",
        ] {
            let CreateFolders { names, parent: Some(parent) } = p(command) else { panic!("{command}") };
            assert_eq!(names, vec!["PDFS"], "{command}");
            assert_eq!(parent.to_lowercase(), "downloads", "{command}");
        }
        assert_eq!(
            p("create folders in Documents named Invoices and Receipts"),
            CreateFolders { names: vec!["Invoices".into(), "Receipts".into()], parent: Some("Documents".into()) }
        );
        assert_eq!(
            p("create a folder in \"Project named Alpha\" called \"Notes in Progress\""),
            CreateFolders { names: vec!["Notes in Progress".into()], parent: Some("Project named Alpha".into()) }
        );
        assert_eq!(
            p("create a folder in Downloads named \"Research and Development\""),
            CreateFolders { names: vec!["Research and Development".into()], parent: Some("Downloads".into()) }
        );
        for command in [
            "create a folder in Downloads",
            "create a folder inside Downloads",
            "create a folder under Downloads named",
            "create a folder in Downloads named \"\"",
            "create a folder in the named PDFS",
        ] {
            assert!(parse(command).is_err(), "{command}");
        }
    }
    #[test]
    fn r16_create_folders() {
        assert_eq!(
            p("create folders 2026, 2027 in Documents"),
            CreateFolders { names: vec!["2026".into(), "2027".into()], parent: Some("Documents".into()) }
        );
    }
    #[test]
    fn r17_sort_type() {
        assert_eq!(p("sort Downloads by type"), SortByType { folder: "Downloads".into(), include_subfolders: false });
    }
    #[test]
    fn r18_sort_month() {
        assert_eq!(p("sort Screenshots by month"), SortByMonth { folder: "Screenshots".into(), include_subfolders: false });
    }
    #[test]
    fn r19_convert_one() {
        assert_eq!(p("convert photo.heic to png"), ConvertImage { src: item("photo.heic"), format: ImageFormat::Png });
    }
    #[test]
    fn r20_convert_many() {
        assert_eq!(
            p("convert all HEIC in Pictures to jpg"),
            ConvertImage {
                src: set(Some("Pictures"), Filter { ext: Some(vec!["heic".into(), "heif".into()]), ..of(Files) }),
                format: ImageFormat::Jpg
            }
        );
    }
    #[test]
    fn r21_resize_one() {
        assert_eq!(p("resize photo.jpg to 1200 pixels wide"), ResizeImage { src: item("photo.jpg"), width: 1200 });
    }
    #[test]
    fn r22_resize_many() {
        assert_eq!(p("resize all images in Trip to 1600 pixels wide"), ResizeImage { src: set(Some("Trip"), of(Images)), width: 1600 });
    }
    #[test]
    fn r23_merge_named() {
        assert_eq!(
            p("merge PDFs in Documents named scan into scan-all.pdf"),
            MergePdf {
                src: set(Some("Documents"), Filter { name_contains: Some("scan".into()), ..of(Pdfs) }),
                output: "scan-all.pdf".into()
            }
        );
    }
    #[test]
    fn r24_merge_all() {
        assert_eq!(
            p("merge all PDFs in Downloads into combined.pdf"),
            MergePdf { src: set(Some("Downloads"), of(Pdfs)), output: "combined.pdf".into() }
        );
    }
    #[test]
    fn r25_split() {
        assert_eq!(p("split report.pdf into pages"), SplitPdf { src: item("report.pdf") });
    }
    #[test]
    fn r26_extract() {
        assert_eq!(
            p("extract pages 2 to 5 from report.pdf into excerpt.pdf"),
            ExtractPages { src: item("report.pdf"), from: 2, to: 5, output: Some("excerpt.pdf".into()) }
        );
    }
    #[test]
    fn r27_images_to_pdf() {
        assert_eq!(
            p("make a PDF from images in Scans named receipts"),
            ImagesToPdf { src: set(Some("Scans"), Filter { name_contains: Some("receipts".into()), ..of(Images) }), output: None }
        );
    }
    #[test]
    fn r28_zip() {
        assert_eq!(p("zip Project into Project.zip"), Zip { src: item("Project"), output: Some("Project.zip".into()) });
    }
    #[test]
    fn r29_unzip() {
        assert_eq!(
            p("unzip archive.zip into Downloads/archive"),
            Unzip { src: item("archive.zip"), destination: Some("Downloads/archive".into()) }
        );
    }
    #[test]
    fn r30_clean_up() {
        assert_eq!(
            p("clean up Downloads older than 90 days"),
            Trash { src: set(Some("Downloads"), Filter { older_than_days: Some(90), ..of(Files) }) }
        );
    }
    #[test]
    fn r31_installers_to_trash() {
        assert_eq!(p("move installers from Downloads to Trash folder"), Trash { src: set(Some("Downloads"), of(Installers)) });
    }
    #[test]
    fn r32_delete_screenshots() {
        assert_eq!(
            p("delete screenshots older than 30 days"),
            Trash { src: set(None, Filter { older_than_days: Some(30), ..of(Screenshots) }) }
        );
    }
    #[test]
    fn r33_list_large() {
        assert_eq!(
            p("list files in Downloads larger than 500 MB"),
            List { src: set(Some("Downloads"), Filter { larger_than_mb: Some(500.0), ..of(Files) }) }
        );
    }
    #[test]
    fn r34_list_old() {
        assert_eq!(
            p("list files older than 1 year in Documents"),
            List { src: set(Some("Documents"), Filter { older_than_days: Some(365), ..of(Files) }) }
        );
    }

    #[test]
    fn stl_file_types_are_extension_filters_not_name_filters() {
        let want = List { src: set(Some("Downloads"), Filter { ext: Some(vec!["stl".into()]), ..of(Files) }) };
        for command in
            [" list the STL files in Downloads. ", "list stl files in Downloads", "list .STL files in Downloads", "list STLs in Downloads"]
        {
            assert_eq!(p(command), want, "{command}");
        }
        assert_eq!(
            p("list files named STL in Downloads"),
            List { src: set(Some("Downloads"), Filter { name_contains: Some("STL".into()), ..of(Files) }) }
        );
    }
    #[test]
    fn r35_size() {
        assert_eq!(p("show size of Downloads"), Size { folder: "Downloads".into() });
    }
    #[test]
    fn r36_count() {
        assert_eq!(p("count PDFs in Documents"), Count { src: set(Some("Documents"), of(Pdfs)) });
    }
    #[test]
    fn r37_duplicates() {
        assert_eq!(p("list duplicate files in Downloads"), Duplicates { folder: "Downloads".into() });
    }
    #[test]
    fn r38_open_folder() {
        assert_eq!(p("open Downloads in Finder"), Reveal { target: "Downloads".into() });
    }
    #[test]
    fn r39_reveal_file() {
        assert_eq!(p("reveal report.pdf in Finder"), Reveal { target: "report.pdf".into() });
    }
    #[test]
    fn r40_undo() {
        assert_eq!(p("undo last"), Undo);
    }
    #[test]
    fn r41_recent() {
        assert_eq!(p("show recent actions"), RecentActions);
    }

    #[test]
    fn spoken_file_and_folder_words_are_dropped_from_names() {
        let want = Trash { src: Source::Item { name: "claude.dmg".into(), folder: Some("downloads".into()) } };
        assert_eq!(p("Delete the claude.dmg file in downloads folder."), want);
        assert_eq!(p("delete the file claude.dmg in downloads"), want);
    }

    #[test]
    fn singular_folder_names_work_like_plural() {
        assert_eq!(
            p("clean up download older than 30 days"),
            Trash { src: set(Some("download"), Filter { older_than_days: Some(30), ..of(Files) }) }
        );
    }

    #[test]
    fn who_did_it_clauses_are_ignored() {
        super::dates::TEST_TODAY.with(|t| t.set(chrono::NaiveDate::from_ymd_opt(2026, 10, 28)));
        assert_eq!(
            p("List all the screenshots I took in the last week"),
            List {
                src: set(
                    None,
                    Filter {
                        since: chrono::NaiveDate::from_ymd_opt(2026, 10, 19),
                        before: chrono::NaiveDate::from_ymd_opt(2026, 10, 26),
                        ..of(Screenshots)
                    }
                )
            }
        );
        assert_eq!(
            p("Show me the PDFs I downloaded yesterday."),
            List { src: set(None, Filter { day: Some(DayRef::Yesterday), ..of(Pdfs) }) }
        );
        assert_eq!(p("delete the installers that I've downloaded from Downloads"), Trash { src: set(Some("Downloads"), of(Installers)) });
    }

    #[test]
    fn month_filters() {
        super::dates::TEST_TODAY.with(|t| t.set(chrono::NaiveDate::from_ymd_opt(2026, 10, 28)));
        assert_eq!(
            p("Find screenshots taken in the month of august."),
            List { src: set(None, Filter { month: Some(8), ..of(Screenshots) }) }
        );
        assert_eq!(
            p("move PDFs from March 2025 to Archive"),
            Move { src: set(None, Filter { month: Some(3), year: Some(2025), ..of(Pdfs) }), to: "Archive".into() }
        );
        assert_eq!(
            p("list screenshots taken last week"),
            List {
                src: set(
                    None,
                    Filter {
                        since: chrono::NaiveDate::from_ymd_opt(2026, 10, 19),
                        before: chrono::NaiveDate::from_ymd_opt(2026, 10, 26),
                        ..of(Screenshots)
                    }
                )
            }
        );
        assert_eq!(
            p("delete installers downloaded in Sept from Downloads"),
            Trash { src: set(Some("Downloads"), Filter { month: Some(9), ..of(Installers) }) }
        );
    }

    #[test]
    fn unrecognized_input_is_not_understood() {
        for s in ["", "hello there", "make me a sandwich", "what's the weather", "move", "rename report.pdf"] {
            assert!(parse(s).is_err(), "should not understand {s:?}");
        }
    }

    #[test]
    fn relative_file_references_do_not_become_filename_searches() {
        for command in [
            "delete the last file",
            "Delete the last file.",
            "delete last files from Downloads",
            "delete the previous file in Downloads",
            "move the last file to Documents",
            "copy the last file to Desktop",
            "rename the last file to report.pdf",
            "list the last file",
            "delete the last one",
            "delete last",
        ] {
            assert!(parse(command).is_err(), "unresolved reference must not search: {command}");
        }
        super::dates::TEST_TODAY.with(|t| t.set(chrono::NaiveDate::from_ymd_opt(2026, 10, 28)));
        assert_eq!(
            p("delete files from the last week"),
            Trash {
                src: set(
                    None,
                    Filter {
                        since: chrono::NaiveDate::from_ymd_opt(2026, 10, 19),
                        before: chrono::NaiveDate::from_ymd_opt(2026, 10, 26),
                        ..of(Files)
                    }
                ),
            }
        );
        assert_eq!(p("undo last"), Undo);
        assert_eq!(p("delete last.pdf"), Trash { src: item("last.pdf") });
        assert_eq!(p("delete \"last\""), Trash { src: item("last") });
        assert_eq!(
            p("delete files named \"last\" in Downloads"),
            Trash { src: set(Some("Downloads"), Filter { name_contains: Some("last".into()), ..of(Files) }) }
        );
    }

    #[test]
    fn quoted_arguments_preserve_literal_names_and_command_words() {
        for name in ["Notes to Self.txt", "Chapter one.pdf", "first", "Month 13", "called notes", "My folder", "the report.pdf"] {
            assert_eq!(
                p(&format!("delete \"{name}\" in Downloads")),
                Trash { src: Source::Item { name: name.into(), folder: Some("Downloads".into()) } },
                "{name}"
            );
        }
        assert_eq!(p("move \"Notes to Self.txt\" to Documents"), Move { src: item("Notes to Self.txt"), to: "Documents".into() });
        assert_eq!(
            p("rename 'Notes to Self.txt' to 'Journal as of today.txt'"),
            Rename { src: item("Notes to Self.txt"), new_name: "Journal as of today.txt".into() }
        );
        assert_eq!(p("copy report.pdf to \"Terms and Conditions\""), Copy { src: item("report.pdf"), to: "Terms and Conditions".into() });
        for command in
            ["move Notes to Self.txt to Documents", "rename Notes to Self.txt to Journal.txt", "move \"unterminated to Documents"]
        {
            assert!(parse(command).is_err(), "{command}");
        }
    }

    #[test]
    fn missing_subjects_and_explicit_file_quantifiers_are_distinct() {
        for command in ["move the file to Documents", "copy file from Downloads to Desktop", "delete from Downloads"] {
            assert!(parse(command).is_err(), "{command}");
        }
        for command in ["move every file in Downloads to Documents", "move all the files from Downloads to Documents"] {
            assert_eq!(p(command), Move { src: set(Some("Downloads"), of(Files)), to: "Documents".into() });
        }
        assert_eq!(p("lowercase all the file names in Desktop"), Lowercase { src: set(Some("Desktop"), of(Files)) });
    }

    #[test]
    fn conflicting_or_repeated_filters_never_drop_constraints() {
        for command in [
            "delete files older than 30 days older than 90 days",
            "delete files larger than 1 MB larger than 5 MB",
            "list files before March 2025 before July 2026",
            "list files after March 2025 since July 2026",
            "delete files larger than 1 GB but smaller than 500 MB",
            "delete files bigger than 5 in Downloads",
            "delete files over 5 in Downloads",
            "delete files under 5 in Downloads",
            "delete files above 5 in Downloads",
            "delete files below 5 in Downloads",
            "delete files at least 5 in Downloads",
            "delete files at most 5 in Downloads",
            "delete files under 0 MB in Downloads",
        ] {
            assert!(parse(command).is_err(), "{command}");
        }
        assert_eq!(
            p("list files between 5 and 10 MB in Downloads"),
            List { src: set(Some("Downloads"), Filter { larger_than_mb: Some(5.0), smaller_than_mb: Some(10.0), ..of(Files) }) }
        );
    }

    #[test]
    fn unsupported_references_are_refused_in_read_only_commands_too() {
        for command in
            ["reveal the first file", "open the last file", "show me the second file in Finder", "what's the size of the largest PDF"]
        {
            assert!(parse(command).is_err(), "{command}");
        }
        assert_eq!(p("open \"first\""), Reveal { target: "first".into() });
        assert!(parse("undo the last delete").is_err());
        assert!(parse("undo the previous move").is_err());
        assert_eq!(p("undo the last change"), Undo);
    }

    #[test]
    fn dictated_forms_match_typed_forms() {
        assert_eq!(p("Please move the report.pdf to Documents."), p("move report.pdf to Documents"));
        assert_eq!(p("Undo that."), parse("undo last").unwrap());
        assert_eq!(p("Delete screenshots older than thirty days."), p("delete screenshots older than 30 days"));
    }

    #[test]
    fn audited_paraphrases_keep_the_same_scope_and_arguments() {
        for (variant, canonical) in [
            ("replace the word draft with final in names in Documents", "replace draft with final in files in Documents"),
            ("replace draft with final in filenames in Documents", "replace draft with final in files in Documents"),
            ("replace draft with final in the names in documents", "replace draft with final in files in documents"),
            ("replace space characters with dashes in files in Downloads", "replace spaces with dashes in files in Downloads"),
            ("lowercase filenames in Downloads", "lowercase files in Downloads"),
            ("merge all PDFs in Downloads into a file called combined.pdf", "merge PDFs in Downloads into combined.pdf"),
            ("split report.pdf into one page each", "split report.pdf into pages"),
            ("delete screenshot files older than 30 days", "delete screenshots older than 30 days"),
            ("reveal the file report.pdf in Finder", "reveal report.pdf in Finder"),
            ("show the file report.pdf in Finder", "show report.pdf in Finder"),
        ] {
            assert_eq!(p(variant), p(canonical), "{variant}");
        }
    }

    #[test]
    fn grammar_fillers_stay_literal_inside_quoted_names() {
        assert_eq!(
            p("replace \"word draft\" with final in names in Documents"),
            ReplaceInNames { src: set(Some("Documents"), of(Files)), find: "word draft".into(), replace: "final".into() }
        );
        assert_eq!(
            p("replace \"space characters\" with \"spaces\" in filenames in downloads"),
            ReplaceInNames { src: set(Some("downloads"), of(Files)), find: "space characters".into(), replace: "spaces".into() }
        );
        assert_eq!(
            p("merge PDFs in Downloads into \"a file called combined.pdf\""),
            MergePdf { src: set(Some("Downloads"), of(Pdfs)), output: "a file called combined.pdf".into() }
        );
    }

    #[test]
    fn email_delivery_never_becomes_a_local_folder_implicitly() {
        for command in
            ["send report.pdf to my email", "copy report.pdf to email", "move report.pdf to bob@example.com", "send report.pdf to my inbox"]
        {
            assert!(parse(command).is_err(), "{command}");
        }
        assert_eq!(p("move report.pdf to \"email\""), Move { src: item("report.pdf"), to: "email".into() });
        assert_eq!(p("move report.pdf to Downloads/email"), Move { src: item("report.pdf"), to: "Downloads/email".into() });
    }

    #[test]
    fn luna_literal_folder_names_are_preserved() {
        for command in
            ["create folders named \"Alpha and Beta\" in Documents/Dev", "create folders in Documents/Dev called \"Alpha and Beta\""]
        {
            assert_eq!(p(command), CreateFolders { names: vec!["Alpha and Beta".into()], parent: Some("Documents/Dev".into()) });
        }
        for name in ["Project/Child Folder", "Other Folder", "Research and Development"] {
            assert_eq!(p(&format!("move folder \"{name}\" to Desktop")), Move { src: item(name), to: "Desktop".into() });
        }
        assert_eq!(p("move Project/Child Folder to Desktop"), Move { src: item("Project/Child Folder"), to: "Desktop".into() });
        assert!(parse("create folders named \"Alpha and Beta in Documents").is_err());
    }

    #[test]
    fn luna_reordered_sources_keep_the_explicit_folder() {
        let source = Source::Item { name: "Project Plan.pdf".into(), folder: Some("Documents/Dev".into()) };
        for command in [
            "move Project Plan.pdf located in Documents/Dev to Backup",
            "move the file in Documents/Dev called Project Plan.pdf to Backup",
            "move the Documents/Dev file Project Plan.pdf to Backup",
            "in Documents/Dev move Project Plan.pdf to Backup",
            "move the file named Project Plan.pdf under Documents/Dev to Backup",
        ] {
            assert_eq!(p(command), Move { src: source.clone(), to: "Backup".into() }, "{command}");
        }
        assert!(parse("in Documents move report.pdf in Downloads to Backup").is_err());
        assert_eq!(
            p("put PDFs in Documents/Dev together as combined.pdf"),
            MergePdf { src: set(Some("Documents/Dev"), of(Pdfs)), output: "combined.pdf".into() }
        );
        assert!(parse("put report.pdf in Downloads together as something").is_err());
        assert!(parse("change photo.heic to pdf").is_err());
        assert_eq!(
            p("move Notes from Monday.pdf from Documents/Dev to Backup"),
            Move { src: Source::Item { name: "Notes from Monday.pdf".into(), folder: Some("Documents/Dev".into()) }, to: "Backup".into() }
        );
        assert_eq!(
            p("give Project Plan.pdf inside Documents/Dev the name Final Plan.pdf"),
            Rename { src: source, new_name: "Final Plan.pdf".into() }
        );
        assert_eq!(
            p("list files named \"located in\" in Downloads"),
            List { src: set(Some("Downloads"), Filter { name_contains: Some("located in".into()), ..of(Files) }) }
        );
    }
}

#[cfg(test)]
mod followup_tests {
    use super::*;

    #[test]
    fn explicit_paraphrases_preserve_the_full_intent() {
        for (said, canonical) in [
            ("Would you mind listing PDFs in Desktop?", "list PDFs in Desktop"),
            ("Would you mind undoing?", "undo"),
            ("Would you mind copying notes.txt to Archive?", "copy notes.txt to Archive"),
            ("Would you mind deleting notes.txt?", "delete notes.txt"),
            ("To Photos 2024, copy images in Downloads", "copy images in Downloads to Photos 2024"),
            ("lowercase PDF names over in Pictures", "lowercase names of PDFs in Pictures"),
            ("lowercase every file name", "lowercase names of files"),
            ("add date prefix to names of files in Downloads", "add date prefix to files in Downloads"),
            ("change menu.pdf's name to final.pdf", "rename menu.pdf to final.pdf"),
            ("combine PDFs into one file called combined.pdf", "merge PDFs into combined.pdf"),
            ("join PDFs into a single file, combined.pdf", "merge PDFs into combined.pdf"),
            ("move PDFs in Downloads if they're larger than 5 MB to Archive", "move PDFs larger than 5 MB in Downloads to Archive"),
            ("list files in Documents whose name is \"budget\"", "list files named budget in Documents"),
            ("count files in Documents have \"budget\" as name", "count files named budget in Documents"),
            ("count files have budget in title", "count files named budget"),
            ("move \"invoice\" PDFs to Archive", "move PDFs named invoice to Archive"),
            ("list out documents in Downloads", "list documents in Downloads"),
            ("count PDFs stored on Desktop", "count PDFs in Desktop"),
            ("how many PDFs there are in Desktop", "count PDFs in Desktop"),
            ("put a copy of notes into Backup folder", "copy notes to Backup"),
            ("list files starting with word invoice", "list files starting with invoice"),
            ("make a folder labeled Reports in Documents", "make a folder named Reports in Documents"),
            ("list images dated July 4 2025", "list images on July 4 2025"),
            ("split each scan PDF in Archive into pages", "split PDFs named scan in Archive into pages"),
            ("convert Downloads' scan images to jpg", "convert images named scan in Downloads to jpg"),
            ("move today's screenshots to Pictures", "move screenshots from today to Pictures"),
            ("list yesterday's screenshots", "list screenshots from yesterday"),
            ("put Invoice- in front of names of PDFs in Projects", "add prefix Invoice- to PDFs in Projects"),
            ("show history of what I did", "show history"),
            ("show everything I've done so far", "show history"),
            ("list screenshots in Downloads, under 5 MB only", "list screenshots smaller than 5 MB in Downloads"),
            ("count how many PDFs are in Downloads", "count PDFs in Downloads"),
            ("how many of documents in Desktop weigh over 100 MB", "count documents larger than 100 MB in Desktop"),
            ("number of images on Desktop from today, quick count", "count images from today in Desktop"),
            ("list videos in Downloads arrived within last 7 days", "list videos from last 7 days in Downloads"),
            ("replace spaces with underscores in PDF names inside Clients", "replace spaces with underscores in names of PDFs in Clients"),
            ("put Invoice- in front of every file's name", "add prefix Invoice- to files"),
            ("list videos called invoice are sitting on Desktop", "list videos called invoice in Desktop"),
            ("list PDFs labelled scan from School", "list PDFs named scan in School"),
            ("count documents whose names include budget inside Documents", "count documents named budget inside Documents"),
            ("pull up a list of files from last week over 10 MB", "list files from last week over 10 MB"),
            ("copy every desktop image to Archive", "copy images in desktop to Archive"),
            ("Please copy videos from Movies to Work, leaving originals alone", "copy videos from Movies to Work"),
            ("show me what I've done so far", "show history"),
            ("display log of commands I used", "show history"),
            ("make a folder named Receipts within Invoices", "make a folder named Receipts in Invoices"),
            ("add folders for Receipts and for Photos to Documents", "create folders named Receipts and Photos in Documents"),
            ("move notes.txt to Documents, would you", "move notes.txt to Documents"),
            ("Archive gets a copy of each Downloads video", "copy videos in Downloads to Archive"),
            ("Archive needs every file dated July 4 2025 or later", "move files since July 4 2025 to Archive"),
            ("count \"budget\" images lying around on Desktop, would you", "count images named budget in Desktop"),
            ("show what installers are knocking about in Downloads, would you", "list installers in Downloads"),
            ("put Tax 2025 in month order, would you", "sort Tax 2025 by month"),
            ("organise Tax 2025 month by month, grand", "sort Tax 2025 by month"),
            (
                "bin off today's screenshots in Downloads which come in above 10 MB, would you",
                "delete screenshots from today larger than 10 MB in Downloads",
            ),
            ("show us screenshots under 5 MB in Downloads, would you", "list screenshots smaller than 5 MB in Downloads"),
            ("chuck out files from last week larger than 5 MB, would you", "delete files from last week larger than 5 MB"),
            ("list screenshots in Downloads that have been around since July 4 2025", "list screenshots since July 4 2025 in Downloads"),
            ("remove files dated within 7 days that weigh over 10 MB", "delete files from last 7 days larger than 10 MB"),
            ("Desktop needs a copy of report.pdf", "copy report.pdf to Desktop"),
            ("show us which videos in Documents are larger than 100 MB", "list videos larger than 100 MB in Documents"),
        ] {
            let expected = parse(canonical).unwrap_or_else(|_| panic!("bad test expectation: {canonical}"));
            let actual = parse(said).unwrap_or_else(|_| panic!("refused: {said}"));
            assert_eq!(serde_json::to_value(actual).unwrap(), serde_json::to_value(expected).unwrap(), "{said}");
        }
    }

    #[test]
    fn unsupported_filters_and_content_edits_are_refused() {
        for command in [
            "delete files bigger than 500 MB or older than 2 years in Movies",
            "list files named budget or invoice in Documents",
            "list screenshots from other day",
            "find holiday.jpg from back in winter in Downloads",
            "find holiday.jpg dated February 30 in Downloads",
            "move photos dated 2026-04-31 to Archive",
            "move April 31 screenshots to Archive",
            "list files modified on 13/13/2025",
            "Would you mind encrypting notes.txt?",
            "Would you mind summarise notes.txt?",
            "Please remove background from photo.png",
            "remove fourth line from todo.txt",
            "delete everything important from Desktop",
            "copy rest of Downloads to Backup",
            "create folder named Reports, put it in Documents",
            "sort Documents by year and then by month",
            "move Hannah's stuff to Archive",
            "find photo.jpg from around Christmas in Pictures",
            "move every photo in Documents excluding JPGs to Archive",
            "copy documents bar spreadsheets to Backup",
            "move everything apart from work files to Backup",
            "copy scan[0-9][0-9].pdf to Backup",
            "delete Img[1-9].jpg",
            "delete for ever: notes.pdf",
            "delete everything in bin",
            "move files to Backup, leaving originals alone",
            "to Archive, transfer \"invoice\" just now, sawa",
            "count files named budget in Downloads just now, sawa",
            "Projects needs sorting by month",
            "Downloads needs a fresh Tax 2025 folder",
            "every PDF needs a date prefix",
            "Documents needs ~/Library/Keychains, relocate it",
        ] {
            assert!(parse(command).is_err(), "{command}");
        }
    }

    #[test]
    fn continuing_dates_do_not_swallow_a_folder_preposition() {
        for command in [
            "list files from August 1 2024 on",
            "list files from August 1 2024 on that are over 5 MB",
            "list files dated August 1 2024 onward",
            "list files dated August 1 2024 and after",
        ] {
            let value = serde_json::to_value(parse(command).unwrap()).unwrap();
            assert_eq!(value["src"]["filter"]["since"], "2024-08-01", "{command}");
            assert!(value["src"]["filter"]["before"].is_null(), "{command}");
        }
        let value = serde_json::to_value(parse("list files from August 1 2024 on Desktop").unwrap()).unwrap();
        assert_eq!(value["src"]["filter"]["since"], "2024-08-01");
        assert_eq!(value["src"]["filter"]["before"], "2024-08-02");
        assert_eq!(value["src"]["folder"], "Desktop");
    }

    #[test]
    fn literal_names_and_supported_bound_forms_stay_literal() {
        for command in [
            "move Screenshot 2026-11-12 at 09.40.00.png to Documents",
            "move screenshot 2026-10-01 at 9.41.12 AM.png to Pictures",
            "copy \"July 31 notes.pdf\" to Archive",
            "copy \"from around Christmas\" to Archive",
            "copy \"encrypt notes.txt\" to Archive",
            "copy \"scan[0-9].pdf\" to Archive",
            "copy Report [final].pdf to Archive",
            "copy notes.txt to \"Work, leaving originals alone\"",
            "make folders named \"Photos to Documents\"",
            "make folders named \"for Photos\" in Documents",
            "list files 1 GB or more in Downloads",
            "list files on or before July 4 2025",
            "list files from 2024 or later",
        ] {
            assert!(parse(command).is_ok(), "{command}");
        }
        let intent = parse("move Screenshot 2026-11-12 at 09.40.00.png to Documents").unwrap();
        assert_eq!(serde_json::to_value(intent).unwrap()["src"]["name"], "Screenshot 2026-11-12 at 09.40.00.png");
        let intent = parse("copy King's report.pdf to Archive").unwrap();
        assert_eq!(serde_json::to_value(intent).unwrap()["src"]["name"], "King's report.pdf");
        let intent = parse("create folders named \"Reports, put it\" in Documents").unwrap();
        assert_eq!(serde_json::to_value(intent).unwrap()["names"], serde_json::json!(["Reports, put it"]));
        let intent = parse("copy \"Would you mind listing.pdf\" to Archive").unwrap();
        assert_eq!(serde_json::to_value(intent).unwrap()["src"]["name"], "Would you mind listing.pdf");
    }
}
