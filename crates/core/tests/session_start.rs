//! What a new coding session starts with: the project's latest session notes, read back, and
//! the session before it written up if it ended without a SessionEnd (crashed, killed).

use chrono::{Duration, Local};
use fastmind_core::notebook::Notebook;
use fastmind_core::sessions::{self, AiSettings};
use serde_json::json;
use std::fs;
use std::path::Path;

fn line(value: serde_json::Value) -> String {
    format!("{value}\n")
}

/// A short Claude Code session in the Payments project.
fn transcript(id: &str, title: &str, asked: &str, answer: &str, minutes_ago: i64) -> String {
    let at = |m: i64| (Local::now() - Duration::minutes(minutes_ago + 10 - m)).to_rfc3339();
    let base = json!({"cwd": "/home/dev/Payments", "gitBranch": "main", "sessionId": id});
    let with = |mut extra: serde_json::Value| {
        for (k, v) in base.as_object().unwrap() {
            extra[k] = v.clone();
        }
        line(extra)
    };
    [
        with(json!({"type": "user", "timestamp": at(0), "message": {"role": "user", "content": asked}})),
        with(json!({"type": "assistant", "timestamp": at(5), "message": {"role": "assistant", "content": [{"type": "text", "text": answer}]}})),
        line(json!({"type": "ai-title", "aiTitle": title, "sessionId": id})),
    ]
    .concat()
}

fn write(path: &Path, text: &str, minutes_ago: i64) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
    let file = fs::OpenOptions::new().write(true).open(path).unwrap();
    file.set_modified((Local::now() - Duration::minutes(minutes_ago)).into())
        .unwrap();
}

#[test]
fn a_new_session_hears_about_the_last_ones_and_a_missed_one_is_found() {
    let dir = tempfile::tempdir().unwrap();
    let nb = Notebook::open(dir.path().join("nb")).unwrap();
    let settings = AiSettings::default();
    let sessions = dir.path().join("projects/-home-dev-Payments");
    let older = sessions.join("older.jsonl");
    let crashed = sessions.join("crashed.jsonl");
    let running = sessions.join("running.jsonl");
    let current = sessions.join("current.jsonl");
    write(
        &older,
        &transcript(
            "older",
            "Ledger lag from missing index",
            "Why does the ledger consumer lag at 2am?",
            "A sequential scan on entries: posted_at had no index. Migration 0042 adds one; the batch takes 40 seconds now.",
            180,
        ),
        180,
    );
    write(
        &crashed,
        &transcript(
            "crashed",
            "Refund webhook retries",
            "Make refund webhooks retry with backoff",
            "Refund webhooks now retry five times with exponential backoff, and give up into the dead-letter table.",
            60,
        ),
        60,
    );
    // Another window, still at work: not a missed session.
    write(
        &running,
        &transcript(
            "running",
            "Still going",
            "Look at the invoices",
            "Looking at the invoices now, one moment please.",
            1,
        ),
        1,
    );

    // The older session ended normally: its SessionEnd wrote it up.
    sessions::import_one(&nb, &settings, None, Local::now(), "", &older).unwrap();
    let now = Local::now();
    assert_eq!(
        sessions::missed(&nb, &settings, &sessions, &current, now),
        Some(crashed.clone()),
        "the crashed session is the one without a write-up; the running one is left alone"
    );
    sessions::import_one(&nb, &settings, None, Local::now(), "", &crashed).unwrap();
    assert_eq!(
        sessions::missed(&nb, &settings, &sessions, &current, now),
        None,
        "nothing is missed once it's written up"
    );

    // A session with nothing in it is remembered as looked at, so it isn't found again.
    let empty = sessions.join("empty.jsonl");
    write(
        &empty,
        &line(json!({"type": "ai-title", "aiTitle": "Nothing", "sessionId": "empty"})),
        45,
    );
    assert_eq!(
        sessions::missed(&nb, &settings, &sessions, &current, now),
        Some(empty.clone())
    );
    let report = sessions::import_one(&nb, &settings, None, Local::now(), "", &empty).unwrap();
    assert!(report.added.is_empty(), "{report:?}");
    assert_eq!(
        sessions::missed(&nb, &settings, &sessions, &current, now),
        None
    );

    // From a worktree of the project, newest first, with where to read more.
    let context = sessions::context(
        &nb,
        &settings,
        "/home/dev/Payments/.claude/worktrees/new-thing",
    )
    .expect("there are earlier sessions");
    assert!(
        context.contains("Earlier sessions in Payments"),
        "{context}"
    );
    assert!(context.contains("read_note") && context.contains("search_notes"));
    let refund = context.find("Refund webhook retries").expect(&context);
    let ledger = context
        .find("Ledger lag from missing index")
        .expect(&context);
    assert!(refund < ledger, "newest first:\n{context}");
    assert!(context.contains("on main"), "{context}");
    assert!(
        context.contains("dead-letter table"),
        "the summary comes along:\n{context}"
    );
    assert!(
        context.contains("(AI sessions/Payments/"),
        "each says where it is:\n{context}"
    );
    assert!(!context.contains("Still going"), "not written up yet");

    let recent = sessions::recent(&nb, &settings, "/home/dev/Payments", 1);
    assert_eq!(recent.len(), 1);
    assert_eq!(recent[0].title, "Refund webhook retries");

    // Another project has nothing to say.
    assert_eq!(sessions::context(&nb, &settings, "/home/dev/Search"), None);
}
