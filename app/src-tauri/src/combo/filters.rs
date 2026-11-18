//! Filter audit: dates, sizes, names, and date/size pairs in both orders and
//! on either side of the source. Run: cargo test --lib combo_filters -- --ignored --nocapture
use super::{Case, part, wanted, report};
use serde_json::{json, Value};

#[test]
#[ignore]
fn combo_filters() {
    use chrono::Datelike;
    let now = chrono::Local::now();
    let monday = now.date_naive() - chrono::Duration::days(now.weekday().num_days_from_monday() as i64);
    let dates = vec![
        ("", json!({})), (" from today", json!({"day":"today"})),
        (" from yesterday", json!({"day":"yesterday"})),
        (" from this week", json!({"since":monday.to_string()})),
        (" from last week", json!({"newerThanDays":7})),
        (" from last month", json!({"newerThanDays":30})),
        (" in October", json!({"month":10})),
        (" from 2024", json!({"since":"2024-01-01","before":"2025-01-01"})),
        (" older than 30 days", json!({"olderThanDays":30})),
        (" newer than 2 weeks", json!({"newerThanDays":14})),
        (" before July 2024", json!({"before":"2024-07-01"})),
        (" after July 2024", json!({"since":"2024-08-01"})),
    ];
    let sizes = vec![("",json!({})),(" bigger than 5 MB",json!({"largerThanMb":5.0})),
        (" larger than 1 GB",json!({"largerThanMb":1024.0})),
        (" over 10 MB",json!({"largerThanMb":10.0})),
        (" smaller than 5 MB",json!({"smallerThanMb":5.0})),
        (" under 1 GB",json!({"smallerThanMb":1024.0}))];
    let names = vec![(" with \"invoice\" in the name",json!({"nameContains":"invoice"})),
        (" named like invoice",json!({"nameContains":"invoice"})),
        (" starting with invoice",json!({"nameStartsWith":"invoice"})),
        (" ending in .txt",json!({"ext":["txt"]}))];
    let mut filters: Vec<(String, Value)> = Vec::new();
    for (d,dv) in &dates { for (s,sv) in &sizes { for reverse in [false,true] {
        if reverse && (d.is_empty() || s.is_empty()) {continue;}
        let mut v=dv.clone(); v.as_object_mut().unwrap().extend(sv.as_object().unwrap().clone());
        filters.push((if reverse {format!("{s}{d}")} else {format!("{d}{s}")},v));
    }}}
    filters.extend(names.into_iter().map(|(s,v)|(s.to_string(),v)));
    let mut cases=Vec::new();
    for (verb,intent) in [("move","move"),("delete","trash"),("list","list")] {
    for q in ["","all ","all the ","every ","my ","the "] {
    for (plural,singular,ty) in [("files","file","files"),("PDFs","PDF","pdfs"),("screenshots","screenshot","screenshots")] {
    for (source,folder) in [("",None),(" in Downloads",Some("downloads")),(" from the Downloads folder",Some("downloads"))] {
    for (filter,fields) in &filters {
        // Extension constraints with an incompatible named type have no useful everyday meaning.
        if fields.get("ext").is_some() && ty != "files" {continue;}
    for before in [true,false] {
    for (schedule,repeat) in [("",None),(" every 2 days",Some(true)),(" daily",Some(true)),(" every monday",Some(true)),(" at 9 pm",Some(false)),(" tomorrow at 9am",Some(false)),(" in 3 hours",Some(false))] {
        if intent=="list" && repeat.is_some() {continue;}
        let dest=if intent=="move" {" to Archive"} else {""};
        let noun=if q=="every " {singular} else {plural};
        let text=if before {format!("{verb} {q}{noun}{filter}{source}{dest}{schedule}")} else {format!("{verb} {q}{noun}{source}{filter}{dest}{schedule}")};
        if !wanted(&text) {continue;}
        let mut expected=fields.clone(); expected["type"]=json!(ty);
        let verdict=(|| -> Result<(),String> {
            let (command,when)=crate::scheduler::extract_schedule(&text,now).map_err(|e|format!("schedule error: {e}"))?;
            if when.as_ref().map(|w|w.rule.is_some())!=repeat {return Err("schedule mismatch".into());}
            let parsed=crate::parse::parse(&command).map_err(|_|"refused".to_string())?;
            let v=serde_json::to_value(parsed).unwrap();
            if v["intent"]!=intent {return Err(format!("intent: {v}"));}
            if v["src"]["kind"]!="set" {return Err(format!("source: {v}"));}
            let got_folder=v["src"]["folder"].as_str().map(|s|s.to_lowercase().trim_end_matches(" folder").to_string());
            if got_folder.as_deref()!=folder {return Err(format!("folder: {v}"));}
            if v["src"]["filter"]!=expected {return Err(format!("filter: got {} expected {expected}",v["src"]["filter"]));}
            if intent=="move" && v["to"].as_str().map(str::to_lowercase).as_deref()!=Some("archive") {return Err(format!("destination: {v}"));}
            Ok(())
        })();
        cases.push(Case{text,parts:vec![part(verb),part(q),part(plural),part(source),part(filter),part(if before {"before source"} else {"after source"}),part(schedule)],verdict});
    }}}}}}}
    report("filters",&["verb","quantity","type","source","filter","position","schedule"],cases);
}
