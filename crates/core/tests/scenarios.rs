//! The scenarios FastMind promises, end to end through the same command surface the desktop app
//! uses: capture and find it again; let Dream compress a messy notebook; undo any part of it;
//! get an archived note back, even from a pack.

use fastmind_core::Notebook;
use fastmind_core::api::App;
use fastmind_core::config::Settings;
use fastmind_core::links::Resolver;
use serde_json::{Value, json};
use std::fs;
use std::path::Path;

fn app_for(dir: &Path) -> App {
    let settings = Settings {
        notebook: Some(dir.to_path_buf()),
        ..Settings::default()
    };
    App::ephemeral(settings).offline_for_tests()
}

fn demo_app() -> (tempfile::TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    let app = app_for(&dir.path().join("nb"));
    app.call("demo_create", json!({ "path": dir.path().join("nb") }))
        .unwrap();
    (dir, app)
}

fn proposal<'a>(report: &'a Value, needle: &str) -> &'a Value {
    report["proposals"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["title"].as_str().unwrap().contains(needle))
        .unwrap_or_else(|| panic!("no proposal mentioning {needle}: {report:#}"))
}

fn titles(report: &Value, kind: &str) -> Vec<String> {
    report["proposals"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|p| p["kind"] == kind)
        .map(|p| p["title"].as_str().unwrap().to_owned())
        .collect()
}

fn note_paths(root: &Path) -> Vec<String> {
    let mut paths: Vec<String> = Notebook::open(root)
        .unwrap()
        .notes()
        .into_iter()
        .map(|n| n.path)
        .collect();
    paths.sort();
    paths
}

#[test]
fn capture_then_find_and_ask() {
    let dir = tempfile::tempdir().unwrap();
    let app = app_for(dir.path());
    let saved = app
        .call("capture", json!({ "text": "Flaky test in checkout: the payment mock times out after 2s on CI. #ci\nRaise the mock timeout to 10s." }))
        .unwrap();
    assert_eq!(
        saved["path"],
        "Fresh memories/Flaky test in checkout the payment mock times out after 2s on CI.md"
    );

    let hits = app
        .call("search", json!({ "query": "payment mock timeout" }))
        .unwrap();
    assert_eq!(hits[0]["path"], saved["path"]);
    let tagged = app.call("search", json!({ "query": "#ci" })).unwrap();
    assert_eq!(tagged.as_array().unwrap().len(), 1);

    let answer = app
        .call(
            "ask",
            json!({ "question": "what timeout should the payment mock use?" }),
        )
        .unwrap();
    assert!(
        answer["text"].as_str().unwrap().contains("10s"),
        "{answer:#}"
    );
    assert_eq!(answer["sources"][0]["path"], saved["path"]);

    let inbox = app.call("inbox", json!({})).unwrap();
    assert_eq!(inbox.as_array().unwrap().len(), 1);
    app.call("inbox_keep", json!({ "path": saved["path"] }))
        .unwrap();
    assert!(
        app.call("inbox", json!({}))
            .unwrap()
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn dream_compresses_a_messy_notebook() {
    let (dir, app) = demo_app();
    let root = dir.path().join("nb");
    let before = note_paths(&root);
    assert_eq!(before.len(), 22);

    let run = app.call("dream_run", json!({})).unwrap();
    let report = &run["report"];
    assert_eq!(titles(report, "merge").len(), 2, "{report:#}");
    proposal(report, "into \"Docker build cache\"");
    proposal(report, "into \"Redis eviction policy\"");
    let topics = titles(report, "topic");
    assert_eq!(
        topics,
        vec![
            "Gather 4 short notes into \"Kubernetes probe\"",
            "Gather 3 short notes into \"Sourdough starter\""
        ]
    );
    let archives = titles(report, "archive");
    assert_eq!(archives.len(), 5, "{archives:?}");
    for expected in [
        "Meeting room booking",
        "Untitled",
        "Conference wifi",
        "Migrate CI",
        "old architecture.png",
    ] {
        assert!(
            archives.iter().any(|t| t.contains(expected)),
            "{expected} not proposed: {archives:?}"
        );
    }
    assert_eq!(titles(report, "fix_link").len(), 1);
    // Packing is applied without asking, by default.
    assert_eq!(run["auto_applied"], 1);
    assert_eq!(proposal(report, "Pack")["status"], "applied");

    // Protected notes are never proposed: the project home note and a pinned note.
    for p in report["proposals"].as_array().unwrap() {
        let notes = p["notes"].to_string();
        assert!(
            !notes.contains("Work.md") && !notes.contains("Reading list"),
            "{p:#}"
        );
    }

    let applied = app.call("dream_apply_all", json!({})).unwrap();
    assert!(
        applied
            .as_array()
            .unwrap()
            .iter()
            .all(|p| p["status"] == "applied"),
        "{applied:#}"
    );
    let after = note_paths(&root);
    assert_eq!(after.len(), 10, "{after:?}");
    assert!(after.contains(&"ideas/Kubernetes probe.md".to_owned()));
    assert!(after.contains(&"home/Sourdough starter.md".to_owned()));

    // Nothing left points nowhere: every link resolves after the merges and the fix.
    let nb = Notebook::open(&root).unwrap();
    let notes = nb.notes();
    let resolver = Resolver::new(&notes);
    for note in &notes {
        for target in &note.links {
            assert!(
                resolver.resolve(target).is_some(),
                "{} links to missing {target}",
                note.path
            );
        }
    }

    // What the copies added survives in the note that stayed.
    let redis = fs::read_to_string(root.join("work/Redis eviction policy.md")).unwrap();
    assert!(redis.contains("Watch evicted_keys in INFO stats"));
    assert!(redis.contains("aliases: [Redis eviction]"));

    // Archived notes are still found, ranked below live ones.
    let hits = app
        .call("search", json!({ "query": "conference wifi password" }))
        .unwrap();
    assert_eq!(hits[0]["path"], "Fresh memories/Conference wifi.md");
    assert!(!hits[0]["archived"].is_null());

    // A second night finds nothing more to do.
    let again = app.call("dream_run", json!({})).unwrap();
    assert!(
        again["report"]["proposals"].as_array().unwrap().is_empty(),
        "{:#}",
        again["report"]
    );
}

#[test]
fn any_dream_change_can_be_undone_on_its_own() {
    let (dir, app) = demo_app();
    let root = dir.path().join("nb");
    let report = app.call("dream_run", json!({})).unwrap()["report"].clone();
    app.call("dream_apply_all", json!({})).unwrap();

    // Undo only the Kubernetes topic: its four fragments come back, the topic note goes.
    let topic = proposal(&report, "Kubernetes probe")["id"]
        .as_str()
        .unwrap()
        .to_owned();
    app.call("dream_undo", json!({ "id": topic })).unwrap();
    let paths = note_paths(&root);
    assert!(!paths.contains(&"ideas/Kubernetes probe.md".to_owned()));
    for fragment in [
        "Fresh memories/Readiness probe.md",
        "Fresh memories/Liveness probe restarts.md",
        "ideas/Startup probe.md",
        "Fresh memories/probe timeouts.md",
    ] {
        assert!(
            paths.contains(&fragment.to_owned()),
            "{fragment} not back: {paths:?}"
        );
    }
    // Everything else Dream did stays done.
    assert!(!paths.contains(&"Fresh memories/Redis eviction.md".to_owned()));
    let history = app.call("history", json!({})).unwrap();
    let blocked: Vec<&Value> = history
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["undone"] == false && e["can_undo"] == false)
        .collect();
    assert!(
        blocked.is_empty(),
        "other changes became impossible to undo: {blocked:#?}"
    );

    // An edit made since is never overwritten: undo refuses instead.
    let merge = proposal(&report, "Redis eviction policy")["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let kept = root.join("work/Redis eviction policy.md");
    let mut text = fs::read_to_string(&kept).unwrap();
    text.push_str("\nMy own addition.\n");
    fs::write(&kept, &text).unwrap();
    let refused = app.call("dream_undo", json!({ "id": merge }));
    assert!(refused.unwrap_err().0.contains("changed since"));
    assert_eq!(fs::read_to_string(&kept).unwrap(), text);
}

#[test]
fn archived_notes_come_back_even_from_a_pack() {
    let (dir, app) = demo_app();
    let root = dir.path().join("nb");
    app.call("dream_run", json!({})).unwrap();
    let archive = app.call("archive_list", json!({})).unwrap();
    let packed: Vec<&Value> = archive["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| !i["pack"].is_null())
        .collect();
    assert_eq!(packed.len(), 2, "{archive:#}");
    assert!(
        !root.join(".fastmind/archive/files").exists()
            || fs::read_dir(root.join(".fastmind/archive/files"))
                .unwrap()
                .next()
                .is_none()
    );

    let id = packed
        .iter()
        .find(|i| i["original"] == "work/Q1 planning.md")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let restored = app.call("archive_restore", json!({ "id": id })).unwrap();
    assert_eq!(restored["path"], "work/Q1 planning.md");
    assert!(
        fs::read_to_string(root.join("work/Q1 planning.md"))
            .unwrap()
            .contains("Hire two engineers")
    );
}
