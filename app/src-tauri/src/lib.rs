use settings::diagnostics;
mod exec;
mod operation_journal;
#[cfg(any(test, feature = "tiny-model"))]
mod fallback_guard;
#[cfg(any(test, feature = "tiny-model"))]
mod fallback_slots;
#[cfg(feature = "tiny-model")]
mod tiny_model;
#[cfg(feature = "tiny-model")]
mod plan_model;
mod fsx;
mod history;
mod learn;
mod model;
mod parse;
#[cfg(test)]
mod luna_folder_audit;
#[cfg(test)]
mod luna_command_audit;
mod pdf;
mod plan;
mod presentation;
#[cfg(target_os = "macos")]
mod outside_click;
mod settings;
mod search;
mod scope;
mod scheduler;
mod trash;
mod tray_face;
#[cfg(test)]
mod paraphrase_audit; // report-only, all tests #[ignore]
#[cfg(test)]
mod combo_audit; // report-only, #[ignore]

use fsx::{Roots, file_name};
use diagnostics::{Event, Code, Stage};
use history::{Entry, History};
use model::{Action, Outcome, PreviewItem, RunResult};
use plan::{Planned, Planner, Stop};
use settings::Settings;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, State};
use tauri_plugin_autostart::ManagerExt as _;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

// Poison means an earlier operation may have left invariants incomplete.
// Never obtain the poisoned guard; fail the request and require a restart.
macro_rules! checked_state {
    ($lock:expr, ()) => {
        match $lock { Ok(guard) => guard, Err(_) => {
            eprintln!("Voxi state lock failed; restart required");
            return;
        } }
    };
    ($lock:expr, $failure:expr) => {
        match $lock { Ok(guard) => guard, Err(_) => {
            eprintln!("Voxi state lock failed; restart required");
            return $failure;
        } }
    };
}

const WINDOW_WIDTH: f64 = 680.0;
const PREVIEW_LIMIT: usize = 20;

struct Pending {
    id: u64,
    command: String,
    actions: Vec<Action>,
    undo_of: Option<u64>,
    context_generation: Option<u64>,
    context_snapshot: Option<scope::Snapshot>,
    scheduled: Option<scheduler::Job>,
    stop_requested: bool,
}
impl Pending {
    fn validate_context(&self, context: &scope::State) -> Result<(), String> {
        if self.context_generation.is_some_and(|generation| generation != context.generation) {
            return Err("The previous list expired. Run the list command again.".into());
        }
        self.context_snapshot.as_ref().map_or(Ok(()), scope::Snapshot::validate)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum RunKey {
    Plan(u64),
    Scheduled(u64, chrono::DateTime<chrono::Local>),
}

type ActiveRuns = Mutex<HashMap<RunKey, Arc<AtomicBool>>>;

struct ActiveRun<'a> {
    runs: &'a ActiveRuns,
    key: RunKey,
    stopped: Arc<AtomicBool>,
}

impl<'a> ActiveRun<'a> {
    fn register(runs: &'a ActiveRuns, key: RunKey, stop_requested: bool) -> Result<Self, String> {
        let stopped = Arc::new(AtomicBool::new(stop_requested));
        runs.lock().map_err(|_| "Execution state unavailable. Restart Voxi.")?.insert(key, stopped.clone());
        Ok(Self { runs, key, stopped })
    }

    fn is_stopped(&self) -> bool { self.stopped.load(Ordering::Relaxed) }
}

impl Drop for ActiveRun<'_> {
    fn drop(&mut self) {
        if let Ok(mut runs) = self.runs.lock() { runs.remove(&self.key); }
    }
}

struct AppState {
    roots: RwLock<Roots>,
    history: Mutex<History>,
    pending: Mutex<Option<Pending>>,
    settings: Mutex<Settings>,
    schedules: Mutex<scheduler::Store>,
    execution: Mutex<()>,
    active_runs: ActiveRuns,
    settings_path: PathBuf,
    /// "This was wrong" reports, kept on this Mac.
    reports_path: PathBuf,
    result_scope: Mutex<scope::State>,
    next_plan: AtomicU64,
    active_search: Mutex<Option<(u64, search::Control)>>,
    presentation: Mutex<presentation::Presentation>,
}

fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("/var/empty/voxi-home-unavailable"))
}

/// The folder macOS saves screenshots to, if the user changed it.
fn screenshot_location() -> Option<PathBuf> {
    let out = Command::new("/usr/bin/defaults").args(["read", "com.apple.screencapture", "location"]).output().ok()?;
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!s.is_empty()).then(|| fsx::expand_tilde(&s, &home()))
}

fn build_roots(settings: &Settings) -> Roots {
    let h = home();
    Roots::new(&h, screenshot_location(), &settings.extra_paths(&h)).with_nicknames(settings.nickname_paths(&h))
}

// ---- Window ------------------------------------------------------------------

#[cfg(target_os = "macos")]
fn fade_window(ns: &objc2_app_kit::NSWindow, visible: bool, animate: bool) {
    use objc2_app_kit::{NSAnimatablePropertyContainer, NSAnimationContext};
    let alpha = if visible { 1.0 } else { 0.0 };
    if !animate { ns.setAlphaValue(alpha); return; }
    NSAnimationContext::beginGrouping();
    NSAnimationContext::currentContext().setDuration(if visible { 0.2 } else { 0.14 });
    ns.animator().setAlphaValue(alpha);
    NSAnimationContext::endGrouping();
}

/// Shows the command window on the screen with the cursor and focuses it.
/// `mode` tells the page which view to open (command, settings, welcome).
fn show_window(app: &AppHandle, mode: &str) {
    let Some(w) = app.get_webview_window("main") else { return };
    if let Ok(cursor) = app.cursor_position()
        && let Ok(Some(m)) = app.monitor_from_point(cursor.x, cursor.y) {
            let (pos, size, scale) = (m.position(), m.size(), m.scale_factor());
            let x = pos.x + (size.width as i32 - (WINDOW_WIDTH * scale) as i32) / 2;
            let y = pos.y + (size.height as f64 * 0.22) as i32;
            let _ = w.set_position(PhysicalPosition::new(x, y));
        }
    let state = app.state::<AppState>();
    let mut presentation = checked_state!(state.presentation.lock(), ());
    let request = presentation.request(mode);
    if presentation.ready { let _ = w.emit("voxi://shown", request); }

}

fn hide(app: &AppHandle) {
    checked_state!(app.state::<AppState>().result_scope.lock(), ()).clear();
    checked_state!(app.state::<AppState>().presentation.lock(), ()).cancel();
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.hide();
    }
    // Hands focus back to the app the user was in.
    #[cfg(target_os = "macos")]
    let _ = app.hide();
}

/// Asks the page to play its close animation; the page then calls
/// `hide_window`.
fn request_hide(app: &AppHandle) {
    checked_state!(app.state::<AppState>().presentation.lock(), ()).cancel();
    match app.get_webview_window("main") {
        Some(w) if w.emit("voxi://hide", ()).is_ok() => {}
        _ => hide(app),
    }
}

fn toggle_window(app: &AppHandle) {
    let Some(w) = app.get_webview_window("main") else { return };
    let (pending, closing) = {
        let state = app.state::<AppState>();
        let presentation = checked_state!(state.presentation.lock(), ());
        (presentation.pending.is_some(), presentation.closing)
    };
    if !closing && (pending || (w.is_visible().unwrap_or(false) && w.is_focused().unwrap_or(false))) {
        request_hide(app);
    } else {
        show_window(app, "command");
    }
}

fn register_hotkey(app: &AppHandle, hotkey: &str) -> Result<(), String> {
    let shortcut: Shortcut = hotkey.parse().map_err(|e| format!("“{hotkey}” isn't a valid shortcut: {e}"))?;
    let gs = app.global_shortcut();
    let _ = gs.unregister_all();
    gs.register(shortcut).map_err(|e| format!("Couldn't register {hotkey}: {e}. Another app may be using it."))
}

// ---- Preview -----------------------------------------------------------------

fn describe(a: &Action, roots: &Roots) -> (String, String, String) {
    let name = |p: &PathBuf| file_name(p);
    let dir = |p: &PathBuf| p.parent().map(|d| roots.display(d)).unwrap_or_default();
    let dest = |from: &PathBuf, to: &PathBuf| {
        if name(from) == name(to) { format!("→ {}", dir(to)) } else { format!("→ {}/{}", dir(to), name(to)) }
    };
    let many = |paths: &Vec<PathBuf>, what: &str| format!("{} {what}", paths.len());
    match a {
        Action::Move { from, to } => ("move".into(), name(from), format!("from {} {}", dir(from), dest(from, to))),
        Action::Copy { from, to } => ("copy".into(), name(from), format!("from {} {}", dir(from), dest(from, to))),
        Action::Rename { path, new_name } => ("rename".into(), name(path), format!("→ {new_name}")),
        Action::Trash { path } => ("trash".into(), name(path), format!("from {} → Trash", dir(path))),
        Action::RestoreFromTrash { to, .. } => ("restore".into(), name(to), format!("back from the Trash to {}", dir(to))),
        Action::CreateFolder { path } => ("create_folder".into(), name(path), format!("new folder in {}", dir(path))),
        Action::ConvertImage { path, output, .. } => ("convert_image".into(), name(path), format!("→ {}", name(output))),
        Action::ResizeImage { path, max_width_px, .. } => ("resize_image".into(), name(path), format!("→ {max_width_px} px wide")),
        Action::ImagesToPdf { paths, output } => ("images_to_pdf".into(), many(paths, "images"), dest(paths.first().unwrap_or(output), output)),
        Action::MergePdf { paths, output } => ("merge_pdf".into(), many(paths, "PDFs"), format!("→ {}/{}", dir(output), name(output))),
        Action::SplitPdf { path, output_dir } => ("split_pdf".into(), name(path), format!("→ {}/ (one PDF per page)", name(output_dir))),
        Action::ExtractPages { path, from, to, output } => ("extract_pages".into(), name(path), format!("pages {from}–{to} → {}", name(output))),
        Action::Zip { path, output } => ("zip".into(), name(path), format!("→ {}", name(output))),
        Action::Unzip { path, destination } => ("unzip".into(), name(path), format!("→ {}", roots.display(destination))),
        Action::OpenInFinder { path, .. } => ("open_in_finder".into(), name(path), "open in Finder".into()),
        Action::RemoveEmptyFolder { path } => ("remove_folder".into(), name(path), "remove this folder if it's empty".into()),
        Action::Restore { from: _, to } => ("restore".into(), name(to), "restore the original".into()),
    }
}

fn preview(actions: &[Action], notes: &[Option<String>], roots: &Roots) -> Vec<PreviewItem> {
    actions
        .iter()
        .zip(notes.iter().chain(std::iter::repeat(&None)))
        .take(PREVIEW_LIMIT)
        .map(|(a, note)| {
            let (kind, primary, detail) = describe(a, roots);
            PreviewItem { kind, primary, detail, note: note.clone() }
        })
        .collect()
}

// ---- Commands ----------------------------------------------------------------

#[tauri::command(async)]
fn plan_command(state: State<AppState>, text: String, choices: HashMap<String, String>, request_id: u64) -> CommandReply {
    let outcome = plan_input(&state, text, choices, request_id);
    let scope = checked_state!(state.result_scope.lock(), CommandReply { outcome: Outcome::Problem { message: "Voxi state is unavailable. Restart the app.".into() }, scope: None }).summary();
    CommandReply { outcome, scope }
}

#[derive(serde::Serialize)]
struct CommandReply { outcome: Outcome, scope: Option<scope::Summary> }

#[tauri::command]
fn clear_result_scope(state: State<AppState>) { checked_state!(state.result_scope.lock(), ()).clear(); }
#[tauri::command]
fn result_scope(state: State<AppState>) -> Option<scope::Summary> { checked_state!(state.result_scope.lock(), None).summary() }

/// A weekday named in the command ("every Monday"), ignoring surrounding words.
#[cfg(feature = "tiny-model")]
fn named_weekday(text: &str) -> Option<chrono::Weekday> {
    use chrono::Weekday;
    text.split(|c: char| !c.is_alphabetic())
        .find_map(|word| match word.to_ascii_lowercase().as_str() {
            "monday" => Some(Weekday::Mon),
            "tuesday" => Some(Weekday::Tue),
            "wednesday" => Some(Weekday::Wed),
            "thursday" => Some(Weekday::Thu),
            "friday" => Some(Weekday::Fri),
            "saturday" => Some(Weekday::Sat),
            "sunday" => Some(Weekday::Sun),
            _ => None,
        })
}

/// The model may only keep a schedule the scheduler also finds in the user's
/// own words, with the same instant and repeat shape. `found` is that single
/// scheduler reading, computed once by the caller. A named weekday must be
/// the day the model's schedule lands on.
#[cfg(feature = "tiny-model")]
fn model_schedule_ok(model: &Option<scheduler::When>, found: Option<&scheduler::When>, text: &str) -> bool {
    use chrono::Datelike;
    match (found, model) {
        (None, None) => {}
        (Some(rule), Some(chosen)) => {
            if rule.due != chosen.due { return false; }
            match (&rule.rule, &chosen.rule) {
                (None, None) => {}
                (Some(a), Some(b)) => {
                    if a.every != b.every || a.unit != b.unit || a.time != b.time { return false; }
                }
                _ => return false,
            }
        }
        _ => return false,
    }
    if let Some(day) = named_weekday(text)
        && !model.as_ref().is_some_and(|w| w.due.weekday() == day)
    {
        return false;
    }
    true
}

/// Where a plan goes, decided without ever loading model weights. A recurring
/// job may only come from a rule interpretation: when a rule exists it wins,
/// otherwise a recurring plan is refused and a one-off plan uses the model.
#[cfg(feature = "tiny-model")]
#[derive(Debug, PartialEq, Eq)]
enum Route { Rule, Model, Refuse }

#[cfg(feature = "tiny-model")]
fn recurring_route(rule_recurring: bool, rule_intent_present: bool, model_recurring: bool) -> Route {
    if rule_recurring || model_recurring {
        if rule_intent_present { Route::Rule } else { Route::Refuse }
    } else {
        Route::Model
    }
}

fn plan_input(state: &AppState, text: String, choices: HashMap<String, String>, request_id: u64) -> Outcome {
    if parse::check_length(&text).is_err() {
        let message = "That command is too long.".to_string();
        let mut active = checked_state!(state.active_search.lock(), Outcome::Problem { message: "Search state unavailable. Restart Voxi.".into() });
        if let Some((_, previous)) = active.take() { previous.cancel(); }
        checked_state!(state.pending.lock(), Outcome::Problem { message: "Preview state unavailable. Restart Voxi.".into() }).take();
        return Outcome::Problem { message };
    }
    let now = chrono::Local::now();
    let control = search::Control::default();
    {
        let mut active = checked_state!(state.active_search.lock(), Outcome::Problem { message: "Voxi state is unavailable. Restart the app.".into() });
        if let Some((_, previous)) = active.take() { previous.cancel(); }
        checked_state!(state.pending.lock(), Outcome::Problem { message: "Voxi state is unavailable. Restart the app.".into() }).take();
        *active = Some((request_id, control.clone()));
    }
    let original_text = text.clone();
    let (initial_generation, previous_scope) = {
        let context = checked_state!(state.result_scope.lock(), Outcome::Problem { message: "Voxi state is unavailable. Restart the app.".into() });
        (context.generation, context.current.clone())
    };
    // One scheduler reading and one parse of the command base are shared by
    // the rule path, the shortcut probe and the model gate. Re-running either
    // for each consumer used to parse the same words four or five times.
    let raw_schedule = scheduler::extract_schedule(&text, now);
    // A rejected time phrase is terminal, including for model fallback.
    if let Err(message) = &raw_schedule { return Outcome::Problem { message: message.clone() }; }
    #[cfg(feature = "tiny-model")]
    let found = raw_schedule.as_ref().ok().and_then(|(_, when)| when.as_ref());
    let rule_schedule = schedule_of(state, &text, now, &raw_schedule);
    let rule_base = rule_schedule.as_ref().ok().map(|(base, _)| base.clone());
    let probe = rule_base.as_deref().unwrap_or(&text);
    let classified = parse::parse_classified(probe);
    #[cfg(feature = "tiny-model")]
    let rule_intent = classified.clone().ok();

    // A saved shortcut wins over both the rule parser and the model. Resolve
    // it up front, exact phrase and near miss, before any model inference.
    let shortcut = {
        let settings = checked_state!(state.settings.lock(), Outcome::Problem { message: "Voxi state is unavailable. Restart the app.".into() });
        settings.find_shortcut(&text)
            .or_else(|| rule_base.as_ref().and_then(|base| settings.find_shortcut(base)))
            .or_else(|| classified.is_err().then(|| settings.find_close_shortcut(probe)).flatten())
            .cloned()
    };

    if let Some(s) = &shortcut
        && let Err(message) = settings::check_shortcut_commands(&s.commands) { return Outcome::Problem { message }; }
    #[cfg(feature = "tiny-model")]
    let interpreted = {
        let mut model_error: Option<String> = None;
        let rule_recurring = rule_schedule.as_ref().ok()
            .and_then(|(_, w)| w.as_ref())
            .is_some_and(|w| w.rule.is_some());
        let rule_intent_present = rule_intent.is_some();
        // A recurring job may only come from a rule interpretation. When the
        // rule parser already read a recurring command, skip model inference
        // entirely: its intent can disagree harmlessly with the model's and
        // must not be used as the job's authoritative interpretation.
        let chosen = if recurring_route(rule_recurring, rule_intent_present, false) == Route::Rule {
            None
        } else if plan_model::enabled()
            && previous_scope.is_none()
            && shortcut.is_none()
            && plan_model::eligible(&text, &classified)
        {
            match plan_model::interpret_at(&text, || control.is_cancelled(), now) {
                Ok(Some(value)) => {
                    let model_recurring = value.when.as_ref().is_some_and(|w| w.rule.is_some());
                    match recurring_route(rule_recurring, rule_intent_present, model_recurring) {
                        Route::Rule => None, // use the rule path; its rule keeps model_intent: None
                        Route::Refuse => return Outcome::Problem { message: "Repeating tasks need a command Voxi understands without the model.".into() },
                        Route::Model => {
                            // Cross-check a non-recurring model schedule against
                            // the scheduler's reading of the user's own words.
                            if !model_schedule_ok(&value.when, found, &text) {
                                model_error = Some("The model's schedule didn't match your command.".into());
                                None
                            } else {
                                Some(value)
                            }
                        }
                    }
                }
                Ok(None) => None,
                Err(message) => { model_error = Some(message); None },
            }
        } else { None };
        // If the rule parser can't read the command either, surface why the
        // model was rejected rather than a generic "didn't understand".
        if chosen.is_none() && rule_intent.is_none()
            && let Some(message) = model_error
        {
            return Outcome::Problem { message };
        }
        chosen
    };
    #[cfg(not(feature = "tiny-model"))]
    let interpreted: Option<RuleInterpretation> = None;
    #[cfg(not(feature = "tiny-model"))]
    struct RuleInterpretation { intent: model::Intent, when: Option<scheduler::When> }
    let used_model = interpreted.is_some();
    let (model_intent, text, when) = match interpreted {
        Some(value) => (Some(value.intent), original_text.clone(), value.when),
        None => match rule_schedule {
            Ok((text, when)) => (None, text, when),
            Err(message) => return Outcome::Problem { message },
        },
    };
    let scheduled_for = when.as_ref().map(|w| w.due);
    let repeat = when.and_then(|w| w.rule);
    // A saved shortcut ("run my usuals") wins over the built-in commands.
    enum Work {
        Shortcut(String, Vec<(String, model::Intent)>),
        Command(model::Intent),
    }
    let mut use_scope = false;
    let work = match shortcut {
        Some(s) => {
            let parsed: Result<Vec<_>, _> = s.commands.iter().map(|c| parse::parse(c).map(|i| (c.clone(), i))).collect();
            match parsed {
                // A shortcut holding one read-only command runs it as if typed.
                Ok(mut commands) if commands.len() == 1 && settings::shortcut_read_only(&commands[0].1) => Work::Command(commands.remove(0).1),
                Ok(commands) => Work::Shortcut(s.phrase, commands),
                Err(bad) => return Outcome::Problem { message: format!("In “{}”, Voxi doesn't understand “{bad}”. Fix it in Settings.", s.phrase) },
            }
        }
        None if model_intent.is_some() => Work::Command(model_intent.expect("is_some match guard establishes model intent")),
        None => match scope::prepare(&text, previous_scope.as_ref()) {
            Ok((intent, scoped)) => { use_scope = scoped; Work::Command(intent) },
            Err(raw) if raw == text.trim() => {
                #[cfg(feature = "tiny-model")]
                let recovered = if scheduled_for.is_none() && previous_scope.is_none() && !plan_model::enabled() && std::env::var_os("VOXI_TINY_MODEL_DIR").is_some() {
                    tiny_model::recover(&text, || control.is_cancelled())
                } else { None };
                #[cfg(not(feature = "tiny-model"))]
                let recovered: Option<model::Intent> = None;
                match recovered {
                    Some(intent) => Work::Command(intent),
                    None => return Outcome::NotUnderstood { text: raw },
                }
            },
            Err(message) => return Outcome::Problem { message },
        },
    };
    let (scope_generation, scoped_snapshot) = {
        let active = checked_state!(state.active_search.lock(), Outcome::Problem { message: "Voxi state is unavailable. Restart the app.".into() });
        if active.as_ref().is_none_or(|(id, _)| *id != request_id) { return Outcome::Problem { message: "Search cancelled.".into() }; }
        let mut context = checked_state!(state.result_scope.lock(), Outcome::Problem { message: "Voxi state is unavailable. Restart the app.".into() });
        if use_scope && context.generation != initial_generation { return Outcome::Problem { message: "The previous list expired. Run the list command again.".into() }; }
        if !use_scope { context.clear(); }
        (context.generation, if use_scope { context.current.clone() } else { None })
    };
    let listing = matches!(&work, Work::Command(model::Intent::List { .. }));
    let roots = checked_state!(state.roots.read(), Outcome::Problem { message: "Voxi state is unavailable. Restart the app.".into() }).clone();
    let choices: HashMap<String, PathBuf> = choices.into_iter().map(|(k, v)| (k, PathBuf::from(v))).collect();
    let planned = {
        let history = checked_state!(state.history.lock(), Outcome::Problem { message: "Voxi state is unavailable. Restart the app.".into() });
        let planner = Planner::new(&roots, &choices).with_control(control.clone()).with_scope(scoped_snapshot.clone());
        match &work {
            Work::Shortcut(phrase, commands) => planner.plan_shortcut(phrase, commands, &history),
            Work::Command(intent) => planner.plan(intent, &history),
        }
    };
    // Fingerprinting can walk a folder tree. Keep it cancellable and outside
    // the publication lock; only installing the resulting preview is serialized.
    let scheduled = match (&planned, scheduled_for) {
        (Ok(Planned::Steps { actions, undo_of, .. }), Some(due)) => Some(
            if undo_of.is_some() { Err("Undo cannot be scheduled.".into()) }
            else if let Some(rule) = &repeat {
                if use_scope { Err("A repeating task can't use a previous list. Name the files or folder instead.".into()) }
                else {
                    scheduler::check_actions(actions).map(|()| scheduler::Job::recurring(original_text.trim().to_string(), due,
                        scheduler::Rule { choices: choices.clone(), ..rule.clone() }))
                }
            }
            else { scheduler::Job::new_with_control(original_text.clone(), due, actions.clone(), &control) }
        ),
        _ => None,
    };
    // Serialize publication with cancellation so a late response cannot create a live preview.
    let active = checked_state!(state.active_search.lock(), Outcome::Problem { message: "Voxi state is unavailable. Restart the app.".into() });
    if control.is_cancelled() || active.as_ref().is_none_or(|(id, _)| *id != request_id) {
        return Outcome::Problem { message: "Search cancelled.".into() };
    }
    match planned {
        Err(Stop::Problem(message)) => Outcome::Problem { message },
        Err(Stop::Ambiguous { name, candidates }) => Outcome::Ambiguous {
            name,
            folders: !candidates.is_empty() && candidates.iter().all(|p| p.is_dir()),
            candidates: candidates.iter().map(|p| p.to_string_lossy().into_owned()).collect(),
        },
        Ok(Planned::Info { .. } | Planned::Direct { .. }) if scheduled_for.is_some() =>
            Outcome::Problem { message: "Schedule a command that changes files. Lists, History, and Finder commands run immediately when requested without a time.".into() },
        Ok(Planned::Info { title, summary, rows, total, listed }) => {
            let mut numbered = false;
            if listing {
                let mut context = checked_state!(state.result_scope.lock(), Outcome::Problem { message: "Voxi state is unavailable. Restart the app.".into() });
                if context.generation == scope_generation {
                    match listed.and_then(|(paths, label)| scope::Snapshot::capture(paths,
                        previous_scope.as_ref().filter(|_| use_scope).map_or(label, |s| s.label.clone()))) {
                        Some(snapshot) => { context.publish(snapshot); numbered = true; },
                        None => context.clear(), // Partial/unreadable results must never become an action scope.
                    }
                }
            }
            Outcome::Info { title, summary, rows, total, numbered }
        },
        Ok(Planned::Direct { action, title }) => {
            let out = exec::run(std::slice::from_ref(&action), &roots);
            match out.error {
                Some(message) => Outcome::Problem { message },
                None => Outcome::Info { title, summary: String::new(), rows: vec![], total: 0, numbered: false },
            }
        }
        Ok(Planned::Steps { title, actions, notes, mut warnings, undo_of }) => {
            if used_model { warnings.insert(0, "Model interpretation: review the source folders and schedule before confirming.".into()); }
            let scheduled = match scheduled.transpose() {
                Ok(job) => job,
                Err(message) => return Outcome::Problem { message },
            };
            match (scheduled_for, &repeat) {
                (Some(due), Some(rule)) if scheduler::runs_on_confirm(&original_text) => warnings.insert(0, format!(
                    "Runs now, then {}. Next run {}. Later runs change whatever matches at that time, without a preview, and can be undone from History. Voxi must be running; a missed run happens once when it next opens.",
                    rule.describe(due), scheduler::display(due))),
                (Some(due), Some(rule)) => warnings.insert(0, format!(
                    "Repeats {}, first on {}. Each run changes whatever matches at that time, without a preview, and can be undone from History. These are the files that match now. Voxi must be running; a missed run happens once when it next opens.",
                    rule.describe(due), scheduler::display(due))),
                (Some(due), None) => warnings.insert(0, format!("Scheduled for {}. Only the files shown here will be changed. Voxi must be running; overdue tasks run when it next opens.", scheduler::display(due))),
                _ => {}
            }
            warnings.extend(checked_state!(state.history.lock(), Outcome::Problem { message: "History unavailable. Restart Voxi.".into() }).notices.iter().cloned());
            let id = state.next_plan.fetch_add(1, Ordering::SeqCst);
            if use_scope { warnings.insert(0, "Using files from the previous searched list.".into()); }
            let items = preview(&actions, &notes, &roots);
            let total = actions.len();
            *checked_state!(state.pending.lock(), Outcome::Problem { message: "Voxi state is unavailable. Restart the app.".into() }) = Some(Pending { id, command: original_text.trim().to_string(), actions, undo_of, scheduled, context_generation: use_scope.then_some(scope_generation), context_snapshot: scoped_snapshot, stop_requested: false });
            Outcome::Plan { id, title, total, items, warnings, is_undo: undo_of.is_some(), scheduled_for, repeats: repeat.as_ref().zip(scheduled_for).map(|(rule, due)| if scheduler::runs_on_confirm(&original_text) { format!("Runs now, then {}", rule.describe(due)) } else { rule.describe(due) }) }
        }
    }
}

#[tauri::command(async)]
fn confirm_plan(state: State<AppState>, id: u64) -> Result<RunResult, String> {
    confirm_input(&state, id)
}

fn confirm_input(state: &AppState, id: u64) -> Result<RunResult, String> {
    let (p, run) = {
        let mut pending = checked_state!(state.pending.lock(), Err("Voxi state is unavailable. Restart the app.".into()));
        let p = pending.take_if(|p| p.id == id).ok_or("That preview expired. Press Enter to plan it again.")?;
        // Publish the flag before releasing the preview lock, including when
        // confirmation has to wait for an earlier filesystem operation.
        let run = ActiveRun::register(&state.active_runs, RunKey::Plan(id), p.stop_requested)?;
        (p, run)
    };
    let is_cancelled = || run.is_stopped();
    let _execution = checked_state!(state.execution.lock(), Err("Voxi state is unavailable. Restart the app.".into()));
    p.validate_context(&*checked_state!(state.result_scope.lock(), Err("Voxi state is unavailable. Restart the app.".into())))?;
    if let Some(mut job) = p.scheduled.clone() {
        if job.repeat.is_some() && scheduler::runs_on_confirm(&job.command) {
            let now = chrono::Local::now();
            let refreshed = scheduler::extract_schedule(&job.command, now)?.1.ok_or("That schedule is no longer valid.")?;
            job.due = refreshed.due;
            // Anchor the saved wall clock at confirmation as well as its due
            // date, retaining the approved folder choices and rule command.
            if let (Some(rule), Some(fresh_rule)) = (&mut job.repeat, refreshed.rule) {
                rule.time = fresh_rule.time;
            }
            job.validate()?;
            // Persist before mutating files: a storage failure must not leave an
            // executed first run with no saved repeat. The execution lock keeps
            // the scheduler out until this approved, durable run finishes.
            let mut store = checked_state!(state.schedules.lock(), Err("Voxi state is unavailable. Restart the app.".into()));
            store.add(job.clone())?;
            let job_id = store.jobs.last().ok_or("That task could not be saved.")?.id;
            drop(store);
            let mut result = execute_pending(state, p, &is_cancelled);
            let run_summary = result.error.as_ref().map_or_else(|| result.summary.clone(), |error| format!("{} {error}", result.summary));
            // As with later recurring runs, execution failures retain the series:
            // missing files and collisions may be transient. Structural errors
            // have already been rejected by validation above.
            if let Err(error) = checked_state!(state.schedules.lock(), Err("Voxi state is unavailable. Restart the app.".into())).record_summary(job_id, run_summary) {
                result.error = Some(result.error.map_or(error.clone(), |first| format!("{first} {error}")));
            }
            let rule = job.repeat.as_ref().ok_or("That task doesn't repeat.")?;
            result.summary.push_str(&format!(" Repeats {}; next run {}. Manage tasks in Settings → Scheduled.", rule.describe(job.due), scheduler::display(job.due)));
            return Ok(result);
        }
        if job.due <= chrono::Local::now() { return Err("That scheduled time passed while reviewing. Preview the command again.".into()); }
        job.validate()?;
        checked_state!(state.schedules.lock(), Err("Voxi state is unavailable. Restart the app.".into())).add(job.clone())?;
        let summary = match &job.repeat {
            Some(rule) => format!("Repeats {}, first on {}. Manage tasks in Settings → Scheduled.", rule.describe(job.due), scheduler::display(job.due)),
            None => format!("Scheduled for {}. Manage tasks in Settings → Scheduled.", scheduler::display(job.due)),
        };
        return Ok(RunResult { done: 0, total: p.actions.len(), error: None, cancelled: false, undoable: false, summary });
    }
    Ok(execute_pending(state, p, &is_cancelled))
}

#[tauri::command]
fn cancel_execution(state: State<AppState>, id: u64, scheduled_for: Option<chrono::DateTime<chrono::Local>>) -> Result<(), String> {
    let key = scheduled_for.map_or(RunKey::Plan(id), |due| RunKey::Scheduled(id, due));
    request_stop(&state, key)
}

fn request_stop(state: &AppState, key: RunKey) -> Result<(), String> {
    if let RunKey::Plan(id) = key {
        let mut pending = state.pending.lock().map_err(|_| "Preview state unavailable. Restart Voxi.")?;
        if let Some(p) = pending.as_mut().filter(|p| p.id == id) {
            p.stop_requested = true;
            return Ok(());
        }
    }
    if let Some(stopped) = state.active_runs.lock().map_err(|_| "Execution state unavailable. Restart Voxi.")?.get(&key) {
        stopped.store(true, Ordering::Relaxed);
    }
    Ok(())
}

fn execute_pending(state: &AppState, p: Pending, is_cancelled: &dyn Fn() -> bool) -> RunResult {
    match execute_durable(state, &p, is_cancelled) {
        Ok(result) => result,
        Err(error) => RunResult { done: 0, total: p.actions.len(), error: Some(error),
            cancelled: false, undoable: false, summary: "Stopped. Durable execution or undo could not be confirmed; review the recovery journal.".into() },
    }
}

fn execute_durable(state: &AppState, p: &Pending, is_cancelled: &dyn Fn() -> bool) -> Result<RunResult, String> {
    let roots = state.roots.read().map_err(|_| "Folder state is unavailable. Restart Voxi.")?.clone();
    let mut history = state.history.lock().map_err(|_| "History state is unavailable. Restart Voxi.")?;
    let path = history.journal_path();
    let entry = Entry { id: history.next_id(), time: chrono::Local::now(), command: p.command.clone(),
        done: vec![], inverse: vec![], undone: false, undo_of: p.undo_of };
    let out = operation_journal::run(&path, &p.actions, &roots, entry, &mut history, is_cancelled)?;
    let mut context = state.result_scope.lock().map_err(|_| "Result state is unavailable. Restart Voxi.")?;
    if p.context_generation == Some(context.generation)
        && let Some(snapshot) = &mut context.current { snapshot.apply(&out.done); }
    let done = out.done.len();
    let undoable = !out.inverse.is_empty() && p.undo_of.is_none();
    let summary = match (out.cancelled, &out.error, p.undo_of) {
        (true, _, _) => stopped_summary(&p.actions, &out.done, undoable, p.undo_of.is_some()),
        (false, None, Some(_)) => "Undone.".to_string(),
        (false, None, None) => format!("Done. {done} {}.", if done == 1 { "change" } else { "changes" }),
        (false, Some(_), _) => format!("Stopped after {done} of {} steps.", p.actions.len()),
    };
    Ok(RunResult { done, total: p.actions.len(), error: out.error, cancelled: out.cancelled, undoable, summary })
}

fn stopped_summary(actions: &[Action], done: &[Action], undoable: bool, was_undo: bool) -> String {
    let is_move = |action: &Action| matches!(action, Action::Move { .. });
    let is_trash = |action: &Action| matches!(action, Action::Trash { .. });
    let mut summary = if was_undo {
        format!("Stopped. Undid {} of {} steps.", done.len(), actions.len())
    } else if actions.iter().all(|action| matches!(action, Action::Move { .. } | Action::CreateFolder { .. }))
        && actions.iter().any(is_move) {
        let moved = done.iter().filter(|action| is_move(action)).count();
        let total = actions.iter().filter(|action| is_move(action)).count();
        let items = if total == 1 { "item" } else { "items" };
        format!("Stopped. Moved {moved} of {total} {items}.")
    } else if actions.iter().all(is_trash) {
        let total = actions.len();
        let items = if total == 1 { "item" } else { "items" };
        format!("Stopped. Moved {} of {total} {items} to the Trash.", done.len())
    } else {
        format!("Stopped. Completed {} of {} steps.", done.len(), actions.len())
    };
    if undoable { summary.push_str(" Undo reverses those changes."); }
    if was_undo { summary.push_str(" Run Undo again to finish."); }
    summary
}

#[tauri::command]
fn scheduled_tasks(state: State<AppState>) -> Vec<scheduler::Summary> {
    checked_state!(state.schedules.lock(), vec![]).jobs.iter().map(scheduler::Job::summary).collect()
}

#[tauri::command]
fn scheduler_notice(state: State<AppState>) -> Option<String> {
    scheduler_notice_for(&state)
}

fn scheduler_notice_for(state: &AppState) -> Option<String> {
    let mut notices = match state.history.try_lock() {
        Ok(history) => history.notices.clone(),
        // Busy history: skip its notices this poll; they return on the next one.
        Err(std::sync::TryLockError::WouldBlock) => vec![],
        Err(_) => vec!["History unavailable. Restart Voxi.".into()],
    };
    if let Some(notice) = checked_state!(state.schedules.lock(), None).notice() { notices.push(notice); }
    (!notices.is_empty()).then(|| notices.join("\n"))
}

#[tauri::command]
fn recovery_status(state: State<AppState>) -> Result<Option<operation_journal::RecoveryStatus>, String> {
    recovery_status_for(&state)
}

fn recovery_status_for(state: &AppState) -> Result<Option<operation_journal::RecoveryStatus>, String> {
    let _execution = match state.execution.try_lock() {
        Ok(guard) => guard,
        Err(std::sync::TryLockError::WouldBlock) => return Ok(None),
        Err(_) => return Err("Execution state unavailable. Restart Voxi.".into()),
    };
    operation_journal::status(&state.reports_path.with_file_name("operation-journal.json"))
}

#[tauri::command(async)]
fn acknowledge_recovery(state: State<AppState>) -> Result<(), String> {
    let _execution = state.execution.lock().map_err(|_| "Execution state unavailable. Restart Voxi.")?;
    let mut history = state.history.lock().map_err(|_| "History unavailable. Restart Voxi.")?;
    operation_journal::acknowledge(&history.journal_path(), &mut history)
}

#[tauri::command]
fn cancel_scheduled_task(state: State<AppState>, id: u64) -> Result<(), String> {
    checked_state!(state.schedules.lock(), Err("Voxi state is unavailable. Restart the app.".into())).cancel(id)
}

#[tauri::command]
fn pause_scheduled_task(state: State<AppState>, id: u64) -> Result<(), String> {
    checked_state!(state.schedules.lock(), Err("Voxi state is unavailable. Restart the app.".into())).pause(id)
}

#[tauri::command]
fn resume_scheduled_task(state: State<AppState>, id: u64) -> Result<(), String> {
    checked_state!(state.schedules.lock(), Err("Voxi state is unavailable. Restart the app.".into())).resume(id, chrono::Local::now())
}

/// Splits a schedule off the end of a command. A repeat phrase counts only
/// when what is left is a command; “rename a.pdf to Daily” keeps its name.
/// `raw` is the single `extract_schedule` reading already taken for `now`.
fn schedule_of(state: &AppState, text: &str, now: chrono::DateTime<chrono::Local>, raw: &Result<(String, Option<scheduler::When>), String>) -> Result<(String, Option<scheduler::When>), String> {
    parse::check_length(text).map_err(|_| "That command is too long.".to_string())?;
    let (command, when) = raw.clone()?;
    if when.as_ref().is_some_and(|w| w.rule.is_some()) {
        let settings = checked_state!(state.settings.lock(), Err("Settings unavailable. Restart Voxi.".into()));
        let known = |t: &str| parse::parse(t).is_ok() || settings.find_shortcut(t).is_some();
        if !known(&command) && known(text.trim()) {
            scheduler::check_unscheduled(text)?;
            return scheduler::extract(text, now).map(|(c, due)| (c, due.map(|due| scheduler::When { due, rule: None })));
        }
    }
    Ok((command, when))
}

/// Plans a repeating task's command against the files that exist now, and runs it.
fn sched_skip(job: &scheduler::Job) { diagnostics::emit(Event::new(Code::SchedulerSkip, Stage::Skipped).operation(job.id)); }

fn run_recurring(state: &AppState, job: &scheduler::Job, rule: &scheduler::Rule, is_cancelled: &dyn Fn() -> bool) -> String {
    let roots = checked_state!(state.roots.read(), "Voxi state is unavailable. Restart the app.".into()).clone();
    let shortcut = checked_state!(state.settings.lock(), "Voxi state is unavailable. Restart the app.".into()).find_shortcut(&rule.command).cloned();
    if let Err(e) = parse::check_length(&rule.command).map_err(|_| "That command is too long.".to_string()).and_then(|()| shortcut.as_ref().map_or(Ok(()), |s| settings::check_shortcut_commands(&s.commands))) { sched_skip(job); return e; }
    let planned = {
        let history = checked_state!(state.history.lock(), "Voxi state is unavailable. Restart the app.".into());
        let planner = Planner::new(&roots, &rule.choices).with_control(search::Control::default());
        // A repeating task is always re-planned from its own rule text. Jobs
        // saved before the rule-only recurring policy may still carry a
        // `model_intent`, but replaying a model plan unattended is not allowed:
        // it was never previewed at run time. Ignore any stored intent and use
        // the rule parser, skipping with the usual clear notice if the text can
        // no longer be read.
        match shortcut {
            Some(s) => match s.commands.iter().map(|c| parse::parse(c).map(|i| (c.clone(), i))).collect::<Result<Vec<_>, _>>() {
                Ok(commands) => planner.plan_shortcut(&s.phrase, &commands, &history),
                Err(bad) => { sched_skip(job); return format!("Skipped: Voxi doesn't understand “{bad}” in “{}”.", s.phrase); },
            },
            None => match parse::parse(&rule.command) {
                Ok(intent) => planner.plan(&intent, &history),
                Err(_) => { sched_skip(job); return format!("Skipped: Voxi doesn't understand “{}”.", rule.command); },
            },
        }
    };
    match planned {
        Ok(Planned::Steps { actions, undo_of: None, .. }) if actions.is_empty() => { sched_skip(job); "Nothing to change.".into() },
        Ok(Planned::Steps { actions, undo_of: None, .. }) => {
            if let Err(message) = scheduler::check_actions(&actions) { sched_skip(job); return format!("Skipped: {message}"); }
            let result = execute_pending(state, Pending { id: 0, command: job.command.clone(), actions,
                undo_of: None, context_generation: None, context_snapshot: None, scheduled: None, stop_requested: false }, is_cancelled);
            match result.error { Some(error) => { diagnostics::emit(Event::new(Code::SchedulerRun, Stage::Failed).operation(job.id)); format!("{} {error}", result.summary) }, None => result.summary }
        }
        Ok(_) => { sched_skip(job); "Skipped: this command no longer changes files.".into() },
        Err(Stop::Problem(message)) => { sched_skip(job); format!("Nothing to change. {message}") },
        Err(Stop::Ambiguous { name, .. }) => { sched_skip(job); format!("Skipped: “{name}” matches more than one item. Schedule it again and pick one.") },
    }
}

fn run_due_tasks(state: &AppState) -> Result<(), String> {
    let result = run_due_tasks_inner(state);
    if result.is_err() { diagnostics::emit(Event::new(Code::SchedulerRun, Stage::Failed)); }
    result
}

fn run_due_tasks_inner(state: &AppState) -> Result<(), String> {
    let _execution = checked_state!(state.execution.lock(), Err("Voxi state is unavailable. Restart the app.".into()));
    // Check under the execution lock, before persisting any job claim.
    if operation_journal::status(&state.reports_path.with_file_name("operation-journal.json"))?.is_some() { return Ok(()); }
    let (job, run) = {
        let mut store = checked_state!(state.schedules.lock(), Err("Voxi state is unavailable. Restart the app.".into()));
        let Some(job) = store.claim(chrono::Local::now())? else { return Ok(()); };
        let run = ActiveRun::register(&state.active_runs, RunKey::Scheduled(job.id, job.due), false)?;
        (job, run)
    };
    let is_cancelled = || run.is_stopped();
    if let Some(rule) = job.repeat.clone() {
        let summary = run_recurring(state, &job, &rule, &is_cancelled);
        return checked_state!(state.schedules.lock(), Err("Voxi state is unavailable. Restart the app.".into())).finish_recurring(job.id, summary, chrono::Local::now());
    }
    let result = match job.validate() {
        Err(message) => ("failed", message),
        Ok(()) => {
            let result = execute_pending(state, Pending { id: 0, command: job.command.clone(), actions: job.actions,
                undo_of: None, context_generation: None, context_snapshot: None, scheduled: None, stop_requested: false }, &is_cancelled);
            (if result.error.is_some() { "failed" } else if result.cancelled { "cancelled" } else { "completed" },
                result.error.map_or(result.summary.clone(), |error| format!("{} {error}", result.summary)))
        }
    };
    if result.0 == "failed" { diagnostics::emit(Event::new(Code::SchedulerRun, Stage::Failed).operation(job.id)); }
    checked_state!(state.schedules.lock(), Err("Voxi state is unavailable. Restart the app.".into())).update(job.id, result.0, result.1)
}

#[tauri::command]
fn cancel_plan(state: State<AppState>) {
    if let Some((_, control)) = checked_state!(state.active_search.lock(), ()).take() { control.cancel(); }
    checked_state!(state.pending.lock(), ()).take();
}

#[tauri::command]
fn search_progress(state: State<AppState>, request_id: u64) -> usize {
    checked_state!(state.active_search.lock(), 0).as_ref()
        .filter(|(id, _)| *id == request_id)
        .map_or(0, |(_, control)| control.visited.load(Ordering::Relaxed))
}

/// The window is frosted glass: macOS's Popover vibrancy material on the
/// window itself, with the system's window shadow.
fn apply_glass(app: &AppHandle) {
    use tauri::window::{Effect, EffectState, EffectsBuilder};
    let Some(w) = app.get_webview_window("main") else { return };
    let effects = EffectsBuilder::new().effect(Effect::Popover).state(EffectState::Active).radius(14.0).build();
    let _ = w.set_effects(effects);
    let _ = w.set_shadow(true);
}

/// The window is the card, so it is sized to the content and macOS animates
/// the resize, keeping the top edge in place.
/// The command box changed: the menu bar face talks along for a moment.
#[tauri::command]
fn tray_typing(app: AppHandle) {
    tray_face::nudge(&app);
}

#[tauri::command]
fn fit_window(app: AppHandle, height: f64, animate: bool) {
    let Some(w) = app.get_webview_window("main") else { return };
    let height = height.clamp(48.0, 640.0);
    let handle = app.clone();
    #[cfg(target_os = "macos")]
    {
        let _ = app.run_on_main_thread(move || {
            if checked_state!(handle.state::<AppState>().presentation.lock(), ()).pending.is_some() { return; }
            let Ok(ptr) = w.ns_window() else { return };
            // SAFETY: Tauri hands back the live NSWindow, and this runs on
            // the main thread, where AppKit requires window calls.
            let ns: &objc2_app_kit::NSWindow = unsafe { &*(ptr as *const objc2_app_kit::NSWindow) };
            let mut frame = ns.frame();
            if (frame.size.height - height).abs() < 0.5 {
                return;
            }
            let top = frame.origin.y + frame.size.height;
            frame.size = objc2_foundation::NSSize::new(WINDOW_WIDTH, height);
            frame.origin.y = top - height;
            ns.setFrame_display_animate(frame, true, animate && ns.isVisible());
        });
    }
    #[cfg(not(target_os = "macos"))]
    let _ = w.set_size(tauri::LogicalSize::new(WINDOW_WIDTH, height));
}

/// The listener is installed before the frontend advertises readiness.
#[tauri::command]
fn frontend_ready(app: AppHandle, state: State<AppState>) {
    let mut presentation = checked_state!(state.presentation.lock(), ());
    presentation.ready = true;
    if let Some(request) = &presentation.pending { let _ = app.emit("voxi://shown", request.clone()); }
}

#[tauri::command]
fn present_window(app: AppHandle, request_id: u64, height: f64, animate: bool) {
    if !height.is_finite() { return; }
    let Some(w) = app.get_webview_window("main") else { return };
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        if !checked_state!(handle.state::<AppState>().presentation.lock(), ()).acknowledge(request_id) { return; }
        #[cfg(target_os = "macos")]
        {
            if let Ok(ptr) = w.ns_window() {
                let ns: &objc2_app_kit::NSWindow = unsafe { &*(ptr as *const objc2_app_kit::NSWindow) };
                if !ns.isVisible() { ns.setAlphaValue(if animate { 0.0 } else { 1.0 }); }
                let mut frame = ns.frame();
                let top = frame.origin.y + frame.size.height;
                frame.size = objc2_foundation::NSSize::new(WINDOW_WIDTH, height.clamp(48.0, 640.0));
                frame.origin.y = top - frame.size.height;
                ns.setFrame_display_animate(frame, true, animate && ns.isVisible());
            }
            let _ = handle.show();
        }
        #[cfg(not(target_os = "macos"))]
        let _ = w.set_size(tauri::LogicalSize::new(WINDOW_WIDTH, height.clamp(48.0, 640.0)));
        let _ = w.show();
        let _ = w.set_focus();
        #[cfg(target_os = "macos")]
        if let Ok(ptr) = w.ns_window() {
            let ns: &objc2_app_kit::NSWindow = unsafe { &*(ptr as *const objc2_app_kit::NSWindow) };
            fade_window(ns, true, animate);
        }
        let _ = w.emit("voxi://presented", request_id);
    });
}

#[tauri::command]
fn dismiss_window(app: AppHandle, request_id: u64, animate: bool) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        {
            let state = handle.state::<AppState>();
            let mut presentation = checked_state!(state.presentation.lock(), ());
            if !presentation.is_current(request_id) { return; }
            presentation.cancel();
        }
        #[cfg(target_os = "macos")]
        if let Some(w) = handle.get_webview_window("main")
            && let Ok(ptr) = w.ns_window() {
                let ns: &objc2_app_kit::NSWindow = unsafe { &*(ptr as *const objc2_app_kit::NSWindow) };
                fade_window(ns, false, animate);
            }
        #[cfg(not(target_os = "macos"))]
        let _ = animate;
    });
}

#[tauri::command]
fn hide_window(app: AppHandle, request_id: u64) {
    if !checked_state!(app.state::<AppState>().presentation.lock(), ()).is_current(request_id) { return; }
    hide(&app);
}

#[tauri::command]
fn get_settings(app: AppHandle, state: State<AppState>) -> Result<Settings, String> {
    let mut settings = checked_state!(state.settings.lock(), Err("Settings unavailable. Restart Voxi.".into())).clone();
    settings.start_at_login = match app.autolaunch().is_enabled() {
        Ok(enabled) => enabled,
        Err(_) => {
            diagnostics::emit(Event::new(Code::LoginItem, Stage::Failed));
            settings.start_at_login
        }
    };
    Ok(settings)
}

#[tauri::command]
fn save_settings(app: AppHandle, state: State<AppState>, mut settings: Settings) -> Result<Settings, String> {
    settings.validate_shortcuts()?;
    settings.validate_nicknames(&home())?;
    let mut current = checked_state!(state.settings.lock(), Err("Settings unavailable. Restart Voxi.".into()));
    let mut roots = checked_state!(state.roots.write(), Err("Folders unavailable. Restart Voxi.".into()));
    let mut context = checked_state!(state.result_scope.lock(), Err("Results unavailable. Restart Voxi.".into()));
    let old = current.clone();
    let al = app.autolaunch();
    let os_before = al.is_enabled().map_err(|e| { diagnostics::emit(Event::new(Code::LoginItem, Stage::Failed)); format!("Cannot read login registration: {e}") })?;
    if settings.start_at_login != os_before && !owns_login_item() {
        diagnostics::emit(Event::new(Code::LoginItem, Stage::Failed));
        return Err("Move Voxi to /Applications or ~/Applications and open that copy before changing Start at Login.".into());
    }
    if settings.hotkey != old.hotkey
        && let Err(e) = register_hotkey(&app, &settings.hotkey) {
        let _ = register_hotkey(&app, &old.hotkey);
        return Err(e);
    }
    let result = settings::login_transaction(&settings, &state.settings_path, os_before,
        |enabled| {
            let result = if enabled { al.enable() } else { al.disable() };
            result.map_err(|e| format!("Cannot change login registration: {e}. Check System Settings → General → Login Items."))?;
            if al.is_enabled().map_err(|e| e.to_string())? != enabled { return Err("macOS did not accept the login registration. Check Login Items in System Settings.".into()); }
            Ok(())
        });
    if let Err(e) = result {
        diagnostics::emit(Event::new(Code::LoginItem, Stage::Failed));
        if settings.hotkey != old.hotkey { let _ = register_hotkey(&app, &old.hotkey); }
        return Err(e);
    }
    *roots = build_roots(&settings);
    *current = settings.clone();
    context.clear();
    Ok(settings)
}

// ---- Learning ----------------------------------------------------------------

/// Saves a "this was wrong" report on this Mac. Nothing is sent anywhere.
#[tauri::command]
fn report_wrong(state: State<AppState>, command: String, outcome: serde_json::Value, note: String) -> Result<learn::Reports, String> {
    learn::record(&state.reports_path, &command, outcome, &note)
}

#[tauri::command]
fn reports_summary(state: State<AppState>) -> learn::Reports {
    learn::summary(&state.reports_path)
}

#[tauri::command]
fn clear_reports(state: State<AppState>) -> Result<learn::Reports, String> {
    learn::clear(&state.reports_path)
}

/// Shows the report file in Finder; only this one fixed file.
#[tauri::command]
fn reveal_reports(state: State<AppState>) -> Result<(), String> {
    let path = &state.reports_path;
    if !path.exists() {
        return Err("No reports yet.".into());
    }
    Command::new("/usr/bin/open").arg("-R").arg(path).status().map_err(|e| format!("Couldn't open Finder: {e}"))?;
    Ok(())
}

/// Teaches a phrase Voxi missed to run a command it then ran.
#[tauri::command]
fn teach_phrase(app: AppHandle, state: State<AppState>, phrase: String, command: String) -> Result<Settings, String> {
    let current = checked_state!(state.settings.lock(), Err("Voxi state is unavailable. Restart the app.".into())).clone();
    let next = learn::teach_phrase(&current, &phrase, &command)?;
    save_settings(app, state, next)
}

/// Remembers a folder the user picked as their name for it. Only folders
/// Voxi may already use can get a nickname.
#[tauri::command]
fn remember_folder(app: AppHandle, state: State<AppState>, name: String, path: String) -> Result<Settings, String> {
    let folder = PathBuf::from(&path);
    if !folder.is_dir() || !checked_state!(state.roots.read(), Err("Voxi state is unavailable. Restart the app.".into())).is_allowed(&folder) {
        return Err("Voxi can't use that folder.".into());
    }
    let current = checked_state!(state.settings.lock(), Err("Voxi state is unavailable. Restart the app.".into())).clone();
    let next = learn::add_nickname(&current, &name, &folder, &home())?;
    save_settings(app, state, next)
}

/// A fixed destination; the frontend cannot ask this command to open arbitrary URLs.
#[tauri::command(async)]
fn open_hex() -> Result<(), String> {
    Command::new("/usr/bin/open").arg("https://anoma.ly/hex/").status()
        .map_err(|e| format!("Couldn't open Hex: {e}"))?
        .success().then_some(()).ok_or_else(|| "Couldn't open the Hex website.".into())
}

/// macOS opens whichever copy of Voxi last turned on "Open at login". Only an
/// installed app may: a development build opened at login shows an empty
/// window, since the page it loads comes from a dev server that isn't running.
fn owns_login_item() -> bool {
    std::env::current_exe().is_ok_and(|p| {
        let installed = p.starts_with("/Applications") || p.starts_with(home().join("Applications"));
        installed && p.to_string_lossy().contains(".app/Contents/MacOS/")
            && !p.to_string_lossy().contains("/AppTranslocation/")
    })
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct FolderAccess {
    name: String,
    path: String,
    allowed: bool,
}

/// Touches each folder macOS protects, so its "allow access" prompt appears
/// now, on the permissions page, rather than in the middle of a command.
/// Reading a folder is also how to learn whether access was granted.
#[tauri::command(async)]
fn request_folder_access(state: State<AppState>) -> Vec<FolderAccess> {
    let roots = checked_state!(state.roots.read(), vec![]).clone();
    roots
        .search_roots()
        .into_iter()
        .map(|p| FolderAccess { name: file_name(&p), path: roots.display(&p), allowed: std::fs::read_dir(&p).is_ok() })
        .collect()
}

/// Opens System Settings at Privacy & Security, Files and Folders.
#[tauri::command]
fn open_privacy_settings() -> Result<(), String> {
    let status = Command::new("/usr/bin/open").arg("x-apple.systempreferences:com.apple.preference.security?Privacy_FilesAndFolders").status().map_err(|e| format!("Couldn't open System Settings: {e}"))?;
    if status.success() { Ok(()) } else { Err("Couldn't open System Settings. Open Privacy & Security → Files and Folders manually.".into()) }
}

/// The folders Voxi may change, for the settings and welcome views.
#[tauri::command]
fn allowed_folders(state: State<AppState>) -> Vec<String> {
    let roots = checked_state!(state.roots.read(), vec![]);
    let mut v: Vec<String> = roots.search_roots().iter().map(|p| roots.display(p)).collect();
    v.dedup();
    v
}

// ---- App ---------------------------------------------------------------------

#[tauri::command]
fn updater_failed() { diagnostics::emit(Event::new(Code::Update, Stage::Failed)); }

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let context = tauri::generate_context!();
    if let Some(data) = dirs::data_dir() { diagnostics::init(data.join(&context.config().identifier).join("diagnostics")); }
    diagnostics::emit(Event::new(Code::Startup, Stage::Begin));
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, None))
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    if event.state() == ShortcutState::Pressed {
                        toggle_window(app);
                    }
                })
                .build(),
        )
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            if dirs::home_dir().is_none() { return Err("Cannot locate your home folder. Check your macOS account and restart Voxi.".into()); }
            let config_dir = app.path().app_config_dir()?;
            let data_dir = app.path().app_data_dir()?;
            let settings_path = config_dir.join("settings.json");
            let (settings, settings_warning) = Settings::load_with_warning(&settings_path);
            let (mut history, history_warning) = History::load_with_warning(data_dir.join("history.json"));
            let mut storage_notices: Vec<String> = [settings_warning, history_warning].into_iter().flatten().map(|w| match w.backup { Some(path) => format!("{} Backup: {}", w.message, path.display()), None => w.message }).collect();
            if let Err(e) = operation_journal::recover(&history.journal_path(), &mut history)
                && !matches!(operation_journal::status(&history.journal_path()), Ok(Some(_))) { storage_notices.push(e); }
            match app.autolaunch().is_enabled() {
                Ok(enabled) if enabled != settings.start_at_login => storage_notices.push("macOS login registration differs from the saved preference. Review Start at Login in Settings.".into()),
                Err(e) => { diagnostics::emit(Event::new(Code::LoginItem, Stage::Failed)); storage_notices.push(format!("Cannot read macOS login registration: {e}. Check System Settings → General → Login Items.")); },
                _ => {}
            }
            history.notices = storage_notices;
            let state = AppState {
                roots: RwLock::new(build_roots(&settings)),
                history: Mutex::new(history),
                pending: Mutex::new(None),
                settings: Mutex::new(settings.clone()),
                schedules: Mutex::new(scheduler::Store::load(data_dir.join("schedules.json"))),
                execution: Mutex::new(()),
                active_runs: Mutex::new(HashMap::new()),
                settings_path,
                reports_path: learn::reports_path(&data_dir),
                result_scope: Mutex::new(scope::State::default()),
                next_plan: AtomicU64::new(1),
                active_search: Mutex::new(None),
                presentation: Mutex::new(presentation::Presentation::default()),
            };
            app.manage(state);
            #[cfg(feature = "tiny-model")]
            if plan_model::enabled() { eprintln!("Voxi: structured model mode enabled ({})", voxi_tiny_native::PLAN_PACKAGE_ID); }
            let scheduler_app = app.handle().clone();
            std::thread::spawn(move || loop {
                let _ = run_due_tasks(&scheduler_app.state::<AppState>());
                std::thread::sleep(std::time::Duration::from_secs(1));
            });

            // Configure Writing Tools before WKWebView copies its configuration.
            // The window is created here rather than automatically from config.
            let window = tauri::WebviewWindowBuilder::from_config(app, &app.config().app.windows[0])?;
            #[cfg(target_os = "macos")]
            let window = {
                let mtm = objc2::MainThreadMarker::new().ok_or("Window setup requires the main thread")?;
                let configuration = unsafe { objc2_web_kit::WKWebViewConfiguration::new(mtm) };
                // The public selector is available starting with macOS 15.
                let supported: bool = unsafe { objc2::msg_send![&*configuration,
                    respondsToSelector: objc2::sel!(setWritingToolsBehavior:)] };
                if supported {
                    unsafe { let _: () = objc2::msg_send![&*configuration,
                        setWritingToolsBehavior: objc2_app_kit::NSWritingToolsBehavior::None]; }
                }
                window.with_webview_configuration(configuration)
            };
            window.build()?;
            #[cfg(target_os = "macos")]
            if let Err(e) = outside_click::install(app.handle()) {
                eprintln!("Voxi: could not install the outside-click monitor: {e}");
                diagnostics::emit(Event::new(Code::Startup, Stage::Failed));
            }

            let handle = app.handle().clone();
            if register_hotkey(&handle, &settings.hotkey).is_err() {
                diagnostics::emit(Event::new(Code::Startup, Stage::Failed));
            }

            let show = MenuItem::with_id(app, "show", "Show Voxi", true, None::<&str>)?;
            let prefs = MenuItem::with_id(app, "settings", "Settings…", true, Some("CmdOrCtrl+,"))?;
            let quit = MenuItem::with_id(app, "quit", "Quit Voxi", true, Some("CmdOrCtrl+Q"))?;
            let menu = Menu::with_items(app, &[&show, &prefs, &PredefinedMenuItem::separator(app)?, &quit])?;
            TrayIconBuilder::with_id(tray_face::TRAY_ID)
                .icon(tray_face::rest_icon()?)
                .icon_as_template(true)
                .tooltip("Voxi")
                .menu(&menu)
                // Left click opens the window; the menu is on right click.
                .show_menu_on_left_click(false)
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                        let app = tray.app_handle();
                        // Clicking the menu bar takes focus from the window,
                        // so go by visibility alone here.
                        match app.get_webview_window("main") {
                            Some(w) if w.is_visible().unwrap_or(false) => request_hide(app),
                            _ => show_window(app, "command"),
                        }
                    }
                })
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "show" => show_window(app, "command"),
                    "settings" => show_window(app, "settings"),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;

            apply_glass(&handle);
            if !settings.onboarded {
                show_window(&handle, "welcome");
            } else if cfg!(feature = "tiny-model") && cfg!(debug_assertions) {
                show_window(&handle, "command");
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            updater_failed,
            plan_command,
            scheduled_tasks,
            scheduler_notice,
            recovery_status,
            acknowledge_recovery,
            cancel_scheduled_task,
            pause_scheduled_task,
            resume_scheduled_task,
            report_wrong,
            reports_summary,
            clear_reports,
            reveal_reports,
            teach_phrase,
            remember_folder,
            clear_result_scope,
            result_scope,
            confirm_plan,
            cancel_execution,
            cancel_plan,
            search_progress,
            open_hex,
            hide_window,
            dismiss_window,
            fit_window,
            tray_typing,
            frontend_ready,
            present_window,
            get_settings,
            save_settings,
            allowed_folders,
            request_folder_access,
            open_privacy_settings
        ])
        .build(context);
    let app = match app { Ok(app) => app, Err(_) => { diagnostics::emit(Event::new(Code::Startup, Stage::Failed)); return; } };
    app.run(|_, event| {
            #[cfg(target_os = "macos")]
            if matches!(event, tauri::RunEvent::Exit) { outside_click::uninstall(); }
        });
}

#[cfg(test)]
mod result_context_tests {
    use super::*;
    use std::{cell::Cell, fs};
    struct Fixture { _temp: tempfile::TempDir, home: PathBuf, state: AppState, request: Cell<u64> }
    impl Fixture {
        fn new() -> Self {
            let temp = tempfile::tempdir().unwrap();
            let home = fsx::canonical(temp.path());
            for folder in ["Downloads", "Desktop", "Documents", "Pictures"] { fs::create_dir(home.join(folder)).unwrap(); }
            let state = AppState {
                roots: RwLock::new(Roots::new(&home, None, &[])), history: Mutex::new(History::load(home.join("history.json"))),
                pending: Mutex::new(None), settings: Mutex::new(Settings::default()), settings_path: home.join("settings.json"), reports_path: home.join("reports.jsonl"),
                next_plan: AtomicU64::new(1), active_search: Mutex::new(None), presentation: Mutex::new(Default::default()),
                result_scope: Mutex::new(Default::default()),
                schedules: Mutex::new(scheduler::Store::load(home.join("schedules.json"))), execution: Mutex::new(()), active_runs: Mutex::new(HashMap::new()),
            };
            Self { _temp: temp, home, state, request: Cell::new(0) }
        }
        fn write(&self, path: &str) -> PathBuf {
            let path = self.home.join(path); fs::create_dir_all(path.parent().unwrap()).unwrap(); fs::write(&path, b"data").unwrap(); path
        }
        fn command(&self, text: &str) -> Outcome { self.request.set(self.request.get()+1); plan_input(&self.state, text.into(), HashMap::new(), self.request.get()) }
        fn paths(&self) -> Vec<PathBuf> { self.state.result_scope.lock().unwrap().current.as_ref().unwrap().paths() }
        fn actions(&self) -> Vec<Action> { self.state.pending.lock().unwrap().as_ref().unwrap().actions.clone() }
        fn list(&self) { assert!(matches!(self.command("list the items in Downloads"), Outcome::Info { .. })); }
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
    fn recovery_and_notice_do_not_wait_for_history_lock() {
        let f = Fixture::new();
        let _history = f.state.history.lock().unwrap();
        assert!(scheduler_notice_for(&f.state).is_none());
        assert!(recovery_status_for(&f.state).unwrap().is_none());
    }

    #[test]
    fn stopping_between_moves_persists_only_completed_changes_and_resolves_journal() {
        let f = Fixture::new();
        let a = f.write("Downloads/a.pdf");
        let b = f.write("Downloads/b.pdf");
        let moved_a = f.home.join("Documents/a.pdf");
        let moved_b = f.home.join("Documents/b.pdf");
        let actions = vec![
            Action::Move { from: a.clone(), to: moved_a.clone() },
            Action::Move { from: b.clone(), to: moved_b.clone() },
        ];
        let key = RunKey::Plan(1);
        let run = ActiveRun::register(&f.state.active_runs, key, false).unwrap();
        let result = execute_pending(&f.state, Pending {
            id: 1, command: "move PDFs".into(), actions: actions.clone(), undo_of: None,
            context_generation: None, context_snapshot: None, scheduled: None, stop_requested: false,
        }, &|| {
            if moved_a.exists() { request_stop(&f.state, key).unwrap(); }
            run.is_stopped()
        });
        drop(run);
        assert!(result.cancelled && result.error.is_none() && result.undoable);
        assert_eq!(result.summary, "Stopped. Moved 1 of 2 items. Undo reverses those changes.");
        assert!(!a.exists() && moved_a.exists() && b.exists() && !moved_b.exists());

        let mut saved = History::load(f.home.join("history.json"));
        let entry = saved.last_undoable().unwrap();
        assert_eq!(entry.done, actions[..1]);
        assert_eq!(entry.inverse, vec![Action::Move { from: moved_a.clone(), to: a.clone() }]);
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
        assert!(recovery_status_for(&f.state).unwrap().is_none());
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
        f.state.schedules.lock().unwrap().add(scheduler::Job::recurring(command.into(), due, rule)).unwrap();
        let state = &f.state;
        std::thread::scope(|scope| {
            // The job registers the occurrence before trying to acquire history.
            let history = state.history.lock().unwrap();
            let worker = scope.spawn(|| run_due_tasks_inner(state));
            let start = std::time::Instant::now();
            loop {
                if state.schedules.lock().unwrap().jobs[0].status == "running" { break; }
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
        assert_eq!(job.status, "pending");
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
        let job = scheduler::Job::new("create scheduled".into(), chrono::Local::now() - chrono::Duration::minutes(1),
            vec![Action::CreateFolder { path: output.clone() }]).unwrap();
        f.state.schedules.lock().unwrap().add(job).unwrap();
        let journal = f.home.join("operation-journal.json");
        fs::write(&journal, b"broken journal").unwrap();
        for _ in 0..2 { run_due_tasks_inner(&f.state).unwrap(); }
        assert_eq!(f.state.schedules.lock().unwrap().jobs[0].status, "pending");
        assert!(!output.exists());
        operation_journal::acknowledge(&journal, &mut f.state.history.lock().unwrap()).unwrap();
        run_due_tasks_inner(&f.state).unwrap();
        assert_eq!(f.state.schedules.lock().unwrap().jobs[0].status, "completed");
        assert!(output.exists());
    }

    #[test]
    fn register_equivalence_binary() {
        let binary = std::env::current_exe().unwrap();
        let variant = if cfg!(feature = "tiny-model") { "tiny-model" } else { "default" };
        fs::write(binary.parent().unwrap().join(format!("voxi-equivalence-{variant}.json")),
            serde_json::to_vec(&binary).unwrap()).unwrap();
    }

    /// Invoked in separate binaries by the cross-feature integration test.
    #[test]
    fn rule_only_probe() {
        let Ok(base) = std::env::var("VOXI_EQUIVALENCE_HOME") else { return };
        let home = fsx::canonical(std::path::Path::new(&base));
        for folder in ["Downloads", "Desktop", "Documents", "Pictures"] { fs::create_dir_all(home.join(folder)).unwrap(); }
        for path in ["Downloads/a.pdf", "Downloads/one/same.txt", "Downloads/two/same.txt"] {
            let path = home.join(path); fs::create_dir_all(path.parent().unwrap()).unwrap(); fs::write(path, b"fixture").unwrap();
        }
        let state = AppState {
            roots: RwLock::new(Roots::new(&home, None, &[])), history: Mutex::new(History::load(home.join("history.json"))),
            pending: Mutex::new(None), settings: Mutex::new(Settings::default()), settings_path: home.join("settings.json"), reports_path: home.join("reports.jsonl"),
            next_plan: AtomicU64::new(1), active_search: Mutex::new(None), presentation: Mutex::new(Default::default()),
            result_scope: Mutex::new(Default::default()), schedules: Mutex::new(scheduler::Store::load(home.join("schedules.json"))), execution: Mutex::new(()), active_runs: Mutex::new(HashMap::new()),
        };
        let oversized = "x".repeat(parse::MAX_COMMAND_BYTES + 1);
        let mut outcomes = Vec::new();
        for text in ["delete everything on my computer", "", "!!!", "launch the moon", &oversized,
            "move a.pdf from Downloads to Documents in 5 minutes", "delete same.txt",
            "list PDFs in Downloads", "delete the first file"] {
            outcomes.push(serde_json::to_value(plan_input(&state, text.into(), HashMap::new(), 1)).unwrap());
        }
        let actions = vec![Action::CreateFolder { path: home.join("Documents/created") },
            Action::Move { from: home.join("Downloads/missing"), to: home.join("Documents/missing") }];
        outcomes.push(serde_json::to_value(execute_pending(&state, Pending { id: 1, command: "partial failure".into(), actions,
            undo_of: None, context_generation: None, context_snapshot: None, scheduled: None, stop_requested: false }, &|| false)).unwrap());
        outcomes.push(serde_json::json!({
            "source": fs::read(home.join("Downloads/a.pdf")).unwrap(),
            "destinationExists": home.join("Documents/a.pdf").exists(),
            "createdExists": home.join("Documents/created").exists(),
            "missingExists": home.join("Documents/missing").exists(),
            "history": state.history.lock().unwrap().entries,
            "journal": serde_json::from_slice::<serde_json::Value>(&fs::read(home.join("operation-journal.json")).unwrap()).unwrap()
        }));
        fs::write(home.join("outcomes.json"), serde_json::to_vec_pretty(&outcomes).unwrap()).unwrap();
    }

    #[test]
    fn shortcut_with_one_read_only_command_runs_it_as_typed() {
        let f = Fixture::new();
        f.write("Downloads/a.pdf");
        f.state.settings.lock().unwrap().shortcuts = vec![settings::Shortcut { phrase: "White".into(), commands: vec!["list downloads".into()] }];
        assert!(matches!(f.command("white"), Outcome::Info { ref title, .. } if title.contains("a.pdf")));
        // It publishes a list like the typed command, so follow-ups work.
        assert!(matches!(f.command("delete the first one"), Outcome::Plan { .. }));
    }

    #[test]
    fn partial_failure_keeps_durable_undo_and_allows_next_command() {
        let f = Fixture::new();
        let created = f.home.join("Documents/created");
        let result = execute_pending(&f.state, Pending { id: 1, command: "partial".into(),
            actions: vec![Action::CreateFolder { path: created.clone() }, Action::Move {
                from: f.home.join("Downloads/missing"), to: f.home.join("Documents/missing") }],
            undo_of: None, context_generation: None, context_snapshot: None, scheduled: None, stop_requested: false }, &|| false);
        assert_eq!(result.done, 1); assert!(result.error.is_some()); assert!(result.undoable);
        assert_eq!(History::load(f.home.join("history.json")).entries[0].done.len(), 1);
        let Outcome::Plan { id, .. } = f.command("undo") else { panic!("expected undo plan") };
        let undo = confirm_input(&f.state, id).unwrap();
        assert!(undo.error.is_none()); assert!(!undo.undoable); assert!(!created.exists());
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
        assert_eq!(history.entries[0].inverse.len(), 1);
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
        f.state.settings.lock().unwrap().shortcuts.push(settings::Shortcut {
            phrase: "run oversized".into(), commands: vec!["delete a.pdf".into(); 9],
        });
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
            let originals: Vec<_> = ["Downloads/a.pdf", "Downloads/one.zip", "Downloads/tax-return-2025.pdf",
                "Downloads/w2-form-2026.pdf", "Desktop/x.pdf", "Desktop/a.pdf", "Desktop/b.pdf", "Desktop/todo.txt"]
                .iter().map(|p| f.write(p)).collect();
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
            let single: Vec<_> = f.actions().into_iter().filter_map(|a| if let Action::Move { from, .. } = a { Some(from) } else { None }).collect();
            assert!(matches!(f.command(&format!("make a folder called Music in Documents and move all {kind} there")), Outcome::Plan { .. }));
            let chained: Vec<_> = f.actions().into_iter().filter_map(|a| if let Action::Move { from, .. } = a { Some(from) } else { None }).collect();
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
        let Outcome::Plan { id, total, .. } = f.command("create a folder in downloads called PDFS and move pdfs into it") else { panic!("expected chain preview") };
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
        let Outcome::Plan { id, total, .. } = f.command("copy PDFs from Downloads to Documents then rename notes.txt in Desktop to saved.txt") else { panic!("expected independent chain") };
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
        let Outcome::Plan { id, scheduled_for: Some(_), .. } = outcome else { panic!("{outcome:?}"); };
        assert!(source.exists());
        let result = confirm_input(&f.state, id).unwrap();
        assert_eq!(result.done, 0); assert!(!result.undoable); assert!(source.exists());
        assert!(f.state.history.lock().unwrap().entries.is_empty());
        // Reload from disk, then make the saved task due without waiting.
        let mut store = scheduler::Store::load(f.home.join("schedules.json"));
        store.jobs[0].due = chrono::Local::now() - chrono::Duration::seconds(1);
        *f.state.schedules.lock().unwrap() = store;
        run_due_tasks(&f.state).unwrap();
        assert!(!source.exists());
        assert_eq!(f.state.schedules.lock().unwrap().jobs[0].status, "completed");
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
        let Outcome::Plan { id, total, warnings, scheduled_for: Some(_), repeats: Some(_), .. } = outcome else { panic!("{outcome:?}") };
        assert_eq!(total, 2);
        assert!(!warnings.iter().any(|w| w.starts_with("Model interpretation:")), "{warnings:?}");
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
        assert_eq!(scheduler::Store::load(f.home.join("schedules.json")).jobs[0].status, "pending");
    }

    #[test]
    fn recurring_confirm_uses_only_approved_files_and_retains_partial_failure() {
        for fail_first in [false, true] {
            let f = Fixture::new();
            let source = f.write("Downloads/a.pdf");
            let Outcome::Plan { id, .. } = f.command("create a folder in downloads called PDFS and move pdfs into it everyday") else { panic!("expected preview") };
            let arrived_after_preview = f.write("Downloads/new.pdf");
            if fail_first { fs::remove_file(&source).unwrap(); }
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
            assert_eq!(store.jobs[0].status, "pending");
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
    fn repeating_move_into_a_new_folder_makes_the_folder() {
        let f = Fixture::new();
        let pdf = f.write("Downloads/bill.pdf");
        let outcome = f.command("move every pdf in downloads to a new folder called PDFS every 2 days");
        let Outcome::Plan { id, repeats: Some(repeats), .. } = outcome else { panic!("{outcome:?}"); };
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
        let Outcome::Plan { id, scheduled_for: Some(_), repeats: Some(repeats), .. } = outcome else { panic!("{outcome:?}"); };
        assert!(repeats.starts_with("Runs now, then every 3 days"), "{repeats}");
        let result = confirm_input(&f.state, id).unwrap();
        assert!(result.summary.starts_with("Done. 1 change. Repeats every 3 days"), "{}", result.summary);
        assert!(!first.exists());
        let due = |f: &Fixture| f.state.schedules.lock().unwrap().jobs[0].due;
        let make_due = |f: &Fixture| f.state.schedules.lock().unwrap().jobs[0].due = chrono::Local::now() - chrono::Duration::days(7);
        // Confirm ran the file that existed in the preview.
        assert!(!first.exists());
        let job = f.state.schedules.lock().unwrap().jobs[0].clone();
        assert_eq!(job.status, "pending");
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
        assert_eq!(f.state.schedules.lock().unwrap().jobs[0].status, "pending");
        assert!(f.state.schedules.lock().unwrap().jobs[0].summary.contains("Nothing to change"));
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
        assert_eq!(f.state.schedules.lock().unwrap().jobs[0].status, "pending");
        assert!(due(&f) > chrono::Local::now());
        let Outcome::Plan { id, .. } = f.command("move c.dmg from downloads to documents in 5 minutes") else { panic!("plan"); };
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
        let model_intent = || model::Intent::CreateFolders {
            names: vec!["MODEL_PLAN".into()],
            parent: Some("Desktop".into()),
        };
        let stale_rule = |command: &str| scheduler::Rule {
            every: 1,
            unit: scheduler::Unit::Days,
            time: None,
            command: command.into(),
            choices: HashMap::new(),
            model_intent: Some(model_intent()),
        };
        // The rule text is still readable: it wins over the stored model plan.
        f.state.schedules.lock().unwrap().add(scheduler::Job::recurring(
            "create a folder in downloads called PDFS and move pdfs into it".into(),
            chrono::Local::now() - chrono::Duration::days(2),
            stale_rule("create a folder in downloads called PDFS and move pdfs into it"),
        )).unwrap();
        run_due_tasks(&f.state).unwrap();
        assert!(!pdf.exists());
        assert!(f.home.join("Downloads/PDFS/a.pdf").exists());
        assert!(!f.home.join("Desktop/MODEL_PLAN").exists(), "the stored model plan must not run");
        assert_eq!(f.state.history.lock().unwrap().entries.len(), 1);

        // Unreadable rule text skips with a clear notice instead of falling
        // back to the stored model plan.
        f.state.schedules.lock().unwrap().add(scheduler::Job::recurring(
            "unparseable legacy task".into(),
            chrono::Local::now() - chrono::Duration::days(2),
            stale_rule("this is not a parseable command"),
        )).unwrap();
        run_due_tasks(&f.state).unwrap();
        assert!(!f.home.join("Desktop/MODEL_PLAN").exists(), "a skipped job must not replay model intent");
        assert_eq!(f.state.history.lock().unwrap().entries.len(), 1);
        let jobs = f.state.schedules.lock().unwrap();
        assert!(jobs.jobs[1].summary.contains("Skipped: Voxi doesn't understand"), "{}", jobs.jobs[1].summary);
        assert_eq!(jobs.jobs[1].status, "pending");
    }

    #[test]
    fn scheduled_changed_sources_and_collisions_stop_without_overwriting() {
        for replace_source in [true, false] {
            let f = Fixture::new(); let source = f.write("Downloads/a.pdf");
            let Outcome::Plan { id, .. } = f.command("move a.pdf from Downloads to Documents in 5 minutes") else { panic!("plan"); };
            confirm_input(&f.state, id).unwrap();
            if replace_source { std::fs::write(&source, "changed source contents").unwrap(); }
            else { f.write("Documents/a.pdf"); }
            f.state.schedules.lock().unwrap().jobs[0].due = chrono::Local::now() - chrono::Duration::seconds(1);
            run_due_tasks(&f.state).unwrap();
            assert!(source.exists());
            assert_eq!(f.state.schedules.lock().unwrap().jobs[0].status, "failed");
            assert!(f.state.history.lock().unwrap().entries.is_empty());
        }
    }
    #[test]
    fn scheduled_cancellation_and_read_only_commands() {
        let f = Fixture::new(); let source = f.write("Downloads/a.pdf");
        let Outcome::Plan { id, .. } = f.command("move a.pdf from Downloads to Documents after 1 hour") else { panic!("plan"); };
        confirm_input(&f.state, id).unwrap();
        f.state.schedules.lock().unwrap().cancel(1).unwrap();
        f.state.schedules.lock().unwrap().jobs[0].due = chrono::Local::now() - chrono::Duration::seconds(1);
        run_due_tasks(&f.state).unwrap(); assert!(source.exists());
        assert!(matches!(f.command("list files in Downloads after 3 minutes"), Outcome::Problem { .. }));
        assert!(f.state.pending.lock().unwrap().is_none());
    }
    #[test]
    fn last_file_is_the_last_displayed_result_and_no_global_scan_occurs() {
        let f = Fixture::new(); f.write("Downloads/a.pdf"); f.write("Downloads/sub/b.txt"); f.write("Desktop/elsewhere.txt");
        f.list(); let paths = f.paths();
        assert!(matches!(f.command("delete the last file"), Outcome::Plan { total: 1, .. }));
        assert_eq!(f.actions(), vec![Action::Trash { path: paths.last().unwrap().clone() }]);
        assert_eq!(f.state.active_search.lock().unwrap().as_ref().unwrap().1.visited.load(Ordering::Relaxed), 0);
        assert!(paths.iter().all(|p| p.exists()));
    }
    #[test]
    fn all_and_filtered_followups_only_use_snapshot_files() {
        let f = Fixture::new(); let pdf=f.write("Downloads/a.pdf"); let txt=f.write("Downloads/sub/b.txt");
        f.write("Documents/outside.pdf"); f.list(); f.write("Downloads/new.pdf");
        assert!(matches!(f.command("delete PDFs"), Outcome::Plan { total: 1, .. }));
        assert_eq!(f.actions(), vec![Action::Trash { path: pdf.clone() }]);
        assert!(matches!(f.command("move those files to Documents"), Outcome::Plan { total: 2, .. }));
        let sources: Vec<_> = f.actions().iter().filter_map(|a| if let Action::Move{from,..}=a {Some(from.clone())} else {None}).collect();
        assert!(sources.contains(&pdf) && sources.contains(&txt));
    }
    #[test]
    fn an_empty_scoped_filter_does_not_start_a_global_search() {
        let f=Fixture::new(); f.write("Downloads/keep.txt"); f.write("Pictures/photo.jpg"); f.list();
        assert!(matches!(f.command("delete images"), Outcome::Problem{..})); assert_eq!(f.paths().len(),1);
        assert!(f.state.pending.lock().unwrap().is_none());
    }
    #[test]
    fn explicit_source_folders_and_outside_filenames_leave_scope() {
        let f=Fixture::new(); f.write("Downloads/a.pdf"); let outside=f.write("Documents/outside.pdf"); f.list();
        assert!(matches!(f.command("delete outside.pdf"), Outcome::Plan{total:1,..}));
        assert!(f.state.result_scope.lock().unwrap().current.is_none());
        assert_eq!(f.actions(),vec![Action::Trash{path:outside.clone()}]);
        f.list(); assert!(matches!(f.command("delete PDFs in Documents"), Outcome::Plan{total:1,..}));
        assert_eq!(f.actions(),vec![Action::Trash{path:outside}]); assert!(f.state.result_scope.lock().unwrap().current.is_none());
    }
    #[test]
    fn a_new_list_replaces_scope_and_filters_can_narrow_it() {
        let f=Fixture::new(); f.write("Downloads/a.pdf"); f.write("Downloads/b.txt"); let image=f.write("Pictures/photo.jpg");
        f.list(); assert!(matches!(f.command("list PDFs"),Outcome::Info{total:1,..})); assert_eq!(f.paths().len(),1);
        assert!(matches!(f.command("list images in Pictures"),Outcome::Info{total:1,..})); assert_eq!(f.paths(),vec![image]);
    }
    #[test]
    fn missing_or_replaced_files_require_a_new_list() {
        let f=Fixture::new(); let path=f.write("Downloads/a.pdf"); f.list();
        fs::rename(&path,f.home.join("Downloads/original.pdf")).unwrap(); fs::write(&path,b"replacement").unwrap();
        assert!(matches!(f.command("delete them"), Outcome::Problem{message} if message.contains("changed")));
        assert!(path.exists()); assert!(f.state.pending.lock().unwrap().is_none());
    }
    #[test]
    fn files_replaced_after_the_preview_cannot_be_confirmed() {
        let f=Fixture::new(); let path=f.write("Downloads/a.pdf"); f.list();
        assert!(matches!(f.command("delete them"), Outcome::Plan{..}));
        let pending=f.state.pending.lock().unwrap().take().unwrap();
        pending.validate_context(&f.state.result_scope.lock().unwrap()).unwrap();
        fs::rename(&path,f.home.join("Downloads/original.pdf")).unwrap();
        fs::write(&path,b"replacement").unwrap();
        assert!(pending.validate_context(&f.state.result_scope.lock().unwrap()).unwrap_err().contains("changed"));
        assert_eq!(fs::read(&path).unwrap(),b"replacement");
    }
    #[test]
    fn clearing_scope_expires_its_preview_and_references() {
        let f=Fixture::new(); f.write("Downloads/a.pdf"); f.list();
        assert!(matches!(f.command("delete them"), Outcome::Plan{..}));
        let pending=f.state.pending.lock().unwrap().take().unwrap();
        f.state.result_scope.lock().unwrap().clear();
        assert!(pending.validate_context(&f.state.result_scope.lock().unwrap()).unwrap_err().contains("expired"));
        assert!(matches!(f.command("delete the last file"),Outcome::NotUnderstood{..}));
    }
    #[test]
    fn empty_lists_stay_bounded_and_destinations_are_never_references() {
        let f=Fixture::new(); f.write("Documents/outside.pdf"); f.list();
        assert!(matches!(f.command("delete everything"),Outcome::Problem{..}));
        assert!(f.state.pending.lock().unwrap().is_none());
        f.write("Downloads/a.pdf"); f.list();
        let snapshot=f.state.result_scope.lock().unwrap().current.clone().unwrap();
        let (intent,used)=scope::prepare("rename the first file to \"last\"",Some(&snapshot)).unwrap();
        assert!(used);
        assert!(matches!(intent,model::Intent::Rename{new_name,..} if new_name=="last"));
        assert!(scope::parse_followup("move a.pdf to last",&snapshot).unwrap().is_none());
        let (intent,used)=scope::prepare("move a.pdf to \"last\"",Some(&snapshot)).unwrap();
        assert!(used);
        assert!(matches!(intent,model::Intent::Move{to,..} if to=="last"));
    }
    #[test]
    fn ambiguous_names_only_offer_current_results_and_reject_outside_choices() {
        let f=Fixture::new(); f.write("Downloads/a/same.pdf"); f.write("Downloads/b/same.pdf"); let outside=f.write("Documents/same.pdf");
        f.list(); assert!(matches!(f.command("delete same.pdf"),Outcome::Ambiguous{candidates,..} if candidates.len()==2 && !candidates.contains(&outside.to_string_lossy().to_string())));
        f.request.set(f.request.get()+1);
        let result=plan_input(&f.state,"delete same.pdf".into(),HashMap::from([("same.pdf".into(),outside.to_string_lossy().into_owned())]),f.request.get());
        assert!(matches!(result,Outcome::Problem{..}));
    }
    #[test]
    fn positions_are_limited_to_displayed_rows() {
        let f=Fixture::new(); let paths:Vec<_>=(0..201).map(|i|f.write(&format!("Downloads/{i}.txt"))).collect();
        let snapshot=scope::Snapshot::capture(paths.clone(),"Downloads".into()).unwrap();
        let (intent,used)=scope::prepare("delete the last file",Some(&snapshot)).unwrap(); assert!(used);
        assert_eq!(intent.source(),Some(&model::Source::item(&paths[199].to_string_lossy())));
        assert!(scope::prepare("delete the 201st file",Some(&snapshot)).is_err());
        assert!(scope::prepare("delete 201",Some(&snapshot)).is_err());
    }
    #[test]
    fn numbered_rows_and_spoken_numbers_target_the_displayed_order() {
        let f=Fixture::new();
        for name in ["a.pdf","b.txt","c.txt","d.pdf","e.txt","4"] { f.write(&format!("Downloads/{name}")); }
        let Outcome::Info{rows,numbered:true,..}=f.command("show items in Downloads") else { panic!("expected numbered list") };
        let paths=f.paths();
        assert_eq!(rows.iter().map(|r|r.name.clone()).collect::<Vec<_>>(),paths.iter().map(|p|file_name(p)).collect::<Vec<_>>());
        for command in ["delete 4","delete four","delete item 4","delete number four"] {
            assert!(matches!(f.command(command),Outcome::Plan{total:1,..}),"{command}");
            assert_eq!(f.actions(),vec![Action::Trash{path:paths[3].clone()}],"{command}");
        }
        assert!(matches!(f.command("Move 5 to desktop"),Outcome::Plan{total:1,..}));
        assert_eq!(f.actions(),vec![Action::Move{from:paths[4].clone(),to:f.home.join("Desktop").join(file_name(&paths[4]))}]);
        assert!(matches!(f.command("delete 1"),Outcome::Plan{total:1,..}));
        assert_eq!(f.actions(),vec![Action::Trash{path:paths[0].clone()}]);
        assert!(matches!(f.command("delete \"4\""),Outcome::Plan{total:1,..}));
        assert_eq!(f.actions(),vec![Action::Trash{path:f.home.join("Downloads/4")}]);
        for command in ["delete 0","delete 7","delete 999999999999999999999999999"] {
            assert!(matches!(f.command(command),Outcome::Problem{..}),"{command}");
            assert!(f.state.pending.lock().unwrap().is_none());
        }
        assert!(matches!(f.command("count files"),Outcome::Info{numbered:false,..}));
    }
    #[test]
    fn numeric_destinations_and_filenames_are_not_row_references() {
        let f=Fixture::new(); f.write("Downloads/4.pdf"); f.write("Downloads/a.txt"); f.list();
        let snapshot=f.state.result_scope.lock().unwrap().current.clone().unwrap();
        for command in ["move a.txt to 5","rename a.txt to 4.pdf","resize images to 400","delete 4.pdf"] {
            assert!(scope::parse_followup(command,&snapshot).unwrap().is_none(),"{command}");
        }
        let (intent,used)=scope::prepare("rename 1 to 5.pdf",Some(&snapshot)).unwrap();
        assert!(used);
        assert!(matches!(intent,model::Intent::Rename{new_name,..} if new_name=="5.pdf"));
    }
    #[test]
    fn deleted_rows_do_not_shift_later_numbers_or_promote_hidden_rows() {
        let f=Fixture::new();
        let paths:Vec<_>=(0..201).map(|i|f.write(&format!("Downloads/{i}.txt"))).collect();
        let mut snapshot=scope::Snapshot::capture(paths.clone(),"Downloads".into()).unwrap();
        fs::remove_file(&paths[3]).unwrap();
        snapshot.apply(&[Action::Trash{path:paths[3].clone()}]);
        snapshot.validate().unwrap();
        assert!(scope::prepare("delete 4",Some(&snapshot)).is_err());
        let (intent,_)=scope::prepare("move 5 to Desktop",Some(&snapshot)).unwrap();
        assert_eq!(intent.source(),Some(&model::Source::item(&paths[4].to_string_lossy())));
        assert!(scope::prepare("delete 201",Some(&snapshot)).is_err());
        assert_eq!(snapshot.summary().count,200);
        assert_eq!(snapshot.summary().visible,199);
        let (intent,_)=scope::prepare("delete the last file",Some(&snapshot)).unwrap();
        assert_eq!(intent.source(),Some(&model::Source::item(&paths[199].to_string_lossy())));
    }
    #[test]
    fn references_require_context_and_quoted_names_stay_literal() {
        let f=Fixture::new(); f.write("Downloads/last"); f.write("Downloads/notes.txt");
        assert!(matches!(f.command("delete the last file"),Outcome::NotUnderstood{..}));
        f.list(); assert!(matches!(f.command("delete \"last\""),Outcome::Plan{total:1,..}));
        assert!(matches!(f.command("delete it"),Outcome::Problem{..}));
    }
    #[test]
    fn confirmed_moves_and_trash_update_the_context_order() {
        let f=Fixture::new(); let old=f.write("Downloads/a.pdf"); let keep=f.write("Downloads/b.txt"); f.list();
        let moved=f.home.join("Documents/a.pdf"); fs::rename(&old,&moved).unwrap();
        let mut context=f.state.result_scope.lock().unwrap(); let snapshot=context.current.as_mut().unwrap();
        snapshot.apply(&[Action::Move{from:old,to:moved.clone()}]); snapshot.validate().unwrap();
        assert!(snapshot.paths().contains(&moved));
        fs::remove_file(&keep).unwrap(); snapshot.apply(&[Action::Trash{path:keep}]);
        assert_eq!(snapshot.paths(),vec![moved]);
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
        let names: Vec<String> = f.actions().iter().filter_map(|a| match a {
            Action::Rename { new_name, .. } => Some(new_name.clone()), _ => None,
        }).collect();
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
}

/// The command box suggests these; each must parse, and "every"/"at" ones must schedule.
#[cfg(test)]
mod plan_guard_tests {
    use super::*;

    #[test]
    fn preview_shows_the_source_folder_for_transfers_and_trash() {
        let temp = tempfile::tempdir().unwrap();
        let home = fsx::canonical(temp.path());
        let roots = Roots::new(&home, None, &[]);
        for (action, kind, primary, source) in [
            (
                Action::Move { from: home.join("Downloads/a.pdf"), to: home.join("Documents/a.pdf") },
                "move", "a.pdf", "Downloads",
            ),
            (
                Action::Copy { from: home.join("Pictures/b.png"), to: home.join("Backup/b.png") },
                "copy", "b.png", "Pictures",
            ),
            (
                Action::Trash { path: home.join("Desktop/c.txt") },
                "trash", "c.txt", "Desktop",
            ),
        ] {
            let (got_kind, got_primary, detail) = describe(&action, &roots);
            assert_eq!((got_kind.as_str(), got_primary.as_str()), (kind, primary));
            assert!(detail.contains(source), "{kind}: {detail}");
        }
    }

    #[cfg(feature = "tiny-model")]
    #[test]
    fn schedule_cross_check_matches_the_scheduler_and_named_weekdays() {
        use chrono::TimeZone;
        let now = chrono::Local.with_ymd_and_hms(2026, 11, 17, 12, 0, 0).single().unwrap();
        let text = "move PDFs from Downloads to Archive at 21:00";
        let (_, chosen) = scheduler::extract_schedule(text, now).unwrap();
        assert!(model_schedule_ok(&chosen, chosen.as_ref(), text));
        let wrong = chosen.clone().map(|mut w| { w.due += chrono::Duration::hours(1); w });
        assert!(!model_schedule_ok(&wrong, chosen.as_ref(), text));
        // A schedule only in the plan, or only in the text, is rejected.
        assert!(!model_schedule_ok(&chosen, None, "move PDFs from Downloads to Archive"));
        assert!(!model_schedule_ok(&None, chosen.as_ref(), text));

        let monday_text = "move PDFs to Archive every monday at 9:00";
        let (_, monday) = scheduler::extract_schedule(monday_text, now).unwrap();
        assert_eq!(named_weekday(monday_text), Some(chrono::Weekday::Mon));
        assert!(model_schedule_ok(&monday, monday.as_ref(), monday_text));
        let (_, tuesday) = scheduler::extract_schedule("move PDFs to Archive every tuesday at 9:00", now).unwrap();
        assert!(!model_schedule_ok(&tuesday, monday.as_ref(), monday_text));
        assert_eq!(named_weekday("rename monday notes.pdf to done.pdf"), Some(chrono::Weekday::Mon));
    }
}

#[cfg(test)]
mod example_tests {
    #[test]
    fn suggested_examples_parse() {
        let list: Vec<String> = serde_json::from_str(include_str!("../../src/examples.json")).unwrap();
        for ex in list {
            let (rest, when) = crate::scheduler::extract_schedule(&ex, chrono::Local::now()).unwrap();
            let timed = ex.contains(" every ") || ex.contains(" at ");
            assert_eq!(when.is_some(), timed, "{ex}: schedule");
            assert!(crate::parse::parse(&rest).is_ok(), "{ex}: doesn't parse");
        }
    }
}
