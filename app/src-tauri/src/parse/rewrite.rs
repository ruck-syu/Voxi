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
    r"(?i)^(?:(?:so|like|just|ok|okay|real\s+quick|quickly|quick|hey|now|then|alright|right|pl[ea]+s[e]?|pl[sz]|kindly|go\s+ahead\s+and|help\s+me(?:\s+to)?|h[el]{2}p\s+me|(?:can|could|would|will)\s+you(?:\s+please)?|(?:can|could)\s+i|i\s+(?:want|need|would\s+like|wanna)\s+(?:you\s+)?to|i'?d\s+like\s+(?:you\s+)?to|let's|tell\s+me|i\s+need\s+to\s+know|i\s+wonder)[,]?\s+)+"
);
/// Fillers after the command that `normalize` doesn't catch.
static TRAIL: LazyLock<Regex> = re!(
    r"(?i)(?:,?\s+(?:real\s+quick|quickly|quick|right\s+now|now|asap|if\s+you\s+can|if\s+you\s+could|okay|ok|like|thanks|thank\s+you|thx|cheers|please|pl[sz]|for\s+me|for\s+us|period|full\s+stop))+$"
);
/// Regional sign-offs after a comma: ", ta", ", sweet as", ", lah". Only
/// after a comma, so a short folder name such as "Ta" is never cut off.
static SIGN_OFF: LazyLock<Regex> = re!(
    r"(?i),\s*(?:ta|cheers(?:\s+mate)?|sweet\s+as|mate|ja|eh|lah|leh|lor|na|yaar|innit|chur|bro|boss|no\s+worries|for\s+us|thanks\s+(?:a\s+lot|mate)|nice\s+one)\s*$"
);
static SEE: LazyLock<Regex> = re!(r"(?i)^(?:i\s+(?:need|want|wanna|would\s+like)\s+to\s+see|(?:let\s+me|lemme)\s+see|i'?d\s+like\s+to\s+see|gimme)\s+");
static SPACES: LazyLock<Regex> = re!(r"\s+");

/// Words that ask for something Voxi never does: deleting past the Trash.
static PERMANENT: LazyLock<Regex> = re!(r"(?i)\b(?:permanently|for\s+good|forever|irreversibly|irrecoverably|securely\s+erase|shred)\b|^(?:empty|clear(?:\s+out)?|wipe)\s+(?:the\s+|my\s+)?(?:trash|bin|recycle\s+bin)$");

/// True when the command must be refused whatever else it says: a wildcard
/// pattern (Voxi names files, it doesn't expand globs) or a permanent delete.
pub fn refused(normalized: &str) -> bool {
    let unquoted = super::phrase::map_unquoted(normalized, |part| part.to_string());
    let mut outside = String::new();
    let mut in_quote = false;
    for ch in unquoted.chars() {
        if ch == '"' { in_quote = !in_quote; continue; }
        if !in_quote { outside.push(ch); }
    }
    outside.contains('*') || PERMANENT.is_match(&outside)
}

/// `(pattern, replacement)` pairs, tried in order; every one that matches is applied.
static RULES: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
    [
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
        // "compress X into a zip called Y.zip" -> "compress X into Y.zip"
        (r"(?i)\s+(?:into|as|to)\s+(?:a\s+)?zip(?:\s+file)?\s+(?:called|named)\s+", " into "),
        // "number PDFs as scan-01 and scan-02" -> "number PDFs as scan-01"
        (r"(?i)^(number\s+.+?\s+(?:as|like)\s+\S+?)(?:,\s*|\s+and\s+|\s*,\s*and\s+)\S+.*$", "$1"),
        // "find where report.pdf is" -> "show me where report.pdf is"
        (r"(?i)^find\s+where\s+(.+)$", "show me where $1"),
        // "pee dee effs": how dictation spells PDFs out.
        (r"(?i)\bpee\s+dee\s+effs?\b", "PDFs"),
        // "put Screenshots in month order" -> "sort Screenshots by month"
        (r"(?i)^(?:put|arrange|sort)\s+(?:the\s+)?(\S+?)(?:\s+files)?\s+(?:in|into)\s+month(?:ly)?\s+order$", "sort $1 by month"),
        // "put the PDFs together into combined.pdf", "... into 1 file named combined.pdf" -> merge
        (r"(?i)^(?:put|stick|join)\s+(.+?)\s+(?:together\s+)?(?:into|as)\s+(?:(?:1|a\s+single)\s+(?:file|pdf)\s+(?:named|called)\s+)?(\S+\.pdf)$", "merge $1 into $2"),
        // "make Documents a new folder called Invoices"
        (r"(?i)^(?:make|create|give)\s+(\S+)\s+a\s+new\s+folder\s+(?:called|named)\s+(.+)$", "create a folder named $2 in $1"),
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
        // "to Desktop move notes.txt" -> "move notes.txt to Desktop"
        (r"(?i)^(?:over\s+)?(?:to|into)\s+(.+?)\s+(move|copy|send|put|transfer|shift|relocate|drag)\s+(.+)$", "$2 $3 to $1"),
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
    for j in 0..=b.len() { d[0][j] = j; }
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

fn strip_fillers(s: &str, fix_typos: bool) -> String {
    let mut s = super::phrase::map_unquoted(s, |part| MID_FILLER.replace_all(part, " ").to_string());
    s = SPACES.replace_all(s.trim(), " ").to_string();
    loop {
        let before = s.clone();
        s = LEAD.replace(&s, "").to_string();
        s = TRAIL.replace(&s, "").to_string();
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
    fn quoted_names_keep_their_filler_words() {
        assert_eq!(canonical("move \"uh notes.txt\" to Documents", true), "move \"uh notes.txt\" to Documents");
        assert!(refused("delete report.pdf permanently"));
        assert!(refused("move *.pdf to Documents"));
        assert!(!refused("move \"star*.pdf\" to Documents"));
        assert!(!refused("delete report.pdf"));
        assert!(refused("clear out trash"));
    }
}
