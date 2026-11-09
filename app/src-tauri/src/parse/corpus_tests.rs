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

/// Opt-in extension inventory audit. The expected parser contract is the
/// current, explicitly supported single-extension vocabulary; catalog entries
/// outside that set are recorded as gaps rather than blessed by observation.
/// Generated expectations use the same independent rule: well-formed custom
/// suffix syntax is expected to produce an extension filter, so current
/// implementation gaps remain visible in the JSONL.
#[test]
#[ignore]
fn extension_inventory_audit_jsonl() {
    let source = crate_root().join("../../docs/testing/extension-audit-catalog.txt");
    let source = fs::canonicalize(source).expect("extension audit catalog");
    let mut extensions = Vec::new();
    for line in fs::read_to_string(&source).unwrap().lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') { continue; }
        let (_, words) = line.split_once(':').expect("category: extensions");
        extensions.extend(words.split_whitespace().map(str::to_string));
    }
    extensions.sort();
    extensions.dedup();

    let mut rows = String::new();
    let mut mismatches = 0usize;
    let mut parser_pass = 0usize;
    for ext in &extensions {
        let command = format!("list .{ext} files in Downloads");
        // A leading dot identifies suffix syntax independently of whether
        // the parser currently has an alias for that suffix.
        let expected = true;
        let outcome = extension_parse_outcome(&command);
        let actual = outcome.0;
        let expected_exts = expected_aliases(ext);
        let actual_exts = outcome.1.pointer("/src/filter/ext").cloned().unwrap_or(Value::Null);
        let actual_type = outcome.1.pointer("/src/filter/type").cloned().unwrap_or(Value::Null);
        let type_supports_requested = match actual_type.as_str() {
            Some("pdfs") => ext == "pdf",
            Some("archives") => ["zip", "rar", "7z", "tar", "gz", "tgz", "bz2", "xz"].contains(&ext.as_str()),
            Some("installers") => ["dmg", "pkg", "mpkg"].contains(&ext.as_str()),
            _ => false,
        };
        let exact_alias_match = actual_exts == serde_json::to_value(&expected_exts).unwrap();
        let supports_requested = exact_alias_match || type_supports_requested
            || outcome.1.pointer("/src/filter/ext").and_then(Value::as_array)
                .is_some_and(|a| a.iter().any(|v| v.as_str() == Some(ext)));
        let pass = actual == expected;
        parser_pass += usize::from(pass);
        mismatches += usize::from(!pass);
        rows.push_str(&serde_json::json!({
            "kind":"catalog", "suffix":ext, "command":command,
            "expected_extension_set":expected, "recognized_extension_syntax":actual,
            "expected_alias_extensions":expected_exts, "actual_filter_ext":actual_exts,
            "actual_filter_type":actual_type, "supports_requested_suffix":supports_requested,
            "exact_alias_match":exact_alias_match, "actual_intent":outcome.1, "pass":pass,
        }).to_string());
        rows.push('\n');
    }

    // Deterministic xorshift32 corpus: seed 0x564f5849, alphabet [a-z0-9],
    // lengths 1..=8. Skip catalog items and duplicates; include unambiguous
    // leading-dot syntax. This samples the domain; it does not enumerate it.
    let mut state = 0x564f_5849u32;
    let alphabet = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let mut generated = std::collections::BTreeSet::new();
    while generated.len() < 1024 {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        let len = 1 + (state as usize % 8);
        let mut word = String::new();
        for _ in 0..len {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            word.push(alphabet[state as usize % alphabet.len()] as char);
        }
        if !extensions.contains(&word) { generated.insert(word); }
    }
    let boundary_samples = [
        "abcdefghij", "extensionlongerthaneight", "abc-def", "abc_def", "éclair", "猫", "a1-b2", "a1_b2",
    ];
    for sample in boundary_samples { generated.insert(sample.to_string()); }
    let compounds = fs::read_to_string(crate_root().join("../../docs/testing/extension-audit-compound-suffixes.txt")).unwrap();
    for suffix in compounds.lines().map(str::trim).filter(|s| !s.is_empty() && !s.starts_with('#')) {
        generated.insert(suffix.to_string());
    }
    let mut generated_pass = 0usize;
    for ext in &generated {
        let command = format!("list .{ext} files in Downloads");
        let outcome = extension_parse_outcome(&command);
        let actual = outcome.0;
        let pass = actual;
        generated_pass += usize::from(pass);
        mismatches += usize::from(!pass);
        rows.push_str(&serde_json::json!({
            "kind":"generated", "seed":"0x564f5849", "domain":"[a-z0-9], lengths 1..=8, xorshift32, 1024 unique custom suffixes",
            "suffix":ext, "command":command, "expected_extension_set":true,
            "recognized_extension_syntax":actual, "actual_intent":outcome.1, "pass":pass,
        }).to_string());
        rows.push('\n');
    }

    let catalog_pass = parser_pass;
    let mut variant_pass = 0;
    // Separately specified syntax variants check case, singular/plural, and
    // list/count forms for a confirmed suffix. All are expected as sets.
    for command in [
        "list .stl file in Downloads", "list .STL files in Downloads",
        "list STL files in Downloads", "count STL files in Downloads",
        "count .stl files in Downloads", "list stl files in Downloads",
    ] {
        let outcome = extension_parse_outcome(command);
        let pass = outcome.0;
        variant_pass += usize::from(pass);
        mismatches += usize::from(!pass);
        rows.push_str(&serde_json::json!({
            "kind":"phrase_variant", "command":command, "expected_extension_set":true,
            "recognized_extension_syntax":outcome.0, "actual_intent":outcome.1, "pass":pass,
        }).to_string());
        rows.push('\n');
    }

    let report = std::env::var_os("VOXI_EXTENSION_AUDIT_REPORT")
        .map(PathBuf::from)
        .unwrap_or_else(|| crate_root().join("target/extension-audit-results.jsonl"));
    fs::create_dir_all(report.parent().unwrap()).unwrap();
    fs::write(&report, rows).unwrap();
    println!("extension audit: catalog {catalog_pass}/{}; generated/boundary {generated_pass}/{}; variants {variant_pass}/6; {mismatches} mismatches; {} distinct catalog suffixes; JSONL {}",
        extensions.len(), generated.len(), extensions.len(), report.display());
}

fn expected_aliases(ext: &str) -> Vec<String> {
    match ext {
        "jpg" | "jpeg" => vec!["jpg".into(), "jpeg".into()],
        "heic" | "heif" => vec!["heic".into(), "heif".into()],
        // `raw` is the parser's authored raw-camera family word. This is a
        // noted ambiguity for explicit `.raw` requests and remains visible.
        "raw" => vec!["raw".into(), "cr2".into(), "nef".into(), "arw".into(), "dng".into()],
        other => vec![other.to_string()],
    }
}

fn extension_parse_outcome(command: &str) -> (bool, Value) {
    use crate::model::{FileType, Source};
    match parse(command) {
        Ok(intent) => {
            let source = match &intent {
                crate::model::Intent::List { src } | crate::model::Intent::Count { src } => Some(src),
                _ => None,
            };
            let is_ext_set = source.is_some_and(|src| matches!(src,
                Source::Set { filter, .. } if filter.name_contains.is_none() && filter.name_starts_with.is_none()
                    && (filter.ext.is_some() || filter.file_type.is_some_and(|t| t != FileType::Files))));
            (is_ext_set, strip_nulls(serde_json::to_value(intent).unwrap()))
        }
        Err(_) => (false, Value::String("!".into())),
    }
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

/// Runs only `corpus/10-haiku-300.txt` and writes one JSON line per row (id,
/// line, category, command, expected, actual, pass) to the file named by
/// `VOXI_CASE_REPORT`, default `target/haiku-300-cases.jsonl`. Writes the report
/// before asserting all 300 audited expectations pass.
/// Run with `cargo test --lib haiku_300 -- --ignored --nocapture`.
#[test]
#[ignore]
fn haiku_300_per_case_report() {
    super::dates::TEST_TODAY.with(|t| t.set(NaiveDate::from_ymd_opt(2026, 10, 28)));
    let source = crate_root().join("src/parse/corpus/10-haiku-300.txt");
    let text = fs::read_to_string(&source).unwrap();
    let report = std::env::var("VOXI_CASE_REPORT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| crate_root().join("target/haiku-300-cases.jsonl"));
    let (mut category, mut rows, mut passed) = (String::new(), String::new(), 0);
    let mut n = 0;
    for (i, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if let Some(header) = line.strip_prefix("# [") {
            category = format!("[{header}");
            continue;
        }
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        n += 1;
        let at = format!("10-haiku-300.txt:{}", i + 1);
        let (command, expected) = line.rsplit_once(" => ").unwrap_or_else(|| panic!("{at}: missing \" => \""));
        let want = expected_value(expected.trim(), &at);
        let got = outcome(command);
        let pass = got == want;
        passed += usize::from(pass);
        let row = serde_json::json!({
            "id": format!("H{n:03}"), "line": i + 1, "category": category,
            "command": command, "expected": want, "actual": got, "pass": pass,
        });
        rows.push_str(&row.to_string());
        rows.push('\n');
    }
    fs::write(&report, rows).unwrap();
    println!("haiku-300: {passed} of {n} rows match; wrote {}", report.display());
    assert_eq!(n, 300);
    assert_eq!(passed, n);
}

#[test]
fn haiku_suite_contains_exactly_300_distinct_phrases() {
    let cases: Vec<_> = cases_in(&crate_root().join("src/parse/corpus")).into_iter()
        .filter(|(_, _, at)| at.starts_with("10-haiku-300.txt:")).collect();
    assert_eq!(cases.len(), 300);
    let unique: std::collections::HashSet<_> = cases.iter().map(|(command, _, _)| command).collect();
    assert_eq!(unique.len(), 300);
}
