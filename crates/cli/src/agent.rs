//! FastMind inside Claude Code: the MCP server (search, read, ask, save, and the `note` prompt,
//! which Claude Code shows as `/mcp__fastmind__note`), a `/fastmind` skill that does the same
//! under a shorter name, and a SessionEnd hook that writes up each session as soon as it ends.

use serde_json::{Value, json};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Claude Code's user folder: `~/.claude`, or `CLAUDE_CONFIG_DIR`.
fn claude_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("CLAUDE_CONFIG_DIR") {
        return Some(PathBuf::from(dir));
    }
    directories::BaseDirs::new().map(|d| d.home_dir().join(".claude"))
}

/// Claude Code's user settings, where hooks live.
pub fn claude_settings_file() -> Option<PathBuf> {
    claude_dir().map(|d| d.join("settings.json"))
}

/// The `/fastmind` skill. Claude Code names MCP prompts `/mcp__<server>__<prompt>`, so the
/// short name needs a skill of its own.
pub fn skill_file() -> Option<PathBuf> {
    claude_dir().map(|d| d.join("skills").join("fastmind").join("SKILL.md"))
}

/// Marks the skill as FastMind's, so a `fastmind` skill the user wrote is never touched.
const SKILL_MARK: &str = "Added by `fastmind agent install`";

/// The skill only points at the `note_instructions` tool, so the steps, the ai-session skill
/// and the user's writing preferences stay in one place and are read fresh each time.
pub const SKILL: &str = "---
name: fastmind
description: Write up this session as a note in the user's FastMind notebook and save it. Use when the user asks to note, save or write up the session or the work in FastMind.
argument-hint: [what the note should focus on]
---

<!-- Added by `fastmind agent install`; `fastmind agent uninstall` removes it. -->

Call the `note_instructions` tool of the `fastmind` MCP server and do what it returns. If the
text in the quotes is not empty, pass it as `focus`: \"$ARGUMENTS\".

If the `fastmind` MCP server isn't connected, say so and suggest running `fastmind agent install`.
";

fn is_our_skill(path: &Path) -> bool {
    std::fs::read_to_string(path).is_ok_and(|t| t.contains(SKILL_MARK))
}

/// Writes the skill, unless the user has a `fastmind` skill of their own there.
fn write_skill() -> Result<Option<PathBuf>, String> {
    let path = skill_file().ok_or("couldn't find your home folder")?;
    if path.exists() && !is_our_skill(&path) {
        return Ok(None);
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    fastmind_core::fsx::write_atomic(&path, SKILL.as_bytes()).map_err(|e| e.0)?;
    Ok(Some(path))
}

/// Removes the skill if it's ours; true when it removed something.
fn remove_skill() -> Result<bool, String> {
    let Some(path) = skill_file().filter(|p| is_our_skill(p)) else {
        return Ok(false);
    };
    std::fs::remove_file(&path).map_err(|e| e.to_string())?;
    // The folder too, if nothing else is in it.
    if let Some(dir) = path.parent() {
        let _ = std::fs::remove_dir(dir);
    }
    Ok(true)
}

fn this_exe() -> Result<String, String> {
    std::env::current_exe()
        .map(|p| p.display().to_string())
        .map_err(|e| e.to_string())
}

/// The hook's command line.
pub fn hook_command(exe: &str) -> String {
    format!("\"{exe}\" hook session-end")
}

fn is_ours(hook: &Value) -> bool {
    hook["command"]
        .as_str()
        .is_some_and(|c| c.contains("hook session-end") && c.to_lowercase().contains("fastmind"))
}

/// `settings` with FastMind's SessionEnd hook added (any earlier copy of it replaced), or
/// removed when `add` is false.
pub fn with_hook(mut settings: Value, exe: &str, add: bool) -> Value {
    if !settings.is_object() {
        settings = json!({});
    }
    let hooks = settings
        .as_object_mut()
        .expect("an object")
        .entry("hooks")
        .or_insert_with(|| json!({}));
    if !hooks.is_object() {
        *hooks = json!({});
    }
    let list = hooks
        .as_object_mut()
        .expect("an object")
        .entry("SessionEnd")
        .or_insert_with(|| json!([]));
    if !list.is_array() {
        *list = json!([]);
    }
    let entries = list.as_array_mut().expect("an array");
    // Drop our hook from every matcher group, and groups it leaves empty.
    for group in entries.iter_mut() {
        if let Some(inner) = group["hooks"].as_array_mut() {
            inner.retain(|h| !is_ours(h));
        }
    }
    entries.retain(|group| group["hooks"].as_array().is_none_or(|h| !h.is_empty()));
    if add {
        entries.push(json!({
            "hooks": [{ "type": "command", "command": hook_command(exe), "timeout": 30 }]
        }));
    }
    settings
}

fn edit_settings(add: bool) -> Result<PathBuf, String> {
    let path = claude_settings_file().ok_or("couldn't find your home folder")?;
    let current: Value = match std::fs::read_to_string(&path) {
        Ok(text) if !text.trim().is_empty() => serde_json::from_str(&text).map_err(|e| {
            format!(
                "{} isn't valid JSON ({e}), so it was left alone",
                path.display()
            )
        })?,
        _ => json!({}),
    };
    let updated = with_hook(current.clone(), &this_exe()?, add);
    if updated == current {
        return Ok(path);
    }
    if path.exists() {
        let backup = path.with_extension("json.fastmind-backup");
        std::fs::copy(&path, &backup).map_err(|e| e.to_string())?;
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let text = serde_json::to_string_pretty(&updated).map_err(|e| e.to_string())? + "\n";
    fastmind_core::fsx::write_atomic(&path, text.as_bytes()).map_err(|e| e.0)?;
    Ok(path)
}

fn claude() -> Result<PathBuf, String> {
    fastmind_core::model::find_claude_code()
        .ok_or_else(|| "Claude Code isn't installed (claude.com/code)".to_owned())
}

fn run_claude(args: &[&str]) -> Result<String, String> {
    let output = Command::new(claude()?)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&output.stdout).into_owned()
        + &String::from_utf8_lossy(&output.stderr);
    if output.status.success() {
        Ok(text)
    } else {
        Err(text.trim().to_owned())
    }
}

/// Connects FastMind to Claude Code: the MCP server for every project, and the hook.
pub fn install(hook: bool) -> Result<Vec<String>, String> {
    let exe = this_exe()?;
    let mut done = Vec::new();
    // Re-added each time, so it points at this copy of fastmind.
    let _ = run_claude(&["mcp", "remove", "--scope", "user", "fastmind"]);
    run_claude(&[
        "mcp", "add", "--scope", "user", "fastmind", "--", &exe, "mcp",
    ])
    .map_err(|e| format!("claude mcp add failed: {e}"))?;
    done.push(format!(
        "MCP server \"fastmind\" added for all your projects ({exe} mcp)"
    ));
    match write_skill()? {
        Some(path) => done.push(format!(
            "Skill added at {}: in Claude Code, /fastmind writes up the session and saves it",
            path.display()
        )),
        None => done.push(
            "You already have a skill called fastmind, so it was left alone; \
             /mcp__fastmind__note writes up the session and saves it"
                .into(),
        ),
    }
    if hook {
        let path = edit_settings(true)?;
        done.push(format!(
            "SessionEnd hook added to {}: each session is written up when it ends",
            path.display()
        ));
    }
    Ok(done)
}

pub fn uninstall() -> Result<Vec<String>, String> {
    let mut done = Vec::new();
    if run_claude(&["mcp", "remove", "--scope", "user", "fastmind"]).is_ok() {
        done.push("MCP server \"fastmind\" removed".into());
    }
    if remove_skill()? {
        done.push("/fastmind skill removed".into());
    }
    let path = edit_settings(false)?;
    done.push(format!("SessionEnd hook removed from {}", path.display()));
    Ok(done)
}

/// What's set up now.
pub fn status() -> Vec<String> {
    let mcp = match run_claude(&["mcp", "get", "fastmind"]) {
        Ok(text) => format!(
            "MCP server: connected\n  {}",
            text.lines()
                .find(|l| l.to_lowercase().contains("command"))
                .unwrap_or("")
                .trim()
        ),
        Err(_) => "MCP server: not added (fastmind agent install)".into(),
    };
    let hook = claude_settings_file()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|s| {
            s["hooks"]["SessionEnd"].as_array().map(|groups| {
                groups
                    .iter()
                    .flat_map(|g| g["hooks"].as_array().cloned().unwrap_or_default())
                    .find(is_ours)
            })
        })
        .flatten();
    let hook = match hook {
        Some(h) => format!(
            "SessionEnd hook: on\n  {}",
            h["command"].as_str().unwrap_or("")
        ),
        None => "SessionEnd hook: off (fastmind agent install)".into(),
    };
    let skill = match skill_file() {
        Some(path) if is_our_skill(&path) => format!("/fastmind skill: on\n  {}", path.display()),
        Some(path) if path.exists() => {
            format!(
                "/fastmind skill: yours, not FastMind's\n  {}",
                path.display()
            )
        }
        _ => "/fastmind skill: off (fastmind agent install)".into(),
    };
    vec![mcp, hook, skill]
}

/// The SessionEnd hook: Claude Code sends the session on stdin. The write-up runs in a process
/// of its own, so quitting Claude Code doesn't wait for the model.
pub fn session_end() -> Result<(), String> {
    let mut input = String::new();
    std::io::stdin()
        .read_to_string(&mut input)
        .map_err(|e| e.to_string())?;
    let event: Value = serde_json::from_str(&input).unwrap_or(Value::Null);
    let Some(transcript) = event["transcript_path"].as_str().filter(|p| !p.is_empty()) else {
        return Ok(());
    };
    let mut command = Command::new(std::env::current_exe().map_err(|e| e.to_string())?);
    command
        .args(["import-session", transcript])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    detach(&mut command);
    if command.spawn().is_err() {
        // Not allowed to leave Claude Code's job: do it here instead.
        let mut inline = Command::new(std::env::current_exe().map_err(|e| e.to_string())?);
        inline
            .args(["import-session", transcript])
            .stdin(Stdio::null());
        let _ = inline.status();
    }
    Ok(())
}

#[cfg(windows)]
fn detach(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(
        DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_BREAKAWAY_FROM_JOB | CREATE_NO_WINDOW,
    );
}

#[cfg(unix)]
fn detach(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    command.process_group(0);
}

#[cfg(not(any(windows, unix)))]
fn detach(_: &mut Command) {}

/// Where the hook's background write-ups say what happened.
pub fn hook_log() -> PathBuf {
    fastmind_core::config::config_file()
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(std::env::temp_dir)
        .join("session-hook.log")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hook_is_added_once_and_removed_cleanly() {
        let mine = json!({
            "model": "opus",
            "hooks": {
                "SessionEnd": [{ "hooks": [{ "type": "command", "command": "notify-send bye" }] }],
                "Stop": [{ "hooks": [{ "type": "command", "command": "echo stop" }] }]
            }
        });
        let added = with_hook(mine.clone(), r"C:\Tools\fastmind.exe", true);
        let again = with_hook(added.clone(), r"D:\New\fastmind.exe", true);
        let groups = again["hooks"]["SessionEnd"].as_array().unwrap();
        assert_eq!(groups.len(), 2, "the user's hook stays, ours once: {again}");
        assert_eq!(
            groups[1]["hooks"][0]["command"],
            r#""D:\New\fastmind.exe" hook session-end"#
        );
        assert_eq!(again["model"], "opus");
        assert_eq!(again["hooks"]["Stop"], mine["hooks"]["Stop"]);
        let removed = with_hook(again, "x", false);
        assert_eq!(
            removed, mine,
            "removing it gives the user's settings back exactly"
        );
        let empty = with_hook(json!({}), "C:/fm/fastmind.exe", true);
        assert!(is_ours(&empty["hooks"]["SessionEnd"][0]["hooks"][0]));
    }

    #[test]
    fn the_skill_is_called_fastmind_and_points_at_the_tool() {
        let front = SKILL.split("---").nth(1).unwrap();
        assert!(front.contains("\nname: fastmind\n"));
        assert!(front.contains("\ndescription: "));
        assert!(SKILL.contains("note_instructions") && SKILL.contains("$ARGUMENTS"));
        assert!(SKILL.contains(SKILL_MARK));
    }
}
