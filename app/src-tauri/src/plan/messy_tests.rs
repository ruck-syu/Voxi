use super::*;
use crate::parse::corpus_tests::{REGRESSION_DATE, pin_regression_date, regression_cases, xorshift32};
use chrono::TimeZone;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::{Instant, SystemTime};

const ENTRY_LIMIT: usize = 192;
const DIRECT_LIMIT: usize = 5;

struct LockedDir {
    path: PathBuf,
    original: fs::Permissions,
}

impl LockedDir {
    fn new(path: PathBuf) -> Self {
        let original = fs::metadata(&path).unwrap().permissions();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
        Self { path, original }
    }
}

impl Drop for LockedDir {
    fn drop(&mut self) {
        let _ = fs::set_permissions(&self.path, self.original.clone());
    }
}

struct MessyMac {
    _dir: tempfile::TempDir,
    roots: Roots,
}

fn regression_noon() -> DateTime<Local> {
    Local.from_local_datetime(&REGRESSION_DATE.and_hms_opt(12, 0, 0).unwrap()).single().unwrap()
}

#[test]
fn vocab_wordings_plan_existing_pdf_and_archive_actions() {
    let fixture = MessyMac::new();
    let roots = fixture.roots();
    let scans = roots.downloads().join("Scans");
    fs::create_dir(&scans).unwrap();
    let image = scans.join("receipts.png");
    fs::write(&image, include_bytes!("../../icons/32x32.png")).unwrap();
    fs::write(scans.join("unrelated.png"), b"image").unwrap();
    let report = roots.home.join("Desktop/report.pdf");
    pdf::images_to_pdf(&vec![image.clone(); 5], &report, &|| Ok(())).unwrap();

    for (wording, canonical) in [
        ("make one PDF from images in Scans named receipts", "make a PDF from images in Scans named receipts"),
        ("break report.pdf into pages", "split report.pdf into pages"),
        ("pull pages 2 to 5 out of report.pdf", "extract pages 2 to 5 from report.pdf"),
        ("uncompress archive.zip into Downloads/archive", "unzip archive.zip into Downloads/archive"),
        ("make one PDF from images in Scans called receipts.pdf", "make a PDF from images in Scans called receipts.pdf"),
    ] {
        assert_eq!(action_plan(&fixture, wording, ENTRY_LIMIT), action_plan(&fixture, canonical, ENTRY_LIMIT), "{wording}");
    }
}

#[test]
fn vocab_preserves_type_aliases_in_planned_actions() {
    let fixture = MessyMac::new();
    let scans = fixture.roots().downloads().join("Scans");
    fs::create_dir(&scans).unwrap();
    for name in ["receipt.jpg", "receipt.jpeg", "receipt.png", "report.pdf"] {
        fs::write(scans.join(name), b"scan").unwrap();
    }
    crate::vocab::with_test_pack(
        r#"{
        "schema":1,"version":1,"minAppVersion":"0.0.0","rules":[{
            "id":"copy-filtered","intent":"copy",
            "pattern":"^duplicate (.+) toward (.+)$","replace":"copy $1 to $2",
            "examples":[{"input":"duplicate JPGs in Scans toward Desktop","expect":"copy JPGs in Scans to Desktop"}]
        }]
    }"#,
        || {
            let actions = action_plan(&fixture, "duplicate JPGs in Scans toward Desktop", ENTRY_LIMIT);
            let names: std::collections::BTreeSet<_> = actions
                .iter()
                .map(|action| {
                    let Action::Copy { from, .. } = action else { panic!("{action:?}") };
                    from.file_name().unwrap().to_str().unwrap()
                })
                .collect();
            assert_eq!(names, ["receipt.jpeg", "receipt.jpg"].into());
        },
    );
}

#[test]
fn vocab_alias_cannot_plan_trash_but_can_plan_an_explicit_folder_path() {
    let fixture = MessyMac::new();
    let roots = fixture.roots();
    let report = roots.downloads().join("vocab-report.pdf");
    fs::write(&report, b"report").unwrap();
    assert!(!roots.home.join("Desktop/Trash").exists());

    crate::vocab::with_test_pack(
        r#"{
        "schema":1,"version":1,"minAppVersion":"0.0.0","rules":[{
            "id":"relay","intent":"move",
            "pattern":"^relay (.+) toward (.+)$","replace":"move $1 to $2",
            "examples":[{"input":"relay vocab-report.pdf toward Documents","expect":"move vocab-report.pdf to Documents"}]
        },{
            "id":"check","intent":"move",
            "pattern":"^check (.+) for (.+) aged (.+)$","replace":"move $1 to $2 $3",
            "examples":[{"input":"check PDFs in Downloads for Desktop aged older than 90 days","expect":"move PDFs in Downloads to Desktop older than 90 days"}]
        }]
    }"#,
        || {
            assert!(parse("relay vocab-report.pdf toward Trash").is_err());
            assert!(parse("check PDFs in Downloads for Trash aged older than 90 days").is_err());
            assert!(parse("check trash for PDFs in Downloads older than 90 days").is_err());
            for destination in ["Documents", "Documents/Trash"] {
                let actions = action_plan(&fixture, &format!("relay vocab-report.pdf toward {destination}"), ENTRY_LIMIT);
                assert!(actions.iter().any(|action| matches!(action, Action::Move { from, .. } if from == &report)));
                assert!(!actions.iter().any(|action| matches!(action, Action::Trash { .. })));
            }
        },
    );
    // The same filtered transfer remains available as an explicit command.
    let intent = parse("move PDFs in Downloads to Trash older than 90 days").unwrap();
    assert!(matches!(intent, Intent::Move { .. }));
    let choices = HashMap::new();
    let mut planner = Planner::new(roots, &choices);
    planner.now = Local::now() + chrono::TimeDelta::days(91);
    let planned = planner.plan(&intent, &History::load(roots.home.join("history.json"))).unwrap();
    assert!(matches!(planned, Planned::Steps { actions, .. } if actions.iter().any(|action|
        matches!(action, Action::Trash { path } if path == &report))));
}

#[test]
fn vocab_numbering_and_spoken_character_replacement_plan_renames() {
    let fixture = MessyMac::new();
    let trip = fixture.roots().downloads().join("Vocab Trip");
    fs::create_dir(&trip).unwrap();
    let image = trip.join("beach photo.jpg");
    fs::write(&image, b"image").unwrap();
    crate::vocab::with_test_pack(
        r#"{
        "schema":1,"version":1,"minAppVersion":"0.0.0","rules":[{
            "id":"enumerate","intent":"number",
            "pattern":"^enumerate (.+) like (.+)$","replace":"number $1 as $2",
            "examples":[{"input":"enumerate images in Vocab Trip like trip-001","expect":"number images in Vocab Trip as trip-001"}]
        },{
            "id":"substitute","intent":"replace_in_names",
            "pattern":"^substitute (.+) for (.+) across (.+)$","replace":"replace $1 with $2 in $3",
            "examples":[{"input":"substitute spaces for dashes across files in Vocab Trip","expect":"replace spaces with dashes in files in Vocab Trip"}]
        }]
    }"#,
        || {
            for (command, new_name) in [
                ("enumerate images in Vocab Trip like trip-001", "trip-001.jpg"),
                ("substitute spaces for dashes across files in Vocab Trip", "beach-photo.jpg"),
            ] {
                assert_eq!(
                    action_plan(&fixture, command, ENTRY_LIMIT),
                    [Action::Rename { path: image.clone(), new_name: new_name.into() }]
                );
            }
        },
    );
}

#[test]
fn demo_sort_scopes_lookup_away_from_unreadable_pictures() {
    let fixture = MessyMac::new();
    let base = &fixture.roots().home;
    let demo = base.join("Downloads/Voxi Demo");
    let blocked = base.join("Pictures/Photo Booth Library");
    fs::create_dir(&demo).unwrap();
    fs::create_dir(&blocked).unwrap();
    for name in ["invoice.pdf", "photo.png", "clip.mp4"] {
        fs::write(demo.join(name), b"demo").unwrap();
    }
    let _locked = LockedDir::new(blocked.clone());
    if fs::read_dir(&blocked).is_err_and(|err| err.kind() == std::io::ErrorKind::PermissionDenied) {
        assert!(matches!(fixture.plan_command("sort Voxi Demo by type", ENTRY_LIMIT),
            Err(Stop::Ambiguous { candidates, reason: AmbiguityReason::IncompleteSearch, note: Some(note), .. })
                if candidates == [demo.clone()] && note.contains("Photo Booth Library")));
    }
    for command in ["sort ~/Downloads/Voxi Demo by type", "sort Voxi Demo in Downloads by type"] {
        let control = search::Control::default();
        let result = fixture.plan_with_control(command, control.clone());
        assert!(
            matches!(&result, Ok(Planned::Steps { actions, .. })
            if actions.iter().filter(|action| matches!(action, Action::Move { from, to }
                if from.parent() == Some(demo.as_path()) && to.starts_with(&demo))).count() == 3),
            "{command}: {:?}",
            result.err()
        );
        assert!(!control.failed_with(search::Failure::Unreadable));
        assert_eq!(control.stats(Duration::ZERO).unreadable, 0);
    }
}

#[test]
fn located_sort_reuses_directory_reads_for_both_names_and_file_expansion() {
    let fixture = MessyMac::new();
    let roots = fixture.roots();
    let demo = roots.downloads().join("Work Bench/Batch/Voxi Demo");
    fs::create_dir_all(&demo).unwrap();
    fs::write(demo.join("photo.png"), b"photo").unwrap();

    let baseline = search::Search::default().with_roots(roots);
    assert!(matches!(baseline.nearest_dirs(&roots.search_roots(), "Voxi Demo in Work Bench"), search::NameLookup::NotFound(_)));
    let control = search::Control::default();
    let result = fixture.plan_with_control("sort Voxi Demo in Work Bench by type", control.clone());
    assert!(matches!(result, Ok(Planned::Steps { .. })));
    assert_eq!(control.stats(Duration::ZERO).visited, baseline.control.stats(Duration::ZERO).visited);
}

#[test]
fn incomplete_literal_lookup_requires_confirmation_for_a_known_match() {
    let fixture = MessyMac::new();
    let literal = fixture.roots().home.join("Documents/Voxi Demo in Downloads");
    let blocked = fixture.roots().home.join("Pictures/Photo Booth Library");
    fs::create_dir(&literal).unwrap();
    fs::create_dir(&blocked).unwrap();
    let _locked = LockedDir::new(blocked.clone());
    if fs::read_dir(&blocked).is_err_and(|err| err.kind() == std::io::ErrorKind::PermissionDenied) {
        assert!(matches!(
            fixture.plan_command("sort Voxi Demo in Downloads by type", ENTRY_LIMIT),
            Err(Stop::Ambiguous { candidates, reason: AmbiguityReason::IncompleteSearch, note: Some(note), .. })
                if candidates == [literal] && note.contains("Photo Booth Library")
        ));
    }
}

#[test]
fn quoted_folder_location_parts_keep_command_words() {
    let fixture = MessyMac::new();
    let roots = fixture.roots();
    let from = roots.home.join("Documents/invoice.pdf");
    for folder in ["Receipts by Month", "My Stuff Here", "Q1 2nd Half", "Items called Misc"] {
        let dest = roots.downloads().join(folder);
        fs::create_dir(&dest).unwrap();
        let command = format!(r#"move invoice.pdf to "{folder}" in Downloads"#);
        let actions = action_plan(&fixture, &command, ENTRY_LIMIT);
        assert_eq!(actions, [Action::Move { from: from.clone(), to: dest.join("invoice.pdf") }]);
    }
    let quoted_parent = roots.downloads().join("Items called Misc/Receipts");
    fs::create_dir(&quoted_parent).unwrap();
    let actions = action_plan(&fixture, r#"copy invoice.pdf to Receipts in "Items called Misc""#, ENTRY_LIMIT);
    assert_eq!(actions, [Action::Copy { from, to: quoted_parent.join("invoice.pdf") }]);

    let path = roots.downloads().join("Receipts by Month/report.pdf");
    fs::write(&path, b"pdf").unwrap();
    let actions = action_plan(&fixture, r#"sort "Receipts by Month" in Downloads by type"#, ENTRY_LIMIT);
    assert!(matches!(actions.last(), Some(Action::Move { from, to }) if *from == path && to.starts_with(path.parent().unwrap())));
}

#[test]
fn unquoted_folder_location_uses_an_existing_literal_name() {
    let fixture = MessyMac::new();
    let roots = fixture.roots();
    let from = roots.desktop().join("report.pdf");
    // An ambiguous parent name with no matching child still lets the
    // literal folder win.
    fs::create_dir(roots.downloads().join("2024")).unwrap();
    fs::create_dir(roots.home.join("Pictures/2024")).unwrap();
    for name in ["Things in Progress", "Archive in 2024"] {
        let folder = roots.home.join("Documents").join(name);
        fs::create_dir(&folder).unwrap();
        let command = format!("move report.pdf to {name}");
        let actions = action_plan(&fixture, &command, ENTRY_LIMIT);
        assert_eq!(actions, [Action::Move { from: from.clone(), to: folder.join("report.pdf") }]);

        let photo = folder.join("photo.png");
        fs::write(&photo, b"photo").unwrap();
        for command in [format!("sort {name} by type"), format!(r#"sort "{name}" by type"#)] {
            let actions = action_plan(&fixture, &command, ENTRY_LIMIT);
            assert!(matches!(actions.last(), Some(Action::Move { from, to }) if *from == photo && to.starts_with(&folder)));
        }
    }

    let scoped = roots.home.join("Pictures/2024/Archive");
    let literal = roots.home.join("Documents/Archive in 2024");
    fs::create_dir(&scoped).unwrap();
    for command in ["move report.pdf to Archive in 2024", "sort Archive in 2024 by type"] {
        assert!(
            matches!(
                fixture.plan_command(command, ENTRY_LIMIT),
                Err(Stop::Ambiguous { name, candidates, .. })
                    if name == "Archive in 2024" && candidates == [literal.clone(), scoped.clone()]
            ),
            "{command}"
        );
    }
}

#[test]
fn chain_binds_a_located_destination_to_the_folder_it_creates() {
    let fixture = MessyMac::new();
    let roots = fixture.roots();
    let folder = roots.home.join("Documents/Archive in 2024");
    let command = r#"create a folder called "Archive in 2024" in Documents and move report.pdf to Archive in 2024"#;
    let actions = action_plan(&fixture, command, ENTRY_LIMIT);
    assert_eq!(
        actions,
        [
            Action::CreateFolder { path: folder.clone() },
            Action::Move { from: roots.desktop().join("report.pdf"), to: folder.join("report.pdf") }
        ]
    );
}

#[test]
fn literal_located_destination_uses_a_configured_nickname() {
    let mut fixture = MessyMac::new();
    let wip = fixture.roots().home.join("Documents/WIP");
    fs::create_dir(&wip).unwrap();
    fs::create_dir(fixture.roots().downloads().join("Progress")).unwrap();
    fixture.roots = fixture.roots.with_nicknames(vec![("work in progress".into(), wip.clone())]);

    let actions = action_plan(&fixture, "move report.pdf to work in progress", ENTRY_LIMIT);
    assert_eq!(actions, [Action::Move { from: fixture.roots().desktop().join("report.pdf"), to: wip.join("report.pdf") }]);
}

#[test]
fn literal_located_destination_uses_an_extra_root() {
    let mut fixture = MessyMac::new();
    let home = &fixture.roots().home;
    let extra = home.join("Projects/Things in Progress");
    fs::create_dir_all(&extra).unwrap();
    fixture.roots = Roots::new(home, None, std::slice::from_ref(&extra)).with_exclusions(&[home.join("Documents/Private")]);

    let actions = action_plan(&fixture, "move report.pdf to Things in Progress", ENTRY_LIMIT);
    assert_eq!(actions, [Action::Move { from: fixture.roots().desktop().join("report.pdf"), to: extra.join("report.pdf") }]);
}

#[test]
fn finder_transfers_resolve_existing_folders_and_create_only_the_leaf() {
    let fixture = MessyMac::new();
    let roots = fixture.roots();
    let demo = roots.downloads().join("Voxi Demo");
    fs::create_dir(&demo).unwrap();
    let from = roots.desktop().join("report.pdf");
    let snapshot = crate::scope::Snapshot::capture(vec![from.clone()], "Finder".into()).unwrap();
    let choices = HashMap::new();
    let history = History::load(roots.home.join("history.json"));
    let plan = |command: &str, snapshot: &crate::scope::Snapshot| {
        let (intent, uses_selection) = crate::scope::prepare(command, Some(snapshot)).unwrap();
        assert!(uses_selection, "{command}");
        Planner::new(roots, &choices).with_scope(Some(snapshot.clone())).plan(&intent, &history)
    };
    for parent in ["Voxi Demoo", "Downloads/Voxi Demoo", "~/Downloads/Voxi Demoo"] {
        let command = format!("move these to Receipts in {parent}");
        assert!(
            matches!(
                plan(&command, &snapshot),
                Err(Stop::Problem(message)) if message == format!("Couldn't find a folder named “{parent}”.")
            ),
            "{command}"
        );
    }

    let named = "move these to Receipts in Voxi Demo";
    let Ok(Planned::Steps { actions, .. }) = plan(named, &snapshot) else { panic!("expected a move plan") };
    let receipts = demo.join("Receipts");
    assert_eq!(actions, [Action::CreateFolder { path: receipts.clone() }, Action::Move { from, to: receipts.join("report.pdf") }]);

    fs::create_dir(&receipts).unwrap();
    let selected = [demo.join("first.pdf"), demo.join("second.png")];
    for path in &selected {
        fs::write(path, b"demo").unwrap();
    }
    let (paths, _) = crate::finder_selection::accept(roots, selected.to_vec()).unwrap();
    let snapshot = crate::scope::Snapshot::capture(paths, crate::finder_selection::LABEL.into()).unwrap();
    let expected = selected.map(|from| Action::Move { to: receipts.join(from.file_name().unwrap()), from });
    for command in [named, "move these to Voxi Demo/Receipts"] {
        let planned = plan(command, &snapshot);
        let Ok(Planned::Steps { actions, .. }) = planned else { panic!("{command}: {:?}", planned.err()) };
        assert_eq!(actions, expected, "{command}");
    }
}

#[test]
fn folder_creation_hint_can_be_used_as_the_transfer_destination() {
    let fixture = MessyMac::new();
    let stuff = fixture.roots().downloads().join("Stuff");
    fs::create_dir(&stuff).unwrap();
    let Err(Stop::Problem(message)) = fixture.plan_command("move report.pdf to Receipts in Stuff", ENTRY_LIMIT) else {
        panic!("expected a hint for creating Receipts");
    };
    let destination = message.split_once(", use ").unwrap().1.strip_suffix('.').unwrap();
    let command = format!("move report.pdf to {destination}");
    let actions = action_plan(&fixture, &command, ENTRY_LIMIT);
    let receipts = stuff.join("Receipts");
    assert_eq!(
        actions,
        [
            Action::CreateFolder { path: receipts.clone() },
            Action::Move { from: fixture.roots().desktop().join("report.pdf"), to: receipts.join("report.pdf") }
        ]
    );
}

#[test]
fn literal_and_parent_scoped_folders_support_choices_and_quoting() {
    let fixture = MessyMac::new();
    let roots = fixture.roots();
    let [literal, scoped, quoted_literal, quoted_scoped] =
        ["Documents/Things in Progress", "Downloads/Progress/Things", "Documents/Receipts in Voxi Demo", "Downloads/Voxi Demo/Receipts"]
            .map(|path| roots.home.join(path));
    for folder in [&literal, &scoped, &quoted_literal, &quoted_scoped] {
        fs::create_dir_all(folder).unwrap();
        fs::write(folder.join("photo.png"), b"photo").unwrap();
    }
    let history = History::load(roots.home.join("history.json"));
    let label = "Things in Progress";
    for command in ["move report.pdf to Things in Progress", "sort Things in Progress by type"] {
        let Err(Stop::Ambiguous { name, candidates, reason: AmbiguityReason::MultipleMatches, .. }) =
            fixture.plan_command(command, ENTRY_LIMIT)
        else {
            panic!("{command}: expected a choice")
        };
        assert_eq!(name, label);
        assert_eq!(candidates, [literal.clone(), scoped.clone()]);
        for path in &candidates {
            let choices = HashMap::from([(label.to_string(), path.clone())]);
            let result = Planner::new(roots, &choices).plan(&parse(command).unwrap(), &history);
            let Ok(Planned::Steps { actions, .. }) = result else { panic!("{command}: {path:?}: {:?}", result.err()) };
            assert!(matches!(actions.last(), Some(Action::Move { to, .. }) if to.starts_with(path)));
        }
    }

    for (command, folder) in [
        (r#"move report.pdf to "Receipts in Voxi Demo""#, &quoted_literal),
        (r#"move report.pdf to Receipts in "Voxi Demo""#, &quoted_scoped),
        (r#"sort "Receipts in Voxi Demo" by type"#, &quoted_literal),
        (r#"sort Receipts in "Voxi Demo" by type"#, &quoted_scoped),
    ] {
        let actions = action_plan(&fixture, command, ENTRY_LIMIT);
        assert!(matches!(actions.last(), Some(Action::Move { to, .. }) if to.starts_with(folder)), "{command}");
    }
}

#[test]
fn scoped_sort_reports_unreadable_folders_inside_its_parent() {
    let fixture = MessyMac::new();
    let downloads = fixture.roots().downloads();
    let blocked = downloads.join("Blocked");
    let candidate = downloads.join("Voxi Demo");
    fs::create_dir(&blocked).unwrap();
    fs::create_dir(&candidate).unwrap();
    let _locked = LockedDir::new(blocked.clone());
    if fs::read_dir(&blocked).is_err_and(|err| err.kind() == std::io::ErrorKind::PermissionDenied) {
        assert!(matches!(
            fixture.plan_command("sort Voxi Demo in Downloads by type", ENTRY_LIMIT),
            Err(Stop::Ambiguous { candidates, reason: AmbiguityReason::IncompleteSearch, note: Some(note), .. })
                if candidates == [candidate] && note.contains("Blocked")
        ));
    }
}

#[test]
fn scoped_sort_rejects_a_choice_outside_its_parent() {
    let fixture = MessyMac::new();
    let roots = fixture.roots();
    let outside = roots.home.join("Documents/Voxi Demo");
    for folder in [outside.clone(), roots.downloads().join("First/Voxi Demo"), roots.downloads().join("Second/Voxi Demo")] {
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join("photo.png"), b"photo").unwrap();
    }
    let label = "Voxi Demo in Downloads";
    let command = "sort Voxi Demo in Downloads by type";
    let Err(Stop::Ambiguous { candidates, .. }) = fixture.plan_command(command, ENTRY_LIMIT) else { panic!("expected a choice") };
    assert_eq!(candidates.len(), 2);
    assert!(candidates.iter().all(|path| path.starts_with(roots.downloads())));
    let history = History::load(roots.home.join("history.json"));
    let choices = HashMap::from([(label.to_string(), outside)]);
    assert!(matches!(
        Planner::new(roots, &choices).plan(&parse(command).unwrap(), &history),
        Err(Stop::Problem(message)) if message.contains("Choose one of the matching folders")
    ));
}

#[test]
fn archive_output_requires_a_filename_after_the_final_slash() {
    let fixture = MessyMac::new();
    assert!(matches!(
        fixture.plan_command("compress all the screenshots into an archive named shots.zip../", ENTRY_LIMIT),
        Err(Stop::Problem(message)) if message == "Name the output file after the final “/”."
    ));
}

#[test]
fn archive_output_rejects_hidden_and_blank_filenames() {
    let fixture = MessyMac::new();
    for (output, expected) in [
        (r#""Downloads/.zip""#, "Choose an output filename that doesn't start with “.”."),
        ("Downloads/.", "Name the output file after the final “/”."),
        ("Downloads/..", "Name the output file after the final “/”."),
        (r#""Downloads/.""#, "Choose an output filename that doesn't start with “.”."),
        (r#""Downloads/..""#, "Choose an output filename that doesn't start with “.”."),
        (r#""Downloads/ ""#, "Name the output file."),
        (r#""""#, "Name the output file."),
    ] {
        let command = format!("zip report.pdf as {output}");
        let result = fixture.plan_command(&command, ENTRY_LIMIT);
        assert!(matches!(&result, Err(Stop::Problem(message)) if message == expected), "{command}: {:?}", result.err());
    }
}

#[test]
fn archive_output_adds_the_extension_to_the_filename_only() {
    let fixture = MessyMac::new();
    for (name, relative) in [
        ("shots", "Desktop/shots.zip"),
        ("Downloads/shots", "Downloads/shots.zip"),
        ("Downloads/shots.zip", "Downloads/shots.zip"),
        ("foo.zip/bar", "Desktop/foo.zip/bar.zip"),
    ] {
        let command = format!("zip report.pdf as {name}");
        let actions = action_plan(&fixture, &command, ENTRY_LIMIT);
        assert!(matches!(actions.last(), Some(Action::Zip { output, .. }) if *output == fixture.roots().home.join(relative)), "{command}");
    }
}

#[test]
fn merge_output_accepts_a_pdf_filename_with_spaces() {
    let fixture = MessyMac::new();
    let receipts = fixture.roots().downloads().join("Receipts");
    fs::create_dir(&receipts).unwrap();
    for name in ["first.pdf", "second.pdf"] {
        fs::write(receipts.join(name), b"pdf").unwrap();
    }
    let command = "merge PDFs in Downloads/Receipts into March receipts.pdf";
    let actions = action_plan(&fixture, command, ENTRY_LIMIT);
    assert!(
        matches!(actions.as_slice(), [Action::MergePdf { output, .. }] if *output == receipts.join("March receipts.pdf")),
        "{actions:?}"
    );
}

#[test]
fn mixed_finder_selection_plans_only_the_selected_allowed_file() {
    let fixture = MessyMac::new();
    let roots = fixture.roots();
    let selected = roots.home.join("Downloads/Resume 📄.pdf");
    let (paths, notice) = crate::finder_selection::accept(
        roots,
        vec![
            selected.clone(),
            roots.home.join("Downloads/Incoming"),
            roots.home.join("Downloads/Outside link/escape.pdf"),
            roots.home.join("Documents/Private/excluded.pdf"),
        ],
    )
    .unwrap();
    assert_eq!(paths, std::slice::from_ref(&selected));
    assert!(notice.unwrap().contains("1 folder left out"));
    let snapshot = crate::scope::Snapshot::capture(paths, crate::finder_selection::LABEL.into()).unwrap();
    let (intent, uses_selection) = crate::scope::prepare("move these to Documents", Some(&snapshot)).unwrap();
    assert!(uses_selection);
    let choices = HashMap::new();
    let planned = Planner::new(roots, &choices).with_scope(Some(snapshot)).plan(&intent, &History::load(roots.home.join("history.json")));
    let Ok(Planned::Steps { actions, .. }) = planned else { panic!("expected the selected file to be moved") };
    assert!(matches!(actions.as_slice(), [Action::Move { from, .. }] if *from == selected));
}

#[test]
fn result_followups_only_change_files_below_the_listed_folder() {
    let fixture = MessyMac::new();
    let roots = fixture.roots();
    let work = roots.downloads().join("Work");
    let direct = work.join("root.pdf");
    let nested = work.join("child/nested.pdf");
    fs::create_dir_all(nested.parent().unwrap()).unwrap();
    for path in [&direct, &nested] {
        fs::write(path, b"pdf").unwrap();
    }
    let Planned::Info { listed: Some(Listed { paths, label, folder }), .. } =
        complete_plan(&fixture, "list PDFs in Downloads/Work", ENTRY_LIMIT)
    else {
        panic!("expected complete list results");
    };
    assert_eq!(paths.len(), 2);
    let direct_row = paths.iter().position(|p| p == &direct).unwrap() + 1;
    let snapshot = crate::scope::Snapshot::capture_list(paths, label, folder).unwrap();
    let choices = HashMap::new();
    let history = History::load(roots.home.join("history.json"));
    let plan = |command| {
        let (intent, scoped) = crate::scope::prepare(command, Some(&snapshot)).unwrap();
        assert!(scoped, "{command}");
        let Ok(Planned::Steps { actions, .. }) = Planner::new(roots, &choices).with_scope(Some(snapshot.clone())).plan(&intent, &history)
        else {
            panic!("expected an action plan for {command}");
        };
        actions
    };
    for command in ["delete them from subfolders", "delete these files in subfolders only"] {
        assert_eq!(plan(command), [Action::Trash { path: nested.clone() }], "{command}");
    }
    assert!(matches!(plan("copy these to Documents from subfolders").as_slice(), [Action::Copy { from, .. }] if from == &nested));
    assert_eq!(plan("delete them including subfolders").len(), 2);
    assert!(matches!(
        crate::scope::prepare(&format!("delete {direct_row} from subfolders"), Some(&snapshot)),
        Err(crate::scope::PrepareError::Problem(message)) if message.contains("isn't in a subfolder")
    ));

    let selection = crate::scope::Snapshot::capture(vec![direct, nested], crate::finder_selection::LABEL.into()).unwrap();
    assert!(matches!(
        crate::scope::prepare("delete these files in subfolders only", Some(&selection)),
        Err(crate::scope::PrepareError::Problem(message)) if message.contains("Name the folder")
    ));
}

impl MessyMac {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let container = fsx::canonical(dir.path());
        let base = container.join("Home");
        let outside = container.join("Outside");
        let dependencies = base.join("Documents/dev/web repo/node_modules");
        let deep = (0..=search::MAX_DEPTH).fold(dependencies.clone(), |path, depth| path.join(format!("layer-{depth}")));
        for folder in [
            "Desktop/Mock.APP/Contents/Resources",
            "Downloads/Incoming/deeper",
            "Downloads/_to_delete",
            "Documents/Archive",
            "Documents/Project",
            "Documents/Private",
            "Documents/dev/web repo/.git",
            "Documents/dev/rust repo/target",
            "Documents/dev/Mock.xcodeproj/project.xcworkspace",
            "Pictures/Screenshots",
        ] {
            fs::create_dir_all(base.join(folder)).unwrap();
        }
        fs::create_dir_all(&deep).unwrap();
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("escape.pdf"), b"outside").unwrap();
        for file in [
            "Desktop/Mock.APP/Contents/Resources/bundled.pdf",
            "Desktop/Screenshot 2026-10-28.png",
            "Desktop/clip.mp4",
            "Desktop/installer.dmg",
            "Desktop/song.mp3",
            "Desktop/report.pdf",
            "Downloads/Resume 📄.pdf",
            "Downloads/Archive.zip",
            "Downloads/Incoming/nested.pdf",
            "Downloads/Incoming/deeper/more.pdf",
            "Downloads/_to_delete/trashed.pdf",
            "Documents/invoice.pdf",
            "Documents/Project/spec.pdf",
            "Documents/Private/excluded.pdf",
            "Documents/dev/web repo/package.json",
            "Documents/dev/web repo/.git/hidden.pdf",
            "Documents/dev/rust repo/Cargo.toml",
            "Documents/dev/rust repo/target/built.pdf",
            "Documents/dev/Mock.xcodeproj/project.xcworkspace/project.pdf",
            "Pictures/Screenshots/Snow 雪.png",
        ] {
            let mut file = fs::File::create(base.join(file)).unwrap();
            std::io::Write::write_all(&mut file, b"fixture").unwrap();
            file.set_modified(SystemTime::from(regression_noon())).unwrap();
        }
        fs::write(deep.join("deep.pdf"), b"deep").unwrap();
        for index in 0..=ENTRY_LIMIT {
            fs::write(dependencies.join(format!("dependency-{index}.pdf")), b"dependency").unwrap();
        }
        for index in 0..96 {
            let mut file = fs::File::create(base.join(format!("Downloads/download-{index}.txt"))).unwrap();
            std::io::Write::write_all(&mut file, b"download").unwrap();
            file.set_modified(SystemTime::from(regression_noon())).unwrap();
        }
        symlink(base.join("Documents/Project"), base.join("Downloads/Project link")).unwrap();
        symlink(&outside, base.join("Downloads/Outside link")).unwrap();
        symlink(base.join("Documents/Private"), base.join("Downloads/Private link")).unwrap();
        let roots = Roots::new(&base, None, &[]).with_exclusions(&[base.join("Documents/Private")]);
        Self { _dir: dir, roots }
    }

    fn roots(&self) -> &Roots {
        &self.roots
    }

    fn plan(&self, intent: &Intent, limit: usize, chain_action_ends: &mut Vec<usize>) -> Result<Planned, Stop> {
        let choices = HashMap::new();
        let mut planner = Planner::new(self.roots(), &choices);
        planner.search.limit = limit;
        planner.now = regression_noon();
        planner.chain_action_ends = Some(chain_action_ends);
        planner.plan(intent, &History::load(self.roots().home.join("history.json")))
    }

    fn plan_command(&self, command: &str, limit: usize) -> Result<Planned, Stop> {
        let _date = pin_regression_date();
        self.plan(&parse(command).unwrap(), limit, &mut Vec::new())
    }

    fn plan_with_control(&self, command: &str, control: search::Control) -> Result<Planned, Stop> {
        let choices = HashMap::new();
        let mut planner = Planner::new(self.roots(), &choices).with_control(control);
        planner.search.limit = ENTRY_LIMIT;
        planner.plan(&parse(command).unwrap(), &History::load(self.roots().home.join("history.json")))
    }
}

fn assert_in_scope(roots: &Roots, command: &str, path: &Path, allow_explicit_package: bool, internal_backup: Option<&Path>) {
    assert!(roots.is_allowed(path), "{command:?} planned a disallowed path: {}", path.display());
    for candidate in [path.to_path_buf(), fsx::canonical(path)] {
        let root = roots.allowed.iter().filter(|root| candidate.starts_with(root)).max_by_key(|root| root.components().count());
        let root = root.unwrap_or_else(|| panic!("{command:?} planned a path outside the allowed roots: {}", candidate.display()));
        if internal_backup.is_some_and(|dir| candidate.starts_with(dir)) {
            continue;
        }
        for ancestor in candidate.ancestors().take_while(|ancestor| *ancestor != root) {
            assert!(!fsx::is_hidden_or_internal(ancestor), "{command:?} planned a hidden or internal path: {}", candidate.display());
            assert!(
                ancestor == candidate || allow_explicit_package || !search::is_bundle(ancestor),
                "{command:?} planned a path inside a package: {}",
                candidate.display()
            );
        }
    }
}

fn assert_plan_in_scope(roots: &Roots, command: &str, plan: &Planned, allow_explicit_package: bool) {
    let actions = match plan {
        Planned::Steps { actions, .. } => actions,
        Planned::Direct { action, .. } => std::slice::from_ref(action),
        Planned::Info { listed, .. } => {
            if let Some(Listed { paths, .. }) = listed {
                for path in paths {
                    assert_in_scope(roots, command, path, allow_explicit_package, None);
                }
            }
            return;
        }
    };
    for action in actions {
        for path in action.paths() {
            assert!(
                matches!(action, Action::OpenInFinder { .. }) && roots.is_configured_root(&path)
                    || path.parent().is_some_and(|parent| roots.is_allowed(parent)),
                "{command:?} planned an action with a disallowed parent: {action:?}"
            );
            let internal_backup = match action {
                Action::ResizeImage { path: source, backup, .. } if path == *backup => {
                    let dir = roots.trash_dir_for(source).join(".voxi-originals");
                    assert!(path.starts_with(&dir), "{command:?}: unexpected resize backup location: {path:?}");
                    Some(dir)
                }
                _ => None,
            };
            assert_in_scope(roots, command, &path, allow_explicit_package, internal_backup.as_deref());
        }
    }
}

fn assert_complete(command: &str, result: &Result<Planned, Stop>) {
    let messages = match result {
        Err(
            Stop::Problem(message)
            | Stop::MissingTransferItem { message, .. }
            | Stop::SearchIncomplete(message)
            | Stop::NothingToChange(message),
        )
        | Ok(Planned::Info { summary: message, .. }) => std::slice::from_ref(message),
        Ok(Planned::Steps { warnings, .. }) => warnings,
        Err(Stop::Ambiguous { candidates, reason, .. }) => {
            assert_ne!(*reason, AmbiguityReason::IncompleteSearch, "{command:?}: incomplete lookup for {candidates:?}");
            &[]
        }
        Ok(Planned::Direct { .. }) => &[],
    };
    assert!(!messages.iter().any(|message| message.contains("Search incomplete")), "{command:?}: {messages:?}");
}

fn assert_result_in_scope(roots: &Roots, command: &str, result: &Result<Planned, Stop>) {
    match result {
        Ok(plan) => assert_plan_in_scope(roots, command, plan, false),
        Err(Stop::Ambiguous { candidates, .. }) => {
            for path in candidates {
                assert_in_scope(roots, command, path, false, None);
            }
        }
        Err(Stop::Problem(_) | Stop::MissingTransferItem { .. } | Stop::SearchIncomplete(_) | Stop::NothingToChange(_)) => {}
    }
}

fn assert_direct_sources(roots: &Roots, command: &str, intent: &Intent, actions: &[Action], chain_action_ends: &[usize]) {
    if let Intent::Chain { commands } = intent {
        assert_eq!(commands.len(), chain_action_ends.len(), "{command:?}: missing chain action boundaries");
        assert_eq!(chain_action_ends.last(), Some(&actions.len()), "{command:?}: unassigned chain actions");
        let mut start = 0;
        for (step, &end) in commands.iter().zip(chain_action_ends) {
            assert_direct_sources(roots, command, step, &actions[start..end], &[]);
            start = end;
        }
        return;
    }
    // A step with nothing planned (e.g. reading a folder the chain is still
    // creating) has no sources to check, and its folder may not exist yet.
    if actions.is_empty() {
        return;
    }
    let choices = HashMap::new();
    let mut planner = Planner::new(roots, &choices);
    let direct_roots = match intent {
        Intent::Move { src, .. } | Intent::Copy { src, .. } | Intent::Trash { src } => match src {
            Source::Set { folder, filter } if !Planner::includes_nested_files(folder.as_deref(), filter) => match folder {
                Some(name) => vec![planner.resolve_folder(name, None).unwrap()],
                None => vec![roots.desktop(), roots.downloads()],
            },
            _ => return,
        },
        Intent::SortByType { folder, include_subfolders: false } | Intent::SortByMonth { folder, include_subfolders: false } => {
            vec![planner.resolve_destination(folder, None).unwrap()]
        }
        _ => return,
    };
    for action in actions {
        let source = match action {
            Action::Move { from, .. } | Action::Copy { from, .. } | Action::Trash { path: from } => from,
            _ => continue,
        };
        assert!(
            source.parent().is_some_and(|parent| direct_roots.iter().any(|root| root == parent)),
            "{command:?} planned a nested source for a direct-only command: {source:?}"
        );
    }
}

fn complete_plan(fixture: &MessyMac, command: &str, limit: usize) -> Planned {
    let _date = pin_regression_date();
    let intent = parse(command).unwrap();
    let mut chain_action_ends = Vec::new();
    let result = fixture.plan(&intent, limit, &mut chain_action_ends);
    assert_result_in_scope(fixture.roots(), command, &result);
    assert_complete(command, &result);
    let plan = result.unwrap_or_else(|err| panic!("{command:?}: {err:?}"));
    if let Planned::Steps { actions, .. } = &plan {
        assert_direct_sources(fixture.roots(), command, &intent, actions, &chain_action_ends);
    }
    plan
}

fn action_plan(fixture: &MessyMac, command: &str, limit: usize) -> Vec<Action> {
    let Planned::Steps { actions, .. } = complete_plan(fixture, command, limit) else { panic!("{command:?}: expected a change plan") };
    actions
}

#[test]
fn messy_mac_command_families() {
    let started = Instant::now();
    let fixture = MessyMac::new();
    let base = &fixture.roots().home;
    let docs = base.join("Documents");
    let invoice = docs.join("invoice.pdf");
    let archive = docs.join("Archive");
    assert_eq!(
        action_plan(&fixture, "Create a folder in dev named flagsmith.", ENTRY_LIMIT),
        [Action::CreateFolder { path: docs.join("dev/flagsmith") }]
    );
    assert_eq!(
        action_plan(&fixture, "move PDFs in Documents to Documents/Archive", DIRECT_LIMIT),
        [Action::Move { from: invoice.clone(), to: archive.join("invoice.pdf") }]
    );
    assert_eq!(
        action_plan(&fixture, "copy PDFs in Documents to Documents/Archive", DIRECT_LIMIT),
        [Action::Copy { from: invoice.clone(), to: archive.join("invoice.pdf") }]
    );
    assert_eq!(action_plan(&fixture, "delete PDFs in Documents", DIRECT_LIMIT), [Action::Trash { path: invoice.clone() }]);
    assert_eq!(
        action_plan(&fixture, "delete PDFs in Incoming", ENTRY_LIMIT),
        [Action::Trash { path: base.join("Downloads/Incoming/nested.pdf") }]
    );
    assert!(matches!(
        action_plan(&fixture, "sort Documents by type", DIRECT_LIMIT).as_slice(),
        [Action::CreateFolder { path }, Action::Move { from, to }] if from == &invoice && to.parent() == Some(path)
    ));
    assert_eq!(
        action_plan(&fixture, "rename Downloads/Resume 📄.pdf to Résumé final 📄.pdf", ENTRY_LIMIT),
        [Action::Rename { path: base.join("Downloads/Resume 📄.pdf"), new_name: "Résumé final 📄.pdf".into() }]
    );
    assert!(matches!(complete_plan(&fixture, "open dev", ENTRY_LIMIT), Planned::Direct { action, .. }
        if action == Action::OpenInFinder { path: docs.join("dev"), reveal: false }));
    assert!(matches!(complete_plan(&fixture, "open Documents", ENTRY_LIMIT), Planned::Direct { action, .. }
        if action == Action::OpenInFinder { path: docs.clone(), reveal: false }));
    assert!(matches!(complete_plan(&fixture, "size of dev", ENTRY_LIMIT), Planned::Info { title, .. } if title.starts_with("dev uses ")));
    assert!(
        matches!(complete_plan(&fixture, "list PDFs in Documents", ENTRY_LIMIT), Planned::Info { listed: Some(Listed { paths, .. }), .. }
        if paths.len() == 2 && paths.contains(&invoice) && paths.contains(&docs.join("Project/spec.pdf")))
    );
    assert!(matches!(complete_plan(&fixture, "count PDFs in Documents", ENTRY_LIMIT), Planned::Info { total: 2, .. }));
    assert!(matches!(action_plan(&fixture, "zip Documents/Project", ENTRY_LIMIT).as_slice(),
        [Action::Zip { path, output }] if path == &docs.join("Project") && output == &docs.join("Project.zip")));
    assert!(matches!(action_plan(&fixture, "extract Archive.zip to Documents", ENTRY_LIMIT).as_slice(),
        [Action::Unzip { path, destination }] if path == &base.join("Downloads/Archive.zip") && destination == &docs.join("Archive 2")));
    assert!(matches!(
        action_plan(&fixture, "create a folder called Taxes in Documents, move all PDFs from Downloads into it, then zip it", ENTRY_LIMIT).as_slice(),
        [Action::CreateFolder { path }, Action::Move { from, to }, Action::Zip { path: zipped, output }]
            if path == &docs.join("Taxes") && from == &base.join("Downloads/Resume 📄.pdf") && to.starts_with(path)
                && zipped == path && output == &docs.join("Taxes.zip")
    ));
    println!("messy Mac command families: {:?}", started.elapsed());
}

#[test]
fn messy_mac_duplicate_folder_names_require_a_choice() {
    let fixture = MessyMac::new();
    let base = &fixture.roots().home;
    for folder in ["Desktop/Archive", "Downloads/Incoming/Archive"] {
        fs::create_dir_all(base.join(folder)).unwrap();
    }
    let command = "delete folder Archive";
    let result = fixture.plan_command(command, ENTRY_LIMIT);
    assert_result_in_scope(fixture.roots(), command, &result);
    assert_complete(command, &result);
    assert!(matches!(result,
        Err(Stop::Ambiguous { candidates, reason: AmbiguityReason::MultipleMatches, .. })
            if candidates.len() == 3 && candidates.contains(&base.join("Documents/Archive"))
                && candidates.contains(&base.join("Desktop/Archive")) && candidates.contains(&base.join("Downloads/Incoming/Archive"))));
}

#[test]
fn messy_mac_unreadable_folder_only_blocks_recursive_changes() {
    let fixture = MessyMac::new();
    let docs = fixture.roots().home.join("Documents");
    let path = docs.join("dev/Locked");
    fs::create_dir(&path).unwrap();
    fs::write(path.join("locked.pdf"), b"locked").unwrap();
    let locked = LockedDir::new(path);

    assert!(matches!(action_plan(&fixture, "Create a folder in dev named flagsmith.", ENTRY_LIMIT).as_slice(),
        [Action::CreateFolder { path }] if path == &docs.join("dev/flagsmith")));
    assert!(matches!(action_plan(&fixture, "move PDFs in Documents to Documents/Archive", DIRECT_LIMIT).as_slice(),
        [Action::Move { from, to }] if from == &docs.join("invoice.pdf") && to == &docs.join("Archive/invoice.pdf")));
    assert!(matches!(action_plan(&fixture, "sort Documents by type", DIRECT_LIMIT).as_slice(),
        [Action::CreateFolder { path }, Action::Move { from, to }] if from == &docs.join("invoice.pdf") && to.parent() == Some(path)));
    assert!(matches!(complete_plan(&fixture, "open dev", ENTRY_LIMIT), Planned::Direct { action, .. }
        if action == Action::OpenInFinder { path: docs.join("dev"), reveal: false }));

    // A privileged runner can read a mode-000 directory.
    match fs::read_dir(&locked.path) {
        Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => {
            let command = "delete PDFs in Documents including subfolders";
            assert!(matches!(fixture.plan_command(command, ENTRY_LIMIT),
                Err(Stop::SearchIncomplete(message)) if message.contains("Search incomplete")
                    && message.contains("Couldn't read “~/Documents/dev/Locked”")));
        }
        Ok(_) => eprintln!("privileged runner can read mode-000 dirs; permission-denied assertion skipped"),
        Err(err) => panic!("unexpected error reading the locked fixture folder: {err}"),
    }
}

#[test]
fn messy_mac_scopes_partial_scans_to_the_step_that_reads_them() {
    let fixture = MessyMac::new();
    let roots = fixture.roots();
    let history = History::load(roots.home.join("history.json"));
    let choices = HashMap::new();
    let planner = || {
        let mut planner = Planner::new(roots, &choices);
        planner.search.depth = 0;
        planner
    };
    let delete = "delete PDFs in Documents including subfolders";
    let create = "create a folder called Ready in Desktop";
    let commands = [(delete.into(), parse(delete).unwrap()), (create.into(), parse(create).unwrap())];
    let expected = Action::CreateFolder { path: roots.desktop().join("Ready") };

    // An incomplete search invalidates the whole shortcut, even with an independent later command.
    assert!(matches!(planner().plan_shortcut("tidy", &commands, &history),
        Err(Stop::SearchIncomplete(message)) if message.contains("~/Documents/Archive") && message.contains("Settings → Folders")));
    // A chain is one sequence and must stop at a failed step.
    let chain = Intent::Chain { commands: commands.iter().rev().map(|(_, intent)| intent.clone()).collect() };
    let control = search::Control::default();
    let mut chain_planner = Planner::new(roots, &choices).with_control(control.clone());
    chain_planner.search.depth = 0;
    assert!(matches!(chain_planner.plan(&chain, &history),
        Err(Stop::SearchIncomplete(message)) if message.contains("Search incomplete")));
    assert!(control.failed_with(search::Failure::Depth));
    let stats = control.stats(Duration::ZERO);
    assert!(stats.visited > 0 && stats.too_deep > 0);
    assert_eq!(stats.unreadable, 0);

    let chain =
        Intent::Chain { commands: vec![parse(create).unwrap(), parse("move invoice.pdf from Documents/Project to Pictures").unwrap()] };
    fs::write(roots.home.join("Documents/Project/invoice.pdf"), b"scoped").unwrap();
    assert!(matches!(planner().plan(&chain, &history),
        Ok(Planned::Steps { actions, .. })
            if actions.len() == 2 && actions[0] == expected
                && matches!(&actions[1], Action::Move { from, .. } if from == &roots.home.join("Documents/Project/invoice.pdf"))));
}

#[test]
fn incomplete_scan_stops_shortcut_before_overlapping_delete_and_chain_before_dependent_move() {
    let fixture = MessyMac::new();
    let roots = fixture.roots();
    let history = History::load(roots.home.join("history.json"));
    let choices = HashMap::new();
    let report = roots.home.join("Documents/report.pdf");
    fs::write(&report, b"keep").unwrap();
    let planner = || {
        let mut planner = Planner::new(roots, &choices);
        planner.search.depth = 0;
        planner
    };
    let copy = "copy PDFs from Documents to Desktop including subfolders";
    let delete = "delete PDFs in Documents";
    let commands = [(copy.into(), parse(copy).unwrap()), (delete.into(), parse(delete).unwrap())];
    assert!(matches!(planner().plan(&commands[1].1, &history),
        Ok(Planned::Steps { actions, .. }) if actions.contains(&Action::Trash { path: report.clone() })));
    assert!(matches!(planner().plan_shortcut("tidy", &commands, &history), Err(Stop::SearchIncomplete(_))));
    assert!(report.exists());

    let chain_dir = roots.home.join("Documents/Chain");
    fs::create_dir_all(chain_dir.join("nested")).unwrap();
    fs::write(chain_dir.join("report.pdf"), b"keep").unwrap();
    let chain = parse("zip PDFs in Documents/Chain as pdfs.zip and move it to Desktop").unwrap();
    assert!(matches!(&chain, Intent::Chain { commands }
        if matches!(commands[1].source(), Some(Source::Item { name, folder: None }) if name == "\0chain-output")));
    assert!(matches!(planner().plan(&chain, &history), Err(Stop::SearchIncomplete(_))));
    assert!(matches!(Planner::new(roots, &choices).plan(&chain, &history),
        Ok(Planned::Steps { actions, .. }) if matches!(actions.as_slice(),
            [Action::Zip { output, .. }, Action::Move { from, .. }] if from == output)));
}

#[test]
fn messy_mac_refuses_to_copy_an_allowed_root() {
    let fixture = MessyMac::new();
    let command = "copy ~/Documents to Downloads";
    let result = fixture.plan_command(command, ENTRY_LIMIT);
    assert_result_in_scope(fixture.roots(), command, &result);
    assert!(matches!(result, Err(Stop::Problem(message)) if message.contains("won't copy an allowed root")));
}

#[test]
fn whole_folder_previews_refuse_excluded_descendants() {
    let fixture = MessyMac::new();
    let base = &fixture.roots().home;
    let project = base.join("Downloads/Project");
    let private = project.join("Private");
    let public = project.join("public.pdf");
    let photo_folder = base.join("Downloads/Project.png");
    fs::create_dir_all(&private).unwrap();
    fs::create_dir_all(photo_folder.join("Private")).unwrap();
    fs::write(private.join("excluded.pdf"), b"private").unwrap();
    fs::write(&public, b"public").unwrap();
    symlink(project, base.join("Downloads/Project alias")).unwrap();
    let roots = fixture.roots().clone().with_exclusions(&[private, photo_folder.join("Private"), base.join("Documents/Private")]);
    let choices = HashMap::new();
    let history = History::load(base.join("history.json"));
    let plan = |command| Planner::new(&roots, &choices).plan(&parse(command).unwrap(), &history);
    let mut missed = Vec::new();
    for command in [
        "copy Downloads/Project to Documents",
        "zip Downloads/Project as export",
        "zip Downloads/Project alias as export",
        "move Downloads/Project to Desktop",
        "delete Downloads/Project",
        "rename Downloads/Project to Renamed",
        "resize Downloads/Project.png to 16 pixels wide",
    ] {
        if !matches!(plan(command), Err(Stop::Problem(message)) if message.contains("contains “Private”")
            && message.contains("excluded in Settings → Folders"))
        {
            missed.push(command);
        }
    }
    assert!(missed.is_empty(), "whole-folder commands allowed excluded contents: {missed:?}");
    assert!(matches!(plan("rename Downloads/Project to Renamed"),
        Err(Stop::Problem(message)) if message.contains("Remove the exclusion first, then rename it")));
    // A destination that doesn't exist yet, spelled with different case from
    // its excluded subfolder, still lands in that subfolder on the drive.
    let lower = base.join("Downloads/project-lower");
    fs::create_dir_all(lower.join("Private")).unwrap();
    fs::write(lower.join("Private/secret.pdf"), b"secret").unwrap();
    let roots_case = fixture.roots().clone().with_exclusions(&[base.join("Desktop/Project-Lower/Private")]);
    let planned = Planner::new(&roots_case, &choices).plan(&parse("copy Downloads/project-lower to Desktop").unwrap(), &history);
    assert!(
        matches!(&planned, Err(Stop::Problem(message)) if message.contains("excluded in Settings")),
        "copy into an excluded subfolder was allowed"
    );
    for command in ["copy Downloads/Project/public.pdf to Documents/Private", "move Downloads/Project/public.pdf to Documents/Private"] {
        assert!(matches!(plan(command), Err(Stop::Problem(message)) if message.contains("outside the folders")), "{command}");
    }
    assert!(matches!(plan("copy PDFs in Downloads/Project to Documents including subfolders"),
        Ok(Planned::Steps { actions, .. }) if matches!(actions.as_slice(), [Action::Copy { from, .. }] if from == &public)));
}

#[test]
fn whole_folder_execution_rechecks_for_excluded_descendants_added_after_preview() {
    let fixture = MessyMac::new();
    let project = fixture.roots().downloads().join("Project");
    let private = project.join("Private");
    fs::create_dir(&project).unwrap();
    fs::write(project.join("public.pdf"), b"public").unwrap();
    let roots = fixture.roots().clone().with_exclusions(std::slice::from_ref(&private));
    let choices = HashMap::new();
    let history = History::load(roots.home.join("history.json"));
    let previews: Vec<_> = [
        "copy Downloads/Project to Documents",
        "zip Downloads/Project as export",
        "move Downloads/Project to Desktop",
        "delete Downloads/Project",
        "rename Downloads/Project to Renamed",
    ]
    .into_iter()
    .map(|command| {
        let Ok(Planned::Steps { actions, .. }) = Planner::new(&roots, &choices).plan(&parse(command).unwrap(), &history) else {
            panic!("expected a preview before the excluded folder exists for {command}");
        };
        (command, actions)
    })
    .collect();
    fs::create_dir(&private).unwrap();
    fs::write(private.join("excluded.pdf"), b"private").unwrap();
    for (command, actions) in previews {
        let outcome = crate::exec::run(&actions, &roots);
        assert!(outcome.done.is_empty(), "{command} changed files");
        assert!(outcome.terminal.error().is_some_and(|e| e.contains("contains “Private”")), "{command}: {:?}", outcome.terminal);
        assert!(private.join("excluded.pdf").exists());
    }
}

#[test]
fn whole_folder_transfer_refuses_an_exclusion_at_the_projected_destination() {
    let fixture = MessyMac::new();
    let project = fixture.roots().downloads().join("Project");
    let private = project.join("Private");
    fs::create_dir_all(&private).unwrap();
    fs::write(private.join("report.pdf"), b"pdf").unwrap();
    let dest = fixture.roots().desktop().join("Project");
    let roots = fixture.roots().clone().with_exclusions(&[dest.join("Private")]);
    let choices = HashMap::new();
    let history = History::load(roots.home.join("history.json"));
    for command in ["copy Downloads/Project to Desktop", "move Downloads/Project to Desktop"] {
        let result = Planner::new(&roots, &choices).plan(&parse(command).unwrap(), &history);
        assert!(matches!(result, Err(Stop::Problem(message)) if message.contains("excluded in Settings → Folders")), "{command}");
    }
    assert!(
        crate::exec::run(&[Action::Copy { from: project, to: dest.clone() }], &roots)
            .terminal
            .error()
            .is_some_and(|e| e.contains("excluded in Settings → Folders"))
    );
    let unzip = parse("unzip Archive.zip into Desktop/Project").unwrap();
    assert!(matches!(Planner::new(&roots, &choices).plan(&unzip, &history),
        Err(Stop::Problem(message)) if message.contains("excluded in Settings → Folders")));
    assert!(!dest.exists());
}

#[test]
fn punctuation_name_filters_match_literally() {
    let dir = tempfile::tempdir().unwrap();
    let home = fsx::canonical(dir.path());
    let downloads = home.join("Downloads");
    fs::create_dir(&downloads).unwrap();
    let roots = Roots::new(&home, None, &[]);
    for name in ["tax-2026.txt", "-draft.txt", "ends-.txt", "receipt_2026.txt", "plain.txt", "README"] {
        fs::write(downloads.join(name), b"fixture").unwrap();
    }
    let choices = HashMap::new();
    let history = History::load(roots.home.join("history.json"));
    let cases: &[(&str, &[&str])] = &[
        (r#"delete files in Downloads containing "-""#, &["-draft.txt", "ends-.txt", "tax-2026.txt"]),
        (r#"delete files in Downloads starting with "-""#, &["-draft.txt"]),
        (r#"delete files in Downloads containing "_""#, &["receipt_2026.txt"]),
        (r#"delete files in Downloads containing "-.""#, &["ends-.txt"]),
        (r#"delete files in Downloads named "_""#, &["receipt_2026.txt"]),
        (r#"delete files in Downloads called "-""#, &["-draft.txt", "ends-.txt", "tax-2026.txt"]),
        (r#"delete files in Downloads containing "tax 2026""#, &["tax-2026.txt"]),
    ];
    let mut mismatches = Vec::new();
    for (command, expected) in cases {
        let Ok(Planned::Steps { actions, .. }) = Planner::new(&roots, &choices).plan(&parse(command).unwrap(), &history) else {
            panic!("expected a preview for {command}");
        };
        let names: Vec<_> = actions
            .into_iter()
            .map(|action| match action {
                Action::Trash { path } => file_name(&path),
                _ => panic!("expected a trash action"),
            })
            .collect();
        if names != *expected {
            mismatches.push((command, names));
        }
    }
    assert!(mismatches.is_empty(), "unexpected matches: {mismatches:?}");
}

#[test]
fn empty_name_and_extension_filters_refuse_to_plan() {
    let fixture = MessyMac::new();
    let roots = fixture.roots();
    fs::write(roots.downloads().join("README"), b"no extension").unwrap();
    let choices = HashMap::new();
    let history = History::load(roots.home.join("history.json"));
    let plan = |intent| Planner::new(roots, &choices).plan(&intent, &history);
    let whitespace = parse(r#"delete files in Downloads containing " ""#).unwrap();
    let mut missed = Vec::new();
    if !matches!(plan(whitespace), Err(Stop::Problem(message)) if message.contains("filename filter")) {
        missed.push("whitespace name filter");
    }
    for (label, filter) in [
        ("empty name filter", Filter { name_starts_with: Some(String::new()), ..Default::default() }),
        ("empty extension", Filter { ext: Some(vec![String::new()]), ..Default::default() }),
        ("empty extension list", Filter { ext: Some(vec![]), ..Default::default() }),
    ] {
        let intent = Intent::Trash { src: Source::set(Some("Downloads"), filter) };
        if !matches!(plan(intent), Err(Stop::Problem(message)) if message.contains("filter")) {
            missed.push(label);
        }
    }
    assert!(missed.is_empty(), "filters were accepted: {missed:?}");
}

#[test]
fn messy_mac_skips_opaque_trees_and_rejects_escaping_links() {
    let fixture = MessyMac::new();
    let base = &fixture.roots().home;
    for command in ["delete Downloads/Outside link/escape.pdf", "delete Downloads/Private link/excluded.pdf"] {
        assert!(
            matches!(fixture.plan_command(command, ENTRY_LIMIT), Err(Stop::Problem(message))
            if message.contains("outside the folders")),
            "{command:?}"
        );
    }
    assert!(matches!(complete_plan(&fixture, "open Downloads/Project link", ENTRY_LIMIT), Planned::Direct { action, .. }
        if action == Action::OpenInFinder { path: base.join("Downloads/Project link"), reveal: false }));
    for command in ["delete bundled.pdf", "delete project.pdf", "delete hidden.pdf", "delete trashed.pdf", "delete built.pdf"] {
        assert!(
            matches!(fixture.plan_command(command, ENTRY_LIMIT), Err(Stop::Problem(message))
            if message.contains("Couldn't find")),
            "{command:?}"
        );
    }
    let command = "open Desktop/Mock.APP/Contents/Resources/bundled.pdf";
    let plan = fixture.plan_command(command, ENTRY_LIMIT).unwrap();
    assert_plan_in_scope(fixture.roots(), command, &plan, true);
    assert!(matches!(plan, Planned::Direct { action, .. }
        if action == Action::OpenInFinder { path: base.join("Desktop/Mock.APP/Contents/Resources/bundled.pdf"), reveal: true }));
}

#[test]
fn messy_mac_plans_every_accepted_corpus_command() {
    let started = Instant::now();
    let fixture = MessyMac::new();
    let _date = pin_regression_date();
    let mut checked = 0;
    let (mut steps, mut info, mut direct) = (0, 0, 0);
    for (command, expected, at) in regression_cases() {
        if matches!(expected.as_str(), "!" | "!miss" | "!hard") {
            continue;
        }
        let mut chain_action_ends = Vec::new();
        let (intent, result) = catch_unwind(AssertUnwindSafe(|| {
            let intent = parse(&command).unwrap_or_else(|err| panic!("accepted corpus command no longer parses: {err:?}"));
            let result = fixture.plan(&intent, ENTRY_LIMIT, &mut chain_action_ends);
            (intent, result)
        }))
        .unwrap_or_else(|_| panic!("{at}: parser or planner panicked for {command:?}"));
        let context = format!("{at}: {command}");
        assert_result_in_scope(fixture.roots(), &context, &result);
        assert_complete(&context, &result);
        match result {
            Ok(Planned::Steps { actions, .. }) => {
                steps += 1;
                assert_direct_sources(fixture.roots(), &context, &intent, &actions, &chain_action_ends);
            }
            Ok(Planned::Info { .. }) => info += 1,
            Ok(Planned::Direct { .. }) => direct += 1,
            Err(_) => {}
        }
        checked += 1;
    }
    println!("messy Mac corpus: {checked} commands, {steps} Steps, {info} Info, {direct} Direct in {:?}", started.elapsed());
    assert!(steps >= 500 && info >= 350 && direct >= 40, "too few successful corpus plans: {steps} Steps, {info} Info, {direct} Direct");
}

fn mutate(command: &str, state: &mut u32) -> String {
    fn pick(state: &mut u32, len: usize) -> usize {
        xorshift32(state) as usize % len
    }

    let mut chars: Vec<_> = command.chars().collect();
    if chars.is_empty() {
        return String::new();
    }
    let inserts = ["雪", "🧪", "e\u{301}", "é", "—", "’", "…", "?!", "../", "\0", "\u{200d}", "\u{202e}"];
    let spaces = [' ', '\t', '\n', '\u{a0}', '\u{2003}'];
    match pick(state, 5) {
        0 => {
            let point = pick(state, chars.len() + 1);
            chars.splice(point..point, inserts[pick(state, inserts.len())].chars());
        }
        1 => {
            let start = pick(state, chars.len());
            let end = start + pick(state, chars.len() - start) + 1;
            let quotes = [('"', '"'), ('“', '”'), ('\'', '\''), ('‘', '’')];
            let (left, right) = quotes[pick(state, quotes.len())];
            chars.insert(end, right);
            chars.insert(start, left);
        }
        2 => {
            for ch in chars.iter_mut().filter(|ch| ch.is_whitespace()) {
                *ch = spaces[pick(state, spaces.len())];
            }
        }
        3 => {
            let point = pick(state, chars.len());
            chars.remove(point);
        }
        _ => chars.extend(inserts[pick(state, inserts.len())].chars()),
    }
    chars.into_iter().collect()
}

#[test]
fn messy_mac_seeded_mutations_stay_in_scope() {
    const SEED: u32 = 0x4d45_5353;
    const SAMPLES: usize = 512;
    let started = Instant::now();
    let fixture = MessyMac::new();
    let cases = regression_cases();
    let _date = pin_regression_date();
    let mut state = SEED;
    let mut accepted = 0;
    let mut steps = 0;
    for sample in 0..SAMPLES {
        let (original, _, at) = &cases[xorshift32(&mut state) as usize % cases.len()];
        let command = mutate(original, &mut state);
        let context = format!("seed {SEED:#x} sample {sample}, {at}, from {original:?}: {command:?}");
        let mut chain_action_ends = Vec::new();
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            let intent = parse(&command).ok()?;
            let plan = fixture.plan(&intent, ENTRY_LIMIT, &mut chain_action_ends);
            Some((intent, plan))
        }))
        .unwrap_or_else(|_| panic!("parser or planner panicked: {context}"));
        if let Some((intent, result)) = outcome {
            accepted += 1;
            assert_result_in_scope(fixture.roots(), &context, &result);
            if let Ok(Planned::Steps { actions, .. }) = result {
                assert_direct_sources(fixture.roots(), &context, &intent, &actions, &chain_action_ends);
                steps += 1;
            }
        }
    }
    println!("messy Mac mutations: {SAMPLES} samples, {accepted} accepted, {steps} Steps in {:?}", started.elapsed());
    assert!(accepted >= 250 && steps >= 50, "too few mutations reached planning: {accepted} accepted, {steps} Steps");
}

#[test]
fn messy_mac_resize_backup_cannot_follow_an_outside_link() {
    let fixture = MessyMac::new();
    let base = &fixture.roots().home;
    symlink(base.parent().unwrap().join("Outside"), base.join("Pictures/_to_delete")).unwrap();
    assert!(matches!(fixture.plan_command("resize images in Pictures to 800 px wide", ENTRY_LIMIT),
        Err(Stop::Problem(message)) if message.contains("outside the folders")));
}

#[test]
fn messy_mac_create_then_delete_contents_targets_the_new_folder() {
    let fixture = MessyMac::new();
    let downloads = fixture.roots().home.join("Downloads");
    let command = "create a folder in downloads named test1 and delete its content";
    // While the folder is new, only the folder is created; nothing else in
    // the flat, busy Downloads folder is touched.
    assert_eq!(action_plan(&fixture, command, ENTRY_LIMIT), [Action::CreateFolder { path: downloads.join("test1") }]);
    fs::create_dir_all(downloads.join("test1/nested")).unwrap();
    fs::write(downloads.join("test1/old.txt"), b"old").unwrap();
    fs::write(downloads.join("test1/nested/keep.txt"), b"keep").unwrap();
    assert_eq!(action_plan(&fixture, command, ENTRY_LIMIT), [Action::Trash { path: downloads.join("test1/old.txt") }]);
}

/// The repeating tasks a user had stopped for review after an update must
/// still plan each run against today's files.
#[test]
fn messy_mac_plans_the_restored_repeating_tasks() {
    let fixture = MessyMac::new();
    let sorted = action_plan(&fixture, "sort downloads by type", ENTRY_LIMIT);
    assert!(sorted.iter().any(|a| matches!(a, Action::Move { from, .. } if from.ends_with("Resume 📄.pdf"))), "{sorted:?}");
    let moved = action_plan(&fixture, "Move screenshots from desktop to screenshots folder", ENTRY_LIMIT);
    assert!(
        moved.iter().any(|a| matches!(a, Action::Move { from, to } if from.ends_with("Screenshot 2026-10-28.png") && to.parent().is_some_and(|p| p.ends_with("Pictures/Screenshots")))),
        "{moved:?}"
    );
}

/// "Documents/Trash" and "Documents/bin" are folder paths: moving a file there
/// must never put it in the macOS Trash, even when the folder doesn't exist.
#[test]
fn messy_mac_a_folder_path_named_trash_is_not_the_trash() {
    let fixture = MessyMac::new();
    for to in ["Documents/Trash", "Documents/bin", "~/Documents/Recycle Bin"] {
        let command = format!("move Downloads/Archive.zip to {to}");
        let actions = action_plan(&fixture, &command, ENTRY_LIMIT);
        assert!(!actions.iter().any(|a| matches!(a, Action::Trash { .. })), "{command}: {actions:?}");
        assert!(actions.iter().any(|a| matches!(a, Action::Move { .. })), "{command}: {actions:?}");
    }
    for command in [
        "move Downloads/Archive.zip to \"Trash\"",
        "move Downloads/Archive.zip to \"bin\"",
        "move Downloads/Archive.zip to a new folder called Trash",
        "copy Downloads/Archive.zip to a folder named \"bin\"",
    ] {
        let actions = action_plan(&fixture, command, ENTRY_LIMIT);
        assert!(!actions.iter().any(|a| matches!(a, Action::Trash { .. })), "{command}: {actions:?}");
        assert!(actions.iter().any(|a| matches!(a, Action::Move { .. } | Action::Copy { .. })), "{command}: {actions:?}");
    }
    for command in ["move Downloads/Archive.zip to Trash", "move Downloads/Archive.zip to the trash"] {
        let actions = action_plan(&fixture, command, ENTRY_LIMIT);
        assert!(actions.iter().any(|a| matches!(a, Action::Trash { .. })), "{command}: {actions:?}");
    }
}
