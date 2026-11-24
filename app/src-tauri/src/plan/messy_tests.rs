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
            if let Some((paths, _)) = listed {
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
    let folder = match intent {
        Intent::Move { src, .. } | Intent::Copy { src, .. } | Intent::Trash { src } => match src {
            Source::Set { folder, filter } if !Planner::includes_nested_files(folder.as_deref(), filter) => folder.as_deref(),
            _ => return,
        },
        Intent::SortByType { folder, include_subfolders: false } | Intent::SortByMonth { folder, include_subfolders: false } => {
            Some(folder.as_str())
        }
        _ => return,
    };
    let direct_roots = match folder {
        Some(name) => vec![
            Planner::new(roots, &HashMap::new())
                .resolve_folder(name, None)
                .unwrap_or_else(|err| panic!("{command:?}: couldn't resolve direct source folder {name:?}: {err:?}")),
        ],
        None => vec![roots.desktop(), roots.downloads()],
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
    assert!(matches!(complete_plan(&fixture, "list PDFs in Documents", ENTRY_LIMIT), Planned::Info { listed: Some((paths, _)), .. }
        if paths.len() == 2 && paths.contains(&invoice) && paths.contains(&docs.join("Project/spec.pdf"))));
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
