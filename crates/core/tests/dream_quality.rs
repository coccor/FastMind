//! Dream must not mistake a notebook's structure for duplication. Real notebooks are full of
//! notes made from the same template (meetings, daily notes, recipes) and short notes that share
//! everyday words. None of those should be merged or gathered.

use chrono::{Duration, Local};
use fastmind_core::Notebook;
use fastmind_core::config::DreamSettings;
use fastmind_core::dream::{self, Kind};
use std::fs;

fn write(root: &std::path::Path, rel: &str, text: &str) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn date(days_ago: i64) -> String {
    (Local::now() - Duration::days(days_ago))
        .format("%Y-%m-%d")
        .to_string()
}

const PEOPLE: [&str; 6] = ["Ana", "Bogdan", "Chloe", "Dmitri", "Elena", "Farid"];
const SUBJECTS: [(&str, &str, &str); 8] = [
    (
        "Payments roadmap",
        "Agreed to ship refunds v2 before the holiday freeze.",
        "Elena drafts the rollout plan",
    ),
    (
        "Hiring sync",
        "Two backend candidates move to the final round; the frontend role stays open.",
        "Bogdan books the panel",
    ),
    (
        "Incident review: ledger lag",
        "The ledger consumer fell behind because of a slow index on entries.",
        "Dmitri adds the index and an alert",
    ),
    (
        "Design review: search page",
        "Filters move to a side panel; results show snippets with highlights.",
        "Chloe updates the mockups",
    ),
    (
        "Quarterly planning",
        "Cloud spend must drop by ten percent; observability work is funded.",
        "Farid writes the budget memo",
    ),
    (
        "Onboarding retro",
        "New hires wanted a starter task list and a buddy for their first week.",
        "Ana writes the onboarding checklist",
    ),
    (
        "Security sync",
        "Rotate the deploy keys and enable secret scanning on every repository.",
        "Dmitri schedules the rotation",
    ),
    (
        "API versioning",
        "Breaking changes go behind a version header; v1 is supported for a year.",
        "Elena documents the policy",
    ),
];

fn meeting(subject: &str, decision: &str, action: &str, people: &[&str], day: i64) -> String {
    format!(
        "---\ntype: meeting\ndate: {}\ntags: [meeting]\n---\n# {subject}\n\n## Attendees\n\n{}\n\n## Agenda\n\n- Status updates\n- Open questions\n- Next steps\n\n## Notes\n\n{decision}\n\n## Action items\n\n- [ ] {action}\n",
        date(day),
        people
            .iter()
            .map(|p| format!("- {p}"))
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

fn daily(day: i64, line: &str) -> String {
    format!(
        "---\ntype: daily\ndate: {d}\n---\n# {d}\n\n## Today\n\n- {line}\n\n## Notes\n\n## Tomorrow\n\n- Review the inbox\n",
        d = date(day)
    )
}

#[test]
fn templates_and_everyday_words_are_not_duplication() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    for (i, (subject, decision, action)) in SUBJECTS.iter().enumerate() {
        let people = [PEOPLE[i % 6], PEOPLE[(i + 2) % 6], PEOPLE[(i + 4) % 6]];
        write(
            root,
            &format!("meetings/{subject}.md"),
            &meeting(subject, decision, action, &people, 20 + i as i64),
        );
    }
    let lines = [
        "Fixed the flaky checkout test",
        "Paired with Ana on the ledger index",
        "Wrote the refunds rollout plan",
        "Interviewed a backend candidate",
        "Read about Postgres partitioning",
        "Cleaned up the deploy scripts",
        "Planned the next sprint",
    ];
    for (i, line) in lines.iter().enumerate() {
        let day = 10 + i as i64;
        write(root, &format!("daily/{}.md", date(day)), &daily(day, line));
    }
    // Short notes on different subjects that share common words.
    let shorts = [
        (
            "ideas/Use a queue for emails.md",
            "# Use a queue for emails\n\nSending emails inline slows the request; put them on a queue and send them from a worker.\n",
        ),
        (
            "ideas/Queue for webhooks.md",
            "# Retry webhooks\n\nWebhook deliveries should retry with backoff and give up after a day.\n",
        ),
        (
            "ideas/Team lunch.md",
            "# Team lunch\n\nBook the Thai place for Friday; ask about vegetarian options.\n",
        ),
        (
            "ideas/Book recommendations.md",
            "# Book recommendations\n\nThinking in Systems, and The Manager's Path for new leads.\n",
        ),
        (
            "ideas/Request timeouts.md",
            "# Request timeouts\n\nEvery outgoing request needs a timeout; the default of none hangs workers.\n",
        ),
    ];
    for (path, text) in shorts {
        write(
            root,
            path,
            &format!("---\ncreated: {}\n---\n{text}", date(15)),
        );
    }
    write(
        root,
        "templates/Meeting.md",
        "---\ntype: meeting\ndate: \ntags: [meeting]\n---\n# Title\n\n## Attendees\n\n## Agenda\n\n## Notes\n\n## Action items\n",
    );

    let nb = Notebook::open(root).unwrap();
    let report = dream::run(&nb, &DreamSettings::default(), None, Local::now()).unwrap();
    let bad: Vec<String> = report
        .proposals
        .iter()
        .filter(|p| matches!(p.kind, Kind::Merge | Kind::Topic | Kind::Archive))
        .map(|p| format!("{:?}: {} — {}", p.kind, p.title, p.reason))
        .collect();
    assert!(
        bad.is_empty(),
        "Dream mistook structure for mess:\n{}",
        bad.join("\n")
    );
}

#[test]
fn old_daily_notes_and_templates_are_never_archived() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(
        root,
        &format!("daily/{}.md", date(400)),
        &daily(400, "Shipped the first version"),
    );
    write(root, "templates/Meeting.md", "# Title\n\n## Attendees\n");
    // Make both look old on disk too.
    for rel in [
        format!("daily/{}.md", date(400)),
        "templates/Meeting.md".to_owned(),
    ] {
        let file = fs::OpenOptions::new()
            .write(true)
            .open(root.join(rel))
            .unwrap();
        file.set_modified((Local::now() - Duration::days(400)).into())
            .unwrap();
    }
    let nb = Notebook::open(root).unwrap();
    let report = dream::run(&nb, &DreamSettings::default(), None, Local::now()).unwrap();
    assert!(
        report.proposals.is_empty(),
        "{:#?}",
        report
            .proposals
            .iter()
            .map(|p| &p.title)
            .collect::<Vec<_>>()
    );
}
