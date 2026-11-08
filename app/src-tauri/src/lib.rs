mod exec;
mod fsx;
mod history;
mod model;
mod parse;
mod pdf;
mod plan;
mod presentation;
mod settings;
mod search;
mod trash;

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
}

struct AppState {
    roots: RwLock<Roots>,
    history: Mutex<History>,
    pending: Mutex<Option<Pending>>,
    settings: Mutex<Settings>,
    settings_path: PathBuf,
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
    Roots::new(&h, screenshot_location(), &settings.extra_paths(&h))
}

// ---- Window ------------------------------------------------------------------

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
    if app.state::<AppState>().presentation.lock().unwrap().pending.is_some()
        || (w.is_visible().unwrap_or(false) && w.is_focused().unwrap_or(false)) {
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
fn plan_command(state: State<AppState>, text: String, choices: HashMap<String, String>, request_id: u64) -> Outcome {
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
        None => match parse::parse(&text) {
            Ok(intent) => Work::Command(intent),
            Err(raw) => return Outcome::NotUnderstood { text: raw },
        },
    };
    let roots = state.roots.read().unwrap().clone();
    let choices: HashMap<String, PathBuf> = choices.into_iter().map(|(k, v)| (k, PathBuf::from(v))).collect();
    let planned = {
        let history = state.history.lock().unwrap();
        let planner = Planner::new(&roots, &choices).with_control(control.clone());
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
            candidates: candidates.iter().map(|p| p.to_string_lossy().into_owned()).collect(),
        },
        Ok(Planned::Info { title, summary, rows, total }) => Outcome::Info { title, summary, rows, total },
        Ok(Planned::Direct { action, title }) => {
            let out = exec::run(std::slice::from_ref(&action), &roots);
            match out.error {
                Some(message) => Outcome::Problem { message },
                None => Outcome::Info { title, summary: String::new(), rows: vec![], total: 0 },
            }
        }
        Ok(Planned::Steps { title, actions, notes, warnings, undo_of }) => {
            let id = state.next_plan.fetch_add(1, Ordering::SeqCst);
            let items = preview(&actions, &notes, &roots);
            let total = actions.len();
            *state.pending.lock().unwrap() = Some(Pending { id, command: text.trim().to_string(), actions, undo_of });
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
    let roots = state.roots.read().unwrap().clone();
    let out = exec::run(&p.actions, &roots);
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
    });
}

#[tauri::command]
fn hide_window(app: AppHandle) {
    hide(&app);
}

#[tauri::command]
fn get_settings(state: State<AppState>) -> Settings {
    state.settings.lock().unwrap().clone()
}

#[tauri::command]
fn save_settings(app: AppHandle, state: State<AppState>, mut settings: Settings) -> Result<Settings, String> {
    settings.validate_shortcuts()?;
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
    Ok(settings)
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
            confirm_plan,
            cancel_plan,
            search_progress,
            open_hex,
            hide_window,
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
