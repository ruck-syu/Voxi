//! Runs the natural-language corpus in `corpus/` through the parser that Enter
//! uses. Each line is `command => expected`; the header of the first file
//! explains the format. Every mismatch is reported in one run, so the failure
//! list is the full gap list.

use super::parse;
use chrono::NaiveDate;
use serde_json::Value;
use std::fs;
use std::path::PathBuf;

/// Drops null fields, so expected JSON can leave out empty options.
fn strip_nulls(v: Value) -> Value {
    match v {
        Value::Object(m) => Value::Object(m.into_iter().filter(|(_, v)| !v.is_null()).map(|(k, v)| (k, strip_nulls(v))).collect()),
        Value::Array(a) => Value::Array(a.into_iter().map(strip_nulls).collect()),
        other => other,
    }
}

/// `(command, expected, location)` for every case in `corpus/`, file by file.
fn cases() -> Vec<(String, String, String)> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/parse/corpus");
    let mut files: Vec<PathBuf> = fs::read_dir(&dir).expect("corpus folder").flatten().map(|e| e.path()).collect();
    files.sort();
    let mut out = Vec::new();
    for file in files {
        let name = file.file_name().unwrap().to_string_lossy().into_owned();
        let text = fs::read_to_string(&file).unwrap();
        for (i, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (command, expected) = line.rsplit_once(" => ").unwrap_or_else(|| panic!("{name}:{}: missing \" => \"", i + 1));
            out.push((command.to_string(), expected.trim().to_string(), format!("{name}:{}", i + 1)));
        }
    }
    out
}

/// The intent the parser builds as JSON, or `!` when it refuses the command.
fn outcome(command: &str) -> Value {
    match parse(command) {
        Ok(intent) => strip_nulls(serde_json::to_value(&intent).unwrap()),
        Err(_) => Value::String("!".into()),
    }
}

#[test]
fn corpus_commands_build_the_intended_intent() {
    // The expected dates were written for this day.
    super::dates::TEST_TODAY.with(|t| t.set(NaiveDate::from_ymd_opt(2026, 10, 28)));
    let cases = cases();
    let mut failures = Vec::new();
    for (command, expected, at) in &cases {
        let want: Value = if expected == "!" {
            Value::String("!".into())
        } else {
            serde_json::from_str(expected).unwrap_or_else(|e| panic!("{at}: expected JSON is invalid: {e}"))
        };
        let got = outcome(command);
        if got != want {
            failures.push(format!("{at}  {command}\n      want {want}\n      got  {got}"));
        }
    }
    println!("corpus: {} of {} commands match", cases.len() - failures.len(), cases.len());
    assert!(failures.is_empty(), "{} of {} commands differ:\n{}", failures.len(), cases.len(), failures.join("\n"));
}
