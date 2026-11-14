//! Report-only paraphrase audit for the Haiku-derived phrase set in
//! `docs/testing/paraphrase-audit-2026-11-09/`. It runs the compiled parser and
//! planner (and `plan_input` for contextual follow-ups) and writes one JSONL row
//! per case. Plans are previews against temporary fixture homes; nothing is
//! confirmed or executed. Every test is `#[ignore]`, so the default suite and the
//! golden corpus never see these cases, and mismatches are reported, not failed.

use crate::fsx::{self, file_name, Roots};
use crate::history::History;
use crate::model::{Action, Intent, Outcome};
use crate::parse::parse;
use crate::plan::{Planned, Planner, Stop};
use crate::settings::Settings;
use crate::{plan_input, AppState};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicU64;
use std::sync::{Mutex, RwLock};
use std::time::Duration;

const AUDIT_DIR: &str = "../../docs/testing/paraphrase-audit-2026-11-09";
const DEFAULT_REPORT: &str = "../../docs/testing/haiku-paraphrase-audit-2026-11-09.jsonl";
/// Only these keys hold folder names, and only these root folders may differ in casing.
const FOLDER_KEYS: [&str; 3] = ["folder", "to", "parent"];
const ROOT_FOLDERS: [&str; 5] = ["downloads", "documents", "desktop", "pictures", "screenshots"];
/// Intents that only read or open; everything else changes files when confirmed.
const READ_ONLY: [&str; 7] = ["list", "count", "size", "duplicates", "reveal", "undo", "recent_actions"];

fn audit_file(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(AUDIT_DIR).join(name)
}

/// Drops nulls and lowercases root-folder names in folder-valued keys. Nothing else changes.
fn canon(v: Value) -> Value {
    match v {
        Value::Object(map) => Value::Object(
            map.into_iter()
                .filter(|(_, v)| !v.is_null())
                .map(|(k, v)| {
                    let v = fold_root(&k, &v).unwrap_or_else(|| canon(v));
                    (k, v)
                })
                .collect(),
        ),
        Value::Array(a) => Value::Array(a.into_iter().map(canon).collect()),
        other => other,
    }
}

fn fold_root(key: &str, v: &Value) -> Option<Value> {
    match v {
        Value::String(s) if FOLDER_KEYS.contains(&key) && ROOT_FOLDERS.contains(&s.to_lowercase().as_str()) => {
            Some(Value::String(s.to_lowercase()))
        }
        _ => None,
    }
}

fn intent_name(v: &Value) -> String {
    v.get("intent").and_then(Value::as_str).unwrap_or("!").to_string()
}

fn is_mutating(intent: &str) -> bool {
    intent != "!" && !READ_ONLY.contains(&intent)
}

/// Severity of a wrong interpretation. "high" means the parser accepted a
/// changing intent and the planner would produce steps for it (preview-gated,
/// but the user would be asked to confirm the wrong change). Otherwise the
/// wrong meaning either changes nothing or only answers a question wrongly.
fn wrong_severity(actual: &str, planner: &str) -> &'static str {
    if is_mutating(actual) && planner.starts_with("steps") { "high" } else { "medium" }
}

/// `move:<name>:<dest folder>`, `trash:<name>`, `rename:<name>:<new>`, or the variant name.
fn action_key(a: &Action) -> String {
    match a {
        Action::Move { from, to } => format!("move:{}:{}", file_name(from), to.parent().map(file_name).unwrap_or_default()),
        Action::Trash { path } => format!("trash:{}", file_name(path)),
        Action::Rename { path, new_name } => format!("rename:{}:{new_name}", file_name(path)),
        other => serde_json::to_value(other).ok()
            .and_then(|v| v.get("kind").and_then(Value::as_str).map(str::to_string))
            .unwrap_or_else(|| "unknown".into()),
    }
}

fn plan_intent(roots: &Roots, intent: &Intent) -> (String, Option<Vec<String>>) {
    let choices = HashMap::new();
    let history = History::load(roots.home.join("history.json"));
    match Planner::new(roots, &choices).plan(intent, &history) {
        Ok(Planned::Steps { actions, .. }) => {
            let keys: Vec<String> = actions.iter().map(action_key).collect();
            (format!("steps:{}:{}", keys.len(), keys.join(";")), Some(keys))
        }
        Ok(Planned::Info { total, .. }) => (format!("info:{total}"), None),
        Ok(Planned::Direct { .. }) => ("direct_preview_not_executed".into(), None),
        Err(Stop::Problem(message)) => (format!("problem:{message}"), None),
        Err(Stop::Ambiguous { name, candidates }) => (format!("ambiguous:{name}:{}", candidates.len()), None),
    }
}

/// Fixture home for single-command planning: every standard folder plus the
/// extra folders the seeds name, with files the seeds refer to by name.
struct Home {
    _dir: tempfile::TempDir,
    roots: Roots,
}

fn main_home() -> Home {
    let dir = tempfile::tempdir().unwrap();
    let home = fsx::canonical(dir.path());
    let folders = ["Desktop", "Downloads", "Documents", "Pictures", "Screenshots", "Trip", "Scans", "External", "Invoices", "Backup", "Project"];
    for f in folders {
        fs::create_dir_all(home.join(f)).unwrap();
    }
    let files = [
        "Downloads/report.pdf", "Downloads/notes.txt", "Downloads/Notes from Monday.pdf", "Downloads/Notes to self.pdf",
        "Downloads/invoice-jan.pdf", "Downloads/invoice-feb.txt", "Downloads/bill-2026.pdf", "Downloads/claude.dmg",
        "Downloads/setup.pkg", "Downloads/archive.zip", "Downloads/a.pdf", "Downloads/b.pdf",
        "Documents/draft-plan.txt", "Documents/scan-1.pdf", "Documents/scan-2.pdf", "Documents/tax.pdf",
        "Pictures/photo.heic", "Pictures/photo.jpg", "Pictures/vacation.heic", "Pictures/shot.png",
        "Screenshots/Screenshot 2026-10-01.png", "Screenshots/shot2.png", "Trip/trip-a.jpg", "Trip/trip-b.png",
        "Scans/receipts-1.jpg", "Scans/receipts-2.jpg", "Project/readme.md",
    ];
    for f in files {
        fs::write(home.join(f), b"data").unwrap();
    }
    let extra: Vec<PathBuf> = ["Trip", "Scans", "External", "Invoices", "Backup", "Project"].iter().map(|f| home.join(f)).collect();
    let roots = Roots::new(&home, Some(home.join("Screenshots")), &extra);
    Home { _dir: dir, roots }
}

#[derive(Clone, Debug)]
struct Seed { intent: String, command: String, expect: Value }

fn load_seeds() -> BTreeMap<String, Seed> {
    let text = fs::read_to_string(audit_file("seeds.jsonl")).expect("seeds.jsonl");
    text.lines().filter(|l| !l.trim().is_empty()).map(|line| {
        let v: Value = serde_json::from_str(line).expect("seed JSON");
        let id = v["id"].as_str().unwrap().to_string();
        (id, Seed {
            intent: v["intent"].as_str().unwrap().to_string(),
            command: v["command"].as_str().unwrap().to_string(),
            expect: canon(v["expect"].clone()),
        })
    }).collect()
}

/// Splits `id | seed | axis | phrase | expectation` rows; `#` lines are comments.
fn rows(file: &str) -> Vec<Vec<String>> {
    fs::read_to_string(audit_file(file)).expect("audit file").lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| l.split(" | ").map(|c| c.trim().to_string()).collect())
        .collect()
}

fn parse_expect(text: &str) -> Value {
    match text {
        "!" | "?" | "-" => Value::String(text.into()),
        other => canon(serde_json::from_str(other).unwrap_or_else(|e| panic!("bad expectation `{other}`: {e}"))),
    }
}

struct Tally { counts: BTreeMap<String, usize> }
impl Tally {
    fn add(&mut self, key: &str) { *self.counts.entry(key.to_string()).or_default() += 1; }
}

/// Parse-level and planner-level audit of every seed variant, negative, and ambiguous case.
#[test]
#[ignore]
fn haiku_paraphrase_audit_report() {
    let seeds = load_seeds();
    let distinct_types: std::collections::BTreeSet<&str> = seeds.values().map(|s| s.intent.as_str()).collect();
    assert_eq!(distinct_types.len(), 27, "seeds must cover all 27 intent types");

    let home = main_home();
    let mut out = String::new();
    let mut tally = Tally { counts: BTreeMap::new() };
    let mut class_totals: BTreeMap<char, usize> = BTreeMap::new();
    let mut seen_ids = std::collections::BTreeSet::new();

    // Each seed command is also run as a control row (class E). A control that
    // disagrees with its own expectation means the seed, not a variant, is suspect.
    let mut all_rows = rows("variants.txt");
    for (seed_id, seed) in &seeds {
        all_rows.push(vec![format!("E{seed_id}"), seed_id.clone(), "seed_control".into(), seed.command.clone(), "-".into()]);
    }
    for row in all_rows {
        assert_eq!(row.len(), 5, "variant row needs 5 columns: {row:?}");
        let (id, seed_id, axis, phrase, override_text) = (&row[0], &row[1], &row[2], &row[3], &row[4]);
        assert!(seen_ids.insert(id.clone()), "duplicate id {id}");
        let class = id.chars().next().unwrap();
        *class_totals.entry(class).or_default() += 1;

        let (expected, seed_label) = match class {
            'P' | 'K' | 'E' => {
                let seed = seeds.get(seed_id).unwrap_or_else(|| panic!("{id}: unknown seed {seed_id}"));
                let expected = if override_text == "-" { seed.expect.clone() } else { parse_expect(override_text) };
                (expected, seed_id.clone())
            }
            'N' => (Value::String("!".into()), "-".into()),
            'A' => (Value::String("?".into()), "-".into()),
            other => panic!("unknown class {other} in {id}"),
        };

        let parsed = parse(phrase);
        let (actual, planner, parse_ok) = match &parsed {
            Ok(intent) => {
                let v = canon(serde_json::to_value(intent).unwrap());
                let (status, _) = plan_intent(&home.roots, intent);
                (v, status, true)
            }
            Err(_) => (Value::String("!".into()), "not_planned".into(), false),
        };
        let actual_name = intent_name(&actual);

        let (category, sev, duplicate, note) = match class {
            'P' | 'K' | 'E' => {
                let dup = if class == 'K' { Some("extension-audit explicit-dot gap (docs/testing/extension-audit-baseline.md)") } else { None };
                if !parse_ok {
                    ("refused", "low", dup, "parser refused a supported-seed paraphrase")
                } else if actual == expected {
                    ("pass", "none", None, "")
                } else {
                    ("wrong_intent", wrong_severity(&actual_name, &planner), dup, "accepted with different intent or arguments")
                }
            }
            'N' => {
                if !parse_ok {
                    ("pass_refused", "none", None, "")
                } else {
                    let s = if is_mutating(&actual_name) { if planner.starts_with("steps") { "high" } else { "medium" } } else { "low" };
                    ("accepted_negative", s, None, "unsupported or unsafe phrasing was accepted")
                }
            }
            _ => (if parse_ok { "ambiguous_accepted" } else { "ambiguous_refused" }, "info", None, "recorded only; expectation is genuinely ambiguous"),
        };
        tally.add(&format!("{class}:{category}"));
        out.push_str(&json!({
            "id": id, "class": class.to_string(), "seed": seed_label, "axis": axis, "command": phrase,
            "expected": expected, "actual": actual, "parse_ok": parse_ok, "planner": planner,
            "category": category, "severity": sev, "duplicate_of": duplicate, "note": note,
        }).to_string());
        out.push('\n');
    }

    // Contextual cases go through plan_input so scope::prepare is exercised too.
    let ctx = context_outcomes();
    for c in &ctx {
        tally.add(&format!("C:{}", c["category"].as_str().unwrap()));
        *class_totals.entry('C').or_default() += 1;
        out.push_str(&c.to_string());
        out.push('\n');
    }

    let report = std::env::var_os("VOXI_PARAPHRASE_AUDIT_REPORT").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(DEFAULT_REPORT));
    fs::write(&report, out).unwrap();

    println!("paraphrase audit: seeds {} (intent types {}); rows {:?}", seeds.len(), distinct_types.len(), class_totals);
    for (k, v) in &tally.counts { println!("  {k}: {v}"); }
    println!("JSONL {}", report.display());
    drop(home);
}

fn state_for(home: &Path) -> AppState {
    AppState {
        roots: RwLock::new(Roots::new(home, None, &[])),
        history: Mutex::new(History::load(home.join("history.json"))),
        pending: Mutex::new(None),
        settings: Mutex::new(Settings::default()),
        schedules: Mutex::new(crate::scheduler::Store::load(home.join("schedules.json"))),
        execution: Mutex::new(()),
        settings_path: home.join("settings.json"),
        reports_path: home.join("reports.jsonl"),
        next_plan: AtomicU64::new(1),
        active_search: Mutex::new(None),
        presentation: Mutex::new(Default::default()),
        result_scope: Mutex::new(Default::default()),
    }
}

/// Five files created oldest to newest, one second apart, so creation order is deterministic.
fn context_home() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let home = fsx::canonical(dir.path());
    for f in ["Downloads", "Desktop", "Documents", "Pictures"] {
        fs::create_dir(home.join(f)).unwrap();
    }
    for (i, name) in ["alpha.pdf", "bravo.txt", "charlie.png", "delta.pdf", "echo.zip"].iter().enumerate() {
        if i > 0 { std::thread::sleep(Duration::from_millis(1100)); }
        fs::write(home.join("Downloads").join(name), b"data").unwrap();
    }
    (dir, home)
}

fn context_outcomes() -> Vec<Value> {
    let (_dir, home) = context_home();
    let mut results = Vec::new();
    for row in rows("contextual.txt") {
        assert_eq!(row.len(), 4, "contextual row needs 4 columns: {row:?}");
        let (id, setup, command, expected_text) = (&row[0], &row[1], &row[2], &row[3]);
        let expected = match expected_text.as_str() {
            "!" | "?" => Value::String(expected_text.clone()),
            other => serde_json::from_str::<Value>(other).unwrap_or_else(|e| panic!("{id}: bad expectation {e}")),
        };
        let state = state_for(&home);
        let mut request = 0u64;
        let mut run = |text: &str| {
            request += 1;
            plan_input(&state, text.to_string(), HashMap::new(), request)
        };
        let setup_ok = match setup.as_str() {
            "none" => true,
            "default" => matches!(run("list files in Downloads"), Outcome::Info { .. }),
            list => matches!(run(list), Outcome::Info { .. }),
        };
        let outcome = run(command);
        let (status, actions) = match &outcome {
            Outcome::Plan { total, .. } => (format!("plan:{total}"), state.pending.lock().unwrap().as_ref().map(|p| p.actions.clone())),
            Outcome::Info { total, .. } => (format!("info:{total}"), None),
            Outcome::Problem { message } => (format!("problem:{message}"), None),
            Outcome::Ambiguous { name, .. } => (format!("ambiguous:{name}"), None),
            Outcome::NotUnderstood { text } => (format!("not_understood:{text}"), None),
        };
        let mut keys: Vec<String> = actions.unwrap_or_default().iter().map(action_key).collect();
        keys.sort();
        let actual = if keys.is_empty() { Value::String("!".into()) } else { json!(keys) };
        let (category, sev) = if !setup_ok {
            ("setup_failed", "info")
        } else if expected == Value::String("?".into()) {
            ("ambiguous_recorded", "info")
        } else if expected == Value::String("!".into()) {
            if actual == Value::String("!".into()) { ("pass_refused", "none") } else { ("wrong_action", "high") }
        } else {
            let mut want: Vec<String> = expected.as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect();
            want.sort();
            if actual == json!(want) {
                ("pass", "none")
            } else if actual == Value::String("!".into()) {
                ("refused", "low")
            } else {
                ("wrong_action", "high")
            }
        };
        results.push(json!({
            "id": id, "class": "C", "seed": setup, "axis": "contextual", "command": command,
            "expected": expected, "actual": actual, "parse_ok": status != "not_understood", "planner": status,
            "category": category, "severity": sev, "duplicate_of": null,
            "note": "setup: list command published the snapshot via plan_input; followup via plan_input",
        }));
    }
    results
}
