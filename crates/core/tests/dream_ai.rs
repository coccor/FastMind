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

/// The diagram the model draws for the starter topic.
const STARTER_DIAGRAM: &str = "```fdraw\nfeed: node \"Feed daily\" at 40,40\nrise: node \"Rises\" at 240,40\nfeed -> rise\n```";

/// The diagram the model draws for the note titled "Request path".
const REQUEST_DIAGRAM: &str =
    "gw: node \"Gateway\" at 40,40\ndb: node \"Database\" at 240,40\ngw -> db \"writes\"";

/// A model with fixed opinions about this notebook.
fn model(system: &str, user: &str, _schema: &Value) -> Result<String> {
    let answer = if system.contains("# Draw a diagram") {
        if user.contains("title=\"Request path\"") {
            json!({"worth": true, "kind": "architecture",
                   "caption": "How a request reaches the database", "diagram": REQUEST_DIAGRAM,
                   "reason": "The note follows a request through five services."})
        } else {
            json!({"worth": false, "kind": "", "caption": "", "diagram": "",
                   "reason": "It reads well as it is."})
        }
    } else if system.contains("# Merge duplicate notes") {
        match (
            id_of(user, "Conference wifi"),
            id_of(user, "Wifi at the conference"),
        ) {
            (Some(a), Some(b)) => json!({"groups": [{
                "keep": a, "absorb": [b], "reason": "Both are the conference wifi details.",
                "text": "# Conference wifi\n\nNetwork DevConf-Guest, password orange-falcon-42 (from the badge desk).",
                "diagram": "", "diagram_caption": ""
            }]}),
            _ => json!({"groups": []}),
        }
    } else if system.contains("# Gather short notes into a topic") {
        let ids: Vec<String> = ["Starter feeding", "Starter too sour", "Starter rye"]
            .iter()
            .filter_map(|t| id_of(user, t))
            .collect();
        json!({"members": ids, "title": "Sourdough starter", "summary": "How the starter is kept.",
               "body": "## Feeding\n\nDaily, rye helps.\n\n## Sourness\n\nFeed more often.", "reason": "All about the starter.",
               "diagram": STARTER_DIAGRAM, "diagram_caption": "How the starter is kept"})
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

// ---- Diagrams in merge and topic answers ----

/// The three starter fragments and the two wifi copies.
fn starter_and_wifi(nb: &Notebook) {
    write(
        nb,
        "Conference wifi.md",
        "# Conference wifi\n\nThe conference wifi network is DevConf-Guest, password orange-falcon-42.\n",
        10,
    );
    write(
        nb,
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
            nb,
            &format!("kitchen/{name}.md"),
            &format!("# {name}\n\n{body}\n"),
            20,
        );
    }
}

fn answers(model: dream::ai::Ask, nb: &Notebook) -> Report {
    dream::run(nb, &DreamSettings::default(), Some(model), Local::now()).unwrap()
}

#[test]
fn topic_proposal_carries_its_diagram() {
    let dir = tempfile::tempdir().unwrap();
    let nb = Notebook::open(dir.path()).unwrap();
    starter_and_wifi(&nb);
    let report = answers(&model, &nb);
    let topic = report
        .proposals
        .iter()
        .find(|p| p.kind == Kind::Topic)
        .expect("the topic is proposed");
    let Plan::Topic {
        diagram: Some(d), ..
    } = &topic.plan
    else {
        panic!(
            "the topic has its diagram: {:?} {:?}",
            topic.plan, report.model_notes
        )
    };
    assert_eq!(d.file, "attachments/Sourdough starter.fdraw");
    assert_eq!(d.caption, "How the starter is kept");
    assert!(d.text.starts_with("feed: node"), "unfenced: {}", d.text);
    let preview = topic.preview.as_ref().unwrap();
    assert!(
        preview.contains(
            "> How the starter is kept.\n\n![How the starter is kept](attachments/Sourdough%20starter.fdraw)\n"
        ),
        "{preview}"
    );
}

#[test]
fn merge_without_diagram_has_none() {
    let dir = tempfile::tempdir().unwrap();
    let nb = Notebook::open(dir.path()).unwrap();
    starter_and_wifi(&nb);
    let report = answers(&model, &nb);
    let merge = report
        .proposals
        .iter()
        .find(|p| p.kind == Kind::Merge)
        .expect("the merge is proposed");
    assert!(matches!(&merge.plan, Plan::Merge { diagram: None, .. }));
    assert!(!merge.preview.as_ref().unwrap().contains(".fdraw"));
}

#[test]
fn broken_topic_diagram_is_held() {
    let dir = tempfile::tempdir().unwrap();
    let nb = Notebook::open(dir.path()).unwrap();
    starter_and_wifi(&nb);
    let broken = |system: &str, user: &str, schema: &Value| -> Result<String> {
        let mut answer: Value = serde_json::from_str(&model(system, user, schema)?).unwrap();
        if answer.get("members").is_some() {
            answer["diagram"] = json!("feed: node \"Feed\" at 40,40\nfeed -> nowhere\n");
        }
        Ok(answer.to_string())
    };
    let report = answers(&broken, &nb);
    let topic = report
        .proposals
        .iter()
        .find(|p| p.kind == Kind::Topic)
        .expect("the topic is still proposed");
    assert!(matches!(&topic.plan, Plan::Topic { diagram: None, .. }));
    assert!(!topic.preview.as_ref().unwrap().contains(".fdraw"));
    assert!(
        report
            .model_notes
            .iter()
            .any(|n| n.starts_with("1 diagram dropped: Line")),
        "{:?}",
        report.model_notes
    );
    assert!(
        !report.summary.contains("couldn't answer"),
        "a dropped diagram isn't a failed question: {}",
        report.summary
    );
}

// ---- The final diagram pass: fixes and the diagram kind ----

/// A note of about 300 words following a request through services, step by step.
fn structural(title: &str, rounds: usize) -> String {
    let steps = [
        "The browser sends the request to the gateway, which checks the session cookie first.",
        "Then the gateway calls the auth service, and the auth service sends a signed token back.",
        "After that the gateway passes the request to the orders service with the token attached.",
        "The orders service depends on the inventory service and calls it before it writes anything.",
        "Then the orders service writes the order to the database and sends an event to the queue.",
        "The mailer reads the queue, and after the event arrives it sends the receipt to the customer.",
    ];
    let mut text = format!("# {title}\n\n");
    for round in 1..=rounds {
        text.push_str(&format!("## Stage {round}\n\n"));
        for (i, step) in steps.iter().enumerate() {
            text.push_str(&format!("{}. {step}\n", i + 1));
        }
        text.push('\n');
    }
    text
}

/// About 300 words of facts in a bullet list: nothing a diagram would show.
fn fact_list() -> String {
    let facts = [
        "Paris is the capital of France and sits on the Seine river.",
        "Lisbon is the capital of Portugal and faces the Atlantic ocean.",
        "Oslo is the capital of Norway and lies at the head of a fjord.",
        "Vienna is the capital of Austria and sits on the Danube river.",
        "Dublin is the capital of Ireland and faces the Irish sea.",
        "Madrid is the capital of Spain and lies on a high plateau.",
    ];
    let mut text = String::from("# Capitals\n\n");
    for _ in 0..4 {
        for fact in facts {
            text.push_str(&format!("- {fact}\n"));
        }
    }
    text
}

/// What a question is: a diagram fix, a diagram question, a merge or something else.
fn label(system: &str, user: &str) -> &'static str {
    if user.contains("<errors>") {
        "fix"
    } else if system.contains("# Draw a diagram") {
        "draw"
    } else if system.contains("# Merge duplicate notes") {
        "merge"
    } else {
        "other"
    }
}

type Answer<'a> = &'a (dyn Fn(&str, &str, &Value) -> Result<String> + Sync);

/// Dream with `answer`, and the questions it asked, by [`label`].
fn counted(nb: &Notebook, settings: &DreamSettings, answer: Answer) -> (Report, Vec<&'static str>) {
    let asked = Mutex::new(Vec::new());
    let ask = |system: &str, user: &str, schema: &Value| {
        asked.lock().unwrap().push(label(system, user));
        answer(system, user, schema)
    };
    let report = dream::run(nb, settings, Some(&ask), Local::now()).unwrap();
    (report, asked.into_inner().unwrap())
}

fn count(asked: &[&str], kind: &str) -> usize {
    asked.iter().filter(|a| **a == kind).count()
}

/// Three pairs of notes that say the same thing.
fn three_merge_groups(nb: &Notebook) {
    for (name, body) in [
        (
            "Conference wifi",
            "The conference wifi network is DevConf-Guest, password orange-falcon-42.",
        ),
        (
            "Wifi at the conference",
            "Got the conference wifi password from the badge desk: orange-falcon-42 on DevConf-Guest.",
        ),
        (
            "Printer setup",
            "The office printer is a Brother laser, add it by address with the generic PCL driver.",
        ),
        (
            "Setting up the printer",
            "Add the office Brother laser printer by its address, using the generic PCL driver.",
        ),
        (
            "Gym hours",
            "The gym opens at half six on weekdays and eight on weekends, closed on holidays.",
        ),
        (
            "When the gym opens",
            "Weekdays the gym opens at half six, weekends at eight; holidays it is closed.",
        ),
    ] {
        write(
            nb,
            &format!("{name}.md"),
            &format!("# {name}\n\n{body}\n"),
            10,
        );
    }
}

#[test]
fn long_structural_note_gets_a_diagram_proposal() {
    let dir = tempfile::tempdir().unwrap();
    let nb = Notebook::open(dir.path()).unwrap();
    let text = structural("Request path", 3);
    write(&nb, "work/Request path.md", &text, 10);
    let (report, asked) = counted(&nb, &DreamSettings::default(), &model);
    assert_eq!(count(&asked, "draw"), 1, "{asked:?}");
    let diagrams: Vec<&Proposal> = report
        .proposals
        .iter()
        .filter(|p| p.kind == Kind::Diagram)
        .collect();
    assert_eq!(diagrams.len(), 1, "{:?}", report.model_notes);
    let proposal = diagrams[0];
    assert_eq!(proposal.title, "Diagram for \"Request path\"");
    assert_eq!(
        proposal.reason,
        "The note follows a request through five services."
    );
    assert_eq!(proposal.notes, ["work/Request path.md"]);
    assert_eq!(proposal.notes_removed, 0);
    assert!(proposal.bytes_freed < -(REQUEST_DIAGRAM.len() as i64));
    let preview = proposal.preview.as_ref().unwrap();
    assert!(
        preview.contains("![How a request reaches the database](attachments/Request%20path.fdraw)"),
        "{preview}"
    );
    assert_eq!(
        proposal.sources,
        [(
            "work/Request path.md".to_owned(),
            fastmind_core::text::fnv(text.as_bytes())
        )]
    );
    let Plan::Diagram { note, diagram } = &proposal.plan else {
        panic!("{:?}", proposal.plan)
    };
    assert_eq!(note, "work/Request path.md");
    assert_eq!(diagram.file, "attachments/Request path.fdraw");
    assert_eq!(diagram.text, REQUEST_DIAGRAM);
}

#[test]
fn short_list_or_imaged_notes_are_not_asked() {
    let dir = tempfile::tempdir().unwrap();
    let nb = Notebook::open(dir.path()).unwrap();
    write(&nb, "work/Short path.md", &structural("Short path", 1), 10);
    write(&nb, "geo/Capitals.md", &fact_list(), 10);
    let imaged =
        structural("Request path", 3).replacen("## Stage 1", "![x](a.png)\n\n## Stage 1", 1);
    write(&nb, "work/Request path.md", &imaged, 10);
    let (_, asked) = counted(&nb, &DreamSettings::default(), &model);
    assert_eq!(count(&asked, "draw"), 0, "{asked:?}");
}

#[test]
fn not_worth_it_is_remembered() {
    let dir = tempfile::tempdir().unwrap();
    let nb = Notebook::open(dir.path()).unwrap();
    write(&nb, "work/Order flow.md", &structural("Order flow", 3), 10);
    let (report, asked) = counted(&nb, &DreamSettings::default(), &model);
    assert_eq!(count(&asked, "draw"), 1, "{asked:?}");
    assert!(report.proposals.iter().all(|p| p.kind != Kind::Diagram));
    let (again, asked) = counted(&nb, &DreamSettings::default(), &model);
    assert_eq!(count(&asked, "draw"), 0, "cached: {asked:?}");
    assert!(again.proposals.iter().all(|p| p.kind != Kind::Diagram));
}

#[test]
fn diagram_notes_zero_asks_nothing_and_reserves_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let nb = Notebook::open(dir.path()).unwrap();
    three_merge_groups(&nb);
    write(
        &nb,
        "work/Request path.md",
        &structural("Request path", 3),
        10,
    );
    let settings = DreamSettings {
        diagram_notes: 0,
        model_requests: 3,
        ..DreamSettings::default()
    };
    let (_, asked) = counted(&nb, &settings, &model);
    assert_eq!(count(&asked, "merge"), 3, "{asked:?}");
    assert_eq!(count(&asked, "draw"), 0, "{asked:?}");
}

#[test]
fn reserve_keeps_three_for_diagrams() {
    let dir = tempfile::tempdir().unwrap();
    let nb = Notebook::open(dir.path()).unwrap();
    three_merge_groups(&nb);
    write(
        &nb,
        "work/Request path.md",
        &structural("Request path", 3),
        10,
    );
    let settings = DreamSettings {
        model_requests: 4,
        ..DreamSettings::default()
    };
    let (report, asked) = counted(&nb, &settings, &model);
    assert_eq!(count(&asked, "merge"), 1, "{asked:?}");
    assert_eq!(count(&asked, "draw"), 1, "{asked:?}");
    assert!(report.proposals.iter().any(|p| p.kind == Kind::Diagram));
}

/// The broken diagram the topic answer carries.
const BROKEN: &str = "feed: node \"Feed\" at 40,40\nfeed -> nowhere\n";

/// `model`, but the topic's diagram doesn't draw, and a fix answers `fixed`.
fn broken_topic<'a>(
    fixed: &'static str,
    fixes: &'a Mutex<usize>,
) -> impl Fn(&str, &str, &Value) -> Result<String> + Sync + 'a {
    move |system: &str, user: &str, schema: &Value| {
        if user.contains("<errors>") {
            assert!(user.contains("Line 2: "), "the errors are given: {user}");
            assert!(user.contains("Fix only these errors; change nothing else."));
            *fixes.lock().unwrap() += 1;
            return Ok(json!({ "diagram": fixed }).to_string());
        }
        let mut answer: Value = serde_json::from_str(&model(system, user, schema)?).unwrap();
        if answer.get("members").is_some() {
            answer["diagram"] = json!(BROKEN);
        }
        Ok(answer.to_string())
    }
}

#[test]
fn broken_diagram_fixed_in_final_pass() {
    let dir = tempfile::tempdir().unwrap();
    let nb = Notebook::open(dir.path()).unwrap();
    starter_and_wifi(&nb);
    let fixes = Mutex::new(0);
    let answer = broken_topic(STARTER_DIAGRAM, &fixes);
    let report = answers(&answer, &nb);
    assert_eq!(*fixes.lock().unwrap(), 1);
    let topic = report
        .proposals
        .iter()
        .find(|p| p.kind == Kind::Topic)
        .expect("the topic is proposed");
    let Plan::Topic {
        diagram: Some(d), ..
    } = &topic.plan
    else {
        panic!("the fixed diagram is kept: {:?}", report.model_notes)
    };
    assert_eq!(d.file, "attachments/Sourdough starter.fdraw");
    assert_eq!(d.caption, "How the starter is kept");
    assert!(
        d.text.starts_with("feed: node \"Feed daily\""),
        "{}",
        d.text
    );
    let preview = topic.preview.as_ref().unwrap();
    assert!(
        preview.contains("![How the starter is kept](attachments/Sourdough%20starter.fdraw)"),
        "{preview}"
    );
    assert!(
        !report.model_notes.iter().any(|n| n.contains("dropped")),
        "{:?}",
        report.model_notes
    );
}

#[test]
fn broken_diagram_still_broken_is_dropped() {
    let dir = tempfile::tempdir().unwrap();
    let nb = Notebook::open(dir.path()).unwrap();
    starter_and_wifi(&nb);
    let fixes = Mutex::new(0);
    let answer = broken_topic(BROKEN, &fixes);
    let report = answers(&answer, &nb);
    assert_eq!(*fixes.lock().unwrap(), 1);
    let topic = report
        .proposals
        .iter()
        .find(|p| p.kind == Kind::Topic)
        .expect("the topic is still proposed");
    assert!(matches!(&topic.plan, Plan::Topic { diagram: None, .. }));
    assert!(!topic.preview.as_ref().unwrap().contains(".fdraw"));
    assert!(
        report
            .model_notes
            .iter()
            .any(|n| n.starts_with("1 diagram dropped: Line")),
        "{:?}",
        report.model_notes
    );
}

#[test]
fn edited_note_makes_diagram_outdated() {
    let dir = tempfile::tempdir().unwrap();
    let nb = Notebook::open(dir.path()).unwrap();
    write(
        &nb,
        "work/Request path.md",
        &structural("Request path", 3),
        10,
    );
    let report = answers(&model, &nb);
    let id = report
        .proposals
        .iter()
        .find(|p| p.kind == Kind::Diagram)
        .expect("the diagram is proposed")
        .id
        .clone();
    write(
        &nb,
        "work/Request path.md",
        "# Request path\n\nRewritten.\n",
        0,
    );
    let outcome = dream::apply(&nb, &id, Local::now()).unwrap();
    assert_eq!(outcome.status, Status::Outdated);
    assert!(!nb.root.join("work/attachments/Request path.fdraw").exists());
}

#[test]
fn two_merges_from_one_question_keep_their_own_fixes() {
    let dir = tempfile::tempdir().unwrap();
    let nb = Notebook::open(dir.path()).unwrap();
    for name in ["Wifi one", "Wifi two", "Wifi three", "Wifi four"] {
        write(
            &nb,
            &format!("{name}.md"),
            &format!(
                "# {name}\n\nThe conference wifi network is DevConf-Guest, password orange-falcon-42.\n"
            ),
            10,
        );
    }
    let fixes = Mutex::new(0);
    let answer = |system: &str, user: &str, _: &Value| -> Result<String> {
        if user.contains("<errors>") {
            *fixes.lock().unwrap() += 1;
            let which = if user.contains("first: node") {
                "first"
            } else {
                "second"
            };
            return Ok(json!({
                "diagram": format!("{which}: node \"{which} fixed\" at 40,40\nend: node \"End\" at 240,40\n{which} -> end")
            })
            .to_string());
        }
        if !system.contains("# Merge duplicate notes") {
            return Err(Error::msg("unexpected question"));
        }
        let ids: Vec<Option<String>> = ["Wifi one", "Wifi two", "Wifi three", "Wifi four"]
            .iter()
            .map(|t| id_of(user, t))
            .collect();
        let [Some(a), Some(b), Some(c), Some(d)] = ids.as_slice() else {
            return Ok(json!({"groups": []}).to_string());
        };
        let group = |keep: &str, absorb: &str, which: &str| {
            json!({"keep": keep, "absorb": [absorb], "reason": "Same wifi.",
                   "text": "# Wifi\n\nDevConf-Guest, orange-falcon-42.",
                   "diagram": format!("{which}: node \"{which}\" at 40,40\n{which} -> nowhere"),
                   "diagram_caption": which})
        };
        Ok(json!({"groups": [group(a, b, "first"), group(c, d, "second")]}).to_string())
    };
    let check = |report: &Report| {
        let merges: Vec<&Proposal> = report
            .proposals
            .iter()
            .filter(|p| p.kind == Kind::Merge)
            .collect();
        assert_eq!(merges.len(), 2, "{:?}", report.model_notes);
        for merge in merges {
            let Plan::Merge {
                diagram: Some(d), ..
            } = &merge.plan
            else {
                panic!("each merge keeps a diagram: {:?}", report.model_notes)
            };
            assert!(
                d.text
                    .starts_with(&format!("{}: node \"{} fixed\"", d.caption, d.caption)),
                "{} got {}",
                d.caption,
                d.text
            );
        }
    };
    let report = answers(&answer, &nb);
    assert_eq!(*fixes.lock().unwrap(), 2);
    check(&report);
    let again = answers(&answer, &nb);
    assert_eq!(*fixes.lock().unwrap(), 2, "the fixes are cached");
    check(&again);
}
