use super::*;
use crate::parse::parse;
use std::fs;

#[path = "messy_tests.rs"]
mod messy;

struct Home {
    _dir: tempfile::TempDir,
    roots: Roots,
}

fn home() -> Home {
    let dir = tempfile::tempdir().unwrap();
    let h = fsx::canonical(dir.path());
    for d in ["Desktop", "Downloads", "Documents", "Pictures"] {
        fs::create_dir(h.join(d)).unwrap();
    }
    let roots = Roots::new(&h, None, &[]);
    Home { _dir: dir, roots }
}

#[test]
fn folder_nicknames_resolve_and_stay_inside_allowed_roots() {
    let mut h = home();
    let dev = h.roots.home.join("Documents/Dev");
    let other = h.roots.home.join("Desktop/Dev");
    fs::create_dir_all(&dev).unwrap();
    fs::create_dir_all(&other).unwrap();
    fs::write(h.roots.home.join("Downloads/report.pdf"), b"x").unwrap();
    // Without a nickname, two folders called Dev need a choice.
    assert!(matches!(plan(&h, "move report.pdf to dev"), Err(Stop::Ambiguous { .. })));
    let outside = tempfile::tempdir().unwrap();
    h.roots = h.roots.clone().with_nicknames(vec![("dev".into(), dev.clone()), ("out".into(), outside.path().to_path_buf())]);
    for cmd in ["move report.pdf to dev", "move report.pdf to the dev folder", "move report.pdf into my Dev folder"] {
        let actions = steps(&h, cmd);
        assert!(matches!(&actions[..], [Action::Move { to, .. }] if to.starts_with(&dev)), "{cmd}: {actions:?}");
    }
    assert!(h.roots.named("out").is_none(), "a nickname can't point outside the allowed folders");
}

fn plan(h: &Home, cmd: &str) -> Result<Planned, Stop> {
    let choices = HashMap::new();
    let history = History::load(h.roots.home.join("history.json"));
    Planner::new(&h.roots, &choices).plan(&parse(cmd).unwrap(), &history)
}

fn steps(h: &Home, cmd: &str) -> Vec<Action> {
    match plan(h, cmd) {
        Ok(Planned::Steps { actions, .. }) => actions,
        Ok(_) => panic!("expected steps"),
        Err(e) => panic!("{e:?}"),
    }
}

fn plan_with_limit(h: &Home, cmd: &str, limit: usize) -> Result<Planned, Stop> {
    let choices = HashMap::new();
    let history = History::load(h.roots.home.join("history.json"));
    let mut planner = Planner::new(&h.roots, &choices);
    planner.search.limit = limit;
    planner.plan(&parse(cmd).unwrap(), &history)
}

#[test]
fn opening_an_allowed_root_folder_plans_a_finder_reveal() {
    let h = home();
    let Ok(Planned::Direct { action, .. }) = plan(&h, "open the Documents folder") else { panic!("expected a direct reveal") };
    assert_eq!(action, Action::OpenInFinder { path: h.roots.home.join("Documents"), reveal: false });
}

#[test]
fn symlinked_configured_roots_plan_like_other_folders() {
    let mut h = home();
    let external = tempfile::tempdir().unwrap();
    let downloads = h.roots.downloads();
    let projects = h.roots.home.join("Projects");
    fs::remove_dir(&downloads).unwrap();
    for (name, link) in [("Downloads", &downloads), ("Projects", &projects)] {
        let target = external.path().join(name);
        fs::create_dir(&target).unwrap();
        fs::write(target.join("bill.pdf"), b"data").unwrap();
        std::os::unix::fs::symlink(&target, link).unwrap();
    }
    h.roots = Roots::new(&h.roots.home, None, std::slice::from_ref(&projects));

    for (name, root) in [("Downloads", downloads), ("Projects", projects)] {
        let target = fsx::canonical(&root);
        assert!(matches!(plan(&h, &format!("open {name}")),
            Ok(Planned::Direct { action: Action::OpenInFinder { path, reveal: false }, .. }) if path == root));
        assert!(matches!(plan(&h, &format!("open \"{}\"", target.display())),
            Ok(Planned::Direct { action: Action::OpenInFinder { path, reveal: false }, .. }) if path == target));
        let settings = crate::learn::add_nickname(&crate::settings::Settings::default(), "offsite vault", &target, &h.roots.home).unwrap();
        let nicknames = settings.nickname_paths(&h.roots.home);
        h.roots = h.roots.with_nicknames(nicknames);
        assert!(matches!(plan(&h, "open offsite vault"),
            Ok(Planned::Direct { action: Action::OpenInFinder { path, reveal: false }, .. }) if path == target));
        let choices = HashMap::from([("picked vault".into(), target.clone())]);
        let history = History::load(h.roots.home.join("history.json"));
        assert!(matches!(Planner::new(&h.roots, &choices).plan(&parse("list files in picked vault").unwrap(), &history),
            Ok(Planned::Info { total: 1, rows, .. }) if rows[0].name == "bill.pdf"));
        assert_eq!(Planner::new(&h.roots, &choices).resolve_item(&format!(" {}", target.display()), None).unwrap(), target);
        for spelling in [&root, &target] {
            let path = spelling.display();
            for (command, refusal) in [
                (format!("delete \"{path}\""), "won't remove"),
                (format!("move \"{path}\" to Desktop"), "won't move an allowed root"),
                (format!("rename \"{path}\" to Archived Files"), "won't rename an allowed root"),
                (format!("copy \"{path}\" to Desktop"), "won't copy an allowed root"),
            ] {
                let result = plan(&h, &command);
                assert!(matches!(&result, Err(Stop::Problem(message)) if message.contains(refusal)), "{command}: {:?}", result.err());
            }
        }
        let list = if name == "Downloads" { "list Downloads" } else { "list files in Projects" };
        assert!(matches!(plan(&h, list),
            Ok(Planned::Info { total: 1, rows, .. }) if rows[0].name == "bill.pdf"));
        assert!(matches!(plan(&h, &format!("size of {name}")),
            Ok(Planned::Info { total: 1, title, .. }) if title == format!("{name} uses 4 B")));
        assert_eq!(
            steps(&h, &format!("sort {name} by type")),
            [
                Action::CreateFolder { path: root.join("PDFs") },
                Action::Move { from: root.join("bill.pdf"), to: root.join("PDFs/bill.pdf") },
            ]
        );
        assert_eq!(steps(&h, &format!("create a folder in {name} named X")), [Action::CreateFolder { path: root.join("X") }]);
        assert_eq!(
            steps(&h, &format!("move PDFs from {name} to Desktop")),
            [Action::Move { from: root.join("bill.pdf"), to: h.roots.desktop().join("bill.pdf") }]
        );
    }
}

#[test]
fn screenshots_at_symlinked_desktop_do_not_duplicate_the_root() {
    let mut h = home();
    let desktop = h.roots.desktop();
    let external = tempfile::tempdir().unwrap();
    let target = fsx::canonical(external.path());
    fs::remove_dir(&desktop).unwrap();
    std::os::unix::fs::symlink(&target, &desktop).unwrap();
    fs::write(desktop.join("tax.pdf"), b"data").unwrap();
    h.roots = Roots::new(&h.roots.home, Some(desktop.clone()), &[]);

    assert_eq!(h.roots.allowed.iter().filter(|p| **p == target).count(), 1);
    assert!(matches!(plan(&h, "open Desktop"),
        Ok(Planned::Direct { action: Action::OpenInFinder { path, reveal: false }, .. }) if path == desktop));
    assert!(matches!(plan(&h, "list files in Desktop"), Ok(Planned::Info { total: 1, rows, .. }) if rows[0].name == "tax.pdf"));
    assert_eq!(steps(&h, "delete tax.pdf"), [Action::Trash { path: desktop.join("tax.pdf") }]);
}

#[test]
fn unconfigured_links_to_a_root_do_not_work_as_folder_arguments() {
    let mut h = home();
    let documents = h.roots.home.join("Documents");
    let private = documents.join("Private");
    let outside = tempfile::tempdir().unwrap();
    fs::create_dir(&private).unwrap();
    h.roots = h.roots.with_exclusions(std::slice::from_ref(&private));

    for parent in [&private, outside.path()] {
        let link = parent.join("docs-link");
        std::os::unix::fs::symlink(&documents, &link).unwrap();
        let folder = link.display();
        for cmd in [
            format!("open \"{folder}\""),
            format!("list files in \"{folder}\""),
            format!("sort \"{folder}\" by type"),
            format!("size of \"{folder}\""),
            format!("create a folder in \"{folder}\" named X"),
            format!("move PDFs from \"{folder}\" to Desktop"),
        ] {
            assert!(matches!(plan(&h, &cmd), Err(Stop::Problem(message)) if message.contains("outside the folders")), "{cmd}");
        }
    }
}

#[test]
fn quoted_paths_with_leading_spaces_do_not_use_spoken_folder_aliases() {
    let h = home();
    for cmd in [r#"open " /documents""#, r#"size of " /documents""#] {
        assert!(matches!(plan(&h, cmd), Err(Stop::Problem(message)) if message.contains("outside the folders")), "{cmd}");
    }
    assert!(matches!(plan(&h, r#"open " ~/Documents""#),
        Ok(Planned::Direct { action: Action::OpenInFinder { path, .. }, .. }) if path == h.roots.home.join("Documents")));
}

#[test]
fn copying_to_another_spelling_of_the_source_folder_plans_a_duplicate() {
    let h = home();
    let docs = h.roots.home.join("Documents");
    let alias = docs.join("Alias");
    std::os::unix::fs::symlink(&docs, &alias).unwrap();
    let bill = docs.join("bill.pdf");
    fs::write(&bill, b"x").unwrap();
    assert_eq!(
        steps(&h, "copy PDFs from Documents to ~/Documents/Alias"),
        [Action::Copy { from: bill.clone(), to: alias.join("bill 2.pdf") }]
    );
    let typed = h.roots.home.join("documents");
    if typed.is_dir() {
        assert_eq!(steps(&h, "copy PDFs from Documents to ~/documents"), [Action::Copy { from: bill, to: typed.join("bill 2.pdf") }]);
    }
}

#[test]
fn quoted_number_template_produces_the_requested_preview_names() {
    let h = home();
    fs::write(h.roots.home.join("Pictures/one.png"), b"1").unwrap();
    fs::write(h.roots.home.join("Pictures/two.jpg"), b"2").unwrap();
    let actions = steps(&h, "number images in Pictures as \"shot-001\", \"shot-002\"");
    let mut names: Vec<_> = actions
        .iter()
        .map(|action| match action {
            Action::Rename { new_name, .. } => new_name.clone(),
            other => panic!("expected rename action, got {other:?}"),
        })
        .collect();
    names.sort();
    assert_eq!(names, ["shot-001.png", "shot-002.jpg"]);
}

#[test]
fn archive_and_dmg_rules_plan_only_the_requested_extensions() {
    let h = home();
    for (name, bytes) in [
        ("Archive.zip", b"z".as_slice()),
        ("notes.txt", b"t".as_slice()),
        ("installer.dmg", b"d".as_slice()),
        ("installer.pkg", b"p".as_slice()),
    ] {
        fs::write(h.roots.home.join("Downloads").join(name), bytes).unwrap();
    }
    let archive_actions = steps(&h, "move archive files from Downloads to External");
    assert!(archive_actions.iter().any(|action| matches!(action, Action::Move { from, .. } if file_name(from) == "Archive.zip")));
    assert!(!archive_actions.iter().any(|action| matches!(action, Action::Move { from, .. } if file_name(from) == "notes.txt")));
    let dmg_actions = steps(&h, "delete dmg files from Downloads");
    assert!(matches!(&dmg_actions[..], [Action::Trash { path }] if file_name(path) == "installer.dmg"));
}

#[test]
fn move_adds_overwrite_suffix() {
    let h = home();
    fs::write(h.roots.home.join("Downloads/report.pdf"), b"a").unwrap();
    fs::write(h.roots.home.join("Documents/report.pdf"), b"b").unwrap();
    let a = steps(&h, "move report.pdf from Downloads to Documents");
    assert_eq!(a, vec![Action::Move { from: h.roots.home.join("Downloads/report.pdf"), to: h.roots.home.join("Documents/report 2.pdf") }]);
}

#[test]
fn index_timeout_failure_and_cancellation_are_reported() {
    let search = search::Search::default();
    let child = Command::new("/bin/sleep").arg("5").stdout(Stdio::piped()).spawn().unwrap();
    assert!(collect_spotlight(child, Some(&search), Duration::from_millis(5)).is_empty());
    search.record_failure();
    assert!(search.control.failed_with(search::Failure::SpotlightTimeout));
    assert!(search.status().unwrap().contains("Spotlight timed out"));
    let mut search = search::Search::default();
    search.timeout = Duration::ZERO;
    let child = Command::new("/bin/sleep").arg("5").stdout(Stdio::piped()).spawn().unwrap();
    assert!(collect_spotlight(child, Some(&search), Duration::from_secs(2)).is_empty());
    assert!(search.status().unwrap().contains("Search time limit"));
    let search = search::Search::default();
    search.control.cancel();
    let child = Command::new("/bin/sleep").arg("5").stdout(Stdio::piped()).spawn().unwrap();
    assert!(collect_spotlight(child, Some(&search), Duration::from_secs(2)).is_empty());
    assert_eq!(search.status().as_deref(), Some("Search cancelled."));
    let search = search::Search::default();
    let mut child = Command::new("/usr/bin/false").stdout(Stdio::piped()).spawn().unwrap();
    child.wait().unwrap();
    assert!(collect_spotlight(child, Some(&search), Duration::from_secs(2)).is_empty());
    assert!(search.status().unwrap().contains("exited unsuccessfully"));
}

#[test]
fn incomplete_search_blocks_changes_and_marks_read_only_results() {
    let h = home();
    let dl = h.roots.downloads();
    fs::write(dl.join("AEROGUARD_1.pdf"), b"a").unwrap();
    fs::write(dl.join("AEROGUARD_2.pdf"), b"b").unwrap();
    // Exact names have a directory budget, while file sets count every entry.
    fs::create_dir_all(h.roots.home.join("Documents/projects/old")).unwrap();
    let choices = HashMap::new();
    let history = History::load(h.roots.home.join("history.json"));
    assert!(matches!(plan_with_limit(&h, "delete Aeroguard files from Downloads", 1),
        Err(Stop::SearchIncomplete(message)) if message.contains("Search incomplete") && message.contains("Entry limit")));
    assert!(matches!(plan_with_limit(&h, "rename AEROGUARD_1.pdf to final.pdf", 1),
        Err(Stop::Ambiguous { candidates, reason: AmbiguityReason::IncompleteSearch, .. }) if candidates == vec![dl.join("AEROGUARD_1.pdf")]));
    for command in ["list PDFs in Downloads", "count PDFs in Downloads", "show size of Downloads"] {
        let mut planner = Planner::new(&h.roots, &choices);
        planner.search.limit = 1;
        assert!(matches!(planner.plan(&parse(command).unwrap(), &history),
            Ok(Planned::Info { summary, listed: None, .. }) if summary.contains("Search incomplete") && summary.contains("Partial results")));
    }
    let mut planner = Planner::new(&h.roots, &choices);
    planner.search.limit = 1;
    let commands = vec![("delete Aeroguard files from Downloads".into(), parse("delete Aeroguard files from Downloads").unwrap())];
    assert!(matches!(planner.plan_shortcut("tidy", &commands, &history),
        Err(Stop::SearchIncomplete(message)) if message.contains("Search incomplete")));
    assert!(dl.join("AEROGUARD_1.pdf").exists());
}

#[test]
fn exact_and_similar_matching_share_one_scan() {
    let h = home();
    let path = h.roots.downloads().join("Aeroguard Case.pdf");
    fs::write(&path, b"a").unwrap();
    let choices = HashMap::new();
    let planner = Planner::new(&h.roots, &choices);
    assert_eq!(planner.resolve_item("aeroguard case pdf", Some("Downloads")).unwrap(), path);
    assert_eq!(planner.search.control.visited.load(std::sync::atomic::Ordering::Relaxed), 1);
}

#[test]
fn unreadable_lookup_does_not_rescan_or_create_folders() {
    use std::os::unix::fs::PermissionsExt;

    let h = home();
    let downloads = h.roots.downloads();
    let blocked = downloads.join("Blocked");
    let report = downloads.join("report.pdf");
    fs::create_dir(&blocked).unwrap();
    fs::write(&report, b"a").unwrap();
    fs::set_permissions(&blocked, fs::Permissions::from_mode(0o000)).unwrap();

    let choices = HashMap::new();
    let item_planner = Planner::new(&h.roots, &choices);
    let item = item_planner.resolve_item("report.pdf", Some("Downloads"));
    let folder_planner = Planner::new(&h.roots, &choices);
    let folder = folder_planner.resolve_folder("Projects", Some(&downloads));
    let history = History::load(h.roots.home.join("history.json"));
    let commands = [
        ("delete report.pdf from Downloads".into(), parse("delete report.pdf from Downloads").unwrap()),
        ("create a folder in Desktop named Projects".into(), parse("create a folder in Desktop named Projects").unwrap()),
    ];
    let shortcut = Planner::new(&h.roots, &choices).plan_shortcut("tidy", &commands, &history);

    fs::set_permissions(&blocked, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(matches!(item, Err(Stop::Ambiguous { reason: AmbiguityReason::IncompleteSearch, candidates, .. })
        if candidates == vec![report.clone()]));
    assert!(matches!(shortcut, Err(Stop::Ambiguous { reason: AmbiguityReason::IncompleteSearch, candidates, .. })
        if candidates == vec![report]));
    assert!(matches!(folder, Err(Stop::SearchIncomplete(message)) if message.contains("Couldn't read “~/Downloads/Blocked”")));
    for planner in [item_planner, folder_planner] {
        assert_eq!(planner.search.control.visited.load(std::sync::atomic::Ordering::Relaxed), 2);
    }
}

#[test]
fn a_shared_lookup_budget_can_retry_with_complete_per_root_walks() {
    let h = home();
    fs::create_dir(h.roots.desktop().join("Work")).unwrap();
    fs::create_dir(h.roots.downloads().join("Projects")).unwrap();
    let report = h.roots.home.join("Documents/report.pdf");
    fs::write(&report, b"a").unwrap();
    assert!(matches!(plan_with_limit(&h, "delete report.pdf", 1), Ok(Planned::Steps { actions, .. })
        if actions == vec![Action::Trash { path: report }]));
}

#[test]
fn exact_miss_with_a_partial_fuzzy_snapshot_refuses_to_resolve_an_item() {
    let h = home();
    let downloads = h.roots.downloads();
    fs::write(downloads.join("Resume.pdf"), b"a").unwrap();
    fs::write(downloads.join("unrelated.txt"), b"b").unwrap();
    let choices = HashMap::new();
    let mut planner = Planner::new(&h.roots, &choices);
    planner.search.limit = 1;
    assert!(
        matches!(planner.resolve_item("resume", Some("Downloads")), Err(Stop::SearchIncomplete(message)) if message.contains("Entry limit"))
    );
    assert_eq!(planner.search.control.visited.load(std::sync::atomic::Ordering::Relaxed), 2);
    assert_eq!(planner.resolve_item("Resume.pdf", Some("Downloads")).unwrap(), downloads.join("Resume.pdf"));
}

#[test]
fn typo_suggestions_require_selection_even_for_one_candidate() {
    let h = home();
    let path = h.roots.downloads().join("Aeroguard-Report.pdf");
    fs::write(&path, b"a").unwrap();
    let command = "delete aerogaurd-report.pdf from Downloads";
    assert!(matches!(plan(&h, command), Err(Stop::Ambiguous { candidates, .. }) if candidates == vec![path.clone()]));
    let choices = HashMap::from([("aerogaurd-report.pdf".into(), path.clone())]);
    let history = History::load(h.roots.home.join("history.json"));
    assert!(matches!(Planner::new(&h.roots, &choices).plan(&parse(command).unwrap(), &history),
        Ok(Planned::Steps { actions, .. }) if actions == vec![Action::Trash { path: path.clone() }]));
    assert!(path.exists());
    assert!(near_typo("aeroguard", "aerogaurd"));
    assert!(!near_typo("aeroguard", "unrelated"));
}

#[test]
fn filename_filters_treat_separators_consistently_without_typo_expansion() {
    let h = home();
    let dl = h.roots.downloads();
    fs::write(dl.join("AERO_GUARD-Case.pdf"), b"a").unwrap();
    fs::write(dl.join("AeroGaurd.pdf"), b"a").unwrap();
    assert_eq!(
        steps(&h, "delete files containing aero guard from Downloads"),
        vec![Action::Trash { path: dl.join("AERO_GUARD-Case.pdf") }]
    );
    assert_eq!(
        steps(&h, "delete files starting with aero guard from Downloads"),
        vec![Action::Trash { path: dl.join("AERO_GUARD-Case.pdf") }]
    );
}

#[test]
fn file_group_commands_include_deeply_nested_files() {
    let h = home();
    let dl = h.roots.downloads();
    let nested = dl.join("one/two/three/four");
    fs::create_dir_all(&nested).unwrap();
    for name in ["Draft Report.pdf", "scan.pdf", "Photo.heic", "setup.dmg"] {
        fs::write(nested.join(name), b"fixture").unwrap();
    }
    for command in [
        "add prefix Invoice- to PDFs in Downloads",
        "add date prefix to PDFs in Downloads",
        "replace spaces with dashes in files in Downloads",
        "lowercase file names in Downloads",
        "number images in Downloads as trip-001, trip-002",
        "convert all HEIC in Downloads to jpg",
        "resize all images in Downloads to 1600 pixels wide",
        "merge all PDFs in Downloads into combined.pdf",
        "make a PDF from images in Downloads",
        // Deletes read only direct children, so they don't include the
        // deeply nested files this fixture sets up (see the delete tests).
        // Sorts are top-level only unless subfolders are asked for.
        "sort Downloads by type including subfolders",
        "sort Downloads by month including subfolders",
    ] {
        assert!(!steps(&h, command).is_empty(), "{command}");
    }
    for command in ["list PDFs in Downloads", "count PDFs in Downloads"] {
        let Ok(Planned::Info { total, .. }) = plan(&h, command) else { panic!("{command}") };
        assert_eq!(total, 2, "{command}");
    }
    assert!(nested.join("Draft Report.pdf").exists(), "planning doesn't execute changes");
}

#[test]
fn chains_preserve_existing_folder_contents_and_destination_collisions() {
    let h = home();
    let dl = h.roots.downloads();
    fs::create_dir(dl.join("PDFS")).unwrap();
    fs::write(dl.join("bill.pdf"), b"new").unwrap();
    fs::write(dl.join("PDFS/bill.pdf"), b"old").unwrap();
    let actions = steps(&h, "create a folder in Downloads called PDFS and move PDFs into it");
    assert_eq!(actions, vec![Action::Move { from: dl.join("bill.pdf"), to: dl.join("PDFS/bill 2.pdf") }]);
    assert_eq!(fs::read(dl.join("PDFS/bill.pdf")).unwrap(), b"old");
    assert!(dl.join("bill.pdf").exists());
}

#[test]
fn chain_steps_share_output_reservations_and_reject_file_dependencies() {
    let h = home();
    let dl = h.roots.downloads();
    fs::write(dl.join("a.pdf"), b"a").unwrap();
    fs::write(h.roots.desktop().join("a.pdf"), b"b").unwrap();
    let actions = steps(&h, "copy a.pdf from Downloads to Documents then copy a.pdf from Desktop to Documents");
    assert_eq!(actions.len(), 2);
    assert!(matches!(&actions[0], Action::Copy { to, .. } if *to == h.roots.home.join("Documents/a.pdf")));
    assert!(matches!(&actions[1], Action::Copy { to, .. } if *to == h.roots.home.join("Documents/a 2.pdf")));
    assert!(plan(&h, "move a.pdf from Downloads to Documents then copy a.pdf from Downloads to Pictures").is_err());
    assert!(plan(&h, "create folder named Archive in Downloads then delete Archive in Downloads").is_err());
    fs::write(dl.join("Archive"), b"existing file").unwrap();
    assert!(plan(&h, "create folder named Archive in Downloads then move PDFs into it").is_err());
}

#[test]
fn chain_rejects_a_later_step_reading_a_folder_an_earlier_step_writes() {
    let h = home();
    let dl = h.roots.downloads();
    let archive = dl.join("Archive");
    fs::create_dir(&archive).unwrap();
    fs::write(dl.join("bill.pdf"), b"new").unwrap();
    fs::write(archive.join("old.txt"), b"old").unwrap();
    // The second step's file list comes from disk before the first step
    // runs, so it can never include the bill.pdf that step moves in.
    assert!(matches!(
        plan(&h, "move PDFs from Downloads to Archive then move files from Archive to Documents"),
        Err(Stop::Problem(message)) if message == "Command 2 depends on files used earlier. Run those commands separately."
    ));
    // "create X then move into X" must keep working.
    let actions = steps(&h, "create a folder in Downloads called PDFS and move PDFs into it");
    assert!(matches!(actions.last(), Some(Action::Move { to, .. }) if to.starts_with(dl.join("PDFS"))));
}

#[test]
fn move_into_a_planned_trash_folder_keeps_it_a_normal_folder() {
    let h = home();
    let dl = h.roots.downloads();
    fs::write(dl.join("a.pdf"), b"a").unwrap();
    let actions = steps(&h, "create a folder in Downloads called Trash and move PDFs into it");
    assert!(actions.iter().any(|a| matches!(a, Action::CreateFolder { path } if path == &dl.join("Trash"))), "{actions:?}");
    assert!(actions.iter().any(|a| matches!(a, Action::Move { to, .. } if to.starts_with(dl.join("Trash")))), "{actions:?}");
    assert!(!actions.iter().any(|a| matches!(a, Action::Trash { .. })), "the PDFs go into the folder planned earlier, not the macOS Trash");
}

#[test]
fn creating_a_folder_does_not_block_a_later_direct_move_from_its_parent() {
    let h = home();
    let dl = h.roots.downloads();
    fs::write(dl.join("a.pdf"), b"a").unwrap();
    let actions = steps(&h, "create folder named Taxes in Downloads then move PDFs from Downloads to Archive");
    assert!(actions.iter().any(|a| matches!(a, Action::CreateFolder { path } if path == &dl.join("Taxes"))), "{actions:?}");
    assert!(
        actions.iter().any(|a| matches!(a, Action::Move { from, to }
        if from == &dl.join("a.pdf") && to.starts_with(dl.join("Archive")))),
        "{actions:?}"
    );
}

#[test]
fn creating_a_folder_no_longer_blocks_a_later_delete_but_writes_still_do() {
    let h = home();
    let dl = h.roots.downloads();
    fs::write(dl.join("a.pdf"), b"a").unwrap();
    // Deletes read only direct children now, so an empty folder created
    // earlier can't make their file list stale.
    let actions = steps(&h, "create folder named Taxes in Downloads then delete PDFs from Downloads");
    assert!(actions.iter().any(|a| matches!(a, Action::CreateFolder { path } if path == &dl.join("Taxes"))), "{actions:?}");
    assert!(actions.iter().any(|a| matches!(a, Action::Trash { path } if path == &dl.join("a.pdf"))), "{actions:?}");
    // A real move destination inside the folder still makes a later
    // direct-only read stale.
    assert!(matches!(
        plan(&h, "move PDFs from Downloads to Archive then move PDFs from Downloads to Pictures"),
        Err(Stop::Problem(message)) if message == "Command 2 depends on files used earlier. Run those commands separately."
    ));
}

#[test]
fn a_later_step_reading_the_created_folder_itself_is_still_refused() {
    let h = home();
    let created = h.roots.downloads().join("Taxes");
    let mut deps = Dependencies::default();
    deps.writes.push((created.clone(), true));
    let choices = HashMap::new();
    let mut planner = Planner::new(&h.roots, &choices);
    planner.push(Action::Move { from: created.join("a.pdf"), to: h.roots.home.join("Documents/a.pdf") }, None);
    assert!(planner.check_dependencies(0, 2, &mut deps).is_err());
}

#[test]
fn shortcut_applies_the_chain_dependency_guard() {
    let h = home();
    fs::write(h.roots.downloads().join("bill.pdf"), b"new").unwrap();
    let commands: Vec<(String, Intent)> =
        ["move PDFs from Downloads to Archive", "delete PDFs in Downloads"].iter().map(|c| (c.to_string(), parse(c).unwrap())).collect();
    let choices = HashMap::new();
    let history = History::load(h.roots.home.join("history.json"));
    assert!(matches!(
        Planner::new(&h.roots, &choices).plan_shortcut("tidy", &commands, &history),
        Err(Stop::Problem(message)) if message == "Command 2 depends on files used earlier. Run those commands separately."
    ));
}

#[test]
fn shortcut_refuses_a_pdf_step_reading_images_trashed_earlier() {
    let h = home();
    let dl = h.roots.downloads();
    fs::write(dl.join("one.png"), b"a").unwrap();
    fs::write(dl.join("two.jpg"), b"b").unwrap();
    let commands: Vec<(String, Intent)> = ["delete all images in Downloads", "make a PDF from images in Downloads"]
        .iter()
        .map(|c| (c.to_string(), parse(c).unwrap()))
        .collect();
    let choices = HashMap::new();
    let history = History::load(h.roots.home.join("history.json"));
    assert!(matches!(
        Planner::new(&h.roots, &choices).plan_shortcut("tidy", &commands, &history),
        Err(Stop::Problem(message)) if message == "Command 2 depends on files used earlier. Run those commands separately."
    ));
}

#[test]
fn shortcut_refuses_a_delete_of_images_used_by_an_earlier_pdf_step() {
    let h = home();
    let dl = h.roots.downloads();
    fs::write(dl.join("one.png"), b"a").unwrap();
    fs::write(dl.join("two.jpg"), b"b").unwrap();
    let commands: Vec<(String, Intent)> = ["make a PDF from images in Downloads", "delete all images in Downloads"]
        .iter()
        .map(|c| (c.to_string(), parse(c).unwrap()))
        .collect();
    let choices = HashMap::new();
    let history = History::load(h.roots.home.join("history.json"));
    assert!(matches!(
        Planner::new(&h.roots, &choices).plan_shortcut("tidy", &commands, &history),
        Err(Stop::Problem(message)) if message == "Command 2 depends on files used earlier. Run those commands separately."
    ));
}

#[test]
fn transfers_leave_files_in_subfolders_alone() {
    for verb in ["move", "copy"] {
        let h = home();
        let dl = h.roots.downloads();
        fs::create_dir_all(dl.join("Project/docs")).unwrap();
        fs::write(dl.join("bill.pdf"), b"a").unwrap();
        fs::write(dl.join("Project/docs/spec.pdf"), b"b").unwrap();
        let actions = steps(&h, &format!("{verb} PDFs from Downloads to Archive"));
        let (from, to) = (dl.join("bill.pdf"), dl.join("Archive/bill.pdf"));
        let expected = if verb == "move" { Action::Move { from, to } } else { Action::Copy { from, to } };
        assert_eq!(actions, vec![Action::CreateFolder { path: dl.join("Archive") }, expected]);

        fs::remove_file(dl.join("bill.pdf")).unwrap();
        let Err(err) = plan(&h, &format!("{verb} PDFs from Downloads to Archive")) else { panic!("{verb}: expected a refusal") };
        assert!(format!("{err:?}").contains("Found 1 matching file in subfolders: add “including subfolders”"), "{err:?}");
    }
}

#[test]
fn delete_leave_files_in_subfolders_alone_with_a_warning() {
    let h = home();
    let dl = h.roots.downloads();
    fs::create_dir_all(dl.join("Project/docs")).unwrap();
    fs::write(dl.join("bill.pdf"), b"a").unwrap();
    fs::write(dl.join("Project/docs/spec.pdf"), b"b").unwrap();
    let Ok(Planned::Steps { actions, warnings, .. }) = plan(&h, "delete PDFs in Downloads") else { panic!("expected a plan") };
    assert_eq!(actions, vec![Action::Trash { path: dl.join("bill.pdf") }]);
    assert!(warnings.iter().any(|w| w.starts_with("1 more matching file was left alone in subfolders.")), "{warnings:?}");
    assert!(dl.join("Project/docs/spec.pdf").exists());
    // When only subfolder files match, refuse just like move/copy do.
    fs::remove_file(dl.join("bill.pdf")).unwrap();
    let Err(err) = plan(&h, "delete PDFs in Downloads") else { panic!("expected a refusal") };
    assert!(format!("{err:?}").contains("Found 1 matching file in subfolders: add “including subfolders”"), "{err:?}");
}

#[test]
fn location_first_folder_creation_targets_downloads() {
    let h = home();
    let destination = h.roots.downloads().join("PDFS");
    assert_eq!(steps(&h, "create a folder in downloads named PDFS"), vec![Action::CreateFolder { path: destination.clone() }]);
    assert!(!destination.exists(), "planning must only preview the new folder");
}

#[test]
fn location_first_creation_accepts_nested_paths_and_spaced_names() {
    let h = home();
    fs::create_dir(h.roots.home.join("Documents/Dev")).unwrap();
    let path = h.roots.home.join("Documents/Dev/Toggle Wars");
    assert_eq!(steps(&h, "create a folder in Documents/Dev called Toggle Wars."), vec![Action::CreateFolder { path: path.clone() }]);
    assert!(!path.exists());
}

#[test]
fn luna_folder_literals_do_not_select_shorter_decoys() {
    let h = home();
    let full = h.roots.home.join("Documents/Dev/Child Folder");
    fs::create_dir_all(&full).unwrap();
    fs::create_dir_all(h.roots.home.join("Documents/Dev/Child")).unwrap();
    assert_eq!(steps(&h, "delete folder \"Documents/Dev/Child Folder\""), vec![Action::Trash { path: full.clone() }]);
    assert_eq!(
        steps(&h, "move Documents/Dev/Child Folder to Desktop"),
        vec![Action::Move { from: full, to: h.roots.home.join("Desktop/Child Folder") }]
    );
    assert_eq!(
        steps(&h, "create folders named \"Alpha and Beta\" in Documents/Dev"),
        vec![Action::CreateFolder { path: h.roots.home.join("Documents/Dev/Alpha and Beta") }]
    );
}

#[test]
fn luna_named_allowed_roots_are_found_and_protected() {
    let mut h = home();
    let root = h.roots.home.join("Other Folder");
    fs::create_dir(&root).unwrap();
    h.roots = Roots::new(&h.roots.home, None, std::slice::from_ref(&root));
    assert!(matches!(plan(&h, "open \"Other Folder\""), Ok(Planned::Direct {
        action: Action::OpenInFinder { path, reveal: false }, ..
    }) if path == root));
    let report = root.join("report.pdf");
    fs::write(&report, b"report").unwrap();
    assert_eq!(
        steps(&h, "copy report.pdf in \"Other Folder\" to Desktop"),
        vec![Action::Copy { from: report, to: h.roots.home.join("Desktop/report.pdf") }]
    );
    // A folder inside a root copies like any other item.
    let inner = root.join("Inner");
    fs::create_dir(&inner).unwrap();
    assert_eq!(
        steps(&h, "copy Inner in \"Other Folder\" to Desktop"),
        vec![Action::Copy { from: inner, to: h.roots.home.join("Desktop/Inner") }]
    );
    for command in [
        "copy \"Other Folder\" to Desktop",
        "delete folder Other Folder",
        "move Other Folder into Documents",
        "rename Other Folder to New Name",
    ] {
        assert!(matches!(plan(&h, command), Err(Stop::Problem(_))), "{command}");
    }
    assert!(root.exists());
}

#[test]
fn filename_wording_preserves_an_explicit_documents_scope() {
    let h = home();
    let document = h.roots.home.join("Documents/draft-plan.txt");
    let outside = h.roots.home.join("Desktop/draft-plan.txt");
    fs::write(&document, b"a").unwrap();
    fs::write(&outside, b"b").unwrap();
    assert_eq!(
        steps(&h, "replace draft with final in the names in documents"),
        vec![Action::Rename { path: document, new_name: "final-plan.txt".into() }]
    );
    assert_eq!(fs::read(&outside).unwrap(), b"b");
}

#[test]
fn screenshot_files_keeps_screenshot_type_and_all_roots() {
    let h = home();
    let image = h.roots.home.join("Pictures/Screenshot 2026-01-01.png");
    let decoy = h.roots.home.join("Desktop/screenshot-notes.txt");
    fs::write(&image, b"a").unwrap();
    fs::write(&decoy, b"b").unwrap();
    assert_eq!(steps(&h, "delete screenshot files"), vec![Action::Trash { path: image }]);
    assert!(decoy.exists());
}

#[test]
fn sorting_subfolders_leaves_correctly_grouped_files_alone() {
    let h = home();
    let dl = h.roots.downloads();
    fs::create_dir_all(dl.join("nested")).unwrap();
    fs::create_dir_all(dl.join("PDFs")).unwrap();
    fs::write(dl.join("nested/report.pdf"), b"a").unwrap();
    fs::write(dl.join("PDFs/report.pdf"), b"b").unwrap();
    // By default a sort touches only top-level files, so the two files in
    // subfolders stay put and there is nothing to change.
    assert!(matches!(plan(&h, "sort Downloads by type"), Err(Stop::NothingToChange(message)) if message.starts_with("Nothing to change.")));
    // The explicit opt-in keeps the historical recursive behaviour.
    let actions = steps(&h, "sort Downloads by type including subfolders");
    assert_eq!(actions, vec![Action::Move { from: dl.join("nested/report.pdf"), to: dl.join("PDFs/report 2.pdf") }]);
    let month = created(&dl.join("nested/report.pdf")).unwrap().format("%Y-%m").to_string();
    fs::create_dir(dl.join(&month)).unwrap();
    fs::rename(dl.join("nested/report.pdf"), dl.join(&month).join("report.pdf")).unwrap();
    fs::remove_file(dl.join("PDFs/report.pdf")).unwrap();
    assert!(
        matches!(plan(&h, "sort Downloads by month"), Err(Stop::NothingToChange(message)) if message.starts_with("Nothing to change."))
    );
}

#[test]
fn single_item_and_folder_commands_find_deeply_nested_names() {
    let h = home();
    let nested = h.roots.downloads().join("one/two/three/Project");
    fs::create_dir_all(&nested).unwrap();
    fs::write(nested.join("report.pdf"), b"a").unwrap();
    fs::write(nested.join("archive.zip"), b"a").unwrap();
    assert_eq!(
        steps(&h, "rename report.pdf to final.pdf"),
        vec![Action::Rename { path: nested.join("report.pdf"), new_name: "final.pdf".into() }]
    );
    assert_eq!(steps(&h, "create folder Output in Project"), vec![Action::CreateFolder { path: nested.join("Output") }]);
    assert!(matches!(steps(&h, "zip Project into Project.zip").last(), Some(Action::Zip { path, .. }) if path == &nested));
    assert!(matches!(steps(&h, "unzip archive.zip").last(), Some(Action::Unzip { path, .. }) if path == &nested.join("archive.zip")));
    fs::write(h.roots.downloads().join("report.pdf"), b"b").unwrap();
    assert!(matches!(plan(&h, "delete report.pdf from Downloads"), Err(Stop::Ambiguous { candidates, .. }) if candidates.len() == 2));
}

#[test]
fn delete_named_files_matches_the_group_in_downloads() {
    let h = home();
    let dl = h.roots.downloads();
    let names = ["AEROGUARD_Design_Report.pdf", "AEROGUARD_Smiley_Case_Final.step", "AEROGUARD_Smiley_Case_Final.stl"];
    for name in names {
        fs::write(dl.join(name), b"a").unwrap();
    }
    fs::write(dl.join("keep.pdf"), b"a").unwrap();
    fs::write(h.roots.desktop().join(names[0]), b"a").unwrap();
    fs::create_dir(dl.join("AEROGUARD_Clip_Case")).unwrap();
    fs::write(dl.join("AEROGUARD_Clip_Case/nested.stl"), b"a").unwrap();

    let actions = steps(&h, "delete all Aeroguard files from Downloads");
    assert_eq!(actions, names.map(|name| Action::Trash { path: dl.join(name) }));
    // Planning must leave the files on disk, pending the user's Apply.
    assert!(names.iter().all(|name| dl.join(name).exists()));
}

#[test]
fn delete_named_files_leaves_matches_in_subfolders_alone() {
    let h = home();
    let dl = h.roots.downloads();
    let names = ["AEROGUARD_Report.pdf", "project/AEROGUARD_Case.stl", "project/models/AEROGUARD_Clip.step"];
    for name in names {
        let path = dl.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"a").unwrap();
    }
    for dir in [".hidden", "_to_delete", "Example.app"] {
        fs::create_dir(dl.join(dir)).unwrap();
        fs::write(dl.join(dir).join("AEROGUARD_Ignore.pdf"), b"a").unwrap();
    }
    fs::write(dl.join("project/keep.pdf"), b"a").unwrap();
    fs::write(h.roots.desktop().join("AEROGUARD_Desktop.pdf"), b"a").unwrap();
    std::os::unix::fs::symlink(h.roots.desktop(), dl.join("linked-folder")).unwrap();

    for command in [
        "Delete all the Aeroguard files from Downloads",
        "delete files containing Aeroguard from Downloads",
        "delete files starting with Aeroguard from Downloads",
    ] {
        let Ok(Planned::Steps { actions, warnings, .. }) = plan(&h, command) else { panic!("expected a plan: {command}") };
        let expected: HashSet<_> = [dl.join(names[0])].into_iter().collect();
        let actual: HashSet<_> = actions
            .into_iter()
            .map(|a| match a {
                Action::Trash { path } => path,
                _ => panic!("expected only Trash actions"),
            })
            .collect();
        assert_eq!(actual, expected, "{command}");
        assert!(warnings.iter().any(|w| w.starts_with("2 more matching files were left alone in subfolders.")), "{command}: {warnings:?}");
        assert!(names.iter().all(|name| dl.join(name).exists()));
    }
}

#[test]
fn singular_folder_name_resolves() {
    let h = home();
    fs::write(h.roots.home.join("Downloads/Claude.dmg"), b"a").unwrap();
    let a = steps(&h, "delete the claude.dmg file in download folder");
    assert_eq!(a.last(), Some(&Action::Trash { path: h.roots.home.join("Downloads/Claude.dmg") }));
}

#[test]
fn dictated_name_without_dot_resolves() {
    let h = home();
    fs::write(h.roots.home.join("Downloads/Claude.dmg"), b"a").unwrap();
    let a = steps(&h, "Delete the Claude dmg file from the Downloads folder.");
    assert!(matches!(a.last(), Some(Action::Trash { path, .. }) if *path == h.roots.home.join("Downloads/Claude.dmg")));
}

#[test]
fn vague_name_lists_similar_files_closest_first() {
    let h = home();
    let dl = h.roots.home.join("Downloads");
    fs::write(dl.join("Claude Setup.dmg"), b"a").unwrap();
    fs::write(dl.join("Claude.dmg"), b"a").unwrap();
    fs::write(dl.join("notes.txt"), b"a").unwrap();
    match plan(&h, "delete claude from downloads") {
        Err(Stop::Ambiguous { name, candidates, .. }) => {
            assert_eq!(name, "claude");
            assert_eq!(candidates, vec![dl.join("Claude.dmg"), dl.join("Claude Setup.dmg")]);
        }
        _ => panic!("expected a question"),
    }
}

#[test]
fn spotlight_hits_are_checked_against_the_disk() {
    let h = home();
    let docs = h.roots.home.join("Documents");
    let choices = HashMap::new();
    let planner = Planner::new(&h.roots, &choices);
    let deep = docs.join("a/b/c/d/Tax Return.pdf");
    let hidden = docs.join("a/.cache/Tax Return.pdf");
    let packages = docs.join("node_modules/tax/Tax Return.pdf");
    let bundle = docs.join("Tax.app/Tax Return.pdf");
    for p in [&deep, &hidden, &packages, &bundle] {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, b"a").unwrap();
    }
    let outside = h.roots.home.join("Tax Return.pdf");
    fs::write(&outside, b"a").unwrap();
    let gone = docs.join("Tax Return old.pdf");
    let hits = vec![deep.clone(), hidden, packages, bundle, outside, gone];
    assert_eq!(planner.verified_hits(hits, &[docs], false, &planner.search), vec![deep]);
}

#[test]
fn cross_root_spotlight_rescues_partial_name_lookup_and_requires_confirmation() {
    fn indexed_report(_: &str, dirs: &[PathBuf], _: &search::Search) -> Vec<PathBuf> {
        let docs = dirs.iter().find(|d| file_name(d) == "Documents").unwrap();
        vec![
            docs.join("Deep/report.pdf"),
            docs.join("Deep/report.pdf"),
            docs.join("Gone/report.pdf"),
            docs.join("node_modules/report.pdf"),
            docs.join("Project/build/report.pdf"),
            docs.join("Preview.XCWORKSPACE/report.pdf"),
            docs.join("Alias/report.pdf"),
            docs.with_file_name("report.pdf"),
        ]
    }

    let h = home();
    let docs = h.roots.home.join("Documents");
    let candidate = docs.join("Deep/report.pdf");
    let shallow = h.roots.downloads().join("report.pdf");
    for dir in ["Deep", "node_modules", "Project/build", "Preview.XCWORKSPACE"] {
        fs::create_dir_all(docs.join(dir)).unwrap();
        fs::write(docs.join(dir).join("report.pdf"), b"x").unwrap();
    }
    fs::write(docs.join("Project/pyproject.toml"), b"").unwrap();
    fs::write(h.roots.home.join("report.pdf"), b"x").unwrap();
    std::os::unix::fs::symlink(docs.join("Project/build"), docs.join("Alias")).unwrap();
    let choices = HashMap::new();
    let history = History::load(h.roots.home.join("history.json"));
    let intent = parse("rename report.pdf to final.pdf").unwrap();
    let plan = |choices: &HashMap<String, PathBuf>, indexed: bool| {
        let mut planner = Planner::new(&h.roots, choices);
        planner.search.depth = 0;
        planner.index_hits = indexed.then_some(indexed_report as IndexLookup);
        planner.plan(&intent, &history)
    };
    assert!(matches!(plan(&choices, false), Err(Stop::SearchIncomplete(message)) if message.contains("nested too deeply")));
    assert!(
        matches!(plan(&choices, true), Err(Stop::Ambiguous { name, candidates, reason: AmbiguityReason::IncompleteSearch, note: Some(note), .. })
        if name == "report.pdf" && candidates == vec![candidate.clone()]
            && note.contains("~/Documents/Deep") && note.contains("Settings → Folders"))
    );

    fs::write(&shallow, b"x").unwrap();
    assert!(matches!(plan(&choices, true), Err(Stop::Ambiguous { candidates, reason: AmbiguityReason::IncompleteSearch, .. })
        if candidates == vec![candidate.clone(), shallow]));

    let picked = HashMap::from([("report.pdf".into(), candidate.clone())]);
    assert!(matches!(plan(&picked, true), Ok(Planned::Steps { actions, .. })
        if actions == vec![Action::Rename { path: candidate.clone(), new_name: "final.pdf".into() }]));
    fs::remove_file(&candidate).unwrap();
    assert!(matches!(plan(&picked, true), Err(Stop::Problem(message)) if message.contains("Couldn't find")));
}

#[test]
fn spotlight_is_not_asked_for_empty_names() {
    assert!(spotlight("a !", &[]).is_empty());
}

#[test]
fn picked_candidate_is_used() {
    let h = home();
    let dl = h.roots.home.join("Downloads");
    fs::write(dl.join("Claude Setup.dmg"), b"a").unwrap();
    fs::write(dl.join("Claude.dmg"), b"a").unwrap();
    let choices = HashMap::from([("claude".to_string(), dl.join("Claude Setup.dmg"))]);
    let history = History::load(h.roots.home.join("history.json"));
    let Ok(Planned::Steps { actions, .. }) =
        Planner::new(&h.roots, &choices).plan(&parse("delete claude from downloads").unwrap(), &history)
    else {
        panic!("expected a plan")
    };
    assert!(matches!(actions.last(), Some(Action::Trash { path, .. }) if *path == dl.join("Claude Setup.dmg")));
}

#[test]
fn picked_folder_is_checked_against_the_allowed_roots() {
    let h = home();
    fs::write(h.roots.downloads().join("report.pdf"), b"a").unwrap();
    let outside = tempfile::tempdir().unwrap();
    let choices = HashMap::from([("offsite".to_string(), outside.path().to_path_buf())]);
    let history = History::load(h.roots.home.join("history.json"));
    assert!(matches!(
        Planner::new(&h.roots, &choices).plan(&parse("move report.pdf to offsite").unwrap(), &history),
        Err(Stop::Problem(message)) if message.contains("outside the folders Voxi may change")
    ));
}

#[test]
fn shortcut_plans_all_commands_and_notes_the_idle_ones() {
    let h = home();
    let desk = h.roots.home.join("Desktop");
    fs::write(desk.join("Screenshot 2026-10-28 at 10.00.00.png"), b"a").unwrap();
    let commands: Vec<(String, Intent)> = ["move screenshots from today to Screenshots", "delete installers from Downloads"]
        .iter()
        .map(|c| (c.to_string(), parse(c).unwrap()))
        .collect();
    let choices = HashMap::new();
    let history = History::load(h.roots.home.join("history.json"));
    let Ok(Planned::Steps { title, actions, warnings, .. }) =
        Planner::new(&h.roots, &choices).plan_shortcut("run my usuals", &commands, &history)
    else {
        panic!("expected one combined plan")
    };
    assert_eq!(title, "Run my usuals: 1 of 2 commands");
    assert!(
        actions.iter().any(|a| matches!(a, Action::Move { to, .. } if to.ends_with("Screenshots/Screenshot 2026-10-28 at 10.00.00.png")))
    );
    assert!(warnings.iter().any(|w| w.starts_with("Skipped “delete installers from Downloads”: ")), "{warnings:?}");
}

#[test]
fn screenshots_are_found_outside_the_desktop() {
    let h = home();
    let pics = h.roots.home.join("Pictures/Screenshots");
    fs::create_dir_all(&pics).unwrap();
    fs::write(pics.join("Screenshot 2026-10-28 at 10.00.00.png"), b"a").unwrap();
    fs::write(h.roots.home.join("Downloads/Screenshot 2026-10-27 at 09.00.00.png"), b"a").unwrap();
    let Ok(Planned::Info { total, .. }) = plan(&h, "list screenshots") else { panic!("expected a list") };
    assert_eq!(total, 2);
}

#[test]
fn ambiguous_name_is_not_picked_silently() {
    let h = home();
    fs::write(h.roots.home.join("Downloads/notes.txt"), b"a").unwrap();
    fs::write(h.roots.home.join("Desktop/notes.txt"), b"b").unwrap();
    assert!(matches!(plan(&h, "copy notes.txt to Documents"), Err(Stop::Ambiguous { candidates, .. }) if candidates.len() == 2));
}

#[test]
fn delete_becomes_move_to_trash() {
    let h = home();
    fs::write(h.roots.home.join("Downloads/setup.dmg"), b"a").unwrap();
    fs::write(h.roots.home.join("Downloads/keep.txt"), b"a").unwrap();
    let a = steps(&h, "delete installers from Downloads");
    assert_eq!(a, vec![Action::Trash { path: h.roots.home.join("Downloads/setup.dmg") }]);
}

#[test]
fn unfiltered_clean_up_is_refused() {
    let h = home();
    fs::write(h.roots.home.join("Downloads/a.txt"), b"a").unwrap();
    assert!(matches!(plan(&h, "delete files in Downloads"), Err(Stop::Problem(_))));
}

#[test]
fn named_subfolder_can_be_emptied_but_top_level_folders_cannot() {
    let h = home();
    let dir = h.roots.home.join("Downloads/installers");
    fs::create_dir_all(dir.join("old")).unwrap();
    for name in ["a.dmg", "notes.txt", "old/b.pkg"] {
        fs::write(dir.join(name), b"a").unwrap();
    }
    for cmd in ["delete items in installers folder", "delete everything in the installers folder", "delete files in installers"] {
        let mut a = steps(&h, cmd);
        a.sort_by_key(|a| format!("{a:?}"));
        assert_eq!(a, vec![Action::Trash { path: dir.join("a.dmg") }, Action::Trash { path: dir.join("notes.txt") }], "{cmd}");
    }
    for cmd in ["delete everything in Downloads", "delete items in Desktop", "delete files in Documents"] {
        assert!(matches!(plan(&h, cmd), Err(Stop::Problem(m)) if m.contains("To be safe")), "{cmd}");
    }
}

#[test]
fn paths_outside_allowed_folders_are_refused() {
    let h = home();
    assert!(matches!(plan(&h, "move ~/Library/x.txt to Documents"), Err(Stop::Problem(_))));
    assert!(matches!(plan(&h, "move ~/Downloads/../x.txt to Documents"), Err(Stop::Problem(_))));
}

#[test]
fn missing_destination_is_created() {
    let h = home();
    fs::write(h.roots.home.join("Downloads/invoice-1.pdf"), b"a").unwrap();
    let a = steps(&h, "move files containing invoice from Downloads to Invoices");
    let dest = h.roots.home.join("Downloads/Invoices");
    assert_eq!(a[0], Action::CreateFolder { path: dest.clone() });
    assert_eq!(a[1], Action::Move { from: h.roots.home.join("Downloads/invoice-1.pdf"), to: dest.join("invoice-1.pdf") });
}

#[test]
fn moving_to_trash_with_a_filter_plans_trash_without_creating_a_folder() {
    let h = home();
    let dl = h.roots.downloads();
    let big = dl.join("big.bin");
    let f = fs::File::create(&big).unwrap();
    f.set_len(20 * 1024 * 1024).unwrap();
    let actions = steps(&h, "move files from Downloads to Trash between 10 and 50 MB");
    assert_eq!(actions, vec![Action::Trash { path: big.clone() }]);
    assert!(!dl.join("Trash").exists(), "planning must not create a folder");
    // A real folder named Trash is still a normal destination.
    fs::create_dir(dl.join("Trash")).unwrap();
    let actions = steps(&h, "move files from Downloads to Trash between 10 and 50 MB");
    assert!(matches!(&actions[..], [Action::Move { from, to, .. }]
        if from == &big && to.starts_with(dl.join("Trash"))));
}

#[test]
fn moving_unfiltered_files_to_trash_requires_a_filter_even_when_the_source_is_empty() {
    let h = home();
    assert!(matches!(plan(&h, "move files from Downloads to Trash"),
        Err(Stop::Problem(message)) if message.starts_with("To be safe")));
    // Quoted, it names a folder called Trash: a normal move, never the Trash.
    assert!(!matches!(plan(&h, r#"move files from Downloads to "Trash""#),
        Ok(Planned::Steps { actions, .. }) if actions.iter().any(|a| matches!(a, Action::Trash { .. }))));
}

#[test]
fn sort_by_type_groups_known_types() {
    let h = home();
    for f in ["a.pdf", "b.png", "c.weird"] {
        fs::write(h.roots.home.join("Downloads").join(f), b"x").unwrap();
    }
    let a = steps(&h, "sort Downloads by type");
    let d = h.roots.home.join("Downloads");
    assert!(a.contains(&Action::Move { from: d.join("a.pdf"), to: d.join("PDFs/a.pdf") }));
    assert!(a.contains(&Action::Move { from: d.join("b.png"), to: d.join("Images/b.png") }));
    assert_eq!(a.iter().filter(|x| matches!(x, Action::Move { .. })).count(), 2);
}

#[test]
fn sort_documents_by_type_avoids_a_documents_documents_folder() {
    let h = home();
    let docs = h.roots.home.join("Documents");
    fs::write(docs.join("report.pdf"), b"x").unwrap();
    fs::write(docs.join("notes.docx"), b"x").unwrap();
    let a = steps(&h, "sort Documents by type");
    let nested = docs.join("Documents");
    assert!(
        !a.iter().any(|action| matches!(action, Action::Move { to, .. } if to.starts_with(&nested))),
        "no file should land in {}: {a:?}",
        nested.display()
    );
    assert!(a.contains(&Action::Move { from: docs.join("report.pdf"), to: docs.join("PDFs/report.pdf") }));
    assert!(a.contains(&Action::Move { from: docs.join("notes.docx"), to: docs.join("Document files/notes.docx") }));
}

#[test]
fn sort_by_type_uses_the_ordinary_category_name_when_there_is_no_collision() {
    let h = home();
    let dl = h.roots.downloads();
    fs::write(dl.join("notes.docx"), b"x").unwrap();
    let a = steps(&h, "sort Downloads by type");
    assert!(a.contains(&Action::Move { from: dl.join("notes.docx"), to: dl.join("Documents/notes.docx") }));
}

#[test]
fn sort_by_type_avoids_a_self_named_folder_for_any_category() {
    let h = home();
    let pics = h.roots.home.join("Pictures/Images");
    fs::create_dir(&pics).unwrap();
    fs::write(pics.join("photo.png"), b"x").unwrap();
    let a = steps(&h, "sort Images by type");
    assert!(
        !a.iter().any(|action| matches!(action, Action::Move { to, .. } if to.starts_with(pics.join("Images")))),
        "no nested Images/Images: {a:?}"
    );
    assert!(a.contains(&Action::Move { from: pics.join("photo.png"), to: pics.join("Image files/photo.png") }));
}

#[test]
fn resorting_a_sorted_documents_folder_does_not_nest_category_folders() {
    let h = home();
    let docs = h.roots.home.join("Documents");
    let alternate = docs.join("Document files");
    fs::create_dir(&alternate).unwrap();
    fs::write(alternate.join("notes.docx"), b"x").unwrap();
    fs::write(docs.join("fresh.docx"), b"y").unwrap();
    let a = steps(&h, "sort Documents by type including subfolders");
    // The file already in the alternate category stays put (the "already
    // there" check is path-based, so it works with the new name too).
    assert!(
        !a.iter().any(|action| matches!(action, Action::Move { from, .. } if from == &alternate.join("notes.docx"))),
        "the already-sorted file should not move: {a:?}"
    );
    assert!(a.contains(&Action::Move { from: docs.join("fresh.docx"), to: alternate.join("fresh.docx") }));
    assert!(
        !a.iter().any(|action| matches!(action, Action::Move { to, .. } if to.starts_with(alternate.join("Document files")))),
        "category folders must not be nested: {a:?}"
    );
}

#[test]
fn month_filter_matches_files_from_that_month() {
    let h = home();
    let pics = h.roots.home.join("Pictures");
    let august = pics.join("Screenshot 2026-08-15 at 10.00.00.png");
    fs::write(&august, b"a").unwrap();
    fs::write(pics.join("Screenshot today.png"), b"a").unwrap();
    let when = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_786_788_000); // 2026-08-15
    fs::File::options().write(true).open(&august).unwrap().set_modified(when).unwrap();
    let Ok(Planned::Info { total, rows, .. }) = plan(&h, "find screenshots taken in August 2026") else { panic!("expected a list") };
    assert_eq!(total, 1);
    assert_eq!(rows[0].name, "Screenshot 2026-08-15 at 10.00.00.png");
}

#[test]
fn month_without_a_year_is_the_most_recent_one() {
    let oct = NaiveDate::from_ymd_opt(2026, 10, 28).unwrap();
    assert_eq!(month_year(8, None, oct), 2026);
    assert_eq!(month_year(12, None, oct), 2025);
    assert_eq!(month_year(10, None, oct), 2026);
    assert_eq!(month_year(3, Some(2024), oct), 2024);
}

/// Sets a file's date as `touch -t` would. On macOS an earlier modified
/// time also moves the creation date, which is what the date filters read.
fn set_taken(p: &Path, (y, m, d): (i32, u32, u32)) {
    let at = NaiveDate::from_ymd_opt(y, m, d).unwrap().and_hms_opt(12, 0, 0).unwrap().and_local_timezone(Local).unwrap();
    fs::File::options().write(true).open(p).unwrap().set_modified(at.into()).unwrap();
}

#[test]
fn screenshots_before_and_after_a_month_are_split_by_date() {
    let h = home();
    let pics = h.roots.home.join("Pictures");
    let june = pics.join("Screenshot 2026-06-10 at 09.00.00.png");
    let august = pics.join("Screenshot 2026-08-02 at 09.00.00.png");
    fs::write(&june, b"a").unwrap();
    fs::write(&august, b"a").unwrap();
    set_taken(&june, (2026, 6, 10));
    set_taken(&august, (2026, 8, 2));
    let Ok(Planned::Info { total, rows, .. }) = plan(&h, "Show me the screenshots taken before July 2026") else {
        panic!("expected a list")
    };
    assert_eq!(total, 1);
    assert_eq!(rows[0].name, "Screenshot 2026-06-10 at 09.00.00.png");
    let Ok(Planned::Info { total, rows, .. }) = plan(&h, "list screenshots taken after July 2026") else { panic!("expected a list") };
    assert_eq!(total, 1);
    assert_eq!(rows[0].name, "Screenshot 2026-08-02 at 09.00.00.png");
}

#[test]
fn name_prefix_filter_matches_only_the_start_of_the_name() {
    let h = home();
    let dl = h.roots.home.join("Downloads");
    fs::write(dl.join("invoice-june.pdf"), b"a").unwrap();
    fs::write(dl.join("old invoice.pdf"), b"a").unwrap();
    let a = steps(&h, "delete files that start with invoice from Downloads");
    assert_eq!(a, vec![Action::Trash { path: dl.join("invoice-june.pdf") }]);
}

#[test]
fn a_name_matches_a_file_that_adds_a_version_number() {
    let h = home();
    let dl = h.roots.downloads();
    fs::write(dl.join("Anigen-Android-1.zip"), b"a").unwrap();
    assert!(matches!(plan(&h, "delete anigen android zip from downloads"),
        Err(Stop::Ambiguous { candidates, .. }) if candidates == vec![dl.join("Anigen-Android-1.zip")]));
}

#[test]
fn trashing_a_named_file_says_its_name_once() {
    let h = home();
    fs::write(h.roots.home.join("Downloads/setup.dmg"), b"a").unwrap();
    let Ok(Planned::Steps { title, .. }) = plan(&h, "delete setup.dmg from downloads") else { panic!("expected a plan") };
    assert_eq!(title, "Move setup.dmg to the Trash");
}

#[test]
fn a_name_that_fits_two_files_asks_which_one() {
    let h = home();
    let dl = h.roots.home.join("Downloads");
    fs::write(dl.join("Anigen-Android-1.zip"), b"a").unwrap();
    fs::write(dl.join("Anigen-Android-2.zip"), b"a").unwrap();
    assert!(matches!(
        plan(&h, "delete anigen android zip from downloads"),
        Err(Stop::Ambiguous { candidates, .. }) if candidates.len() == 2
    ));
}

#[test]
fn duplicate_base_names() {
    assert_eq!(base_name("Report (1).pdf"), "report.pdf");
    assert_eq!(base_name("report 2.pdf"), "report.pdf");
    assert_eq!(base_name("report copy.pdf"), "report.pdf");
    assert_eq!(base_name("Chapter 12.pdf"), "chapter.pdf");
}

#[test]
fn replace_keeps_extension_and_case() {
    assert_eq!(replace_in_stem("My Draft v2.docx", "draft", "final").as_deref(), Some("My final v2.docx"));
    assert_eq!(replace_in_stem("a b c.txt", " ", "-").as_deref(), Some("a-b-c.txt"));
    assert_eq!(replace_in_stem("abc.txt", "z", "y"), None);
}

// ---- Haiku test set (corpus 10): planner checks on temporary folders. ----
// These call `plan` only. None of them run an action, and each fixture is a
// fresh temp home. Spotlight answers come from fixtures when needed.

fn touch(path: &Path, bytes: usize) {
    fs::write(path, vec![b'x'; bytes]).unwrap();
}

/// The paths of a plan that trashes files; any other action fails the test.
fn trash_paths(actions: &[Action]) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = actions
        .iter()
        .map(|a| match a {
            Action::Trash { path } => path.clone(),
            other => panic!("expected only trash actions, got {other:?}"),
        })
        .collect();
    out.sort();
    out
}

#[test]
fn haiku_explicit_filename_touches_only_the_named_file() {
    let h = home();
    let dl = h.roots.downloads();
    for name in ["last.pdf", "last.txt", "last", "invoice.pdf"] {
        touch(&dl.join(name), 10);
    }
    assert_eq!(trash_paths(&steps(&h, "delete last.pdf")), vec![dl.join("last.pdf")]);
    assert_eq!(trash_paths(&steps(&h, "delete \"last\"")), vec![dl.join("last")]);
}

#[test]
fn haiku_type_group_stays_in_its_folder() {
    let h = home();
    let (dl, docs, desk) = (h.roots.downloads(), h.roots.home.join("Documents"), h.roots.desktop());
    fs::create_dir(dl.join("sub")).unwrap();
    touch(&dl.join("a.pdf"), 10);
    touch(&dl.join("b.txt"), 10);
    touch(&dl.join("sub/c.pdf"), 10);
    touch(&docs.join("d.pdf"), 10);
    touch(&desk.join("e.pdf"), 10);
    // Deletes take only direct children, so the PDF in `sub` stays put.
    assert_eq!(trash_paths(&steps(&h, "delete PDFs in Downloads")), vec![dl.join("a.pdf")]);
}

#[test]
fn haiku_group_takes_direct_children_but_skips_hidden_ones() {
    let h = home();
    let dl = h.roots.downloads();
    fs::create_dir_all(dl.join("Old")).unwrap();
    fs::create_dir_all(dl.join(".hidden")).unwrap();
    touch(&dl.join("root.pdf"), 10);
    touch(&dl.join("Old/x.pdf"), 10);
    touch(&dl.join(".hidden/y.pdf"), 10);
    assert_eq!(trash_paths(&steps(&h, "delete PDFs in Downloads")), vec![dl.join("root.pdf")]);
}

#[test]
fn haiku_name_group_does_not_reach_other_folders() {
    let h = home();
    let (dl, desk) = (h.roots.downloads(), h.roots.desktop());
    touch(&dl.join("budget-2026.pdf"), 10);
    touch(&dl.join("notes.txt"), 10);
    touch(&desk.join("budget-old.pdf"), 10);
    assert_eq!(trash_paths(&steps(&h, "delete files named budget in Downloads")), vec![dl.join("budget-2026.pdf")]);
}

#[test]
fn haiku_size_filter_selects_only_matching_files() {
    let h = home();
    let dl = h.roots.downloads();
    touch(&dl.join("big.zip"), 2 * 1024 * 1024);
    touch(&dl.join("small.zip"), 10 * 1024);
    assert_eq!(trash_paths(&steps(&h, "delete files larger than 1 MB in Downloads")), vec![dl.join("big.zip")]);
}

#[test]
fn haiku_same_name_in_two_folders_needs_a_choice() {
    let h = home();
    touch(&h.roots.downloads().join("report.pdf"), 10);
    touch(&h.roots.home.join("Documents/report.pdf"), 10);
    assert!(matches!(plan(&h, "move report.pdf to Pictures"), Err(Stop::Ambiguous { candidates, .. }) if candidates.len() == 2));
}

#[test]
fn haiku_typo_never_picks_a_file_silently() {
    let h = home();
    let dl = h.roots.downloads();
    touch(&dl.join("invoice.pdf"), 10);
    assert!(!matches!(plan(&h, "delete invoce.pdf"), Ok(Planned::Steps { .. })));
    assert!(dl.join("invoice.pdf").exists());
}

#[test]
fn haiku_paths_outside_voxi_folders_are_refused() {
    let h = home();
    assert!(matches!(plan(&h, "move /etc/hosts to Documents"), Err(Stop::Problem(_))));
    assert!(matches!(plan(&h, "move ../secret.txt to Documents"), Err(Stop::Problem(_))));
}

#[test]
fn haiku_unfiltered_clean_up_is_refused_by_the_planner() {
    let h = home();
    touch(&h.roots.downloads().join("keep.pdf"), 10);
    assert!(matches!(plan(&h, "delete everything in Downloads"), Err(Stop::Problem(m)) if m.contains("To be safe")));
}

#[test]
fn haiku_delete_plans_a_trash_move_and_touches_nothing() {
    let h = home();
    let f = h.roots.downloads().join("report.pdf");
    touch(&f, 10);
    assert_eq!(steps(&h, "delete report.pdf"), vec![Action::Trash { path: f.clone() }]);
    assert!(f.exists(), "planning must not move or remove files");
}

#[test]
fn haiku_unresolved_references_never_reach_the_planner() {
    let h = home();
    let dl = h.roots.downloads();
    touch(&dl.join("last"), 10);
    touch(&dl.join("last.pdf"), 10);
    for command in ["delete the last file", "move the previous file to Documents", "delete last"] {
        assert!(parse(command).is_err(), "{command} must be refused before planning");
    }
    assert!(dl.join("last").exists() && dl.join("last.pdf").exists());
}

fn haiku_plan_input(h: &Home, command: &str) -> Result<Planned, Stop> {
    let intent = parse(command).map_err(|_| Stop::Problem("Name a specific file or group.".into()))?;
    let choices = HashMap::new();
    let history = History::load(h.roots.home.join("history.json"));
    Planner::new(&h.roots, &choices).plan(&intent, &history)
}

/// Regression (corpus 10, H155/H156): ordinal words that
/// reach the planner must not plan any change.
#[test]
fn haiku_ordinal_references_do_not_plan_changes() {
    let h = home();
    let dl = h.roots.downloads();
    for name in ["first.pdf", "first-draft.txt", "second.pdf"] {
        touch(&dl.join(name), 10);
    }
    for command in ["delete the first file in Downloads", "move the second file in Downloads to Archive"] {
        if let Ok(Planned::Steps { actions, .. }) = haiku_plan_input(&h, command) {
            panic!("{command} planned {actions:?}");
        }
    }
    assert!(dl.join("first.pdf").exists() && dl.join("second.pdf").exists());
}

/// Regression (corpus 10, H185): "the file" with no name or
/// folder must not plan a bulk move of every file in Downloads and Desktop.
#[test]
fn haiku_unscoped_file_move_is_refused_by_the_planner() {
    let h = home();
    touch(&h.roots.downloads().join("a.pdf"), 10);
    touch(&h.roots.desktop().join("b.txt"), 10);
    assert!(matches!(haiku_plan_input(&h, "move the file to Documents"), Err(Stop::Problem(_))));
}

/// Regression (corpus 10, H193): "move from Downloads" names
/// no files, so it must not plan a move of everything in that folder.
#[test]
fn haiku_move_from_folder_without_a_subject_is_refused() {
    let h = home();
    touch(&h.roots.downloads().join("a.pdf"), 10);
    touch(&h.roots.desktop().join("b.txt"), 10);
    assert!(matches!(haiku_plan_input(&h, "move from Downloads to Documents"), Err(Stop::Problem(_))));
}

#[test]
fn haiku_listing_is_read_only() {
    let h = home();
    let f = h.roots.downloads().join("report.pdf");
    touch(&f, 10);
    let history = History::load(h.roots.home.join("history.json"));
    let choices = HashMap::new();
    let result = Planner::new(&h.roots, &choices).plan(&parse("list PDFs in Downloads").unwrap(), &history);
    assert!(matches!(result, Ok(Planned::Info { .. })));
    assert!(f.exists());
}

#[test]
fn haiku_quoted_command_words_select_only_the_literal_file() {
    let h = home();
    let dl = h.roots.downloads();
    for name in ["Notes", "Notes to Self.txt", "Chapter one.pdf", "Chapter 1.pdf"] {
        touch(&dl.join(name), 10);
    }
    for name in ["Notes to Self.txt", "Chapter one.pdf"] {
        assert_eq!(trash_paths(&steps(&h, &format!("delete \"{name}\" in Downloads"))), vec![dl.join(name)]);
    }
    let actions = steps(&h, "move \"Notes to Self.txt\" from Downloads to Documents");
    assert_eq!(actions, vec![Action::Move { from: dl.join("Notes to Self.txt"), to: h.roots.home.join("Documents/Notes to Self.txt") }]);
    assert!(dl.join("Notes").exists() && dl.join("Notes to Self.txt").exists());
}

#[test]
fn haiku_quoted_folder_words_do_not_turn_into_date_filters() {
    let h = home();
    let dl = h.roots.downloads();
    fs::create_dir(dl.join("last week")).unwrap();
    touch(&dl.join("last week/keep.txt"), 10);
    touch(&dl.join("last week/report.pdf"), 10);
    touch(&dl.join("outside.pdf"), 10);
    assert_eq!(trash_paths(&steps(&h, "delete PDFs in \"last week\"")), vec![dl.join("last week/report.pdf")]);
}

#[test]
fn haiku_invalid_references_and_empty_names_visit_no_files() {
    let h = home();
    let choices = HashMap::new();
    let history = History::load(h.roots.home.join("history.json"));
    for command in [
        "delete the last file",
        "delete the first file",
        "delete the 2nd file",
        "move the file to Documents",
        "move from Downloads to Documents",
    ] {
        let control = search::Control::default();
        let result = parse(command).map(|intent| Planner::new(&h.roots, &choices).with_control(control.clone()).plan(&intent, &history));
        assert!(result.is_err(), "{command} must fail at parsing");
        assert_eq!(control.visited.load(std::sync::atomic::Ordering::Relaxed), 0);
    }
    let control = search::Control::default();
    assert!(matches!(
        Planner::new(&h.roots, &choices).with_control(control.clone()).plan(&Intent::Trash { src: Source::item("") }, &history),
        Err(Stop::Problem(_))
    ));
    assert_eq!(control.visited.load(std::sync::atomic::Ordering::Relaxed), 0);
}

#[test]
fn haiku_erase_everything_still_cannot_create_a_trash_plan() {
    let h = home();
    let file = h.roots.home.join("Documents/keep.pdf");
    touch(&file, 10);
    assert!(matches!(plan(&h, "erase everything in Documents"), Err(Stop::Problem(message)) if message.contains("To be safe")));
    assert!(file.exists());
}

#[test]
fn stl_listing_filters_extensions_recursively_inside_downloads() {
    let h = home();
    let dl = h.roots.downloads();
    fs::create_dir(dl.join("Models")).unwrap();
    for name in ["part.stl", "UPPER.STL", "Models/nested.stl", "STL guide.txt", "STL-backup.zip", "model.stl.txt"] {
        touch(&dl.join(name), 10);
    }
    touch(&h.roots.home.join("Documents/outside.stl"), 10);
    let Ok(Planned::Info { rows, total, .. }) = plan(&h, " list the STL files in Downloads. ") else { panic!("expected a read-only list") };
    let mut names: Vec<_> = rows.iter().map(|row| row.name.as_str()).collect();
    names.sort();
    assert_eq!(names, vec!["UPPER.STL", "nested.stl", "part.stl"]);
    assert_eq!(total, 3);
    assert!(dl.join("part.stl").exists());
}

#[test]
#[ignore]
fn extension_audit_planner_preview_controls() {
    let h = home();
    let dl = h.roots.downloads();
    fs::create_dir(dl.join("Models")).unwrap();
    fs::create_dir(dl.join("folder.stl")).unwrap();
    for name in ["part.stl", "UPPER.STL", "Models/nested.stl", "STL guide.txt", "STL-backup.zip", "model.stl.txt", "Dockerfile"] {
        touch(&dl.join(name), 10);
    }
    touch(&h.roots.home.join("Documents/outside.stl"), 10);

    let listed = plan(&h, "list the STL files in Downloads").unwrap();
    let Planned::Info { rows, total, .. } = listed else { panic!("expected read-only list") };
    let names: std::collections::BTreeSet<_> = rows.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(total, 3);
    assert_eq!(names, ["UPPER.STL", "nested.stl", "part.stl"].into_iter().collect());

    let counted = plan(&h, "count the STL files in Downloads").unwrap();
    let Planned::Info { total, .. } = counted else { panic!("expected count preview") };
    assert_eq!(total, 3);

    let moved = steps(&h, "move the STL files in Downloads to Documents");
    let moved_names: std::collections::BTreeSet<_> = moved
        .iter()
        .filter_map(|a| match a {
            Action::Move { from, .. } => Some(file_name(from)),
            _ => None,
        })
        .collect();
    assert_eq!(moved_names, ["UPPER.STL", "part.stl"].into_iter().map(str::to_string).collect());

    let trashed = steps(&h, "delete the STL files in Downloads");
    let trashed_names: std::collections::BTreeSet<_> = trashed
        .iter()
        .filter_map(|a| match a {
            Action::Trash { path } => Some(file_name(path)),
            _ => None,
        })
        .collect();
    assert_eq!(trashed_names, ["UPPER.STL", "nested.stl", "part.stl"].into_iter().map(str::to_string).collect());
    assert!(dl.join("part.stl").exists(), "planning previews must not execute changes");
}

#[test]
#[ignore]
fn extension_audit_planner_suffix_matrix_jsonl() {
    let h = home();
    let dl = h.roots.downloads();
    let samples =
        [("obj", "mesh.obj"), ("3mf", "print.3mf"), ("bmp", "bitmap.bmp"), ("tar.gz", "archive.tar.gz"), ("zzcustom", "custom.zzcustom")];
    let mut report = String::new();
    for (requested, filename) in samples {
        touch(&dl.join(filename), 1);
        touch(&dl.join(format!("guide.{requested}.txt")), 1);
        fs::create_dir(dl.join(format!("folder.{requested}"))).unwrap();
        let intent = Intent::List {
            src: Source::Set {
                folder: Some("Downloads".into()),
                filter: Filter { file_type: Some(FileType::Files), ext: Some(vec![requested.to_string()]), ..Default::default() },
            },
        };
        let history = History::load(h.roots.home.join("history.json"));
        let actual = Planner::new(&h.roots, &HashMap::new()).plan(&intent, &history);
        let (names, total) = match actual {
            Ok(Planned::Info { rows, total, .. }) => (rows.into_iter().map(|r| r.name).collect::<Vec<_>>(), total),
            Ok(_) => (Vec::new(), 0),
            Err(error) => (vec![format!("ERROR:{error:?}")], 0),
        };
        let expected_name = Path::new(filename).file_name().unwrap().to_string_lossy().into_owned();
        let expected = vec![expected_name];
        report.push_str(&serde_json::json!({
            "requested_suffix":requested, "planner_filter_extensions":[requested],
            "expected_names":expected, "actual_names":names, "expected_total":1, "actual_total":total,
            "pass":total == 1 && names == expected,
            "note":if requested.contains('.') { "literal compound suffix is preserved in the requested filter" } else { "custom filters use direct Source::Set so parser aliases are not involved" },
        }).to_string());
        report.push('\n');
    }
    let path = std::env::var_os("VOXI_EXTENSION_PLANNER_REPORT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/extension-planner-results.jsonl"));
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, report).unwrap();
}

#[test]
fn excluded_folders_are_refused_and_never_found_by_name() {
    let h = home();
    let archive = h.roots.home.join("Documents/Archive");
    fs::create_dir_all(&archive).unwrap();
    fs::write(archive.join("old.pdf"), b"a").unwrap();
    let roots = Roots::new(&h.roots.home, None, &[]).with_exclusions(std::slice::from_ref(&archive));
    let h = Home { _dir: h._dir, roots };
    assert!(matches!(plan(&h, "move ~/Documents/Archive/old.pdf to Desktop"), Err(Stop::Problem(m)) if m.contains("outside the folders")));
    assert!(matches!(plan(&h, "move old.pdf to Desktop"), Err(Stop::MissingTransferItem { .. })));
    fs::write(h.roots.home.join("Documents/new.pdf"), b"a").unwrap();
    // The excluded folder is invisible: the name is treated as a new folder,
    // and nothing planned reaches inside the excluded one.
    let actions = steps(&h, "move new.pdf to Archive");
    assert!(actions.iter().all(|a| !format!("{a:?}").contains("Documents/Archive")), "{actions:?}");
}

/// An old file edited today: "older than a year" reads its creation date,
/// "not modified in a year" its last change, and the preview says which.
#[cfg(target_os = "macos")]
#[test]
fn modified_filters_read_the_last_change_not_the_creation_date() {
    let h = home();
    let draft = h.roots.home.join("Downloads/draft.pdf");
    fs::write(&draft, b"a").unwrap();
    // An earlier modified time also moves the creation date back on macOS.
    set_taken(&draft, (2024, 1, 10));
    fs::File::options().write(true).open(&draft).unwrap().set_modified(SystemTime::now()).unwrap();
    assert!(created(&draft).unwrap().year() <= 2024, "the creation date stays old");

    let Ok(Planned::Steps { actions, warnings, .. }) = plan(&h, "remove files in Downloads older than a year") else {
        panic!("expected steps")
    };
    assert!(matches!(&actions[..], [Action::Trash { path }] if path == &draft), "{actions:?}");
    assert!(warnings.iter().any(|w| w == "Dates are when each file was created (or last modified, where that's unknown)."), "{warnings:?}");

    for cmd in ["remove files in Downloads not modified in a year", "remove files in Downloads not edited in a year"] {
        match plan(&h, cmd) {
            Err(Stop::NothingToChange(m)) => assert!(m.starts_with("No files match"), "{cmd}: {m}"),
            Err(e) => panic!("{cmd}: {e:?}"),
            Ok(_) => panic!("{cmd}: a file edited today must not match"),
        }
    }
    let Ok(Planned::Info { total, summary, rows, .. }) = plan(&h, "list files in Downloads modified today") else {
        panic!("expected a list")
    };
    assert_eq!(total, 1);
    assert!(summary.ends_with("Dates are when each file was last modified."), "{summary}");
    assert!(rows[0].detail.contains("· modified "), "{}", rows[0].detail);
    let Ok(Planned::Steps { warnings, .. }) = plan(&h, "remove files in Downloads modified today") else { panic!("expected steps") };
    assert!(warnings.iter().any(|w| w == "Dates are when each file was last modified."), "{warnings:?}");
}

#[test]
fn including_subfolders_reaches_files_in_them() {
    let h = home();
    let dl = h.roots.downloads();
    fs::create_dir_all(dl.join("Project/docs")).unwrap();
    fs::write(dl.join("bill.pdf"), b"a").unwrap();
    fs::write(dl.join("Project/docs/spec.pdf"), b"b").unwrap();
    fs::write(dl.join("Project/notes.txt"), b"c").unwrap();
    let mut trashed: Vec<PathBuf> = steps(&h, "delete PDFs in Downloads including subfolders")
        .into_iter()
        .map(|a| match a {
            Action::Trash { path } => path,
            other => panic!("{other:?}"),
        })
        .collect();
    trashed.sort();
    assert_eq!(trashed, [dl.join("Project/docs/spec.pdf"), dl.join("bill.pdf")]);
    let moved = steps(&h, "move PDFs from Downloads to Documents including subfolders");
    assert_eq!(moved.iter().filter(|a| matches!(a, Action::Move { .. })).count(), 2, "{moved:?}");
}

#[test]
fn direct_commands_stay_within_budget_when_a_subfolder_is_large() {
    let h = home();
    let docs = h.roots.home.join("Documents");
    let invoice = docs.join("invoice.pdf");
    fs::create_dir(docs.join("Archive")).unwrap();
    fs::create_dir_all(docs.join("dev/repo")).unwrap();
    fs::write(&invoice, b"a").unwrap();
    for i in 0..32 {
        fs::write(docs.join(format!("dev/repo/file-{i}.pdf")), b"x").unwrap();
    }

    for command in
        ["move PDFs in Documents to Archive", "copy PDFs in Documents to Archive", "delete PDFs in Documents", "sort Documents by type"]
    {
        let Ok(Planned::Steps { actions, warnings, .. }) = plan_with_limit(&h, command, 3) else {
            panic!("expected shallow changes for {command}");
        };
        let changed: Vec<_> = actions
            .iter()
            .filter_map(|action| match action {
                Action::Move { from, .. } | Action::Copy { from, .. } | Action::Trash { path: from } => Some(from),
                _ => None,
            })
            .collect();
        assert_eq!(changed, vec![&invoice], "{command}: {actions:?}");
        assert!(!warnings.iter().any(|w| w.contains("matching") || w.contains("Search incomplete")), "{command}: {warnings:?}");
    }

    for command in [
        "delete PDFs in Documents including subfolders",
        "delete PDFs in Documents from subfolders",
        "move PDFs in Documents to Archive including subfolders",
        "sort Documents by type including subfolders",
    ] {
        assert!(
            matches!(plan_with_limit(&h, command, 3), Err(Stop::SearchIncomplete(message)) if message.contains("Entry limit")),
            "{command}"
        );
    }
    assert_eq!(trash_paths(&steps(&h, "delete PDFs in Documents including subfolders")).len(), 33);
    assert_eq!(trash_paths(&steps(&h, "delete PDFs in Documents from subfolders")).len(), 32);
}

#[test]
fn recursive_preview_reports_skipped_trees_and_still_scans_ordinary_subfolders() {
    let h = home();
    let docs = h.roots.home.join("Documents");
    let repo = docs.join("repo");
    fs::create_dir_all(repo.join("build/a/b")).unwrap();
    fs::create_dir_all(repo.join("node_modules/a/b")).unwrap();
    fs::create_dir_all(repo.join("Preview.XcArChIvE/a/b")).unwrap();
    fs::create_dir_all(repo.join("guides")).unwrap();
    fs::create_dir_all(docs.join("Reference/vendor")).unwrap();
    fs::write(repo.join("package.json"), b"{}").unwrap();
    for skipped in ["build", "node_modules", "Preview.XcArChIvE"] {
        for i in 0..32 {
            fs::write(repo.join(format!("{skipped}/a/b/skip-{i}.pdf")), b"x").unwrap();
        }
    }
    let guide = repo.join("guides/guide.pdf");
    let invoice = docs.join("Reference/vendor/invoice.pdf");
    fs::write(&guide, b"a").unwrap();
    fs::write(&invoice, b"b").unwrap();
    for cmd in ["delete PDFs in Documents including subfolders", "move PDFs from Documents to Trash including subfolders"] {
        let Ok(Planned::Steps { actions, warnings, .. }) = plan_with_limit(&h, cmd, 12) else { panic!("expected recursive preview") };
        assert_eq!(trash_paths(&actions), vec![invoice.clone(), guide.clone()]);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains(SKIPPED_TREES_NOTE));
        for name in ["build", "node_modules", "Preview.XcArChIvE"] {
            assert!(warnings[0].contains(name), "{warnings:?}");
        }
    }
}

#[test]
fn naming_a_skipped_folder_or_its_descendant_makes_it_the_scan_root() {
    let h = home();
    let docs = h.roots.home.join("Documents");
    let repo = docs.join("dev/app");
    let dep = repo.join("node_modules/LibraryDocs");
    let build = repo.join("build");
    let env = repo.join(".venv");
    for p in [&dep, &env, &build] {
        fs::create_dir_all(p).unwrap();
        fs::write(p.join("note.pdf"), b"x").unwrap();
    }
    fs::write(repo.join("package.json"), b"{}").unwrap();
    for folder in ["node_modules", ".venv", "Documents/dev/app/build", "Documents/dev/app/node_modules/LibraryDocs"] {
        let result = plan(&h, &format!("list files in {folder}"));
        assert!(matches!(result, Ok(Planned::Info { total: 1, summary, .. }) if !summary.contains(SKIPPED_TREES_NOTE)), "{folder}");
    }
    for cmd in ["open LibraryDocs", "list files in LibraryDocs"] {
        assert!(
            matches!(plan(&h, cmd), Err(Stop::Ambiguous { candidates, reason: AmbiguityReason::IncompleteSearch, .. })
            if candidates == vec![dep.clone()]),
            "{cmd}"
        );
    }
    assert_eq!(trash_paths(&steps(&h, "delete PDFs in ~/Documents/dev/app/build including subfolders")), vec![build.join("note.pdf")]);
}

#[test]
fn missing_destinations_are_created_outside_same_named_skipped_folders() {
    let h = home();
    let docs = h.roots.home.join("Documents");
    let report = h.roots.downloads().join("report.pdf");
    fs::write(&report, b"x").unwrap();
    for rel in
        ["MyApp.app/Contents/Resources", "Report.pages/Data", "Repo/target/debug", "Repo/node_modules/lib/Guides", "Repo/.venv/Resources"]
    {
        fs::create_dir_all(docs.join(rel)).unwrap();
    }
    fs::write(docs.join("Repo/Cargo.toml"), b"").unwrap();
    for name in ["Resources", "Data", "debug", "Guides"] {
        let dest = h.roots.desktop().join(name);
        assert_eq!(
            steps(&h, &format!("move report.pdf to {name}")),
            [Action::CreateFolder { path: dest.clone() }, Action::Move { from: report.clone(), to: dest.join("report.pdf") },]
        );
    }
    let dest = h.roots.downloads().join("Resources");
    assert_eq!(
        steps(&h, "move PDFs in Downloads to Resources"),
        [Action::CreateFolder { path: dest.clone() }, Action::Move { from: report, to: dest.join("report.pdf") },]
    );
}

#[test]
fn destructive_bare_names_do_not_search_inside_skipped_trees() {
    let h = home();
    let docs = h.roots.home.join("Documents");
    let library = docs.join("node_modules/LibraryDocs");
    let resources = docs.join("MyApp.ApP/Contents/Resources");
    fs::create_dir_all(&library).unwrap();
    fs::create_dir_all(&resources).unwrap();
    for nested in [false, true] {
        if nested {
            fs::create_dir_all(library.join("sub/LibraryDocs")).unwrap();
        }
        for cmd in ["delete LibraryDocs", "rename LibraryDocs to Legal", "delete Resources", "open Resources"] {
            assert!(matches!(plan(&h, cmd), Err(Stop::Problem(message)) if message.contains("Couldn't find")), "{cmd}, nested={nested}");
        }
    }
    let explicit = format!("delete {}", library.display());
    assert_eq!(trash_paths(&steps(&h, &explicit)), [library]);
    assert!(matches!(plan(&h, "open Documents/MyApp.ApP/Contents/Resources"), Ok(Planned::Direct { action, .. })
        if action == Action::OpenInFinder { path: resources, reveal: false }));
}

#[test]
fn recursive_changes_cannot_start_inside_hidden_trash_or_package_folders() {
    let h = home();
    let dl = h.roots.downloads();
    for folder in ["_to_delete", ".hidden", "Test.ApP", "Test.ApP/Contents", "Library.photoslibrary"] {
        let dir = dl.join(folder);
        fs::create_dir_all(dir.join("nested")).unwrap();
        fs::write(dir.join("nested/report.pdf"), b"x").unwrap();
        for cmd in [
            format!("sort ~/Downloads/{folder} by type including subfolders"),
            format!("sort ~/Downloads/{folder} by month including subfolders"),
            format!("delete PDFs in ~/Downloads/{folder} including subfolders"),
            format!("delete PDFs in ~/Downloads/{folder} from subfolders"),
            format!("move PDFs in ~/Downloads/{folder} to Documents including subfolders"),
        ] {
            assert!(matches!(plan(&h, &cmd), Err(Stop::Problem(message)) if message.contains("Recursive changes are blocked")), "{cmd}");
        }
    }
    assert!(matches!(plan(&h, "list files in ~/Downloads/Test.ApP/Contents"), Ok(Planned::Info { total: 1, .. })));
}

#[test]
fn package_checks_stop_at_the_configured_root() {
    let mut h = home();
    let workspace = h.roots.desktop().join("Tool.app/Contents/Workspace");
    let package = h.roots.home.join("Library.pages");
    for folder in [&workspace, &package] {
        fs::create_dir_all(folder.join("nested")).unwrap();
        fs::write(folder.join("nested/report.pdf"), b"x").unwrap();
    }
    h.roots = Roots::new(&h.roots.home, None, &[workspace.clone(), package]);

    assert_eq!(trash_paths(&steps(&h, "delete PDFs in Workspace including subfolders")), [workspace.join("nested/report.pdf")]);
    assert!(matches!(plan(&h, "delete PDFs in Library.pages including subfolders"),
        Err(Stop::Problem(message)) if message.contains("Recursive changes are blocked")));

    let skipped = workspace.join("node_modules/LibraryDocs");
    fs::create_dir_all(&skipped).unwrap();
    assert!(matches!(plan(&h, "list files in LibraryDocs"),
        Err(Stop::Ambiguous { candidates, .. }) if candidates == vec![skipped]));
}

#[test]
fn flat_files_do_not_use_the_nested_hint_budget() {
    let h = home();
    let dl = h.roots.downloads();
    fs::create_dir(dl.join("Archive")).unwrap();
    for i in 0..600 {
        fs::write(dl.join(format!("file-{i}.txt")), b"x").unwrap();
    }
    let direct = dl.join("direct.pdf");
    fs::write(&direct, b"x").unwrap();
    fs::write(dl.join("Archive/old.pdf"), b"x").unwrap();
    assert!(matches!(plan(&h, "delete PDFs in Downloads"), Ok(Planned::Steps { actions, warnings, .. })
        if trash_paths(&actions) == vec![direct] && warnings.iter().any(|w| w.starts_with("1 more matching file was left alone"))));
}

#[test]
fn recursive_changes_check_package_and_outside_destinations_through_symlinks() {
    let h = home();
    let dl = h.roots.downloads();
    let package = dl.join("MyApp.app/Contents");
    fs::create_dir_all(&package).unwrap();
    fs::write(dl.join("report.pdf"), b"x").unwrap();
    let category = dl.join("PDFs");
    std::os::unix::fs::symlink(&package, &category).unwrap();
    assert!(
        matches!(plan(&h, "sort Downloads by type including subfolders"), Err(Stop::Problem(message)) if message.contains("macOS package"))
    );
    assert!(
        matches!(plan(&h, "move PDFs in Downloads to ~/Downloads/PDFs including subfolders"), Err(Stop::Problem(message)) if message.contains("macOS package"))
    );

    fs::remove_file(&category).unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(outside.path(), category).unwrap();
    assert!(
        matches!(plan(&h, "sort Downloads by type including subfolders"), Err(Stop::Problem(message)) if message.contains("outside the folders"))
    );
}

#[test]
fn recursive_read_only_summaries_name_skipped_trees() {
    let h = home();
    let docs = h.roots.home.join("Documents");
    fs::create_dir(docs.join("node_modules")).unwrap();
    fs::write(docs.join("node_modules/ignored.pdf"), b"xx").unwrap();
    fs::write(docs.join("report.pdf"), b"x").unwrap();
    for cmd in ["list PDFs in Documents", "count PDFs in Documents", "size of Documents", "find duplicates in Documents"] {
        assert!(
            matches!(plan(&h, cmd), Ok(Planned::Info { summary, .. })
            if summary.contains(SKIPPED_TREES_NOTE) && summary.contains("node_modules")),
            "{cmd}"
        );
    }
}

#[test]
fn skipped_tree_notes_belong_to_the_step_that_skipped_them() {
    let h = home();
    let docs = h.roots.home.join("Documents");
    fs::create_dir(docs.join("node_modules")).unwrap();
    fs::write(docs.join("report.pdf"), b"x").unwrap();
    let first = "delete PDFs in Documents including subfolders";
    let second = "delete PDFs in Pictures including subfolders";
    assert!(matches!(plan(&h, &format!("{first} then {second}")), Err(Stop::Problem(message))
        if message.contains("No files match in Pictures") && !message.contains(SKIPPED_TREES_NOTE)));

    let choices = HashMap::new();
    let history = History::load(h.roots.home.join("history.json"));
    let cmds = [first, second].map(|cmd| (cmd.into(), parse(cmd).unwrap()));
    let result = Planner::new(&h.roots, &choices).plan_shortcut("tidy", &cmds, &history);
    assert!(matches!(result, Ok(Planned::Steps { title, warnings, .. })
        if title == "Tidy: 1 of 2 commands" && warnings.len() == 2 && warnings[0].contains(SKIPPED_TREES_NOTE)
            && warnings[1].starts_with(&format!("Skipped “{second}”: No files match in Pictures.")) && !warnings[1].contains(SKIPPED_TREES_NOTE)));
}

#[test]
fn shortcut_does_not_skip_a_chain_that_could_not_finish_planning() {
    let h = home();
    fs::write(h.roots.home.join("Documents/report.pdf"), b"x").unwrap();
    let choices = HashMap::new();
    let history = History::load(h.roots.home.join("history.json"));
    for chain in [
        "copy PDFs from Pictures to Desktop then create a folder in Desktop named Backup",
        "create a folder in Desktop named Backup then copy PDFs from Pictures to Desktop/Backup",
    ] {
        let commands = [chain, "delete PDFs in Documents"].map(|cmd| (cmd.into(), parse(cmd).unwrap()));
        let result = Planner::new(&h.roots, &choices).plan_shortcut("backup and cleanup", &commands, &history);
        assert!(matches!(result, Err(Stop::Problem(message)) if message == "No files match in Pictures."), "{chain}");
    }
}

#[test]
fn shortcut_with_other_commands_aborts_when_a_chain_drafts_no_actions() {
    let h = home();
    fs::create_dir(h.roots.home.join("Desktop/Backup")).unwrap();
    fs::create_dir(h.roots.home.join("Desktop/Archive")).unwrap();
    let chain = "create a folder in Desktop named Backup then create a folder in Desktop named Archive";
    let choices = HashMap::new();
    let history = History::load(h.roots.home.join("history.json"));
    for commands in [[chain, "create a folder in Desktop named Completed"], ["delete PDFs in Pictures", chain]] {
        let commands = commands.map(|cmd| (cmd.into(), parse(cmd).unwrap()));
        let result = Planner::new(&h.roots, &choices).plan_shortcut("prepare folders", &commands, &history);
        assert!(matches!(result, Err(Stop::Problem(message)) if message == "Backup already exists in Desktop."));
    }
}

#[test]
fn spotlight_failures_after_complete_disk_misses_are_speculative() {
    fn timeout(_: &str, _: &[PathBuf], index: &search::Search) -> Vec<PathBuf> {
        index.spotlight_timeout();
        Vec::new()
    }

    let h = home();
    // Read-only folder resolution will also probe this skipped tree.
    fs::create_dir(h.roots.home.join("Documents/node_modules")).unwrap();
    let choices = HashMap::new();
    let history = History::load(h.roots.home.join("history.json"));
    for cmd in ["delete missing.pdf", "open MissingFolder", "list files in MissingFolder"] {
        let mut planner = Planner::new(&h.roots, &choices);
        planner.index_hits = Some(timeout);
        let control = planner.search.control.clone();
        let result = planner.plan(&parse(cmd).unwrap(), &history);
        assert!(matches!(result, Err(Stop::Problem(message)) if message.starts_with("Couldn't find")), "{cmd}");
        let (_, events) = crate::diagnostics::capture(|| {
            crate::emit_planning_failure(crate::Code::PlanningFailed, &control, control.stats(Duration::ZERO));
        });
        assert_eq!(events.len(), 1, "{cmd}: {events:?}");
        assert!(events[0].contains("code=E_PLANNING_FAILED "), "{cmd}: {events:?}");
    }
}

#[test]
fn incomplete_spotlight_hits_after_a_complete_disk_miss_require_confirmation() {
    fn partial_hit(_: &str, dirs: &[PathBuf], index: &search::Search) -> Vec<PathBuf> {
        let docs = dirs.iter().find(|d| file_name(d) == "Documents").unwrap();
        let candidate = docs.join("report.pdf");
        // The file arrives after the disk walk and can be verified on disk.
        fs::write(&candidate, b"x").unwrap();
        index.issue("Spotlight result limit reached".into());
        vec![candidate]
    }

    let h = home();
    let candidate = h.roots.home.join("Documents/report.pdf");
    let choices = HashMap::new();
    let history = History::load(h.roots.home.join("history.json"));
    let command = "rename report.pdf to final.pdf";
    assert!(matches!(plan(&h, command), Err(Stop::Problem(message)) if message.starts_with("Couldn't find")));

    let mut planner = Planner::new(&h.roots, &choices);
    planner.index_hits = Some(partial_hit);
    let control = planner.search.control.clone();
    let result = planner.plan(&parse(command).unwrap(), &history);
    assert!(matches!(result, Err(Stop::Ambiguous { candidates, reason: AmbiguityReason::IncompleteSearch, note: Some(note), .. })
        if candidates == vec![candidate] && note.contains("Spotlight result limit reached")));
    assert!(control.failed_with(search::Failure::Incomplete));
}

#[test]
fn spotlight_failures_are_recorded_when_rescuing_an_incomplete_walk() {
    fn timeout(_: &str, _: &[PathBuf], index: &search::Search) -> Vec<PathBuf> {
        index.spotlight_timeout();
        Vec::new()
    }

    let h = home();
    fs::create_dir(h.roots.home.join("Documents/Deep")).unwrap();
    let choices = HashMap::new();
    let history = History::load(h.roots.home.join("history.json"));
    let mut planner = Planner::new(&h.roots, &choices);
    planner.search.depth = 0;
    planner.index_hits = Some(timeout);
    let control = planner.search.control.clone();
    assert!(matches!(planner.plan(&parse("delete missing.pdf").unwrap(), &history), Err(Stop::SearchIncomplete(_))));
    assert!(control.failed_with(search::Failure::Depth));
    assert!(control.failed_with(search::Failure::SpotlightTimeout));
}

#[test]
fn skipped_folder_spotlight_fallback_requires_confirmation_and_verifies_hits() {
    fn hits(_: &str, dirs: &[PathBuf], _: &search::Search) -> Vec<PathBuf> {
        let docs = dirs.iter().find(|d| file_name(d) == "Documents").unwrap();
        vec![
            docs.join("node_modules/a/b/c/d/e/LibraryDocs"),
            docs.join("node_modules/Preview.app/LibraryDocs"),
            docs.join("node_modules/missing/LibraryDocs"),
            docs.with_file_name("LibraryDocs"),
        ]
    }

    let h = home();
    let docs = h.roots.home.join("Documents");
    let candidate = docs.join("node_modules/a/b/c/d/e/LibraryDocs");
    for dir in [&candidate, &docs.join("node_modules/Preview.app/LibraryDocs"), &docs.with_file_name("LibraryDocs")] {
        fs::create_dir_all(dir).unwrap();
    }
    let choices = HashMap::new();
    let history = History::load(h.roots.home.join("history.json"));
    for cmd in ["list files in LibraryDocs", "open LibraryDocs"] {
        assert!(matches!(plan(&h, cmd), Err(Stop::Problem(message)) if message.contains("Couldn't find")), "{cmd}");
        let mut planner = Planner::new(&h.roots, &choices);
        planner.index_hits = Some(hits);
        assert!(
            matches!(planner.plan(&parse(cmd).unwrap(), &history),
            Err(Stop::Ambiguous { candidates, reason: AmbiguityReason::IncompleteSearch, .. }) if candidates == vec![candidate.clone()]),
            "{cmd}"
        );
    }
}

#[test]
fn skipped_folder_spotlight_fallback_rejects_symlink_targets_inside_packages() {
    fn hits(_: &str, dirs: &[PathBuf], _: &search::Search) -> Vec<PathBuf> {
        let docs = dirs.iter().find(|d| file_name(d) == "Documents").unwrap();
        vec![docs.join("node_modules/pkg/Tool.app")]
    }

    let h = home();
    let skipped = h.roots.home.join("Documents/node_modules/pkg");
    let link = skipped.join("Tool.app");
    let package = h.roots.desktop().join("Preview.app");
    let inside_package = package.join("Contents/Resources");
    fs::create_dir_all(&skipped).unwrap();
    fs::create_dir_all(&inside_package).unwrap();

    let choices = HashMap::new();
    let history = History::load(h.roots.home.join("history.json"));
    let plan_with_hits = || {
        let mut planner = Planner::new(&h.roots, &choices);
        planner.index_hits = Some(hits);
        planner.plan(&parse("list files in Tool.app").unwrap(), &history)
    };
    std::os::unix::fs::symlink(inside_package, &link).unwrap();
    assert!(matches!(plan_with_hits(), Err(Stop::Problem(message)) if message.contains("Couldn't find")));

    fs::remove_file(&link).unwrap();
    std::os::unix::fs::symlink(package, &link).unwrap();
    assert!(matches!(plan_with_hits(), Err(Stop::Ambiguous { candidates, .. }) if candidates == vec![link]));
}

#[test]
fn per_root_lookup_retains_skipped_folders_and_respects_match_ancestry() {
    let h = home();
    for name in ["A", "B", "C", "D"] {
        fs::create_dir(h.roots.desktop().join(name)).unwrap();
    }
    let dev = h.roots.home.join("Documents/Z/Dev");
    fs::create_dir_all(dev.join("Dev/Dev")).unwrap();
    let project = h.roots.home.join("Pictures/Project");
    let env = project.join(".venv");
    let dep = project.join("node_modules/LibraryDocs");
    fs::create_dir_all(&env).unwrap();
    fs::create_dir_all(&dep).unwrap();
    assert!(matches!(plan_with_limit(&h, "create a folder in dev named new", 4), Ok(Planned::Steps { actions, .. })
        if actions == vec![Action::CreateFolder { path: dev.join("new") }]));
    assert!(matches!(plan_with_limit(&h, "open dev", 4), Ok(Planned::Direct { action, .. })
        if action == Action::OpenInFinder { path: dev.clone(), reveal: false }));
    assert!(
        matches!(plan_with_limit(&h, "delete dev", 4), Err(Stop::Ambiguous { candidates, reason: AmbiguityReason::MultipleMatches, .. })
        if candidates == vec![dev.clone(), dev.join("Dev"), dev.join("Dev/Dev")])
    );
    assert!(matches!(plan_with_limit(&h, "open .venv", 4), Ok(Planned::Direct { action, .. })
        if action == Action::OpenInFinder { path: env.clone(), reveal: false }));
    assert!(matches!(plan_with_limit(&h, "list files in .venv", 4), Ok(Planned::Info { total: 0, .. })));
    assert!(matches!(plan_with_limit(&h, "list files in LibraryDocs", 4),
        Err(Stop::Ambiguous { candidates, reason: AmbiguityReason::IncompleteSearch, .. }) if candidates == vec![dep]));
}

#[test]
fn ordinary_vendor_target_and_pods_folders_remain_searchable() {
    let h = home();
    let docs = h.roots.home.join("Documents");
    for marker in ["Cargo.toml", "go.mod", "Podfile", "package.json"] {
        fs::write(docs.join(marker), b"").unwrap();
    }
    fs::create_dir(docs.join("Project")).unwrap();
    fs::write(docs.join("Project/package.json"), b"{}").unwrap();
    let mut expected = Vec::new();
    for (i, folder) in
        ["Vendor", "Target", "Pods", "build", "dist", "Project/Vendor", "Project/Target", "Project/Pods"].into_iter().enumerate()
    {
        let dir = docs.join(folder);
        fs::create_dir(&dir).unwrap();
        let file = dir.join(format!("invoice-{i}.pdf"));
        fs::write(&file, b"x").unwrap();
        expected.push(file);
    }
    for (i, file) in expected.iter().enumerate() {
        assert_eq!(trash_paths(&steps(&h, &format!("delete invoice-{i}.pdf"))), std::slice::from_ref(file));
    }
    let Ok(Planned::Steps { actions, warnings, .. }) = plan(&h, "delete PDFs in Documents including subfolders") else {
        panic!("expected a preview")
    };
    expected.sort();
    assert_eq!(trash_paths(&actions), expected);
    assert!(!warnings.iter().any(|w| w.contains(SKIPPED_TREES_NOTE)));
}

#[test]
fn scoped_partial_matches_require_confirmation_for_files_and_folders() {
    let h = home();
    let dl = h.roots.downloads();
    fs::create_dir_all(dl.join("ProjectAlpha/Deep")).unwrap();
    fs::write(dl.join("report.pdf"), b"x").unwrap();
    for (name, expected) in [("report.pdf", dl.join("report.pdf")), ("ProjectAlpha", dl.join("ProjectAlpha"))] {
        let cmd = format!("delete {name} from Downloads");
        let result = plan_with_limit(&h, &cmd, 1);
        assert!(
            matches!(&result, Err(Stop::Ambiguous { candidates, reason: AmbiguityReason::IncompleteSearch, .. })
            if candidates == std::slice::from_ref(&expected)),
            "{cmd}: {:?}",
            result.err()
        );
    }
}

#[test]
fn replace_in_stem_handles_unicode_case_length_changes() {
    assert_eq!(replace_in_stem("İ_draft.pdf", "draft", "final").as_deref(), Some("İ_final.pdf"));
    assert_eq!(replace_in_stem("İ_draft.pdf", "İ", "I").as_deref(), Some("I_draft.pdf"));
    assert_eq!(replace_in_stem("İ_draft.pdf", "i", "X"), None);
    assert_eq!(replace_in_stem("İ_draft_i.pdf", "i", "X").as_deref(), Some("İ_draft_X.pdf"));
}

#[test]
fn batch_rename_refuses_allowed_root_folder() {
    let h = home();
    let shots = h.roots.desktop().join("Screenshots");
    fs::create_dir_all(&shots).unwrap();
    let roots = Roots::new(&h.roots.home, Some(shots.clone()), &[]);
    let h = Home { _dir: h._dir, roots };
    assert!(h.roots.is_root(&shots));
    // Unquoted "Screenshots" names the files inside it, which may be renamed.
    for command in [
        "add prefix old- to \"Screenshots\"",
        "add prefix old- to Desktop/Screenshots",
        "add date prefix to \"Screenshots\"",
        "replace Screen with Old in \"Screenshots\"",
        "lowercase names of \"Screenshots\"",
        "add prefix old- to \"Downloads\"",
    ] {
        let result = plan(&h, command);
        assert!(matches!(&result, Err(Stop::Problem(m)) if m.contains("rename an allowed root")), "{command}: {:?}", result.err());
    }
}

#[test]
fn from_subfolders_reaches_only_files_in_them() {
    let h = home();
    let dl = h.roots.downloads();
    fs::create_dir_all(dl.join("Project/docs")).unwrap();
    fs::write(dl.join("bill.pdf"), b"a").unwrap();
    fs::write(dl.join("Project/docs/spec.pdf"), b"b").unwrap();
    fs::write(dl.join("Project/report.pdf"), b"c").unwrap();
    fs::write(dl.join("Project/notes.txt"), b"d").unwrap();
    for command in ["delete the pdfs in downloads from subfolders", "delete PDFs in Downloads only in subfolders"] {
        let mut trashed: Vec<PathBuf> = steps(&h, command)
            .into_iter()
            .map(|a| match a {
                Action::Trash { path } => path,
                other => panic!("{other:?}"),
            })
            .collect();
        trashed.sort();
        assert_eq!(trashed, [dl.join("Project/docs/spec.pdf"), dl.join("Project/report.pdf")], "{command}");
    }
    let moved = steps(&h, "move PDFs from Downloads to Documents from subfolders");
    assert_eq!(moved.iter().filter(|a| matches!(a, Action::Move { .. })).count(), 2, "{moved:?}");
    assert!(dl.join("bill.pdf").exists());
}

#[test]
fn from_subfolders_with_nothing_nested_says_so() {
    let h = home();
    let dl = h.roots.downloads();
    fs::write(dl.join("bill.pdf"), b"a").unwrap();
    assert!(matches!(plan(&h, "delete PDFs in Downloads from subfolders"), Err(Stop::NothingToChange(_))));
}

#[test]
fn a_named_folder_resolves_to_the_nearest_match() {
    let h = home();
    let docs = h.roots.home.join("Documents");
    for i in 0..8 {
        fs::create_dir_all(docs.join(format!("dev/project {i}/scripts/dev"))).unwrap();
    }
    let command = "Create a folder in dev named flagsmith.";
    let result = plan_with_limit(&h, command, 2);
    assert!(
        matches!(result, Ok(Planned::Steps { actions, .. }) if actions == vec![Action::CreateFolder { path: docs.join("dev/flagsmith") }])
    );
    // Two at the same depth still need a choice.
    fs::create_dir_all(h.roots.home.join("Desktop/dev")).unwrap();
    assert!(matches!(plan_with_limit(&h, command, 2), Err(Stop::Ambiguous { candidates, .. }) if candidates.len() == 2));
}

#[test]
fn a_deeper_folder_elsewhere_or_an_unchecked_branch_requires_a_choice() {
    let h = home();
    let shallow = h.roots.downloads().join("Dev");
    let deeper = h.roots.home.join("Documents/Work/Dev");
    fs::create_dir_all(shallow.join("projects")).unwrap();
    fs::create_dir_all(&deeper).unwrap();
    let command = "create a folder in dev/projects named flagsmith";
    assert!(
        matches!(plan_with_limit(&h, command, 3), Err(Stop::Ambiguous { name, candidates, reason: AmbiguityReason::MultipleMatches, .. })
        if name == "dev" && candidates == vec![deeper, shallow.clone()])
    );
    assert!(
        matches!(plan_with_limit(&h, command, 2), Err(Stop::Ambiguous { name, candidates, reason: AmbiguityReason::IncompleteSearch, .. })
        if name == "dev" && candidates == vec![shallow.clone()])
    );

    let choices = HashMap::from([("dev".into(), shallow.clone())]);
    let history = History::load(h.roots.home.join("history.json"));
    let mut planner = Planner::new(&h.roots, &choices);
    planner.search.limit = 2;
    let result = planner.plan(&parse(command).unwrap(), &history);
    assert!(matches!(result, Ok(Planned::Steps { actions, .. })
        if actions == vec![Action::CreateFolder { path: shallow.join("projects/flagsmith") }]));
}

#[test]
fn exact_sources_and_new_destinations_ignore_unrelated_file_volume() {
    let h = home();
    let docs = h.roots.home.join("Documents");
    let from = h.roots.downloads().join("report.pdf");
    fs::write(&from, b"a").unwrap();
    fs::create_dir(docs.join("Archive")).unwrap();
    fs::create_dir(docs.join("Dev")).unwrap();
    for i in 0..8 {
        fs::write(docs.join(format!("Archive/f{i}.txt")), b"a").unwrap();
    }
    let existing = plan_with_limit(&h, "move report.pdf to dev", 2);
    assert!(matches!(existing, Ok(Planned::Steps { actions, .. })
        if actions == vec![Action::Move { from: from.clone(), to: docs.join("Dev/report.pdf") }]));

    let new = h.roots.desktop().join("NewFolder");
    let missing = plan_with_limit(&h, "move report.pdf to NewFolder", 2);
    assert!(matches!(missing, Ok(Planned::Steps { actions, .. })
        if actions == vec![Action::CreateFolder { path: new.clone() }, Action::Move { from: from.clone(), to: new.join("report.pdf") }]));

    // A deeper exact duplicate still prevents an automatic file move.
    let duplicate = docs.join("Archive/report.pdf");
    fs::write(&duplicate, b"a").unwrap();
    assert!(matches!(plan_with_limit(&h, "move report.pdf to dev", 2), Err(Stop::Ambiguous { candidates, .. })
        if candidates == vec![duplicate, from]));
}

#[test]
fn incomplete_folder_lookup_falls_back_and_never_assumes_absence() {
    let h = home();
    let dev = h.roots.home.join("Documents/Dev");
    let other = h.roots.desktop().join("Other");
    fs::create_dir(&dev).unwrap();
    fs::create_dir(&other).unwrap();
    // The BFS limit is shared across roots, and the fallback scans them individually.
    assert!(matches!(plan_with_limit(&h, "create a folder in dev named flagsmith", 1), Ok(Planned::Steps { actions, .. })
        if actions == vec![Action::CreateFolder { path: dev.join("flagsmith") }]));

    let from = h.roots.downloads().join("report.pdf");
    fs::write(&from, b"a").unwrap();
    fs::create_dir(other.join("subfolder")).unwrap();
    let command = Intent::Move { src: Source::item(from.to_str().unwrap()), to: "NewFolder".into() };
    let choices = HashMap::new();
    let history = History::load(h.roots.home.join("history.json"));
    let mut planner = Planner::new(&h.roots, &choices);
    planner.search.limit = 1;
    assert!(matches!(planner.plan(&command, &history), Err(Stop::SearchIncomplete(message)) if message.contains("Entry limit")));
}

#[test]
fn bare_folders_resolve_for_finder_size_and_item_commands() {
    let h = home();
    let dev = h.roots.home.join("Documents/dev");
    fs::create_dir(&dev).unwrap();
    fs::write(dev.join("hello.txt"), b"a").unwrap();
    for i in 0..8 {
        fs::write(h.roots.downloads().join(format!("f{i}.txt")), b"a").unwrap();
    }

    assert!(matches!(plan_with_limit(&h, "open dev", 1), Ok(Planned::Direct { action, .. })
        if action == Action::OpenInFinder { path: dev.clone(), reveal: false }));
    assert!(matches!(plan_with_limit(&h, "size of dev", 1), Ok(Planned::Info { title, summary, .. })
        if title.starts_with("dev uses") && summary.starts_with("1 file in total")));
    assert!(matches!(plan_with_limit(&h, "rename dev to Projects", 1), Ok(Planned::Steps { actions, .. })
        if actions == vec![Action::Rename { path: dev.clone(), new_name: "Projects".into() }]));

    fs::write(dev.join("dev"), b"a").unwrap();
    assert!(matches!(plan_with_limit(&h, "open dev", 1), Err(Stop::Ambiguous { candidates, .. })
        if candidates == vec![dev.clone(), dev.join("dev")]));
    fs::remove_file(dev.join("dev")).unwrap();
    fs::write(dev.join("bye.txt"), b"b").unwrap();
    assert!(matches!(plan_with_limit(&h, "size of dev", 1), Ok(Planned::Info { title, summary, .. })
        if title == "dev uses at least 1 B" && summary.contains("Partial results") && summary.contains("At least 1 file")));
}

#[test]
fn size_of_a_directory_symlink_groups_files_with_or_without_a_cached_ancestor() {
    let h = home();
    let docs = h.roots.home.join("Documents");
    let dev = docs.join("Dev");
    let link = h.roots.downloads().join("dev-link");
    fs::create_dir_all(dev.join("src")).unwrap();
    fs::write(dev.join("src/main.rs"), b"hello").unwrap();
    fs::write(dev.join("readme.txt"), b"hi").unwrap();
    std::os::unix::fs::symlink(&dev, &link).unwrap();

    let choices = HashMap::new();
    let intent = Intent::Size { folder: link.to_str().unwrap().into() };
    let history = History::load(h.roots.home.join("history.json"));
    for cached in [false, true] {
        let planner = Planner::new(&h.roots, &choices);
        if cached {
            assert_eq!(planner.search.entries(&docs).len(), 4);
        }
        assert!(matches!(planner.plan(&intent, &history), Ok(Planned::Info { title, summary, rows, total: 2, .. })
            if title == "dev-link uses 7 B" && summary.starts_with("2 files in total")
                && rows[0].name == "src" && rows[0].detail == "5 B · folder"
                && rows[1].name == "readme.txt" && rows[1].detail == "2 B"));
    }
}

#[test]
fn large_folder_contents_do_not_block_finder_or_size_but_rename_requires_a_choice() {
    let h = home();
    let dev = h.roots.home.join("Documents/dev");
    for index in 0..4 {
        let subdir = dev.join(format!("project{index}/src/deep"));
        fs::create_dir_all(&subdir).unwrap();
        fs::write(subdir.join("main.rs"), b"").unwrap();
    }
    let history = History::load(h.roots.home.join("history.json"));
    let choices = HashMap::new();
    for (limit, depth) in [(2, search::MAX_DEPTH), (search::MAX_ENTRIES, 0)] {
        let plan = |command| {
            let mut planner = Planner::new(&h.roots, &choices);
            planner.search.limit = limit;
            planner.search.depth = depth;
            planner.plan(&parse(command).unwrap(), &history)
        };
        assert!(matches!(plan("open dev"), Ok(Planned::Direct { action, .. })
            if action == Action::OpenInFinder { path: dev.clone(), reveal: false }));
        assert!(matches!(plan("size of dev"), Ok(Planned::Info { title, summary, .. })
            if title.starts_with("dev uses") && summary.contains("Partial results")));
        assert!(matches!(plan("rename dev to Projects"), Err(Stop::Ambiguous {
            candidates, reason: AmbiguityReason::IncompleteSearch, ..
        }) if candidates == vec![dev.clone()]));
    }
}

#[test]
fn a_search_root_is_also_visible_by_its_folder_name() {
    let mut h = home();
    let captures = h.roots.home.join("Documents/Captures");
    fs::create_dir_all(&captures).unwrap();
    h.roots = Roots::new(&h.roots.home, Some(captures.clone()), &[]);
    assert!(h.roots.named("Captures").is_none());
    let report = h.roots.downloads().join("report.pdf");
    fs::write(&report, b"").unwrap();
    assert_eq!(steps(&h, "create a folder in Captures named Q1"), vec![Action::CreateFolder { path: captures.join("Q1") }]);
    assert_eq!(steps(&h, "move report.pdf to Captures"), vec![Action::Move { from: report, to: captures.join("report.pdf") }]);
    assert!(matches!(plan(&h, "open Captures"), Ok(Planned::Direct { action, .. })
        if action == Action::OpenInFinder { path: captures.clone(), reveal: false }));

    let other = h.roots.home.join("Documents/Archive/Captures");
    fs::create_dir_all(&other).unwrap();
    for command in ["create a folder in Captures named Q1", "move report.pdf to Captures", "open Captures"] {
        let result = plan(&h, command);
        assert!(
            matches!(&result, Err(Stop::Ambiguous { candidates, reason: AmbiguityReason::MultipleMatches, .. })
            if candidates == &[other.clone(), captures.clone()]),
            "{command}: {:?}",
            result.err()
        );
    }
}

#[test]
fn a_nested_search_root_can_match_with_a_small_lookup_budget() {
    let mut h = home();
    let captures = h.roots.home.join("Documents/Work/Captures");
    for i in 0..4 {
        fs::create_dir_all(captures.join(format!("project{i}/src"))).unwrap();
    }
    h.roots = Roots::new(&h.roots.home, Some(captures.clone()), &[]);
    assert!(matches!(plan_with_limit(&h, "create a folder in Captures named Q1", 1), Ok(Planned::Steps { actions, .. })
        if actions == vec![Action::CreateFolder { path: captures.join("Q1") }]));
    assert!(matches!(plan_with_limit(&h, "open Captures", 1), Ok(Planned::Direct { action, .. })
        if action == Action::OpenInFinder { path: captures.clone(), reveal: false }));
    assert!(matches!(plan_with_limit(&h, "copy Captures to Desktop", 1),
        Err(Stop::Ambiguous { candidates, reason: AmbiguityReason::IncompleteSearch, .. }) if candidates == vec![captures]));
}

#[test]
fn destructive_item_commands_keep_same_named_descendants_as_candidates() {
    let h = home();
    let dev = h.roots.home.join("Documents/dev");
    let nested = dev.join("project/dev");
    fs::create_dir_all(&nested).unwrap();
    for command in ["delete dev", "rename dev to Projects", "move dev to Downloads", "zip dev"] {
        let result = plan(&h, command);
        assert!(
            matches!(&result, Err(Stop::Ambiguous { candidates, reason: AmbiguityReason::MultipleMatches, .. })
            if candidates == &[dev.clone(), nested.clone()]),
            "{command}: {:?}",
            result.err()
        );
    }
}

#[test]
fn named_symlinks_only_match_when_the_target_is_allowed() {
    let mut h = home();
    let outside = tempfile::tempdir().unwrap();
    let docs = h.roots.home.join("Documents");
    let dl = h.roots.downloads();
    let private = docs.join("Private");
    fs::create_dir(&private).unwrap();
    fs::write(outside.path().join("external.pdf"), b"").unwrap();
    fs::write(private.join("secret.pdf"), b"").unwrap();
    fs::write(docs.join("report.pdf"), b"").unwrap();
    let allowed = dl.join("shortcut.pdf");
    let excluded_link = private.join("public-link.pdf");
    let excluded_root_link = private.join("docs-link");
    let outside_root_link = outside.path().join("docs-link");
    for (target, link) in [
        (outside.path().join("external.pdf"), dl.join("external.pdf")),
        (private.join("secret.pdf"), dl.join("secret.pdf")),
        (docs.join("report.pdf"), allowed.clone()),
        (docs.join("report.pdf"), excluded_link.clone()),
        (docs.clone(), excluded_root_link.clone()),
        (docs.clone(), outside_root_link.clone()),
    ] {
        std::os::unix::fs::symlink(target, link).unwrap();
    }
    h.roots = h.roots.with_exclusions(&[private]);
    assert_eq!(steps(&h, "delete shortcut.pdf from Downloads"), vec![Action::Trash { path: allowed }]);
    for command in ["delete external.pdf from Downloads", "delete secret.pdf from Downloads", "open Downloads/external.pdf"] {
        assert!(matches!(plan(&h, command), Err(Stop::Problem(_))), "{command}");
    }
    let choices = HashMap::new();
    let planner = Planner::new(&h.roots, &choices);
    assert!(planner.resolve_any(excluded_link.to_str().unwrap()).is_err());
    let history = History::load(h.roots.home.join("history.json"));
    for link in [&excluded_root_link, &outside_root_link] {
        let name = link.to_str().unwrap().to_owned();
        assert!(planner.resolve_any(&name).is_err());
        let intents = [
            Intent::Copy { src: Source::Item { name: name.clone(), folder: None }, to: "Desktop".into() },
            Intent::Size { folder: name.clone() },
            Intent::CreateFolders { names: vec!["New".into()], parent: Some(name) },
        ];
        for intent in &intents {
            assert!(
                matches!(Planner::new(&h.roots, &choices).plan(intent, &history), Err(Stop::Problem(message))
                if message.contains("outside the folders")),
                "{intent:?}"
            );
        }
    }
    assert!(
        planner
            .verified_hits(
                vec![excluded_link, excluded_root_link, outside_root_link],
                &[docs, outside.path().to_path_buf()],
                false,
                &planner.search,
            )
            .is_empty()
    );
}

#[test]
fn hidden_and_trash_folders_are_not_reported_as_skipped() {
    let h = home();
    let dl = h.roots.downloads();
    for dir in ["_to_delete", ".git", ".vscode"] {
        fs::create_dir_all(dl.join(dir)).unwrap();
        fs::write(dl.join(dir).join("old.pdf"), b"x").unwrap();
    }
    fs::create_dir_all(dl.join("app/node_modules/lib")).unwrap();
    fs::write(dl.join("app/package.json"), b"{}").unwrap();
    fs::write(dl.join("app/node_modules/lib/x.pdf"), b"x").unwrap();
    fs::write(dl.join("report.pdf"), b"x").unwrap();
    let Ok(Planned::Steps { warnings, .. }) = plan(&h, "delete PDFs in Downloads including subfolders") else { panic!() };
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("node_modules"), "{warnings:?}");
    for hidden in ["_to_delete", ".git", ".vscode"] {
        assert!(!warnings[0].contains(hidden), "{warnings:?}");
    }
    fs::remove_dir_all(dl.join("app")).unwrap();
    let Ok(Planned::Info { summary, title, .. }) = plan(&h, "size of Downloads") else { panic!() };
    assert!(!summary.contains(SKIPPED_TREES_NOTE) && !title.contains("at least"), "{title} / {summary}");
}

#[test]
fn sorting_never_follows_a_category_folder_into_trash_or_a_package() {
    let h = home();
    let dl = h.roots.downloads();
    fs::write(dl.join("report.pdf"), b"x").unwrap();
    fs::create_dir_all(dl.join("_to_delete")).unwrap();
    std::os::unix::fs::symlink(dl.join("_to_delete"), dl.join("PDFs")).unwrap();
    assert!(matches!(plan(&h, "sort Downloads by type"), Err(Stop::Problem(_))));
    fs::remove_file(dl.join("PDFs")).unwrap();
    fs::create_dir_all(dl.join("Tool.app/Contents")).unwrap();
    std::os::unix::fs::symlink(dl.join("Tool.app/Contents"), dl.join("PDFs")).unwrap();
    assert!(matches!(plan(&h, "sort Downloads by type"), Err(Stop::Problem(_))));
}

#[test]
fn an_existing_excluded_folder_is_not_nothing_to_change() {
    let mut h = home();
    let backup = h.roots.home.join("Desktop/Backup");
    fs::create_dir_all(&backup).unwrap();
    h.roots = h.roots.with_exclusions(std::slice::from_ref(&backup));
    assert!(matches!(plan(&h, "create a folder named Backup"), Err(Stop::Problem(_))));
}
