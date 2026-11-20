use settings::diagnostics;
/// Report-only audits over the parser and planner; test builds only.
#[cfg(test)]
mod audits;
mod exec;
#[cfg(any(test, feature = "tiny-model"))]
mod fallback_guard;
#[cfg(any(test, feature = "tiny-model"))]
mod fallback_slots;
mod finder_selection;
mod fsx;
mod history;
mod learn;
mod model;
#[cfg(feature = "tiny-model")]
mod model_slot;
mod operation_journal;
#[cfg(target_os = "macos")]
mod outside_click;
mod parse;
mod pdf;
mod plan;
#[cfg(feature = "tiny-model")]
mod plan_model;
mod presentation;
mod scheduler;
mod scope;
mod search;
mod settings;
mod skip;
#[cfg(feature = "tiny-model")]
mod tiny_model;
mod trash;
mod tray_face;

use diagnostics::{Code, Event, Stage};
use fsx::{Roots, file_name};
use history::{Entry, History};
use model::{Action, Outcome, PreviewItem, PreviewRow, RunResult};
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
        match $lock {
            Ok(guard) => guard,
            Err(_) => {
                eprintln!("Voxi state lock failed; restart required");
                return;
            }
        }
    };
    ($lock:expr, $failure:expr) => {
        match $lock {
            Ok(guard) => guard,
            Err(_) => {
                eprintln!("Voxi state lock failed; restart required");
                return $failure;
            }
        }
    };
}

const WINDOW_WIDTH: f64 = 680.0;
const PREVIEW_LIMIT: usize = 20;
/// How many more steps each "Show more" loads.
const PREVIEW_PAGE: usize = 200;

struct Pending {
    id: u64,
    command: String,
    actions: Vec<Action>,
    /// What describes each step on request, frozen with the plan so every
    /// page the user loads describes the same plan.
    preview: PreviewSource,
    /// Each existing source file and when it was last modified at preview,
    /// so a file edited before confirming (and perhaps no longer matching a
    /// date filter) stops the run instead of being acted on unseen.
    inputs: Vec<(PathBuf, std::time::SystemTime)>,
    undo_of: Option<u64>,
    context_generation: Option<u64>,
    context_snapshot: Option<scope::Snapshot>,
    scheduled: Option<scheduler::Job>,
    stop_requested: bool,
}
impl Pending {
    /// Fails when a source file of a step in `kept` changed since the preview.
    fn validate_inputs(&self, kept: &[Action]) -> Result<(), String> {
        let read: std::collections::HashSet<&PathBuf> = kept.iter().flat_map(sources).collect();
        match self
            .inputs
            .iter()
            .filter(|(p, _)| read.contains(p))
            .find(|(p, at)| std::fs::metadata(p).and_then(|m| m.modified()).ok() != Some(*at))
        {
            Some((p, _)) => Err(format!("“{}” changed after the preview. Press Enter to preview it again.", fsx::file_name(p))),
            None => Ok(()),
        }
    }

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

    fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::Relaxed)
    }
}

impl Drop for ActiveRun<'_> {
    fn drop(&mut self) {
        if let Ok(mut runs) = self.runs.lock() {
            runs.remove(&self.key);
        }
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
    /// What the window should say about the last Finder selection, with the
    /// results-list generation it belongs to. It shows only while that list
    /// is current; one with no generation (sent before setup) shows once.
    selection_notice: Mutex<Option<(Option<u64>, String)>>,
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
    Roots::new(&h, screenshot_location(), &settings.extra_paths(&h))
        .with_exclusions(&settings.excluded_paths(&h))
        .with_nicknames(settings.nickname_paths(&h))
}

// ---- Window ------------------------------------------------------------------

#[cfg(target_os = "macos")]
fn fade_window(ns: &objc2_app_kit::NSWindow, visible: bool, animate: bool) {
    use objc2_app_kit::{NSAnimatablePropertyContainer, NSAnimationContext};
    let alpha = if visible { 1.0 } else { 0.0 };
    if !animate {
        ns.setAlphaValue(alpha);
        return;
    }
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
        && let Ok(Some(m)) = app.monitor_from_point(cursor.x, cursor.y)
    {
        let (pos, size, scale) = (m.position(), m.size(), m.scale_factor());
        let x = pos.x + (size.width as i32 - (WINDOW_WIDTH * scale) as i32) / 2;
        let y = pos.y + (size.height as f64 * 0.22) as i32;
        let _ = w.set_position(PhysicalPosition::new(x, y));
    }
    let state = app.state::<AppState>();
    let mut presentation = checked_state!(state.presentation.lock(), ());
    let request = presentation.request(mode);
    if presentation.ready {
        let _ = w.emit("voxi://shown", request);
    }
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

/// Files handed over by "Organize with Voxi" in Finder become the results
/// list, then the window opens so the user can say what to do with them.
pub(crate) fn use_finder_selection(app: &AppHandle, selected: Vec<PathBuf>) {
    let state = app.state::<AppState>();
    let onboarded = checked_state!(state.settings.lock(), ()).onboarded;
    let notice = if onboarded {
        let (generation, notice) = take_selection(&state, selected);
        notice.map(|n| (Some(generation), n))
    } else {
        // Finishing setup rebuilds the allowed folders and clears the list.
        Some((None, "Finish setting up Voxi, then choose Organize with Voxi in Finder again.".into()))
    };
    *checked_state!(state.selection_notice.lock(), ()) = notice;
    show_window(app, "command");
}

/// Publishes the selection as the results list. Returns the list's new
/// generation and the notice to show: what was left out, or why nothing
/// could be used.
fn take_selection(state: &AppState, selected: Vec<PathBuf>) -> (u64, Option<String>) {
    let unavailable = || (0, Some("Voxi state is unavailable. Restart the app.".into()));
    let Ok(roots) = state.roots.read().map(|r| r.clone()) else { return unavailable() };
    let Ok(mut context) = state.result_scope.lock() else { return unavailable() };
    let notice = match finder_selection::accept(&roots, selected) {
        Err(message) => {
            context.clear();
            Some(message)
        }
        Ok((files, notice)) => match scope::Snapshot::capture(files, finder_selection::LABEL.into()) {
            Some(snapshot) => {
                context.publish(snapshot);
                notice
            }
            None => {
                context.clear();
                Some("The selected files changed in Finder. Select them again.".into())
            }
        },
    };
    (context.generation, notice)
}

/// What a newly opened command view starts from: the current results list,
/// its generation, and any notice about the Finder selection that made it.
/// Reading it twice gives the same answer, except a notice sent before setup.
#[derive(serde::Serialize)]
struct Opening {
    scope: Option<scope::Summary>,
    generation: u64,
    notice: Option<String>,
}

#[tauri::command]
fn opening_context(state: State<AppState>) -> Opening {
    opening(&state)
}

fn opening(state: &AppState) -> Opening {
    let context = checked_state!(state.result_scope.lock(), Opening { scope: None, generation: 0, notice: None });
    let mut notice = checked_state!(state.selection_notice.lock(), Opening { scope: None, generation: 0, notice: None });
    let text = match notice.as_ref() {
        Some((Some(g), text)) if *g == context.generation => Some(text.clone()),
        Some((None, _)) => notice.take().map(|(_, text)| text),
        _ => None,
    };
    Opening { scope: context.summary(), generation: context.generation, notice: text }
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
        Action::ImagesToPdf { paths, output } => {
            ("images_to_pdf".into(), many(paths, "images"), dest(paths.first().unwrap_or(output), output))
        }
        Action::MergePdf { paths, output } => ("merge_pdf".into(), many(paths, "PDFs"), format!("→ {}/{}", dir(output), name(output))),
        Action::SplitPdf { path, output_dir } => ("split_pdf".into(), name(path), format!("→ {}/ (one PDF per page)", name(output_dir))),
        Action::ExtractPages { path, from, to, output } => {
            ("extract_pages".into(), name(path), format!("pages {from}–{to} → {}", name(output)))
        }
        Action::Zip { path, output } => ("zip".into(), name(path), format!("→ {}", name(output))),
        Action::Unzip { path, destination } => ("unzip".into(), name(path), format!("→ {}", roots.display(destination))),
        Action::OpenInFinder { path, .. } => ("open_in_finder".into(), name(path), "open in Finder".into()),
        Action::RemoveEmptyFolder { path } => ("remove_folder".into(), name(path), "remove this folder if it's empty".into()),
        Action::Restore { from: _, to } => ("restore".into(), name(to), "restore the original".into()),
    }
}

/// The files a step reads.
fn sources(a: &Action) -> Vec<&PathBuf> {
    match a {
        Action::Move { from, .. } | Action::Copy { from, .. } => vec![from],
        Action::Trash { path }
        | Action::Rename { path, .. }
        | Action::ConvertImage { path, .. }
        | Action::ResizeImage { path, .. }
        | Action::SplitPdf { path, .. }
        | Action::ExtractPages { path, .. }
        | Action::Zip { path, .. }
        | Action::Unzip { path, .. } => vec![path],
        Action::ImagesToPdf { paths, .. } | Action::MergePdf { paths, .. } => paths.iter().collect(),
        _ => vec![],
    }
}

/// The source files a plan reads, with when each was last modified.
fn input_times(actions: &[Action]) -> Vec<(PathBuf, std::time::SystemTime)> {
    actions
        .iter()
        .flat_map(sources)
        .filter_map(|p| {
            let m = std::fs::symlink_metadata(p).ok().filter(|m| m.is_file())?;
            Some((p.clone(), m.modified().ok()?))
        })
        .collect()
}

/// What the preview needs to describe any step of a pending plan. Rows are
/// formatted only when shown, so a large plan doesn't build thousands of
/// strings the user may never page to.
#[derive(Default)]
struct PreviewSource {
    notes: Vec<Option<String>>,
    skippable: Vec<bool>,
    roots: Option<Roots>,
}

impl PreviewSource {
    /// `skippable` is true for a plan the user may trim before confirming:
    /// one that runs now, rather than an undo or a scheduled task.
    fn new(actions: &[Action], notes: Vec<Option<String>>, roots: &Roots, skippable: bool) -> Self {
        let skippable = if skippable { skip::skippable(actions) } else { vec![false; actions.len()] };
        PreviewSource { notes, skippable, roots: Some(roots.clone()) }
    }

    fn noted(&self) -> usize {
        self.notes.iter().filter(|n| n.is_some()).count()
    }

    fn item(&self, actions: &[Action], step: usize) -> Option<PreviewItem> {
        let (kind, primary, detail) = describe(actions.get(step)?, self.roots.as_ref()?);
        let note = self.notes.get(step).cloned().flatten();
        Some(PreviewItem { kind, primary, detail, note, skippable: self.skippable.get(step).copied().unwrap_or(false) })
    }
}

// ---- Commands ----------------------------------------------------------------

#[tauri::command(async)]
fn plan_command(state: State<AppState>, text: String, choices: HashMap<String, String>, request_id: u64) -> CommandReply {
    let outcome = plan_input(&state, text, choices, request_id);
    let context = checked_state!(
        state.result_scope.lock(),
        CommandReply {
            outcome: Outcome::Problem { message: "Voxi state is unavailable. Restart the app.".into() },
            scope: None,
            generation: 0
        }
    );
    CommandReply { outcome, scope: context.summary(), generation: context.generation }
}

#[derive(serde::Serialize)]
struct CommandReply {
    outcome: Outcome,
    scope: Option<scope::Summary>,
    /// The results-list generation after this command.
    generation: u64,
}

/// Clears the results list, but only the one at `generation` when given, so
/// a closing view can't clear a list that arrived after it (a new Finder
/// selection, say).
#[tauri::command]
fn clear_result_scope(state: State<AppState>, generation: Option<u64>) {
    clear_scope(&state, generation);
}

fn clear_scope(state: &AppState, generation: Option<u64>) {
    let mut context = checked_state!(state.result_scope.lock(), ());
    if generation.is_none_or(|g| g == context.generation) {
        context.clear();
    }
}
#[tauri::command]
fn result_scope(state: State<AppState>) -> Option<scope::Summary> {
    checked_state!(state.result_scope.lock(), None).summary()
}

/// A weekday named in the command ("every Monday"), ignoring surrounding words.
#[cfg(feature = "tiny-model")]
fn named_weekday(text: &str) -> Option<chrono::Weekday> {
    use chrono::Weekday;
    text.split(|c: char| !c.is_alphabetic()).find_map(|word| match word.to_ascii_lowercase().as_str() {
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
            if rule.due != chosen.due {
                return false;
            }
            match (&rule.rule, &chosen.rule) {
                (None, None) => {}
                (Some(a), Some(b)) => {
                    if a.every != b.every || a.unit != b.unit || a.time != b.time {
                        return false;
                    }
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
enum Route {
    Rule,
    Model,
    Refuse,
}

#[cfg(feature = "tiny-model")]
fn recurring_route(rule_recurring: bool, rule_intent_present: bool, model_recurring: bool) -> Route {
    if rule_recurring || model_recurring { if rule_intent_present { Route::Rule } else { Route::Refuse } } else { Route::Model }
}

fn plan_input(state: &AppState, text: String, choices: HashMap<String, String>, request_id: u64) -> Outcome {
    if parse::check_length(&text).is_err() {
        let message = "That command is too long.".to_string();
        let mut active =
            checked_state!(state.active_search.lock(), Outcome::Problem { message: "Search state unavailable. Restart Voxi.".into() });
        if let Some((_, previous)) = active.take() {
            previous.cancel();
        }
        checked_state!(state.pending.lock(), Outcome::Problem { message: "Preview state unavailable. Restart Voxi.".into() }).take();
        return Outcome::Problem { message };
    }
    let now = chrono::Local::now();
    let control = search::Control::default();
    {
        let mut active =
            checked_state!(state.active_search.lock(), Outcome::Problem { message: "Voxi state is unavailable. Restart the app.".into() });
        if let Some((_, previous)) = active.take() {
            previous.cancel();
        }
        checked_state!(state.pending.lock(), Outcome::Problem { message: "Voxi state is unavailable. Restart the app.".into() }).take();
        *active = Some((request_id, control.clone()));
    }
    let original_text = text.clone();
    let (initial_generation, previous_scope) = {
        let context =
            checked_state!(state.result_scope.lock(), Outcome::Problem { message: "Voxi state is unavailable. Restart the app.".into() });
        (context.generation, context.current.clone())
    };
    // One scheduler reading and one parse of the command base are shared by
    // the rule path, the shortcut probe and the model gate. Re-running either
    // for each consumer used to parse the same words four or five times.
    let raw_schedule = scheduler::extract_schedule(&text, now);
    // A rejected time phrase is terminal, including for model fallback.
    if let Err(message) = &raw_schedule {
        return Outcome::Problem { message: message.clone() };
    }
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
        let settings =
            checked_state!(state.settings.lock(), Outcome::Problem { message: "Voxi state is unavailable. Restart the app.".into() });
        settings
            .find_shortcut(&text)
            .or_else(|| rule_base.as_ref().and_then(|base| settings.find_shortcut(base)))
            .or_else(|| classified.is_err().then(|| settings.find_close_shortcut(probe)).flatten())
            .cloned()
    };

    if let Some(s) = &shortcut
        && let Err(message) = settings::check_shortcut_commands(&s.commands)
    {
        return Outcome::Problem { message };
    }
    #[cfg(feature = "tiny-model")]
    let interpreted = {
        let mut model_error: Option<String> = None;
        let rule_recurring = rule_schedule.as_ref().ok().and_then(|(_, w)| w.as_ref()).is_some_and(|w| w.rule.is_some());
        let rule_intent_present = rule_intent.is_some();
        // A recurring job may only come from a rule interpretation. When the
        // rule parser already read a recurring command, skip model inference
        // entirely: its intent can disagree harmlessly with the model's and
        // must not be used as the job's authoritative interpretation.
        let chosen = if recurring_route(rule_recurring, rule_intent_present, false) == Route::Rule {
            None
        } else if plan_model::enabled() && previous_scope.is_none() && shortcut.is_none() && plan_model::eligible(&text, &classified) {
            match plan_model::interpret_at(&text, || control.is_cancelled(), now) {
                Ok(Some(value)) => {
                    let model_recurring = value.when.as_ref().is_some_and(|w| w.rule.is_some());
                    match recurring_route(rule_recurring, rule_intent_present, model_recurring) {
                        Route::Rule => None, // use the rule path; its rule keeps model_intent: None
                        Route::Refuse => {
                            return Outcome::Problem {
                                message: "Repeating tasks need a command Voxi understands without the model.".into(),
                            };
                        }
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
                Err(message) => {
                    model_error = Some(message);
                    None
                }
            }
        } else {
            None
        };
        // If the rule parser can't read the command either, surface why the
        // model was rejected rather than a generic "didn't understand".
        if chosen.is_none()
            && rule_intent.is_none()
            && let Some(message) = model_error
        {
            return Outcome::Problem { message };
        }
        chosen
    };
    #[cfg(not(feature = "tiny-model"))]
    let interpreted: Option<RuleInterpretation> = None;
    #[cfg(not(feature = "tiny-model"))]
    struct RuleInterpretation {
        intent: model::Intent,
        when: Option<scheduler::When>,
    }
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
                Ok(mut commands) if commands.len() == 1 && settings::shortcut_read_only(&commands[0].1) => {
                    Work::Command(commands.remove(0).1)
                }
                Ok(commands) => Work::Shortcut(s.phrase, commands),
                Err(bad) => {
                    return Outcome::Problem {
                        message: format!("In “{}”, Voxi doesn't understand “{bad}”. Fix it in Settings.", s.phrase),
                    };
                }
            }
        }
        None if model_intent.is_some() => Work::Command(model_intent.expect("is_some match guard establishes model intent")),
        None => match scope::prepare(&text, previous_scope.as_ref()) {
            Ok((intent, scoped)) => {
                use_scope = scoped;
                Work::Command(intent)
            }
            Err(scope::PrepareError::NotUnderstood(raw)) => {
                #[cfg(feature = "tiny-model")]
                let recovered = if scheduled_for.is_none()
                    && previous_scope.is_none()
                    && !plan_model::enabled()
                    && std::env::var_os("VOXI_TINY_MODEL_DIR").is_some()
                {
                    tiny_model::recover(&text, || control.is_cancelled())
                } else {
                    None
                };
                #[cfg(not(feature = "tiny-model"))]
                let recovered: Option<model::Intent> = None;
                match recovered {
                    Some(intent) => Work::Command(intent),
                    None => return Outcome::NotUnderstood { text: raw },
                }
            }
            Err(scope::PrepareError::Problem(message)) => return Outcome::Problem { message },
        },
    };
    let (scope_generation, scoped_snapshot) = {
        let active =
            checked_state!(state.active_search.lock(), Outcome::Problem { message: "Voxi state is unavailable. Restart the app.".into() });
        if active.as_ref().is_none_or(|(id, _)| *id != request_id) {
            return Outcome::Problem { message: "Search cancelled.".into() };
        }
        let mut context =
            checked_state!(state.result_scope.lock(), Outcome::Problem { message: "Voxi state is unavailable. Restart the app.".into() });
        if use_scope && context.generation != initial_generation {
            return Outcome::Problem { message: "The previous list expired. Run the list command again.".into() };
        }
        if !use_scope {
            context.clear();
        }
        (context.generation, if use_scope { context.current.clone() } else { None })
    };
    let listing = matches!(&work, Work::Command(model::Intent::List { .. }));
    let roots =
        checked_state!(state.roots.read(), Outcome::Problem { message: "Voxi state is unavailable. Restart the app.".into() }).clone();
    let choices: HashMap<String, PathBuf> = choices.into_iter().map(|(k, v)| (k, PathBuf::from(v))).collect();
    let planned = {
        let history =
            checked_state!(state.history.lock(), Outcome::Problem { message: "Voxi state is unavailable. Restart the app.".into() });
        let planner = Planner::new(&roots, &choices).with_control(control.clone()).with_scope(scoped_snapshot.clone());
        match &work {
            Work::Shortcut(phrase, commands) => planner.plan_shortcut(phrase, commands, &history),
            Work::Command(intent) => planner.plan(intent, &history),
        }
    };
    // Fingerprinting can walk a folder tree. Keep it cancellable and outside
    // the publication lock; only installing the resulting preview is serialized.
    let scheduled = match (&planned, scheduled_for) {
        (Ok(Planned::Steps { actions, undo_of, .. }), Some(due)) => Some(if undo_of.is_some() {
            Err("Undo cannot be scheduled.".into())
        } else if let Some(rule) = &repeat {
            if use_scope {
                Err("A repeating task can't use a previous list. Name the files or folder instead.".into())
            } else {
                scheduler::check_actions(actions).map(|()| {
                    scheduler::Job::recurring(
                        original_text.trim().to_string(),
                        due,
                        scheduler::Rule { choices: choices.clone(), ..rule.clone() },
                    )
                })
            }
        } else {
            scheduler::Job::new_with_control(original_text.clone(), due, actions.clone(), &control)
        }),
        _ => None,
    };
    // Serialize publication with cancellation so a late response cannot create a live preview.
    let active =
        checked_state!(state.active_search.lock(), Outcome::Problem { message: "Voxi state is unavailable. Restart the app.".into() });
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
        Ok(Planned::Info { .. } | Planned::Direct { .. }) if scheduled_for.is_some() => Outcome::Problem {
            message:
                "Schedule a command that changes files. Lists, History, and Finder commands run immediately when requested without a time."
                    .into(),
        },
        Ok(Planned::Info { title, summary, rows, total, listed }) => {
            let mut numbered = false;
            if listing {
                let mut context = checked_state!(
                    state.result_scope.lock(),
                    Outcome::Problem { message: "Voxi state is unavailable. Restart the app.".into() }
                );
                if context.generation == scope_generation {
                    match listed.and_then(|(paths, label)| {
                        scope::Snapshot::capture(paths, previous_scope.as_ref().filter(|_| use_scope).map_or(label, |s| s.label.clone()))
                    }) {
                        Some(snapshot) => {
                            context.publish(snapshot);
                            numbered = true;
                        }
                        None => context.clear(), // Partial/unreadable results must never become an action scope.
                    }
                }
            }
            Outcome::Info { title, summary, rows, total, numbered }
        }
        Ok(Planned::Direct { action, title }) => {
            let out = exec::run(std::slice::from_ref(&action), &roots);
            match out.error {
                Some(message) => Outcome::Problem { message },
                None => Outcome::Info { title, summary: String::new(), rows: vec![], total: 0, numbered: false },
            }
        }
        Ok(Planned::Steps { title, actions, notes, mut warnings, undo_of }) => {
            if used_model {
                warnings.insert(0, "Model interpretation: review the source folders and schedule before confirming.".into());
            }
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
            warnings.extend(
                checked_state!(state.history.lock(), Outcome::Problem { message: "History unavailable. Restart Voxi.".into() })
                    .notices
                    .iter()
                    .cloned(),
            );
            let id = state.next_plan.fetch_add(1, Ordering::SeqCst);
            if use_scope {
                warnings.insert(0, "Using files from the previous searched list.".into());
            }
            let source = PreviewSource::new(&actions, notes, &roots, undo_of.is_none() && scheduled.is_none());
            let items = (0..actions.len().min(PREVIEW_LIMIT)).filter_map(|step| source.item(&actions, step)).collect();
            let noted = source.noted();
            let total = actions.len();
            *checked_state!(state.pending.lock(), Outcome::Problem { message: "Voxi state is unavailable. Restart the app.".into() }) =
                Some(Pending {
                    id,
                    command: original_text.trim().to_string(),
                    inputs: if undo_of.is_none() && scheduled.is_none() { input_times(&actions) } else { Vec::new() },
                    actions,
                    preview: source,
                    undo_of,
                    scheduled,
                    context_generation: use_scope.then_some(scope_generation),
                    context_snapshot: scoped_snapshot,
                    stop_requested: false,
                });
            Outcome::Plan {
                id,
                title,
                total,
                items,
                noted,
                warnings,
                is_undo: undo_of.is_some(),
                scheduled_for,
                repeats: repeat.as_ref().zip(scheduled_for).map(|(rule, due)| {
                    if scheduler::runs_on_confirm(&original_text) {
                        format!("Runs now, then {}", rule.describe(due))
                    } else {
                        rule.describe(due)
                    }
                }),
            }
        }
    }
}

#[tauri::command(async)]
fn confirm_plan(state: State<AppState>, id: u64, skip: Option<Vec<usize>>) -> Result<RunResult, String> {
    confirm_trimmed(&state, id, &skip.unwrap_or_default())
}

#[cfg(test)]
fn confirm_input(state: &AppState, id: u64) -> Result<RunResult, String> {
    confirm_trimmed(state, id, &[])
}

/// Confirms the pending plan without the steps at `skip` (0-based), which
/// the user unticked in the preview.
fn confirm_trimmed(state: &AppState, id: u64, skip: &[usize]) -> Result<RunResult, String> {
    let (p, run) = {
        let mut pending = checked_state!(state.pending.lock(), Err("Voxi state is unavailable. Restart the app.".into()));
        // Trim and check before taking the preview, so a refused one stays
        // to fix. Only the steps that will run are checked for changed files.
        let trimmed = {
            let p = pending.as_ref().filter(|p| p.id == id).ok_or("That preview expired. Press Enter to plan it again.")?;
            let trimmed = if skip.is_empty() {
                None
            } else if p.undo_of.is_some() || p.scheduled.is_some() {
                return Err("Steps can't be left out of an undo or a scheduled task.".into());
            } else {
                Some(skip::without(&p.actions, skip)?)
            };
            p.validate_inputs(trimmed.as_deref().unwrap_or(&p.actions))?;
            trimmed
        };
        let mut p = pending.take_if(|p| p.id == id).ok_or("That preview expired. Press Enter to plan it again.")?;
        if let Some(actions) = trimmed {
            p.actions = actions;
        }
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
            if let Err(error) = checked_state!(state.schedules.lock(), Err("Voxi state is unavailable. Restart the app.".into()))
                .record_summary(job_id, run_summary)
            {
                result.error = Some(result.error.map_or(error.clone(), |first| format!("{first} {error}")));
            }
            let rule = job.repeat.as_ref().ok_or("That task doesn't repeat.")?;
            result.summary.push_str(&format!(
                " Repeats {}; next run {}. Manage tasks in Settings → Scheduled.",
                rule.describe(job.due),
                scheduler::display(job.due)
            ));
            return Ok(result);
        }
        if job.due <= chrono::Local::now() {
            return Err("That scheduled time passed while reviewing. Preview the command again.".into());
        }
        job.validate()?;
        checked_state!(state.schedules.lock(), Err("Voxi state is unavailable. Restart the app.".into())).add(job.clone())?;
        let summary = match &job.repeat {
            Some(rule) => format!(
                "Repeats {}, first on {}. Manage tasks in Settings → Scheduled.",
                rule.describe(job.due),
                scheduler::display(job.due)
            ),
            None => format!("Scheduled for {}. Manage tasks in Settings → Scheduled.", scheduler::display(job.due)),
        };
        return Ok(RunResult { done: 0, total: p.actions.len(), error: None, cancelled: false, undoable: false, summary });
    }
    Ok(execute_pending(state, p, &is_cancelled))
}

/// More of the pending preview: up to `PREVIEW_PAGE` steps from `offset`,
/// or with `only_notes`, the steps that carry a note (a name changed to
/// avoid a clash, say) from `offset` on, each with its step number. Fails
/// once the plan is no longer the pending one.
#[tauri::command]
fn preview_page(state: State<AppState>, id: u64, offset: usize, only_notes: bool) -> Result<Vec<PreviewRow>, String> {
    let pending = state.pending.lock().map_err(|_| "Preview state unavailable. Restart Voxi.")?;
    let p = pending.as_ref().filter(|p| p.id == id).ok_or("This preview expired. Run the command again.")?;
    Ok(page_steps(&p.preview.notes, p.actions.len(), offset, only_notes)
        .into_iter()
        .filter_map(|step| Some(PreviewRow { step, item: p.preview.item(&p.actions, step)? }))
        .collect())
}

/// The steps a page shows: up to `PREVIEW_PAGE` from `offset`, or only
/// those with a note.
fn page_steps(notes: &[Option<String>], total: usize, offset: usize, only_notes: bool) -> Vec<usize> {
    (offset..total).filter(|&i| !only_notes || notes.get(i).is_some_and(Option::is_some)).take(PREVIEW_PAGE).collect()
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
        Err(error) => RunResult {
            done: 0,
            total: p.actions.len(),
            error: Some(error),
            cancelled: false,
            undoable: false,
            summary: "Stopped. Durable execution or undo could not be confirmed; review the recovery journal.".into(),
        },
    }
}

fn execute_durable(state: &AppState, p: &Pending, is_cancelled: &dyn Fn() -> bool) -> Result<RunResult, String> {
    let roots = state.roots.read().map_err(|_| "Folder state is unavailable. Restart Voxi.")?.clone();
    let mut history = state.history.lock().map_err(|_| "History state is unavailable. Restart Voxi.")?;
    let path = history.journal_path();
    let entry = Entry {
        id: history.next_id(),
        time: chrono::Local::now(),
        command: p.command.clone(),
        done: vec![],
        inverse: vec![],
        undone: false,
        undo_of: p.undo_of,
    };
    let out = operation_journal::run(&path, &p.actions, &roots, entry, &mut history, is_cancelled)?;
    let mut context = state.result_scope.lock().map_err(|_| "Result state is unavailable. Restart Voxi.")?;
    if p.context_generation == Some(context.generation)
        && let Some(snapshot) = &mut context.current
    {
        snapshot.apply(&out.done);
    }
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
        && actions.iter().any(is_move)
    {
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
    if undoable {
        summary.push_str(" Undo reverses those changes.");
    }
    if was_undo {
        summary.push_str(" Run Undo again to finish.");
    }
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
    if let Some(notice) = checked_state!(state.schedules.lock(), None).notice() {
        notices.push(notice);
    }
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
fn schedule_of(
    state: &AppState,
    text: &str,
    now: chrono::DateTime<chrono::Local>,
    raw: &Result<(String, Option<scheduler::When>), String>,
) -> Result<(String, Option<scheduler::When>), String> {
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
fn sched_skip(job: &scheduler::Job) {
    diagnostics::emit(Event::new(Code::SchedulerSkip, Stage::Skipped).operation(job.id));
}

/// True when any filter in `intent` reads the last-modified date.
fn reads_modified_date(intent: &model::Intent) -> bool {
    fn walk(v: &serde_json::Value) -> bool {
        match v {
            serde_json::Value::Object(m) => m.get("dateField").is_some_and(|f| f == "modified") || m.values().any(walk),
            serde_json::Value::Array(a) => a.iter().any(walk),
            _ => false,
        }
    }
    serde_json::to_value(intent).is_ok_and(|v| walk(&v))
}

fn run_recurring(state: &AppState, job: &scheduler::Job, rule: &scheduler::Rule, is_cancelled: &dyn Fn() -> bool) -> String {
    let roots = checked_state!(state.roots.read(), "Voxi state is unavailable. Restart the app.".into()).clone();
    let shortcut =
        checked_state!(state.settings.lock(), "Voxi state is unavailable. Restart the app.".into()).find_shortcut(&rule.command).cloned();
    if let Err(e) = parse::check_length(&rule.command)
        .map_err(|_| "That command is too long.".to_string())
        .and_then(|()| shortcut.as_ref().map_or(Ok(()), |s| settings::check_shortcut_commands(&s.commands)))
    {
        sched_skip(job);
        return e;
    }
    let texts: Vec<&String> = shortcut.as_ref().map_or_else(|| vec![&rule.command], |s| s.commands.iter().collect());
    // A rule saved before "including subfolders" opted in read those words
    // as part of a name; it may now reach other files. Don't run it unattended.
    if !rule.subfolder_opt_in && texts.iter().any(|t| parse::names_subfolders(t)) {
        sched_skip(job);
        return format!("Skipped: Voxi now reads “subfolders” differently. Schedule “{}” again to keep this task.", job.command);
    }
    // A rule saved before "modified" meant the last edit was approved with
    // the creation date. Don't run it unattended with the new meaning.
    if !rule.modified_dates && texts.iter().any(|t| parse::parse(t).is_ok_and(|intent| reads_modified_date(&intent))) {
        sched_skip(job);
        return format!(
            "Skipped: “modified” now means the date a file was last edited. Schedule “{}” again to keep this task.",
            job.command
        );
    }
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
                Err(bad) => {
                    sched_skip(job);
                    return format!("Skipped: Voxi doesn't understand “{bad}” in “{}”.", s.phrase);
                }
            },
            None => match parse::parse(&rule.command) {
                Ok(intent) => planner.plan(&intent, &history),
                Err(_) => {
                    sched_skip(job);
                    return format!("Skipped: Voxi doesn't understand “{}”.", rule.command);
                }
            },
        }
    };
    match planned {
        Ok(Planned::Steps { actions, undo_of: None, .. }) if actions.is_empty() => {
            sched_skip(job);
            "Nothing to change.".into()
        }
        Ok(Planned::Steps { actions, undo_of: None, .. }) => {
            if let Err(message) = scheduler::check_actions(&actions) {
                sched_skip(job);
                return format!("Skipped: {message}");
            }
            let result = execute_pending(
                state,
                Pending {
                    id: 0,
                    command: job.command.clone(),
                    actions,
                    preview: PreviewSource::default(),
                    inputs: Vec::new(),
                    undo_of: None,
                    context_generation: None,
                    context_snapshot: None,
                    scheduled: None,
                    stop_requested: false,
                },
                is_cancelled,
            );
            match result.error {
                Some(error) => {
                    diagnostics::emit(Event::new(Code::SchedulerRun, Stage::Failed).operation(job.id));
                    format!("{} {error}", result.summary)
                }
                None => result.summary,
            }
        }
        Ok(_) => {
            sched_skip(job);
            "Skipped: this command no longer changes files.".into()
        }
        Err(Stop::Problem(message)) => {
            sched_skip(job);
            format!("Nothing to change. {message}")
        }
        Err(Stop::Ambiguous { name, .. }) => {
            sched_skip(job);
            format!("Skipped: “{name}” matches more than one item. Schedule it again and pick one.")
        }
    }
}

fn run_due_tasks(state: &AppState) -> Result<(), String> {
    let result = run_due_tasks_inner(state);
    if result.is_err() {
        diagnostics::emit(Event::new(Code::SchedulerRun, Stage::Failed));
    }
    result
}

fn run_due_tasks_inner(state: &AppState) -> Result<(), String> {
    let _execution = checked_state!(state.execution.lock(), Err("Voxi state is unavailable. Restart the app.".into()));
    // Check under the execution lock, before persisting any job claim.
    if operation_journal::status(&state.reports_path.with_file_name("operation-journal.json"))?.is_some() {
        return Ok(());
    }
    let (job, run) = {
        let mut store = checked_state!(state.schedules.lock(), Err("Voxi state is unavailable. Restart the app.".into()));
        let Some(job) = store.claim(chrono::Local::now())? else {
            return Ok(());
        };
        let run = ActiveRun::register(&state.active_runs, RunKey::Scheduled(job.id, job.due), false)?;
        (job, run)
    };
    let is_cancelled = || run.is_stopped();
    if let Some(rule) = job.repeat.clone() {
        let summary = run_recurring(state, &job, &rule, &is_cancelled);
        return checked_state!(state.schedules.lock(), Err("Voxi state is unavailable. Restart the app.".into())).finish_recurring(
            job.id,
            summary,
            chrono::Local::now(),
        );
    }
    let result = match job.validate() {
        Err(message) => ("failed", message),
        Ok(()) => {
            let result = execute_pending(
                state,
                Pending {
                    id: 0,
                    command: job.command.clone(),
                    actions: job.actions,
                    preview: PreviewSource::default(),
                    inputs: Vec::new(),
                    undo_of: None,
                    context_generation: None,
                    context_snapshot: None,
                    scheduled: None,
                    stop_requested: false,
                },
                &is_cancelled,
            );
            (
                if result.error.is_some() {
                    "failed"
                } else if result.cancelled {
                    "cancelled"
                } else {
                    "completed"
                },
                result.error.map_or(result.summary.clone(), |error| format!("{} {error}", result.summary)),
            )
        }
    };
    if result.0 == "failed" {
        diagnostics::emit(Event::new(Code::SchedulerRun, Stage::Failed).operation(job.id));
    }
    checked_state!(state.schedules.lock(), Err("Voxi state is unavailable. Restart the app.".into())).update(job.id, result.0, result.1)
}

#[tauri::command]
fn cancel_plan(state: State<AppState>) {
    if let Some((_, control)) = checked_state!(state.active_search.lock(), ()).take() {
        control.cancel();
    }
    checked_state!(state.pending.lock(), ()).take();
}

#[tauri::command]
fn search_progress(state: State<AppState>, request_id: u64) -> usize {
    checked_state!(state.active_search.lock(), 0)
        .as_ref()
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
            if checked_state!(handle.state::<AppState>().presentation.lock(), ()).pending.is_some() {
                return;
            }
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
    if let Some(request) = &presentation.pending {
        let _ = app.emit("voxi://shown", request.clone());
    }
}

#[tauri::command]
fn present_window(app: AppHandle, request_id: u64, height: f64, animate: bool) {
    if !height.is_finite() {
        return;
    }
    let Some(w) = app.get_webview_window("main") else { return };
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        if !checked_state!(handle.state::<AppState>().presentation.lock(), ()).acknowledge(request_id) {
            return;
        }
        #[cfg(target_os = "macos")]
        {
            if let Ok(ptr) = w.ns_window() {
                let ns: &objc2_app_kit::NSWindow = unsafe { &*(ptr as *const objc2_app_kit::NSWindow) };
                if !ns.isVisible() {
                    ns.setAlphaValue(if animate { 0.0 } else { 1.0 });
                }
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
            if !presentation.is_current(request_id) {
                return;
            }
            presentation.cancel();
        }
        #[cfg(target_os = "macos")]
        if let Some(w) = handle.get_webview_window("main")
            && let Ok(ptr) = w.ns_window()
        {
            let ns: &objc2_app_kit::NSWindow = unsafe { &*(ptr as *const objc2_app_kit::NSWindow) };
            fade_window(ns, false, animate);
        }
        #[cfg(not(target_os = "macos"))]
        let _ = animate;
    });
}

#[tauri::command]
fn hide_window(app: AppHandle, request_id: u64) {
    if !checked_state!(app.state::<AppState>().presentation.lock(), ()).is_current(request_id) {
        return;
    }
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
    let os_before = al.is_enabled().map_err(|e| {
        diagnostics::emit(Event::new(Code::LoginItem, Stage::Failed));
        format!("Cannot read login registration: {e}")
    })?;
    if settings.start_at_login != os_before && !owns_login_item() {
        diagnostics::emit(Event::new(Code::LoginItem, Stage::Failed));
        return Err("Move Voxi to /Applications or ~/Applications and open that copy before changing Start at Login.".into());
    }
    if settings.hotkey != old.hotkey
        && let Err(e) = register_hotkey(&app, &settings.hotkey)
    {
        let _ = register_hotkey(&app, &old.hotkey);
        return Err(e);
    }
    let result = settings::login_transaction(&settings, &state.settings_path, os_before, |enabled| {
        let result = if enabled { al.enable() } else { al.disable() };
        result.map_err(|e| format!("Cannot change login registration: {e}. Check System Settings → General → Login Items."))?;
        if al.is_enabled().map_err(|e| e.to_string())? != enabled {
            return Err("macOS did not accept the login registration. Check Login Items in System Settings.".into());
        }
        Ok(())
    });
    match result {
        Ok(settings::Durability::Durable) => {}
        Ok(settings::Durability::Uncertain(warning)) => eprintln!("{warning}"),
        Err(e) => {
            diagnostics::emit(Event::new(Code::LoginItem, Stage::Failed));
            if settings.hotkey != old.hotkey {
                let _ = register_hotkey(&app, &old.hotkey);
            }
            return Err(e);
        }
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
    if !folder.is_dir()
        || !checked_state!(state.roots.read(), Err("Voxi state is unavailable. Restart the app.".into())).is_allowed(&folder)
    {
        return Err("Voxi can't use that folder.".into());
    }
    let current = checked_state!(state.settings.lock(), Err("Voxi state is unavailable. Restart the app.".into())).clone();
    let next = learn::add_nickname(&current, &name, &folder, &home())?;
    save_settings(app, state, next)
}

/// A fixed destination; the frontend cannot ask this command to open arbitrary URLs.
#[tauri::command(async)]
fn open_hex() -> Result<(), String> {
    Command::new("/usr/bin/open")
        .arg("https://anoma.ly/hex/")
        .status()
        .map_err(|e| format!("Couldn't open Hex: {e}"))?
        .success()
        .then_some(())
        .ok_or_else(|| "Couldn't open the Hex website.".into())
}

/// macOS opens whichever copy of Voxi last turned on "Open at login". Only an
/// installed app may: a development build opened at login shows an empty
/// window, since the page it loads comes from a dev server that isn't running.
fn owns_login_item() -> bool {
    std::env::current_exe().is_ok_and(|p| {
        let installed = p.starts_with("/Applications") || p.starts_with(home().join("Applications"));
        installed && p.to_string_lossy().contains(".app/Contents/MacOS/") && !p.to_string_lossy().contains("/AppTranslocation/")
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
    let status = Command::new("/usr/bin/open")
        .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_FilesAndFolders")
        .status()
        .map_err(|e| format!("Couldn't open System Settings: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err("Couldn't open System Settings. Open Privacy & Security → Files and Folders manually.".into())
    }
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
fn updater_failed() {
    diagnostics::emit(Event::new(Code::Update, Stage::Failed));
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let mut context = tauri::generate_context!();
    // Development builds get their own identity, so a dev build never shares
    // settings, history, schedules, the undo journal or the login item with
    // an installed Voxi on the same Mac.
    if cfg!(debug_assertions) {
        context.config_mut().identifier.push_str(".dev");
        context.package_info_mut().name = "Voxi Dev".into();
    }
    if let Some(data) = dirs::data_dir() {
        diagnostics::init(data.join(&context.config().identifier).join("diagnostics"));
    }
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

            if dirs::home_dir().is_none() {
                return Err("Cannot locate your home folder. Check your macOS account and restart Voxi.".into());
            }
            let config_dir = app.path().app_config_dir()?;
            let data_dir = app.path().app_data_dir()?;
            let settings_path = config_dir.join("settings.json");
            let (settings, settings_warning) = Settings::load_with_warning(&settings_path);
            let (mut history, history_warning) = History::load_with_warning(data_dir.join("history.json"));
            let mut storage_notices: Vec<String> = [settings_warning, history_warning]
                .into_iter()
                .flatten()
                .map(|w| match w.backup {
                    Some(path) => format!("{} Backup: {}", w.message, path.display()),
                    None => w.message,
                })
                .collect();
            if let Err(e) = operation_journal::recover(&history.journal_path(), &mut history)
                && !matches!(operation_journal::status(&history.journal_path()), Ok(Some(_)))
            {
                storage_notices.push(e);
            }
            match app.autolaunch().is_enabled() {
                Ok(enabled) if enabled != settings.start_at_login => storage_notices
                    .push("macOS login registration differs from the saved preference. Review Start at Login in Settings.".into()),
                Err(e) => {
                    diagnostics::emit(Event::new(Code::LoginItem, Stage::Failed));
                    storage_notices
                        .push(format!("Cannot read macOS login registration: {e}. Check System Settings → General → Login Items."));
                }
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
                selection_notice: Mutex::new(None),
            };
            app.manage(state);
            #[cfg(feature = "tiny-model")]
            if plan_model::enabled() {
                eprintln!("Voxi: structured model mode enabled ({})", voxi_tiny_native::PLAN_PACKAGE_ID);
            }
            let scheduler_app = app.handle().clone();
            std::thread::spawn(move || {
                loop {
                    let _ = run_due_tasks(&scheduler_app.state::<AppState>());
                    std::thread::sleep(std::time::Duration::from_secs(1));
                }
            });

            // Configure Writing Tools before WKWebView copies its configuration.
            // The window is created here rather than automatically from config.
            let window = tauri::WebviewWindowBuilder::from_config(app, &app.config().app.windows[0])?;
            #[cfg(target_os = "macos")]
            let window = {
                let mtm = objc2::MainThreadMarker::new().ok_or("Window setup requires the main thread")?;
                let configuration = unsafe { objc2_web_kit::WKWebViewConfiguration::new(mtm) };
                // The public selector is available starting with macOS 15.
                let supported: bool = unsafe {
                    objc2::msg_send![&*configuration,
                    respondsToSelector: objc2::sel!(setWritingToolsBehavior:)]
                };
                if supported {
                    unsafe {
                        let _: () = objc2::msg_send![&*configuration,
                        setWritingToolsBehavior: objc2_app_kit::NSWritingToolsBehavior::None];
                    }
                }
                window.with_webview_configuration(configuration)
            };
            window.build()?;
            #[cfg(target_os = "macos")]
            if let Err(e) = finder_selection::service::install(app.handle()) {
                eprintln!("Voxi: could not register the Finder service: {e}");
            }
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
            preview_page,
            opening_context,
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
    let app = match app {
        Ok(app) => app,
        Err(_) => {
            diagnostics::emit(Event::new(Code::Startup, Stage::Failed));
            return;
        }
    };
    app.run(|_, event| {
        #[cfg(target_os = "macos")]
        if matches!(event, tauri::RunEvent::Exit) {
            outside_click::uninstall();
        }
    });
}

#[cfg(test)]
#[path = "lib_tests/result_context.rs"]
mod result_context_tests;

#[cfg(test)]
#[path = "lib_tests/plan_guard.rs"]
mod plan_guard_tests;

/// The command box suggests these; each must parse, and "every"/"at" ones must schedule.
#[cfg(test)]
#[path = "lib_tests/examples.rs"]
mod example_tests;

#[cfg(test)]
#[path = "lib_tests/preview_pages.rs"]
mod preview_page_tests;
