use super::*;
#[cfg(feature = "tiny-model")]
use plan_model::RoutingMode::{Fallback, Legacy};
use std::{cell::Cell, fs};
struct Fixture {
    _temp: tempfile::TempDir,
    home: PathBuf,
    state: AppState,
    request: Cell<u64>,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let home = fsx::canonical(temp.path());
        for folder in ["Downloads", "Desktop", "Documents", "Pictures"] {
            fs::create_dir(home.join(folder)).unwrap();
        }
        let state = AppState {
            roots: RwLock::new(Roots::new(&home, None, &[])),
            history: Mutex::new(History::load(home.join("history.json"))),
            pending: Mutex::new(None),
            settings: Mutex::new(Settings::default()),
            settings_blocked: None,
            settings_path: home.join("settings.json"),
            reports_path: home.join("reports.jsonl"),
            next_plan: AtomicU64::new(1),
            active_search: Mutex::new(None),
            presentation: Mutex::new(Default::default()),
            result_scope: Mutex::new(Default::default()),
            schedules: Mutex::new(scheduler::Store::load(home.join("schedules.json"))),
            execution: Mutex::new(()),
            active_runs: Mutex::new(HashMap::new()),
            selection_notice: Mutex::new(None),
        };
        Self { _temp: temp, home, state, request: Cell::new(0) }
    }
    fn write(&self, path: &str) -> PathBuf {
        let path = self.home.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"data").unwrap();
        path
    }
    fn command(&self, text: &str) -> Outcome {
        self.request.set(self.request.get() + 1);
        plan_input(&self.state, text.into(), HashMap::new(), self.request.get())
    }
    #[cfg(feature = "tiny-model")]
    fn command_with_model(&self, text: &str, mode: plan_model::RoutingMode, wire: impl Fn(&str) -> &'static str) -> Outcome {
        self.request.set(self.request.get() + 1);
        plan_input_routed(
            &self.state,
            text.into(),
            HashMap::new(),
            self.request.get(),
            None,
            &search::Control::default(),
            ModelRouting { enabled: true, mode, interpret: &|text, _, now| plan_model::Prepared::new(text)?.finish(wire(text), text, now) },
        )
    }
    fn paths(&self) -> Vec<PathBuf> {
        self.state.result_scope.lock().unwrap().current.as_ref().unwrap().paths()
    }
    fn actions(&self) -> Vec<Action> {
        self.state.pending.lock().unwrap().as_ref().unwrap().actions.clone()
    }
    fn list(&self) {
        assert!(matches!(self.command("list the items in Downloads"), Outcome::Info { .. }));
    }
}

#[cfg(feature = "tiny-model")]
const MOVE_ITS_CONTENT: &str = "move its content from Downloads to Pictures";

#[cfg(feature = "tiny-model")]
fn unexpected_inference(text: &str) -> &'static str {
    panic!("Unexpected model call for {text:?}");
}

#[test]
#[cfg(feature = "tiny-model")]
fn model_fallback_previews_a_missing_transfer_item_with_its_original_schedule() {
    let f = Fixture::new();
    let file = f.write("Downloads/report.pdf");
    for scheduled in [false, true] {
        let command = if scheduled { format!("{MOVE_ITS_CONTENT} at 5 pm") } else { MOVE_ITS_CONTENT.into() };
        let calls = Cell::new(0);
        let outcome = f.command_with_model(&command, Fallback, |text| {
            assert_eq!(text, command);
            calls.set(calls.get() + 1);
            if scheduled {
                r#"["plan",[["move","files","Downloads","Pictures"]],["at","17:00","next"]]"#
            } else {
                r#"["plan",[["move","files","Downloads","Pictures"]],null]"#
            }
        });
        assert_eq!(calls.get(), 1);
        assert!(
            matches!(&outcome, Outcome::Plan { total: 1, model_interpreted: true, scheduled_for, .. }
            if scheduled_for.is_some() == scheduled),
            "{outcome:?}"
        );
        assert!(file.exists());
        assert!(!f.home.join("Pictures/report.pdf").exists());
    }
}

#[test]
#[cfg(feature = "tiny-model")]
fn model_fallback_keeps_clean_rules_and_explicit_item_names() {
    let f = Fixture::new();
    for command in ["create a folder in downloads named test1", "create a folder in downloads named test1 and delete its content"] {
        let outcome = f.command_with_model(command, Fallback, unexpected_inference);
        assert!(matches!(&outcome, Outcome::Plan { total: 1, model_interpreted: false, .. }), "{outcome:?}");
    }
    for name in [r#""its    content""#, "its-notes", "IT-docs"] {
        let outcome = f.command_with_model(&format!("move {name} from Downloads to Pictures"), Fallback, unexpected_inference);
        assert!(matches!(&outcome, Outcome::Problem { message } if message.starts_with("Couldn't find")), "{outcome:?}");
    }
    f.write("Downloads/its content");
    let outcome = f.command_with_model(MOVE_ITS_CONTENT, Fallback, unexpected_inference);
    assert!(matches!(outcome, Outcome::Plan { total: 1, model_interpreted: false, .. }));
}

#[test]
#[cfg(feature = "tiny-model")]
fn model_fallback_keeps_explicit_folder_lookups_on_the_rule_path() {
    let f = Fixture::new();
    f.write("Downloads/unrelated.pdf");
    for folder in ["Contents", r#""IT""#, r#""Stuff""#, r#""The   Inbox""#] {
        for suffix in ["", " every week"] {
            let command = format!("move PDFs from {folder} to Downloads{suffix}");
            let outcome = f.command_with_model(&command, Fallback, unexpected_inference);
            assert!(
                matches!(&outcome, Outcome::Problem { message } if message.starts_with("Couldn't find a folder named")),
                "{command}: {outcome:?}"
            );
            assert!(f.state.pending.lock().unwrap().is_none());
        }
    }
    // Unquoted IT is already refused as a pronoun by the parser.
    let outcome =
        f.command_with_model("move PDFs from IT to Downloads", Fallback, |_| r#"["plan",[["move","pdfs","Downloads","IT"]],null]"#);
    assert!(matches!(outcome, Outcome::NotUnderstood { .. }));
    for command in [r#"create a folder named Reports in "The Inbox""#, r#"rename "The Inbox" from Documents to Reports"#] {
        let outcome = f.command_with_model(command, Fallback, unexpected_inference);
        assert!(matches!(&outcome, Outcome::Problem { message } if message.starts_with("Couldn't find")), "{command}: {outcome:?}");
    }
    for folder in ["Contents", "IT"] {
        let path = f.write(&format!("Documents/{folder}/report.pdf"));
        let command = format!(r#"move PDFs from "{folder}" to Downloads"#);
        let outcome = f.command_with_model(&command, Fallback, unexpected_inference);
        assert!(matches!(&outcome, Outcome::Plan { total: 1, model_interpreted: false, .. }), "{outcome:?}");
        assert!(matches!(&f.state.pending.lock().unwrap().as_ref().unwrap().actions[..], [Action::Move { from, .. }] if *from == path));
    }
}

#[test]
#[cfg(feature = "tiny-model")]
fn model_fallback_keeps_filters_on_transfer_grammar_misses() {
    let f = Fixture::new();
    f.write("Downloads/report.pdf");
    for command in [
        "Could PDFs older than 90 days from Downloads be moved to Documents",
        "Could report.pdf from Downloads be relocated to Documents",
        "back up PDFs larger than 50 MB from Downloads to Documents",
        "place files containing invoice from Downloads into Documents",
        "put unread PDFs from Downloads in the folder inside Documents",
        r#"stash "report.pdf" from Downloads to the folder inside Documents"#,
        "file big files from Downloads to the folder inside Documents",
    ] {
        let outcome = f.command_with_model(command, Fallback, |_| {
            if command.contains('"') {
                r#"["plan",[["rename","~0~","Downloads","Documents"]],null]"#
            } else {
                r#"["plan",[["move","pdfs","Downloads","Documents"]],null]"#
            }
        });
        assert!(matches!(outcome, Outcome::NotUnderstood { .. } | Outcome::Problem { .. }), "{command}: {outcome:?}");
        assert!(f.state.pending.lock().unwrap().is_none());
    }
    let command = "Could PDFs from Downloads be relocated to Documents?";
    assert_eq!(parse::parse_classified(command), Err(parse::ParseFailure::GrammarMiss));
    let outcome = f.command_with_model(command, Fallback, |_| r#"["plan",[["move","pdfs","Downloads","Documents"]],null]"#);
    assert!(matches!(outcome, Outcome::Plan { total: 1, model_interpreted: true, .. }), "{outcome:?}");
}

#[test]
#[cfg(feature = "tiny-model")]
fn model_fallback_keeps_the_lookup_error_when_the_model_changes_the_source() {
    let f = Fixture::new();
    let outcome = f.command_with_model(MOVE_ITS_CONTENT, Fallback, |_| r#"["plan",[["move","files","Pictures","Downloads"]],null]"#);
    assert!(matches!(&outcome, Outcome::Problem { message } if message.starts_with("Couldn't find “its content”")), "{outcome:?}");
    assert!(f.state.pending.lock().unwrap().is_none());
}

#[test]
#[cfg(feature = "tiny-model")]
fn model_fallback_refuses_repeating_reinterpretation() {
    let f = Fixture::new();
    for command in [format!("{MOVE_ITS_CONTENT} every week"), "move the PDFs somewhere useful every week".into()] {
        let outcome = f.command_with_model(&command, Fallback, unexpected_inference);
        assert!(matches!(&outcome, Outcome::Problem { message } if message == RULE_ONLY_REPEAT), "{outcome:?}");
        assert!(f.state.pending.lock().unwrap().is_none());
    }
    let command = "delet the last file";
    assert_eq!(parse::parse_classified(command), Err(parse::ParseFailure::GrammarMiss));
    let outcome = f.command_with_model(&format!("{command} every week"), Fallback, unexpected_inference);
    assert!(matches!(outcome, Outcome::NotUnderstood { .. }), "{outcome:?}");
}

#[test]
fn chain_transfers_only_the_created_folders_contents() {
    for (verb, subject) in [("move", "its content"), ("copy", "its files"), ("move", "everything in it"), ("copy", "the files in it")] {
        let f = Fixture::new();
        f.write("Downloads/unrelated.pdf");
        let command = format!("create a folder in downloads named test1 and {verb} {subject} to pictures");
        let folder = f.home.join("Downloads/test1");
        let outcome = f.command(&command);
        assert!(
            matches!(&outcome, Outcome::Plan { warnings, model_interpreted: false, .. }
            if warnings.iter().any(|warning| warning.contains("nothing to do yet"))),
            "{command}: {outcome:?}"
        );
        assert_eq!(f.actions(), [Action::CreateFolder { path: folder.clone() }]);

        let file = f.write("Downloads/test1/report.pdf");
        let to = f.home.join("Pictures/report.pdf");
        let outcome = f.command(&command);
        assert!(matches!(outcome, Outcome::Plan { model_interpreted: false, .. }), "{command}: {outcome:?}");
        let expected = if verb == "move" { Action::Move { from: file, to } } else { Action::Copy { from: file, to } };
        assert_eq!(f.actions(), [expected]);
    }
}

#[test]
#[cfg(feature = "tiny-model")]
fn model_routing_keeps_trash_commands_on_the_rule_path() {
    let f = Fixture::new();
    let path = f.write("Downloads/report.pdf");
    for mode in [Fallback, Legacy] {
        for command in ["send to the trash all files in Downloads", r#"move files from Downloads to "Trash""#] {
            let outcome = f.command_with_model(command, mode, unexpected_inference);
            assert!(matches!(&outcome, Outcome::Problem { message } if message.starts_with("To be safe")), "{command}: {outcome:?}");
            assert!(f.state.pending.lock().unwrap().is_none());
        }
        let outcome = f.command_with_model("send to the trash PDFs in Downloads", mode, unexpected_inference);
        assert!(matches!(&outcome, Outcome::Plan { total: 1, model_interpreted: false, .. }), "{outcome:?}");
        assert!(path.exists());

        let outcome = f.command_with_model(r#"move its content from Downloads to "Trash""#, mode, unexpected_inference);
        assert!(matches!(&outcome, Outcome::Problem { message } if message.starts_with("Couldn't find")), "{outcome:?}");

        let command = r#"move files from Downloads to the "Trash" folder inside Documents"#;
        assert_eq!(parse::parse_classified(command), Err(parse::ParseFailure::GrammarMiss));
        let calls = Cell::new(0);
        let outcome = f.command_with_model(command, mode, |_| {
            calls.set(calls.get() + 1);
            r#"["plan",[["move","files","Downloads","~0~"]],null]"#
        });
        assert_eq!(calls.get(), 1);
        assert!(matches!(&outcome, Outcome::Problem { message } if message.starts_with("Trash commands")), "{outcome:?}");
        assert!(f.state.pending.lock().unwrap().is_none());
    }
}

#[test]
#[cfg(feature = "tiny-model")]
fn model_routing_refuses_a_nickname_that_resolves_to_trash() {
    let f = Fixture::new();
    f.write("Downloads/report.pdf");
    let trash = f.home.join("Documents/Trash");
    fs::create_dir(&trash).unwrap();
    let mut roots = f.state.roots.write().unwrap();
    *roots = roots.clone().with_nicknames(vec![("archive".into(), trash)]);
    drop(roots);
    let command = "copy PDFs from Downloads to Archive";
    let outcome = f.command_with_model(command, Legacy, |_| r#"["plan",[["copy","pdfs","Downloads","Archive"]],null]"#);
    assert!(matches!(&outcome, Outcome::Problem { message } if message.starts_with("Trash commands")), "{outcome:?}");
    assert!(f.state.pending.lock().unwrap().is_none());
}

#[test]
#[cfg(feature = "tiny-model")]
fn model_routing_allows_a_root_under_a_bin_directory() {
    let f = Fixture::new();
    f.write("Downloads/report.pdf");
    let work = f.home.join("bin/Work");
    fs::create_dir_all(&work).unwrap();
    *f.state.roots.write().unwrap() = Roots::new(&f.home, None, std::slice::from_ref(&work));
    let outcome =
        f.command_with_model("copy PDFs from Downloads to Work", Legacy, |_| r#"["plan",[["copy","pdfs","Downloads","Work"]],null]"#);
    assert!(matches!(outcome, Outcome::Plan { model_interpreted: true, .. }), "{outcome:?}");
    assert!(matches!(&f.actions()[..], [Action::Copy { to, .. }] if to == &work.join("report.pdf")));
}

#[test]
#[cfg(feature = "tiny-model")]
fn legacy_model_routing_still_probes_before_rules_and_does_not_retry_misses() {
    let f = Fixture::new();
    let clean = "create a folder in downloads named test1";
    let calls = Cell::new(0);
    let outcome = f.command_with_model(clean, Legacy, |text| {
        assert_eq!(text, clean);
        calls.set(calls.get() + 1);
        r#"["plan",[["mkdir","test1","downloads"]],null]"#
    });
    assert!(matches!(outcome, Outcome::Plan { model_interpreted: true, .. }));
    assert_eq!(calls.get(), 1);
    for command in [MOVE_ITS_CONTENT, &format!("{MOVE_ITS_CONTENT} every week")] {
        let outcome = f.command_with_model(command, Legacy, unexpected_inference);
        assert!(matches!(&outcome, Outcome::Problem { message } if message.starts_with("Couldn't find “its content”")), "{outcome:?}");
    }
}

#[test]
fn schedule_phrases_never_create_accidental_destinations() {
    let f = Fixture::new();
    f.write("Desktop/deck.pptx");
    f.write("Desktop/dinner.jpg");
    f.write("Downloads/a.pdf");
    for text in [
        "move deck.pptx from Desktop to Documents on Friday",
        "move deck.pptx from Desktop to Documents on firday",
        "move a.pdf from Downloads to Documents yesterday at 3pm",
        "move a.pdf from Downloads to Documents on 2025-01-01 at 10am",
        "move dinner.jpg from Desktop to Pictures each morning",
        "move a.pdf from Downloads to Documents every month",
        "move a.pdf from Downloads to Documents every -5 minutes",
        "move a.pdf from Downloads to Documents every 0 minutes",
        "move a.pdf from Downloads to Documents every day and move deck.pptx from Desktop to Documents every week",
    ] {
        assert!(matches!(f.command(text), Outcome::Problem { .. }), "{text}");
        assert!(f.state.pending.lock().unwrap().is_none(), "{text}");
    }
    for text in [
        "move deck.pptx from Desktop to Documents on 2099-01-01 at 2pm",
        "in 10 minutes move a.pdf from Downloads to Documents",
        "move a.pdf from Downloads to Documents in 2 hours and 30 minutes",
        "move dinner.jpg from Desktop to Pictures weekly on Friday at 5pm",
        "move dinner.jpg from Desktop to Pictures every morning at 8",
    ] {
        let outcome = f.command(text);
        assert!(matches!(outcome, Outcome::Plan { scheduled_for: Some(_), .. }), "{text}: {outcome:?}");
        assert!(f.actions().iter().all(|a| !matches!(a, Action::CreateFolder { .. })), "{text}");
    }
}
#[test]
fn weekly_emptying_of_a_named_subfolder_is_scheduled() {
    let f = Fixture::new();
    f.write("Downloads/installers/a.dmg");
    let outcome = f.command("delete items in installers folder every week");
    let Outcome::Plan { id, scheduled_for: Some(_), repeats: Some(_), .. } = outcome else { panic!("{outcome:?}") };
    assert!(confirm_input(&f.state, id).unwrap().error.is_none());
    let job = f.state.schedules.lock().unwrap().jobs[0].clone();
    assert_eq!(job.repeat.as_ref().unwrap().command, "delete items in installers folder");
    assert!(matches!(f.command("delete everything in Downloads every week"), Outcome::Problem { .. }));
}
#[test]
fn recovery_and_notice_do_not_wait_for_history_lock() {
    let f = Fixture::new();
    let _history = f.state.history.lock().unwrap();
    assert!(scheduler_notice_for(&f.state).is_none());
    assert!(matches!(recovery_status_for(&f.state).unwrap(), operation_journal::RecoveryPoll::Clear));
}

#[test]
fn recovery_poll_distinguishes_busy_clear_and_needs_review() {
    use operation_journal::RecoveryPoll;
    let f = Fixture::new();
    assert!(matches!(recovery_status_for(&f.state).unwrap(), RecoveryPoll::Clear));
    let running = f.state.execution.lock().unwrap();
    let busy = recovery_status_for(&f.state).unwrap();
    assert_eq!(serde_json::to_value(busy).unwrap(), serde_json::json!({"status": "busy"}));
    drop(running);
    fs::write(f.home.join("operation-journal.json"), br#"{"entry":null,"actions":[],"in_flight":0,"finished":false}"#).unwrap();
    let review = serde_json::to_value(recovery_status_for(&f.state).unwrap()).unwrap();
    assert_eq!(review["status"], "needs_review");
    assert_eq!(review["acknowledged"], 0);
}

#[test]
fn undo_rejects_replacement_at_plan_and_confirm() {
    for replace_after_preview in [false, true] {
        let f = Fixture::new();
        let source = f.write("Downloads/a.pdf");
        let copy = f.home.join("Documents/a.pdf");
        let Outcome::Plan { id, .. } = f.command("copy a.pdf from Downloads to Documents") else { panic!("expected copy plan") };
        assert!(confirm_input(&f.state, id).unwrap().undoable);
        let undo_preview = replace_after_preview.then(|| f.command("undo last"));
        fs::rename(&copy, f.home.join("Documents/older.pdf")).unwrap();
        fs::write(&copy, b"new file").unwrap();
        let failure = if let Some(Outcome::Plan { id, is_undo: true, .. }) = undo_preview {
            confirm_input(&f.state, id).unwrap().error.unwrap()
        } else {
            let Outcome::Problem { message } = f.command("undo last") else { panic!("replacement should block the preview") };
            message
        };
        assert!(failure.contains("was replaced"), "{failure}");
        assert_eq!(fs::read(&copy).unwrap(), b"new file");
        assert_eq!(fs::read(source).unwrap(), b"data");
        assert!(!f.home.join(".Trash").exists());
        assert!(matches!(recovery_status_for(&f.state).unwrap(), operation_journal::RecoveryPoll::Clear));
    }
}

#[test]
fn undo_entry_targets_the_displayed_run_after_a_scheduled_run() {
    let f = Fixture::new();
    f.write("Downloads/a.pdf");
    f.write("Downloads/b.pdf");
    let Outcome::Plan { id, .. } = f.command("copy a.pdf from Downloads to Documents") else { panic!("expected copy plan") };
    let first = confirm_input(&f.state, id).unwrap();
    let Outcome::Plan { id, .. } = f.command("in 10 minutes copy b.pdf from Downloads to Documents") else {
        panic!("expected scheduled plan")
    };
    let scheduled = confirm_input(&f.state, id).unwrap();
    assert!(matches!(scheduled.status, model::RunStatus::Scheduled));
    assert!(scheduled.entry_id.is_none());
    f.state.schedules.lock().unwrap().jobs[0].due = chrono::Local::now() - chrono::Duration::seconds(1);
    run_due_tasks_inner(&f.state).unwrap();
    assert!(f.home.join("Documents/b.pdf").exists());
    assert_ne!(first.entry_id, f.state.history.lock().unwrap().last_undoable().map(|e| e.id));

    let planned = plan_input_for(&f.state, "undo last".into(), HashMap::new(), f.request.get() + 1, first.entry_id);
    let Outcome::Plan { id, is_undo: true, .. } = planned else { panic!("{planned:?}") };
    assert!(confirm_input(&f.state, id).unwrap().error.is_none());
    assert!(!f.home.join("Documents/a.pdf").exists());
    assert!(f.home.join("Documents/b.pdf").exists());
    assert!(matches!(f.command("undo last"), Outcome::Plan { is_undo: true, .. }));
    assert!(matches!(&f.actions()[0], Action::Trash { path } if path.ends_with("b.pdf")));
}

#[test]
fn published_schedule_is_retained_when_directory_sync_fails() {
    let f = Fixture::new();
    f.write("Downloads/a.pdf");
    let Outcome::Plan { id, .. } = f.command("in 10 minutes copy a.pdf from Downloads to Documents") else {
        panic!("expected scheduled plan")
    };
    settings::WRITE_FAILURE.with(|failure| failure.set(4));
    let result = confirm_input(&f.state, id);
    settings::WRITE_FAILURE.with(|failure| failure.set(0));
    assert!(matches!(result.unwrap().status, model::RunStatus::Scheduled));
    let current = f.state.schedules.lock().unwrap();
    assert_eq!(current.jobs.len(), 1);
    assert!(current.notice().unwrap().contains("durability could not be confirmed"));
    let reloaded = scheduler::Store::load(f.home.join("schedules.json"));
    assert_eq!(reloaded.jobs[0].id, current.jobs[0].id);
}

#[test]
fn stopping_between_moves_persists_only_completed_changes_and_resolves_journal() {
    let f = Fixture::new();
    let a = f.write("Downloads/a.pdf");
    let b = f.write("Downloads/b.pdf");
    let moved_a = f.home.join("Documents/a.pdf");
    let moved_b = f.home.join("Documents/b.pdf");
    let actions = vec![Action::Move { from: a.clone(), to: moved_a.clone() }, Action::Move { from: b.clone(), to: moved_b.clone() }];
    let key = RunKey::Plan(1);
    let run = ActiveRun::register(&f.state.active_runs, key, false).unwrap();
    let result = execute_pending(
        &f.state,
        Pending {
            id: 1,
            command: "move PDFs".into(),
            actions: actions.clone(),
            preview: PreviewSource::default(),
            inputs: Vec::new(),
            undo_of: None,
            context: None,
            scheduled: None,
            stop_requested: false,
        },
        &|| {
            if moved_a.exists() {
                request_stop(&f.state, key).unwrap();
            }
            run.is_stopped()
        },
    );
    drop(run);
    assert!(result.cancelled && result.error.is_none() && result.undoable);
    assert_eq!(result.summary, "Stopped. Moved 1 of 2 items. Undo reverses those changes.");
    assert!(!a.exists() && moved_a.exists() && b.exists() && !moved_b.exists());

    let mut saved = History::load(f.home.join("history.json"));
    let entry = saved.last_undoable().unwrap();
    assert_eq!(entry.done, actions[..1]);
    assert_eq!(entry.inverses()[0].action, Action::Move { from: moved_a.clone(), to: a.clone() });
    let journal = saved.journal_path();
    assert!(operation_journal::status(&journal).unwrap().is_none());
    operation_journal::recover(&journal, &mut saved).unwrap();
    assert_eq!(saved.entries.len(), 1);

    let Outcome::Plan { id, total: 1, .. } = f.command("undo") else { panic!("expected one undo step") };
    let undo = confirm_input(&f.state, id).unwrap();
    assert!(undo.error.is_none() && !undo.cancelled);
    assert!(a.exists() && !moved_a.exists() && b.exists() && !moved_b.exists());
    assert!(History::load(f.home.join("history.json")).last_undoable().is_none());
}

#[test]
fn stop_requested_before_confirmation_never_changes_files_or_the_next_plan() {
    let f = Fixture::new();
    let source = f.write("Downloads/a.pdf");
    let target = f.home.join("Documents/a.pdf");
    let command = "move a.pdf from Downloads to Documents";
    let Outcome::Plan { id, .. } = f.command(command) else { panic!("expected move plan") };
    request_stop(&f.state, RunKey::Plan(id)).unwrap();
    let stopped = confirm_input(&f.state, id).unwrap();
    assert!(stopped.cancelled && stopped.error.is_none() && !stopped.undoable);
    assert_eq!(stopped.done, 0);
    assert!(source.exists() && !target.exists());
    assert!(matches!(recovery_status_for(&f.state).unwrap(), operation_journal::RecoveryPoll::Clear));
    assert!(f.state.history.lock().unwrap().entries.is_empty());

    let Outcome::Plan { id: next_id, .. } = f.command(command) else { panic!("expected new move plan") };
    request_stop(&f.state, RunKey::Plan(id)).unwrap();
    let finished = confirm_input(&f.state, next_id).unwrap();
    assert!(finished.error.is_none() && !finished.cancelled);
    assert!(!source.exists() && target.exists());
}

#[test]
fn stopping_a_scheduled_occurrence_keeps_its_next_run() {
    let f = Fixture::new();
    let source = f.write("Downloads/a.pdf");
    let command = "move a.pdf from Downloads to Documents";
    let due = chrono::Local::now() - chrono::Duration::minutes(1);
    let (_, when) = scheduler::extract_schedule(&format!("{command} daily"), chrono::Local::now()).unwrap();
    let rule = when.unwrap().rule.unwrap();
    f.state.schedules.lock().unwrap().add(scheduler::Job::recurring_later(command.into(), due, rule)).unwrap();
    let state = &f.state;
    std::thread::scope(|scope| {
        // The job registers the occurrence before trying to acquire history.
        let history = state.history.lock().unwrap();
        let worker = scope.spawn(|| run_due_tasks_inner(state));
        let start = std::time::Instant::now();
        loop {
            if state.schedules.lock().unwrap().jobs[0].occurrence == scheduler::OccurrenceState::Running {
                break;
            }
            assert!(start.elapsed() < std::time::Duration::from_secs(5), "job did not start");
            std::thread::yield_now();
        }
        request_stop(state, RunKey::Scheduled(1, due)).unwrap();
        drop(history);
        worker.join().unwrap().unwrap();
    });
    assert!(source.exists());
    let saved = scheduler::Store::load(f.home.join("schedules.json"));
    let job = &saved.jobs[0];
    assert_eq!(job.series, scheduler::SeriesState::Active);
    assert!(job.due > chrono::Local::now());
    assert!(job.summary.contains("Stopped. Moved 0 of 1 item."));
    let next = ActiveRun::register(&state.active_runs, RunKey::Scheduled(job.id, job.due), false).unwrap();
    request_stop(state, RunKey::Scheduled(job.id, due)).unwrap();
    assert!(!next.is_stopped());
}

#[test]
fn scheduler_leaves_due_job_pending_until_recovery_reviewed() {
    let f = Fixture::new();
    let output = f.home.join("Documents/scheduled");
    let job = scheduler::Job::new(
        "create scheduled".into(),
        chrono::Local::now() - chrono::Duration::minutes(1),
        vec![Action::CreateFolder { path: output.clone() }],
    )
    .unwrap();
    f.state.schedules.lock().unwrap().add(job).unwrap();
    let journal = f.home.join("operation-journal.json");
    fs::write(&journal, b"broken journal").unwrap();
    for _ in 0..2 {
        run_due_tasks_inner(&f.state).unwrap();
    }
    assert_eq!(f.state.schedules.lock().unwrap().jobs[0].series, scheduler::SeriesState::Active);
    assert!(!output.exists());
    operation_journal::acknowledge(&journal, &mut f.state.history.lock().unwrap()).unwrap();
    run_due_tasks_inner(&f.state).unwrap();
    assert_eq!(f.state.schedules.lock().unwrap().jobs[0].series, scheduler::SeriesState::Completed);
    assert!(output.exists());
}

#[test]
fn register_equivalence_binary() {
    let binary = std::env::current_exe().unwrap();
    let variant = if cfg!(feature = "tiny-model") { "tiny-model" } else { "default" };
    // Must match profile_dir in tests/rule_only_equivalence.rs: the shared
    // profile folder above deps/ or build/<pkg>/<hash>/out/.
    let profile = binary
        .ancestors()
        .skip(1)
        .find(|dir| matches!(dir.file_name().and_then(|n| n.to_str()), Some("deps" | "build")))
        .and_then(std::path::Path::parent)
        .unwrap_or_else(|| binary.parent().unwrap());
    fs::write(profile.join(format!("voxi-equivalence-{variant}.json")), serde_json::to_vec(&binary).unwrap()).unwrap();
}

/// Invoked in separate binaries by the cross-feature integration test.
#[test]
fn rule_only_probe() {
    let Ok(base) = std::env::var("VOXI_EQUIVALENCE_HOME") else { return };
    let home = fsx::canonical(std::path::Path::new(&base));
    for folder in ["Downloads", "Desktop", "Documents", "Pictures"] {
        fs::create_dir_all(home.join(folder)).unwrap();
    }
    for path in ["Downloads/a.pdf", "Downloads/one/same.txt", "Downloads/two/same.txt"] {
        let path = home.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"fixture").unwrap();
    }
    let state = AppState {
        roots: RwLock::new(Roots::new(&home, None, &[])),
        history: Mutex::new(History::load(home.join("history.json"))),
        pending: Mutex::new(None),
        settings: Mutex::new(Settings::default()),
        settings_blocked: None,
        settings_path: home.join("settings.json"),
        reports_path: home.join("reports.jsonl"),
        next_plan: AtomicU64::new(1),
        active_search: Mutex::new(None),
        presentation: Mutex::new(Default::default()),
        result_scope: Mutex::new(Default::default()),
        schedules: Mutex::new(scheduler::Store::load(home.join("schedules.json"))),
        execution: Mutex::new(()),
        active_runs: Mutex::new(HashMap::new()),
        selection_notice: Mutex::new(None),
    };
    let oversized = "x".repeat(parse::MAX_COMMAND_BYTES + 1);
    let mut outcomes = Vec::new();
    for text in [
        "delete everything on my computer",
        "",
        "!!!",
        "launch the moon",
        &oversized,
        "move a.pdf from Downloads to Documents in 5 minutes",
        "delete same.txt",
        "list PDFs in Downloads",
        "delete the first file",
    ] {
        outcomes.push(serde_json::to_value(plan_input(&state, text.into(), HashMap::new(), 1)).unwrap());
    }
    let actions = vec![
        Action::CreateFolder { path: home.join("Documents/created") },
        Action::Move { from: home.join("Downloads/missing"), to: home.join("Documents/missing") },
    ];
    outcomes.push(
        serde_json::to_value(execute_pending(
            &state,
            Pending {
                id: 1,
                command: "partial failure".into(),
                actions,
                preview: PreviewSource::default(),
                inputs: Vec::new(),
                undo_of: None,
                context: None,
                scheduled: None,
                stop_requested: false,
            },
            &|| false,
        ))
        .unwrap(),
    );
    let effects = serde_json::json!({
        "source": fs::read(home.join("Downloads/a.pdf")).unwrap(),
        "destinationExists": home.join("Documents/a.pdf").exists(),
        "createdExists": home.join("Documents/created").exists(),
        "missingExists": home.join("Documents/missing").exists(),
        "history": state.history.lock().unwrap().entries,
        "journal": serde_json::from_slice::<serde_json::Value>(&fs::read(home.join("operation-journal.json")).unwrap()).unwrap()
    });
    let identity = serde_json::to_value(fsx::FileIdentity::read(&home.join("Documents/created")).unwrap()).unwrap();
    assert_eq!(effects.pointer("/history/0/state/undoable/0/identities/0/root"), Some(&identity));
    assert_eq!(effects.pointer("/journal/data/state/finished/entry/state/undoable/0/identities/0/root"), Some(&identity));
    outcomes.push(effects);
    fs::write(home.join("outcomes.json"), serde_json::to_vec_pretty(&outcomes).unwrap()).unwrap();
}

#[test]
fn shortcut_with_one_read_only_command_runs_it_as_typed() {
    let f = Fixture::new();
    f.write("Downloads/a.pdf");
    f.state.settings.lock().unwrap().shortcuts =
        vec![settings::Shortcut { phrase: "White".into(), commands: vec!["list downloads".into()] }];
    assert!(matches!(f.command("white"), Outcome::Info { ref title, .. } if title.contains("a.pdf")));
    // It publishes a list like the typed command, so follow-ups work.
    assert!(matches!(f.command("delete the first one"), Outcome::Plan { .. }));
}

#[test]
fn partial_failure_keeps_durable_undo_and_allows_next_command() {
    let f = Fixture::new();
    let created = f.home.join("Documents/created");
    let result = execute_pending(
        &f.state,
        Pending {
            id: 1,
            command: "partial".into(),
            actions: vec![
                Action::CreateFolder { path: created.clone() },
                Action::Move { from: f.home.join("Downloads/missing"), to: f.home.join("Documents/missing") },
            ],
            preview: PreviewSource::default(),
            inputs: Vec::new(),
            undo_of: None,
            context: None,
            scheduled: None,
            stop_requested: false,
        },
        &|| false,
    );
    assert_eq!(result.done, 1);
    assert!(result.error.is_some());
    assert!(result.undoable);
    assert_eq!(History::load(f.home.join("history.json")).entries[0].done.len(), 1);
    let Outcome::Plan { id, .. } = f.command("undo") else { panic!("expected undo plan") };
    let undo = confirm_input(&f.state, id).unwrap();
    assert!(undo.error.is_none());
    assert!(!undo.undoable);
    assert!(!created.exists());
}

#[test]
fn unfinished_journal_does_not_block_search_list_or_info() {
    let f = Fixture::new();
    f.write("Downloads/a.pdf");
    let path = f.home.join("operation-journal.json");
    let bytes = br#"{"entry":{"id":1,"time":"2026-11-17T12:00:00+05:30","command":"interrupted","done":[],"inverse":[],"undone":false,"undo_of":null},"actions":[],"in_flight":null,"finished":false}"#;
    fs::write(&path, bytes).unwrap();
    for command in ["list the items in Downloads", "find a.pdf", "how big is Downloads"] {
        assert!(matches!(f.command(command), Outcome::Info { .. }), "{command}");
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }
}

#[test]
fn history_save_failure_does_not_advertise_durable_undo() {
    let f = Fixture::new();
    let source = f.write("Downloads/a.pdf");
    let Outcome::Plan { id, .. } = f.command("move a.pdf from Downloads to Documents") else { panic!("expected plan") };
    fs::create_dir(f.home.join("history.json")).unwrap();
    let result = confirm_input(&f.state, id).unwrap();
    assert!(result.error.is_some());
    assert!(!result.undoable);
    assert!(!source.exists());
    assert!(f.home.join("Documents/a.pdf").exists());
    assert!(f.home.join("operation-journal.json").exists());
    // Repair the history destination, then recover the acknowledged change.
    fs::remove_dir(f.home.join("history.json")).unwrap();
    let mut history = History::load(f.home.join("history.json"));
    assert!(operation_journal::recover(&history.journal_path(), &mut history).is_err());
    assert_eq!(history.entries.len(), 1);
    assert_eq!(history.entries[0].inverses().len(), 1);
    let _ = operation_journal::recover(&history.journal_path(), &mut history);
    assert_eq!(history.entries.len(), 1);
}

#[test]
fn ingress_bounds_precede_scheduling_and_never_leave_partial_preview() {
    let f = Fixture::new();
    f.write("Downloads/a.pdf");
    assert!(matches!(f.command("move a.pdf from Downloads to Documents"), Outcome::Plan { .. }));
    let text = format!("{} every day", "x".repeat(parse::MAX_COMMAND_BYTES));
    assert!(matches!(f.command(&text), Outcome::Problem { message } if message == "That command is too long."));
    assert!(f.state.pending.lock().unwrap().is_none());
    assert!(f.state.schedules.lock().unwrap().jobs.is_empty());
    f.state
        .settings
        .lock()
        .unwrap()
        .shortcuts
        .push(settings::Shortcut { phrase: "run oversized".into(), commands: vec!["delete a.pdf".into(); 9] });
    assert!(matches!(f.command("run oversized"), Outcome::Problem { message } if message == "That command is too long."));
    assert!(f.state.pending.lock().unwrap().is_none());
}
#[test]
#[cfg(feature = "tiny-model")]
#[ignore = "requires VOXI_TINY_MODEL_DIR pointing to the trained package"]
fn trained_model_recovers_through_app_and_keeps_schedule_and_scope_boundaries() {
    let f = Fixture::new();
    let text = "Create folders \"Xylophone\", \"Quasar\" in Documents";
    assert!(matches!(f.command(&format!("{text} after 3 hours")), Outcome::NotUnderstood { .. }));
    assert!(f.state.pending.lock().unwrap().is_none());
    f.write("Downloads/example.txt");
    f.list();
    assert!(!matches!(f.command(text), Outcome::Plan { .. }));
    assert!(f.state.pending.lock().unwrap().is_none());
    f.state.result_scope.lock().unwrap().clear();
    assert!(matches!(f.command(text), Outcome::Plan { total: 2, .. }));
    assert_eq!(f.actions().len(), 2);
    assert!(!f.home.join("Documents/Xylophone").exists());
    assert!(!f.home.join("Documents/Quasar").exists());
}

#[test]
fn named_chain_destination_and_filename_list_confirm_and_undo() {
    for text in [
        "create a folder called PDFS in Documents and copy a.pdf from Downloads into PDFS",
        "create a folder called PDFS in Documents and move a.pdf from Downloads into PDFS",
        "make folder ZIP_Archive in Documents and move all zips from Downloads into ZIP_Archive",
        "create a folder called Taxes2026 in Documents and move tax-return-2025.pdf and w2-form-2026.pdf from Downloads into it",
        "create a folder called Travel in Documents and also move x.pdf from Desktop into it",
        "move a.pdf from Desktop to Documents and also move b.pdf from Desktop to Documents",
        "rename todo.txt on Desktop to tasks.txt and move it to Documents",
    ] {
        let f = Fixture::new();
        let originals: Vec<_> = [
            "Downloads/a.pdf",
            "Downloads/one.zip",
            "Downloads/tax-return-2025.pdf",
            "Downloads/w2-form-2026.pdf",
            "Desktop/x.pdf",
            "Desktop/a.pdf",
            "Desktop/b.pdf",
            "Desktop/todo.txt",
        ]
        .iter()
        .map(|p| f.write(p))
        .collect();
        let outcome = f.command(text);
        let Outcome::Plan { id, .. } = outcome else { panic!("{text}: {outcome:?}") };
        assert!(!f.actions().iter().any(|a| matches!(a, Action::CreateFolder { path } if path.starts_with(f.home.join("Desktop")) || path.starts_with(f.home.join("Downloads")))));
        assert!(confirm_input(&f.state, id).unwrap().error.is_none(), "{text}");
        let Outcome::Plan { id, is_undo: true, .. } = f.command("undo last") else { panic!("{text}") };
        assert!(confirm_input(&f.state, id).unwrap().error.is_none(), "{text}");
        assert!(originals.iter().all(|p| p.exists()), "{text}");
    }
}

#[test]
fn all_type_chain_uses_the_single_command_source_roots() {
    for kind in ["mp3s", "pdfs"] {
        let f = Fixture::new();
        let ext = if kind == "mp3s" { "mp3" } else { "pdf" };
        let download = f.write(&format!("Downloads/song.{ext}"));
        let desktop = f.write(&format!("Desktop/track.{ext}"));
        let document = f.write(&format!("Documents/podcast.{ext}"));
        assert!(matches!(f.command(&format!("move all {kind} to Documents/Music")), Outcome::Plan { .. }));
        let single: Vec<_> =
            f.actions().into_iter().filter_map(|a| if let Action::Move { from, .. } = a { Some(from) } else { None }).collect();
        assert!(matches!(f.command(&format!("make a folder called Music in Documents and move all {kind} there")), Outcome::Plan { .. }));
        let chained: Vec<_> =
            f.actions().into_iter().filter_map(|a| if let Action::Move { from, .. } = a { Some(from) } else { None }).collect();
        assert_eq!(single, chained);
        assert!(chained.contains(&download) && chained.contains(&desktop) && !chained.contains(&document));
    }
}

#[test]
fn chained_archive_steps_bind_actual_outputs() {
    for text in [
        "create a folder called Taxes in Documents, move all PDFs from Downloads into it, then zip it",
        "create a folder called Taxes in Documents, move all PDFs from Downloads into it and zip it",
        "copy assets.zip from Downloads to Desktop and unzip it",
        "zip all pdfs in Downloads as pdfs.zip and move it to Documents",
    ] {
        let f = Fixture::new();
        f.write("Downloads/a.pdf");
        f.write("Downloads/assets.zip");
        // Collision forces copy output binding to use the reserved name.
        f.write("Desktop/assets.zip");
        let outcome = f.command(text);
        let Outcome::Plan { .. } = outcome else { panic!("{text}: {outcome:?}") };
        let actions = f.actions();
        if text.contains("unzip") {
            let copied = actions.iter().find_map(|a| if let Action::Copy { to, .. } = a { Some(to) } else { None }).unwrap();
            assert!(actions.iter().any(|a| matches!(a, Action::Unzip { path, .. } if path == copied)));
            assert_ne!(copied, &f.home.join("Desktop/assets.zip"));
        } else if text.starts_with("create") {
            assert!(actions.iter().any(|a| matches!(a, Action::Zip { path, .. } if path == &f.home.join("Documents/Taxes"))));
        } else {
            assert!(actions.iter().any(|a| matches!(a, Action::Move { from, .. } if from == &f.home.join("Downloads/pdfs.zip"))));
        }
    }
}

#[test]
fn folder_move_zip_chain_confirms_and_undoes_in_dependency_order() {
    let f = Fixture::new();
    let original = f.write("Downloads/a.pdf");
    let outcome = f.command("create a folder called Taxes in Documents, move all PDFs from Downloads into it, then zip it");
    let Outcome::Plan { id, .. } = outcome else { panic!("{outcome:?}") };
    let result = confirm_input(&f.state, id).unwrap();
    assert!(result.error.is_none(), "{:?}", result.error);
    assert!(f.home.join("Documents/Taxes.zip").exists());
    assert!(f.home.join("Documents/Taxes/a.pdf").exists());
    assert!(!original.exists());
    let Outcome::Plan { id, is_undo: true, .. } = f.command("undo last") else { panic!() };
    assert!(confirm_input(&f.state, id).unwrap().error.is_none());
    assert!(original.exists());
    assert!(!f.home.join("Documents/Taxes").exists());
    assert!(!f.home.join("Documents/Taxes.zip").exists());
}

#[test]
fn multi_file_zip_chain_refuses_with_an_actionable_message() {
    let f = Fixture::new();
    f.write("Downloads/a.pdf");
    f.write("Downloads/b.pdf");
    let outcome = f.command("zip all pdfs in Downloads as pdfs.zip and move it to Documents");
    let Outcome::Problem { message, .. } = outcome else { panic!("{outcome:?}") };
    assert!(message.contains("put them in one folder"), "{message}");
    assert!(f.state.pending.lock().unwrap().is_none());
}

#[test]
fn convert_then_resize_uses_reserved_output_without_reading_old_png() {
    let f = Fixture::new();
    f.write("Pictures/photo.heic");
    let existing = f.write("Pictures/photo.png");
    let outcome = f.command("convert Pictures/photo.heic to png then resize Pictures/photo.png to 800px wide");
    let Outcome::Plan { .. } = outcome else { panic!("{outcome:?}") };
    let actions = f.actions();
    let output = actions.iter().find_map(|a| if let Action::ConvertImage { output, .. } = a { Some(output) } else { None }).unwrap();
    assert_ne!(output, &existing);
    assert!(actions.iter().any(|a| matches!(a, Action::ResizeImage { path, max_width_px: 800, .. } if path == output)));
}

#[test]
fn chained_folder_and_transfer_preview_confirm_and_undo_are_one_change() {
    let f = Fixture::new();
    let pdf = f.write("Downloads/invoice.pdf");
    let text = f.write("Downloads/notes.txt");
    let nested = f.write("Downloads/Project/keep.pdf");
    let Outcome::Plan { id, total, .. } = f.command("create a folder in downloads called PDFS and move pdfs into it") else {
        panic!("expected chain preview")
    };
    assert_eq!(total, 2);
    assert!(pdf.exists());
    assert!(!f.home.join("Downloads/PDFS").exists());
    let result = confirm_input(&f.state, id).unwrap();
    assert!(result.error.is_none(), "{:?}", result.error);
    assert_eq!(result.done, 2);
    assert!(!pdf.exists());
    assert!(f.home.join("Downloads/PDFS/invoice.pdf").exists());
    assert!(text.exists() && nested.exists());
    assert_eq!(f.state.history.lock().unwrap().entries.len(), 1);
    let Outcome::Plan { id, is_undo: true, .. } = f.command("undo last") else { panic!("expected single undo") };
    let result = confirm_input(&f.state, id).unwrap();
    assert!(result.error.is_none(), "{:?}", result.error);
    assert!(pdf.exists());
    assert!(!f.home.join("Downloads/PDFS").exists());
}

#[test]
fn independent_chained_copy_and_rename_share_one_confirmation() {
    let f = Fixture::new();
    let pdf = f.write("Downloads/invoice.pdf");
    let notes = f.write("Desktop/notes.txt");
    let Outcome::Plan { id, total, .. } = f.command("copy PDFs from Downloads to Documents then rename notes.txt in Desktop to saved.txt")
    else {
        panic!("expected independent chain")
    };
    assert_eq!(total, 2);
    assert!(notes.exists());
    assert!(!f.home.join("Documents/invoice.pdf").exists());
    assert!(confirm_input(&f.state, id).unwrap().error.is_none());
    assert!(pdf.exists());
    assert!(f.home.join("Documents/invoice.pdf").exists());
    assert!(f.home.join("Desktop/saved.txt").exists());
    assert_eq!(f.state.history.lock().unwrap().entries.len(), 1);
    let Outcome::Plan { id, .. } = f.command("undo last") else { panic!() };
    assert!(confirm_input(&f.state, id).unwrap().error.is_none());
    assert!(notes.exists());
    assert!(!f.home.join("Desktop/saved.txt").exists());
}

#[test]
fn chained_commands_never_publish_a_partial_or_stale_scope_plan() {
    let f = Fixture::new();
    let source = f.write("Downloads/a.pdf");
    for text in [
        "create a folder named Archive in Downloads and move images from Pictures into it",
        "move a.pdf from Downloads to Documents then copy a.pdf from Downloads to Desktop",
        "rename a.pdf in Downloads to b.pdf then copy a.pdf from Downloads to Desktop",
    ] {
        let outcome = f.command(text);
        assert!(!matches!(outcome, Outcome::Plan { .. }), "{text}");
        assert!(f.state.pending.lock().unwrap().is_none(), "{text}");
        assert!(source.exists());
        assert!(!f.home.join("Downloads/Archive").exists());
    }
    f.list();
    assert!(matches!(f.command("create a folder named Archive in Downloads and move PDFs into it"), Outcome::Problem { .. }));
    assert!(f.state.pending.lock().unwrap().is_none());
}

#[test]
fn scheduling_preserves_screenshot_names_and_later_time_clauses() {
    let f = Fixture::new();
    let screenshot = f.write("Desktop/Screenshot 2026-11-12 at 09.40.00.png");
    let trip = f.write("Desktop/Trip at Home.txt");
    for (name, source) in [("Screenshot 2026-11-12 at 09.40.00.png", &screenshot), ("Trip at Home.txt", &trip)] {
        let outcome = f.command(&format!("move {name} to Documents"));
        assert!(matches!(outcome, Outcome::Plan { scheduled_for: None, .. }), "{outcome:?}");
        assert_eq!(f.actions(), vec![Action::Move { from: source.clone(), to: f.home.join("Documents").join(name) }]);
        let outcome = f.command(&format!("move {name} to Documents at 9:40 pm"));
        assert!(matches!(outcome, Outcome::Plan { scheduled_for: Some(_), .. }), "{outcome:?}");
        assert_eq!(f.actions(), vec![Action::Move { from: source.clone(), to: f.home.join("Documents").join(name) }]);
    }
}
#[test]
fn scheduled_move_is_deferred_persisted_and_undoable() {
    let f = Fixture::new();
    let source = f.write("Desktop/Screenshot 2026-11-12 at 09.40.00.png");
    let outcome = f.command("Move the screenshots from desktop to Screenshots folder after 3 hours");
    let Outcome::Plan { id, scheduled_for: Some(_), .. } = outcome else {
        panic!("{outcome:?}");
    };
    assert!(source.exists());
    let result = confirm_input(&f.state, id).unwrap();
    assert_eq!(result.done, 0);
    assert!(!result.undoable);
    assert!(source.exists());
    assert!(f.state.history.lock().unwrap().entries.is_empty());
    // Reload from disk, then make the saved task due without waiting.
    let mut store = scheduler::Store::load(f.home.join("schedules.json"));
    store.jobs[0].due = chrono::Local::now() - chrono::Duration::seconds(1);
    *f.state.schedules.lock().unwrap() = store;
    run_due_tasks(&f.state).unwrap();
    assert!(!source.exists());
    assert_eq!(f.state.schedules.lock().unwrap().jobs[0].series, scheduler::SeriesState::Completed);
    assert_eq!(f.state.history.lock().unwrap().entries.len(), 1);
    run_due_tasks(&f.state).unwrap();
    assert_eq!(f.state.history.lock().unwrap().entries.len(), 1);
    assert!(matches!(f.command("undo last"), Outcome::Plan { is_undo: true, .. }));
}
#[test]
#[cfg(feature = "tiny-model")]
#[ignore = "requires VOXI_PLAN_MODEL_DIR pointing to native v4c weights"]
fn structured_model_enabled_recurring_commands_use_the_rule_plan() {
    let f = Fixture::new();
    let source = f.write("Downloads/invoice.pdf");
    let outcome = f.command("create a folder in downloads called PDFS and move pdfs into it everyday");
    let Outcome::Plan { id, total, model_interpreted: false, scheduled_for: Some(_), repeats: Some(_), .. } = outcome else {
        panic!("{outcome:?}")
    };
    assert_eq!(total, 2);
    assert!(source.exists());
    assert!(!f.home.join("Downloads/PDFS").exists());
    assert!(confirm_input(&f.state, id).unwrap().error.is_none());
    let mut store = scheduler::Store::load(f.home.join("schedules.json"));
    assert_eq!(store.jobs[0].repeat.as_ref().unwrap().model_intent, None);
    // A later due run re-plans from the command text, not from a stored
    // typed model plan: the model is not consulted for repeating tasks.
    store.jobs[0].due = chrono::Local::now() - chrono::Duration::days(2);
    *f.state.schedules.lock().unwrap() = store;
    run_due_tasks(&f.state).unwrap();
    assert!(!source.exists());
    assert!(f.home.join("Downloads/PDFS/invoice.pdf").exists());
    assert_eq!(f.state.history.lock().unwrap().entries.len(), 1);
    let Outcome::Plan { id, is_undo: true, .. } = f.command("undo last") else { panic!() };
    assert!(confirm_input(&f.state, id).unwrap().error.is_none());
    assert!(source.exists());
    assert!(!f.home.join("Downloads/PDFS").exists());
}

#[test]
fn daily_chained_task_replans_new_files_and_undoes_each_run_together() {
    let f = Fixture::new();
    let first = f.write("Downloads/a.pdf");
    let untouched = f.write("Downloads/notes.txt");
    let outcome = f.command("create a folder in downloads called PDFS and move pdfs into it everyday");
    let Outcome::Plan { id, scheduled_for: Some(_), repeats: Some(repeats), total, .. } = outcome else { panic!("{outcome:?}") };
    assert_eq!(total, 2);
    assert!(repeats.starts_with("Runs now, then every day"), "{repeats}");
    let result = confirm_input(&f.state, id).unwrap();
    assert!(result.error.is_none());
    assert!(!first.exists());
    assert!(f.home.join("Downloads/PDFS/a.pdf").exists());
    assert_eq!(result.done, 2);
    assert!(result.undoable);
    assert_eq!(f.state.history.lock().unwrap().entries.len(), 1);
    let job = f.state.schedules.lock().unwrap().jobs[0].clone();
    assert_eq!(job.repeat.as_ref().unwrap().command, "create a folder in downloads called PDFS and move pdfs into it");
    let gap = job.due - chrono::Local::now();
    assert!(gap > chrono::Duration::days(1) - chrono::Duration::seconds(5) && gap <= chrono::Duration::days(1));
    let make_due = || f.state.schedules.lock().unwrap().jobs[0].due = chrono::Local::now() - chrono::Duration::days(2);
    assert!(!first.exists());
    assert!(f.home.join("Downloads/PDFS/a.pdf").exists());
    assert_eq!(f.state.history.lock().unwrap().entries.len(), 1);
    // New files enter the following run; the existing PDFS folder is reused.
    let second = f.write("Downloads/b.pdf");
    make_due();
    run_due_tasks(&f.state).unwrap();
    assert!(!second.exists());
    assert!(f.home.join("Downloads/PDFS/b.pdf").exists());
    assert!(untouched.exists());
    assert_eq!(f.state.history.lock().unwrap().entries.len(), 2);
    let Outcome::Plan { id, is_undo: true, .. } = f.command("undo last") else { panic!() };
    assert!(confirm_input(&f.state, id).unwrap().error.is_none());
    assert!(second.exists());
    assert!(f.home.join("Downloads/PDFS/a.pdf").exists());
    f.state.schedules.lock().unwrap().cancel(job.id).unwrap();
    make_due();
    run_due_tasks(&f.state).unwrap();
    assert!(second.exists(), "cancelled schedules must stay stopped");
}

#[test]
fn pdfs_folder_every_two_days_previews_then_reuses_the_folder_on_later_runs() {
    let f = Fixture::new();
    let first = f.write("Downloads/a.pdf");
    let untouched = f.write("Downloads/notes.txt");
    let outcome = f.command("create a folder in downloads named PDFS and move pdf into it every 2 days");
    let Outcome::Plan { id, scheduled_for: Some(_), repeats: Some(repeats), total, .. } = outcome else { panic!("{outcome:?}") };
    assert_eq!(total, 2);
    assert!(repeats.starts_with("Runs now, then every 2 days"), "{repeats}");
    let result = confirm_input(&f.state, id).unwrap();
    assert!(result.error.is_none(), "{result:?}");
    assert!(!first.exists());
    assert!(untouched.exists());
    assert!(f.home.join("Downloads/PDFS/a.pdf").exists());
    assert_eq!(result.done, 2);
    assert!(result.undoable);
    assert_eq!(f.state.history.lock().unwrap().entries.len(), 1);
    let job = f.state.schedules.lock().unwrap().jobs[0].clone();
    assert_eq!(job.repeat.as_ref().unwrap().command, "create a folder in downloads named PDFS and move pdf into it");
    let gap = job.due - chrono::Local::now();
    assert!(gap > chrono::Duration::days(2) - chrono::Duration::seconds(5) && gap <= chrono::Duration::days(2));
    let make_due = || f.state.schedules.lock().unwrap().jobs[0].due = chrono::Local::now() - chrono::Duration::days(3);

    // Confirm created PDFS and moved the PDFs in the approved preview.
    assert!(!first.exists());
    assert!(f.home.join("Downloads/PDFS/a.pdf").exists());
    assert!(untouched.exists());
    assert_eq!(f.state.history.lock().unwrap().entries.len(), 1);

    // Second due run: PDFS already exists (with a.pdf inside it) and a new
    // PDF arrived. The folder is reused, only the new file moves, and the
    // file already in PDFS stays where it is.
    let second = f.write("Downloads/b.pdf");
    make_due();
    run_due_tasks(&f.state).unwrap();
    assert!(!second.exists());
    assert!(f.home.join("Downloads/PDFS/b.pdf").exists());
    assert!(f.home.join("Downloads/PDFS/a.pdf").exists(), "files already in PDFS stay put");
    assert!(!f.home.join("Downloads/PDFS 2").exists(), "no duplicate folder is created");
    assert!(untouched.exists());
    assert_eq!(f.state.history.lock().unwrap().entries.len(), 2);
}

/// Recurring folder-and-PDF chains must preview through the rule path.
/// When the dev model is enabled its cross-check must accept the rule
/// intent when both paths read the same command; otherwise `plan_input`
/// refuses with "Repeating tasks need a command Voxi understands without
/// the model." even though the rule parser understood it. With no model
/// environment variables the rule path is exercised on its own.
#[test]
fn rule_only_recurring_folder_and_pdf_chain_variants_preview() {
    for text in [
        "create a folder in downloads called PDFS and move pdfs into it everyday",
        "create a folder in downloads called PDFS and move pdfs into it every day",
        "create a folder in downloads called PDFS and move pdfs into it daily",
        "create a folder in downloads named PDFS and move pdfs into it everyday",
        "create a folder in downloads called PDFS and move pdf into it every 2 days",
        "make a folder called PDFS in downloads and move all pdfs into it every day at 9 am",
    ] {
        let f = Fixture::new();
        f.write("Downloads/a.pdf");
        let outcome = f.command(text);
        let Outcome::Plan { total, repeats, .. } = &outcome else { panic!("{text}: {outcome:?}") };
        assert_eq!(*total, 2, "{text}");
        assert!(repeats.is_some(), "{text}");
    }
}

/// Pure routing decisions, checked without loading any model weights.
#[cfg(feature = "tiny-model")]
#[test]
fn recurring_route_sends_rule_understood_jobs_to_the_rule_path() {
    // A recurring command the rule parser understood uses the rule path,
    // whether or not the model also saw a repeat rule. No intent equality
    // comparison is involved.
    assert_eq!(recurring_route(true, true, false), Route::Rule);
    assert_eq!(recurring_route(true, true, true), Route::Rule);
    assert_eq!(recurring_route(false, true, true), Route::Rule);
}

#[cfg(feature = "tiny-model")]
#[test]
fn recurring_route_refuses_model_only_recurring_jobs() {
    // Recurring with no rule interpretation is refused, regardless of
    // which path detected the repeat rule.
    assert_eq!(recurring_route(false, false, true), Route::Refuse);
    assert_eq!(recurring_route(true, false, false), Route::Refuse);
    assert_eq!(recurring_route(true, false, true), Route::Refuse);
}

#[cfg(feature = "tiny-model")]
#[test]
fn recurring_route_leaves_non_recurring_planning_unchanged() {
    // Non-recurring commands still take the model plan; the caller keeps
    // applying the schedule cross-check to it.
    assert_eq!(recurring_route(false, false, false), Route::Model);
    assert_eq!(recurring_route(false, true, false), Route::Model);
}

#[test]
fn chain_schedules_apply_to_the_whole_chain_and_keep_literal_names() {
    let f = Fixture::new();
    f.write("Downloads/a.pdf");
    for suffix in ["everyday", "every day", "daily", "everyday at 9 pm", "after 3 hours"] {
        let outcome = f.command(&format!("create a folder in Downloads called PDFS and move PDFs into it {suffix}"));
        let Outcome::Plan { scheduled_for: Some(_), repeats, total, .. } = outcome else { panic!("{suffix}: {outcome:?}") };
        assert_eq!(total, 2, "{suffix}");
        assert_eq!(repeats.is_some(), suffix != "after 3 hours");
        let pending = f.state.pending.lock().unwrap();
        let job = pending.as_ref().unwrap().scheduled.as_ref().unwrap();
        assert_eq!(job.repeat.is_some(), suffix != "after 3 hours");
        assert!(!f.home.join("Downloads/PDFS").exists());
    }
    assert!(matches!(f.command("rename a.pdf in Downloads to Everyday"), Outcome::Problem { .. }));
    assert!(matches!(f.command("rename a.pdf in Downloads to \"everyday\""), Outcome::Plan { scheduled_for: None, .. }));
    let outcome = f.command("create a folder in Downloads called PDFS after 3 hours and move PDFs into it everyday");
    assert!(!matches!(outcome, Outcome::Plan { .. }), "a time on an individual step must not disappear");
    assert!(f.state.pending.lock().unwrap().is_none());
}

#[test]
fn recurring_confirm_with_no_completed_steps_still_saves_repeat() {
    let f = Fixture::new();
    let source = f.write("Downloads/a.pdf");
    let Outcome::Plan { id, .. } = f.command("move PDFs from Downloads to Documents every day") else { panic!("expected preview") };
    fs::remove_file(source).unwrap();
    let result = confirm_input(&f.state, id).unwrap();
    assert_eq!(result.done, 0);
    assert!(result.error.is_some());
    assert!(!result.undoable);
    assert!(f.state.history.lock().unwrap().entries.is_empty());
    assert_eq!(scheduler::Store::load(f.home.join("schedules.json")).jobs[0].series, scheduler::SeriesState::Active);
}

#[test]
fn recurring_confirm_uses_only_approved_files_and_retains_partial_failure() {
    for fail_first in [false, true] {
        let f = Fixture::new();
        let source = f.write("Downloads/a.pdf");
        let Outcome::Plan { id, .. } = f.command("create a folder in downloads called PDFS and move pdfs into it everyday") else {
            panic!("expected preview")
        };
        let arrived_after_preview = f.write("Downloads/new.pdf");
        if fail_first {
            fs::remove_file(&source).unwrap();
        }
        let result = confirm_input(&f.state, id).unwrap();
        assert_eq!(result.done, if fail_first { 1 } else { 2 });
        assert_eq!(result.error.is_some(), fail_first);
        assert!(result.undoable);
        assert!(arrived_after_preview.exists(), "confirm must execute exactly the approved preview");
        let history = History::load(f.home.join("history.json"));
        assert_eq!(history.entries.len(), 1);
        assert_eq!(history.entries[0].done.len(), result.done);
        let store = scheduler::Store::load(f.home.join("schedules.json"));
        assert_eq!(store.jobs.len(), 1);
        assert_eq!(store.jobs[0].series, scheduler::SeriesState::Active);
        assert!(store.jobs[0].due > chrono::Local::now());
        let Outcome::Plan { id, is_undo: true, .. } = f.command("undo last") else { panic!("expected undo") };
        assert!(confirm_input(&f.state, id).unwrap().error.is_none());
        assert!(!f.home.join("Downloads/PDFS").exists());
        assert_eq!(source.exists(), !fail_first);
        f.state.schedules.lock().unwrap().jobs[0].due = chrono::Local::now() - chrono::Duration::days(2);
        run_due_tasks(&f.state).unwrap();
        assert!(!arrived_after_preview.exists());
        assert!(f.home.join("Downloads/PDFS/new.pdf").exists());
        assert_eq!(f.state.history.lock().unwrap().entries.len(), 3);
    }
}

#[test]
fn preview_policy_and_persisted_first_run_agree() {
    use scheduler::FirstRunPolicy::*;
    let f = Fixture::new();
    f.write("Downloads/a.pdf");
    for (suffix, policy, wire_policy) in [
        ("", Immediate, "immediate"),
        (" in 20 minutes", OneTimeDeferred, "oneTimeDeferred"),
        (" every day at 9 pm", RepeatLater, "repeatLater"),
        (" every day", RunNowAndRepeat, "runNowAndRepeat"),
    ] {
        let outcome = f.command(&format!("copy PDFs from Downloads to Documents{suffix}"));
        let Outcome::Plan { first_run, .. } = outcome else { panic!("{outcome:?}") };
        assert_eq!(first_run, policy);
        assert_eq!(serde_json::to_value(outcome).unwrap()["firstRun"], wire_policy);
        let pending = f.state.pending.lock().unwrap();
        let job = pending.as_ref().unwrap().scheduled.as_ref();
        assert_eq!(job.map_or(Immediate, |job| job.first_run), policy);
    }
}

#[test]
fn repeating_shortcut_checks_the_approved_snapshot_before_each_run() {
    for removed in [false, true] {
        let f = Fixture::new();
        let first = f.write("Downloads/first.pdf");
        let phrase = "run invoice filing";
        let commands = vec!["move PDFs from Downloads to Documents".into()];
        f.state.settings.lock().unwrap().shortcuts.push(settings::Shortcut { phrase: phrase.into(), commands: commands.clone() });
        let outcome = f.command(&format!("{phrase} every day at 9 pm"));
        let Outcome::Plan { id, first_run: scheduler::FirstRunPolicy::RepeatLater, .. } = outcome else { panic!("{outcome:?}") };
        assert_eq!(confirm_input(&f.state, id).unwrap().done, 0);
        let path = f.home.join("schedules.json");
        let store = scheduler::Store::load(path.clone());
        assert_eq!(
            store.jobs[0].repeat.as_ref().unwrap().approved_source,
            Some(scheduler::RuleSource::Shortcut { phrase: phrase.into(), commands })
        );

        let make_due = || f.state.schedules.lock().unwrap().jobs[0].due = chrono::Local::now() - chrono::Duration::minutes(2);
        make_due();
        run_due_tasks(&f.state).unwrap();
        assert!(!first.exists());
        assert!(f.home.join("Documents/first.pdf").exists());

        let second = f.write("Downloads/second.pdf");
        let mut settings = f.state.settings.lock().unwrap();
        if removed {
            settings.shortcuts.clear();
        } else {
            settings.shortcuts[0].commands = vec!["move PDFs from Downloads to Desktop".into()];
        }
        drop(settings);
        make_due();
        run_due_tasks(&f.state).unwrap();
        assert!(second.exists());
        assert!(!f.home.join("Desktop/second.pdf").exists());
        let mut store = scheduler::Store::load(path);
        assert_eq!(store.jobs[0].series, scheduler::SeriesState::NeedsReview);
        assert!(store.jobs[0].summary.contains("changed or removed"));
        assert!(store.claim(chrono::Local::now()).unwrap().is_none());
        assert!(store.resume(1, chrono::Local::now()).is_err());
    }
}

#[test]
fn repeating_with_explicit_clock_does_not_run_on_confirm() {
    for suffix in ["every day at 9 pm", "every Monday at 8:30 am"] {
        let f = Fixture::new();
        let source = f.write("Downloads/a.pdf");
        let outcome = f.command(&format!("create a folder in downloads called PDFS and move pdfs into it {suffix}"));
        let Outcome::Plan { id, repeats: Some(repeats), .. } = outcome else { panic!("{outcome:?}") };
        assert!(!repeats.starts_with("Runs now"));
        let result = confirm_input(&f.state, id).unwrap();
        assert_eq!(result.done, 0);
        assert!(!result.undoable);
        assert!(result.error.is_none());
        assert!(source.exists());
        assert!(!f.home.join("Downloads/PDFS").exists());
        assert!(f.state.history.lock().unwrap().entries.is_empty());
        assert!(f.state.schedules.lock().unwrap().jobs[0].due > chrono::Local::now());
    }
}

#[test]
fn repeating_shortcut_skips_the_whole_run_after_an_incomplete_search() {
    let f = Fixture::new();
    let notes = f.write("Downloads/notes.txt");
    let phrase = "run document cleanup";
    let commands = vec![
        "move notes.txt from Downloads to Pictures".into(),
        "copy PDFs from Documents to Desktop including subfolders".into(),
        "delete PDFs in Documents".into(),
    ];
    f.state.settings.lock().unwrap().shortcuts.push(settings::Shortcut { phrase: phrase.into(), commands });
    let outcome = f.command(&format!("{phrase} every day at 9 pm"));
    let Outcome::Plan { id, first_run: scheduler::FirstRunPolicy::RepeatLater, .. } = outcome else { panic!("{outcome:?}") };
    assert_eq!(confirm_input(&f.state, id).unwrap().done, 0);

    let report = f.write("Documents/report.pdf");
    let deep = "nested/".repeat(search::MAX_DEPTH + 1);
    f.write(&format!("Documents/{deep}other.pdf"));
    f.state.schedules.lock().unwrap().jobs[0].due = chrono::Local::now() - chrono::Duration::minutes(2);
    let (result, events) = diagnostics::capture(|| run_due_tasks(&f.state));
    result.unwrap();

    assert!(notes.exists());
    assert!(report.exists());
    assert!(!f.home.join("Pictures/notes.txt").exists());
    assert!(!f.home.join("Desktop/report.pdf").exists());
    assert!(f.state.history.lock().unwrap().entries.is_empty());
    let store = scheduler::Store::load(f.home.join("schedules.json"));
    let task = store.jobs[0].summary();
    assert_eq!(task.series, scheduler::SeriesState::Active);
    assert!(task.summary.contains("Skipped: Search incomplete") && task.summary.contains("Settings → Folders"), "{}", task.summary);
    assert!(events.iter().any(|line| line.contains("code=E_SEARCH_DEPTH ")), "{events:?}");
    assert!(events.iter().any(|line| line.contains("code=E_SCHED_SKIP ")), "{events:?}");
    assert!(!events.iter().any(|line| line.contains("code=E_PLANNING_FAILED ")), "{events:?}");
}

#[test]
fn shortcut_planning_failures_block_previews_and_scheduled_runs() {
    for (first, excluded, reason) in [
        ("copy PDFs from Documents to Desktop/Backup", true, "is outside the folders Voxi may change"),
        ("create a folder in Desktop named Backup", false, "A file named “Backup” already exists in Desktop"),
    ] {
        let f = Fixture::new();
        let phrase = "run backup and cleanup";
        let commands = vec![first.into(), "delete PDFs in Documents".into(), "create a folder in Pictures named Completed".into()];
        f.state.settings.lock().unwrap().shortcuts.push(settings::Shortcut { phrase: phrase.into(), commands });
        let outcome = f.command(&format!("{phrase} every day at 9 pm"));
        let Outcome::Plan { id, first_run: scheduler::FirstRunPolicy::RepeatLater, .. } = outcome else { panic!("{first}: {outcome:?}") };
        assert_eq!(confirm_input(&f.state, id).unwrap().done, 0);

        let report = f.write("Documents/report.pdf");
        let blocked = f.home.join("Desktop/Backup");
        if excluded {
            fs::create_dir(&blocked).unwrap();
            let mut roots = f.state.roots.write().unwrap();
            *roots = roots.clone().with_exclusions(std::slice::from_ref(&blocked));
        } else {
            fs::write(&blocked, b"existing file").unwrap();
        }

        let preview = f.command(phrase);
        assert!(matches!(&preview, Outcome::Problem { message } if message.contains(reason)), "{first}: {preview:?}");
        assert!(f.state.pending.lock().unwrap().is_none());

        f.state.schedules.lock().unwrap().jobs[0].due = chrono::Local::now() - chrono::Duration::minutes(2);
        let (result, events) = diagnostics::capture(|| run_due_tasks(&f.state));
        result.unwrap();
        assert!(report.exists(), "{first}");
        assert!(!blocked.join("report.pdf").exists(), "{first}");
        assert!(!f.home.join("Pictures/Completed").exists(), "{first}");
        assert!(f.state.history.lock().unwrap().entries.is_empty());
        let store = scheduler::Store::load(f.home.join("schedules.json"));
        let task = store.jobs[0].summary();
        assert_eq!(task.series, scheduler::SeriesState::Active);
        assert!(task.summary.contains(": Skipped: ") && task.summary.contains(reason), "{first}: {}", task.summary);
        assert!(events.iter().any(|line| line.contains("code=E_PLANNING_FAILED ")), "{events:?}");
        assert!(events.iter().any(|line| line.contains("code=E_SCHED_SKIP ")), "{events:?}");
    }
}

#[test]
fn recurring_chain_distinguishes_an_idle_start_from_an_unfinished_plan() {
    for (removed, summary, planning_failed) in [
        ("Documents/report.pdf", "Nothing to change. No files match in Documents.", false),
        ("Pictures/old.pdf", "Skipped: No files match in Pictures.", true),
    ] {
        let f = Fixture::new();
        let report = f.write("Documents/report.pdf");
        let old = f.write("Pictures/old.pdf");
        let outcome = f.command("copy PDFs from Documents to Desktop then delete PDFs in Pictures every day at 9 pm");
        let Outcome::Plan { id, first_run: scheduler::FirstRunPolicy::RepeatLater, total: 2, .. } = outcome else { panic!("{outcome:?}") };
        assert_eq!(confirm_input(&f.state, id).unwrap().done, 0);
        fs::remove_file(f.home.join(removed)).unwrap();

        f.state.schedules.lock().unwrap().jobs[0].due = chrono::Local::now() - chrono::Duration::minutes(2);
        let (result, events) = diagnostics::capture(|| run_due_tasks(&f.state));
        result.unwrap();

        let store = scheduler::Store::load(f.home.join("schedules.json"));
        let task = store.jobs[0].summary();
        assert_eq!(task.series, scheduler::SeriesState::Active);
        assert!(task.summary.ends_with(&format!(": {summary}")), "{}", task.summary);
        assert_eq!(events.iter().any(|line| line.contains("code=E_PLANNING_FAILED ")), planning_failed, "{events:?}");
        assert_eq!(report.exists(), planning_failed);
        assert_eq!(old.exists(), !planning_failed);
        assert!(!f.home.join("Desktop/report.pdf").exists());
        assert!(f.state.history.lock().unwrap().entries.is_empty());
    }
}

#[test]
fn recurring_shortcut_reports_an_idle_run_without_repeating_the_prefix() {
    let phrase = "run document backup";
    let copy = "copy PDFs from Documents to Desktop";
    let chain = "copy PDFs from Documents to Desktop then delete PDFs in Pictures";
    for (command, later_command, idle_in_pictures) in
        [(copy, None, false), (chain, None, false), (chain, Some("create folder Completed in Desktop"), false), (chain, None, true)]
    {
        let f = Fixture::new();
        let report = f.write("Documents/report.pdf");
        let old = f.write("Pictures/old.pdf");
        let commands = std::iter::once(command).chain(later_command).map(String::from).collect();
        f.state.settings.lock().unwrap().shortcuts.push(settings::Shortcut { phrase: phrase.into(), commands });
        let outcome = f.command(&format!("{phrase} every day at 9 pm"));
        let Outcome::Plan { id, .. } = outcome else { panic!("{outcome:?}") };
        assert_eq!(confirm_input(&f.state, id).unwrap().done, 0);
        fs::remove_file(if idle_in_pictures { &old } else { &report }).unwrap();

        f.state.schedules.lock().unwrap().jobs[0].due = chrono::Local::now() - chrono::Duration::minutes(2);
        let (result, events) = diagnostics::capture(|| run_due_tasks(&f.state));
        result.unwrap();

        let store = scheduler::Store::load(f.home.join("schedules.json"));
        let task = store.jobs[0].summary();
        assert_eq!(task.series, scheduler::SeriesState::Active);
        let summary = if idle_in_pictures {
            ": Skipped: No files match in Pictures.".into()
        } else if later_command.is_some() {
            ": Skipped: No files match in Documents.".into()
        } else {
            format!(": Nothing to change for “{phrase}”. Skipped “{command}”: No files match in Documents.")
        };
        assert!(task.summary.ends_with(&summary), "{command}, {later_command:?}: {}", task.summary);
        assert_eq!(
            events.iter().any(|line| line.contains("code=E_PLANNING_FAILED ")),
            idle_in_pictures || later_command.is_some(),
            "{events:?}"
        );
        if idle_in_pictures {
            assert_eq!(fs::read(report).unwrap(), b"data");
        } else {
            assert_eq!(fs::read(old).unwrap(), b"data");
        }
        assert!(!f.home.join("Desktop/report.pdf").exists());
        assert!(!f.home.join("Desktop/Completed").exists());
        assert!(f.state.history.lock().unwrap().entries.is_empty());
    }
}

#[test]
fn planning_diagnostics_route_search_failures_ambiguity_and_cancellation() {
    let f = Fixture::new();
    let report = f.write("Documents/report.pdf");
    let deep = "nested/".repeat(search::MAX_DEPTH + 1);
    f.write(&format!("Documents/{deep}other.pdf"));
    let plan_and_capture =
        |text: &str, request_id| diagnostics::capture(|| plan_with_stats(&f.state, text.into(), HashMap::new(), request_id, None));
    let codes = |events: Vec<String>| {
        events
            .iter()
            .map(|line| line.split_whitespace().find_map(|word| word.strip_prefix("code=")).unwrap().to_owned())
            .collect::<Vec<_>>()
    };

    let ((outcome, stats), events) = plan_and_capture("delete PDFs in Downloads", 1);
    assert!(matches!(outcome, Outcome::Problem { .. }));
    assert_eq!(stats.too_deep, 0);
    assert_eq!(codes(events), ["E_PLANNING_FAILED"]);

    let ((outcome, stats), events) = plan_and_capture("delete PDFs in Documents including subfolders", 2);
    assert!(matches!(outcome, Outcome::Problem { message } if message.contains("Search incomplete")));
    assert_eq!(stats.too_deep, 1);
    assert_eq!(codes(events), ["E_SEARCH_DEPTH"]);

    let ((outcome, stats), events) = plan_and_capture("delete report.pdf from Documents", 3);
    assert!(matches!(outcome, Outcome::Ambiguous { candidates, reason: model::AmbiguityReason::IncompleteSearch, .. }
        if candidates == [report.to_string_lossy()]));
    assert_eq!(stats.too_deep, 1);
    assert_eq!(codes(events), ["E_SEARCH_DEPTH", "E_AMBIGUOUS"]);

    let search = search::Search::default();
    search.issue("Spotlight exited unsuccessfully".into());
    search.record_failure();
    let (_, events) = diagnostics::capture(|| emit_planning_failure(Code::Ambiguous, &search.control, stats));
    assert_eq!(codes(events), ["E_SEARCH_INCOMPLETE", "E_AMBIGUOUS"]);

    let control = f.state.active_search.lock().unwrap().as_ref().unwrap().1.clone();
    assert!(control.failed_with(search::Failure::Depth));
    control.cancel();
    let (_, events) = diagnostics::capture(|| emit_planning_failure(Code::PlanningFailed, &control, stats));
    assert!(events.is_empty(), "{events:?}");
}

#[test]
fn repeating_move_into_a_new_folder_makes_the_folder() {
    let f = Fixture::new();
    let pdf = f.write("Downloads/bill.pdf");
    let outcome = f.command("move every pdf in downloads to a new folder called PDFS every 2 days");
    let Outcome::Plan { id, repeats: Some(repeats), .. } = outcome else {
        panic!("{outcome:?}");
    };
    assert!(repeats.starts_with("Runs now, then every 2 days"), "{repeats}");
    confirm_input(&f.state, id).unwrap();
    f.state.schedules.lock().unwrap().jobs[0].due = chrono::Local::now() - chrono::Duration::days(3);
    run_due_tasks(&f.state).unwrap();
    assert!(!pdf.exists());
    assert!(f.home.join("Downloads/PDFS/bill.pdf").exists());
}
#[test]
fn repeating_task_replans_each_run_and_keeps_going() {
    let f = Fixture::new();
    let first = f.write("Downloads/a.dmg");
    let outcome = f.command("delete dmg files from downloads every 3 days");
    let Outcome::Plan { id, scheduled_for: Some(_), repeats: Some(repeats), .. } = outcome else {
        panic!("{outcome:?}");
    };
    assert!(repeats.starts_with("Runs now, then every 3 days"), "{repeats}");
    let result = confirm_input(&f.state, id).unwrap();
    assert!(result.summary.starts_with("Done. 1 change. Repeats every 3 days"), "{}", result.summary);
    assert!(!first.exists());
    let due = |f: &Fixture| f.state.schedules.lock().unwrap().jobs[0].due;
    let make_due = |f: &Fixture| f.state.schedules.lock().unwrap().jobs[0].due = chrono::Local::now() - chrono::Duration::days(7);
    // Confirm ran the file that existed in the preview.
    assert!(!first.exists());
    let job = f.state.schedules.lock().unwrap().jobs[0].clone();
    assert_eq!(job.series, scheduler::SeriesState::Active);
    assert!(job.due > chrono::Local::now() && job.due <= chrono::Local::now() + chrono::Duration::days(3), "missed runs collapse into one");
    assert!(job.summary.contains("Done. 1 change."), "{}", job.summary);
    // A file added later is picked up by the next run.
    let second = f.write("Downloads/b.dmg");
    make_due(&f);
    run_due_tasks(&f.state).unwrap();
    assert!(!second.exists());
    assert_eq!(f.state.history.lock().unwrap().entries.len(), 2);
    // Nothing left to match: the series keeps going.
    make_due(&f);
    run_due_tasks(&f.state).unwrap();
    assert_eq!(f.state.schedules.lock().unwrap().jobs[0].series, scheduler::SeriesState::Active);
    assert!(f.state.schedules.lock().unwrap().jobs[0].summary.ends_with(": Nothing to change. No files match in Downloads."));
    // Paused tasks don't run; resuming skips the missed runs.
    let third = f.write("Downloads/c.dmg");
    f.state.schedules.lock().unwrap().pause(1).unwrap();
    make_due(&f);
    run_due_tasks(&f.state).unwrap();
    assert!(third.exists());
    f.state.schedules.lock().unwrap().resume(1, chrono::Local::now()).unwrap();
    assert!(due(&f) > chrono::Local::now());
    f.state.schedules.lock().unwrap().cancel(1).unwrap();
    make_due(&f);
    run_due_tasks(&f.state).unwrap();
    assert!(third.exists());
    // A stopped repeating task can be made active again; a one-time task can't.
    f.state.schedules.lock().unwrap().resume(1, chrono::Local::now()).unwrap();
    assert_eq!(f.state.schedules.lock().unwrap().jobs[0].series, scheduler::SeriesState::Active);
    assert!(due(&f) > chrono::Local::now());
    let Outcome::Plan { id, .. } = f.command("move c.dmg from downloads to documents in 5 minutes") else {
        panic!("plan");
    };
    confirm_input(&f.state, id).unwrap();
    f.state.schedules.lock().unwrap().cancel(2).unwrap();
    assert!(f.state.schedules.lock().unwrap().resume(2, chrono::Local::now()).is_err());
    // Schedule vocabulary in names must be quoted to stay literal.
    f.write("Desktop/a.pdf");
    let outcome = f.command("rename a.pdf on desktop to Daily");
    assert!(matches!(outcome, Outcome::Problem { .. }), "{outcome:?}");
    let outcome = f.command("rename a.pdf on desktop to \"Daily\"");
    assert!(matches!(outcome, Outcome::Plan { scheduled_for: None, .. }), "{outcome:?}");
}
/// Jobs saved before recurring tasks became rule-only still carry a
/// `model_intent`. Such a plan was never previewed at run time, so it must
/// never be replayed unattended: re-plan from the rule text, or skip with
/// the usual notice when the rule parser can't read it.
#[test]
fn stored_recurring_model_intent_is_never_replayed_unattended() {
    let f = Fixture::new();
    let pdf = f.write("Downloads/a.pdf");
    let model_intent = || model::Intent::CreateFolders { names: vec!["MODEL_PLAN".into()], parent: Some("Desktop".into()) };
    let stale_rule = |command: &str| scheduler::Rule {
        every: 1,
        unit: scheduler::Unit::Days,
        time: None,
        command: command.into(),
        choices: HashMap::new(),
        model_intent: Some(model_intent()),
        approved_version: scheduler::APPROVED_RULE_VERSION,
        approved_source: Some(scheduler::RuleSource::Command),
        approved_nicknames: None,
    };
    // The rule text is still readable: it wins over the stored model plan.
    f.state
        .schedules
        .lock()
        .unwrap()
        .add(scheduler::Job::recurring_later(
            "create a folder in downloads called PDFS and move pdfs into it".into(),
            chrono::Local::now() - chrono::Duration::days(2),
            stale_rule("create a folder in downloads called PDFS and move pdfs into it"),
        ))
        .unwrap();
    run_due_tasks(&f.state).unwrap();
    assert!(!pdf.exists());
    assert!(f.home.join("Downloads/PDFS/a.pdf").exists());
    assert!(!f.home.join("Desktop/MODEL_PLAN").exists(), "the stored model plan must not run");
    assert_eq!(f.state.history.lock().unwrap().entries.len(), 1);

    // Unreadable rule text skips with a clear notice instead of falling
    // back to the stored model plan.
    f.state
        .schedules
        .lock()
        .unwrap()
        .add(scheduler::Job::recurring_later(
            "unparseable legacy task".into(),
            chrono::Local::now() - chrono::Duration::days(2),
            stale_rule("this is not a parseable command"),
        ))
        .unwrap();
    run_due_tasks(&f.state).unwrap();
    assert!(!f.home.join("Desktop/MODEL_PLAN").exists(), "a skipped job must not replay model intent");
    assert_eq!(f.state.history.lock().unwrap().entries.len(), 1);
    let jobs = f.state.schedules.lock().unwrap();
    assert!(jobs.jobs[1].summary.contains("Skipped: Voxi doesn't understand"), "{}", jobs.jobs[1].summary);
    assert_eq!(jobs.jobs[1].series, scheduler::SeriesState::Active);
}

#[test]
fn scheduled_changed_sources_and_collisions_stop_without_overwriting() {
    for replace_source in [true, false] {
        let f = Fixture::new();
        let source = f.write("Downloads/a.pdf");
        let Outcome::Plan { id, .. } = f.command("move a.pdf from Downloads to Documents in 5 minutes") else {
            panic!("plan");
        };
        confirm_input(&f.state, id).unwrap();
        if replace_source {
            std::fs::write(&source, "changed source contents").unwrap();
        } else {
            f.write("Documents/a.pdf");
        }
        f.state.schedules.lock().unwrap().jobs[0].due = chrono::Local::now() - chrono::Duration::seconds(1);
        run_due_tasks(&f.state).unwrap();
        assert!(source.exists());
        assert_eq!(f.state.schedules.lock().unwrap().jobs[0].series, scheduler::SeriesState::Failed);
        assert!(f.state.history.lock().unwrap().entries.is_empty());
    }
}
#[test]
fn scheduled_cancellation_and_read_only_commands() {
    let f = Fixture::new();
    let source = f.write("Downloads/a.pdf");
    let Outcome::Plan { id, .. } = f.command("move a.pdf from Downloads to Documents after 1 hour") else {
        panic!("plan");
    };
    confirm_input(&f.state, id).unwrap();
    f.state.schedules.lock().unwrap().cancel(1).unwrap();
    f.state.schedules.lock().unwrap().jobs[0].due = chrono::Local::now() - chrono::Duration::seconds(1);
    run_due_tasks(&f.state).unwrap();
    assert!(source.exists());
    assert!(matches!(f.command("list files in Downloads after 3 minutes"), Outcome::Problem { .. }));
    assert!(f.state.pending.lock().unwrap().is_none());
}
#[test]
fn last_file_is_the_last_displayed_result_and_no_global_scan_occurs() {
    let f = Fixture::new();
    f.write("Downloads/a.pdf");
    f.write("Downloads/sub/b.txt");
    f.write("Desktop/elsewhere.txt");
    f.list();
    let paths = f.paths();
    assert!(matches!(f.command("delete the last file"), Outcome::Plan { total: 1, .. }));
    assert_eq!(f.actions(), vec![Action::Trash { path: paths.last().unwrap().clone() }]);
    assert_eq!(f.state.active_search.lock().unwrap().as_ref().unwrap().1.visited.load(Ordering::Relaxed), 0);
    assert!(paths.iter().all(|p| p.exists()));
}
#[test]
fn all_and_filtered_followups_only_use_snapshot_files() {
    let f = Fixture::new();
    let pdf = f.write("Downloads/a.pdf");
    let txt = f.write("Downloads/sub/b.txt");
    f.write("Documents/outside.pdf");
    f.list();
    f.write("Downloads/new.pdf");
    assert!(matches!(f.command("delete PDFs"), Outcome::Plan { total: 1, .. }));
    assert_eq!(f.actions(), vec![Action::Trash { path: pdf.clone() }]);
    assert!(matches!(f.command("move those files to Documents"), Outcome::Plan { total: 2, .. }));
    let sources: Vec<_> =
        f.actions().iter().filter_map(|a| if let Action::Move { from, .. } = a { Some(from.clone()) } else { None }).collect();
    assert!(sources.contains(&pdf) && sources.contains(&txt));
}
#[test]
fn an_empty_scoped_filter_does_not_start_a_global_search() {
    let f = Fixture::new();
    f.write("Downloads/keep.txt");
    f.write("Pictures/photo.jpg");
    f.list();
    assert!(matches!(f.command("delete images"), Outcome::Problem { .. }));
    assert_eq!(f.paths().len(), 1);
    assert!(f.state.pending.lock().unwrap().is_none());
}
#[test]
fn explicit_source_folders_and_outside_filenames_leave_scope() {
    let f = Fixture::new();
    f.write("Downloads/a.pdf");
    let outside = f.write("Documents/outside.pdf");
    f.list();
    assert!(matches!(f.command("delete outside.pdf"), Outcome::Plan { total: 1, .. }));
    assert!(f.state.result_scope.lock().unwrap().current.is_none());
    assert_eq!(f.actions(), vec![Action::Trash { path: outside.clone() }]);
    f.list();
    assert!(matches!(f.command("delete PDFs in Documents"), Outcome::Plan { total: 1, .. }));
    assert_eq!(f.actions(), vec![Action::Trash { path: outside }]);
    assert!(f.state.result_scope.lock().unwrap().current.is_none());
}
#[test]
fn a_new_list_replaces_scope_and_filters_can_narrow_it() {
    let f = Fixture::new();
    f.write("Downloads/a.pdf");
    f.write("Downloads/b.txt");
    let image = f.write("Pictures/photo.jpg");
    f.list();
    assert!(matches!(f.command("list PDFs"), Outcome::Info { total: 1, .. }));
    assert_eq!(f.paths().len(), 1);
    assert!(matches!(f.command("list images in Pictures"), Outcome::Info { total: 1, .. }));
    assert_eq!(f.paths(), vec![image]);
}
#[test]
fn missing_or_replaced_files_require_a_new_list() {
    let f = Fixture::new();
    let path = f.write("Downloads/a.pdf");
    f.list();
    fs::rename(&path, f.home.join("Downloads/original.pdf")).unwrap();
    fs::write(&path, b"replacement").unwrap();
    assert!(matches!(f.command("delete them"), Outcome::Problem{message} if message.contains("changed")));
    assert!(path.exists());
    assert!(f.state.pending.lock().unwrap().is_none());
}
#[test]
fn files_replaced_after_the_preview_cannot_be_confirmed() {
    let f = Fixture::new();
    let path = f.write("Downloads/a.pdf");
    f.list();
    assert!(matches!(f.command("delete them"), Outcome::Plan { .. }));
    let pending = f.state.pending.lock().unwrap().take().unwrap();
    pending.validate_context(&f.state.result_scope.lock().unwrap()).unwrap();
    fs::rename(&path, f.home.join("Downloads/original.pdf")).unwrap();
    fs::write(&path, b"replacement").unwrap();
    assert!(pending.validate_context(&f.state.result_scope.lock().unwrap()).unwrap_err().contains("changed"));
    assert_eq!(fs::read(&path).unwrap(), b"replacement");
}
#[test]
fn clearing_scope_expires_its_preview_and_references() {
    let f = Fixture::new();
    f.write("Downloads/a.pdf");
    f.list();
    assert!(matches!(f.command("delete them"), Outcome::Plan { .. }));
    let pending = f.state.pending.lock().unwrap().take().unwrap();
    f.state.result_scope.lock().unwrap().clear();
    assert!(pending.validate_context(&f.state.result_scope.lock().unwrap()).unwrap_err().contains("expired"));
    assert!(matches!(f.command("delete the last file"), Outcome::NotUnderstood { .. }));
}
#[test]
fn empty_lists_stay_bounded_and_destinations_are_never_references() {
    let f = Fixture::new();
    f.write("Documents/outside.pdf");
    f.list();
    assert!(matches!(f.command("delete everything"), Outcome::Problem { .. }));
    assert!(f.state.pending.lock().unwrap().is_none());
    f.write("Downloads/a.pdf");
    f.list();
    let snapshot = f.state.result_scope.lock().unwrap().current.clone().unwrap();
    let (intent, used) = scope::prepare("rename the first file to \"last\"", Some(&snapshot)).unwrap();
    assert!(used);
    assert!(matches!(intent,model::Intent::Rename{new_name,..} if new_name=="last"));
    assert!(scope::parse_followup("move a.pdf to last", &snapshot).unwrap().is_none());
    let (intent, used) = scope::prepare("move a.pdf to \"last\"", Some(&snapshot)).unwrap();
    assert!(used);
    assert!(matches!(intent,model::Intent::Move{to,..} if to=="last"));
}
#[test]
fn ambiguous_names_only_offer_current_results_and_reject_outside_choices() {
    let f = Fixture::new();
    f.write("Downloads/a/same.pdf");
    f.write("Downloads/b/same.pdf");
    let outside = f.write("Documents/same.pdf");
    f.list();
    assert!(
        matches!(f.command("delete same.pdf"),Outcome::Ambiguous{candidates,..} if candidates.len()==2 && !candidates.contains(&outside.to_string_lossy().to_string()))
    );
    f.request.set(f.request.get() + 1);
    let result = plan_input(
        &f.state,
        "delete same.pdf".into(),
        HashMap::from([("same.pdf".into(), outside.to_string_lossy().into_owned())]),
        f.request.get(),
    );
    assert!(matches!(result, Outcome::Problem { .. }));
}
#[test]
fn positions_are_limited_to_displayed_rows() {
    let f = Fixture::new();
    let paths: Vec<_> = (0..201).map(|i| f.write(&format!("Downloads/{i}.txt"))).collect();
    let snapshot = scope::Snapshot::capture(paths.clone(), "Downloads".into()).unwrap();
    let (intent, used) = scope::prepare("delete the last file", Some(&snapshot)).unwrap();
    assert!(used);
    assert_eq!(intent.source(), Some(&model::Source::item(&paths[199].to_string_lossy())));
    assert!(scope::prepare("delete the 201st file", Some(&snapshot)).is_err());
    assert!(scope::prepare("delete 201", Some(&snapshot)).is_err());
}
#[test]
fn numbered_rows_and_spoken_numbers_target_the_displayed_order() {
    let f = Fixture::new();
    for name in ["a.pdf", "b.txt", "c.txt", "d.pdf", "e.txt", "4"] {
        f.write(&format!("Downloads/{name}"));
    }
    let Outcome::Info { rows, numbered: true, .. } = f.command("show items in Downloads") else { panic!("expected numbered list") };
    let paths = f.paths();
    assert_eq!(rows.iter().map(|r| r.name.clone()).collect::<Vec<_>>(), paths.iter().map(|p| file_name(p)).collect::<Vec<_>>());
    for command in ["delete 4", "delete four", "delete item 4", "delete number four"] {
        assert!(matches!(f.command(command), Outcome::Plan { total: 1, .. }), "{command}");
        assert_eq!(f.actions(), vec![Action::Trash { path: paths[3].clone() }], "{command}");
    }
    assert!(matches!(f.command("Move 5 to desktop"), Outcome::Plan { total: 1, .. }));
    assert_eq!(f.actions(), vec![Action::Move { from: paths[4].clone(), to: f.home.join("Desktop").join(file_name(&paths[4])) }]);
    assert!(matches!(f.command("delete 1"), Outcome::Plan { total: 1, .. }));
    assert_eq!(f.actions(), vec![Action::Trash { path: paths[0].clone() }]);
    assert!(matches!(f.command("delete \"4\""), Outcome::Plan { total: 1, .. }));
    assert_eq!(f.actions(), vec![Action::Trash { path: f.home.join("Downloads/4") }]);
    for command in ["delete 0", "delete 7", "delete 999999999999999999999999999"] {
        assert!(matches!(f.command(command), Outcome::Problem { .. }), "{command}");
        assert!(f.state.pending.lock().unwrap().is_none());
    }
    assert!(matches!(f.command("count files"), Outcome::Info { numbered: false, .. }));
}
#[test]
fn numeric_destinations_and_filenames_are_not_row_references() {
    let f = Fixture::new();
    f.write("Downloads/4.pdf");
    f.write("Downloads/a.txt");
    f.list();
    let snapshot = f.state.result_scope.lock().unwrap().current.clone().unwrap();
    for command in ["move a.txt to 5", "rename a.txt to 4.pdf", "resize images to 400", "delete 4.pdf"] {
        assert!(scope::parse_followup(command, &snapshot).unwrap().is_none(), "{command}");
    }
    let (intent, used) = scope::prepare("rename 1 to 5.pdf", Some(&snapshot)).unwrap();
    assert!(used);
    assert!(matches!(intent,model::Intent::Rename{new_name,..} if new_name=="5.pdf"));
}
#[test]
fn deleted_rows_do_not_shift_later_numbers_or_promote_hidden_rows() {
    let f = Fixture::new();
    let paths: Vec<_> = (0..201).map(|i| f.write(&format!("Downloads/{i}.txt"))).collect();
    let mut snapshot = scope::Snapshot::capture(paths.clone(), "Downloads".into()).unwrap();
    fs::remove_file(&paths[3]).unwrap();
    snapshot.apply(&[Action::Trash { path: paths[3].clone() }]);
    snapshot.validate().unwrap();
    assert!(scope::prepare("delete 4", Some(&snapshot)).is_err());
    let (intent, _) = scope::prepare("move 5 to Desktop", Some(&snapshot)).unwrap();
    assert_eq!(intent.source(), Some(&model::Source::item(&paths[4].to_string_lossy())));
    assert!(scope::prepare("delete 201", Some(&snapshot)).is_err());
    assert_eq!(snapshot.summary().count, 200);
    assert_eq!(snapshot.summary().visible, 199);
    let (intent, _) = scope::prepare("delete the last file", Some(&snapshot)).unwrap();
    assert_eq!(intent.source(), Some(&model::Source::item(&paths[199].to_string_lossy())));
}
#[test]
fn references_require_context_and_quoted_names_stay_literal() {
    let f = Fixture::new();
    f.write("Downloads/last");
    f.write("Downloads/notes.txt");
    assert!(matches!(f.command("delete the last file"), Outcome::NotUnderstood { .. }));
    f.list();
    assert!(matches!(f.command("delete \"last\""), Outcome::Plan { total: 1, .. }));
    assert!(matches!(f.command("delete it"), Outcome::Problem { .. }));
}
#[test]
fn confirmed_moves_and_trash_update_the_context_order() {
    let f = Fixture::new();
    let old = f.write("Downloads/a.pdf");
    let keep = f.write("Downloads/b.txt");
    f.list();
    let moved = f.home.join("Documents/a.pdf");
    fs::rename(&old, &moved).unwrap();
    let mut context = f.state.result_scope.lock().unwrap();
    let snapshot = context.current.as_mut().unwrap();
    snapshot.apply(&[Action::Move { from: old, to: moved.clone() }]);
    snapshot.validate().unwrap();
    assert!(snapshot.paths().contains(&moved));
    fs::remove_file(&keep).unwrap();
    snapshot.apply(&[Action::Trash { path: keep }]);
    assert_eq!(snapshot.paths(), vec![moved]);
}

#[test]
fn unscoped_search_says_which_folders_it_searched_and_skips_the_others() {
    let f = Fixture::new();
    f.write("Downloads/here.pdf");
    f.write("Documents/there.pdf");
    let Outcome::Info { summary, rows, .. } = f.command("find all PDFs") else { panic!("expected an info answer") };
    assert!(summary.contains("Searched only Downloads and Desktop"), "{summary}");
    assert!(summary.contains("in Documents"), "{summary}");
    assert!(rows.iter().any(|r| r.name == "here.pdf"));
    assert!(!rows.iter().any(|r| r.name == "there.pdf"));
    let Outcome::Info { summary, .. } = f.command("how many PDFs are there?") else { panic!("expected a count") };
    assert!(summary.contains("Searched only Downloads and Desktop"), "{summary}");
    // An explicit folder is a different, narrower search and needs no note.
    let Outcome::Info { summary, rows, .. } = f.command("find all PDFs in Documents") else { panic!("expected an info answer") };
    assert!(!summary.contains("Searched only Downloads and Desktop"), "{summary}");
    assert!(rows.iter().any(|r| r.name == "there.pdf"));
}

#[test]
fn sorting_is_top_level_unless_subfolders_are_requested() {
    let f = Fixture::new();
    let nested = f.write("Downloads/Project/a.pdf");
    let top = f.write("Downloads/b.pdf");
    let Outcome::Plan { .. } = f.command("sort Downloads by type") else { panic!("expected a plan") };
    let moves: Vec<_> = f.actions();
    assert!(moves.iter().any(|a| matches!(a, Action::Move { from, .. } if from == &top)), "{moves:?}");
    assert!(!moves.iter().any(|a| matches!(a, Action::Move { from, .. } if from == &nested)), "{moves:?}");
    let Outcome::Plan { .. } = f.command("sort Downloads by type including subfolders") else { panic!("expected a plan") };
    assert!(f.actions().iter().any(|a| matches!(a, Action::Move { from, .. } if from == &nested)));
}

#[test]
fn bulk_rename_plans_sequential_names_with_extensions() {
    let f = Fixture::new();
    f.write("Downloads/Screenshot one.png");
    f.write("Downloads/Screenshot two.png");
    let Outcome::Plan { .. } = f.command("rename all screenshots to holiday") else { panic!("expected a plan") };
    let names: Vec<String> = f
        .actions()
        .iter()
        .filter_map(|a| match a {
            Action::Rename { new_name, .. } => Some(new_name.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(names.len(), 2, "{names:?}");
    assert!(names.contains(&"holiday-001.png".into()), "{names:?}");
    assert!(names.contains(&"holiday-002.png".into()), "{names:?}");
}

#[test]
fn info_about_a_file_answers_with_its_facts() {
    let f = Fixture::new();
    f.write("Downloads/report.pdf");
    for command in ["info about report.pdf", "when was report.pdf modified", "details of report.pdf"] {
        let Outcome::Info { rows, total, .. } = f.command(command) else { panic!("{command}") };
        assert_eq!(total, 1, "{command}");
        assert!(rows.iter().any(|r| r.name == "report.pdf" && r.detail.contains("~/Downloads")), "{command}: {rows:?}");
    }
}
#[test]
fn unticked_steps_are_left_out_and_undo_covers_only_what_ran() {
    let f = Fixture::new();
    let a = f.write("Downloads/a.pdf");
    let b = f.write("Downloads/b.pdf");
    let c = f.write("Downloads/c.pdf");
    let Outcome::Plan { id, items, .. } = f.command("move PDFs from Downloads to a new folder called Filed in Documents") else {
        panic!("expected a plan")
    };
    let steps = f.actions();
    let at = |path: &PathBuf| steps.iter().position(|s| matches!(s, Action::Move { from, .. } if from == path)).unwrap();
    assert!(items[at(&b)].skippable && !items[0].skippable, "the folder creation stays: {items:?}");
    // A step that isn't skippable is refused, and the preview stays to fix.
    assert!(confirm_trimmed(&f.state, id, &[0]).unwrap_err().contains("can't be left out"));
    let result = confirm_trimmed(&f.state, id, &[at(&b)]).unwrap();
    assert!(result.error.is_none(), "{result:?}");
    assert_eq!(result.total, steps.len() - 1);
    let filed = f.home.join("Documents/Filed");
    assert!(filed.join("a.pdf").exists() && filed.join("c.pdf").exists() && b.exists());
    assert!(!a.exists() && !c.exists());
    let Outcome::Plan { id, .. } = f.command("undo") else { panic!("expected an undo plan") };
    assert!(confirm_trimmed(&f.state, id, &[0]).unwrap_err().contains("undo"));
    assert!(confirm_input(&f.state, id).unwrap().error.is_none());
    assert!(a.exists() && b.exists() && c.exists() && !filed.exists());
}
#[test]
fn a_finder_selection_becomes_the_list_that_these_refers_to() {
    let f = Fixture::new();
    let a = f.write("Downloads/a.pdf");
    let b = f.write("Downloads/b.pdf");
    let c = f.write("Downloads/c.pdf");
    let outside = tempfile::tempdir().unwrap();
    let stray = outside.path().join("x.pdf");
    fs::write(&stray, b"x").unwrap();
    let notice = take_selection(&f.state, vec![a.clone(), c.clone(), stray.clone()]).1.unwrap();
    assert!(notice.contains("1 item outside the folders") && notice.contains("“x.pdf”"), "{notice}");
    let summary = f.state.result_scope.lock().unwrap().summary().unwrap();
    assert_eq!((summary.count, summary.label.as_str()), (2, "your Finder selection"));
    let Outcome::Plan { id, total, .. } = f.command("move these to Documents") else { panic!("expected a plan") };
    assert_eq!(total, 2);
    assert!(confirm_input(&f.state, id).unwrap().error.is_none());
    assert!(f.home.join("Documents/a.pdf").exists() && f.home.join("Documents/c.pdf").exists());
    assert!(b.exists() && stray.exists());
    // Nothing usable clears the list rather than leaving an older one in place.
    assert!(take_selection(&f.state, vec![stray]).1.unwrap().contains("outside the folders"));
    assert!(f.state.result_scope.lock().unwrap().current.is_none());
}
#[test]
fn repeats_saved_before_modified_meant_last_edit_are_skipped_not_reread() {
    let f = Fixture::new();
    let a = f.write("Downloads/a.pdf");
    let rule = |modified_dates: bool| scheduler::Rule {
        every: 1,
        unit: scheduler::Unit::Days,
        time: None,
        command: "trash files in Downloads modified today".into(),
        choices: HashMap::new(),
        model_intent: None,
        approved_version: if modified_dates { scheduler::APPROVED_RULE_VERSION } else { 0 },
        approved_source: Some(scheduler::RuleSource::Command),
        approved_nicknames: None,
    };
    let job = |modified_dates| {
        scheduler::Job::recurring_later(
            "trash files in Downloads modified today daily".into(),
            chrono::Local::now() - chrono::Duration::minutes(5),
            rule(modified_dates),
        )
    };
    // A saved rule's JSON from before the field existed reads as an old rule.
    let legacy: scheduler::Rule =
        serde_json::from_value(serde_json::json!({"every": 1, "unit": "days", "time": null, "command": "x"})).unwrap();
    assert_eq!(legacy.approved_version, 0);

    f.state.schedules.lock().unwrap().add(job(false)).unwrap();
    run_due_tasks(&f.state).unwrap();
    assert!(a.exists(), "an old rule must not run with the new meaning");
    let summary = f.state.schedules.lock().unwrap().jobs[0].summary.clone();
    assert!(summary.contains("Review “trash files in Downloads modified today daily” to keep this task"), "{summary}");

    f.state.schedules.lock().unwrap().add(job(true)).unwrap();
    run_due_tasks(&f.state).unwrap();
    assert!(!a.exists(), "a rule saved now reads the last-edited date and runs");
}
#[test]
fn a_file_edited_after_the_preview_stops_the_run() {
    let f = Fixture::new();
    let old = f.write("Downloads/old.pdf");
    let long_ago = std::time::SystemTime::now() - std::time::Duration::from_secs(400 * 86_400);
    fs::File::options().write(true).open(&old).unwrap().set_modified(long_ago).unwrap();
    let Outcome::Plan { id, total: 1, .. } = f.command("trash files in Downloads not modified in 90 days") else {
        panic!("expected a plan")
    };
    fs::write(&old, b"edited").unwrap();
    let err = confirm_input(&f.state, id).unwrap_err();
    assert!(err.contains("“old.pdf” changed after the preview"), "{err}");
    assert!(old.exists());
    // The preview stays, so confirming it again keeps refusing until it's planned again.
    assert!(confirm_input(&f.state, id).is_err());
    assert!(matches!(f.command("trash files in Downloads not modified in 90 days"), Outcome::Problem { .. }));
}
#[test]
fn a_closing_view_only_clears_the_list_it_knew_and_notices_follow_their_list() {
    let f = Fixture::new();
    let a = f.write("Downloads/a.pdf");
    let outside = tempfile::tempdir().unwrap();
    let stray = outside.path().join("x.pdf");
    fs::write(&stray, b"x").unwrap();
    let old_view = opening(&f.state).generation;
    let (generation, _) = take_selection(&f.state, vec![a.clone(), stray]);
    *f.state.selection_notice.lock().unwrap() = Some((Some(generation), "left out".into()));
    // The old view closing after the selection arrived leaves it alone.
    clear_scope(&f.state, Some(old_view));
    let first = opening(&f.state);
    let again = opening(&f.state);
    assert_eq!((first.scope.unwrap().count, first.notice.as_deref()), (1, Some("left out")));
    assert_eq!(again.notice.as_deref(), Some("left out"), "reading twice gives the same notice");
    // Once the list is cleared, its notice no longer shows.
    f.state.result_scope.lock().unwrap().clear();
    assert!(opening(&f.state).notice.is_none());
    // A notice sent before setup shows once, whatever the list.
    *f.state.selection_notice.lock().unwrap() = Some((None, "finish setup".into()));
    assert_eq!(opening(&f.state).notice.as_deref(), Some("finish setup"));
    assert!(opening(&f.state).notice.is_none());
}
#[test]
fn an_unticked_file_edited_after_the_preview_doesnt_stop_the_rest() {
    let f = Fixture::new();
    let a = f.write("Downloads/a.pdf");
    let b = f.write("Downloads/b.pdf");
    let Outcome::Plan { id, .. } = f.command("move PDFs from Downloads to Documents") else { panic!("expected a plan") };
    let at = f.actions().iter().position(|s| matches!(s, Action::Move { from, .. } if *from == b)).unwrap();
    fs::write(&b, b"edited").unwrap();
    assert!(confirm_input(&f.state, id).unwrap_err().contains("“b.pdf” changed"));
    assert!(confirm_trimmed(&f.state, id, &[at]).unwrap().error.is_none());
    assert!(!a.exists() && b.exists());
}
#[test]
fn subfolder_opt_in_guards_old_rules_skipped_trees_and_followups() {
    let f = Fixture::new();
    let nested = f.write("Downloads/Project/spec.pdf");
    // A rule saved before the opt-in read "Subfolders" as a name: skipped, not widened.
    let rule = scheduler::Rule {
        every: 1,
        unit: scheduler::Unit::Days,
        time: None,
        command: "delete PDFs in Downloads including subfolders".into(),
        choices: HashMap::new(),
        model_intent: None,
        approved_version: 1,
        approved_source: Some(scheduler::RuleSource::Command),
        approved_nicknames: None,
    };
    let job = scheduler::Job::recurring_later(
        "delete PDFs in Downloads including subfolders daily".into(),
        chrono::Local::now() - chrono::Duration::minutes(5),
        rule,
    );
    f.state.schedules.lock().unwrap().add(job).unwrap();
    run_due_tasks(&f.state).unwrap();
    assert!(nested.exists());
    assert!(f.state.schedules.lock().unwrap().jobs[0].summary.contains("reads “subfolders” differently"));

    // Starting inside Voxi's own trash or a hidden folder is refused.
    f.write("Downloads/_to_delete/keep/a.pdf");
    f.write("Downloads/.private/inner/b.pdf");
    for text in [
        "move PDFs from Downloads/_to_delete to Documents including subfolders",
        "move PDFs from Downloads/.private to Documents including subfolders",
    ] {
        assert!(matches!(f.command(text), Outcome::Problem { ref message } if message.contains("can't reach inside")), "{text}");
    }

    // A move with direct and nested matches says what it left alone.
    f.write("Downloads/top.pdf");
    let Outcome::Plan { warnings, .. } = f.command("move PDFs from Downloads to Documents") else { panic!("expected a plan") };
    assert!(warnings.iter().any(|w| w.contains("left alone in subfolders")), "{warnings:?}");

    // After a list, "delete them including subfolders" uses the whole list.
    f.list();
    let Outcome::Plan { total, .. } = f.command("delete them including subfolders") else { panic!("expected a plan") };
    assert_eq!(total, f.paths().len());
}
#[test]
fn a_refused_confirm_says_whether_its_preview_is_kept_and_late_stops_say_so() {
    let f = Fixture::new();
    let a = f.write("Downloads/a.pdf");
    f.write("Downloads/b.pdf");
    let Outcome::Plan { id, .. } = f.command("move PDFs from Downloads to Documents") else { panic!("expected a plan") };
    // A step that can't be left out is refused before the preview is taken.
    let err = confirm_reporting(&f.state, id, &[99]).unwrap_err();
    assert!(err.preview_kept, "{err:?}");
    // A changed file too; the preview stays to cancel or plan again.
    fs::write(&a, b"edited").unwrap();
    let err = confirm_reporting(&f.state, id, &[]).unwrap_err();
    assert!(err.preview_kept && err.message.contains("changed after the preview"), "{err:?}");
    // Once planned again and run, a Stop that comes too late says there was nothing to stop.
    let Outcome::Plan { id, .. } = f.command("move PDFs from Downloads to Documents") else { panic!("expected a plan") };
    assert!(confirm_reporting(&f.state, id, &[]).unwrap().error.is_none());
    assert!(!request_stop(&f.state, RunKey::Plan(id)).unwrap());
    let err = confirm_reporting(&f.state, id, &[]).unwrap_err();
    assert!(!err.preview_kept, "{err:?}");
}
#[test]
fn run_now_and_repeat_refuses_a_file_replaced_after_the_preview() {
    let f = Fixture::new();
    let a = f.write("Downloads/a.pdf");
    let Outcome::Plan { id, .. } = f.command("move PDFs from Downloads to Documents every day") else { panic!("expected a plan") };
    // Replace the file, keeping its old timestamp: only its identity differs.
    let when = fs::metadata(&a).unwrap().modified().unwrap();
    let other = f.write("Desktop/replacement.pdf");
    fs::File::options().write(true).open(&other).unwrap().set_modified(when).unwrap();
    fs::rename(&other, &a).unwrap();
    let err = confirm_reporting(&f.state, id, &[]).unwrap_err();
    assert!(err.message.contains("changed after the preview"), "{err:?}");
    assert!(a.exists() && !f.home.join("Documents/a.pdf").exists());
}
#[test]
fn a_stop_during_a_refused_confirm_does_not_carry_over() {
    let f = Fixture::new();
    let a = f.write("Downloads/a.pdf");
    let Outcome::Plan { id, .. } = f.command("move PDFs from Downloads to Documents") else { panic!("expected a plan") };
    assert!(request_stop(&f.state, RunKey::Plan(id)).unwrap());
    assert!(confirm_reporting(&f.state, id, &[99]).unwrap_err().preview_kept);
    assert!(!f.state.pending.lock().unwrap().as_ref().unwrap().stop_requested);
    let result = confirm_reporting(&f.state, id, &[]).unwrap();
    assert!(!result.cancelled && result.error.is_none(), "{result:?}");
    assert!(!a.exists());
}

#[test]
fn settings_write_blocked_refuses_save_and_surfaces_warning() {
    let mut f = Fixture::new();
    let bad = br#"{"version":1,"data":{"hotkey":12345}}"#;
    fs::write(&f.state.settings_path, bad).unwrap();
    settings::WRITE_FAILURE.with(|fail| fail.set(5));
    let (loaded, warning) = Settings::load_with_warning(&f.state.settings_path);
    settings::WRITE_FAILURE.with(|fail| fail.set(0));
    let warning = warning.unwrap();
    assert!(warning.write_blocked);
    assert!(warning.backup.is_none());
    *f.state.settings.lock().unwrap() = loaded;
    f.state.settings_blocked = warning.write_blocked.then_some(warning.message.clone());
    let err = f.state.ensure_settings_writable().unwrap_err();
    assert_eq!(err, warning.message);
    assert_eq!(fs::read(&f.state.settings_path).unwrap(), bad);
}

#[test]
fn run_now_and_repeat_first_run_needs_review_cancelled_and_finish_failure() {
    // NeedsReview on first run must mark the series NeedsReview, not Active.
    {
        let f = Fixture::new();
        f.write("Downloads/a.dmg");
        let Outcome::Plan { id, first_run: scheduler::FirstRunPolicy::RunNowAndRepeat, .. } =
            f.command("delete dmg files from downloads every day")
        else {
            panic!("expected RunNowAndRepeat plan")
        };
        fs::write(f.home.join(".Trash"), b"blocked").unwrap();
        let result = confirm_input(&f.state, id).unwrap();
        assert!(matches!(result.status, model::RunStatus::NeedsReview), "{:?}", result.status);
        assert!(!result.summary.contains("Repeats "), "{}", result.summary);
        let store = scheduler::Store::load(f.home.join("schedules.json"));
        assert_eq!(store.jobs[0].series, scheduler::SeriesState::NeedsReview);
        assert_eq!(store.jobs[0].occurrence, scheduler::OccurrenceState::Finished);
    }

    // Cancelled first run must not activate the series.
    {
        let f = Fixture::new();
        let source = f.write("Downloads/a.pdf");
        let Outcome::Plan { id, first_run: scheduler::FirstRunPolicy::RunNowAndRepeat, .. } =
            f.command("move PDFs from Downloads to Documents every day")
        else {
            panic!("expected RunNowAndRepeat plan")
        };
        request_stop(&f.state, RunKey::Plan(id)).unwrap();
        let result = confirm_input(&f.state, id).unwrap();
        assert!(matches!(result.status, model::RunStatus::Cancelled), "{:?}", result.status);
        assert!(result.cancelled);
        assert!(!result.summary.contains("Repeats "), "{}", result.summary);
        assert!(source.exists());
        let mut store = scheduler::Store::load(f.home.join("schedules.json"));
        assert_eq!(store.jobs[0].series, scheduler::SeriesState::Stopped);
        assert_eq!(store.jobs[0].occurrence, scheduler::OccurrenceState::Finished);
        assert!(store.claim(chrono::Local::now() + chrono::Duration::days(10)).unwrap().is_none());
    }

    // finish() failure on a completed first run must produce a non-Completed status.
    {
        let f = Fixture::new();
        f.write("Downloads/a.pdf");
        let Outcome::Plan { id, first_run: scheduler::FirstRunPolicy::RunNowAndRepeat, .. } =
            f.command("move PDFs from Downloads to Documents every day")
        else {
            panic!("expected RunNowAndRepeat plan")
        };
        // Writes: store.add (1) + journal Prepared/InFlight/Ack/History/HistorySaved/Finished (6) = 7; fail write #8 (store.finish).
        settings::WRITE_FAILURE.with(|w| w.set(2));
        settings::WRITE_FAILURE_AFTER.with(|n| n.set(7));
        settings::WRITE_FAILURE_ONCE.with(|o| o.set(true));
        let result = confirm_input(&f.state, id).unwrap();
        settings::WRITE_FAILURE.with(|w| w.set(0));
        settings::WRITE_FAILURE_AFTER.with(|n| n.set(0));
        settings::WRITE_FAILURE_ONCE.with(|o| o.set(false));
        assert!(result.error.as_deref().is_some_and(|e| e.contains("Couldn't save the first run's result")), "{result:?}");
        assert!(!matches!(result.status, model::RunStatus::Completed), "{:?}", result.status);
    }
}

#[test]
fn recurring_later_run_needs_review_marks_series_needs_review() {
    let f = Fixture::new();
    f.write("Downloads/a.dmg");
    let Outcome::Plan { id, first_run: scheduler::FirstRunPolicy::RepeatLater, .. } =
        f.command("delete dmg files from downloads every day at 9 pm")
    else {
        panic!("expected RepeatLater plan")
    };
    confirm_input(&f.state, id).unwrap();
    fs::write(f.home.join(".Trash"), b"blocked").unwrap();
    f.state.schedules.lock().unwrap().jobs[0].due = chrono::Local::now() - chrono::Duration::minutes(1);
    run_due_tasks(&f.state).unwrap();
    let mut store = scheduler::Store::load(f.home.join("schedules.json"));
    assert_eq!(store.jobs[0].series, scheduler::SeriesState::NeedsReview);
    assert_eq!(store.jobs[0].occurrence, scheduler::OccurrenceState::Finished);
    assert!(store.claim(chrono::Local::now() + chrono::Duration::days(10)).unwrap().is_none());
}

#[test]
fn recurring_folder_and_move_chain_is_idle_once_the_folder_exists() {
    let f = Fixture::new();
    f.write("Downloads/bill.pdf");
    let outcome = f.command("create a folder in downloads called PDFS and move pdfs into it everyday");
    let Outcome::Plan { id, .. } = outcome else { panic!("{outcome:?}") };
    confirm_input(&f.state, id).unwrap();
    assert!(f.home.join("Downloads/PDFS/bill.pdf").is_file());

    f.state.schedules.lock().unwrap().jobs[0].due = chrono::Local::now() - chrono::Duration::minutes(2);
    let (result, events) = diagnostics::capture(|| run_due_tasks(&f.state));
    result.unwrap();

    let store = scheduler::Store::load(f.home.join("schedules.json"));
    let task = store.jobs[0].summary();
    assert_eq!(task.series, scheduler::SeriesState::Active);
    assert!(task.summary.contains("Nothing to change."), "{}", task.summary);
    assert!(!task.summary.contains("Skipped"), "{}", task.summary);
    assert!(!events.iter().any(|line| line.contains("code=E_PLANNING_FAILED ")), "{events:?}");
}

#[test]
fn daily_create_then_delete_contents_creates_now_and_empties_later() {
    let f = Fixture::new();
    let outcome = f.command("create a folder in downloads named test1 and delete its content every day");
    let Outcome::Plan { id, total: 1, warnings, .. } = outcome else { panic!("{outcome:?}") };
    assert!(warnings.iter().any(|w| w.contains("“test1” is new")), "{warnings:?}");
    assert!(confirm_input(&f.state, id).unwrap().error.is_none());
    assert!(f.home.join("Downloads/test1").is_dir());
    let old = f.write("Downloads/test1/old.txt");
    f.state.schedules.lock().unwrap().jobs[0].due = chrono::Local::now() - chrono::Duration::minutes(2);
    run_due_tasks(&f.state).unwrap();
    assert!(!old.exists());
    assert!(f.home.join("Downloads/test1").is_dir());
}
