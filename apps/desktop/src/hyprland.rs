//! Hyprland registers global shortcuts from the portal but leaves their keys to the user's
//! config. Rather than edit that config, FastMind binds its keys live over Hyprland's IPC while
//! it runs (`hyprctl eval` with a Lua config, `hyprctl keyword` with a classic one), binds them
//! again after each config reload (a reload drops runtime binds), and unbinds them when it quits.
//! Nothing is written to the user's files.

use std::io::{BufRead, BufReader};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;
use std::time::Duration;

use serde_json::{Value, json};

use crate::wayland_keys::{APP_ID, keysym, split};

/// Binds FastMind makes carry this description, which tells them from the user's own.
const DESCRIPTION: &str = "FastMind";

/// The binds FastMind made and hasn't taken back, to unbind exactly those.
static BOUND: Mutex<Vec<Bind>> = Mutex::new(Vec::new());

/// True when FastMind runs under Hyprland.
pub fn running() -> bool {
    std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some_and(|v| !v.is_empty())
}

/// Hyprland reads a Lua config since 0.56 when `hyprland.lua` exists; IPC then takes Lua
/// (`eval`), and `keyword` is refused.
fn lua_config() -> bool {
    std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .is_some_and(|dir| dir.join("hypr").join("hyprland.lua").exists())
}

/// One key bind, in the forms both config styles take.
#[derive(Debug, Clone, PartialEq)]
struct Bind {
    id: &'static str,
    /// "CTRL ALT" and "SPACE", as `bind =` lines write them.
    modifiers: String,
    key: String,
    /// The modifier mask `hyprctl binds` reports, to find binds already on these keys.
    mask: u64,
    accelerator: String,
}

impl Bind {
    fn new(id: &'static str, accelerator: &str) -> Option<Bind> {
        let (modifiers, key) = split(accelerator)?;
        let mut mask = 0;
        let modifiers: Vec<&str> = modifiers
            .iter()
            .map(|m| {
                let (name, bit) = match *m {
                    "SHIFT" => ("SHIFT", 1),
                    "CTRL" => ("CTRL", 4),
                    "ALT" => ("ALT", 8),
                    _ => ("SUPER", 64),
                };
                mask |= bit;
                name
            })
            .collect();
        // xkb's names, as Hyprland reads them: SPACE, R, comma, Return.
        let key = match keysym(&key) {
            name if name.chars().count() == 1 || name == "space" => name.to_ascii_uppercase(),
            name => name,
        };
        Some(Bind {
            id,
            modifiers: modifiers.join(" "),
            key,
            mask,
            accelerator: accelerator.to_owned(),
        })
    }

    fn description(&self) -> String {
        let what = if self.id == "capture" {
            "Capture"
        } else {
            "Remember"
        };
        format!("{DESCRIPTION}: {what}")
    }

    /// "CTRL + ALT + SPACE", as Lua binds write the chord.
    fn chord(&self) -> String {
        self.modifiers
            .split(' ')
            .filter(|m| !m.is_empty())
            .chain([self.key.as_str()])
            .collect::<Vec<_>>()
            .join(" + ")
    }

    /// The hyprctl arguments that make this bind.
    fn bind_args(&self, lua: bool) -> [String; 2] {
        if lua {
            [
                "eval".into(),
                format!(
                    "hl.bind(\"{}\", hl.dsp.global(\"{APP_ID}:{}\"), {{ description = \"{}\" }})",
                    self.chord(),
                    self.id,
                    self.description()
                ),
            ]
        } else {
            [
                "keyword".into(),
                format!(
                    "bindd {}, {}, {}, global, {APP_ID}:{}",
                    self.modifiers,
                    self.key,
                    self.description(),
                    self.id
                ),
            ]
        }
    }

    /// The hyprctl arguments that take this bind back.
    fn unbind_args(&self, lua: bool) -> [String; 2] {
        if lua {
            ["eval".into(), format!("hl.unbind(\"{}\")", self.chord())]
        } else {
            [
                "keyword".into(),
                format!("unbind {}, {}", self.modifiers, self.key),
            ]
        }
    }
}

fn hyprctl(args: &[String]) -> Result<(), String> {
    let output = Command::new("hyprctl")
        .args(args)
        .output()
        .map_err(|e| format!("couldn't run hyprctl: {e}"))?;
    let said = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if output.status.success() && (said.is_empty() || said == "ok") {
        Ok(())
    } else {
        Err(format!("Hyprland refused the bind: {said}"))
    }
}

/// Takes back the binds FastMind made.
pub fn clear() {
    let lua = lua_config();
    let mut bound = BOUND.lock().expect("bound lock");
    for bind in bound.drain(..) {
        let _ = hyprctl(&bind.unbind_args(lua));
    }
}

/// After a config reload Hyprland has dropped the runtime binds itself: forget them, so a later
/// unbind can't hit a bind the user has since put on the same keys.
fn forget() {
    BOUND.lock().expect("bound lock").clear();
}

/// Binds these (id, accelerator) keys in place of FastMind's previous ones, leaving out keys the
/// user already bound. Returns what Settings shows.
pub fn apply(keys: &[(&'static str, String)]) -> Value {
    clear();
    let lua = lua_config();
    let binds: Vec<Bind> = keys.iter().filter_map(|(id, a)| Bind::new(id, a)).collect();
    let existing = current_binds();
    let mut taken = Vec::new();
    let mut problems = Vec::new();
    let mut bound = BOUND.lock().expect("bound lock");
    for bind in binds {
        if let Some(what) = taken_by(&existing, &bind) {
            taken.push(format!(
                "{} is already {what}",
                shown_key(&bind.accelerator)
            ));
            continue;
        }
        match hyprctl(&bind.bind_args(lua)) {
            Ok(()) => bound.push(bind),
            Err(error) => problems.push(error),
        }
    }
    json!({
        "active": !bound.is_empty(),
        "conflicts": taken,
        "problems": problems,
    })
}

fn current_binds() -> Vec<Value> {
    Command::new("hyprctl")
        .args(["binds", "-j"])
        .output()
        .ok()
        .and_then(|output| serde_json::from_slice(&output.stdout).ok())
        .unwrap_or_default()
}

/// What the user's own bind on these keys does, if there is one.
fn taken_by(binds: &[Value], bind: &Bind) -> Option<String> {
    binds.iter().find_map(|existing| {
        let same_key = existing["key"]
            .as_str()
            .is_some_and(|k| k.eq_ignore_ascii_case(&bind.key));
        let ours = existing["description"]
            .as_str()
            .is_some_and(|d| d.starts_with(DESCRIPTION));
        (existing["modmask"].as_u64() == Some(bind.mask) && same_key && !ours).then(|| {
            existing["description"]
                .as_str()
                .filter(|d| !d.is_empty())
                .or_else(|| existing["arg"].as_str().filter(|a| !a.is_empty()))
                .unwrap_or("another bind")
                .to_owned()
        })
    })
}

/// An accelerator as people write it: "Ctrl+Alt+Space".
fn shown_key(accelerator: &str) -> String {
    accelerator
        .split('+')
        .map(|part| match part.trim().to_ascii_lowercase().as_str() {
            "commandorcontrol" | "cmdorctrl" | "commandorctrl" | "cmdorcontrol" | "control" => {
                "Ctrl".to_owned()
            }
            _ => part.trim().to_owned(),
        })
        .collect::<Vec<_>>()
        .join("+")
}

/// Listens on Hyprland's event socket and calls `rebind` after every config reload, which drops
/// runtime binds. Reconnects if the socket closes.
pub fn watch_reloads(rebind: impl Fn() + Send + 'static) {
    let Some(socket) = event_socket() else {
        return;
    };
    std::thread::spawn(move || {
        loop {
            if let Ok(stream) = UnixStream::connect(&socket) {
                for line in BufReader::new(stream).lines() {
                    let Ok(line) = line else { break };
                    if line.starts_with("configreloaded>>") {
                        forget();
                        rebind();
                    }
                }
            }
            std::thread::sleep(Duration::from_secs(5));
        }
    });
}

fn event_socket() -> Option<PathBuf> {
    let signature = std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE")?;
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")?;
    Some(
        PathBuf::from(runtime)
            .join("hypr")
            .join(signature)
            .join(".socket2.sock"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bind(id: &'static str, accelerator: &str) -> Bind {
        Bind::new(id, accelerator).unwrap()
    }

    #[test]
    fn binds_in_lua_and_classic_form() {
        let capture = bind("capture", "CommandOrControl+Alt+Space");
        assert_eq!(
            capture.bind_args(true)[1],
            r#"hl.bind("CTRL + ALT + SPACE", hl.dsp.global("dev.coccor.fastmind:capture"), { description = "FastMind: Capture" })"#
        );
        assert_eq!(
            capture.unbind_args(true)[1],
            r#"hl.unbind("CTRL + ALT + SPACE")"#
        );
        assert_eq!(
            capture.bind_args(false)[1],
            "bindd CTRL ALT, SPACE, FastMind: Capture, global, dev.coccor.fastmind:capture"
        );
        assert_eq!(capture.unbind_args(false)[1], "unbind CTRL ALT, SPACE");
        let remember = bind("remember", "Super+Shift+R");
        assert_eq!(remember.chord(), "SUPER + SHIFT + R");
        assert_eq!(remember.mask, 65);
    }

    #[test]
    fn keys_use_xkb_names() {
        assert_eq!(bind("capture", "Ctrl+Comma").chord(), "CTRL + comma");
        assert_eq!(bind("capture", "Ctrl+Enter").chord(), "CTRL + Return");
    }

    #[test]
    fn nothing_but_known_keys_reaches_hyprland() {
        for bad in [
            "Ctrl+Alt+X\")) os.execute(\"touch /tmp/pwned\") --",
            "Ctrl+Alt+SPACE, exec, rm",
            "Ctrl+Alt+Space\nexec-once = x",
        ] {
            assert_eq!(Bind::new("capture", bad), None, "{bad:?}");
        }
    }

    #[test]
    fn the_users_binds_are_left_alone() {
        let binds = vec![
            json!({ "modmask": 12, "key": "space", "description": "Terminal", "arg": "" }),
            json!({ "modmask": 12, "key": "R", "description": "FastMind: Remember", "arg": "" }),
            json!({ "modmask": 65, "key": "R", "description": "", "arg": "hyprctl reload" }),
        ];
        assert_eq!(
            taken_by(&binds, &bind("capture", "CommandOrControl+Alt+Space")).as_deref(),
            Some("Terminal")
        );
        assert_eq!(taken_by(&binds, &bind("remember", "Ctrl+Alt+R")), None);
        assert_eq!(
            taken_by(&binds, &bind("remember", "Super+Shift+R")).as_deref(),
            Some("hyprctl reload")
        );
        assert_eq!(shown_key("CommandOrControl+Alt+Space"), "Ctrl+Alt+Space");
    }
}
