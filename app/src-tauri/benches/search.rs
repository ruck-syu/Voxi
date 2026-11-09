//! Repeatable search baseline using the production parser, walker, and planner.
#![allow(dead_code)]
#[path = "../src/fsx.rs"] mod fsx;
#[path = "../src/history.rs"] mod history;
#[path = "../src/model.rs"] mod model;
#[path = "../src/parse/mod.rs"] mod parse;
#[path = "../src/pdf.rs"] mod pdf;
#[path = "../src/plan.rs"] mod plan;
#[path = "../src/search.rs"] mod search;

use std::{collections::HashMap, fs, hint::black_box, path::Path, time::Instant};
use serde_json::{Value, json};

fn fixture(home: &Path, files: usize, nested: bool) {
    for folder in ["Downloads", "Desktop", "Documents", "Pictures"] {
        fs::create_dir_all(home.join(folder)).unwrap();
    }
    for i in 0..files {
        let dir = if nested {
            home.join(format!("Downloads/group_{:02}/batch_{:02}", i / 1000, (i / 100) % 10))
        } else { home.join("Downloads") };
        if i % 100 == 0 { fs::create_dir_all(&dir).unwrap(); }
        let name = if i % 100 == 0 { format!("AEROGUARD_{i:06}.pdf") } else { format!("file_{i:06}.txt") };
        fs::File::create(dir.join(name)).unwrap();
    }
}

fn measure(mut f: impl FnMut() -> (usize, String), expected: usize, warmups: usize, repeats: usize) -> Value {
    for _ in 0..warmups { black_box(f()); }
    let mut samples = Vec::new();
    let mut counts = Vec::new();
    let mut outcomes = Vec::new();
    for _ in 0..repeats {
        let start = Instant::now();
        let (count, outcome) = black_box(f());
        samples.push(start.elapsed().as_secs_f64() * 1000.0);
        counts.push(count);
        outcomes.push(outcome);
    }
    json!({"samples_ms": samples, "expected_matches": expected, "observed_matches": counts, "outcomes": outcomes})
}

fn main() {
    let repeats = 15;
    let warmups = 2;
    let mut results = Vec::new();
    for (name, files, nested) in [
        ("flat-1k", 1000, false), ("flat-10k", 10000, false),
        ("nested-10k", 10000, true), ("nested-40k", 40000, true),
        ("budget-60k", 60000, true),
    ] {
        eprintln!("Benchmarking {name} ({files} files)");
        let temp = tempfile::tempdir().unwrap();
        let home = fs::canonicalize(temp.path()).unwrap();
        fixture(&home, files, nested);
        let roots = fsx::Roots::new(&home, None, &[]);
        let history = history::History::load(home.join("history.json"));
        let choices = HashMap::new();
        let dl = roots.downloads();
        let walk = measure(|| {
            let mut count = 0;
            plan::walk(&dl, 12, &mut |_, dir| if !dir { count += 1 });
            (count, "walk".into())
        }, files, warmups, repeats);
        results.push(json!({"fixture": name, "files": files, "operation": "walk", "measurement": walk}));
        for (op, command, expected) in [
            ("group", "Delete all the Aeroguard files from Downloads".to_string(), files / 100),
            ("exact", format!("delete file_{:06}.txt from Downloads", files - 1), 1),
            ("normalized", format!("delete file {:06} txt from Downloads", files - 1), 1),
            ("missing", "delete BenchmarkAbsentNeedle987.txt from Downloads".to_string(), 0),
        ] {
            let measurement = measure(|| {
                // Include parser + complete planning; never call the executor.
                let intent = parse::parse(black_box(&command)).unwrap();
                match plan::Planner::new(&roots, &choices).plan(&intent, &history) {
                    Ok(plan::Planned::Steps { actions, .. }) => (actions.len(), "plan".into()),
                    Err(plan::Stop::Ambiguous { candidates, .. }) => (candidates.len(), "ambiguous".into()),
                    Err(plan::Stop::Problem(message)) => (0, message),
                    Ok(_) => (0, "other".into()),
                }
            }, expected, warmups, repeats);
            results.push(json!({"fixture": name, "files": files, "operation": op, "command": command, "measurement": measurement}));
        }
    }
    // A target beyond the depth cap: both group search and raw enumeration miss it.
    let temp = tempfile::tempdir().unwrap();
    let home = fs::canonicalize(temp.path()).unwrap();
    fixture(&home, 0, false);
    let deep = home.join("Downloads").join((0..14).map(|i| format!("d{i}")).collect::<Vec<_>>().join("/"));
    fs::create_dir_all(&deep).unwrap();
    fs::File::create(deep.join("AEROGUARD_deep.pdf")).unwrap();
    let roots = fsx::Roots::new(&home, None, &[]);
    let history = history::History::load(home.join("history.json"));
    let choices = HashMap::new();
    let measurement = measure(|| {
        let intent = parse::parse("delete Aeroguard files from Downloads").unwrap();
        match plan::Planner::new(&roots, &choices).plan(&intent, &history) {
            Ok(plan::Planned::Steps { actions, .. }) => (actions.len(), "plan".into()),
            Err(plan::Stop::Problem(message)) => (0, message),
            _ => (0, "other".into()),
        }
    }, 1, warmups, repeats);
    results.push(json!({"fixture": "depth-14", "files": 1, "operation": "group", "measurement": measurement}));
    let output = std::env::var("VOXI_BENCH_OUTPUT").expect("set VOXI_BENCH_OUTPUT");
    fs::write(output, serde_json::to_string_pretty(&json!({
        "warmups": warmups, "repeats": repeats, "results": results
    })).unwrap()).unwrap();
}
