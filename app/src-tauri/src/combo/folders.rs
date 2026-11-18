//! Folder creation, sorting, cleanup, and undo/refusal combinations.
//! Run: cargo test --lib combo_folders -- --ignored --nocapture
use super::{Case, part, wanted, report};
use serde_json::{Value, json};

fn normalized(v: Value) -> Value {
    match v {
        Value::String(s) => Value::String(s.to_lowercase().trim_end_matches(" folder").to_string()),
        Value::Array(a) => Value::Array(a.into_iter().map(normalized).collect()),
        Value::Object(o) => Value::Object(o.into_iter().filter(|(_,v)| !v.is_null()).map(|(k,v)| (k,normalized(v))).collect()),
        x => x,
    }
}
fn add(out: &mut Vec<Case>, parts: &[&str], expected: Option<Value>, repeat: Option<bool>) {
    let text = parts.concat();
    if !wanted(&text) { return; }
    let verdict = (|| {
        let (command, schedule) = crate::scheduler::extract_schedule(&text, chrono::Local::now()).map_err(|e| e.to_string())?;
        if schedule.as_ref().map(|s| s.rule.is_some()) != repeat { return Err("schedule mismatch".into()); }
        match (crate::parse::parse(&command), expected) {
            (Err(_), None) => Ok(()),
            (Ok(got), Some(want)) => {
                let got = serde_json::to_value(got).unwrap();
                if normalized(got.clone()) == normalized(want.clone()) { Ok(()) }
                else { Err(format!("got {got}; expected {want}")) }
            }
            (Err(_), Some(want)) => Err(format!("refused; expected {want}")),
            (Ok(got), None) => Err(format!("accepted intentionally unsupported command: {got:?}")),
        }
    })();
    out.push(Case { text, parts: parts.iter().map(|s| part(s)).collect(), verdict });
}
#[test]
#[ignore]
fn combo_folders() {
    let mut cases = Vec::new();
    let schedules = [("",None),(" every 2 days",Some(true)),(" daily",Some(true)),(" every monday",Some(true)),(" at 9 pm",Some(false)),(" tomorrow at 9am",Some(false)),(" in 3 hours",Some(false))];
    for prefix in ["", "please ", "could you ", "um "] {
        for verb in ["create ", "make ", "add ", "new "] {
            for noun in ["folder ", "a folder ", "a new folder ", "folders "] {
                for naming in ["", "called ", "named "] {
                    for (names, expected) in [("Taxes",vec!["Taxes"]),("Tax 2025",vec!["Tax 2025"]),("Receipts and Photos",vec!["Receipts","Photos"])] {
                        // Multiple names require a plural noun.
                        if expected.len()>1 && noun != "folders " { continue; }
                        for (location,parent) in [("",None),(" in Documents",Some("Documents")),(" on the Desktop",Some("Desktop")),(" inside Downloads",Some("Downloads")),(" in my Documents folder",Some("Documents"))] {
                            for (schedule,repeat) in schedules {
                                add(&mut cases,&[prefix,verb,noun,naming,names,location,schedule],Some(json!({"intent":"create_folders","names":expected,"parent":parent})),repeat);
                            }
                        }
                    }
                }
            }
        }
    }
    report("folder creation", &["prefix","verb","noun","naming","names","parent","when"], cases);
    let mut cases = Vec::new();
    for prefix in ["", "please ", "could you ", "um "] {
        for verb in ["sort ","organize ","organise ","group ","tidy up ","clean up "] {
            for quantity in ["", "the ", "my "] {
                for folder in ["Downloads","Desktop","Pictures","Screenshots"] {
                    for suffix in ["", " folder"] {
                        for (by,intent) in [(" by type","sort_by_type"),(" by file type","sort_by_type"),(" into type folders","sort_by_type"),(" by month","sort_by_month"),(" by date","sort_by_month"),(" into month folders","sort_by_month")] {
                            for (schedule,repeat) in schedules {
                                add(&mut cases,&[prefix,verb,quantity,folder,suffix,by,schedule],Some(json!({"intent":intent,"folder":folder})),repeat);
                            }
                        }
                    }
                }
            }
        }
    }
    report("folder sorting", &["prefix","verb","quantity","folder","suffix","grouping","when"], cases);
    let mut cases = Vec::new();
    for prefix in ["", "please ", "um "] {
        for command in ["undo", "undo the last change", "take that back", "revert that", "redo last move", "sort Pictures by year", "sort Downloads by size", "clean up my desktop mess"] {
            let expected = if command.starts_with("undo") || command=="take that back" || command=="revert that" { Some(json!({"intent":"undo"})) } else { None };
            add(&mut cases,&[prefix,command],expected,None);
        }
        for folder in ["Downloads", "Desktop"] {
            let command=format!("clean up {folder}");
            add(&mut cases,&[prefix,&command],Some(json!({"intent":"trash","src":{"kind":"set","folder":folder,"filter":{"type":"files"}}})),None);
        }
    }
    report("cleanup and history (year/size/redo deliberately refused)", &["prefix","command"], cases);
}
