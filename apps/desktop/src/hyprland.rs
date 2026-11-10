//! Hyprland leaves global shortcuts' keys to the user's config. "Set up the hotkeys" in Settings
//! writes them there, in a block FastMind owns and marks, so it never touches anything else:
//! `~/.config/hypr/hyprland.lua` (Hyprland 0.56 and later) or `hyprland.conf`. Once the block is
//! there, changing a key in Settings rewrites it. Each write reloads Hyprland.

use std::path::PathBuf;
use std::process::Command;

use serde_json::{Value, json};

use crate::wayland_keys::{APP_ID, keysym, split};

/// Binds FastMind wrote carry this description, which tells them from the user's own.
const DESCRIPTION: &str = "FastMind";

/// The config file Hyprland reads, when running under Hyprland.
#[derive(Debug, Clone)]
pub struct Config {
    pub path: PathBuf,
    pub lua: bool,
}

impl Config {
    pub fn find() -> Option<Config> {
        std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE")?;
        let dir = std::env::var_os("XDG_CONFIG_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?
            .join("hypr");
        let lua = dir.join("hyprland.lua");
        Some(if lua.exists() {
            Config {
                path: lua,
                lua: true,
            }
        } else {
            Config {
                path: dir.join("hyprland.conf"),
                lua: false,
            }
        })
    }

    /// The file as the user sees it in their home folder, for Settings.
    pub fn shown(&self) -> String {
        let path = self.path.to_string_lossy().into_owned();
        match std::env::var("HOME") {
            Ok(home) if !home.is_empty() && path.starts_with(&home) => {
                format!("~{}", &path[home.len()..])
            }
            _ => path,
        }
    }

    fn markers(&self) -> (&'static str, &'static str) {
        if self.lua {
            (
                "-- >>> FastMind hotkeys: FastMind writes this block, change the keys in its Settings >>>",
                "-- <<< FastMind hotkeys <<<",
            )
        } else {
            (
                "# >>> FastMind hotkeys: FastMind writes this block, change the keys in its Settings >>>",
                "# <<< FastMind hotkeys <<<",
            )
        }
    }

    pub fn installed(&self) -> bool {
        std::fs::read_to_string(&self.path).is_ok_and(|text| text.contains(self.markers().0))
    }

    /// The block's lines for these (id, accelerator) keys.
    pub fn lines(&self, keys: &[(&'static str, String)]) -> Vec<String> {
        keys.iter()
            .filter_map(|(id, accelerator)| line(self.lua, id, accelerator))
            .collect()
    }

    /// The text with the block set to `lines`, or taken out when `lines` is None. Everything
    /// outside the markers stays as it was.
    pub fn with_block(&self, text: &str, lines: Option<&[String]>) -> String {
        let (begin, end) = self.markers();
        let mut out = String::with_capacity(text.len() + 512);
        let mut rest = text;
        // Drop the existing block (and the blank line FastMind put before it).
        if let Some(start) = rest.find(begin) {
            let after = rest[start..]
                .find(end)
                .map(|i| start + i + end.len())
                .unwrap_or(rest.len());
            let mut before = &rest[..start];
            if before.ends_with("\n\n") {
                before = &before[..before.len() - 1];
            }
            out.push_str(before);
            rest = rest[after..].strip_prefix('\n').unwrap_or(&rest[after..]);
            out.push_str(rest);
        } else {
            out.push_str(rest);
        }
        if let Some(lines) = lines {
            if !out.is_empty() && !out.ends_with('\n') {
                out.push('\n');
            }
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(begin);
            out.push('\n');
            for line in lines {
                out.push_str(line);
                out.push('\n');
            }
            out.push_str(end);
            out.push('\n');
        }
        out
    }

    /// Writes the block for `keys` (or removes it) and reloads Hyprland when that changed the
    /// file.
    pub fn write(&self, keys: Option<&[(&'static str, String)]>) -> Result<(), String> {
        if self.write_file(keys)? {
            reload()?;
        }
        Ok(())
    }

    /// Writes the block for `keys` (or removes it), saying whether the file changed. The first
    /// time, the config as it was is kept beside it, ending in `.before-fastmind`.
    fn write_file(&self, keys: Option<&[(&'static str, String)]>) -> Result<bool, String> {
        let text = match std::fs::read_to_string(&self.path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && keys.is_some() => String::new(),
            Err(e) => return Err(format!("couldn't read {}: {e}", self.shown())),
        };
        let lines = keys.map(|keys| self.lines(keys));
        let new = self.with_block(&text, lines.as_deref());
        if new == text {
            return Ok(false);
        }
        if !self.installed() && !text.is_empty() {
            let mut backup = self.path.clone().into_os_string();
            backup.push(".before-fastmind");
            let backup = PathBuf::from(backup);
            if !backup.exists() {
                std::fs::write(&backup, &text)
                    .map_err(|e| format!("couldn't keep a copy of {}: {e}", self.shown()))?;
            }
        }
        // Write beside it, then rename, so Hyprland never reads half a file.
        let mut temp = self.path.clone().into_os_string();
        temp.push(".fastmind-tmp");
        let temp = PathBuf::from(temp);
        std::fs::write(&temp, &new).map_err(|e| format!("couldn't write {}: {e}", self.shown()))?;
        std::fs::rename(&temp, &self.path)
            .map_err(|e| format!("couldn't write {}: {e}", self.shown()))?;
        Ok(true)
    }
}

fn reload() -> Result<(), String> {
    let output = Command::new("hyprctl")
        .arg("reload")
        .output()
        .map_err(|e| format!("couldn't run hyprctl: {e}"))?;
    let said = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if output.status.success() && (said.is_empty() || said == "ok") {
        Ok(())
    } else {
        Err(format!("Hyprland didn't reload: {said}"))
    }
}

/// A bind line, for the Lua config or the classic one.
pub fn line(lua: bool, id: &str, accelerator: &str) -> Option<String> {
    let (modifiers, key) = split(accelerator)?;
    let modifiers: Vec<&str> = modifiers.iter().map(|m| hypr_modifier(m)).collect();
    // xkb's names, as Hyprland reads them: SPACE, R, comma, Return.
    let key = match keysym(&key) {
        name if name.chars().count() == 1 || name == "space" => name.to_ascii_uppercase(),
        name => name,
    };
    let description = format!("{DESCRIPTION}: {}", title(id));
    Some(if lua {
        let mut chord = modifiers;
        chord.push(&key);
        format!(
            "hl.bind(\"{}\", hl.dsp.global(\"{APP_ID}:{id}\"), {{ description = \"{description}\" }})",
            chord.join(" + ")
        )
    } else {
        format!(
            "bindd = {}, {key}, {description}, global, {APP_ID}:{id}",
            modifiers.join(" ")
        )
    })
}

fn hypr_modifier(m: &str) -> &'static str {
    match m {
        "CTRL" => "CTRL",
        "ALT" => "ALT",
        "SHIFT" => "SHIFT",
        _ => "SUPER",
    }
}

fn title(id: &str) -> &'static str {
    match id {
        "capture" => "Capture",
        _ => "Remember",
    }
}

/// Hyprland's modifier mask for the modifiers of an accelerator.
fn modmask(accelerator: &str) -> Option<(u64, String)> {
    let (modifiers, key) = split(accelerator)?;
    let mask = modifiers
        .iter()
        .map(|m| match hypr_modifier(m) {
            "SHIFT" => 1,
            "CTRL" => 4,
            "ALT" => 8,
            _ => 64,
        })
        .sum();
    Some((mask, key))
}

/// The user's own binds already on these keys, as "Ctrl+Alt+Space: Terminal".
pub fn conflicts(keys: &[(&'static str, String)]) -> Vec<String> {
    let Ok(output) = Command::new("hyprctl").args(["binds", "-j"]).output() else {
        return Vec::new();
    };
    let Ok(binds) = serde_json::from_slice::<Vec<Value>>(&output.stdout) else {
        return Vec::new();
    };
    conflicts_in(&binds, keys)
}

fn conflicts_in(binds: &[Value], keys: &[(&'static str, String)]) -> Vec<String> {
    let mut found = Vec::new();
    for (_, accelerator) in keys {
        let Some((mask, key)) = modmask(accelerator) else {
            continue;
        };
        for bind in binds {
            let same_key = bind["key"]
                .as_str()
                .is_some_and(|k| k.eq_ignore_ascii_case(&key));
            let ours = bind["description"]
                .as_str()
                .is_some_and(|d| d.starts_with(DESCRIPTION));
            if bind["modmask"].as_u64() == Some(mask) && same_key && !ours {
                let what = bind["description"]
                    .as_str()
                    .filter(|d| !d.is_empty())
                    .or_else(|| bind["arg"].as_str().filter(|a| !a.is_empty()))
                    .unwrap_or("another bind");
                found.push(format!("{} is already {what}", shown_key(accelerator)));
            }
        }
    }
    found
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

/// What Settings shows about Hyprland: the file, whether FastMind's block is in it, the lines
/// it would write, and keys already taken.
pub fn status(keys: &[(&'static str, String)]) -> Value {
    let Some(config) = Config::find() else {
        return Value::Null;
    };
    json!({
        "file": config.shown(),
        "installed": config.installed(),
        "lines": config.lines(keys),
        "conflicts": conflicts(keys),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lua() -> Config {
        Config {
            path: PathBuf::from("/nonexistent/hyprland.lua"),
            lua: true,
        }
    }

    fn keys() -> Vec<(&'static str, String)> {
        vec![
            ("capture", "CommandOrControl+Alt+Space".into()),
            ("remember", "Super+Shift+R".into()),
        ]
    }

    #[test]
    fn nothing_but_known_keys_reaches_the_config() {
        for bad in [
            "Ctrl+Alt+X\")) os.execute(\"touch /tmp/pwned\") --",
            "Ctrl+Alt+SPACE, exec, rm",
            "Ctrl+Alt+Space\nexec-once = x",
        ] {
            assert_eq!(line(true, "capture", bad), None, "{bad:?}");
            assert_eq!(line(false, "capture", bad), None, "{bad:?}");
        }
    }

    #[test]
    fn keys_use_xkb_names() {
        assert_eq!(
            line(false, "capture", "Ctrl+Comma").as_deref(),
            Some("bindd = CTRL, comma, FastMind: Capture, global, dev.coccor.fastmind:capture")
        );
    }

    #[test]
    fn lines_fit_either_config() {
        assert_eq!(
            line(false, "capture", "CommandOrControl+Alt+Space").as_deref(),
            Some("bindd = CTRL ALT, SPACE, FastMind: Capture, global, dev.coccor.fastmind:capture")
        );
        assert_eq!(
            line(true, "remember", "Super+Shift+R").as_deref(),
            Some(
                r#"hl.bind("SUPER + SHIFT + R", hl.dsp.global("dev.coccor.fastmind:remember"), { description = "FastMind: Remember" })"#
            )
        );
    }

    #[test]
    fn the_block_is_added_replaced_and_removed_leaving_the_rest() {
        let config = lua();
        let original = "require(\"default.hypr.omarchy\")\n-- mine\n";
        let lines = config.lines(&keys());
        let added = config.with_block(original, Some(&lines));
        assert!(added.starts_with(original));
        assert!(added.contains("dev.coccor.fastmind:capture"));
        assert_eq!(added.matches(">>> FastMind hotkeys").count(), 1);

        let other = config.lines(&[("capture", "Ctrl+Alt+K".into())]);
        let replaced = config.with_block(&added, Some(&other));
        assert_eq!(replaced.matches(">>> FastMind hotkeys").count(), 1);
        assert!(replaced.contains("CTRL + ALT + K"));
        assert!(!replaced.contains("SPACE"));

        assert_eq!(config.with_block(&replaced, None), original);
    }

    #[test]
    fn text_after_the_block_survives() {
        let config = lua();
        let lines = config.lines(&keys());
        let with = config.with_block("a\n", Some(&lines)) + "b\n";
        assert_eq!(config.with_block(&with, None), "a\nb\n");
    }

    #[test]
    fn writing_keeps_a_copy_and_follows_the_keys() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config {
            path: dir.path().join("hyprland.lua"),
            lua: true,
        };
        let original = "require(\"default.hypr.omarchy\")\n";
        std::fs::write(&config.path, original).unwrap();

        assert!(config.write_file(Some(&keys())).unwrap());
        assert!(config.installed());
        let backup = dir.path().join("hyprland.lua.before-fastmind");
        assert_eq!(std::fs::read_to_string(&backup).unwrap(), original);
        // Nothing to change: the file isn't written again (and Hyprland not reloaded).
        assert!(!config.write_file(Some(&keys())).unwrap());

        let other = vec![("capture", "Ctrl+Alt+K".to_string())];
        assert!(config.write_file(Some(&other)).unwrap());
        let text = std::fs::read_to_string(&config.path).unwrap();
        assert!(text.contains("CTRL + ALT + K") && !text.contains("SPACE"));
        assert_eq!(std::fs::read_to_string(&backup).unwrap(), original);

        assert!(config.write_file(None).unwrap());
        assert_eq!(std::fs::read_to_string(&config.path).unwrap(), original);
        assert!(!config.installed());
        assert!(!dir.path().join("hyprland.lua.fastmind-tmp").exists());
    }

    #[test]
    fn conflicts_skip_fastminds_own_binds() {
        let binds = vec![
            json!({ "modmask": 12, "key": "space", "description": "Terminal", "arg": "" }),
            json!({ "modmask": 12, "key": "SPACE", "description": "FastMind: Capture", "arg": "" }),
            json!({ "modmask": 12, "key": "R", "description": "", "arg": "" }),
            json!({ "modmask": 65, "key": "R", "description": "", "arg": "hyprctl reload" }),
        ];
        assert_eq!(
            conflicts_in(&binds, &keys()),
            [
                "Ctrl+Alt+Space is already Terminal",
                "Super+Shift+R is already hyprctl reload"
            ]
        );
    }
}
