//! FastMind inside Claude Code: the MCP server (search, read, ask, save, and the `note` prompt,
//! which Claude Code shows as `/mcp__fastmind__note`), a `/fastmind` skill that does the same
//! under a shorter name, a SessionEnd hook that writes up each session as soon as it ends, and a
//! SessionStart hook that tells a new session what the earlier ones in its project did.

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

/// FastMind's hooks: the Claude Code event, the `fastmind hook` argument, which starts it
/// fires on (none: all), and how long Claude Code waits for it.
const HOOKS: [(&str, &str, Option<&str>, u32); 2] = [
    ("SessionEnd", "session-end", None, 30),
    // Not on resume (the session already has it) or after a compaction (its summary keeps it).
    ("SessionStart", "session-start", Some("startup|clear"), 10),
];

/// A hook's command line.
pub fn hook_command(exe: &str, arg: &str) -> String {
    format!("\"{exe}\" hook {arg}")
}

fn is_ours(hook: &Value, arg: &str) -> bool {
    hook["command"].as_str().is_some_and(|c| {
        c.contains(&format!("hook {arg}")) && c.to_lowercase().contains("fastmind")
    })
}

/// `settings` with FastMind's hooks added (any earlier copies replaced), or removed when `add`
/// is false.
pub fn with_hooks(mut settings: Value, exe: &str, add: bool) -> Value {
    for (event, arg, matcher, timeout) in HOOKS {
        settings = with_hook(settings, event, arg, matcher, timeout, exe, add);
    }
    settings
}

fn with_hook(
    mut settings: Value,
    event: &str,
    arg: &str,
    matcher: Option<&str>,
    timeout: u32,
    exe: &str,
    add: bool,
) -> Value {
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
        .entry(event)
        .or_insert_with(|| json!([]));
    if !list.is_array() {
        *list = json!([]);
    }
    let entries = list.as_array_mut().expect("an array");
    // Drop our hook from every matcher group, and groups it leaves empty.
    for group in entries.iter_mut() {
        if let Some(inner) = group["hooks"].as_array_mut() {
            inner.retain(|h| !is_ours(h, arg));
        }
    }
    entries.retain(|group| group["hooks"].as_array().is_none_or(|h| !h.is_empty()));
    if add {
        let mut group = json!({
            "hooks": [{ "type": "command", "command": hook_command(exe, arg), "timeout": timeout }]
        });
        if let Some(matcher) = matcher {
            group["matcher"] = json!(matcher);
        }
        entries.push(group);
    }
    // An event left with no hooks at all goes, so removing ours gives the settings back.
    let hooks = settings["hooks"].as_object_mut().expect("an object");
    if hooks
        .get(event)
        .and_then(Value::as_array)
        .is_some_and(Vec::is_empty)
    {
        hooks.remove(event);
    }
    if hooks.is_empty() {
        settings.as_object_mut().expect("an object").remove("hooks");
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
    let updated = with_hooks(current.clone(), &this_exe()?, add);
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
            "Hooks added to {}: each session is written up when it ends, and the next one \
             in the project starts knowing what the last ones did",
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
    done.push(format!("Hooks removed from {}", path.display()));
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
    let settings = claude_settings_file()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .unwrap_or(Value::Null);
    let hooks = HOOKS.iter().map(|(event, arg, _, _)| {
        let found = settings["hooks"][event].as_array().and_then(|groups| {
            groups
                .iter()
                .flat_map(|g| g["hooks"].as_array().cloned().unwrap_or_default())
                .find(|h| is_ours(h, arg))
        });
        match found {
            Some(h) => format!(
                "{event} hook: on\n  {}",
                h["command"].as_str().unwrap_or("")
            ),
            None => format!("{event} hook: off (fastmind agent install)"),
        }
    });
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
    let mut lines = vec![mcp];
    lines.extend(hooks);
    lines.push(skill);
    lines
}

/// The SessionEnd hook: Claude Code sends the session on stdin. The write-up runs in a process
/// of its own, so quitting Claude Code doesn't wait for the model.
pub fn session_end() -> Result<(), String> {
    let mut input = String::new();
    std::io::stdin()
        .read_to_string(&mut input)
        .map_err(|e| e.to_string())?;
    let event: Value = serde_json::from_str(&input).unwrap_or(Value::Null);
    // `claude mcp …` and other commands that never start a session still fire SessionEnd,
    // with a transcript that was never written: nothing to do, and nothing worth logging.
    let Some(transcript) = event["transcript_path"]
        .as_str()
        .filter(|p| !p.is_empty() && Path::new(p).is_file())
    else {
        return Ok(());
    };
    // Run in the session's folder, so a project with a notebook of its own gets the note.
    let cwd = event["cwd"]
        .as_str()
        .map(PathBuf::from)
        .filter(|dir| dir.is_dir());
    let import = || -> Result<Command, String> {
        let mut command = Command::new(std::env::current_exe().map_err(|e| e.to_string())?);
        command
            .args(["import-session", transcript])
            .stdin(Stdio::null());
        if let Some(dir) = &cwd {
            command.current_dir(dir);
        }
        Ok(command)
    };
    let mut command = import()?;
    command.stdout(Stdio::null()).stderr(Stdio::null());
    detach(&mut command);
    if command.spawn().is_err() {
        // Not allowed to leave Claude Code's job: do it here instead.
        let _ = import()?.status();
    }
    Ok(())
}

/// The SessionStart hook: what it prints goes into the context a new session starts with. It
/// reads a few notes and never calls a model, so starting stays quick; a session that ended
/// without a write-up (crashed, killed) gets one in the background.
pub fn session_start() -> Result<(), String> {
    let mut input = String::new();
    std::io::stdin()
        .read_to_string(&mut input)
        .map_err(|e| e.to_string())?;
    let event: Value = serde_json::from_str(&input).unwrap_or(Value::Null);
    let cwd = event["cwd"]
        .as_str()
        .map(PathBuf::from)
        .filter(|dir| dir.is_dir())
        .or_else(|| std::env::current_dir().ok())
        .ok_or("no working folder")?;
    let settings = fastmind_core::config::Settings::load();
    // The project's notebook, else the usual one; never a folder the user didn't make.
    let Some(root) = settings
        .notebook_for(&cwd)
        .or(settings.notebook.as_ref())
        .filter(|root| root.is_dir())
    else {
        return Ok(());
    };
    let nb = fastmind_core::notebook::Notebook { root: root.clone() };
    let transcript = event["transcript_path"].as_str().map(PathBuf::from);
    if let Some(missed) = transcript.as_ref().and_then(|current| {
        let dir = current.parent()?;
        fastmind_core::sessions::missed(&nb, &settings.ai, dir, current, chrono::Local::now())
    }) {
        let mut command = Command::new(std::env::current_exe().map_err(|e| e.to_string())?);
        command
            .arg("import-session")
            .arg(&missed)
            .current_dir(&cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        detach(&mut command);
        // If it can't start apart from Claude Code, the next session start tries again.
        let _ = command.spawn();
    }
    if let Some(context) =
        fastmind_core::sessions::context(&nb, &settings.ai, &cwd.to_string_lossy())
    {
        let output = json!({
            "hookSpecificOutput": {
                "hookEventName": "SessionStart",
                "additionalContext": context,
            }
        });
        println!("{output}");
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
        let added = with_hooks(mine.clone(), r"C:\Tools\fastmind.exe", true);
        let again = with_hooks(added.clone(), r"D:\New\fastmind.exe", true);
        let groups = again["hooks"]["SessionEnd"].as_array().unwrap();
        assert_eq!(groups.len(), 2, "the user's hook stays, ours once: {again}");
        assert_eq!(
            groups[1]["hooks"][0]["command"],
            r#""D:\New\fastmind.exe" hook session-end"#
        );
        let start = again["hooks"]["SessionStart"].as_array().unwrap();
        assert_eq!(start.len(), 1, "{again}");
        assert_eq!(start[0]["matcher"], "startup|clear");
        assert_eq!(
            start[0]["hooks"][0]["command"],
            r#""D:\New\fastmind.exe" hook session-start"#
        );
        assert_eq!(again["model"], "opus");
        assert_eq!(again["hooks"]["Stop"], mine["hooks"]["Stop"]);
        let removed = with_hooks(again, "x", false);
        assert_eq!(
            removed, mine,
            "removing them gives the user's settings back exactly"
        );
        let empty = with_hooks(json!({}), "C:/fm/fastmind.exe", true);
        assert!(is_ours(
            &empty["hooks"]["SessionEnd"][0]["hooks"][0],
            "session-end"
        ));
        assert!(!is_ours(
            &empty["hooks"]["SessionEnd"][0]["hooks"][0],
            "session-start"
        ));
        assert_eq!(with_hooks(empty, "x", false), json!({}));
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
