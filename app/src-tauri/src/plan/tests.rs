use super::*;
use crate::parse::parse;
use std::fs;

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

#[test]
fn opening_an_allowed_root_folder_plans_a_finder_reveal() {
    let h = home();
    let Ok(Planned::Direct { action, .. }) = plan(&h, "open the Documents folder") else { panic!("expected a direct reveal") };
    assert_eq!(action, Action::OpenInFinder { path: h.roots.home.join("Documents"), reveal: false });
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
    assert!(search.status().unwrap().contains("Spotlight timed out"));
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
    let choices = HashMap::new();
    let history = History::load(h.roots.home.join("history.json"));
    for command in ["delete Aeroguard files from Downloads", "rename AEROGUARD_1.pdf to final.pdf"] {
        let mut planner = Planner::new(&h.roots, &choices);
        planner.search.limit = 1;
        assert!(matches!(planner.plan(&parse(command).unwrap(), &history),
            Err(Stop::Problem(message)) if message.contains("Search incomplete") && message.contains("entry limit")));
    }
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
        Err(Stop::Problem(message)) if message.contains("Search incomplete")));
    assert!(dl.join("AEROGUARD_1.pdf").exists());
}

#[test]
fn exact_and_similar_matching_share_one_scan() {
    let h = home();
    fs::write(h.roots.downloads().join("Aeroguard Case.pdf"), b"a").unwrap();
    let choices = HashMap::new();
    let planner = Planner::new(&h.roots, &choices);
    let dirs = vec![h.roots.downloads()];
    assert!(planner.find_entries("aeroguard case pdf", false, &dirs).is_empty());
    let visits = planner.search.control.visited.load(std::sync::atomic::Ordering::Relaxed);
    assert_eq!(planner.find_similar("aeroguard case pdf", &dirs).len(), 1);
    assert_eq!(planner.search.control.visited.load(std::sync::atomic::Ordering::Relaxed), visits);
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
    assert_eq!(
        steps(&h, "copy \"Other Folder\" to Desktop"),
        vec![Action::Copy { from: root.clone(), to: h.roots.home.join("Desktop/Other Folder") }]
    );
    for command in ["delete folder Other Folder", "move Other Folder into Documents", "rename Other Folder to New Name"] {
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
    assert!(matches!(plan(&h, "sort Downloads by type"), Err(Stop::Problem(message)) if message.starts_with("Nothing to change.")));
    // The explicit opt-in keeps the historical recursive behaviour.
    let actions = steps(&h, "sort Downloads by type including subfolders");
    assert_eq!(actions, vec![Action::Move { from: dl.join("nested/report.pdf"), to: dl.join("PDFs/report 2.pdf") }]);
    let month = created(&dl.join("nested/report.pdf")).unwrap().format("%Y-%m").to_string();
    fs::create_dir(dl.join(&month)).unwrap();
    fs::rename(dl.join("nested/report.pdf"), dl.join(&month).join("report.pdf")).unwrap();
    fs::remove_file(dl.join("PDFs/report.pdf")).unwrap();
    assert!(matches!(plan(&h, "sort Downloads by month"), Err(Stop::Problem(message)) if message.starts_with("Nothing to change.")));
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
        Err(Stop::Ambiguous { name, candidates }) => {
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
    assert_eq!(planner.usable_hits(hits, &[docs]), vec![deep]);
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
    assert_eq!(title, "Run my usuals: 2 commands");
    assert!(
        actions.iter().any(|a| matches!(a, Action::Move { to, .. } if to.ends_with("Screenshots/Screenshot 2026-10-28 at 10.00.00.png")))
    );
    assert!(warnings.iter().any(|w| w.starts_with("“delete installers from Downloads”")), "{warnings:?}");
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
// fresh temp home; actions are never executed. Spotlight may be queried,
// but only hits inside the fixture roots can be returned.

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
    assert!(matches!(plan(&h, "move old.pdf to Desktop"), Err(Stop::Problem(_))));
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
            Err(Stop::Problem(m)) => assert!(m.starts_with("No files match"), "{cmd}: {m}"),
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
