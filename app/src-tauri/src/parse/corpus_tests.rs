//! Runs natural-language command files through the parser that Enter uses.
//! Each line is `command => expected`: the intent JSON, or `!` for a refusal.
//! The regression corpus in `src/parse/corpus/` must always pass. Held-out
//! sets in `eval/` only measure accuracy until they are used to fix something.

use super::parse;
use chrono::NaiveDate;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

/// Drops null fields, so expected JSON can leave out empty options.
fn strip_nulls(v: Value) -> Value {
    match v {
        Value::Object(m) => Value::Object(m.into_iter().filter(|(_, v)| !v.is_null()).map(|(k, v)| (k, strip_nulls(v))).collect()),
        Value::Array(a) => Value::Array(a.into_iter().map(strip_nulls).collect()),
        other => other,
    }
}

/// `(command, expected, location)` for every line of every file in `dir`, file by file.
fn cases_in(dir: &Path) -> Vec<(String, String, String)> {
    let mut files: Vec<PathBuf> = fs::read_dir(dir).expect("command folder").flatten().map(|e| e.path()).collect();
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

/// The expected value: the intent JSON, or `!` for a refusal.
fn expected_value(expected: &str, at: &str) -> Value {
    if expected == "!" {
        Value::String("!".into())
    } else {
        serde_json::from_str(expected).unwrap_or_else(|e| panic!("{at}: expected JSON is invalid: {e}"))
    }
}

/// The crate's root folder, where `src/` and `eval/` live.
fn crate_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn corpus_commands_build_the_intended_intent() {
    // The expected dates were written for this day.
    super::dates::TEST_TODAY.with(|t| t.set(NaiveDate::from_ymd_opt(2026, 10, 28)));
    let cases = cases_in(&crate_root().join("src/parse/corpus"));
    let mut failures = Vec::new();
    for (command, expected, at) in &cases {
        let want = expected_value(expected, at);
        let got = outcome(command);
        if got != want {
            failures.push(format!("{at}  {command}\n      want {want}\n      got  {got}"));
        }
    }
    println!("corpus: {} of {} commands match", cases.len() - failures.len(), cases.len());
    assert!(failures.is_empty(), "{} of {} commands differ:\n{}", failures.len(), cases.len(), failures.join("\n"));
}

/// Measures the parser on a held-out set in `eval/`, named by the `HELDOUT`
/// environment variable (default `heldout-2`). Prints every miss and the error
/// rate, and does not fail. Run it with
/// `HELDOUT=heldout-2 cargo test --lib heldout -- --ignored --nocapture`.
#[test]
#[ignore]
fn heldout_error_rate() {
    super::dates::TEST_TODAY.with(|t| t.set(NaiveDate::from_ymd_opt(2026, 10, 28)));
    let folder = std::env::var("HELDOUT").unwrap_or_else(|_| "heldout-2".into());
    let cases = cases_in(&crate_root().join("eval").join(&folder));
    let mut misses = 0;
    for (command, expected, at) in &cases {
        let want = expected_value(expected, at);
        let got = outcome(command);
        if got != want {
            misses += 1;
            println!("MISS {at}  {command}\n     want {want}\n     got  {got}");
        }
    }
    println!("heldout: {misses} of {} commands wrong ({:.1}%)", cases.len(), 100.0 * misses as f64 / cases.len() as f64);
}
