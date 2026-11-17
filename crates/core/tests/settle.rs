//! Dream settles new memories: captures still in the inbox move to where they belong overnight.

use chrono::{Duration, Local};
use fastmind_core::Notebook;
use fastmind_core::config::DreamSettings;
use fastmind_core::dream::{self, Kind, Places, Plan};
use fastmind_core::journal;
use std::fs;
use std::time::SystemTime;

fn write(nb: &Notebook, rel: &str, text: &str, hours_ago: u64) {
    let path = nb.root.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, text).unwrap();
    let at = SystemTime::now() - std::time::Duration::from_secs(hours_ago * 3600);
    fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(at)
        .unwrap();
}

fn inbox(title: &str, extra: &str, body: &str) -> String {
    format!("---\ntype: capture\nstatus: inbox\n{extra}---\n# {title}\n\n{body}\n")
}

#[test]
fn new_memories_settle_where_they_belong_and_the_rest_wait() {
    let dir = tempfile::tempdir().unwrap();
    let nb = Notebook::open(dir.path()).unwrap();
    let old = 24 * 30;
    write(
        &nb,
        "work/Work.md",
        "---\ntype: project\n---\n# Work\n\n## Notes\n\n- [[Release checklist]]\n",
        old,
    );
    write(
        &nb,
        "work/Release checklist.md",
        "# Release checklist\n\nTag, build, sign.\n",
        old,
    );
    write(
        &nb,
        "work/db/Postgres vacuum.md",
        "# Postgres vacuum\n\nAutovacuum scale factor.\n",
        old,
    );
    for (name, body) in [
        (
            "Starter feeding",
            "Feed the sourdough starter flour and water every morning, keep the jar warm.",
        ),
        (
            "Bake day",
            "Sourdough bake day: levain at night, bulk ferment, shape, cold proof, bake hot.",
        ),
        (
            "Sour starter",
            "The sourdough starter smells too sour: feed it more often, keep it warmer.",
        ),
    ] {
        write(
            &nb,
            &format!("kitchen/{name}.md"),
            &format!("# {name}\n\n{body}\n"),
            old,
        );
    }
    // The model suggested a home note, and a folder.
    write(
        &nb,
        "Fresh memories/Invoices batch.md",
        &inbox(
            "Invoices batch",
            "suggested_home: \"[[Work]]\"\nsuggested_because: Ops work.\n",
            "Batch moved to 02:00.",
        ),
        10,
    );
    write(
        &nb,
        "Fresh memories/Vacuum tuning.md",
        &inbox(
            "Vacuum tuning",
            "suggested_home: work/db/\n",
            "Scale factor 0.01 for orders.",
        ),
        10,
    );
    // No suggestion: its neighbours decide.
    write(
        &nb,
        "Fresh memories/Rye in the starter.md",
        &inbox(
            "Rye in the starter",
            "",
            "Add rye flour to the sourdough starter feeding so the starter rises faster in the morning.",
        ),
        10,
    );
    // Too new, and nowhere obvious: both wait.
    write(
        &nb,
        "Fresh memories/Fresh.md",
        &inbox("Fresh", "suggested_home: \"[[Work]]\"\n", "Just captured."),
        1,
    );
    write(
        &nb,
        "Fresh memories/Odd one.md",
        &inbox("Odd one", "", "Zebra quantum lighthouse."),
        10,
    );

    let report = dream::run_in(
        &nb,
        &DreamSettings::default(),
        &Places::default(),
        None,
        Local::now(),
    )
    .unwrap();
    let settles: Vec<_> = report
        .proposals
        .iter()
        .filter(|p| p.kind == Kind::Settle)
        .collect();
    let find = |path: &str| {
        settles
            .iter()
            .find(|p| p.notes.contains(&path.to_owned()))
            .copied()
    };
    let to_work = find("Fresh memories/Invoices batch.md").expect("the model's home note is used");
    assert!(matches!(&to_work.plan, Plan::Settle { home: Some(h), .. } if h == "work/Work.md"));
    assert!(to_work.reason.contains("Ops work."), "{}", to_work.reason);
    let to_db = find("Fresh memories/Vacuum tuning.md").expect("the model's folder is used");
    assert!(matches!(&to_db.plan, Plan::Settle { folder: Some(f), .. } if f == "work/db"));
    let to_kitchen =
        find("Fresh memories/Rye in the starter.md").expect("neighbours decide without a model");
    assert!(
        matches!(&to_kitchen.plan, Plan::Settle { folder: Some(f), .. } if f == "kitchen"),
        "{:?}",
        to_kitchen.plan
    );
    assert!(
        find("Fresh memories/Fresh.md").is_none(),
        "captured an hour ago: not yet"
    );
    assert!(
        find("Fresh memories/Odd one.md").is_none(),
        "no clear home: it stays new"
    );
    assert!(
        report.summary.contains("3 fresh memories to settle"),
        "{}",
        report.summary
    );

    // Settling is on by default, so Dream does it without asking; History undoes it.
    assert!(
        DreamSettings::default()
            .auto_apply
            .contains(&"settle".to_owned())
    );
    let applied =
        dream::apply_all(&nb, &[Kind::Settle], Local::now() + Duration::seconds(1)).unwrap();
    assert_eq!(applied.len(), 3);
    assert!(nb.exists("work/Invoices batch.md"));
    assert!(nb.exists("work/db/Vacuum tuning.md"));
    assert!(nb.exists("kitchen/Rye in the starter.md"));
    assert!(nb.exists("Fresh memories/Fresh.md") && nb.exists("Fresh memories/Odd one.md"));
    let work = fs::read_to_string(nb.root.join("work/Work.md")).unwrap();
    assert!(
        work.contains("- [[Invoices batch]]"),
        "the home note links to it: {work}"
    );
    let settled = fs::read_to_string(nb.root.join("work/db/Vacuum tuning.md")).unwrap();
    assert!(
        !settled.contains("status: inbox"),
        "settled notes leave the inbox: {settled}"
    );

    for entry in journal::history(&nb).into_iter().filter(|e| !e.undone) {
        journal::undo(&nb, &entry.id).unwrap();
    }
    assert!(
        nb.exists("Fresh memories/Invoices batch.md")
            && nb.exists("Fresh memories/Vacuum tuning.md")
    );
    assert!(
        !fs::read_to_string(nb.root.join("work/Work.md"))
            .unwrap()
            .contains("Invoices batch")
    );
}

#[test]
fn settled_note_keeps_its_diagram() {
    let dir = tempfile::tempdir().unwrap();
    let nb = Notebook::open(dir.path()).unwrap();
    write(
        &nb,
        "Fresh memories/Flow.md",
        "---\nstatus: inbox\n---\n# Flow\n\n![d](attachments/x%20y.fdraw)\n",
        1,
    );
    write(&nb, "Fresh memories/attachments/x y.fdraw", "box A\n", 1);
    let target =
        fastmind_core::filing::file(&nb, "Fresh memories/Flow.md", Some("work"), None).unwrap();
    assert_eq!(target, "work/Flow.md");
    // The link resolves the way the reader resolves it: beside the note, in attachments/.
    assert!(
        nb.exists("work/attachments/x y.fdraw"),
        "the diagram moved with its note"
    );
    assert!(
        fs::read_to_string(nb.root.join(&target))
            .unwrap()
            .contains("attachments/x%20y.fdraw")
    );
    assert!(
        !nb.exists("Fresh memories/attachments/x y.fdraw"),
        "and left nothing behind"
    );
    // Undo puts everything back.
    let entry = journal::history(&nb).into_iter().next().unwrap();
    journal::undo(&nb, &entry.id).unwrap();
    assert!(nb.exists("Fresh memories/attachments/x y.fdraw"));
    assert!(!nb.exists("work/attachments/x y.fdraw"));
}

fn inbox_note(nb: &Notebook, name: &str, body: &str) {
    write(
        nb,
        &format!("Fresh memories/{name}.md"),
        &format!("---\nstatus: inbox\n---\n# {name}\n\n{body}\n"),
        1,
    );
}

fn read(nb: &Notebook, rel: &str) -> String {
    fs::read_to_string(nb.root.join(rel)).unwrap()
}

#[test]
fn shared_attachment_is_copied_and_the_original_kept() {
    let dir = tempfile::tempdir().unwrap();
    let nb = Notebook::open(dir.path()).unwrap();
    inbox_note(&nb, "A", "![p](attachments/logo.png)");
    inbox_note(&nb, "B", "![p](attachments/logo.png)");
    inbox_note(&nb, "C", "![[logo.png]]");
    write(&nb, "Fresh memories/attachments/logo.png", "PNG", 1);
    fastmind_core::filing::file(&nb, "Fresh memories/A.md", Some("work"), None).unwrap();
    assert_eq!(read(&nb, "work/attachments/logo.png"), "PNG");
    assert!(
        nb.exists("Fresh memories/attachments/logo.png"),
        "B still shows it"
    );
    assert!(read(&nb, "work/A.md").contains("![p](attachments/logo.png)"));
    // C shows it by an embed, which also resolves beside the note.
    fastmind_core::filing::file(&nb, "Fresh memories/B.md", Some("work"), None).unwrap();
    assert!(
        nb.exists("Fresh memories/attachments/logo.png"),
        "the embed in C still shows it"
    );
    fastmind_core::filing::file(&nb, "Fresh memories/C.md", Some("kitchen"), None).unwrap();
    assert!(nb.exists("kitchen/attachments/logo.png"));
    assert!(
        !nb.exists("Fresh memories/attachments/logo.png"),
        "nobody shows it there now"
    );
}

#[test]
fn embed_shared_with_a_note_that_stays_keeps_the_original() {
    let dir = tempfile::tempdir().unwrap();
    let nb = Notebook::open(dir.path()).unwrap();
    inbox_note(&nb, "A", "![p](attachments/logo.png)");
    inbox_note(&nb, "Stay", "![[logo.png]]");
    write(&nb, "Fresh memories/attachments/logo.png", "PNG", 1);
    fastmind_core::filing::file(&nb, "Fresh memories/A.md", Some("work"), None).unwrap();
    assert!(nb.exists("Fresh memories/attachments/logo.png"));
    assert!(nb.exists("work/attachments/logo.png"));
}

#[test]
fn name_collision_in_the_destination_gets_a_free_name() {
    let dir = tempfile::tempdir().unwrap();
    let nb = Notebook::open(dir.path()).unwrap();
    inbox_note(&nb, "A", "![p](attachments/logo.png) ![[logo.png]]");
    write(&nb, "Fresh memories/attachments/logo.png", "NEW", 1);
    write(&nb, "work/attachments/logo.png", "OLD", 1);
    fastmind_core::filing::file(&nb, "Fresh memories/A.md", Some("work"), None).unwrap();
    assert_eq!(read(&nb, "work/attachments/logo.png"), "OLD");
    assert_eq!(read(&nb, "work/attachments/logo 2.png"), "NEW");
    assert!(read(&nb, "work/A.md").contains("![p](attachments/logo%202.png)"));
}

#[test]
fn attachments_follow_to_and_from_the_notebook_root() {
    let dir = tempfile::tempdir().unwrap();
    let nb = Notebook::open(dir.path()).unwrap();
    inbox_note(&nb, "A", "![d](attachments/Redis%20%28x%29.fdraw)");
    write(
        &nb,
        "Fresh memories/attachments/Redis (x).fdraw",
        "box A\n",
        1,
    );
    fastmind_core::filing::file(&nb, "Fresh memories/A.md", Some(""), None).unwrap();
    assert!(nb.exists("attachments/Redis (x).fdraw"));
    assert!(read(&nb, "A.md").contains("![d](attachments/Redis%20%28x%29.fdraw)"));
    fastmind_core::filing::file(&nb, "A.md", Some("work"), None).unwrap();
    assert!(nb.exists("work/attachments/Redis (x).fdraw"));
    assert!(!nb.exists("attachments/Redis (x).fdraw"));
}

#[test]
fn only_the_notes_own_attachments_are_carried() {
    let dir = tempfile::tempdir().unwrap();
    let nb = Notebook::open(dir.path()).unwrap();
    write(
        &nb,
        "Fresh memories/sub/N.md",
        "---\nstatus: inbox\n---\n# N\n\n![a](../shared/attachments/l.png) ![b](myattachments/m.png)\n",
        1,
    );
    write(&nb, "Fresh memories/shared/attachments/l.png", "L", 1);
    write(&nb, "Fresh memories/sub/myattachments/m.png", "M", 1);
    fastmind_core::filing::file(&nb, "Fresh memories/sub/N.md", Some("work"), None).unwrap();
    assert!(nb.exists("Fresh memories/shared/attachments/l.png"));
    assert!(nb.exists("Fresh memories/sub/myattachments/m.png"));
    assert!(!nb.exists("work/attachments/l.png") && !nb.exists("work/attachments/m.png"));
}

#[test]
fn settling_notes_that_share_an_attachment_in_one_change_keeps_it_valid_and_undoes() {
    let dir = tempfile::tempdir().unwrap();
    let nb = Notebook::open(dir.path()).unwrap();
    let old = 24 * 30;
    write(
        &nb,
        "kitchen/Bread.md",
        "# Bread\n\nSourdough starter flour water.\n",
        old,
    );
    for name in ["Starter feeding", "Starter rye"] {
        write(
            &nb,
            &format!("Fresh memories/{name}.md"),
            &format!(
                "---\nstatus: inbox\n---\n# {name}\n\nSourdough starter flour water, feed daily.\n\n![s](attachments/jar.png)\n"
            ),
            24 * 2,
        );
    }
    write(&nb, "Fresh memories/attachments/jar.png", "PNG", 24 * 2);
    let plan = dream::Plan::Settle {
        folder: Some("kitchen".into()),
        home: None,
        paths: vec![
            "Fresh memories/Starter feeding.md".into(),
            "Fresh memories/Starter rye.md".into(),
        ],
    };
    let proposal = dream::Proposal {
        id: "s1".into(),
        kind: Kind::Settle,
        title: "Settle".into(),
        reason: "t".into(),
        notes: vec![],
        notes_removed: 0,
        bytes_freed: 0,
        disk_freed: 0,
        preview: None,
        status: dream::Status::Pending,
        entry: None,
        error: None,
        plan,
        sources: vec![],
    };
    let report = dream::Report {
        id: "r".into(),
        ran_at: Local::now(),
        took_ms: 0,
        before: Default::default(),
        proposals: vec![proposal],
        model: None,
        model_notes: vec![],
        summary: String::new(),
    };
    dream::save_report(&nb, &report).unwrap();
    let done = dream::apply(&nb, "s1", Local::now()).unwrap();
    assert_eq!(done.status, dream::Status::Applied, "{:?}", done.error);
    for name in ["Starter feeding", "Starter rye"] {
        assert!(read(&nb, &format!("kitchen/{name}.md")).contains("![s](attachments/"));
    }
    let files: Vec<String> = fs::read_dir(nb.root.join("kitchen/attachments"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(files.contains(&"jar.png".to_owned()), "{files:?}");
    for name in ["Starter feeding", "Starter rye"] {
        let text = read(&nb, &format!("kitchen/{name}.md"));
        let link = text
            .split("](")
            .nth(1)
            .unwrap()
            .split(')')
            .next()
            .unwrap()
            .to_owned();
        let target =
            fastmind_core::links::relative_target_path(&format!("kitchen/{name}.md"), &link);
        assert!(nb.exists(&target), "{link} -> {target}");
    }
    dream::undo(&nb, "s1").unwrap();
    assert!(nb.exists("Fresh memories/attachments/jar.png"));
    assert!(!nb.exists("kitchen/attachments/jar.png"));
    assert!(
        nb.exists("Fresh memories/Starter feeding.md")
            && nb.exists("Fresh memories/Starter rye.md")
    );
}
