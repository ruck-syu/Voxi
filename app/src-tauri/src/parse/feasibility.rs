//! Temporary bridge for the fallback-model feasibility test (report-only).
//! `feas_export` writes the corpus and the held-out banks (with today's rule
//! results) as JSON lines; `feas_score` scores a model's rewrites of them.
//! FEAS_DIR names the folder both use.

use super::*;
use std::io::Write;

fn classified_outcome(command: &str, row: &mut Value) -> Value {
    match crate::parse::parse_classified(command) {
        Ok(intent) => {
            row["parse_status"] = serde_json::json!("accepted");
            row["fallback_eligible"] = serde_json::json!(false);
            strip_nulls(serde_json::to_value(intent).unwrap())
        }
        Err(error) => {
            row["fallback_eligible"] = serde_json::json!(error.allows_fallback());
            match error {
                crate::parse::ParseFailure::HardRefusal(reason) => {
                    row["parse_status"] = serde_json::json!("hard_refusal");
                    row["hard_refusal_reason"] = serde_json::to_value(reason).unwrap();
                }
                crate::parse::ParseFailure::GrammarMiss => row["parse_status"] = serde_json::json!("grammar_miss"),
            }
            serde_json::json!("!")
        }
    }
}

fn dir() -> PathBuf {
    PathBuf::from(std::env::var("FEAS_DIR").expect("FEAS_DIR"))
}

#[test]
#[ignore]
fn feas_export() {
    super::super::dates::TEST_TODAY.with(|t| t.set(NaiveDate::from_ymd_opt(2026, 10, 28)));
    let root = crate_root();
    let mut out = fs::File::create(dir().join("corpus.jsonl")).unwrap();
    for (command, expected, at) in cases_in(&root.join("src/parse/corpus")) {
        let want = expected_value(&expected, &at);
        writeln!(out, "{}", serde_json::json!({ "text": command, "want": want, "at": at })).unwrap();
    }
    for bank in ["bank-2", "bank-3", "heldout-1", "heldout-2"] {
        let mut out = fs::File::create(dir().join(format!("{bank}.jsonl"))).unwrap();
        for (command, expected, at) in cases_in(&root.join("eval").join(bank)) {
            let want = expected_value(&expected, &at);
            let got = outcome(&command);
            writeln!(out, "{}", serde_json::json!({ "text": command, "want": want, "rules": got, "at": at })).unwrap();
        }
    }
}

/// Reads FEAS_DIR/rewrites.jsonl ({at, text, rewrite, want}) and writes
/// scored.jsonl with what the parser makes of each rewrite.
#[test]
#[ignore]
fn feas_score() {
    super::super::dates::TEST_TODAY.with(|t| t.set(NaiveDate::from_ymd_opt(2026, 10, 28)));
    let text = fs::read_to_string(dir().join("rewrites.jsonl")).unwrap();
    let mut out = fs::File::create(dir().join("scored.jsonl")).unwrap();
    for line in text.lines() {
        let mut row: Value = serde_json::from_str(line).unwrap();
        // Inputs may be copied from an earlier score. Recompute policy and
        // schedule metadata for this command; never trust caller-supplied flags.
        for key in [
            "parse_status",
            "hard_refusal_reason",
            "fallback_eligible",
            "blocked_before_fallback",
            "schedule_application_refusal",
            "schedule_due",
            "schedule_rule",
        ] {
            row.as_object_mut().unwrap().remove(key);
        }
        if row["want"].is_object() {
            let mut label = row["want"].clone();
            label.as_object_mut().unwrap().remove("scheduled");
            match serde_json::from_value::<crate::model::Intent>(label.clone()) {
                Ok(intent) => {
                    let canonical = strip_nulls(serde_json::to_value(intent).unwrap());
                    row["want_schema_valid"] = serde_json::json!(canonical == label);
                    row["want_canonical"] = canonical;
                }
                Err(error) => {
                    row["want_schema_valid"] = serde_json::json!(false);
                    row["want_schema_error"] = serde_json::json!(error.to_string());
                }
            }
        }
        let command = row["rewrite"].as_str().unwrap().to_string();
        let got = if row["pipeline"] == "scheduler" {
            use chrono::TimeZone;
            let now = chrono::Local.with_ymd_and_hms(2026, 10, 28, 12, 0, 0).single().unwrap();
            match crate::scheduler::extract_schedule(&command, now) {
                Err(_) => {
                    row["blocked_before_fallback"] = serde_json::json!(true);
                    row["parse_status"] = serde_json::json!("schedule_refusal");
                    row["fallback_eligible"] = serde_json::json!(false);
                    serde_json::json!("!")
                }
                Ok((base, when)) => {
                    if let Some(when) = &when {
                        row["schedule_due"] = serde_json::json!(when.due.to_rfc3339());
                        row["schedule_rule"] = serde_json::json!(format!("{:?}", when.rule));
                    }
                    let mut value = classified_outcome(&base, &mut row);
                    // The app rejects scheduled Info/Direct results and Undo
                    // after planning (lib.rs). Merely extracting a schedule is
                    // not evidence that the request passes that boundary.
                    if when.is_some()
                        && matches!(
                            value["intent"].as_str(),
                            Some("list" | "count" | "size" | "duplicates" | "recent_actions" | "reveal" | "undo")
                        )
                    {
                        row["blocked_before_fallback"] = serde_json::json!(true);
                        row["schedule_application_refusal"] = serde_json::json!(true);
                        row["parse_status"] = serde_json::json!("schedule_refusal");
                        row["fallback_eligible"] = serde_json::json!(false);
                        value = serde_json::json!("!");
                    }
                    if let Some(object) = value.as_object_mut()
                        && when.is_some()
                    {
                        object.insert("scheduled".into(), serde_json::json!(true));
                    }
                    value
                }
            }
        } else {
            classified_outcome(&command, &mut row)
        };
        row["got"] = got;
        writeln!(out, "{row}").unwrap();
    }
}
