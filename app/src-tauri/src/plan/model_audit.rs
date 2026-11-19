//! Planner-boundary evidence for the preserved generated error audit. No actions execute.
use super::*;
use serde_json::json;

fn fixture() -> (tempfile::TempDir, Roots) {
    let dir = tempfile::tempdir().unwrap();
    let home = fsx::canonical(dir.path());
    for name in [
        "Desktop",
        "Documents",
        "Downloads",
        "Pictures",
        "Library/cache",
        "Library/Preferences",
        "Library/Mail",
        ".ssh",
        ".config",
        "Applications/Adobe",
        "Downloads/Archive",
        "Downloads/Backup",
        "Downloads/photo library",
        "Downloads/Keychains",
        "Documents/projects/.git",
        "Downloads/.hidden",
    ] {
        fs::create_dir_all(home.join(name)).unwrap();
    }
    for name in [
        ".env",
        ".bash_history",
        "Documents/.DS_Store",
        "Documents/.localized",
        "Downloads/.env",
        "Downloads/.hidden/secret.pdf",
        "Downloads/report.pdf",
        "Downloads/hidden-public.pdf",
        "Library/cache/private.db",
    ] {
        fs::write(home.join(name), b"fixture").unwrap();
    }
    let roots = Roots::new(&home, None, &[]);
    (dir, roots)
}

#[test]
fn model_audit_scope_hidden_and_local_folder_boundaries() {
    let (_dir, roots) = fixture();
    let choices = HashMap::new();
    let history = History::load(roots.home.join("history.json"));
    for command in
        ["delete ~/Library/cache", "copy ~/.ssh to Documents", "list files in /System", "move Downloads/../../Library/cache to Documents"]
    {
        let intent = crate::parse::parse(command).unwrap();
        assert!(Planner::new(&roots, &choices).plan(&intent, &history).is_err(), "{command}");
    }
    // Local names are not blacklisted merely because they resemble protected
    // folders or online services. The concrete local path is what matters.
    for folder in ["Library", "Dropbox", "Priya", "Google Photos"] {
        fs::create_dir_all(roots.home.join("Documents").join(folder)).unwrap();
        let intent = crate::parse::parse(&format!("copy report.pdf to \"{folder}\"")).unwrap();
        let Planned::Steps { actions, .. } = Planner::new(&roots, &choices).plan(&intent, &history).unwrap() else {
            panic!("expected preview");
        };
        assert!(actions.iter().all(|a| matches!(a, Action::Copy { to, .. } if to.starts_with(roots.home.join("Documents")))));
    }
    let intent = crate::parse::parse("list files in Downloads").unwrap();
    let Planned::Info { listed: Some((paths, _)), .. } = Planner::new(&roots, &choices).plan(&intent, &history).unwrap() else {
        panic!("expected listing");
    };
    assert!(!paths.is_empty());
    assert!(paths.iter().all(|p| !p.to_string_lossy().contains(".hidden") && fsx::file_name(p) != ".env"));
    // Explicit in-root hidden paths are allowed by the current product contract;
    // only discovery skips them. This differs from home/outside-root access.
    let intent = crate::parse::parse("copy Downloads/.env to Documents").unwrap();
    assert!(Planner::new(&roots, &choices).plan(&intent, &history).is_ok());
}

#[test]
#[ignore]
fn model_audit_all_flagged_planner_cases() {
    let input = std::env::var("VOXI_PLANNER_AUDIT_INPUT").expect("VOXI_PLANNER_AUDIT_INPUT");
    let output = std::env::var("VOXI_PLANNER_AUDIT_OUTPUT").expect("VOXI_PLANNER_AUDIT_OUTPUT");
    let mut records = Vec::new();
    for line in fs::read_to_string(input).unwrap().lines() {
        let mut row: serde_json::Value = serde_json::from_str(line).unwrap();
        if row["category"] != "review_planner_scope_or_hidden_path" {
            continue;
        }
        let (_dir, roots) = fixture();
        let choices = HashMap::new();
        let history = History::load(roots.home.join("history.json"));
        let result = match crate::parse::parse(row["text"].as_str().unwrap()) {
            Err(_) => json!({"status":"parser_refused"}),
            Ok(intent) => match Planner::new(&roots, &choices).plan(&intent, &history) {
                Err(Stop::Problem(message)) => json!({"status":"planner_refused", "message":message}),
                Err(Stop::Ambiguous { name, candidates }) => json!({"status":"planner_ambiguous", "name":name, "candidates":candidates}),
                Ok(Planned::Steps { actions, title, .. }) => json!({"status":"preview_only", "title":title, "actions":actions}),
                Ok(Planned::Info { title, summary, total, .. }) => {
                    json!({"status":"read_only", "title":title, "summary":summary, "total":total})
                }
                Ok(Planned::Direct { action, title }) => json!({"status":"direct_plan_only", "title":title, "action":action}),
            },
        };
        row["planner_fixture"] = result;
        row["fixture_note"] = json!(
            "Temporary home with real outside-root Library/Applications/hidden sentinels and allowed local names; planning only, no actions executed. Absence in this fixture is not proof of a universal restriction."
        );
        records.push(row);
    }
    assert_eq!(records.len(), 67, "all preserved flagged cases must be checked");
    fs::write(output, records.iter().map(|r| format!("{r}\n")).collect::<String>()).unwrap();
}
