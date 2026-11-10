//! FastMind for the desktop: one process in the tray (menu bar on macOS) that owns the hotkeys,
//! the capture window, the main window and Dream's nightly run. All the work happens in
//! `fastmind-core`; this file wires it to the operating system.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use chrono::{Local, NaiveTime};
use fastmind_core::api::App;
use fastmind_core::config::Settings;
use fastmind_core::dream;
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

#[cfg(target_os = "linux")]
mod wayland_keys;

struct Core(Arc<App>);

/// How the hotkeys are wired, for Settings: grabbed by FastMind ("native"), asked of the
/// desktop's shortcuts service on Wayland ("portal"), or not available ("none").
#[derive(Default)]
struct HotkeyStatus(std::sync::Mutex<Value>);

fn set_hotkey_status(app: &AppHandle, status: Value) {
    *app.state::<HotkeyStatus>()
        .0
        .lock()
        .expect("hotkey status lock") = status;
    let _ = app.emit("hotkeys-changed", ());
}

#[tauri::command]
fn hotkey_status(status: tauri::State<'_, HotkeyStatus>) -> Value {
    status.0.lock().expect("hotkey status lock").clone()
}

/// What a launch asks for: `--capture` opens the capture box, `--remember` the search, so a
/// desktop's own key bindings can run `fastmind-desktop --capture`. A second launch hands its
/// arguments to the running FastMind.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Launch {
    Main,
    Capture,
    Remember,
}

fn launch_from(args: &[String]) -> Launch {
    if args.iter().any(|a| a == "--capture") {
        Launch::Capture
    } else if args.iter().any(|a| a == "--remember") {
        Launch::Remember
    } else {
        Launch::Main
    }
}

fn open_for(app: &AppHandle, launch: Launch) {
    match launch {
        Launch::Main => show_main(app, None),
        Launch::Capture => show_capture(app),
        Launch::Remember => show_main(app, Some("remember")),
    }
}

/// The one command the UI calls: every FastMind command by name, run off the UI thread.
#[tauri::command]
async fn call(
    core: tauri::State<'_, Core>,
    app: AppHandle,
    name: String,
    args: Value,
) -> Result<Value, String> {
    let core = core.0.clone();
    let changes = matches!(
        name.as_str(),
        "capture"
            | "dream_run"
            | "dream_apply"
            | "dream_apply_all"
            | "dream_undo"
            | "undo"
            | "archive_restore"
            | "archive_note"
            | "inbox_keep"
            | "demo_create"
            | "settings_set"
    );
    let from_capture_window = name == "capture";
    let settings_changed = name == "settings_set";
    let result = tauri::async_runtime::spawn_blocking(move || core.call(&name, args))
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.0);
    if result.is_ok() && changes {
        // The other window refreshes; the capture window's save shows up in the main one.
        let _ = app.emit("changed", ());
        if from_capture_window {
            update_tray(&app);
        }
        if settings_changed {
            apply_autostart(&app);
            let settings = app.state::<Core>().0.settings();
            register_hotkeys(&app, &settings);
        }
    }
    result
}

/// Starts FastMind with the computer (hidden, in the tray) when Settings say so.
fn apply_autostart(app: &AppHandle) {
    use tauri_plugin_autostart::ManagerExt;
    let wanted = app.state::<Core>().0.settings().autostart;
    let launcher = app.autolaunch();
    let result = if wanted {
        launcher.enable()
    } else if launcher.is_enabled().unwrap_or(false) {
        launcher.disable()
    } else {
        Ok(())
    };
    if let Err(error) = result {
        eprintln!("Couldn't change starting with the computer: {error}");
    }
}

/// Opens a note in the system's editor for Markdown. Only files inside the notebook.
#[tauri::command]
fn open_path(core: tauri::State<'_, Core>, path: String) -> Result<(), String> {
    let notebook = core.0.notebook().map_err(|e| e.0)?;
    let path = std::path::PathBuf::from(path)
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let root = notebook.root.canonicalize().map_err(|e| e.to_string())?;
    if !path.starts_with(&root) {
        return Err("only notes inside the notebook can be opened".into());
    }
    opener::open(&path).map_err(|e| e.to_string())
}

/// A folder picker for choosing the notebook.
#[tauri::command]
async fn pick_folder(app: AppHandle) -> Option<String> {
    use tauri_plugin_dialog::DialogExt;
    let (send, receive) = std::sync::mpsc::channel();
    app.dialog()
        .file()
        .set_title("Choose the folder for your notes")
        .pick_folder(move |folder| {
            let _ = send.send(
                folder
                    .and_then(|f| f.into_path().ok())
                    .map(|p| p.to_string_lossy().into_owned()),
            );
        });
    tauri::async_runtime::spawn_blocking(move || receive.recv().ok().flatten())
        .await
        .ok()
        .flatten()
}

#[tauri::command]
fn hide_capture(app: AppHandle) {
    if let Some(window) = app.get_webview_window("capture") {
        let _ = window.hide();
    }
}

fn show_main(app: &AppHandle, view: Option<&str>) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        match view {
            Some("remember") => {
                let _ = app.emit("focus-remember", ());
            }
            Some(view) => {
                let _ = app.emit("show-view", view);
            }
            None => {}
        }
    }
}

/// The capture box's size, in logical pixels.
const CAPTURE_SIZE: (f64, f64) = (560.0, 300.0);

fn show_capture(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("capture") {
        if window.is_visible().unwrap_or(false) && window.is_focused().unwrap_or(false) {
            let _ = window.hide();
            return;
        }
        let _ = window.center();
        let _ = window.show();
        let _ = window.set_focus();
        let _ = app.emit_to("capture", "capture-shown", ());
    }
}

fn open_notebook_folder(app: &AppHandle) {
    let core = app.state::<Core>();
    if let Ok(notebook) = core.0.notebook() {
        let _ = opener::open(&notebook.root);
    }
}

/// Runs Dream in the background and tells the windows when it's done.
fn dream_now(app: &AppHandle) {
    let core = app.state::<Core>().0.clone();
    let app = app.clone();
    std::thread::spawn(move || {
        let result = core.call("dream_run", json!({}));
        let _ = app.emit("changed", ());
        update_tray(&app);
        if let Err(error) = result {
            eprintln!("Dream failed: {error}");
        }
    });
}

/// The tray tooltip says what's waiting: inbox notes and Dream's proposals.
fn update_tray(app: &AppHandle) {
    let core = app.state::<Core>().0.clone();
    let Some(tray) = app.tray_by_id("fastmind") else {
        return;
    };
    let tip = match core.call("status", json!({})) {
        Ok(status) => {
            let inbox = status["inbox"].as_u64().unwrap_or(0);
            let pending = status["dream"]["pending"].as_u64().unwrap_or(0);
            let mut parts = vec![format!("{} notes", status["notes"].as_u64().unwrap_or(0))];
            if inbox > 0 {
                parts.push(format!("{inbox} in the inbox"));
            }
            if pending > 0 {
                parts.push(format!("Dream has {pending} proposals"));
            }
            format!("FastMind · {}", parts.join(" · "))
        }
        Err(_) => "FastMind".to_owned(),
    };
    let _ = tray.set_tooltip(Some(tip));
}

fn within(now: NaiveTime, start: &str, end: &str) -> bool {
    let (Ok(start), Ok(end)) = (
        NaiveTime::parse_from_str(start, "%H:%M"),
        NaiveTime::parse_from_str(end, "%H:%M"),
    ) else {
        return false;
    };
    if start <= end {
        now >= start && now < end
    } else {
        now >= start || now < end
    }
}

/// Once a night, inside the window set in Settings, Dream looks over the notebook. It only
/// proposes, apart from the kinds the user lets it apply on its own.
fn nightly(app: AppHandle) {
    std::thread::spawn(move || {
        let mut last_import: Option<std::time::Instant> = None;
        loop {
            // Captures saved while FastMind was closed, or before a model was chosen, are
            // written now (the last week's, and only notes nobody has edited since).
            let core = app.state::<Core>().0.clone();
            let _ = core.call("write_pending", json!({}));
            std::thread::sleep(Duration::from_secs(300));
            let settings = core.settings();
            // AI sessions arrive through the day: import them every half hour, so they're
            // searchable the same afternoon.
            let due = last_import.is_none_or(|at| at.elapsed() >= Duration::from_secs(30 * 60));
            if settings.ai.enabled && due {
                last_import = Some(std::time::Instant::now());
                if let Ok(report) = core.call("ai_import", json!({}))
                    && report["added"]
                        .as_array()
                        .is_some_and(|added| !added.is_empty())
                {
                    let _ = app.emit("changed", ());
                    update_tray(&app);
                }
            }
            if !settings.dream.nightly {
                continue;
            }
            let now = Local::now();
            if !within(
                now.time(),
                &settings.dream.window_start,
                &settings.dream.window_end,
            ) {
                continue;
            }
            let Ok(notebook) = core.notebook() else {
                continue;
            };
            let dreamed_recently = dream::latest(&notebook)
                .is_some_and(|report| now - report.ran_at < chrono::Duration::hours(12));
            if dreamed_recently {
                continue;
            }
            if let Ok(result) = core.call("dream_run", json!({})) {
                notify_dream(&app, &result["report"]);
            }
            let _ = app.emit("changed", ());
            update_tray(&app);
        }
    });
}

/// The shortcuts in use, read by the handler, so changing them in Settings works at once.
#[derive(Default)]
struct Keys(std::sync::Mutex<(Option<Shortcut>, Option<Shortcut>)>);

/// After a nightly Dream with something to review: one quiet notification, read in the morning.
fn notify_dream(app: &AppHandle, report: &Value) {
    use tauri_plugin_notification::NotificationExt;
    let proposals = report["proposals"].as_array().cloned().unwrap_or_default();
    let pending = proposals
        .iter()
        .filter(|p| p["status"] == "pending")
        .count();
    if pending == 0 {
        return;
    }
    let before = report["before"]["notes"].as_u64().unwrap_or(0);
    let removed: u64 = proposals
        .iter()
        .map(|p| p["notes_removed"].as_u64().unwrap_or(0))
        .sum();
    let _ = app
        .notification()
        .builder()
        .title("While you slept")
        .body(format!(
            "Dream can take your notebook from {before} to {} notes. {pending} proposal{} to review.",
            before.saturating_sub(removed),
            if pending == 1 { "" } else { "s" }
        ))
        .show();
}

fn register_hotkeys(app: &AppHandle, settings: &Settings) {
    let shortcuts = app.global_shortcut();
    let _ = shortcuts.unregister_all();
    #[cfg(target_os = "linux")]
    if wayland_keys::in_wayland_session() {
        let keys = [
            ("capture", &settings.hotkeys.capture),
            ("remember", &settings.hotkeys.remember),
        ]
        .into_iter()
        .filter(|(_, keys)| !keys.trim().is_empty())
        .map(|(id, keys)| (id, keys.trim().to_owned()))
        .collect();
        wayland_keys::bind(app.clone(), keys);
        return;
    }
    let mut active = (None, None);
    let mut problems = Vec::new();
    for (name, keys) in [
        ("capture", &settings.hotkeys.capture),
        ("remember", &settings.hotkeys.remember),
    ] {
        if keys.trim().is_empty() {
            continue;
        }
        match keys.parse::<Shortcut>() {
            Ok(shortcut) => match shortcuts.register(shortcut) {
                Ok(()) => {
                    if name == "capture" {
                        active.0 = Some(shortcut);
                    } else {
                        active.1 = Some(shortcut);
                    }
                }
                Err(error) => {
                    let problem = format!("The {name} key {keys} is taken or not allowed: {error}");
                    eprintln!("{problem}");
                    problems.push(problem);
                }
            },
            Err(error) => {
                let problem = format!("The {name} key {keys} isn't a valid shortcut: {error}");
                eprintln!("{problem}");
                problems.push(problem);
            }
        }
    }
    *app.state::<Keys>().0.lock().expect("keys lock") = active;
    set_hotkey_status(app, json!({ "how": "native", "problems": problems }));
}

fn main() {
    let settings = Settings::load();
    let args: Vec<String> = std::env::args().collect();
    let start_hidden = args.iter().any(|arg| arg == "--hidden");
    let launch = launch_from(&args);
    let core = Arc::new(App::new(settings.clone()));

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            open_for(app, launch_from(&args))
        }))
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--hidden"]),
        ))
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(move |app, shortcut, event| {
                    if event.state() != ShortcutState::Pressed {
                        return;
                    }
                    let (capture, remember) = *app.state::<Keys>().0.lock().expect("keys lock");
                    if Some(*shortcut) == capture {
                        show_capture(app);
                    } else if Some(*shortcut) == remember {
                        show_main(app, Some("remember"));
                    }
                })
                .build(),
        )
        .manage(Core(core))
        .manage(Keys::default())
        .manage(HotkeyStatus::default())
        .invoke_handler(tauri::generate_handler![
            call,
            hotkey_status,
            open_path,
            hide_capture,
            pick_folder
        ])
        .setup(move |app| {
            let handle = app.handle().clone();

            // On Linux the smallest and largest sizes are the same: tiling window managers
            // (Hyprland, Sway) float a window whose size can't change, and would otherwise tile
            // the capture box like any window.
            let (min, max) = if cfg!(target_os = "linux") {
                (CAPTURE_SIZE, Some(CAPTURE_SIZE))
            } else {
                ((360.0, 200.0), None)
            };
            let mut capture_window = WebviewWindowBuilder::new(
                app,
                "capture",
                WebviewUrl::App("index.html?mode=capture".into()),
            )
            .title("FastMind · Capture")
            .inner_size(CAPTURE_SIZE.0, CAPTURE_SIZE.1)
            .min_inner_size(min.0, min.1);
            if let Some(max) = max {
                capture_window = capture_window.max_inner_size(max.0, max.1);
            }
            capture_window
                // Borderless: the page draws its own title row, which moves the window. The system
                // shadow stays (and Windows 11 rounds the corners).
                .decorations(false)
                .shadow(true)
                .resizable(true)
                .always_on_top(true)
                .skip_taskbar(true)
                .visible(false)
                .center()
                // Dropped files go to the page, which attaches them to the capture.
                .disable_drag_drop_handler()
                .build()?;

            let capture = MenuItem::with_id(app, "capture", "Capture…", true, None::<&str>)?;
            let remember = MenuItem::with_id(app, "remember", "Remember…", true, None::<&str>)?;
            let dream = MenuItem::with_id(app, "dream", "Dream now", true, None::<&str>)?;
            let show_dream =
                MenuItem::with_id(app, "show-dream", "Dream's proposals", true, None::<&str>)?;
            let folder =
                MenuItem::with_id(app, "folder", "Open notebook folder", true, None::<&str>)?;
            let settings_item =
                MenuItem::with_id(app, "settings", "Settings…", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit FastMind", true, None::<&str>)?;
            let separator = PredefinedMenuItem::separator(app)?;
            let separator2 = PredefinedMenuItem::separator(app)?;
            let menu = Menu::with_items(
                app,
                &[
                    &capture,
                    &remember,
                    &separator,
                    &dream,
                    &show_dream,
                    &separator2,
                    &folder,
                    &settings_item,
                    &quit,
                ],
            )?;
            let icon = tauri::image::Image::from_bytes(include_bytes!("../icons/tray.png"))?;
            TrayIconBuilder::with_id("fastmind")
                .icon(icon)
                .tooltip("FastMind")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "capture" => show_capture(app),
                    "remember" => show_main(app, Some("remember")),
                    "dream" => dream_now(app),
                    "show-dream" => show_main(app, Some("dream")),
                    "folder" => open_notebook_folder(app),
                    "settings" => show_main(app, Some("settings")),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        show_main(tray.app_handle(), None);
                    }
                })
                .build(app)?;

            register_hotkeys(&handle, &settings);
            // Reading a large notebook takes a moment: do it off the startup path, so the tray
            // and hotkeys are there at once and the first search is instant.
            let warm = handle.clone();
            std::thread::spawn(move || update_tray(&warm));
            nightly(handle.clone());
            if launch != Launch::Main || !start_hidden {
                open_for(&handle, launch);
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing a window keeps FastMind running in the tray.
            if let WindowEvent::CloseRequested { api, .. } = event {
                let _ = window.hide();
                api.prevent_close();
            }
        })
        .run(tauri::generate_context!())
        .expect("FastMind failed to start");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dream_window_wraps_midnight() {
        let at = |text: &str| NaiveTime::parse_from_str(text, "%H:%M").unwrap();
        assert!(within(at("03:00"), "02:00", "06:00"));
        assert!(!within(at("07:00"), "02:00", "06:00"));
        assert!(within(at("23:30"), "23:00", "05:00"));
        assert!(within(at("01:00"), "23:00", "05:00"));
        assert!(!within(at("12:00"), "23:00", "05:00"));
    }

    #[test]
    fn launch_flags_pick_the_window() {
        let args = |list: &[&str]| list.iter().map(|a| a.to_string()).collect::<Vec<_>>();
        assert_eq!(launch_from(&args(&["fastmind-desktop"])), Launch::Main);
        assert_eq!(
            launch_from(&args(&["fastmind-desktop", "--capture"])),
            Launch::Capture
        );
        assert_eq!(
            launch_from(&args(&["fastmind-desktop", "--hidden", "--remember"])),
            Launch::Remember
        );
    }
}
