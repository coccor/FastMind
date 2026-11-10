//! The Claude Code provider: the user's own installed `claude`, signed in to their plan, run
//! once per request with no tools, no MCP servers and none of their settings. The request goes
//! in as one stream-json message (images first, then the text) and the answer comes back on
//! the stream's `result` line.

use super::Image;
use crate::{Error, Result};
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const TIMEOUT: Duration = Duration::from_secs(240);

/// Where `claude` is: on the PATH, or where its installer puts it.
pub fn find() -> Option<PathBuf> {
    let names: &[&str] = if cfg!(windows) {
        &["claude.exe", "claude.cmd"]
    } else {
        &["claude"]
    };
    let on_path = std::env::var_os("PATH")
        .map(|value| std::env::split_paths(&value).collect::<Vec<_>>())
        .unwrap_or_default();
    for name in names {
        if let Some(found) = on_path
            .iter()
            .map(|dir| dir.join(name))
            .find(|path| path.is_file())
        {
            return Some(found);
        }
    }
    let home = directories::BaseDirs::new()?.home_dir().to_path_buf();
    names
        .iter()
        .map(|name| home.join(".local").join("bin").join(name))
        .find(|path| path.is_file())
}

/// The command line: print mode, stream-json both ways, nothing the user configured.
pub fn args(model: &str, system: &str, schema: Option<&Value>) -> Vec<String> {
    let mut args: Vec<String> = [
        "-p",
        "--output-format",
        "stream-json",
        "--verbose",
        "--input-format",
        "stream-json",
        "--tools",
        "",
        "--model",
        model,
        "--strict-mcp-config",
        "--setting-sources",
        "",
        "--no-session-persistence",
        "--system-prompt",
        system,
    ]
    .iter()
    .map(|arg| (*arg).to_owned())
    .collect();
    if let Some(schema) = schema {
        args.push("--json-schema".into());
        args.push(schema.to_string());
    }
    args
}

/// The one stream-json line sent on stdin.
pub fn message(user: &str, images: &[Image]) -> String {
    let mut content: Vec<Value> = images
        .iter()
        .map(|image| {
            json!({"type": "image", "source": {"type": "base64", "media_type": image.media_type, "data": image.base64()}})
        })
        .collect();
    content.push(json!({"type": "text", "text": user}));
    json!({"type": "user", "message": {"role": "user", "content": content}}).to_string()
}

/// The answer on the stream's last `result` line: the structured output when there is one,
/// else the text.
pub fn answer(stdout: &str, stderr: &str, code: Option<i32>) -> Result<String> {
    let result = stdout
        .lines()
        .rev()
        .filter_map(|line| serde_json::from_str::<Value>(line.trim()).ok())
        .find(|event| event["type"] == "result");
    let Some(result) = result else {
        let said = stderr
            .lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("")
            .trim();
        return Err(Error::msg(format!(
            "Claude Code stopped without an answer (exit code {}){}",
            code.map_or("none".into(), |c| c.to_string()),
            if said.is_empty() {
                String::new()
            } else {
                format!(": {said}")
            }
        )));
    };
    let text = result["result"].as_str().unwrap_or("");
    if result["is_error"].as_bool() == Some(true) {
        let lower = text.to_lowercase();
        if ["/login", "not logged in", "invalid api key", "oauth token"]
            .iter()
            .any(|sign| lower.contains(sign))
        {
            return Err(Error::msg(
                "Claude Code isn't signed in: run `claude` in a terminal, then /login",
            ));
        }
        if lower.contains("limit") {
            return Err(Error::msg(format!(
                "Claude Code: your plan's usage limit is reached ({})",
                crate::text::excerpt(text, 120)
            )));
        }
        return Err(Error::msg(format!(
            "Claude Code: {}",
            crate::text::excerpt(text, 200)
        )));
    }
    if let Some(structured) = result.get("structured_output").filter(|v| !v.is_null()) {
        return Ok(structured.to_string());
    }
    if text.trim().is_empty() {
        return Err(Error::msg("Claude Code's reply had no text"));
    }
    Ok(text.trim().to_owned())
}

pub fn run(
    model: &str,
    system: &str,
    user: &str,
    images: &[Image],
    schema: Option<&Value>,
) -> Result<String> {
    let program = find().ok_or_else(|| {
        Error::msg(
            "Claude Code isn't installed: get it from claude.com/code, or choose another model",
        )
    })?;
    // An empty folder of its own, so no project's CLAUDE.md or settings are read.
    let dir = std::env::temp_dir().join("fastmind-claude-code");
    std::fs::create_dir_all(&dir)?;
    let mut command = Command::new(&program);
    command
        .args(args(model, system, schema))
        .current_dir(&dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // The user's plan pays, not an API key that happens to be set; and a Claude Code session
    // that started FastMind mustn't leak into this one.
    for (name, _) in std::env::vars_os() {
        let upper = name.to_string_lossy().to_ascii_uppercase();
        if upper.starts_with("ANTHROPIC_")
            || upper.starts_with("CLAUDE_CODE_")
            || upper == "CLAUDECODE"
        {
            command.env_remove(&name);
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = command
        .spawn()
        .map_err(|error| Error::msg(format!("Claude Code couldn't be started: {error}")))?;
    let line = message(user, images);
    let mut stdin = child.stdin.take().expect("piped stdin");
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(line.as_bytes());
        let _ = stdin.write_all(b"\n");
    });
    let mut stdout = child.stdout.take().expect("piped stdout");
    let mut stderr = child.stderr.take().expect("piped stderr");
    let out = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = stdout.read_to_string(&mut text);
        text
    });
    let err = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = stderr.read_to_string(&mut text);
        text
    });
    let started = Instant::now();
    let code = loop {
        if let Some(status) = child.try_wait()? {
            break status.code();
        }
        if started.elapsed() > TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Error::msg("Claude Code didn't answer within 4 minutes"));
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let _ = writer.join();
    let stdout = out.join().unwrap_or_default();
    let stderr = err.join().unwrap_or_default();
    answer(&stdout, &stderr, code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_command_line_has_no_tools_and_no_user_settings() {
        let args = args("sonnet", "Rules.", Some(&json!({"type": "object"})));
        let pairs: Vec<_> = args
            .windows(2)
            .map(|w| (w[0].as_str(), w[1].as_str()))
            .collect();
        assert!(pairs.contains(&("--tools", "")));
        assert!(pairs.contains(&("--setting-sources", "")));
        assert!(pairs.contains(&("--model", "sonnet")));
        assert!(pairs.contains(&("--system-prompt", "Rules.")));
        assert!(pairs.contains(&("--json-schema", r#"{"type":"object"}"#)));
        assert!(args.contains(&"--no-session-persistence".to_owned()));
    }

    #[test]
    fn images_go_before_the_text() {
        let line = message(
            "hi",
            &[Image {
                media_type: "image/png".into(),
                data: vec![1, 2],
            }],
        );
        let value: Value = serde_json::from_str(&line).unwrap();
        let content = &value["message"]["content"];
        assert_eq!(content[0]["type"], "image");
        assert_eq!(content[1]["text"], "hi");
    }

    #[test]
    fn the_answer_is_the_structured_output_or_the_text() {
        let out = "{\"type\":\"system\"}\nnot json\n{\"type\":\"result\",\"result\":\"x\",\"structured_output\":{\"title\":\"T\"}}\n";
        assert_eq!(answer(out, "", Some(0)).unwrap(), r#"{"title":"T"}"#);
        let out = "{\"type\":\"result\",\"result\":\"plain\"}";
        assert_eq!(answer(out, "", Some(0)).unwrap(), "plain");
        let out = "{\"type\":\"result\",\"is_error\":true,\"result\":\"Not logged in · Please run /login\"}";
        assert!(
            answer(out, "", Some(1))
                .unwrap_err()
                .to_string()
                .contains("signed in")
        );
        assert!(
            answer("", "error: unknown option\n", Some(1))
                .unwrap_err()
                .to_string()
                .contains("unknown option")
        );
    }
}
