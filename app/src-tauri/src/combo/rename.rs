//! Rename audit: batch prefix/date/replacement/case/number rules and single names.
//! Run `cargo test --lib combo_rename -- --ignored --nocapture`.
//! Suffix and uppercase are omitted: Intent has no corresponding operation.
use super::{Case, part, wanted, report};
use serde_json::{json, Value};

fn check(text: &str, mut expected: Value, repeating: Option<bool>) -> Result<(), String> {
    let (command, schedule) = crate::scheduler::extract_schedule(text, chrono::Local::now())
        .map_err(|e| format!("schedule: {e}"))?;
    if schedule.as_ref().map(|s| s.rule.is_some()) != repeating {
        return Err("schedule presence/repetition mismatch".into());
    }
    let parsed = crate::parse::parse(&command).map_err(|_| "refused".to_string())?;
    let mut actual = serde_json::to_value(parsed).unwrap();
    fn normalize(v: &mut Value) {
        if let Value::Object(o) = v {
            o.retain(|_, v| !v.is_null());
            if let Some(Value::String(f)) = o.get_mut("folder") {
                *f = f.to_lowercase().trim_end_matches(" folder").to_string();
            }
            for v in o.values_mut() { normalize(v); }
        }
    }
    normalize(&mut actual); normalize(&mut expected);
    if actual == expected { Ok(()) } else { Err(format!("got {actual} expected {expected}")) }
}

#[test]
#[ignore]
fn combo_rename() {
    let mut cases = Vec::new();
    // Expected shapes transcribed from 5-counts-renames and 10-haiku-300.
    let rules = [
        ("add prefix Invoice- to ", "", "add_prefix", json!({"prefix":"Invoice-"})),
        ("add a prefix Invoice dash to ", "", "add_prefix", json!({"prefix":"Invoice-"})),
        ("add date prefix to ", "", "add_date_prefix", json!({})),
        ("add a date prefix to ", "", "add_date_prefix", json!({})),
        ("replace spaces with underscores in names of ", "", "replace_in_names", json!({"find":" ","replace":"_"})),
        ("replace dashes with spaces in names of ", "", "replace_in_names", json!({"find":"-","replace":" "})),
        ("lowercase names of ", "", "lowercase", json!({})),
        ("lower case names of ", "", "lowercase", json!({})),
        ("number ", " as trip-001", "number", json!({"base":"trip-","width":3})),
        ("renumber ", " as trip-001", "number", json!({"base":"trip-","width":3})),
    ];
    for polite in ["", "please ", "can you "] {
    for (verb, output, intent, fields) in &rules {
    for q in ["", "all ", "all the ", "every ", "my ", "the "] {
    for (plural, singular, ty) in [("files","file","files"),("PDFs","PDF","pdfs"),("photos","photo","images"),("screenshots","screenshot","screenshots")] {
    for (source, folder) in [("",None),(" in Downloads",Some("Downloads")),(" in my Downloads folder",Some("Downloads")),(" on the Desktop",Some("Desktop"))] {
    for (filter, extra) in [("",json!({})),(" older than 7 days",json!({"olderThanDays":7}))] {
    for (schedule, rep) in [("",None),(" every 2 days",Some(true)),(" daily",Some(true)),(" every monday",Some(true)),(" at 9 pm",Some(false)),(" tomorrow at 9am",Some(false)),(" in 3 hours",Some(false))] {
        let what = if q == "every " { singular } else { plural };
        let text = format!("{polite}{verb}{q}{what}{filter}{source}{output}{schedule}");
        if !wanted(&text) { continue; }
        let mut f = extra.clone(); f["type"] = json!(ty);
        let mut expected = fields.clone();
        expected["intent"] = json!(intent);
        expected["src"] = json!({"kind":"set","folder":folder,"filter":f});
        let verdict = check(&text, expected, rep);
        cases.push(Case { text, parts: [polite,*verb,q,plural,filter,source,*output,schedule].map(part).to_vec(), verdict });
    }}}}}}}
    report("rename batch", &["politeness","rule","quantity","type","filter","source","output","schedule"], cases);
    let mut singles = Vec::new();
    for verb in ["rename ", "change the name of "] {
    for name in ["report", "report.pdf", "notes.txt", "Draft 2"] {
    for output in ["Final", "final.pdf"] {
    for source in ["", " in Downloads", " in my Downloads folder"] {
        let text = format!("{verb}{name}{source} to {output}");
        if !wanted(&text) { continue; }
        let folder = if source.is_empty() { None } else { Some("Downloads") };
        let verdict = check(&text,json!({"intent":"rename","src":{"kind":"item","name":name,"folder":folder},"new_name":output}),None);
        singles.push(Case {text,parts:[verb,name,source,output].map(part).to_vec(),verdict});
    }}}}
    report("rename single", &["verb","name","source","output"], singles);
}
