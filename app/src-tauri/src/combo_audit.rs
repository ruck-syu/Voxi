//! Report-only combination audit. Instead of single hand-written sentences, it
//! builds every combination of the parts of a file command, so a wording that
//! fails in one part shows up across every sentence that uses it:
//!
//!   verb · how many · file type · from where · to where · when
//!
//! Each sentence goes through the same path the app uses: the schedule is taken
//! off the end first, then the rest is parsed. The report gives the overall
//! failure rate, the failure rate for each value of each part, and examples.
//!
//! `cargo test --lib combo_audit -- --ignored --nocapture`
//! Set COMBO_EXAMPLES=n to print n failing sentences per part value (default 2),
//! and COMBO_ONLY=text to audit only the sentences that contain it.

#[path = "combo/delete.rs"] mod delete;
#[path = "combo/readonly.rs"] mod readonly;
#[path = "combo/rename.rs"] mod rename;
#[path = "combo/pdf_image_zip.rs"] mod pdf_image_zip;
#[path = "combo/filters.rs"] mod filters;
#[path = "combo/folders.rs"] mod folders;

use crate::parse::parse;
use crate::scheduler::extract_schedule;
use serde_json::Value;
use std::collections::BTreeMap;

const VERBS: &[(&str, &str)] = &[
    ("move", "move"),
    ("copy", "copy"),
    ("put", "move"),
    ("shift", "move"),
    ("transfer", "move"),
    ("send", "move"),
    ("Move", "move"),
];

const QUANTITY: &[&str] = &["", "all ", "all the ", "all my ", "every ", "the ", "my ", "any "];

/// (plural, singular after "every", expected type)
const TYPES: &[(&str, &str, &str)] = &[
    ("pdfs", "pdf", "pdfs"),
    ("PDFs", "PDF", "pdfs"),
    ("pdf files", "pdf file", "pdfs"),
    ("screenshots", "screenshot", "screenshots"),
    ("images", "image", "images"),
    ("photos", "photo", "images"),
    ("videos", "video", "videos"),
    ("documents", "document", "documents"),
];

/// (phrase, folder it names)
const SOURCES: &[(&str, Option<&str>)] = &[
    ("", None),
    (" in Downloads", Some("downloads")),
    (" in downloads", Some("downloads")),
    (" from Downloads", Some("downloads")),
    (" in my Downloads", Some("downloads")),
    (" in my Downloads folder", Some("downloads")),
    (" from the Downloads folder", Some("downloads")),
    (" on the Desktop", Some("desktop")),
    (" on my desktop", Some("desktop")),
];

/// (phrase, destination it names)
const DESTINATIONS: &[(&str, &str)] = &[
    (" to Archive", "archive"),
    (" into Archive", "archive"),
    (" to the Archive folder", "archive"),
    (" to my Archive folder", "archive"),
    (" into the Archive folder", "archive"),
    (" over to Archive", "archive"),
    (" to a new folder called Archive", "archive"),
    (" into a folder named Archive", "archive"),
    (" to Archive folder", "archive"),
    (" to Documents/Archive", "documents/archive"),
];

/// (phrase, repeats?) — None means no schedule.
const SCHEDULES: &[(&str, Option<bool>)] = &[
    ("", None),
    (" every 2 days", Some(true)),
    (" every day", Some(true)),
    (" daily", Some(true)),
    (" every week", Some(true)),
    (" every monday", Some(true)),
    (" every day at 9 pm", Some(true)),
    (" at 9 pm", Some(false)),
    (" tomorrow at 9am", Some(false)),
    (" in 3 hours", Some(false)),
];

/// One generated sentence: its text, the value it used for each part (in the
/// order of the report's part names), and whether the parser got it right.
pub(crate) struct Case {
    pub text: String,
    pub parts: Vec<String>,
    pub verdict: Result<(), String>,
}

/// A part value for the report: the trimmed phrase, or "(none)".
pub(crate) fn part(s: &str) -> String {
    if s.trim().is_empty() { "(none)".to_string() } else { s.trim().to_string() }
}

/// Whether COMBO_ONLY (if set) lets this sentence into the audit.
pub(crate) fn wanted(text: &str) -> bool {
    std::env::var("COMBO_ONLY").map_or(true, |o| text.contains(&o))
}

fn lower(v: Option<&Value>) -> Option<String> {
    v.and_then(Value::as_str).map(str::to_lowercase)
}

fn check(text: &str, intent: &str, ty: &str, folder: Option<&str>, to: &str, schedule: Option<bool>) -> Result<(), String> {
    let (command, when) = extract_schedule(text, chrono::Local::now()).map_err(|e| format!("schedule error: {e}"))?;
    match (schedule, &when) {
        (None, Some(_)) => return Err("found a schedule that isn't there".into()),
        (Some(_), None) => return Err("schedule not recognised".into()),
        (Some(rep), Some(w)) if rep != w.rule.is_some() => return Err(format!("repeat={} expected {rep}", w.rule.is_some())),
        _ => {}
    }
    let parsed = parse(&command).map_err(|_| format!("didn't understand {command:?}"))?;
    let v = serde_json::to_value(&parsed).unwrap();
    let got_intent = v["intent"].as_str().unwrap_or("");
    if got_intent != intent {
        return Err(format!("intent {got_intent} expected {intent}"));
    }
    let src = &v["src"];
    if src["kind"] != "set" {
        return Err(format!("source {src}"));
    }
    if lower(src.get("folder")).as_deref() != folder {
        return Err(format!("from {:?} expected {folder:?}", src.get("folder")));
    }
    let filter: BTreeMap<String, Value> = src["filter"]
        .as_object()
        .map(|o| o.iter().filter(|(_, v)| !v.is_null()).map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default();
    if filter.len() != 1 || filter.get("type").and_then(Value::as_str) != Some(ty) {
        return Err(format!("filter {} expected type {ty}", serde_json::to_string(&filter).unwrap()));
    }
    if lower(v.get("to")).as_deref() != Some(to) {
        return Err(format!("to {} expected {to:?}", v["to"]));
    }
    Ok(())
}

fn cases() -> Vec<Case> {
    let mut out = Vec::new();
    for &(verb, intent) in VERBS {
        for &q in QUANTITY {
            for &(plural, single, ty) in TYPES {
                let what = if q == "every " { single } else { plural };
                for &(source, folder) in SOURCES {
                    for &(dest, to) in DESTINATIONS {
                        for &(sched, rep) in SCHEDULES {
                            let text = format!("{verb} {q}{what}{source}{dest}{sched}");
                            if !wanted(&text) { continue; }
                            let verdict = check(&text, intent, ty, folder, to, rep);
                            let parts = [verb, q, plural, source, dest, sched].map(part).to_vec();
                            out.push(Case { text, parts, verdict });
                        }
                    }
                }
            }
        }
    }
    out
}

#[test]
#[ignore]
fn combo_audit() {
    report("move/copy", &["verb", "how many", "file type", "from", "to", "when"], cases());
}

/// Prints the failure rate overall, by reason, and for each value of each
/// part, with COMBO_EXAMPLES failing sentences under each value.
pub(crate) fn report(title: &str, names: &[&str], all: Vec<Case>) {
    let examples: usize = std::env::var("COMBO_EXAMPLES").ok().and_then(|s| s.parse().ok()).unwrap_or(2);
    let all: Vec<Case> = all.into_iter().filter(|c| wanted(&c.text)).collect();
    // COMBO_DUMP=path appends every COMBO_DUMP_EVERY-th correct sentence with
    // its intent, as JSON lines: training data for a fallback model.
    if let Ok(path) = std::env::var("COMBO_DUMP") {
        use std::io::Write;
        let every: usize = std::env::var("COMBO_DUMP_EVERY").ok().and_then(|s| s.parse().ok()).unwrap_or(10);
        let mut out = std::fs::OpenOptions::new().create(true).append(true).open(path).unwrap();
        for c in all.iter().filter(|c| c.verdict.is_ok()).step_by(every.max(1)) {
            let Ok((command, _)) = extract_schedule(&c.text, chrono::Local::now()) else { continue };
            let Ok(intent) = parse(&command) else { continue };
            let want = serde_json::to_value(&intent).unwrap();
            writeln!(out, "{}", serde_json::json!({ "text": c.text, "command": command, "want": want, "src": title })).unwrap();
        }
    }
    let wrong = all.iter().filter(|c| c.verdict.is_err()).count();
    println!("\ncombo {title}: {wrong} of {} wrong ({:.1}%)", all.len(), 100.0 * wrong as f64 / all.len().max(1) as f64);

    // Why sentences fail, grouped by the first words of the reason.
    let mut reasons: BTreeMap<String, usize> = BTreeMap::new();
    for c in &all {
        if let Err(e) = &c.verdict {
            let key: String = e.split(['"', '{']).next().unwrap_or(e).trim().to_string();
            *reasons.entry(key).or_default() += 1;
        }
    }
    let mut reasons: Vec<_> = reasons.into_iter().collect();
    reasons.sort_by_key(|b| std::cmp::Reverse(b.1));
    println!("\nreasons:");
    for (r, n) in reasons.iter().take(15) {
        println!("  {n:>7}  {r}");
    }

    for (i, name) in names.iter().enumerate() {
        let mut by: BTreeMap<&str, (usize, usize, Vec<&Case>)> = BTreeMap::new();
        for c in &all {
            let e = by.entry(c.parts[i].as_str()).or_default();
            e.0 += 1;
            if c.verdict.is_err() {
                e.1 += 1;
                if e.2.len() < examples { e.2.push(c); }
            }
        }
        let mut rows: Vec<_> = by.into_iter().collect();
        rows.sort_by(|a, b| (b.1 .1 * a.1 .0).cmp(&(a.1 .1 * b.1 .0)));
        println!("\n{name}:");
        for (value, (n, bad, ex)) in rows {
            println!("  {:>5.1}%  {value}", 100.0 * bad as f64 / n as f64);
            for c in ex {
                println!("           {}  →  {}", c.text, c.verdict.as_ref().unwrap_err());
            }
        }
    }
}
