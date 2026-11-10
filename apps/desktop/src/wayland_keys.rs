//! Global hotkeys on Linux under Wayland.
//!
//! Wayland doesn't let an app grab keys for the whole desktop: the X11 grab the global-shortcut
//! plugin uses only sees keys typed into XWayland windows, so on Wayland it registers fine and
//! then never fires. Instead FastMind asks the desktop's global shortcuts service (the XDG
//! portal, which KDE, GNOME and Hyprland provide) for its two keys. Where the desktop leaves
//! binding to the user (Hyprland), Settings shows the line to add to its config, and
//! `fastmind-desktop --capture` works as a command for any desktop's own key bindings.

use std::sync::atomic::{AtomicU64, Ordering};

use ashpd::desktop::global_shortcuts::{GlobalShortcuts, NewShortcut};
use futures_util::StreamExt;
use serde_json::{Value, json};
use tauri::AppHandle;

/// True in a Wayland session, where X11 key grabs can't see the desktop's keys.
pub fn in_wayland_session() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty())
        || std::env::var("XDG_SESSION_TYPE").is_ok_and(|v| v.eq_ignore_ascii_case("wayland"))
}

/// The id the desktop knows FastMind by: the bundle identifier, as in tauri.conf.json.
pub const APP_ID: &str = "dev.coccor.fastmind";

/// Which binding is current: a rebind (Settings saved) retires the previous listener.
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// The portal session of the current binding, closed when the keys are bound again.
static SESSION: tokio::sync::Mutex<Option<ashpd::desktop::Session<GlobalShortcuts>>> =
    tokio::sync::Mutex::const_new(None);

/// Asks the desktop for FastMind's hotkeys and listens for them. `keys` holds
/// (id, accelerator) pairs, ids being "capture" and "remember".
pub fn bind(app: AppHandle, keys: Vec<(&'static str, String)>) {
    let generation = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    tauri::async_runtime::spawn(async move {
        let status = match bind_portal(&app, generation, &keys).await {
            Ok((bound, can_configure)) => {
                // Once FastMind's block is in Hyprland's config, it follows the keys in Settings.
                let mut problem = None;
                if let Some(config) = crate::hyprland::Config::find()
                    && config.installed()
                    && let Err(error) = config.write(Some(&keys))
                {
                    eprintln!("{error}");
                    problem = Some(error);
                }
                json!({
                    "how": "portal",
                    "bound": bound,
                    "can_configure": can_configure && crate::hyprland::Config::find().is_none(),
                    "hyprland": crate::hyprland::status(&keys),
                    "problem": problem,
                })
            }
            Err(error) => {
                eprintln!("The desktop's global shortcuts service isn't available: {error}");
                json!({ "how": "none", "detail": error.to_string() })
            }
        };
        if GENERATION.load(Ordering::SeqCst) == generation {
            crate::set_hotkey_status(&app, status);
        }
    });
}

/// Binds the keys, returning what the desktop bound and whether it can show its own dialog for
/// changing them (portal version 2).
async fn bind_portal(
    app: &AppHandle,
    generation: u64,
    keys: &[(&'static str, String)],
) -> Result<(Vec<Value>, bool), ashpd::Error> {
    let mut current = SESSION.lock().await;
    if let Some(old) = current.take() {
        let _ = old.close().await;
    }
    if keys.is_empty() {
        return Ok((Vec::new(), false));
    }
    register_app_id().await?;
    let portal = GlobalShortcuts::new().await?;
    let session = portal.create_session(Default::default()).await?;
    let shortcuts: Vec<NewShortcut> = keys
        .iter()
        .map(|(id, accelerator)| {
            let trigger = xdg_trigger(accelerator);
            NewShortcut::new(*id, describe(id)).preferred_trigger(trigger.as_deref())
        })
        .collect();
    // Subscribe before binding, so an activation right after binding isn't missed.
    let mut activated = portal.receive_activated().await?;
    let response = portal
        .bind_shortcuts(&session, &shortcuts, None, Default::default())
        .await?
        .response()?;
    let bound = response
        .shortcuts()
        .iter()
        .map(|s| json!({ "id": s.id(), "trigger": s.trigger_description() }))
        .collect();
    *current = Some(session);
    drop(current);
    let can_configure = portal.version() >= 2;

    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        // Keep the proxy alive with the stream.
        let _portal = portal;
        while let Some(event) = activated.next().await {
            if GENERATION.load(Ordering::SeqCst) != generation {
                break;
            }
            let app = app.clone();
            let id = event.shortcut_id().to_owned();
            let _ = app.clone().run_on_main_thread(move || match id.as_str() {
                "capture" => crate::show_capture(&app),
                "remember" => crate::show_main(&app, Some("remember")),
                _ => {}
            });
        }
    });
    Ok((bound, can_configure))
}

/// Opens the desktop's own dialog for changing FastMind's shortcuts (KDE, GNOME).
pub async fn configure() -> Result<(), String> {
    let current = SESSION.lock().await;
    let session = current
        .as_ref()
        .ok_or("FastMind's shortcuts aren't registered with the desktop")?;
    let portal = GlobalShortcuts::new().await.map_err(|e| e.to_string())?;
    portal
        .configure_shortcuts(session, None, Default::default())
        .await
        .map_err(|e| e.to_string())
}

/// Outside a sandbox the portal can't tell which app is asking, and refuses global shortcuts
/// ("An app id is required") until the process registers its id, once, before other requests.
/// The portal only accepts an id it finds a desktop entry for, so FastMind writes its own first.
async fn register_app_id() -> Result<(), ashpd::Error> {
    static REGISTERED: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();
    REGISTERED
        .get_or_try_init(|| async {
            if let Err(error) = write_desktop_entry() {
                eprintln!("Couldn't write FastMind's desktop entry: {error}");
            }
            let id = ashpd::AppID::try_from(APP_ID)?;
            ashpd::register_host_app(id).await
        })
        .await
        .map(|_| ())
}

/// Marks the desktop entry FastMind writes itself, so it never touches one an installer (or
/// the user) put there.
const GENERATED: &str = "X-FastMind-Generated=true";

/// Makes sure the portal finds a `dev.coccor.fastmind.desktop`. An installed one (in any of the
/// XDG data folders) is left alone. Otherwise FastMind writes one to
/// `~/.local/share/applications`, hidden from menus and pointing at this binary, and rewrites
/// it only when its text changes, so it follows the binary when FastMind moves.
fn write_desktop_entry() -> std::io::Result<()> {
    let home_data = std::env::var_os("XDG_DATA_HOME")
        .filter(|v| !v.is_empty())
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| std::path::Path::new(&home).join(".local/share"))
        })
        .ok_or_else(|| std::io::Error::other("no home folder"))?;
    let system_data = std::env::var("XDG_DATA_DIRS")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    let name = format!("{APP_ID}.desktop");
    let path = home_data.join("applications").join(&name);
    let installed = std::iter::once(home_data.clone())
        .chain(std::env::split_paths(&system_data))
        .map(|dir| dir.join("applications").join(&name))
        .filter_map(|entry| std::fs::read_to_string(entry).ok())
        .any(|text| !text.contains(GENERATED));
    if installed {
        return Ok(());
    }
    let exe = std::env::current_exe()?;
    let text = desktop_entry(&exe.to_string_lossy());
    if std::fs::read_to_string(&path).is_ok_and(|current| current == text) {
        return Ok(());
    }
    std::fs::create_dir_all(path.parent().expect("has a parent"))?;
    std::fs::write(&path, text)
}

fn desktop_entry(exe: &str) -> String {
    // The Exec key quotes arguments with double quotes, escaping `"`, `` ` ``, `$` and `\`; then
    // the whole value is a desktop-entry string, which escapes each `\` once more.
    let mut quoted = String::from("\"");
    for c in exe.chars() {
        if matches!(c, '"' | '`' | '$' | '\\') {
            quoted.push('\\');
        }
        quoted.push(c);
    }
    quoted.push('"');
    let quoted = quoted.replace('\\', "\\\\");
    format!(
        "[Desktop Entry]\nType=Application\nName=FastMind\n\
         Comment=Lets the desktop's shortcuts service know FastMind\n\
         Exec={quoted}\nIcon=fastmind-desktop\nNoDisplay=true\nTerminal=false\n{GENERATED}\n"
    )
}

fn describe(id: &str) -> &'static str {
    match id {
        "capture" => "Capture a thought",
        _ => "Search your notes",
    }
}

/// The parts of an accelerator ("CommandOrControl+Alt+Space"): modifiers, then the key.
pub fn split(accelerator: &str) -> Option<(Vec<&'static str>, String)> {
    let mut modifiers = Vec::new();
    let mut key = None;
    for part in accelerator.split('+').map(str::trim) {
        let modifier = match part.to_ascii_lowercase().as_str() {
            "commandorcontrol" | "cmdorctrl" | "commandorctrl" | "cmdorcontrol" | "control"
            | "ctrl" => Some("CTRL"),
            "alt" | "option" | "altgr" => Some("ALT"),
            "shift" => Some("SHIFT"),
            "super" | "meta" | "command" | "cmd" | "logo" | "win" => Some("LOGO"),
            _ => None,
        };
        match modifier {
            Some(m) if !modifiers.contains(&m) => modifiers.push(m),
            Some(_) => {}
            None if !known_key(part) || key.is_some() => return None,
            None => key = Some(part.to_owned()),
        }
    }
    Some((modifiers, key?))
}

/// The keys FastMind takes in a hotkey: a letter or digit, F1 to F24, or one of the named keys
/// `keysym` knows. Anything else is refused, since hotkeys end up in other programs' config
/// (Hyprland's), where a stray quote would be code.
const NAMED_KEYS: &[&str] = &[
    "space",
    "enter",
    "return",
    "esc",
    "escape",
    "tab",
    "backspace",
    "delete",
    "del",
    "insert",
    "home",
    "end",
    "pageup",
    "pagedown",
    "up",
    "arrowup",
    "down",
    "arrowdown",
    "left",
    "arrowleft",
    "right",
    "arrowright",
    "minus",
    "equal",
    "comma",
    "period",
    "slash",
    "semicolon",
    "quote",
    "bracketleft",
    "bracketright",
    "backslash",
    "backquote",
];

fn known_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    let function_key = lower
        .strip_prefix('f')
        .and_then(|n| n.parse::<u8>().ok())
        .is_some_and(|n| (1..=24).contains(&n));
    (key.len() == 1 && key.chars().all(|c| c.is_ascii_alphanumeric()))
        || function_key
        || NAMED_KEYS.contains(&lower.as_str())
}

/// An accelerator as the XDG shortcuts spec writes triggers: "CTRL+ALT+space".
pub fn xdg_trigger(accelerator: &str) -> Option<String> {
    let (modifiers, key) = split(accelerator)?;
    let key = keysym(&key);
    let mut parts: Vec<&str> = modifiers;
    parts.push(&key);
    Some(parts.join("+"))
}

/// The xkb keysym name for a key as accelerators write it.
pub fn keysym(key: &str) -> String {
    let lower = key.to_ascii_lowercase();
    match lower.as_str() {
        "space" => "space".into(),
        "enter" | "return" => "Return".into(),
        "esc" | "escape" => "Escape".into(),
        "tab" => "Tab".into(),
        "backspace" => "BackSpace".into(),
        "delete" | "del" => "Delete".into(),
        "insert" => "Insert".into(),
        "home" => "Home".into(),
        "end" => "End".into(),
        "pageup" => "Prior".into(),
        "pagedown" => "Next".into(),
        "up" | "arrowup" => "Up".into(),
        "down" | "arrowdown" => "Down".into(),
        "left" | "arrowleft" => "Left".into(),
        "right" | "arrowright" => "Right".into(),
        "minus" => "minus".into(),
        "equal" => "equal".into(),
        "comma" => "comma".into(),
        "period" => "period".into(),
        "slash" => "slash".into(),
        "semicolon" => "semicolon".into(),
        "quote" => "apostrophe".into(),
        "bracketleft" => "bracketleft".into(),
        "bracketright" => "bracketright".into(),
        "backslash" => "backslash".into(),
        "backquote" => "grave".into(),
        _ if key.chars().count() == 1 => lower,
        _ if lower.starts_with('f') && lower[1..].parse::<u8>().is_ok() => key.to_ascii_uppercase(),
        _ => key.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn triggers_follow_the_xdg_shortcuts_spec() {
        assert_eq!(
            xdg_trigger("CommandOrControl+Alt+Space").as_deref(),
            Some("CTRL+ALT+space")
        );
        assert_eq!(
            xdg_trigger("CommandOrControl+Alt+R").as_deref(),
            Some("CTRL+ALT+r")
        );
        assert_eq!(
            xdg_trigger("Super+Shift+F5").as_deref(),
            Some("LOGO+SHIFT+F5")
        );
        assert_eq!(xdg_trigger("Ctrl+Enter").as_deref(), Some("CTRL+Return"));
        assert_eq!(
            xdg_trigger("Ctrl+Alt+Comma").as_deref(),
            Some("CTRL+ALT+comma")
        );
        assert_eq!(xdg_trigger("Ctrl+Alt"), None);
        assert_eq!(xdg_trigger("Ctrl+A+B"), None);
        assert_eq!(xdg_trigger("Ctrl+F25"), None);
        assert_eq!(xdg_trigger("Ctrl+Alt+X\"))os.execute(\"id\")--"), None);
        assert_eq!(xdg_trigger("Ctrl+Alt+Nope"), None);
    }

    #[test]
    fn the_desktop_entry_quotes_the_binary() {
        let entry = desktop_entry("/opt/Fast Mind/$bin\\fastmind-desktop");
        // `$` → `\$` → `\\$`, and `\` → `\\` → `\\\\`.
        assert!(entry.contains(
            r#"Exec="/opt/Fast Mind/\\$bin\\\\fastmind-desktop"
"#
        ));
        assert!(entry.contains("NoDisplay=true\n"));
        assert!(entry.contains(GENERATED));
    }
}
