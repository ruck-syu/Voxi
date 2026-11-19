//! PDF, image and archive Cartesian audit, including filters and schedules.
//! Run: cargo test --lib combo_pdf_image_zip -- --ignored --nocapture
use super::{Case, part, report, wanted};
use serde_json::{Value, json};

fn normalize(v: &mut Value) {
    match v {
        Value::Object(o) => {
            o.retain(|_, x| !x.is_null());
            for (k, x) in o {
                if k == "folder" {
                    if let Some(s) = x.as_str() {
                        *x = json!(s.to_lowercase().trim_end_matches(" folder"));
                    }
                } else {
                    normalize(x);
                }
            }
        }
        Value::Array(a) => {
            for x in a {
                normalize(x);
            }
        }
        _ => {}
    }
}
#[test]
#[ignore]
fn combo_pdf_image_zip() {
    let mut cases = Vec::new();
    let schedules = [
        ("", None),
        (" every 2 days", Some(true)),
        (" daily", Some(true)),
        (" every monday", Some(true)),
        (" at 9 pm", Some(false)),
        (" tomorrow at 9am", Some(false)),
        (" in 3 hours", Some(false)),
    ];
    for (intent, verbs, types, tails) in [
        (
            "merge_pdf",
            vec!["merge", "combine", "join"],
            vec![("PDFs", "PDF", "pdfs"), ("pdf files", "pdf file", "pdfs")],
            vec![
                (" into out.pdf", json!({"output":"out.pdf"})),
                (" as out.pdf", json!({"output":"out.pdf"})),
                (" into a file called out.pdf", json!({"output":"out.pdf"})),
                (" into a file named out.pdf", json!({"output":"out.pdf"})),
            ],
        ),
        (
            "split_pdf",
            vec!["split"],
            vec![("PDFs", "PDF", "pdfs"), ("pdf files", "pdf file", "pdfs")],
            vec![("", json!({})), (" into pages", json!({})), (" into separate pages", json!({}))],
        ),
        (
            "convert_image",
            vec!["convert", "turn", "change", "save"],
            vec![("images", "image", "images"), ("photos", "photo", "images"), ("screenshots", "screenshot", "screenshots")],
            vec![(" to jpg", json!({"format":"jpg"})), (" into png", json!({"format":"png"})), (" as heic", json!({"format":"heic"}))],
        ),
        (
            "resize_image",
            vec!["resize", "shrink", "scale down"],
            vec![("images", "image", "images"), ("photos", "photo", "images"), ("screenshots", "screenshot", "screenshots")],
            vec![(" to 800 px wide", json!({"width":800})), (" to 1200 pixels wide", json!({"width":1200}))],
        ),
        (
            "zip",
            vec!["zip", "zip up", "compress", "archive"],
            vec![("files", "file", "files"), ("PDFs", "PDF", "pdfs")],
            vec![
                (" into out.zip", json!({"output":"out.zip"})),
                (" as out.zip", json!({"output":"out.zip"})),
                (" into a file called out.zip", json!({"output":"out.zip"})),
                (" into a file named out.zip", json!({"output":"out.zip"})),
            ],
        ),
    ] {
        for verb in verbs {
            for q in ["", "all ", "all the ", "every ", "my ", "the "] {
                for &(plural, singular, ty) in &types {
                    for (source, folder) in [
                        ("", None),
                        (" in Downloads", Some("downloads")),
                        (" from Downloads", Some("downloads")),
                        (" in my Downloads folder", Some("downloads")),
                        (" on the Desktop", Some("desktop")),
                    ] {
                        for filter in ["", " named scan", " older than 7 days"] {
                            for (tail, extra) in &tails {
                                for &(schedule, repeat) in &schedules {
                                    let text = format!(
                                        "{verb} {q}{}{source}{filter}{tail}{schedule}",
                                        if q == "every " { singular } else { plural }
                                    );
                                    if !wanted(&text) {
                                        continue;
                                    }
                                    let mut f = json!({"type":ty});
                                    if filter == " named scan" {
                                        f["nameContains"] = json!("scan");
                                    }
                                    if filter == " older than 7 days" {
                                        f["olderThanDays"] = json!(7);
                                    }
                                    let mut expected = extra.clone();
                                    expected["intent"] = json!(intent);
                                    expected["src"] = json!({"kind":"set","folder":folder,"filter":f});
                                    normalize(&mut expected);
                                    let verdict = check(&text, expected, repeat);
                                    cases.push(Case {
                                        text,
                                        parts: [verb, q, plural, source, filter, tail, schedule].map(part).to_vec(),
                                        verdict,
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    // Archive sets are deliberately refused; use explicit zip filenames.
    for verb in ["unzip", "extract", "expand", "unpack", "decompress"] {
        for name in ["archive.zip", "Backup.zip", "Downloads/bundle.zip"] {
            for tail in ["", " to Documents", " into Documents"] {
                for &(schedule, repeat) in &schedules {
                    let text = format!("{verb} {name}{tail}{schedule}");
                    if !wanted(&text) {
                        continue;
                    }
                    let mut expected = json!({"intent":"unzip","src":{"kind":"item","name":name}});
                    if !tail.is_empty() {
                        expected["destination"] = json!("Documents");
                    }
                    let verdict = check(&text, expected, repeat);
                    cases.push(Case { text, parts: [verb, "", name, "", "", tail, schedule].map(part).to_vec(), verdict });
                }
            }
        }
    }
    report("PDF/image/zip", &["verb", "quantity", "type/item", "source", "filter", "output", "schedule"], cases);
}
fn check(text: &str, expected: Value, repeat: Option<bool>) -> Result<(), String> {
    let (command, schedule) = crate::scheduler::extract_schedule(text, chrono::Local::now()).map_err(|e| format!("schedule error: {e}"))?;
    if schedule.as_ref().map(|s| s.rule.is_some()) != repeat {
        return Err("schedule mismatch".into());
    }
    let parsed = crate::parse::parse(&command).map_err(|_| "refused".to_string())?;
    let mut got = serde_json::to_value(parsed).unwrap();
    normalize(&mut got);
    if got != expected {
        return Err(format!("got {got}; expected {expected}"));
    }
    Ok(())
}
