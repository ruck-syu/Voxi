//! Delete/trash sets across verbs, quantities, types, folders, filters and schedules.
//! Run: `cargo test --lib combo_delete -- --ignored --nocapture`.
//! Permanent deletion, ambiguous references, exclusions and compound commands are
//! deliberately omitted: the hand-checked corpus refuses those categories.
use super::{Case, part, report, wanted};
use serde_json::{Value, json};

fn check(text: &str, ty: &str, folder: Option<&str>, extra: &Value, schedule: Option<bool>) -> Result<(), String> {
    let (command, when) = crate::scheduler::extract_schedule(text, chrono::Local::now()).map_err(|e| format!("schedule error: {e}"))?;
    if when.as_ref().map(|w| w.rule.is_some()) != schedule {
        return Err(format!("schedule {:?} expected {schedule:?}", when.as_ref().map(|w| w.rule.is_some())));
    }
    let intent = crate::parse::parse(&command).map_err(|e| format!("refused: {e:?}"))?;
    let mut got = serde_json::to_value(intent).unwrap();
    fn normalize(v: &mut Value) {
        if let Value::Object(o) = v {
            o.retain(|_, v| !v.is_null());
            if let Some(Value::String(s)) = o.get_mut("folder") {
                *s = s.to_lowercase().trim_end_matches(" folder").to_string();
            }
            for v in o.values_mut() {
                normalize(v);
            }
        }
    }
    normalize(&mut got);
    let mut filter = json!({"type": ty});
    for (k, v) in extra.as_object().unwrap() {
        filter[k] = v.clone();
    }
    let mut expected = json!({"intent":"trash", "src":{"kind":"set", "folder":folder, "filter":filter}});
    normalize(&mut expected);
    if got != expected {
        return Err(format!("got {got} expected {expected}"));
    }
    Ok(())
}

#[test]
#[ignore]
fn combo_delete() {
    let verbs =
        [("delete", ""), ("trash", ""), ("remove", ""), ("get rid of", ""), ("clear out", ""), ("bin", ""), ("move", " to the trash")];
    let types = [
        ("files", "file", "files"),
        ("pdfs", "pdf", "pdfs"),
        ("screenshots", "screenshot", "screenshots"),
        ("images", "image", "images"),
        ("videos", "video", "videos"),
        ("documents", "document", "documents"),
        ("installers", "installer", "installers"),
        ("zip files", "zip file", "archives"),
    ];
    let sources = [
        ("", None),
        (" in Downloads", Some("downloads")),
        (" from Downloads", Some("downloads")),
        (" in my Downloads folder", Some("downloads")),
        (" from the Downloads folder", Some("downloads")),
        (" on the Desktop", Some("desktop")),
    ];
    let filters = [
        ("", json!({})),
        (" from today", json!({"day":"today"})),
        (" older than 30 days", json!({"olderThanDays":30})),
        (" bigger than 100 MB", json!({"largerThanMb":100.0})),
    ];
    let schedules = [
        ("", None),
        (" every 2 days", Some(true)),
        (" daily", Some(true)),
        (" every monday", Some(true)),
        (" at 9 pm", Some(false)),
        (" tomorrow at 9am", Some(false)),
        (" in 3 hours", Some(false)),
    ];
    let mut cases = Vec::new();
    for (verb, dest) in verbs {
        for q in ["", "all ", "all the ", "every ", "my ", "the "] {
            for (plural, single, ty) in types {
                for (source, folder) in sources {
                    for (filter, extra) in &filters {
                        for (sched, rep) in schedules {
                            let what = if q == "every " { single } else { plural };
                            let text = format!("{verb} {q}{what}{source}{filter}{dest}{sched}");
                            if !wanted(&text) {
                                continue;
                            }
                            let verdict = check(&text, ty, folder, extra, rep);
                            cases.push(Case { text, parts: [verb, q, plural, source, filter, dest, sched].map(part).to_vec(), verdict });
                        }
                    }
                }
            }
        }
    }
    report("delete/trash", &["verb", "quantity", "type", "source", "filter", "destination", "schedule"], cases);
}
