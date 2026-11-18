//! Read-only list/count questions across quantity, type, folder and filters; folder sizes.
//! Run: cargo test --lib combo_readonly -- --ignored --nocapture
//! Schedules, output destinations and contextual follow-ups are excluded: these questions
//! return immediate information and have no output/destination field in the model.
use super::{Case, part, wanted, report};
use serde_json::{json, Value};

fn norm(s: &str) -> String { s.to_lowercase().trim_end_matches(" folder").to_string() }
fn check(text: &str, expected: Value) -> Result<(), String> {
    let (command, schedule) = crate::scheduler::extract_schedule(text, chrono::Local::now()).map_err(|e| e.to_string())?;
    if schedule.is_some() { return Err("unexpected schedule".into()); }
    let intent = crate::parse::parse(&command).map_err(|_| "refused".to_string())?;
    let mut got = serde_json::to_value(intent).unwrap();
    fn clean(v: &mut Value) {
        if let Some(o) = v.as_object_mut() {
            o.retain(|_, v| !v.is_null());
            for (k, v) in o.iter_mut() {
                if k == "folder" && let Some(s) = v.as_str() { *v = json!(norm(s)); }
                clean(v);
            }
        }
    }
    let mut expected = expected;
    clean(&mut got); clean(&mut expected);
    if got == expected { Ok(()) } else { Err(format!("got {got} expected {expected}")) }
}

#[test]
#[ignore]
fn combo_readonly() {
    let mut cases = Vec::new();
    let verbs = [("list", "list"), ("show", "list"), ("show me", "list"), ("find", "list"), ("display", "list"), ("count", "count"), ("how many", "count"), ("what", "list")];
    let types = [("files","file","files"),("pdfs","pdf","pdfs"),("PDF files","PDF file","pdfs"),("screenshots","screenshot","screenshots"),("images","image","images"),("photos","photo","images"),("videos","video","videos"),("documents","document","documents"),("archives","archive","archives"),("audio files","audio file","audio")];
    let sources = [("",None),(" in Downloads",Some("Downloads")),(" in Documents",Some("Documents")),(" in my Downloads folder",Some("Downloads")),(" from Downloads",Some("Downloads")),(" on the Desktop",Some("Desktop"))];
    let filters = [("",json!({})),(" from today",json!({"day":"today"})),(" from yesterday",json!({"day":"yesterday"})),(" from last week",json!({"newerThanDays":7})),(" larger than 5 MB",json!({"largerThanMb":5.0})),(" larger than 100 MB",json!({"largerThanMb":100.0})),(" with invoice in the name",json!({"nameContains":"invoice"})),(" named like budget",json!({"nameContains":"budget"}))];
    for (verb,intent) in verbs { for q in ["","all ","all the ","every ","my ","the "] { for (plural,single,ty) in types { for (source,folder) in sources { for (filter,fields) in &filters { for order in [false,true] {
        // “what ... are in” needs a location; “every” is singular.
        if verb == "what" && (source.is_empty() || source.starts_with(" from") || source.starts_with(" on") || q == "every ") { continue; }
        let noun = if q == "every " { single } else { plural };
        let link = if verb == "what" { " are" } else { "" };
        let text = if order && verb != "what" { format!("{verb} {q}{noun}{filter}{source}") } else { format!("{verb} {q}{noun}{link}{source}{filter}") };
        if order && (source.is_empty() || filter.is_empty() || verb == "what") { continue; }
        if !wanted(&text) { continue; }
        let mut f = fields.clone(); f["type"] = json!(ty);
        let expected = json!({"intent":intent,"src":{"kind":"set","folder":folder,"filter":f}});
        let verdict = check(&text, expected);
        cases.push(Case{text,parts:[verb,q,plural,source,filter,if order {"filter first"} else {"source first"}].map(part).to_vec(),verdict});
    }}}}}}
    for verb in ["how big is", "show size of", "what is the size of", "check the size of"] { for folder in ["Downloads","Documents","Desktop","Pictures"] { for decoration in ["", "the ", "my "] { for suffix in ["", " folder"] {
        let text = format!("{verb} {decoration}{folder}{suffix}");
        if !wanted(&text) { continue; }
        let verdict = check(&text,json!({"intent":"size","folder":folder}));
        cases.push(Case{text,parts:[verb,decoration,"(folder)",folder,suffix,"(none)"].map(part).to_vec(),verdict});
    }}}}
    report("readonly (immediate questions; no schedules, outputs or follow-ups)", &["verb","quantity","type","source","filter","order"],cases);
}
