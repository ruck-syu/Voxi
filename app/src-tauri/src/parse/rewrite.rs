//! Canonical forms for common rewordings, applied after `phrase::normalize`
//! and before the command patterns. Each rule turns a phrasing people use
//! ("to Desktop move X", "X should be called Y", "mvoe X") into one the
//! patterns already understand, so the patterns stay few and strict.
//!
//! Rules only reorder or swap a verb; they never invent a name, folder, or
//! number. Quoted text is left alone by the filler rules.

use regex::Regex;
use std::sync::LazyLock;

macro_rules! re {
    ($s:expr) => {
        LazyLock::new(|| Regex::new($s).unwrap())
    };
}

/// Fillers left anywhere in the text by dictation: "move the uh report.pdf".
static MID_FILLER: LazyLock<Regex> = re!(r"(?i)(?:^|\s)(?:u+h+|u+m+|e+r+m+|uhm+|hmm+)(?:,|\s|$)");
/// Fillers before the verb that `normalize` doesn't catch.
static LEAD: LazyLock<Regex> = re!(
    r"(?i)^(?:(?:so|like|just|ok|okay|real\s+quick|quickly|quick|hey|now|then|alright|right|pl[ea]+s[e]?|pl[sz]|kindly|go\s+ahead\s+and|help\s+me(?:\s+to)?|h[el]{2}p\s+me|would\s+you\s+mind|(?:can|could|would|will)\s+you(?:\s+please)?|(?:can|could)\s+i|i\s+(?:want|need|would\s+like|wanna)\s+(?:you\s+)?to|i'?d\s+like\s+(?:you\s+)?to|let's|tell\s+me|i\s+need\s+to\s+know|i\s+wonder)[,]?\s+)+"
);
/// Fillers after the command that `normalize` doesn't catch.
static TRAIL: LazyLock<Regex> = re!(
    r"(?i)(?:,?\s+(?:real\s+quick|quickly|quick|right\s+now|now|asap|if\s+you\s+can|if\s+you\s+could|okay|ok|like|thanks|thank\s+you|thx|cheers|please|plese|pleas|pl[sz]|for\s+me|for\s+us|for\s+us\s+mate|if\s+possible|when\s+you\s+can|much\s+appreciated|instead|period|full\s+stop))+$"
);
/// Regional sign-offs after a comma: ", ta", ", sweet as", ", lah". Only
/// after a comma, so a short folder name such as "Ta" is never cut off.
static SIGN_OFF: LazyLock<Regex> = re!(
    r"(?i),\s*(?:ta|cheers(?:\s+mate)?|sweet\s+as|mate|ja|eh|lah|leh|lor|na|yaar|innit|chur|bro|boss|no\s+worries|for\s+us|thanks\s+(?:a\s+lot|mate)|nice\s+one|yeah|yep|yes|ok|okay|sound|pl[sz]|please|plese|ty|thx|lol|k|kk|rn|asap|dude|buddy|sir|(?:my\s+)?friend|if\s+possible|when\s+you\s+(?:can|get\s+a\s+sec)|much\s+appreciated|(?:would|will)\s+(?:ya|you)|thanks\s+po|po|sawa|grand|anyhow|can|quick\s+count|thanks|right)\s*$"
);
static SEE: LazyLock<Regex> = re!(r"(?i)^(?:i\s+(?:need|want|wanna|would\s+like)\s+to\s+see|(?:let\s+me|lemme)\s+see|i'?d\s+like\s+to\s+see|gimme)\s+");
static SPACES: LazyLock<Regex> = re!(r"\s+");
static JUST_NOW: LazyLock<Regex> = re!(r"(?i)\bjust\s+now$");

/// Words that ask for something Voxi never does: deleting past the Trash.
/// These are *phrases*: a filename token such as "Permanent Resident Card.pdf"
/// must stay a literal name, so the caller removes filename tokens first.
static PERMANENT_PHRASE: LazyLock<Regex> = re!(r"(?i)(?:\bpermanently\b|\bfor\s+(?:good|ever|real)\b|\bforever\b|\birreversibly\b|\birrecoverably\b|\bsecurely\s+erase\b|\b(?:shred|wipe)\s+|\bpermanent\s+(?:delete|deletion|removal|erase|wipe|trash)\b|\b(?:delete|deletion|remove|removal|trash)\b[^.?!]{0,80}\bpermanent\b|\b(?:empty|emty|erase|clear(?:\s+out)?)\s+(?:the\s+|my\s+)?(?:trash|bin|recycle\s+bin)\b|\bwithout\s+(?:the\s+|using\s+(?:the\s+)?)?(?:trash|bin|recycle\s+bin)\b|\b(?:so\s+(?:it|they)\s+(?:can't|cannot|can\s+not|won't|will\s+not)\s+be\s+recovered|can't\s+be\s+recovered)\b)");
/// "erase notes.pdf" is a permanent-delete request, but "erase everything in
/// Documents" is a supported bulk Trash request (see the corpus). Only an
/// explicit named item after "erase" is refused here.
static NAMED_ERASE: LazyLock<Regex> = re!(r#"(?i)^\s*erase\s+(?:(?:the|my|these|those)\s+)?(?:\S*\.[a-z0-9]{1,8}\b|"[^"]+"|'[^']+')"#);

/// A `?` is fine inside a filename token ("What?.pdf"), but a bare `?` or a
/// leading `?` ("?.txt") is unsupported wildcard syntax.
fn unsupported_question(outside: &str) -> bool {
    outside.split_whitespace().any(|tok| {
        let core = tok.trim_matches(|c: char| matches!(c, ',' | ';' | ':' | '!' | '\'' | '"'));
        (core.starts_with('?') || !super::phrase::has_extension(core)) && tok.contains('?')
    })
}

/// Drops literal filename tokens so a permanence phrase is never found inside
/// a name such as "Permanent Resident Card.pdf" or "for real.pdf".
pub(super) fn without_filenames(outside: &str) -> String {
    outside.split_whitespace()
        .filter(|tok| {
            let core = tok.trim_matches(|c: char| matches!(c, ',' | ';' | ':' | '!' | '\'' | '"'));
            !super::phrase::has_extension(core)
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Requests that explicitly bypass the Trash or confirmation cannot be
/// honored by silently treating their extra words as a file or folder name.
static BYPASS: LazyLock<Regex> = re!(r"(?i)\b(?:no\s+(?:trash|bin|questions|confirmation)|(?:skip|bypass|avoid)\s+(?:trash|bin|confirmation)|(?:do\s+not|don't|dont)\s+(?:move|send|put)\s+(?:it\s+|them\s+)?(?:to|in|into)\s+(?:trash|bin))\b");
/// A second action is never part of an archive output name. Names containing
/// these words can still be written literally inside quotes.
static CHAINED_ACTION: LazyLock<Regex> = re!(r"(?i)\b(?:and\s+then|then|and)\s+(?:delete|trash|remove|email|mail|upload|send|move|copy|rename|sort|encrypt|open|zip|unzip)\b");

/// Removing content from a file is not deleting the file itself.
static CONTENT_EDIT: LazyLock<Regex> = re!(r"(?i)^(?:remove|delete|strip)\s+(?:background\s+from|(?:(?:first|second|third|fourth|last)|\d+(?:st|nd|rd|th)?)\s+(?:line|paragraph|word)\s+from)\b");

static KEEP_ORIGINALS: LazyLock<Regex> = re!(r"(?i)\b(?:leaving|keeping)\s+originals\s+(?:alone|intact|untouched)\b");
static COPY_COMMAND: LazyLock<Regex> = re!(r"(?i)^(?:copy|duplicate|copying)\s+");

/// Character-class ranges request pattern expansion. Quoted filenames remain
/// literal, and ordinary bracketed names such as "Report [final].pdf" are valid.
static REGEX_RANGE: LazyLock<Regex> = re!(r"\[[^\]]*(?:[0-9]-[0-9]|a-z|A-Z|\\d)[^\]]*\]");
static TRASH_CONTENTS: LazyLock<Regex> = re!(r"(?i)^(?:delete|remove|trash|bin)\s+(?:(?:everything|all\s+files|(?:all\s+)?contents)\s+(?:in|from)\s+)?(?:trash|bin)$");

static DESTINATION_NEEDS: LazyLock<Regex> = re!(r"^([A-Z][^\s,]*)\s+(?i)needs\s+(.+)$");
static NEEDS_COPY: LazyLock<Regex> = re!(r"(?i)^(?:a\s+)?cop(?:y|ies)\s+of\s+");
// These ask for unsupported selection/compression semantics. Quoted names
// remain literal; naming a file "hidden files" is not a hidden-file query.
static HIDDEN_SELECTION: LazyLock<Regex> = re!(r"(?i)\bhidden\s+(?:files|folders)\b");
static IMAGE_COMPRESSION: LazyLock<Regex> = re!(r"(?i)^compress\s+.+\s+(?:to\s+make\s+(?:them|it)\s+smaller|without\s+losing\s+quality|to\s+reduce\s+(?:their|its)\s+size)$");

/// True when the command must be refused regardless of its matching pattern.
pub fn refused(normalized: &str) -> bool {
    let Some(outside) = super::phrase::unquoted_text(normalized) else { return true };
    let stripped = strip_fillers(normalized, false);
    if let Some(c) = DESTINATION_NEEDS.captures(&stripped)
        && !NEEDS_COPY.is_match(&c[2])
        && !matches!(super::phrase::parse_source(&c[2]), Some(crate::model::Source::Set { .. }))
    {
        // "Projects needs sorting" is not a request to move an item named
        // "sorting". Only an explicit filtered set supports this shorthand.
        return true;
    }
    outside.contains('*') || unsupported_question(&outside)
        || PERMANENT_PHRASE.is_match(&without_filenames(&outside)) || NAMED_ERASE.is_match(&outside)
        || REGEX_RANGE.is_match(&outside)
        || TRASH_CONTENTS.is_match(outside.trim())
        || BYPASS.is_match(&outside)
        || CHAINED_ACTION.is_match(&outside)
        || CONTENT_EDIT.is_match(outside.trim())
        || HIDDEN_SELECTION.is_match(&outside)
        || IMAGE_COMPRESSION.is_match(strip_fillers(&outside, false).trim())
        || (KEEP_ORIGINALS.is_match(&outside) && !COPY_COMMAND.is_match(strip_fillers(normalized, false).trim()))
}

/// `(pattern, replacement)` pairs, tried in order; every one that matches is applied.
static RULES: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
    [
        // Repeating the query verb must not become a filename filter.
        (r"(?i)^count\s+how\s+many\s+", "how many "),
        (r"(?i)^how\s+many\s+of\s+", "how many "),
        // This redundant instruction is valid for copies only. A move must
        // never silently discard a request to keep the originals.
        (r"(?i)^(copy|duplicate)\s+(.+),\s*(?:leaving|keeping)\s+originals\s+(?:alone|intact|untouched)$", "$1 $2"),
        (r"(?i)^show\s+us\s+", "show "),
        (r"(?i)^(show|list)\s+which\s+", "$1 "),
        (r"(?i)^(?:bin\s+off|chuck\s+out)\s+", "delete "),
        // Explicit gerunds in polite requests, never guessed from typos.
        (r"(?i)^listing\s+", "list "),
        (r"(?i)^copying\s+", "copy "),
        (r"(?i)^moving\s+", "move "),
        (r"(?i)^renaming\s+", "rename "),
        (r"(?i)^deleting\s+", "delete "),
        (r"(?i)^undoing$", "undo"),
        (r"(?i)^list\s+out\s+", "list "),
        // "Archive should get the PDFs from Downloads" -> "move the PDFs from Downloads to Archive"
        (r"^([A-Z][^\s,]*)\s+(?i)(?:should|must|needs\s+to|has\s+to)\s+(?:get|receive|have|hold)\s+cop(?:y|ies)\s+of\s+(.+)$", "copy $2 to $1"),
        (r"^([A-Z][^\s,]*)\s+(?i)needs\s+(?:a\s+)?cop(?:y|ies)\s+of\s+(.+)$", "copy $2 to $1"),
        (r"^([A-Z][^\s,]*)\s+(?i)(?:should|must|can)\s+(?:get|receive|have|hold|take)\s+(?:whatever\s+)?(.+)$", "move $2 to $1"),

        (r"^([A-Z][^\s,]*)\s+(?i)(?:gets|receives)\s+(?:a\s+)?cop(?:y|ies)\s+of\s+(.+)$", "copy $2 to $1"),

        (r"^([A-Z][^\s,]*)\s+(?i)(?:gets|receives)\s+(.+)$", "move $2 to $1"),
        // "put a copy of each image in Movies into School" -> "copy each image in Movies to School"
        (r"(?i)^(?:put|place|stick|drop|make|save|leave)\s+(?:a\s+)?cop(?:y|ies)\s+of\s+(.+)\s+(?:into|to|inside|in)\s+(\S+(?:\s+folder)?)$", "copy $1 to $2"),
        // "move the Invoices folder over from Documents" -> "... from Documents"
        (r"(?i)\s+over\s+from\s+", " from "),
        // "the claude.dmg file that's in Downloads" -> "claude.dmg in Downloads"
        // "the zip files that are in Downloads" -> "zip files in Downloads"
        (r"(?i)\b(files|folders|[a-z]+s)\s+that\s+are\s+(in|on|inside)\s+", "$1 $2 "),
        (r"(?i)(\S+\.[a-z0-9]{1,5})\s+(?:file\s+)?that(?:'s|\s+is)\s+(in|on|inside)\s+", "$1 $2 "),
        // "split up report.pdf", "split report.pdf apart", "into each page"
        (r"(?i)^split\s+up\s+", "split "),
        (r"(?i)^split\s+(.+?)\s+apart\b", "split $1"),
        (r"(?i)\s+into\s+(?:each|every)\s+page$", " into separate pages"),
        // An explicit zip request must survive the generic output-label cleanup,
        // otherwise "put PDFs together" reaches the PDF merge shorthand.
        (r"(?i)^put\s+(.+?)\s+(?:together\s+)?(?:into|as)\s+(?:a\s+)?zip(?:\s+file)?\s+(?:called|named)\s+(.+)$", "zip $1 into $2"),
        // "compress X into a zip called Y.zip" -> "compress X into Y.zip"
        (r"(?i)\s+(?:into|as|to)\s+(?:a\s+)?zip(?:\s+file)?\s+(?:called|named)\s+", " into "),
        // "number PDFs as scan-01 and scan-02" -> "number PDFs as scan-01"
        (r"(?i)^(number\s+.+?\s+(?:as|like)\s+\S+?)(?:,\s*|\s+and\s+|\s*,\s*and\s+)\S+.*$", "$1"),
        // "find where report.pdf is" -> "show me where report.pdf is"
        (r"(?i)^find\s+where\s+(.+)$", "show me where $1"),
        // "pee dee effs": how dictation spells PDFs out.
        (r"(?i)\bpee\s+dee\s+effs?\b", "PDFs"),
        // "put Screenshots in month order" -> "sort Screenshots by month"
        (r"(?i)^(?:put|arrange|sort)\s+(?:the\s+)?(.+?)(?:\s+(?:files|folder))?\s+(?:in|into)\s+month(?:ly)?\s+order$", "sort $1 by month"),
        (r"(?i)^(?:sort|organize|organise|group)\s+(.+?)\s+month\s+by\s+month$", "sort $1 by month"),
        // "put the PDFs together into combined.pdf", "... into 1 file named combined.pdf" -> merge
        (r"(?i)^(?:put|stick|join)\s+(.+?)\s+(?:together\s+)?(?:into|as)\s+(?:(?:1|a\s+single)\s+(?:file|pdf)\s+(?:named|called)\s+)?(\S+\.pdf)$", "merge $1 into $2"),
        // "make Documents a new folder called Invoices"
        (r"(?i)^(?:make|create|give)\s+(\S+)\s+a\s+new\s+folder\s+(?:called|named)\s+(.+)$", "create a folder named $2 in $1"),
        // "move the PDFs into a new folder called Bills on the Desktop" -> "move the PDFs to Desktop/Bills".
        // Moving and copying make a missing destination, so "a new folder" needs no extra step.
        (r"(?i)^(move|copy|put|send|shift|transfer)\s+(.+?)\s+(?:to|into|in)\s+(?:a\s+)?(?:brand\s+)?(?:new\s+)?folder\s+(?:called|named)\s+(.+?)\s+(?:on|in|inside)\s+(?:the\s+|my\s+)?(\S+?)(?:\s+folder)?$", "$1 $2 to $4/$3"),
        // "move every pdf in Downloads to a new folder called PDFs" -> "move every pdf in Downloads to PDFs"
        (r"(?i)^(move|copy|put|send|shift|transfer)\s+(.+?)\s+(?:to|into|in)\s+(?:a\s+)?(?:brand\s+)?(?:new\s+)?folder\s+(?:called|named)\s+(.+)$", "$1 $2 to $3"),
        // "make a zip archive from Project called Project.zip" -> "zip Project into Project.zip"
        (r"(?i)^(?:make|create)\s+(?:a\s+)?zip(?:\s+archive|\s+file)?\s+(?:from|of|out\s+of)\s+(.+?)\s+(?:called|named)\s+(\S+\.zip)$", "zip $1 into $2"),
        // Deleting, said other ways. Everything still goes to the Trash with a preview.
        // "chuck a new Invoices folder in Documents": a new folder is made, never binned.
        (r"(?i)^(?:toss|chuck|throw|stick|pop|bung)\s+(?:a\s+)?new\s+(.+?)\s+folder\s+(?:in|into|on|inside)\s+(.+)$", "create a folder named $1 in $2"),
        // "chuck X in the bin", "send X to the trash can": the Trash, said casually.
        (r"(?i)^(?:toss|chuck|throw|dump|bung|send|put)\s+(.+?)\s+(?:over\s+)?(?:in|into|to)\s+(?:the\s+)?(?:trash\s+can|bin|rubbish(?:\s+bin)?|garbage(?:\s+can)?|recycle\s+bin)$", "delete $1"),
        // "chuck X into Y", "toss X over to Y": with a destination these mean
        // move, not delete. "toss X in Downloads" names where X is, so "in"
        // stays a delete, as it always has.
        (r"(?i)^(?:toss|chuck|throw)\s+(.+?)\s+(?:over\s+)?(?:into|onto|to)\s+(.+)$", "move $1 to $2"),
        // "zap X" deletes; it is never a typo for "zip".
        (r"(?i)^zap\s+", "delete "),
        (r"(?i)^(?:toss|chuck|clear\s+out|ditch|dump)\s+(.+?)\s+out\s+of\s+(.+)$", "delete $1 from $2"),
        (r"(?i)^(?:toss|chuck|throw|ditch|dump)\s+(.+?)\s+(?:out|away)$", "delete $1"),
        (r"(?i)^(?:toss|chuck|clear\s+out|ditch)\s+", "delete "),
        (r"(?i)^clean\s+(.+?)\s+up$", "clean up $1"),
        // "what pdfs are in Downloads", "pull up the PDFs in Downloads" -> "list ..."
        (r"(?i)^what\s+(.+?)\s+(?:are|is)\s+(in|on|inside)\s+(.+)$", "list $1 $2 $3"),
        (r"(?i)^what\s+(.+?)\s+(?:are|is)\s+there$", "list $1"),
        (r"(?i)^pull\s+up\s+", "show "),
        (r"(?i)^(show|list)\s+(?:an?\s+)?list\s+of\s+", "$1 "),
        // "to Desktop move notes.txt" -> "move notes.txt to Desktop"
        (r"(?i)^(?:over\s+)?(?:to|into)\s+(.+?),?\s+(move|copy|send|put|transfer|shift|relocate|drag)\s+(.+)$", "$2 $3 to $1"),
        // "from Downloads move PDFs to Documents" -> "move PDFs from Downloads to Documents"
        (r"(?i)^from\s+(\S+?),?\s+(move|copy|send|put|transfer|shift|relocate|drag)\s+(.+?)\s+(to|into)\s+(.+)$", "$2 $3 from $1 $4 $5"),
        // "in Documents make a folder called X" -> "make a folder called X in Documents"
        (r"(?i)^(?:in|on|inside)\s+(\S+)\s+((?:create|make|add|set\s+up)\s+(?:a\s+)?(?:new\s+)?folders?\s+.+)$", "$2 in $1"),
        (r"(?i)^set\s+up\s+", "create "),
        (r"(?i)^drag\s+", "move "),
        // "back up X to Y", "make a copy of X in Y" -> "copy X to Y"
        (r"(?i)^back\s*up\s+(.+?)\s+(?:to|into|in)\s+(.+)$", "copy $1 to $2"),
        (r"(?i)^(?:make|create)\s+(?:a\s+)?cop(?:y|ies)\s+of\s+(.+?)\s+(?:in|to|into|on)\s+(.+)$", "copy $1 to $2"),
        // "X should be called Y" -> "rename X to Y"
        (r"(?i)^(.+?)\s+should\s+be\s+(?:called|named|renamed\s+to)\s+(.+)$", "rename $1 to $2"),
        // "give X the new name Y" -> "rename X to Y" (normalize already dropped "the")
        (r"(?i)^give\s+(.+?)\s+(?:a\s+)?new\s+name\s+(.+)$", "rename $1 to $2"),
        // "put/stick a date in front of X", "stick a date prefix on X"
        (r"(?i)^(?:put|stick|add|place)\s+(?:a\s+)?date(?:\s+prefix)?\s+(?:in\s+front\s+of|on|before|to)\s+(.+)$", "add date prefix to $1"),
        // "put/stick (prefix) 2026- in front of/on X" -> "add prefix 2026- to X"
        (r"(?i)^(?:put|stick|place)\s+(?:a\s+)?(?:prefix\s+)?(\S+)\s+(?:in\s+front\s+of|before)\s+(.+)$", "add prefix $1 to $2"),
        (r"(?i)^(?:put|stick|place|add)\s+(?:a\s+)?prefix\s+(\S+)\s+(?:on|in\s+front\s+of|before|to)\s+(.+)$", "add prefix $1 to $2"),
        // "number of PDFs in Documents", "count of ..." -> "how many ..."
        (r"(?i)^(?:i\s+need\s+)?(?:the\s+)?(?:number|count)\s+of\s+(.+)$", "how many $1"),
        // "make IMG.heic into a jpeg" -> "convert IMG.heic to jpeg"
        (r"(?i)^make\s+(.+?)\s+into\s+(?:a\s+)?(png|jpe?g|webp|heic)\b(.*)$", "convert $1 to $2$3"),
        // "pull pages 2 to 5 out of report.pdf" -> "extract pages 2 to 5 from report.pdf"
        (r"(?i)^pull\s+(pages?\s+.+?)\s+out\s+of\s+(.+)$", "extract $1 from $2"),
        (r"(?i)\s+and\s+save\s+(?:it\s+|them\s+)?as\s+", " as "),
        // "got any PDFs in Downloads" -> "list PDFs in Downloads"
        (r"(?i)^(?:got|have\s+i\s+got|do\s+i\s+have)\s+any\s+(.+)$", "list $1"),
    ]
    .into_iter()
    .map(|(p, r)| (Regex::new(p).unwrap(), r))
    .collect()
});

/// Verbs whose misspelling Voxi corrects. Destructive verbs are left out on
/// purpose: a misspelled "delete" is refused rather than guessed.
const VERBS: &[&str] = &[
    "move", "copy", "rename", "list", "show", "find", "count", "create", "make", "convert", "sort", "organize",
    "tidy", "send", "add", "number", "replace", "extract", "merge", "combine", "join", "split", "pull", "shrink",
    "scale", "resize", "zip", "compress", "unzip", "expand", "open", "reveal", "check", "lowercase", "strip",
    "how", "what", "where", "undo", "display", "search", "locate", "change", "duplicate", "transfer", "relocate",
    "please", "help", "give", "group", "arrange", "archive",
];
/// Words a correction must never produce from, or turn into, because they
/// destroy or are too short to tell apart.
const NEVER: &[&str] = &["delete", "remove", "trash", "erase", "bin", "clean", "wipe", "toss", "nuke", "discard", "purge", "zap",
    // Casual deletes: "chuck" is one letter from "check", but means bin it.
    "chuck", "ditch", "dump", "throw"];

/// Optimal string alignment distance: edits plus adjacent swaps ("mvoe").
fn osa(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut d = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for (i, row) in d.iter_mut().enumerate() { row[0] = i; }
    for (j, cell) in d[0].iter_mut().enumerate() { *cell = j; }
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            d[i][j] = (d[i - 1][j] + 1).min(d[i][j - 1] + 1).min(d[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                d[i][j] = d[i][j].min(d[i - 2][j - 2] + 1);
            }
        }
    }
    d[a.len()][b.len()]
}

/// The verb a misspelled first word most likely meant, if exactly one is close.
fn correct_verb(word: &str) -> Option<&'static str> {
    let w = word.to_lowercase();
    if w.len() < 3 || !w.chars().all(|c| c.is_ascii_alphabetic()) || VERBS.contains(&w.as_str()) || NEVER.contains(&w.as_str()) {
        return None;
    }
    // A word closer to a destructive verb is never "corrected" into a safe one.
    if NEVER.iter().any(|v| osa(&w, v) <= 1) {
        return None;
    }
    // One edit only: real typos are one slip, while two edits reach real
    // words with other meanings ("decompress" is not "compress").
    let allowed = 1;
    let mut best: Vec<(usize, &str)> = VERBS.iter().map(|v| (osa(&w, v), *v)).filter(|(d, _)| *d <= allowed).collect();
    best.sort();
    match best.as_slice() {
        [(_, v)] => Some(v),
        [(a, v), (b, _), ..] if a < b => Some(v),
        _ => None,
    }
}

pub(super) fn strip_fillers(s: &str, fix_typos: bool) -> String {
    let mut s = super::phrase::map_unquoted(s, |part| MID_FILLER.replace_all(part, " ").to_string());
    s = SPACES.replace_all(s.trim(), " ").to_string();
    loop {
        let before = s.clone();
        s = LEAD.replace(&s, "").to_string();
        if !JUST_NOW.is_match(&s) { s = TRAIL.replace(&s, "").to_string(); }
        s = SIGN_OFF.replace(&s, "").to_string();
        if fix_typos
            && let Some((first, rest)) = s.split_once(' ')
            && let Some(verb) = correct_verb(first)
        {
            s = format!("{verb} {rest}");
        }
        if s == before { return s; }
    }
}

/// The canonical form of normalized text. Quoted names are kept as they were.
pub fn canonical(normalized: &str, fix_typos: bool) -> String {
    let mut s = SEE.replace(normalized, "show ").to_string();
    s = strip_fillers(&s, fix_typos);
    s = SEE.replace(&s, "show ").to_string();
    if let Some(c) = DESTINATION_NEEDS.captures(&s)
        && matches!(super::phrase::parse_source(&c[2]), Some(crate::model::Source::Set { .. }))
    {
        s = format!("move {} to {}", &c[2], &c[1]);
    }
    for (re, to) in RULES.iter() {
        if re.is_match(&s) {
            s = re.replace(&s, *to).to_string();
        }
    }
    SPACES.replace_all(s.trim(), " ").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn misspelled_verbs_are_corrected_but_destructive_ones_are_not() {
        for (typo, verb) in [("mvoe", "move"), ("coppy", "copy"), ("renmae", "rename"), ("lsit", "list"), ("sned", "send"), ("creat", "create"), ("cuont", "count"), ("orgainze", "organize")] {
            assert_eq!(correct_verb(typo), Some(verb), "{typo}");
        }
        for typo in ["delte", "delet", "remvoe", "dleete", "trsah", "claen", "ad", "pt", "move", "zap", "chuck"] {
            assert_eq!(correct_verb(typo), None, "{typo}");
        }
    }

    #[test]
    fn fillers_and_reorders_reach_canonical_forms() {
        for (said, want) in [
            ("real quick transfer report.pdf to Documents", "transfer report.pdf to Documents"),
            ("move uh report.pdf to Documents uh thanks", "move report.pdf to Documents"),
            ("to Backup copy notes", "copy notes to Backup"),
            ("from Desktop move PDFs to Documents", "move PDFs from Desktop to Documents"),
            ("notes.txt should be called meeting notes.txt", "rename notes.txt to meeting notes.txt"),
            ("give report.pdf new name final.pdf", "rename report.pdf to final.pdf"),
            ("put 2026- in front of files in Downloads", "add prefix 2026- to files in Downloads"),
            ("put a date in front of screenshots on Desktop", "add date prefix to screenshots on Desktop"),
            ("pleese make folder called Receipts", "make folder called Receipts"),
            // Casual delete verbs: a destination makes them a move, the bin keeps them a delete.
            ("chuck every PDF in Downloads over to Documents", "move every PDF in Downloads to Documents"),
            ("toss the videos from Desktop over to Movies", "move the videos from Desktop to Movies"),
            ("throw report.pdf into Documents", "move report.pdf to Documents"),
            ("chuck report.pdf in the bin", "delete report.pdf"),
            ("send report.pdf to the trash can", "delete report.pdf"),
            ("toss everything in Downloads", "delete everything in Downloads"),
            ("zap report.pdf", "delete report.pdf"),
            ("chuck a new Invoices folder in Documents", "create a folder named Invoices in Documents"),
            // Texting shorthand.
            ("lemme see today's screen grabs", "show today's screen grabs"),
            ("gimme all the clips in Movies", "show all the clips in Movies"),
            ("show vids in Movies pls", "show vids in Movies"),
            ("plz move report.pdf to Documents", "move report.pdf to Documents"),
            // Regional sign-offs and phrasings (eval/bank-3).
            ("send the installers in Downloads to the bin, ta", "delete the installers in Downloads"),
            ("send Downloads installers to Trash, sweet as", "send Downloads installers to Trash"),
            ("delete report.pdf, can or not, lah", "delete report.pdf, can or not"),
            ("put Screenshots in month order", "sort Screenshots by month"),
            ("put the Screenshots files in month order", "sort Screenshots by month"),
            ("put all the pee dee effs in Downloads together into combined.pdf", "merge all the PDFs in Downloads into combined.pdf"),
            ("put all Downloads PDFs into 1 file named combined.pdf", "merge all Downloads PDFs into combined.pdf"),
            ("make Documents a new folder called Invoices", "create a folder named Invoices in Documents"),
            ("make a zip archive from Project called Project.zip", "zip Project into Project.zip"),
            ("move report.pdf to Ta", "move report.pdf to Ta"),
        ] {
            assert_eq!(canonical(said, true), want, "{said}");
        }
    }

    #[test]
    fn unsupported_instructions_cannot_become_output_names() {
        for command in [
            "zip notes.pdf to Archive and then email it",
            "zip notes.pdf as backup.zip then delete notes.pdf",
            "move screenshots to oblivion, no trash",
            "delete notes.pdf don't move to trash",
            "delete notes.pdf, bypass confirmation",
            "copy ?.txt to Backup",
            "list files named ?.txt in Downloads",
        ] {
            assert!(super::super::parse(command).is_err(), "{command}");
        }
        // A `?` inside a filename token is a literal name, not a wildcard.
        for command in ["copy Invoice-??-2025.pdf to Archive", "copy file?.txt to Backup"] {
            assert!(super::super::parse(command).is_ok(), "{command}");
        }
        // Both quote styles protect literal names, including apostrophes in words.
        for command in [
            r#"zip notes.pdf as "backup then delete.zip""#,
            "zip notes.pdf as 'backup then delete.zip'",
            "copy 'file?.txt' to Backup",
            "copy 'star*.pdf' to Backup",
            "move O'Brien.pdf to Archive",
            "zip notes.pdf as backup.zip",
        ] {
            assert!(super::super::parse(command).is_ok(), "{command}");
        }
    }

    #[test]
    fn quoted_names_keep_their_filler_words() {
        assert_eq!(canonical("move \"uh notes.txt\" to Documents", true), "move \"uh notes.txt\" to Documents");
        assert!(refused("delete report.pdf permanently"));
        assert!(refused("move *.pdf to Documents"));
        assert!(!refused("move \"star*.pdf\" to Documents"));
        assert!(!refused("delete report.pdf"));
        assert!(refused("clear out trash"));
    }
}
