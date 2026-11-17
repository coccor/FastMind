//! Dream with a model: statistics find candidates, the model decides and writes.

use chrono::Local;
use fastmind_core::Notebook;
use fastmind_core::config::DreamSettings;
use fastmind_core::dream::{self, Diagram, Kind, Plan, Proposal, Report, Status};
use fastmind_core::{Error, Result};
use serde_json::{Value, json};
use std::fs;
use std::sync::Mutex;
use std::time::SystemTime;

fn write(nb: &Notebook, rel: &str, text: &str, days_ago: u64) {
    let path = nb.root.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, text).unwrap();
    let at = SystemTime::now() - std::time::Duration::from_secs(days_ago * 86_400);
    fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(at)
        .unwrap();
}

/// The id the user turn gave the note titled `title`.
fn id_of(user: &str, title: &str) -> Option<String> {
    let at = user.find(&format!("title=\"{title}\""))?;
    let start = user[..at].rfind("id=\"")? + 4;
    let end = start + user[start..].find('"')?;
    Some(user[start..end].to_owned())
}

/// A model with fixed opinions about this notebook.
fn model(system: &str, user: &str, _schema: &Value) -> Result<String> {
    let answer = if system.contains("# Merge duplicate notes") {
        match (
            id_of(user, "Conference wifi"),
            id_of(user, "Wifi at the conference"),
        ) {
            (Some(a), Some(b)) => json!({"groups": [{
                "keep": a, "absorb": [b], "reason": "Both are the conference wifi details.",
                "text": "# Conference wifi\n\nNetwork DevConf-Guest, password orange-falcon-42 (from the badge desk)."
            }]}),
            _ => json!({"groups": []}),
        }
    } else if system.contains("# Gather short notes into a topic") {
        let ids: Vec<String> = ["Starter feeding", "Starter too sour", "Starter rye"]
            .iter()
            .filter_map(|t| id_of(user, t))
            .collect();
        json!({"members": ids, "title": "Sourdough starter", "summary": "How the starter is kept.",
               "body": "## Feeding\n\nDaily, rye helps.\n\n## Sourness\n\nFeed more often.", "reason": "All about the starter."})
    } else if system.contains("# Decide what to archive") {
        let keep_reason = id_of(user, "Old decision");
        let archive = id_of(user, "Old todo");
        let mut decisions = Vec::new();
        if let Some(id) = archive {
            decisions.push(json!({"id": id, "archive": true, "reason": "A finished to-do list."}));
        }
        if let Some(id) = keep_reason {
            decisions
                .push(json!({"id": id, "archive": false, "reason": "A decision worth keeping."}));
        }
        json!({"decisions": decisions})
    } else if system.contains("# Mend broken links") {
        json!({"fixes": [{"id": "l1", "target": "Release checklist", "reason": "A typo of Release checklist."}]})
    } else {
        return Err(Error::msg("unexpected question"));
    };
    Ok(answer.to_string())
}

#[test]
fn the_model_decides_and_writes_and_is_asked_once() {
    let dir = tempfile::tempdir().unwrap();
    let nb = Notebook::open(dir.path()).unwrap();
    // Paraphrased copies: too different for statistics alone, the model says they're one.
    write(
        &nb,
        "Conference wifi.md",
        "# Conference wifi\n\nThe conference wifi network is DevConf-Guest, password orange-falcon-42.\n",
        10,
    );
    write(
        &nb,
        "Wifi at the conference.md",
        "# Wifi at the conference\n\nGot the conference wifi password from the badge desk: orange-falcon-42 on DevConf-Guest.\n",
        9,
    );
    for (name, body) in [
        (
            "Starter feeding",
            "Feed the sourdough starter every morning with flour and water.",
        ),
        (
            "Starter too sour",
            "The sourdough starter smells too sour, feed the starter more often.",
        ),
        (
            "Starter rye",
            "Rye flour in the sourdough starter feeding makes the starter rise faster.",
        ),
    ] {
        write(
            &nb,
            &format!("kitchen/{name}.md"),
            &format!("# {name}\n\n{body}\n"),
            20,
        );
    }
    write(
        &nb,
        "Old todo.md",
        "# Old todo\n\n- [x] buy milk\n- [x] call the bank\n",
        400,
    );
    write(
        &nb,
        "Old decision.md",
        "# Old decision\n\nWe chose Postgres over Mongo because of transactions across accounts.\n",
        400,
    );
    write(
        &nb,
        "Release checklist.md",
        "# Release checklist\n\nTag, build, sign.\n",
        30,
    );
    write(
        &nb,
        "Shipping.md",
        "# Shipping\n\nFollow [[Relase checklist]] before tagging.\n",
        30,
    );

    let asked = Mutex::new(Vec::<String>::new());
    let ask = |system: &str, user: &str, schema: &Value| {
        asked
            .lock()
            .unwrap()
            .push(system.lines().next().unwrap_or("").to_owned());
        model(system, user, schema)
    };
    let report = dream::run(&nb, &DreamSettings::default(), Some(&ask), Local::now()).unwrap();
    let find = |kind: Kind| report.proposals.iter().filter(move |p| p.kind == kind);

    let merge = find(Kind::Merge)
        .next()
        .expect("the model's merge is proposed");
    assert_eq!(merge.reason, "Both are the conference wifi details.");
    assert!(
        merge
            .preview
            .as_ref()
            .unwrap()
            .contains("orange-falcon-42 (from the badge desk)")
    );
    let topic = find(Kind::Topic)
        .next()
        .expect("the model's topic is proposed");
    assert!(
        topic.title.contains("\"Sourdough starter\""),
        "{}",
        topic.title
    );
    let preview = topic.preview.as_ref().unwrap();
    assert!(
        preview.contains("type: topic")
            && preview.contains("## Feeding")
            && preview.contains("## Gathered from"),
        "{preview}"
    );
    let archived: Vec<&str> = find(Kind::Archive).map(|p| p.title.as_str()).collect();
    assert_eq!(
        archived,
        ["Archive \"Old todo\""],
        "the model keeps the decision"
    );
    assert!(
        find(Kind::Archive)
            .next()
            .unwrap()
            .reason
            .starts_with("A finished to-do list.")
    );
    let fix = find(Kind::FixLink)
        .next()
        .expect("the model's link fix is proposed");
    assert!(
        matches!(&fix.plan, Plan::FixLink { from, to, .. } if from == "Relase checklist" && to == "Release checklist")
    );
    let first_night = asked.lock().unwrap().len();
    assert!(
        first_night >= 4,
        "merge, topic, archive and links were asked: {first_night}"
    );

    // The same notes again: nothing new to ask.
    let again = dream::run(&nb, &DreamSettings::default(), Some(&ask), Local::now()).unwrap();
    assert_eq!(
        asked.lock().unwrap().len(),
        first_night,
        "verdicts are reused"
    );
    assert_eq!(again.proposals.len(), report.proposals.len());

    // A source note edited after Dream wrote the merge: it isn't overwritten.
    let merge_id = again
        .proposals
        .iter()
        .find(|p| p.kind == Kind::Merge)
        .unwrap()
        .id
        .clone();
    write(
        &nb,
        "Wifi at the conference.md",
        "# Wifi at the conference\n\nThe password changed: lime-otter-7.\n",
        0,
    );
    let outcome = dream::apply(&nb, &merge_id, Local::now()).unwrap();
    assert_eq!(outcome.status, Status::Outdated);
    assert!(nb.exists("Wifi at the conference.md"));

    // The topic applies exactly as previewed.
    let topic_id = again
        .proposals
        .iter()
        .find(|p| p.kind == Kind::Topic)
        .unwrap()
        .id
        .clone();
    let applied = dream::apply(&nb, &topic_id, Local::now()).unwrap();
    assert_eq!(applied.status, Status::Applied);
    let Plan::Topic { path, .. } = &applied.plan else {
        unreachable!()
    };
    assert_eq!(
        &fs::read_to_string(nb.root.join(path)).unwrap(),
        applied.preview.as_ref().unwrap()
    );
    assert!(
        !nb.exists("kitchen/Starter feeding.md"),
        "fragments are archived"
    );
}

#[test]
fn a_model_that_cant_answer_leaves_the_notebook_alone_and_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let nb = Notebook::open(dir.path()).unwrap();
    write(
        &nb,
        "Conference wifi.md",
        "# Conference wifi\n\nThe conference wifi network is DevConf-Guest, password orange-falcon-42.\n",
        10,
    );
    write(
        &nb,
        "Wifi at the conference.md",
        "# Wifi at the conference\n\nThe conference wifi password is orange-falcon-42 on DevConf-Guest.\n",
        9,
    );
    write(&nb, "Old todo.md", "# Old todo\n\n- [x] buy milk\n", 400);
    let ask = |_: &str, _: &str, _: &Value| -> Result<String> {
        Err(Error::msg("Claude Code isn't signed in"))
    };
    let report = dream::run(&nb, &DreamSettings::default(), Some(&ask), Local::now()).unwrap();
    assert!(
        report
            .proposals
            .iter()
            .all(|p| matches!(p.kind, Kind::Pack | Kind::Settle)),
        "no unjudged proposals"
    );
    assert!(
        report.summary.contains("Claude Code isn't signed in"),
        "{}",
        report.summary
    );
}

#[test]
fn unlinked_fdraw_is_archived_like_an_image() {
    let dir = tempfile::tempdir().unwrap();
    let nb = Notebook::open(dir.path()).unwrap();
    write(
        &nb,
        "x/a note.md",
        "# A note\n\nNothing links to a drawing.\n",
        1,
    );
    write(&nb, "x/attachments/old.fdraw", "box A\n", 200);
    let report = dream::run(&nb, &DreamSettings::default(), None, Local::now()).unwrap();
    assert!(
        report.proposals.iter().any(|p| p.kind == Kind::Archive
            && matches!(&p.plan, Plan::Archive { path } if path == "x/attachments/old.fdraw")),
        "{:?}",
        report
            .proposals
            .iter()
            .map(|p| &p.title)
            .collect::<Vec<_>>()
    );
}

// ---- Diagrams in plans ----

fn diagram(caption: &str) -> Diagram {
    Diagram {
        file: "attachments/Note.fdraw".into(),
        caption: caption.into(),
        text: "box A\nbox B\nA -> B\n".into(),
    }
}

fn proposal(id: &str, kind: Kind, notes: &[&str], plan: Plan) -> Proposal {
    Proposal {
        id: id.into(),
        kind,
        title: format!("Proposal {id}"),
        reason: "test".into(),
        notes: notes.iter().map(|n| n.to_string()).collect(),
        notes_removed: 0,
        bytes_freed: 0,
        disk_freed: 0,
        preview: None,
        status: Status::Pending,
        entry: None,
        error: None,
        plan,
        sources: Vec::new(),
    }
}

/// Saves `proposals` as the notebook's latest Dream report.
fn save_report(nb: &Notebook, proposals: Vec<Proposal>) {
    let report = Report {
        id: "r1".into(),
        ran_at: Local::now(),
        took_ms: 0,
        before: Default::default(),
        proposals,
        model: None,
        model_notes: Vec::new(),
        summary: String::new(),
    };
    dream::save_report(nb, &report).unwrap();
}

fn read(nb: &Notebook, rel: &str) -> String {
    fs::read_to_string(nb.root.join(rel)).unwrap()
}

fn diagram_plan(caption: &str) -> Plan {
    Plan::Diagram {
        note: "notes/Note.md".into(),
        diagram: diagram(caption),
    }
}

#[test]
fn old_report_loads() {
    let dir = tempfile::tempdir().unwrap();
    let nb = Notebook::open(dir.path()).unwrap();
    let json = json!({
        "id": "r0", "ran_at": "2026-01-01T00:00:00+00:00", "took_ms": 1,
        "before": {"notes": 0, "bytes": 0, "attachments": 0, "attachment_bytes": 0,
                   "archive_bytes": 0, "journal_bytes": 0},
        "proposals": [{
            "id": "p1", "kind": "topic", "title": "T", "reason": "r", "notes": ["a.md"],
            "notes_removed": 1, "bytes_freed": 0, "preview": null, "status": "pending",
            "plan": {"type": "topic", "path": "T.md", "title": "T", "summary": "s",
                     "fragments": ["a.md"]}
        }, {
            "id": "p2", "kind": "merge", "title": "M", "reason": "r", "notes": ["a.md"],
            "notes_removed": 1, "bytes_freed": 0, "preview": null, "status": "pending",
            "plan": {"type": "merge", "keep": "a.md", "absorb": ["b.md"]}
        }],
        "model": null, "summary": ""
    });
    let state = nb
        .root
        .join(fastmind_core::notebook::STATE_DIR)
        .join("dream");
    fs::create_dir_all(&state).unwrap();
    fs::write(state.join("latest.json"), json.to_string()).unwrap();
    let report = dream::latest(&nb).expect("an old report loads");
    assert!(matches!(
        &report.proposals[0].plan,
        Plan::Topic { diagram: None, .. }
    ));
    assert!(matches!(
        &report.proposals[1].plan,
        Plan::Merge { diagram: None, .. }
    ));
}

#[test]
fn kind_parses_diagram() {
    assert_eq!(Kind::parse("draw"), Some(Kind::Diagram));
    assert_eq!(Kind::parse("diagram"), Some(Kind::Diagram));
}

#[test]
fn applying_diagram_plan_writes_file_and_link() {
    let dir = tempfile::tempdir().unwrap();
    let nb = Notebook::open(dir.path()).unwrap();
    let original = "# Note\n\nBody.\n";
    write(&nb, "notes/Note.md", original, 1);
    save_report(
        &nb,
        vec![proposal(
            "d1",
            Kind::Diagram,
            &["notes/Note.md"],
            diagram_plan("c"),
        )],
    );
    let done = dream::apply(&nb, "d1", Local::now()).unwrap();
    assert_eq!(done.status, Status::Applied, "{:?}", done.error);
    assert_eq!(
        read(&nb, "notes/attachments/Note.fdraw"),
        "box A\nbox B\nA -> B\n"
    );
    assert!(read(&nb, "notes/Note.md").contains("![c](attachments/Note.fdraw)"));
    dream::undo(&nb, "d1").unwrap();
    assert!(!nb.root.join("notes/attachments/Note.fdraw").exists());
    assert_eq!(read(&nb, "notes/Note.md"), original);
}

#[test]
fn second_diagram_gets_a_free_name() {
    let dir = tempfile::tempdir().unwrap();
    let nb = Notebook::open(dir.path()).unwrap();
    write(&nb, "notes/Note.md", "# Note\n\nBody.\n", 1);
    write(&nb, "notes/attachments/Note.fdraw", "box Old\n", 1);
    save_report(
        &nb,
        vec![proposal(
            "d1",
            Kind::Diagram,
            &["notes/Note.md"],
            diagram_plan("c"),
        )],
    );
    let done = dream::apply(&nb, "d1", Local::now()).unwrap();
    assert_eq!(done.status, Status::Applied, "{:?}", done.error);
    assert_eq!(read(&nb, "notes/attachments/Note.fdraw"), "box Old\n");
    assert_eq!(
        read(&nb, "notes/attachments/Note 2.fdraw"),
        "box A\nbox B\nA -> B\n"
    );
    assert!(read(&nb, "notes/Note.md").contains("![c](attachments/Note%202.fdraw)"));
}

#[test]
fn topic_with_diagram_writes_both() {
    let dir = tempfile::tempdir().unwrap();
    let nb = Notebook::open(dir.path()).unwrap();
    write(&nb, "kitchen/Feeding.md", "# Feeding\n\nDaily.\n", 1);
    let plan = Plan::Topic {
        path: "kitchen/Starter.md".into(),
        title: "Starter".into(),
        summary: "How it is kept.".into(),
        fragments: vec!["kitchen/Feeding.md".into()],
        text: Some("# Starter\n\n> How it is kept.\n\nDaily.\n".into()),
        diagram: Some(diagram("flow")),
    };
    save_report(
        &nb,
        vec![proposal("t1", Kind::Topic, &["kitchen/Feeding.md"], plan)],
    );
    let done = dream::apply(&nb, "t1", Local::now()).unwrap();
    assert_eq!(done.status, Status::Applied, "{:?}", done.error);
    let topic = read(&nb, "kitchen/Starter.md");
    assert!(
        topic.contains("> How it is kept.\n\n![flow](attachments/Starter.fdraw)\n"),
        "{topic}"
    );
    assert!(nb.root.join("kitchen/attachments/Starter.fdraw").exists());
    // One journal entry holds both, so one undo removes both.
    dream::undo(&nb, "t1").unwrap();
    assert!(!nb.root.join("kitchen/Starter.md").exists());
    assert!(!nb.root.join("kitchen/attachments/Starter.fdraw").exists());
    assert!(nb.root.join("kitchen/Feeding.md").exists());
}

#[test]
fn merge_with_diagram_writes_it_beside_the_kept_note() {
    let dir = tempfile::tempdir().unwrap();
    let nb = Notebook::open(dir.path()).unwrap();
    write(&nb, "w/Keep.md", "# Keep\n\nOne.\n", 1);
    write(&nb, "w/Other.md", "# Other\n\nOne again.\n", 1);
    let plan = Plan::Merge {
        keep: "w/Keep.md".into(),
        absorb: vec!["w/Other.md".into()],
        text: Some("# Keep\n\nOne.\n".into()),
        diagram: Some(diagram("c")),
    };
    save_report(
        &nb,
        vec![proposal(
            "m1",
            Kind::Merge,
            &["w/Keep.md", "w/Other.md"],
            plan,
        )],
    );
    let done = dream::apply(&nb, "m1", Local::now()).unwrap();
    assert_eq!(done.status, Status::Applied, "{:?}", done.error);
    assert!(read(&nb, "w/Keep.md").contains("![c](attachments/Keep.fdraw)"));
    assert!(nb.root.join("w/attachments/Keep.fdraw").exists());
}

#[test]
fn diagram_link_for_a_name_with_parentheses_resolves() {
    let dir = tempfile::tempdir().unwrap();
    let nb = Notebook::open(dir.path()).unwrap();
    write(
        &nb,
        "notes/Redis (eviction).md",
        "# Redis (eviction)\n\nBody.\n",
        1,
    );
    let plan = Plan::Diagram {
        note: "notes/Redis (eviction).md".into(),
        diagram: diagram("c"),
    };
    save_report(
        &nb,
        vec![proposal(
            "d1",
            Kind::Diagram,
            &["notes/Redis (eviction).md"],
            plan,
        )],
    );
    let done = dream::apply(&nb, "d1", Local::now()).unwrap();
    assert_eq!(done.status, Status::Applied, "{:?}", done.error);
    let text = read(&nb, "notes/Redis (eviction).md");
    assert!(
        text.contains("![c](attachments/Redis%20%28eviction%29.fdraw)"),
        "{text}"
    );
    let link = text.split("](").nth(1).unwrap().split(')').next().unwrap();
    let target = fastmind_core::links::relative_target_path("notes/Redis (eviction).md", link);
    assert_eq!(target, "notes/attachments/Redis (eviction).fdraw");
    assert!(nb.exists(&target));
}
