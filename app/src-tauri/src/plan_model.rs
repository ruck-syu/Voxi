//! Native structured-plan interpretation for the explicitly enabled dev model.
//! This validates the model's protocol and literal integrity, not its semantics.
use crate::model_slot::{get_or_load, load_from_env};
use crate::{
    model::{FileType, Filter, Intent, Source},
    parse, scheduler,
};
use chrono::{DateTime, Local};
use regex::Regex;
use serde_json::Value;
use std::{
    collections::HashSet,
    path::Path,
    sync::{LazyLock, Mutex},
};
use voxi_tiny_native::NativeRewriter;

// Only a successfully loaded model is cached. A failed load is retried on the
// next call instead of being remembered forever.
static MODEL: LazyLock<Mutex<Option<NativeRewriter>>> = LazyLock::new(|| Mutex::new(None));
static ALIAS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*~\s*(\d+)\s*~\s*$").unwrap());
static EMBEDDED_ALIAS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"~\s*\d+\s*~").unwrap());

pub struct Interpreted {
    pub intent: Intent,
    pub when: Option<scheduler::When>,
}

pub fn enabled() -> bool {
    std::env::var_os("VOXI_PLAN_MODEL_DIR").is_some()
}

/// The structured schema only carries a folder plus a file type. Age, size,
/// name and other narrowing filters (and single items) would be silently
/// dropped, so commands using them must not reach the model.
fn only_file_type(filter: &Filter) -> bool {
    filter.ext.is_none()
        && filter.older_than_days.is_none()
        && filter.newer_than_days.is_none()
        && filter.day.is_none()
        && filter.month.is_none()
        && filter.year.is_none()
        && filter.since.is_none()
        && filter.before.is_none()
        && filter.larger_than_mb.is_none()
        && filter.smaller_than_mb.is_none()
        && filter.name_contains.is_none()
        && filter.name_starts_with.is_none()
        && filter.date_field.is_none()
        && !filter.include_subfolders
}

fn model_can_express(intent: &Intent) -> bool {
    match intent {
        Intent::Move { src, .. } | Intent::Copy { src, .. } | Intent::Trash { src } => match src {
            Source::Set { filter, .. } => only_file_type(filter),
            Source::Item { .. } => false,
        },
        Intent::Chain { commands } => commands.iter().all(model_can_express),
        // CreateFolders and Rename map directly onto schema steps.
        _ => true,
    }
}

/// Whole-word, case-insensitive search in an already-lowercased haystack.
/// Boundaries are word characters (alphanumeric or `_`), so `rm` won't match
/// inside `arm` and `del` won't match inside `delet`.
fn contains_word(lower: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    let needle = needle.to_lowercase();
    let boundary = |c: Option<char>| c.is_none_or(|c| !c.is_alphanumeric() && c != '_');
    let mut from = 0;
    while let Some(offset) = lower[from..].find(&needle) {
        let start = from + offset;
        let end = start + needle.len();
        if boundary(lower[..start].chars().next_back()) && boundary(lower[end..].chars().next()) {
            return true;
        }
        from = start + lower[start..].chars().next().unwrap().len_utf8();
    }
    false
}

/// Byte ranges of quoted spans in `text`, with their unescaped values. Shared
/// by name binding and the destructive-verb guard so a quoted name is never
/// mistaken for a command verb.
fn quoted_spans(text: &str) -> Result<Vec<(usize, usize, String)>, String> {
    let mut chars = text.char_indices().peekable();
    let mut spans = Vec::new();
    while let Some((start, ch)) = chars.next() {
        let closing = match ch {
            '"' => '"',
            '“' => '”',
            '‘' => '’',
            '\'' if start == 0 || text[..start].ends_with(char::is_whitespace) => '\'',
            _ => continue,
        };
        let mut end = None;
        while let Some((index, next)) = chars.next() {
            if ch == '"' && next == '\\' {
                chars.next();
                continue;
            }
            if next == closing {
                // A bare apostrophe inside a name ("King's Papers") is not
                // a close unless a boundary follows it.
                if closing == '\'' {
                    let after = index + next.len_utf8();
                    if text[after..].chars().next().is_some_and(|c| !c.is_whitespace() && !",.;:!?)".contains(c)) {
                        continue;
                    }
                }
                end = Some(index + next.len_utf8());
                break;
            }
        }
        let end = end.ok_or("Close the quotation marks first.")?;
        let value = if ch == '"' {
            // Bind the raw span; escapes other than \" are literal text.
            text[start + ch.len_utf8()..end - closing.len_utf8()].replace("\\\"", "\"")
        } else {
            text[start + ch.len_utf8()..end - closing.len_utf8()].to_string()
        };
        spans.push((start, end, value));
    }
    Ok(spans)
}

/// The trash schema can only say "trash <file type> in <folder>". A GrammarMiss
/// that names a destructive, outward or overwrite operation must never reach
/// the model, which would silently drop its qualifiers. Fail closed (stay
/// ineligible) whenever such a word appears outside quoted names.
fn destructive_miss(text: &str) -> bool {
    let spans = quoted_spans(text).unwrap_or_default();
    let mut masked = String::new();
    let mut end = 0;
    for (start, stop, _) in &spans {
        masked.push_str(&text[end..*start]);
        masked.extend(std::iter::repeat_n(' ', stop - start));
        end = *stop;
    }
    masked.push_str(&text[end..]);
    let lower = masked.to_lowercase();

    const WORDS: &[&str] = &[
        "delete",
        "del",
        "trash",
        "remove",
        "rm",
        "erase",
        "wipe",
        "shred",
        "purge",
        "nuke",
        "zap",
        "toss",
        "bin",
        "discard",
        "destroy",
        "delet",
        "delte",
        "dlete",
        "deleet",
        "dleete",
        "remvoe",
        "trsh",
        "send",
        "email",
        "upload",
        "share",
        "overwrite",
        "replace",
    ];
    if WORDS.iter().any(|word| contains_word(&lower, word)) {
        return true;
    }
    if ["get rid of", "throw away", "throw out"].iter().any(|phrase| contains_word(&lower, phrase)) {
        return true;
    }
    // "junk" is only a verb when it leads the command; elsewhere it is a noun.
    let leading = lower.trim_start();
    leading.starts_with("junk") && leading["junk".len()..].chars().next().is_none_or(|c| !c.is_alphanumeric() && c != '_')
}

/// `text` is the user's original command; `classified` is the caller's single
/// parse of the command base (the text with any trailing schedule removed),
/// shared with the rule/intent paths so the same command isn't parsed again.
pub fn eligible(text: &str, classified: &Result<Intent, parse::ParseFailure>) -> bool {
    match classified {
        // Destructive/outward GrammarMiss commands are too lossy for the schema.
        Err(parse::ParseFailure::GrammarMiss) => !destructive_miss(text),
        // Every hard refusal (including MultipleActions) stays terminal.
        Err(parse::ParseFailure::HardRefusal(_)) => false,
        // Only the operations the schema can express are worth asking about;
        // read-only and other intents stay on their existing paths.
        Ok(
            intent @ (Intent::CreateFolders { .. }
            | Intent::Move { .. }
            | Intent::Copy { .. }
            | Intent::Trash { .. }
            | Intent::Rename { .. }
            | Intent::Chain { .. }),
        ) => model_can_express(intent),
        Ok(_) => false,
    }
}

/// Called on Tauri's existing blocking planning worker. Errors never authorize
/// a partial model plan. The caller checks its request generation before publish.
#[cfg(test)]
pub fn interpret(text: &str, cancelled: impl Fn() -> bool) -> Result<Option<Interpreted>, String> {
    interpret_at(text, cancelled, Local::now())
}

/// Same as `interpret`, with `now` supplied by the caller so the schedule
/// cross-check uses the same instant as the model's compiled schedule.
pub fn interpret_at(text: &str, cancelled: impl Fn() -> bool, now: DateTime<Local>) -> Result<Option<Interpreted>, String> {
    let prepared = Prepared::new(text)?;
    if cancelled() {
        return Err("Search cancelled.".into());
    }
    let raw = {
        let mut slot = MODEL.lock().map_err(|_| "The model worker is unavailable.".to_string())?;
        if cancelled() {
            return Err("Search cancelled.".into());
        }
        let model = get_or_load(&mut slot, || {
            load_from_env("VOXI_PLAN_MODEL_DIR", "plan model", voxi_tiny_native::PLAN_PACKAGE_ID, NativeRewriter::load_plan)
        })?;
        if cancelled() {
            return Err("Search cancelled.".into());
        }
        model.rewrite(&prepared.input).map_err(|e| e.to_string())?
    };
    if cancelled() {
        return Err("Search cancelled.".into());
    }
    eprintln!("Voxi: structured plan inference completed");
    prepared.finish(&raw, text, now)
}

/// A name the model typed itself (not a quoted binding) must still appear as a
/// whole word or phrase in what the user wrote, ignoring case.
fn literal_in_text(needle: &str, text: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    contains_word(&text.to_lowercase(), needle)
}

/// The exact spelling of a validated literal in the user's own words. The
/// model supplies structure, not names: returning its casing instead made a
/// rule reading of "downloads" and a model reading of "Downloads" look like a
/// semantic disagreement, so a repeating plan both paths understood was
/// refused. Whole-word matching keeps the integrity check intact.
fn original_spelling(value: &str, original: &str) -> Option<String> {
    let folded = value.to_lowercase();
    if folded.is_empty() {
        return None;
    }
    let mut lower = String::with_capacity(original.len());
    let mut orig_at: Vec<Option<usize>> = Vec::with_capacity(original.len() + 1);
    for (orig_byte, ch) in original.char_indices() {
        orig_at.push(Some(orig_byte));
        let before = lower.len();
        for lc in ch.to_lowercase() {
            lower.push(lc);
        }
        for _ in 1..(lower.len() - before) {
            orig_at.push(None);
        }
    }
    orig_at.push(Some(original.len()));

    let boundary = |c: Option<char>| c.is_none_or(|c| !c.is_alphanumeric() && c != '_');
    let mut from = 0;
    while let Some(offset) = lower[from..].find(&folded) {
        let start = from + offset;
        let end = start + folded.len();
        if let (Some(orig_start), Some(orig_end)) = (orig_at[start], orig_at[end])
            && boundary(original[..orig_start].chars().next_back())
            && boundary(original[orig_end..].chars().next())
        {
            return Some(original[orig_start..orig_end].to_string());
        }
        from = start + lower[start..].chars().next().map_or(1, char::len_utf8);
    }
    None
}

struct Prepared {
    input: String,
    bindings: Vec<(String, String)>,
}
impl Prepared {
    fn new(text: &str) -> Result<Self, String> {
        if text.len() > 16_384 {
            return Err("That command is too long.".into());
        }
        let spans = quoted_spans(text)?;
        let mut input = String::new();
        let mut bindings: Vec<(String, String)> = Vec::new();
        let (mut end, mut number) = (0, 0);
        for (start, stop, value) in spans {
            let alias = match bindings.iter().find(|(_, existing)| *existing == value) {
                Some((alias, _)) => alias.clone(),
                None => {
                    while text.contains(&format!("~{number}~")) {
                        number += 1;
                    }
                    let alias = format!("~{number}~");
                    number += 1;
                    bindings.push((alias.clone(), value));
                    alias
                }
            };
            input.push_str(&text[end..start]);
            input.push_str(&serde_json::to_string(&alias).unwrap());
            end = stop;
        }
        input.push_str(&text[end..]);
        Ok(Self { input, bindings })
    }

    fn literal(&self, value: &Value, used: &mut HashSet<String>, original: &str) -> Result<String, String> {
        let value = value.as_str().ok_or("The model returned an invalid name.")?;
        let restored = if let Some(caps) = ALIAS.captures(value) {
            let alias = format!("~{}~", &caps[1]);
            let (_, original) = self.bindings.iter().find(|(key, _)| *key == alias).ok_or("The model invented a name reference.")?;
            used.insert(alias);
            original.clone()
        } else {
            if EMBEDDED_ALIAS.is_match(value) {
                return Err("The model changed a name reference.".into());
            }
            if !literal_in_text(value, original) {
                return Err("The model changed a name you typed.".into());
            }
            original_spelling(value, original).ok_or_else(|| "The model changed a name you typed.".to_string())?
        };
        if restored.trim().is_empty() || restored.chars().count() > 256 || restored.chars().any(char::is_control) {
            return Err("The model returned an invalid name.".into());
        }
        Ok(restored)
    }

    fn finish(&self, raw: &str, original: &str, now: DateTime<Local>) -> Result<Option<Interpreted>, String> {
        if raw.trim() == "REFUSE" {
            return Ok(None);
        }
        let wire: Value = serde_json::from_str(raw).map_err(|_| "The model returned an incomplete plan. Try rephrasing.")?;
        let parts = wire.as_array().filter(|a| a.len() == 3 && a[0] == "plan").ok_or("The model returned an invalid plan.")?;
        let steps = parts[1].as_array().filter(|a| (1..=8).contains(&a.len())).ok_or("A model plan must have 1 to 8 steps.")?;
        let mut commands = Vec::new();
        let mut folders: Vec<Option<String>> = Vec::new();
        let mut used = HashSet::new();
        for step in steps {
            let s = step.as_array().ok_or("Invalid model step.")?;
            let op = s.first().and_then(Value::as_str).ok_or("Invalid model operation.")?;
            let mut lit =
                |index: usize| s.get(index).ok_or("Missing model argument.".into()).and_then(|v| self.literal(v, &mut used, original));
            let intent = match (op, s.len()) {
                ("mkdir", 3) => {
                    let name = lit(1)?;
                    if name.contains(['/', '\\']) || matches!(name.as_str(), "." | "..") {
                        return Err("Invalid model folder name.".into());
                    }
                    Intent::CreateFolders { names: vec![name], parent: Some(lit(2)?) }
                }
                ("move" | "copy", 4) => {
                    let kind = file_type(&s[1])?;
                    let folder = lit(2)?;
                    let to = if let Some(reference) = s[3].as_array() {
                        let index = reference
                            .get(1)
                            .and_then(Value::as_u64)
                            .filter(|_| reference.len() == 2 && reference[0] == "ref")
                            .ok_or("Invalid model folder reference.")?;
                        folders
                            .get(index as usize)
                            .and_then(Clone::clone)
                            .ok_or("The model referenced a folder that hasn't been created.")?
                    } else {
                        lit(3)?
                    };
                    let src = Source::Set { folder: Some(folder), filter: Filter { file_type: Some(kind), ..Default::default() } };
                    if op == "move" { Intent::Move { src, to } } else { Intent::Copy { src, to } }
                }
                ("rename", 4) => Intent::Rename { src: Source::Item { name: lit(1)?, folder: Some(lit(2)?) }, new_name: lit(3)? },
                ("trash", 3) => Intent::Trash {
                    src: Source::Set { folder: Some(lit(2)?), filter: Filter { file_type: Some(file_type(&s[1])?), ..Default::default() } },
                },
                _ => return Err("The model requested an unsupported operation.".into()),
            };
            folders.push(match &intent {
                Intent::CreateFolders { names, parent: Some(parent) } => {
                    Some(Path::new(parent).join(&names[0]).to_string_lossy().into_owned())
                }
                _ => None,
            });
            commands.push(intent);
        }
        if used.len() != self.bindings.len() {
            return Err("The model omitted a quoted name. Try rephrasing.".into());
        }
        let intent = if commands.len() == 1 { commands.pop().unwrap() } else { Intent::Chain { commands } };
        let when = compile_schedule(&parts[2], &intent, original, now)?;
        Ok(Some(Interpreted { intent, when }))
    }
}

fn file_type(value: &Value) -> Result<FileType, String> {
    match value.as_str() {
        Some("pdfs") => Ok(FileType::Pdfs),
        Some("images") => Ok(FileType::Images),
        Some("videos") => Ok(FileType::Videos),
        Some("files") => Ok(FileType::Files),
        Some("documents") => Ok(FileType::Documents),
        Some("screenshots") => Ok(FileType::Screenshots),
        _ => Err("The model returned an unsupported file type.".into()),
    }
}

fn compile_schedule(value: &Value, intent: &Intent, original: &str, now: DateTime<Local>) -> Result<Option<scheduler::When>, String> {
    if value.is_null() {
        return Ok(None);
    }
    let s = value.as_array().ok_or("Invalid model schedule.")?;
    let tag = s.first().and_then(Value::as_str).ok_or("Invalid model schedule.")?;
    let number = |v: &Value| v.as_u64().filter(|n| (1..=365).contains(n)).ok_or("Invalid model schedule interval.");
    let clock = |v: &Value| -> Result<String, String> {
        let raw = v.as_str().ok_or("Invalid model clock time.")?;
        if raw.len() != 5 || raw.as_bytes()[2] != b':' || !raw.bytes().enumerate().all(|(i, c)| i == 2 || c.is_ascii_digit()) {
            return Err("Invalid model clock time.".into());
        }
        if raw[..2].parse::<u32>().unwrap() > 23 || raw[3..].parse::<u32>().unwrap() > 59 {
            return Err("Invalid model clock time.".into());
        }
        Ok(raw.to_string())
    };
    // Only typed, validated model fields enter the scheduler's canonical forms.
    // User language is interpreted by the model, not rewritten by new patterns.
    let suffix = match (tag, s.len()) {
        ("repeat", 4) => {
            let n = number(&s[1])?;
            let unit = s[2].as_str().filter(|u| ["minutes", "hours", "days", "weeks"].contains(u)).ok_or("Invalid model repeat unit.")?;
            if unit == "minutes" && n < 15 {
                return Err("Repeat at most every 15 minutes.".into());
            }
            let mut suffix = format!("every {n} {unit}");
            if !s[3].is_null() {
                if !["days", "weeks"].contains(&unit) {
                    return Err("A clock time requires days or weeks.".into());
                }
                suffix.push_str(&format!(" at {}", clock(&s[3])?));
            }
            suffix
        }
        ("weekday", 3) => {
            let day = s[1]
                .as_str()
                .filter(|d| ["monday", "tuesday", "wednesday", "thursday", "friday", "saturday", "sunday"].contains(d))
                .ok_or("Invalid model weekday.")?;
            format!("every {day} at {}", clock(&s[2])?)
        }
        ("after", 3) => {
            let n = number(&s[1])?;
            let unit = s[2].as_str().filter(|u| ["minutes", "hours"].contains(u)).ok_or("Invalid model delay unit.")?;
            format!("after {n} {unit}")
        }
        ("at", 3) => {
            let day = match s[2].as_str() {
                Some("next") => "",
                Some("tomorrow") => " tomorrow",
                _ => return Err("Invalid model date reference.".into()),
            };
            format!("at {}{day}", clock(&s[1])?)
        }
        _ => return Err("Unsupported model schedule.".into()),
    };
    let (_, mut when) = scheduler::extract_schedule(&format!("model-command {suffix}"), now)?;
    if let Some(rule) = when.as_mut().and_then(|w| w.rule.as_mut()) {
        rule.command = original.to_string();
        rule.model_intent = Some(intent.clone());
    }
    Ok(when)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quoted_values_restore_exactly_and_bad_references_refuse() {
        let text = "make \"資料 2028\" in Documents and move PDFs into it daily";
        let p = Prepared::new(text).unwrap();
        let raw = r#"["plan",[["mkdir","~ 0~ ","Documents"],["move","pdfs","Documents",["ref",0]]],["repeat",1,"days",null]]"#;
        let result = p.finish(raw, text, Local::now()).unwrap().unwrap();
        let Intent::Chain { commands } = &result.intent else { panic!() };
        assert!(matches!(&commands[0], Intent::CreateFolders { names, .. } if names == &["資料 2028"]));
        assert_eq!(result.when.unwrap().rule.unwrap().model_intent, Some(result.intent));
        for bad in [
            raw.replace("~ 0~ ", "invented"),
            raw.replace("~ 0~ ", "~9~"),
            raw.replace(r#"["ref",0]"#, r#"["ref",1]"#),
            raw.replace(r#""mkdir""#, r#""upload""#),
        ] {
            assert!(p.finish(&bad, text, Local::now()).is_err());
        }
    }
    #[test]
    fn model_literals_keep_the_users_spelling() {
        // The model writes "Downloads"; the user typed "downloads". Names come
        // from the user's words, so a planning cross-check must not read this
        // as a different command and refuse a repeating plan.
        let text = "create a folder in downloads called PDFS and move pdfs into it everyday";
        let p = Prepared::new(text).unwrap();
        let raw = r#"["plan",[["mkdir","PDFS","Downloads"],["move","pdfs","Downloads",["ref",0]]],["repeat",1,"days",null]]"#;
        let result = p.finish(raw, text, Local::now()).unwrap().unwrap();
        let Intent::Chain { commands } = &result.intent else { panic!() };
        assert!(
            matches!(&commands[0], Intent::CreateFolders { names, parent: Some(parent) } if names == &["PDFS"] && parent == "downloads")
        );
        assert!(
            matches!(&commands[1], Intent::Move { src: Source::Set { folder: Some(folder), .. }, to } if folder == "downloads" && to == "downloads/PDFS")
        );
        // A genuinely different name is still refused, not silently repaired.
        for invented in ["Other", "Documents"] {
            assert!(p.finish(&raw.replace("\"Downloads\"", &format!("\"{invented}\"")), text, Local::now()).is_err(), "{invented}");
        }
    }
    #[test]
    fn schema_checks_and_protocol_never_execute_or_repair_semantics() {
        let text = "copy PDFs from Downloads to Documents daily";
        let p = Prepared::new(text).unwrap();
        assert!(p.finish("REFUSE", text, Local::now()).unwrap().is_none());
        assert!(p.finish("not json", text, Local::now()).is_err());
        for schedule in [r#"["repeat",5,"minutes",null]"#, r#"["repeat",2,"hours","09:00"]"#, r#"["at","25:00","next"]"#] {
            assert!(p.finish(&format!(r#"["plan",[["copy","pdfs","Downloads","Documents"]],{schedule}]"#), text, Local::now()).is_err());
        }
    }
    fn eligible_text(text: &str) -> bool {
        let base = scheduler::extract_schedule(text, Local::now()).ok().map(|(base, _)| base);
        eligible(text, &parse::parse_classified(base.as_deref().unwrap_or(text)))
    }

    #[test]
    fn read_only_commands_and_explicit_unsupported_requests_keep_their_paths() {
        assert!(!eligible_text("undo last"));
        assert!(!eligible_text("list PDFs in Downloads"));
        assert!(!eligible_text("upload PDFs to Drive"));
        assert!(eligible_text("create a folder in downloads called PDFS and move pdfs into it everyday"));
    }
    #[test]
    fn inexpressible_filters_items_and_refusals_are_not_eligible() {
        // Filters the schema cannot carry get silently dropped by the model.
        assert!(!eligible_text("move files larger than 100 MB from Downloads to External"));
        assert!(!eligible_text("move files containing invoice from Downloads to Invoices"));
        assert!(!eligible_text("delete screenshots older than 30 days"));
        assert!(!eligible_text("clean up Downloads older than 90 days"));
        // Single items aren't representable as a file-type set either.
        assert!(!eligible_text("copy notes.txt to Desktop"));
        assert!(!eligible_text("Delete the claude.dmg file in downloads folder."));
        // A plain file-type set is exactly what the model can express.
        assert!(eligible_text("move all PDFs from Downloads to Documents"));
        assert!(eligible_text("copy all images from Pictures to Backup"));
        // Hard refusals stay terminal, including MultipleActions.
        assert!(!eligible_text("move PDFs to Documents and upload them to Drive"));
    }
    #[test]
    fn destructive_grammar_misses_never_reach_the_model() {
        // Qualifiers the trash schema drops must fail closed.
        assert!(!eligible_text("delete everything in Downloads without PDFs"));
        assert!(!eligible_text("delete duplicate files in Downloads"));
        assert!(!eligible_text("delete the most recent file in Downloads"));
        // Misspelled destructive verbs are blocked too.
        assert!(!eligible_text("delet the last file"));
        assert!(!eligible_text("delte them"));
        // Phrasal and outward/overwrite verbs are equally lossy.
        assert!(!eligible_text("get rid of the old stuff"));
        assert!(!eligible_text("throw away the old one please"));
        assert!(!eligible_text("send them"));
        assert!(!eligible_text("overwrite report.pdf with final.pdf"));
    }
    #[test]
    fn non_destructive_grammar_misses_and_quoted_names_stay_eligible() {
        // A miss with no destructive/outward verb remains eligible.
        assert!(matches!(parse::parse_classified("move the previous file to Documents"), Err(parse::ParseFailure::GrammarMiss)));
        assert!(eligible_text("move the previous file to Documents"));
        // A quoted name containing "delete" is not a command verb.
        let quoted = "move \"delete me.txt\" to the Finance folder inside Documents";
        assert!(matches!(parse::parse_classified(quoted), Err(parse::ParseFailure::GrammarMiss)));
        assert!(eligible_text(quoted));
        // "bin" inside a quoted name does not false-match either.
        let binned = "move \"bin folder\" to the Finance folder inside Documents";
        assert!(eligible_text(binned));
    }
    #[test]
    fn unquoted_literals_must_match_typed_text() {
        let text = "move PDFs from Downloads to Archive";
        let p = Prepared::new(text).unwrap();
        let good = r#"["plan",[["move","pdfs","Downloads","Archive"]],null]"#;
        assert!(p.finish(good, text, Local::now()).is_ok());
        // The model invented a destination and widened a typed folder name.
        for bad in [good.replace("\"Archive\"", "\"Backup\""), good.replace("\"Downloads\"", "\"Download\"")] {
            let error = p.finish(&bad, text, Local::now()).err().unwrap();
            assert_eq!(error, "The model changed a name you typed.");
        }
        assert!(literal_in_text("archive", text));
        assert!(!literal_in_text("archives", text));
    }
    #[test]
    fn original_spelling_handles_unicode_expansion_and_longer_needles() {
        assert_eq!(original_spelling("i\u{307}", "İ").as_deref(), Some("İ"));
        assert_eq!(original_spelling("longer", "hi"), None);
        assert_eq!(original_spelling("i", "İ"), None);
    }
    #[test]
    fn single_quotes_bind_through_inner_apostrophes() {
        let text = "move 'King's Papers' to Archive";
        let p = Prepared::new(text).unwrap();
        assert_eq!(p.bindings, vec![("~0~".into(), "King's Papers".into())]);
        assert!(p.input.contains("\"~0~\""));
        // A trailing possessive followed by punctuation still closes.
        let p = Prepared::new("move 'James's' papers to Archive").unwrap();
        assert_eq!(p.bindings, vec![("~0~".into(), "James's".into())]);
    }
    #[test]
    fn double_quoted_spans_bind_raw_text() {
        let p = Prepared::new(r#"move "a\nb.pdf" to Archive"#).unwrap();
        assert_eq!(p.bindings, vec![("~0~".into(), r"a\nb.pdf".into())]);
        // \" remains an escaped quote, but only that escape is unescaped.
        let p = Prepared::new(r#"move "say \"hi\".pdf" to Archive"#).unwrap();
        assert_eq!(p.bindings, vec![("~0~".into(), "say \"hi\".pdf".into())]);
        // An unmatched quote is still refused.
        assert!(Prepared::new(r#"move "unclosed to Archive"#).is_err());
    }
    #[test]
    #[ignore = "requires VOXI_PLAN_MODEL_DIR pointing to the native v4c package"]
    fn structured_model_native_infers_the_known_daily_plan() {
        let text = "create a folder in downloads called PDFS and move pdfs into it everyday";
        let result = interpret(text, || false).unwrap().unwrap();
        assert!(matches!(result.intent, Intent::Chain { ref commands } if commands.len() == 2));
        let rule = result.when.unwrap().rule.unwrap();
        assert_eq!(rule.every, 1);
        assert_eq!(rule.unit, scheduler::Unit::Days);
        assert!(rule.model_intent.is_some());
    }

    #[test]
    #[ignore = "report-only native audit; requires VOXI_PLAN_MODEL_DIR and VOXI_PLAN_AUDIT_OUT"]
    fn structured_model_native_audit() {
        use chrono::TimeZone;
        use std::time::Instant;
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/testing/plan-model-v4-2026-11-17/challenge-003.jsonl");
        let mut model = NativeRewriter::load_plan(std::env::var_os("VOXI_PLAN_MODEL_DIR").unwrap()).unwrap();
        let now = Local.with_ymd_and_hms(2026, 11, 17, 12, 0, 0).single().unwrap();
        let mut records = Vec::new();
        for line in std::fs::read_to_string(path).unwrap().lines().filter(|l| !l.is_empty()) {
            let row: Value = serde_json::from_str(line).unwrap();
            let text = row["text"].as_str().unwrap();
            let prepared = Prepared::new(text).unwrap();
            let start = Instant::now();
            let raw = model.rewrite(&prepared.input);
            let elapsed_ms = start.elapsed().as_secs_f64() * 1000.;
            let actual = raw.as_ref().map_err(ToString::to_string).and_then(|raw| prepared.finish(raw, text, now));
            let target = &row["target"];
            let expected = if target == "REFUSE" {
                None
            } else {
                let mut steps = target["steps"].clone();
                for step in steps.as_array_mut().unwrap() {
                    for arg in step.as_array_mut().unwrap() {
                        if let Some(index) = arg.get("ref").cloned() {
                            *arg = serde_json::json!(["ref", index]);
                        }
                    }
                }
                let wire = serde_json::json!(["plan", steps, target["schedule"]]).to_string();
                Prepared { input: String::new(), bindings: vec![] }.finish(&wire, text, now).unwrap()
            };
            let accepted = matches!(&actual, Ok(Some(_)));
            let correct = match (&actual, &expected) {
                (Ok(Some(a)), Some(b)) => a.intent == b.intent && a.when == b.when,
                (Ok(None) | Err(_), None) => true,
                _ => false,
            };
            records.push(serde_json::json!({"id":row["id"],"text":text,"target":target,"raw":raw.as_ref().ok(),"error":actual.as_ref().err(),"accepted":accepted,"correct":correct,"elapsed_ms":elapsed_ms}));
        }
        assert_eq!(records.len(), 50);
        let output = records.iter().map(|r| format!("{r}\n")).collect::<String>();
        std::fs::write(std::env::var_os("VOXI_PLAN_AUDIT_OUT").unwrap(), output).unwrap();
        eprintln!(
            "Native audit: {} correct/blocked of 50; {} accepted wrong plans",
            records.iter().filter(|r| r["correct"] == true).count(),
            records.iter().filter(|r| r["accepted"] == true && r["correct"] == false).count()
        );
    }
}
