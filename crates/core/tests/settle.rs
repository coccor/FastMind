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
