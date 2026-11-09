//! Learning from AI sessions, end to end: transcripts from Claude Code and Codex become notes,
//! secrets never reach the notebook, sessions still running wait, edited notes are left alone,
//! and Remember finds what a session found.

use chrono::{Duration, Local};
use fastmind_core::api::App;
use fastmind_core::config::Settings;
use serde_json::json;
use std::fs;
use std::path::Path;

fn line(value: serde_json::Value) -> String {
    format!("{value}\n")
}

fn claude_transcript(when: chrono::DateTime<Local>) -> String {
    let at = |minutes: i64| (when + Duration::minutes(minutes)).to_rfc3339();
    let base = json!({"cwd": "C:\\Projects\\Payments", "gitBranch": "fix-ledger-lag", "sessionId": "abc-123"});
    let with = |mut extra: serde_json::Value| {
        for (k, v) in base.as_object().unwrap() {
            extra[k] = v.clone();
        }
        line(extra)
    };
    [
        with(json!({"type": "user", "timestamp": at(0), "message": {"role": "user", "content": "<system-reminder>ignore me</system-reminder>Why does the ledger consumer lag every night at 2am?"}})),
        with(json!({"type": "assistant", "timestamp": at(1), "message": {"role": "assistant", "content": [
            {"type": "thinking", "thinking": "private reasoning"},
            {"type": "text", "text": "Let me look at the batch job."},
            {"type": "tool_use", "name": "Bash", "input": {"command": "psql -c 'explain analyze select * from entries'  # password=hunter22"}}
        ]}})),
        with(json!({"type": "user", "timestamp": at(2), "message": {"role": "user", "content": [{"type": "tool_result", "content": "Seq Scan on entries"}]}})),
        with(json!({"type": "assistant", "isSidechain": true, "timestamp": at(3), "message": {"role": "assistant", "content": [{"type": "text", "text": "SUBAGENT CHATTER"}]}})),
        with(json!({"type": "assistant", "timestamp": at(4), "message": {"role": "assistant", "content": [
            {"type": "tool_use", "name": "Edit", "input": {"file_path": "C:\\Projects\\Payments\\migrations\\0042_entries_index.sql", "old_string": "", "new_string": ""}}
        ]}})),
        with(json!({"type": "user", "timestamp": at(5), "message": {"role": "user", "content": "Use this key to test: sk-ant-api03-ABCDEFGHIJKLMNOPQRSTUVWX"}})),
        with(json!({"type": "assistant", "timestamp": at(6), "message": {"role": "assistant", "content": [
            {"type": "text", "text": "The lag comes from a sequential scan on entries during the nightly batch: the query filters on posted_at, which had no index. I added migration 0042 with an index on entries(posted_at); the batch now runs in 40 seconds instead of 14 minutes."}
        ]}})),
        line(json!({"type": "ai-title", "aiTitle": "Ledger lag from missing index", "sessionId": "abc-123"})),
    ]
    .concat()
}

fn codex_transcript(when: chrono::DateTime<Local>) -> String {
    let at = when.to_rfc3339();
    [
        line(json!({"timestamp": at, "type": "session_meta", "payload": {"id": "cdx-9", "cwd": "/home/dev/api", "git": {"branch": "main"}}})),
        line(json!({"timestamp": at, "type": "response_item", "payload": {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "<environment_context>cwd</environment_context>"}]}})),
        line(json!({"timestamp": at, "type": "response_item", "payload": {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "Make the rate limiter per API key instead of per IP"}]}})),
        line(json!({"timestamp": at, "type": "response_item", "payload": {"type": "function_call", "name": "shell", "arguments": "{\"command\":[\"bash\",\"-lc\",\"cargo test rate_limit\"]}"}})),
        line(json!({"timestamp": at, "type": "response_item", "payload": {"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "The limiter now keys buckets by API key, falling back to IP for anonymous calls; all rate_limit tests pass."}]}})),
    ]
    .concat()
}

fn write(path: &Path, text: &str, age_minutes: i64) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
    let file = fs::OpenOptions::new().write(true).open(path).unwrap();
    file.set_modified((Local::now() - Duration::minutes(age_minutes)).into())
        .unwrap();
}

#[test]
fn ai_sessions_become_safe_searchable_notes() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    // SAFETY: this test binary has one test, so nothing else reads the environment meanwhile.
    unsafe { std::env::set_var("FASTMIND_AI_HOME", &home) };
    let started = Local::now() - Duration::hours(3);
    let claude = home.join(".claude/projects/C--Projects-Payments/abc-123.jsonl");
    write(&claude, &claude_transcript(started), 120);
    write(
        &home.join(".codex/sessions/2026/11/09/rollout-cdx-9.jsonl"),
        &codex_transcript(started),
        90,
    );
    // Still running: written to a minute ago.
    write(
        &home.join(".claude/projects/C--Projects-Payments/live.jsonl"),
        &claude_transcript(Local::now()).replace("abc-123", "live-1"),
        1,
    );

    let mut settings = Settings {
        notebook: Some(dir.path().join("nb")),
        ..Settings::default()
    };
    settings.ai.enabled = true;
    let app = App::ephemeral(settings);

    let status = app.call("ai_status", json!({})).unwrap();
    assert_eq!(status["claude_code"], 2);
    assert_eq!(status["codex"], 1);

    let report = app.call("ai_import", json!({})).unwrap();
    let added: Vec<String> = serde_json::from_value(report["added"].clone()).unwrap();
    assert_eq!(added.len(), 2, "{report:#}");
    assert_eq!(report["skipped"], 1, "the running session waits");

    let claude_note = added
        .iter()
        .find(|p| p.starts_with("AI sessions/Payments/"))
        .expect("Payments note");
    assert!(
        claude_note.ends_with("Ledger lag from missing index.md"),
        "{claude_note}"
    );
    let text = fs::read_to_string(dir.path().join("nb").join(claude_note)).unwrap();
    assert!(text.contains("type: ai-session"));
    assert!(text.contains("Why does the ledger consumer lag every night at 2am?"));
    assert!(
        text.contains("index on entries(posted_at)"),
        "the outcome is kept"
    );
    assert!(
        text.contains("`migrations/0042_entries_index.sql`"),
        "files are relative to the project"
    );
    assert!(
        !text.contains("ignore me")
            && !text.contains("SUBAGENT")
            && !text.contains("private reasoning")
    );
    assert!(
        !text.contains("sk-ant-api03") && !text.contains("hunter22"),
        "secrets leaked:\n{text}"
    );

    let codex_note = added
        .iter()
        .find(|p| p.starts_with("AI sessions/api/"))
        .expect("Codex note");
    let codex = fs::read_to_string(dir.path().join("nb").join(codex_note)).unwrap();
    assert!(codex.contains("source: codex") && codex.contains("cargo test rate_limit"));
    assert!(!codex.contains("environment_context"));

    // Remember finds what the session found.
    let answer = app
        .call(
            "ask",
            json!({"question": "why did the ledger lag at night?"}),
        )
        .unwrap();
    assert_eq!(answer["sources"][0]["path"], json!(claude_note));

    // Nothing new: nothing written. An edited note is never overwritten when the session grows.
    let again = app.call("ai_import", json!({})).unwrap();
    assert_eq!(again["added"].as_array().unwrap().len(), 0);
    let mine = format!("{text}\nMy own conclusion.\n");
    fs::write(dir.path().join("nb").join(claude_note), &mine).unwrap();
    let mut grown = claude_transcript(started);
    grown.push_str(&line(json!({"type": "user", "cwd": "C:\\Projects\\Payments", "sessionId": "abc-123", "timestamp": Local::now().to_rfc3339(), "message": {"role": "user", "content": "Thanks!"}})));
    write(&claude, &grown, 60);
    app.call("ai_import", json!({})).unwrap();
    assert_eq!(
        fs::read_to_string(dir.path().join("nb").join(claude_note)).unwrap(),
        mine
    );
}
