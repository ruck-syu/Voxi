mod exec;
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
mod settings;
mod search;
mod scope;
mod trash;
#[cfg(test)]
mod paraphrase_audit; // report-only, all tests #[ignore]

use fsx::{Roots, file_name};
use history::{Entry, History};
use model::{Action, Outcome, PreviewItem, RunResult};
use plan::{Planned, Planner, Stop};
use settings::Settings;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, RwLock};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, State};
use tauri_plugin_autostart::ManagerExt as _;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

const WINDOW_WIDTH: f64 = 680.0;
const PREVIEW_LIMIT: usize = 20;

struct Pending {
    id: u64,
    command: String,
    actions: Vec<Action>,
    undo_of: Option<u64>,
    context_generation: Option<u64>,
    context_snapshot: Option<scope::Snapshot>,
}
impl Pending {
    fn validate_context(&self, context: &scope::State) -> Result<(), String> {
        if self.context_generation.is_some_and(|generation| generation != context.generation) {
            return Err("The previous list expired. Run the list command again.".into());
        }
        self.context_snapshot.as_ref().map_or(Ok(()), scope::Snapshot::validate)
    }
}

struct AppState {
    roots: RwLock<Roots>,
    history: Mutex<History>,
    pending: Mutex<Option<Pending>>,
    settings: Mutex<Settings>,
    settings_path: PathBuf,
    /// "This was wrong" reports, kept on this Mac.
    reports_path: PathBuf,
    result_scope: Mutex<scope::State>,
    next_plan: AtomicU64,
    active_search: Mutex<Option<(u64, search::Control)>>,
    presentation: Mutex<presentation::Presentation>,
}

fn home() -> PathBuf {
    dirs::home_dir().expect("no home folder")
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
    if let Ok(cursor) = app.cursor_position() {
        if let Ok(Some(m)) = app.monitor_from_point(cursor.x, cursor.y) {
            let (pos, size, scale) = (m.position(), m.size(), m.scale_factor());
            let x = pos.x + (size.width as i32 - (WINDOW_WIDTH * scale) as i32) / 2;
            let y = pos.y + (size.height as f64 * 0.22) as i32;
            let _ = w.set_position(PhysicalPosition::new(x, y));
        }
    }
    let state = app.state::<AppState>();
    let mut presentation = state.presentation.lock().unwrap();
    let request = presentation.request(mode);
    if presentation.ready { let _ = w.emit("voxi://shown", request); }

}

fn hide(app: &AppHandle) {
    app.state::<AppState>().result_scope.lock().unwrap().clear();
    app.state::<AppState>().presentation.lock().unwrap().cancel();
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
    app.state::<AppState>().presentation.lock().unwrap().cancel();
    match app.get_webview_window("main") {
        Some(w) if w.emit("voxi://hide", ()).is_ok() => {}
        _ => hide(app),
    }
}

fn toggle_window(app: &AppHandle) {
    let Some(w) = app.get_webview_window("main") else { return };
    let (pending, closing) = {
        let state = app.state::<AppState>();
        let presentation = state.presentation.lock().unwrap();
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
        Action::Move { from, to } => ("move".into(), name(from), dest(from, to)),
        Action::Copy { from, to } => ("copy".into(), name(from), dest(from, to)),
        Action::Rename { path, new_name } => ("rename".into(), name(path), format!("→ {new_name}")),
        Action::Trash { path } => ("trash".into(), name(path), "→ Trash".into()),
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
    let scope = state.result_scope.lock().unwrap().summary();
    CommandReply { outcome, scope }
}

#[derive(serde::Serialize)]
struct CommandReply { outcome: Outcome, scope: Option<scope::Summary> }

#[tauri::command]
fn clear_result_scope(state: State<AppState>) { state.result_scope.lock().unwrap().clear(); }
#[tauri::command]
fn result_scope(state: State<AppState>) -> Option<scope::Summary> { state.result_scope.lock().unwrap().summary() }

fn plan_input(state: &AppState, text: String, choices: HashMap<String, String>, request_id: u64) -> Outcome {
    let control = search::Control::default();
    {
        let mut active = state.active_search.lock().unwrap();
        if let Some((_, previous)) = active.take() { previous.cancel(); }
        state.pending.lock().unwrap().take();
        *active = Some((request_id, control.clone()));
    }
    // A saved shortcut ("run my usuals") wins over the built-in commands.
    enum Work {
        Shortcut(String, Vec<(String, model::Intent)>),
        Command(model::Intent),
    }
    let (initial_generation, previous_scope) = {
        let context = state.result_scope.lock().unwrap();
        (context.generation, context.current.clone())
    };
    let mut use_scope = false;
    let shortcut = {
        let settings = state.settings.lock().unwrap();
        // An exact phrase wins over the built-in commands. A near miss
        // ("run my visuals") counts only when the text isn't a command.
        settings
            .find_shortcut(&text)
            .or_else(|| parse::parse(&text).is_err().then(|| settings.find_close_shortcut(&text)).flatten())
            .cloned()
    };
    let work = match shortcut {
        Some(s) => {
            let parsed: Result<Vec<_>, _> = s.commands.iter().map(|c| parse::parse(c).map(|i| (c.clone(), i))).collect();
            match parsed {
                Ok(commands) => Work::Shortcut(s.phrase, commands),
                Err(bad) => return Outcome::Problem { message: format!("In “{}”, Voxi doesn't understand “{bad}”. Fix it in Settings.", s.phrase) },
            }
        }
        None => match scope::prepare(&text, previous_scope.as_ref()) {
            Ok((intent, scoped)) => { use_scope = scoped; Work::Command(intent) },
            Err(raw) if raw == text.trim() => return Outcome::NotUnderstood { text: raw },
            Err(message) => return Outcome::Problem { message },
        },
    };
    let (scope_generation, scoped_snapshot) = {
        let active = state.active_search.lock().unwrap();
        if !active.as_ref().is_some_and(|(id, _)| *id == request_id) { return Outcome::Problem { message: "Search cancelled.".into() }; }
        let mut context = state.result_scope.lock().unwrap();
        if use_scope && context.generation != initial_generation { return Outcome::Problem { message: "The previous list expired. Run the list command again.".into() }; }
        if !use_scope { context.clear(); }
        (context.generation, if use_scope { context.current.clone() } else { None })
    };
    let listing = matches!(&work, Work::Command(model::Intent::List { .. }));
    let roots = state.roots.read().unwrap().clone();
    let choices: HashMap<String, PathBuf> = choices.into_iter().map(|(k, v)| (k, PathBuf::from(v))).collect();
    let planned = {
        let history = state.history.lock().unwrap();
        let planner = Planner::new(&roots, &choices).with_control(control.clone()).with_scope(scoped_snapshot.clone());
        match &work {
            Work::Shortcut(phrase, commands) => planner.plan_shortcut(phrase, commands, &history),
            Work::Command(intent) => planner.plan(intent, &history),
        }
    };
    // Serialize publication with cancellation so a late response cannot create a live preview.
    let active = state.active_search.lock().unwrap();
    if control.is_cancelled() || !active.as_ref().is_some_and(|(id, _)| *id == request_id) {
        return Outcome::Problem { message: "Search cancelled.".into() };
    }
    match planned {
        Err(Stop::Problem(message)) => Outcome::Problem { message },
        Err(Stop::Ambiguous { name, candidates }) => Outcome::Ambiguous {
            name,
            folders: !candidates.is_empty() && candidates.iter().all(|p| p.is_dir()),
            candidates: candidates.iter().map(|p| p.to_string_lossy().into_owned()).collect(),
        },
        Ok(Planned::Info { title, summary, rows, total, listed }) => {
            let mut numbered = false;
            if listing {
                let mut context = state.result_scope.lock().unwrap();
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
            let id = state.next_plan.fetch_add(1, Ordering::SeqCst);
            if use_scope { warnings.insert(0, "Using files from the previous searched list.".into()); }
            let items = preview(&actions, &notes, &roots);
            let total = actions.len();
            *state.pending.lock().unwrap() = Some(Pending { id, command: text.trim().to_string(), actions, undo_of, context_generation: use_scope.then_some(scope_generation), context_snapshot: scoped_snapshot });
            Outcome::Plan { id, title, total, items, warnings, is_undo: undo_of.is_some() }
        }
    }
}

#[tauri::command(async)]
fn confirm_plan(state: State<AppState>, id: u64) -> Result<RunResult, String> {
    let pending = state.pending.lock().unwrap().take();
    let Some(p) = pending.filter(|p| p.id == id) else {
        return Err("That preview expired. Press Enter to plan it again.".into());
    };
    p.validate_context(&state.result_scope.lock().unwrap())?;
    let roots = state.roots.read().unwrap().clone();
    let out = exec::run(&p.actions, &roots);
    {
        let mut context = state.result_scope.lock().unwrap();
        if p.context_generation == Some(context.generation) {
            if let Some(snapshot) = &mut context.current { snapshot.apply(&out.done); }
        }
    }
    let done = out.done.len();
    let undoable = !out.inverse.is_empty() && p.undo_of.is_none();
    if done > 0 {
        let mut history = state.history.lock().unwrap();
        let id = history.next_id();
        history.push(Entry {
            id,
            time: chrono::Local::now(),
            command: p.command.clone(),
            done: out.done,
            inverse: out.inverse,
            undone: false,
            undo_of: p.undo_of,
        });
    }
    let summary = match (&out.error, p.undo_of) {
        (None, Some(_)) => "Undone.".to_string(),
        (None, None) => format!("Done. {done} {}.", if done == 1 { "change" } else { "changes" }),
        (Some(_), _) => format!("Stopped after {done} of {} steps.", p.actions.len()),
    };
    Ok(RunResult { done, total: p.actions.len(), error: out.error, undoable, summary })
}

#[tauri::command]
fn cancel_plan(state: State<AppState>) {
    if let Some((_, control)) = state.active_search.lock().unwrap().take() { control.cancel(); }
    state.pending.lock().unwrap().take();
}

#[tauri::command]
fn search_progress(state: State<AppState>, request_id: u64) -> usize {
    state.active_search.lock().unwrap().as_ref()
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
#[tauri::command]
fn fit_window(app: AppHandle, height: f64, animate: bool) {
    let Some(w) = app.get_webview_window("main") else { return };
    let height = height.clamp(48.0, 640.0);
    let handle = app.clone();
    #[cfg(target_os = "macos")]
    {
        let _ = app.run_on_main_thread(move || {
            if handle.state::<AppState>().presentation.lock().unwrap().pending.is_some() { return; }
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
    let mut presentation = state.presentation.lock().unwrap();
    presentation.ready = true;
    if let Some(request) = &presentation.pending { let _ = app.emit("voxi://shown", request.clone()); }
}

#[tauri::command]
fn present_window(app: AppHandle, request_id: u64, height: f64, animate: bool) {
    if !height.is_finite() { return; }
    let Some(w) = app.get_webview_window("main") else { return };
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        if !handle.state::<AppState>().presentation.lock().unwrap().acknowledge(request_id) { return; }
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
            let mut presentation = state.presentation.lock().unwrap();
            if !presentation.is_current(request_id) { return; }
            presentation.cancel();
        }
        #[cfg(target_os = "macos")]
        if let Some(w) = handle.get_webview_window("main") {
            if let Ok(ptr) = w.ns_window() {
                let ns: &objc2_app_kit::NSWindow = unsafe { &*(ptr as *const objc2_app_kit::NSWindow) };
                fade_window(ns, false, animate);
            }
        }
        #[cfg(not(target_os = "macos"))]
        let _ = animate;
    });
}

#[tauri::command]
fn hide_window(app: AppHandle, request_id: u64) {
    if !app.state::<AppState>().presentation.lock().unwrap().is_current(request_id) { return; }
    hide(&app);
}

#[tauri::command]
fn get_settings(state: State<AppState>) -> Settings {
    state.settings.lock().unwrap().clone()
}

#[tauri::command]
fn save_settings(app: AppHandle, state: State<AppState>, mut settings: Settings) -> Result<Settings, String> {
    settings.validate_shortcuts()?;
    settings.validate_nicknames(&home())?;
    let old = state.settings.lock().unwrap().clone();
    if settings.hotkey != old.hotkey {
        if let Err(e) = register_hotkey(&app, &settings.hotkey) {
            let _ = register_hotkey(&app, &old.hotkey);
            return Err(e);
        }
    }
    if settings.start_at_login != old.start_at_login && owns_login_item() {
        let al = app.autolaunch();
        let r = if settings.start_at_login { al.enable() } else { al.disable() };
        r.map_err(|e| format!("Couldn't change the login item: {e}"))?;
    }
    settings.save(&state.settings_path)?;
    *state.roots.write().unwrap() = build_roots(&settings);
    *state.settings.lock().unwrap() = settings.clone();
    state.result_scope.lock().unwrap().clear();
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
    let current = state.settings.lock().unwrap().clone();
    let next = learn::teach_phrase(&current, &phrase, &command)?;
    save_settings(app, state, next)
}

/// Remembers a folder the user picked as their name for it. Only folders
/// Voxi may already use can get a nickname.
#[tauri::command]
fn remember_folder(app: AppHandle, state: State<AppState>, name: String, path: String) -> Result<Settings, String> {
    let folder = PathBuf::from(&path);
    if !folder.is_dir() || !state.roots.read().unwrap().is_allowed(&folder) {
        return Err("Voxi can't use that folder.".into());
    }
    let current = state.settings.lock().unwrap().clone();
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
        let p = p.to_string_lossy();
        p.contains(".app/Contents/MacOS/") && !p.contains("/AppTranslocation/")
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
    let roots = state.roots.read().unwrap().clone();
    roots
        .search_roots()
        .into_iter()
        .map(|p| FolderAccess { name: file_name(&p), path: roots.display(&p), allowed: std::fs::read_dir(&p).is_ok() })
        .collect()
}

/// Opens System Settings at Privacy & Security, Files and Folders.
#[tauri::command]
fn open_privacy_settings() {
    let _ = Command::new("/usr/bin/open").arg("x-apple.systempreferences:com.apple.preference.security?Privacy_FilesAndFolders").status();
}

/// The folders Voxi may change, for the settings and welcome views.
#[tauri::command]
fn allowed_folders(state: State<AppState>) -> Vec<String> {
    let roots = state.roots.read().unwrap();
    let mut v: Vec<String> = roots.search_roots().iter().map(|p| roots.display(p)).collect();
    v.dedup();
    v
}

// ---- App ---------------------------------------------------------------------

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
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

            let config_dir = app.path().app_config_dir()?;
            let data_dir = app.path().app_data_dir()?;
            let settings_path = config_dir.join("settings.json");
            let settings = Settings::load(&settings_path);
            let state = AppState {
                roots: RwLock::new(build_roots(&settings)),
                history: Mutex::new(History::load(data_dir.join("history.json"))),
                pending: Mutex::new(None),
                settings: Mutex::new(settings.clone()),
                settings_path,
                reports_path: learn::reports_path(&data_dir),
                result_scope: Mutex::new(scope::State::default()),
                next_plan: AtomicU64::new(1),
                active_search: Mutex::new(None),
                presentation: Mutex::new(presentation::Presentation::default()),
            };
            app.manage(state);

            // Configure Writing Tools before WKWebView copies its configuration.
            // The window is created here rather than automatically from config.
            let window = tauri::WebviewWindowBuilder::from_config(app, &app.config().app.windows[0])?;
            #[cfg(target_os = "macos")]
            let window = {
                let mtm = objc2::MainThreadMarker::new().expect("setup runs on the main thread");
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

            // Points the login item back at this copy, in case another
            // copy (an older install, a development build) took it over.
            if settings.start_at_login && owns_login_item() {
                if let Err(e) = app.autolaunch().enable() {
                    eprintln!("Couldn't update the login item: {e}");
                }
            }

            let handle = app.handle().clone();
            if let Err(e) = register_hotkey(&handle, &settings.hotkey) {
                eprintln!("{e}");
            }

            let show = MenuItem::with_id(app, "show", "Show Voxi", true, None::<&str>)?;
            let prefs = MenuItem::with_id(app, "settings", "Settings…", true, Some("CmdOrCtrl+,"))?;
            let quit = MenuItem::with_id(app, "quit", "Quit Voxi", true, Some("CmdOrCtrl+Q"))?;
            let menu = Menu::with_items(app, &[&show, &prefs, &PredefinedMenuItem::separator(app)?, &quit])?;
            TrayIconBuilder::with_id("voxi")
                .icon(tauri::image::Image::from_bytes(include_bytes!("../icons/tray.png"))?)
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
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            plan_command,
            report_wrong,
            reports_summary,
            clear_reports,
            reveal_reports,
            teach_phrase,
            remember_folder,
            clear_result_scope,
            result_scope,
            confirm_plan,
            cancel_plan,
            search_progress,
            open_hex,
            hide_window,
            dismiss_window,
            fit_window,
            frontend_ready,
            present_window,
            get_settings,
            save_settings,
            allowed_folders,
            request_folder_access,
            open_privacy_settings
        ])
        .run(tauri::generate_context!())
        .expect("error while running Voxi");
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
}
