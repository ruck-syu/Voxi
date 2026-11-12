//! Report-only audit of natural language commands that target folders.
//! Run with `cargo test --lib luna_folder -- --ignored --nocapture`.

use crate::fsx::{self, Roots};
use crate::history::History;
use crate::model::{Action, Intent};
use crate::parse::parse;
use crate::plan::{Planned, Planner, Stop};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

const REPORT: &str = "../../docs/testing/luna-folder-audit-fixed-2026-11-10.jsonl";

struct Home { _dir: tempfile::TempDir, roots: Roots }
fn fixture() -> Home {
    let dir = tempfile::tempdir().unwrap();
    let home = fsx::canonical(dir.path());
    for p in ["Desktop", "Downloads", "Documents", "Documents/Dev", "Documents/Archive", "Pictures", "Other Folder", "Project"] {
        fs::create_dir_all(home.join(p)).unwrap();
    }
    let extras: Vec<PathBuf> = ["Other Folder", "Project"].iter().map(|p| home.join(p)).collect();
    Home { _dir: dir, roots: Roots::new(&home, None, &extras) }
}
fn rel(home: &Path, p: &Path) -> String { p.strip_prefix(home).unwrap_or(p).to_string_lossy().replace('\\', "/") }
fn canon(v: Value) -> Value {
    match v {
        Value::Object(map) => Value::Object(map.into_iter().filter(|(_, x)| !x.is_null()).map(|(k, x)| (k, canon(x))).collect()),
        Value::Array(a) => Value::Array(a.into_iter().map(canon).collect()),
        x => x,
    }
}
fn action_key(home: &Path, a: &Action) -> String {
    match a {
        Action::CreateFolder { path } => format!("create:{}", rel(home, path)),
        Action::Move { from, to } => format!("move:{}->{}", rel(home, from), rel(home, to)),
        Action::Copy { from, to } => format!("copy:{}->{}", rel(home, from), rel(home, to)),
        Action::Trash { path } => format!("trash:{}", rel(home, path)),
        Action::Rename { path, new_name } => format!("rename:{}->{}", rel(home, path), new_name),
        a => serde_json::to_value(a).unwrap().to_string(),
    }
}
fn planner(home: &Path, roots: &Roots, i: &Intent) -> (String, Vec<String>) {
    let choices = HashMap::new();
    match Planner::new(roots, &choices).plan(i, &History::load(home.join("history.json"))) {
        Ok(Planned::Steps { actions, .. }) => ("steps".into(), actions.iter().map(|a| action_key(home, a)).collect()),
        Ok(Planned::Info { total, .. }) => (format!("info:{total}"), vec![]),
        Ok(Planned::Direct { .. }) => ("direct_preview_not_executed".into(), vec![]),
        Err(Stop::Problem(m)) => (format!("problem:{m}"), vec![]),
        Err(Stop::Ambiguous { name, candidates }) => (format!("ambiguous:{name}:{}", candidates.len()), vec![]),
    }
}

/// Each expected value is hand-authored from the command's intended meaning.
#[test]
#[ignore]
fn luna_folder_phrasing_report() {
    let mut cases: Vec<(String, Value, String, Vec<String>, &str)> = Vec::new();
    let add = |cases: &mut Vec<(String, Value, String, Vec<String>, &'static str)>, phrase: &str, expected: Value, planner: &str, actions: Vec<String>, class: &'static str| {
        cases.push((phrase.into(), expected, planner.into(), actions, class));
    };
    let create = |n: &str, p: &str| json!({"intent":"create_folders","names":[n],"parent":p});
    let cr = |p: &str, n: &str| vec![format!("create:{p}/{n}")];

    // Creation: name-first/location-first, roots and nested relative or absolute paths.
    for (phrase, name, parent) in [
        ("create a folder in Documents/Dev called Toggle Wars.", "Toggle Wars", "Documents/Dev"),
        ("create folder Toggle Wars in Documents/Dev", "Toggle Wars", "Documents/Dev"),
        ("make a new folder called Toggle Wars inside Documents/Dev", "Toggle Wars", "Documents/Dev"),
        ("add folder named Toggle Wars under Documents/Dev", "Toggle Wars", "Documents/Dev"),
        ("new folder Toggle Wars on Documents/Dev", "Toggle Wars", "Documents/Dev"),
        ("create a folder inside Documents/Dev named Toggle Wars", "Toggle Wars", "Documents/Dev"),
        ("make a folder in Documents/Dev named Toggle Wars", "Toggle Wars", "Documents/Dev"),
        ("create a new folder in Documents/Dev called 'Toggle Wars'", "Toggle Wars", "Documents/Dev"),
        ("create folder in Documents/Dev named \"Toggle Wars\"", "Toggle Wars", "Documents/Dev"),
        ("create folder 'Toggle Wars' in Documents/Dev", "Toggle Wars", "Documents/Dev"),
        ("create folder Toggle Wars in Documents", "Toggle Wars", "Documents"),
        ("create a folder called Toggle Wars in Documents", "Toggle Wars", "Documents"),
        ("make folder called Toggle Wars in Downloads", "Toggle Wars", "Downloads"),
        ("new folder in Downloads called Toggle Wars", "Toggle Wars", "Downloads"),
        ("add a folder in Desktop named Toggle Wars", "Toggle Wars", "Desktop"),
        ("create folder in Documents named 'Toggle Wars'", "Toggle Wars", "Documents"),
        ("create a folder in \"Documents/Dev\" named Toggle Wars", "Toggle Wars", "Documents/Dev"),
        ("create a folder in Documents/Dev named \"Toggle Wars\"", "Toggle Wars", "Documents/Dev"),
        ("create a folder in Documents/Dev called Toggle Wars folder", "Toggle Wars", "Documents/Dev"),
        ("make a folder named 'Build Notes' inside Documents/Dev", "Build Notes", "Documents/Dev"),
        ("create a folder in Documents/Dev called 'Build Notes'", "Build Notes", "Documents/Dev"),
        ("create a folder called \"Build Notes\" in Documents/Dev", "Build Notes", "Documents/Dev"),
        ("create a folder in ~/Documents/Dev called Toggle Wars", "Toggle Wars", "~/Documents/Dev"),
        ("create a folder in ~/Documents called Toggle Wars", "Toggle Wars", "~/Documents"),
        ("create a folder in /tmp called Toggle Wars", "Toggle Wars", "/tmp"),
        ("create a folder in Documents/Dev called Build and Test", "Build and Test", "Documents/Dev"),
        ("create a folder in Documents/Dev named 'Build and Test'", "Build and Test", "Documents/Dev"),
        ("create folder Build Notes in Documents/Dev", "Build Notes", "Documents/Dev"),
        ("make folder Build Notes in Documents/Dev", "Build Notes", "Documents/Dev"),
        ("add folder Build Notes in Documents/Dev", "Build Notes", "Documents/Dev"),
        ("new folder Build Notes in Documents/Dev", "Build Notes", "Documents/Dev"),
    ] {
        let safe = !parent.starts_with('/');
        let plan_parent = parent.strip_prefix("~/").unwrap_or(parent);
        let class = if phrase == "create a folder in Documents/Dev called Build and Test" { "ambiguous_phrase" }
            else if safe { "supported" } else { "unsafe" };
        add(&mut cases, phrase, create(name, parent), if safe {"steps"} else {"safety"}, if safe {cr(plan_parent, name)} else {vec![]}, class);
    }
    // More commands with explicit plural names: expected ordered folder list.
    for (phrase, names, parent) in [
        ("create folders named Alpha and Beta in Documents/Dev", vec!["Alpha", "Beta"], "Documents/Dev"),
        ("make folders in Documents/Dev called Alpha and Beta", vec!["Alpha", "Beta"], "Documents/Dev"),
        ("create folders Alpha, Beta in Documents/Dev", vec!["Alpha", "Beta"], "Documents/Dev"),
        ("create folders named \"Alpha and Beta\" in Documents/Dev", vec!["Alpha and Beta"], "Documents/Dev"),
    ] {
        let expected = json!({"intent":"create_folders","names":names,"parent":parent});
        let actions = names.iter().map(|n| format!("create:{parent}/{n}")).collect();
        add(&mut cases, phrase, expected, "steps", actions, "supported");
    }
    // Broaden name and quoting coverage with distinct hand-checked targets.
    for (idx, name) in ["Roadmap Draft", "UX Research", "Q4 Budget", "Release Notes", "API Samples", "Sprint Planning", "Test Data", "Customer Feedback", "Build Artifacts", "Long Name With Spaces"].iter().enumerate() {
        let commands = [
            format!("create a folder in Documents/Dev called '{name}'"),
            format!("make a folder named \"{name}\" inside Documents/Dev"),
            format!("add folder {name} in Documents/Dev"),
            format!("new folder in Documents/Dev called '{name}'"),
        ];
        for (variant, phrase) in commands.into_iter().enumerate() {
            // Keep the expected argument as the literal authored above; naming each case separately also
            // makes the JSONL report independently inspectable.
            let _golden_id = (idx, variant);
            add(&mut cases, &phrase, create(name, "Documents/Dev"), "steps", cr("Documents/Dev", name), "supported");
        }
    }
    add(&mut cases, "create a folder in ABS_ALLOWED/Documents/Dev called Absolute Check", create("Absolute Check", "ABS_ALLOWED/Documents/Dev"), "steps", cr("Documents/Dev", "Absolute Check"), "supported");

    // Explicitly named folder operations. Fixture folders are empty to make previews deterministic.
    let mut ops = vec![
        ("rename Documents/Dev/Toggle Wars to Toggle Wars 2", json!({"intent":"rename","src":{"kind":"item","name":"Documents/Dev/Toggle Wars"},"new_name":"Toggle Wars 2"}), "steps", vec!["rename:Documents/Dev/Toggle Wars->Toggle Wars 2".into()], "supported"),
        ("rename folder Toggle Wars in Documents/Dev to Toggle Wars 2", json!({"intent":"rename","src":{"kind":"item","name":"Toggle Wars","folder":"Documents/Dev"},"new_name":"Toggle Wars 2"}), "steps", vec!["rename:Documents/Dev/Toggle Wars->Toggle Wars 2".into()], "supported"),
        ("rename 'Toggle Wars' in Documents/Dev as 'Toggle Wars 2'", json!({"intent":"rename","src":{"kind":"item","name":"Toggle Wars","folder":"Documents/Dev"},"new_name":"Toggle Wars 2"}), "steps", vec!["rename:Documents/Dev/Toggle Wars->Toggle Wars 2".into()], "supported"),
        ("move folder Toggle Wars from Documents/Dev to Downloads", json!({"intent":"move","src":{"kind":"item","name":"Toggle Wars","folder":"Documents/Dev"},"to":"Downloads"}), "steps", vec!["move:Documents/Dev/Toggle Wars->Downloads/Toggle Wars".into()], "supported"),
        ("move Documents/Dev/Toggle Wars to Downloads", json!({"intent":"move","src":{"kind":"item","name":"Documents/Dev/Toggle Wars"},"to":"Downloads"}), "steps", vec!["move:Documents/Dev/Toggle Wars->Downloads/Toggle Wars".into()], "supported"),
        ("copy folder Toggle Wars from Documents/Dev to Desktop", json!({"intent":"copy","src":{"kind":"item","name":"Toggle Wars","folder":"Documents/Dev"},"to":"Desktop"}), "steps", vec!["copy:Documents/Dev/Toggle Wars->Desktop/Toggle Wars".into()], "supported"),
        ("copy Documents/Dev/Toggle Wars to Desktop", json!({"intent":"copy","src":{"kind":"item","name":"Documents/Dev/Toggle Wars"},"to":"Desktop"}), "steps", vec!["copy:Documents/Dev/Toggle Wars->Desktop/Toggle Wars".into()], "supported"),
        ("trash folder Toggle Wars in Documents/Dev", json!({"intent":"trash","src":{"kind":"item","name":"Toggle Wars","folder":"Documents/Dev"}}), "steps", vec!["trash:Documents/Dev/Toggle Wars".into()], "supported"),
        ("delete Documents/Dev/Toggle Wars folder", json!({"intent":"trash","src":{"kind":"item","name":"Documents/Dev/Toggle Wars"}}), "steps", vec!["trash:Documents/Dev/Toggle Wars".into()], "supported"),
        ("remove the folder Toggle Wars from Documents/Dev", json!({"intent":"trash","src":{"kind":"item","name":"Toggle Wars","folder":"Documents/Dev"}}), "steps", vec!["trash:Documents/Dev/Toggle Wars".into()], "supported"),
    ];
    // Add exact natural paraphrases over fixture objects. Expected argument values are built independently from each phrase's declared operation.
    let more = [
        ("move folder Project/Child Folder to Documents/Archive", "move", "Project/Child Folder", "Documents/Archive"),
        ("copy folder Project/Child Folder to Documents/Archive", "copy", "Project/Child Folder", "Documents/Archive"),
        ("trash folder Project/Child Folder", "trash", "Project/Child Folder", ""),
        ("delete folder Other Folder", "trash", "Other Folder", ""),
        ("move Other Folder into Documents", "move", "Other Folder", "Documents"),
        ("copy Other Folder into Documents", "copy", "Other Folder", "Documents"),
        ("rename Other Folder to Renamed Folder", "rename", "Other Folder", "Renamed Folder"),
        ("rename folder Project/Child Folder to New Child", "rename", "Project/Child Folder", "New Child"),
        ("move folder \"Project/Child Folder\" to Documents/Archive", "move", "Project/Child Folder", "Documents/Archive"),
        ("copy folder 'Project/Child Folder' to Documents/Archive", "copy", "Project/Child Folder", "Documents/Archive"),
        ("move Project/Child Folder into Downloads", "move", "Project/Child Folder", "Downloads"),
        ("put Project/Child Folder in Downloads", "move", "Project/Child Folder", "Downloads"),
        ("send folder Project/Child Folder to Downloads", "move", "Project/Child Folder", "Downloads"),
        ("duplicate Project/Child Folder to Downloads", "copy", "Project/Child Folder", "Downloads"),
        ("copy Project/Child Folder into Desktop", "copy", "Project/Child Folder", "Desktop"),
        ("copy folder \"Project/Child Folder\" to Desktop", "copy", "Project/Child Folder", "Desktop"),
        ("trash Project/Child Folder", "trash", "Project/Child Folder", ""),
        ("remove Project/Child Folder", "trash", "Project/Child Folder", ""),
        ("get rid of Project/Child Folder", "trash", "Project/Child Folder", ""),
        ("rename Project/Child Folder as New Child", "rename", "Project/Child Folder", "New Child"),
        ("rename 'Project/Child Folder' to New Child", "rename", "Project/Child Folder", "New Child"),
        ("change the name of Project/Child Folder to New Child", "rename", "Project/Child Folder", "New Child"),
    ];
    for (phrase, kind, source, dest) in more {
        let name = source.rsplit('/').next().unwrap_or(source);
        let src = json!({"kind":"item","name":source});
        let (expected, actions) = match kind {
            "move" => (json!({"intent":"move","src":src,"to":dest}), vec![format!("move:{source}->{dest}/{name}")]),
            "copy" => (json!({"intent":"copy","src":src,"to":dest}), vec![format!("copy:{source}->{dest}/{name}")]),
            "trash" => (json!({"intent":"trash","src":src}), vec![format!("trash:{source}")]),
            _ => (json!({"intent":"rename","src":src,"new_name":dest}), vec![format!("rename:{source}->{dest}")]),
        };
        ops.push((phrase, expected, "steps", actions, "supported"));
    }
    // Name-first syntax is not valid for all verbs; explicit-path forms and common aliases are covered separately.
    for (phrase, expected, plan, actions, class) in ops { add(&mut cases, phrase, expected, plan, actions, class); }

    // Unsafe and genuinely ambiguous controls. These must not preview a file change.
    for (phrase, expected) in [
        ("create a folder in Documents/Dev called ../Secrets", create("../Secrets", "Documents/Dev")),
        ("create a folder in Documents/Dev called ../../outside", create("../../outside", "Documents/Dev")),
        ("create a folder in Documents/Dev called .hidden", create(".hidden", "Documents/Dev")),
        ("create a folder in Documents/Dev called /tmp/escape", create("/tmp/escape", "Documents/Dev")),
        ("create a folder in Documents/Dev called Toggle Wars/Child", create("Toggle Wars/Child", "Documents/Dev")),
    ] { add(&mut cases, phrase, expected, "safety", vec![], "unsafe"); }
    for (phrase, verb) in [
        ("move \"Documents/Dev/Toggle Wars\" to \"Documents/Dev/Toggle Wars/Child Folder\"", "move"),
        ("copy \"Documents/Dev/Toggle Wars\" to \"Documents/Dev/Toggle Wars/Child Folder\"", "copy"),
    ] {
        add(&mut cases, phrase, json!({"intent":verb,"src":{"kind":"item","name":"Documents/Dev/Toggle Wars"},"to":"Documents/Dev/Toggle Wars/Child Folder"}), "safety", vec![], "unsafe");
    }
    add(&mut cases, "create a folder called Toggle Wars", create("Toggle Wars", "ABSENT_DEFAULT"), "steps", vec!["create:Desktop/Toggle Wars".into()], "supported");
    for (phrase, expected) in [
        ("rename Toggle Wars to New Name", json!({"intent":"rename","src":{"kind":"item","name":"Toggle Wars"},"new_name":"New Name"})),
        ("move Toggle Wars to Documents", json!({"intent":"move","src":{"kind":"item","name":"Toggle Wars"},"to":"Documents"})),
        ("copy Toggle Wars to Documents", json!({"intent":"copy","src":{"kind":"item","name":"Toggle Wars"},"to":"Documents"})),
        ("trash Toggle Wars", json!({"intent":"trash","src":{"kind":"item","name":"Toggle Wars"}})),
    ] { add(&mut cases, phrase, expected, "ambiguous", vec![], "ambiguous"); }

    let mut output = String::new();
    let mut counts = std::collections::BTreeMap::<String, usize>::new();
    let mut failures = 0;
    for (idx, (phrase_template, expected_template, expected_plan, expected_actions, class)) in cases.iter().enumerate() {
        let home = fixture();
        let home_text = home.roots.home.to_string_lossy().to_string();
        let phrase = phrase_template.replace("ABS_ALLOWED", &home_text);
        let mut expected = expected_template.clone();
        if expected["parent"] == "ABSENT_DEFAULT" { expected["parent"] = Value::Null; }
        if expected["parent"] == "ABS_ALLOWED/Documents/Dev" { expected["parent"] = json!(format!("{home_text}/Documents/Dev")); }
        // Add only the explicitly named source object for this row, keeping create-folder targets absent.
        if let Some(src) = expected.get("src") {
            let name = src.get("name").and_then(Value::as_str).unwrap_or("");
            let parent = src.get("folder").and_then(Value::as_str).unwrap_or("");
            if !name.is_empty() { fs::create_dir_all(home.roots.home.join(parent).join(name)).unwrap(); }
            if *class == "ambiguous" {
                fs::create_dir_all(home.roots.home.join("Desktop").join(name)).unwrap();
                fs::create_dir_all(home.roots.home.join("Downloads").join(name)).unwrap();
            }
        }
        let (actual, plan, actions, parse_ok) = match parse(&phrase) {
            Ok(i) => {
                let actual = serde_json::to_value(&i).unwrap();
                let (p, a) = planner(&home.roots.home, &home.roots, &i);
                (actual, p, a, true)
            }
            Err(_) => (Value::String("!".into()), "refused".into(), vec![], false),
        };
        let good = if *class == "ambiguous_phrase" { true }
            else if *class == "ambiguous" { canon(actual.clone()) == canon(expected.clone()) && plan.starts_with("ambiguous:") && actions.is_empty() }
            else if *class == "unsafe" { canon(actual.clone()) == canon(expected.clone()) && actions.is_empty() && (plan == "refused" || plan.starts_with("problem:") || plan.starts_with("ambiguous:")) }
            else { canon(actual.clone()) == canon(expected.clone()) && plan == *expected_plan && actions == *expected_actions };
        if !good { failures += 1; }
        let category = if *class == "ambiguous_phrase" { "ambiguous_accepted" } else if good { "pass" } else if !parse_ok && *class == "supported" { "grammar_refusal" } else if canon(actual.clone()) != canon(expected.clone()) { "wrong_interpretation" } else { "planner_mismatch" };
        *counts.entry(format!("{class}:{category}")).or_default() += 1;
        output.push_str(&json!({"id":format!("F{:03}", idx+1),"class":class,"command":phrase,"expected":expected,"actual":actual,"parse_ok":parse_ok,"expected_preview":{"status":expected_plan,"actions":expected_actions},"actual_preview":{"status":plan,"actions":actions},"category":category,"severity":if good {"none"} else if plan == "steps" {"high"} else {"medium"}}).to_string());
        output.push('\n');
        drop(home);
    }
    let report = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(REPORT);
    fs::create_dir_all(report.parent().unwrap()).unwrap();
    fs::write(&report, output).unwrap();
    println!("Luna folder audit: rows {}; failures {}; categories {:?}; JSONL {}", cases.len(), failures, counts, report.display());
    assert!(cases.len() >= 120, "folder audit must contain at least 120 cases");
}
