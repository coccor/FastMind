//! Dream: while you sleep, FastMind compresses the notebook. It finds notes that say the same
//! thing and merges them, gathers scattered fragments into topic notes, retires notes nothing
//! uses any more, mends broken links and packs old archives into zip files.
//!
//! Dream proposes; you decide. Every proposal says what it will do and what it frees, and every
//! accepted one can be undone from the journal. Only the kinds you allow (packing, by default)
//! are applied on their own.

pub mod analyze;
pub mod compose;

use crate::archive::{self, Index};
use crate::config::DreamSettings;
use crate::journal::{self, Transaction};
use crate::links::{self, Resolver};
use crate::model::{self, Model};
use crate::notebook::{self, Note, Notebook, STATE_DIR};
use crate::usage::Usage;
use crate::{Error, Result, fsx, text};
use chrono::{DateTime, Duration, Local};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Merge,
    Topic,
    Archive,
    FixLink,
    Pack,
}

impl Kind {
    pub fn parse(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().replace('-', "_").as_str() {
            "merge" => Some(Kind::Merge),
            "topic" => Some(Kind::Topic),
            "archive" => Some(Kind::Archive),
            "fix_link" | "link" => Some(Kind::FixLink),
            "pack" => Some(Kind::Pack),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Plan {
    Merge {
        keep: String,
        absorb: Vec<String>,
    },
    Topic {
        path: String,
        title: String,
        summary: String,
        fragments: Vec<String>,
    },
    Archive {
        path: String,
    },
    FixLink {
        note: String,
        from: String,
        to: String,
    },
    Pack {
        items: Vec<String>,
        journal: Vec<String>,
    },
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Pending,
    Applied,
    Rejected,
    /// The notes changed or went away since Dream looked; run Dream again.
    Outdated,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Proposal {
    pub id: String,
    pub kind: Kind,
    pub title: String,
    pub reason: String,
    /// Notes the proposal touches, by path.
    pub notes: Vec<String>,
    /// How many notes leave the notebook's folders.
    pub notes_removed: usize,
    /// Bytes the notebook's folders shrink by (negative when a note grows).
    pub bytes_freed: i64,
    /// Bytes freed on disk overall (packing).
    #[serde(default)]
    pub disk_freed: i64,
    /// The note as it would read, for merges and topics.
    pub preview: Option<String>,
    pub status: Status,
    /// The journal entry that applied it, to undo.
    #[serde(default)]
    pub entry: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    pub plan: Plan,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Totals {
    pub notes: usize,
    pub bytes: u64,
    pub attachments: usize,
    pub attachment_bytes: u64,
    pub archive_bytes: u64,
    pub journal_bytes: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Report {
    pub id: String,
    pub ran_at: DateTime<Local>,
    pub took_ms: u128,
    pub before: Totals,
    pub proposals: Vec<Proposal>,
    pub model: Option<String>,
    #[serde(default)]
    pub model_notes: Vec<String>,
    pub summary: String,
}

impl Report {
    pub fn pending(&self) -> impl Iterator<Item = &Proposal> {
        self.proposals
            .iter()
            .filter(|p| p.status == Status::Pending)
    }
}

fn dream_dir(nb: &Notebook) -> std::path::PathBuf {
    nb.root.join(STATE_DIR).join("dream")
}

pub fn latest(nb: &Notebook) -> Option<Report> {
    fs::read(dream_dir(nb).join("latest.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
}

fn save(nb: &Notebook, report: &Report) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(report)?;
    fsx::write_atomic(&dream_dir(nb).join("latest.json"), &bytes)?;
    let line = serde_json::json!({
        "id": report.id,
        "ran_at": report.ran_at,
        "summary": report.summary,
        "proposals": report.proposals.len(),
        "applied": report.proposals.iter().filter(|p| p.status == Status::Applied).count(),
    });
    let log = dream_dir(nb).join("log.jsonl");
    let mut existing = fs::read_to_string(&log).unwrap_or_default();
    // One line per run; a re-save of the same run replaces its line.
    existing = existing
        .lines()
        .filter(|l| !l.contains(&format!("\"id\":\"{}\"", report.id)))
        .map(|l| format!("{l}\n"))
        .collect();
    existing.push_str(&format!("{line}\n"));
    fsx::write_atomic(&log, existing.as_bytes())
}

pub fn totals(nb: &Notebook, notes: &[Note]) -> Totals {
    let attachments = nb.attachments();
    Totals {
        notes: notes.len(),
        bytes: notes.iter().map(|note| note.size).sum(),
        attachments: attachments.len(),
        attachment_bytes: attachments.iter().map(|(_, size)| size).sum(),
        archive_bytes: archive::disk_size(nb),
        journal_bytes: journal::size(nb),
    }
}

const DONE: [&str; 8] = [
    "done",
    "finished",
    "obsolete",
    "cancelled",
    "canceled",
    "archived",
    "superseded",
    "resolved",
];
const ATTACHMENT_EXT: [&str; 16] = [
    "png", "jpg", "jpeg", "gif", "webp", "bmp", "svg", "pdf", "docx", "xlsx", "pptx", "zip", "mp3",
    "mp4", "mov", "wav",
];

/// Looks at the whole notebook and writes proposals. Changes nothing in the notes.
pub fn run(
    nb: &Notebook,
    settings: &DreamSettings,
    model: Option<&Model>,
    now: DateTime<Local>,
) -> Result<Report> {
    let started = std::time::Instant::now();
    let notes = nb.notes();
    let before = totals(nb, &notes);
    let usage = Usage::load(nb);
    let inbound = notebook::backlinks(&notes);
    let features = analyze::features(&notes);
    let mut proposals: Vec<Proposal> = Vec::new();
    let mut claimed: HashSet<usize> = HashSet::new();
    let mut model_budget = settings.model_requests;
    let mut model_notes = Vec::new();

    let protected = |note: &Note| {
        note.is_home()
            || note.is_pinned()
            || matches!(
                note.stem().to_ascii_lowercase().as_str(),
                "readme" | "index" | "home"
            )
    };
    let recent = |note: &Note| now - note.touched < Duration::days(2);

    // 1. Duplicates: notes that say the same thing. The best-connected copy stays.
    let allowed: Vec<bool> = notes
        .iter()
        .map(|note| !protected(note) && !recent(note))
        .collect();
    let mut groups = analyze::UnionFind::new(notes.len());
    let mut best_similarity: HashMap<(usize, usize), f64> = HashMap::new();
    for (a, b) in analyze::candidate_pairs(&features, &allowed) {
        if let Some(similarity) =
            analyze::duplicates(&features[a], &features[b], settings.duplicate_similarity)
        {
            groups.union(a, b);
            best_similarity.insert((a, b), similarity);
        }
    }
    for group in groups.groups() {
        let keep = *group
            .iter()
            .max_by(|a, b| {
                let score = |i: &usize| {
                    let note = &notes[*i];
                    (
                        inbound.get(&note.path).map_or(0, Vec::len),
                        (note.status() != "inbox") as u8,
                        features[*i].words,
                        note.touched,
                    )
                };
                score(a).cmp(&score(b)).then(b.cmp(a))
            })
            .expect("a group has members");
        let absorb: Vec<usize> = group.iter().copied().filter(|i| *i != keep).collect();
        let similarity = group
            .iter()
            .flat_map(|a| group.iter().map(move |b| (*a, *b)))
            .filter_map(|pair| best_similarity.get(&pair))
            .fold(0.0f64, |acc, s| acc.max(*s));
        let absorbed: Vec<&Note> = absorb.iter().map(|i| &notes[*i]).collect();
        let merged = compose::merge(&notes[keep], &absorbed, now);
        let freed = absorbed.iter().map(|n| n.size as i64).sum::<i64>() + notes[keep].size as i64
            - merged.len() as i64;
        let names: Vec<String> = absorbed
            .iter()
            .map(|n| format!("\"{}\"", n.title))
            .collect();
        proposals.push(Proposal {
            id: String::new(),
            kind: Kind::Merge,
            title: format!("Merge {} into \"{}\"", names.join(", "), notes[keep].title),
            reason: format!(
                "{} say the same thing ({:.0}% alike). The copy with the most links stays; what the others add is kept under it, and their titles become aliases.",
                if group.len() == 2 { "Both notes".to_owned() } else { format!("These {} notes", group.len()) },
                similarity * 100.0
            ),
            notes: group.iter().map(|i| notes[*i].path.clone()).collect(),
            notes_removed: absorb.len(),
            bytes_freed: freed,
            disk_freed: 0,
            preview: Some(merged),
            status: Status::Pending,
            entry: None,
            error: None,
            plan: Plan::Merge {
                keep: notes[keep].path.clone(),
                absorb: absorbed.iter().map(|n| n.path.clone()).collect(),
            },
        });
        claimed.extend(group);
    }

    // 2. Topics: short notes circling one subject, gathered into one note.
    let fragments: Vec<usize> = (0..notes.len())
        .filter(|i| {
            !claimed.contains(i)
                && allowed[*i]
                && features[*i].words <= settings.fragment_words
                && features[*i].words >= 3
        })
        .collect();
    let tags: Vec<HashSet<String>> = notes
        .iter()
        .map(|n| {
            n.front
                .tags()
                .into_iter()
                .map(|t| t.to_lowercase())
                .collect()
        })
        .collect();
    let similarity = |a: usize, b: usize| {
        let shared_tag = !tags[a].is_disjoint(&tags[b]);
        analyze::cosine(&features[a].vector, &features[b].vector)
            + if shared_tag { 0.1 } else { 0.0 }
    };
    let clusters = analyze::topic_clusters(
        &fragments,
        &similarity,
        settings.topic_similarity,
        settings.topic_min_notes.max(2),
    );
    let mut planned_paths: HashSet<String> = HashSet::new();
    for members in clusters {
        let words = analyze::naming_words(&features, &members, 3);
        let group: Vec<&Note> = members.iter().map(|i| &notes[*i]).collect();
        let mut title = match analyze::naming_title(&group, &features, &members) {
            Some(phrase) => compose::title_from_words(&[phrase]),
            None => compose::title_from_words(&words),
        };
        let mut summary = compose::extract_summary(&group, &words, 2);
        if let (Some(model), true) = (model, model_budget > 0) {
            model_budget -= 1;
            match name_topic(model, &group) {
                Ok((better_title, better_summary)) => {
                    title = better_title;
                    summary = better_summary;
                }
                Err(error) => model_notes.push(format!("Topic title: {error}")),
            }
        }
        let folder = majority_folder(&group);
        let stem = fsx::safe_stem(&title);
        let name = fsx::free_name(&stem, "md", |candidate| {
            let rel = join(&folder, candidate);
            nb.exists(&rel) || planned_paths.contains(&rel)
        });
        let path = join(&folder, &name);
        planned_paths.insert(path.clone());
        let text = compose::topic(&title, &summary, &group, now);
        let freed = group.iter().map(|n| n.size as i64).sum::<i64>() - text.len() as i64;
        proposals.push(Proposal {
            id: String::new(),
            kind: Kind::Topic,
            title: format!("Gather {} short notes into \"{title}\"", group.len()),
            reason: format!(
                "{} short notes are about the same subject but live apart. One topic note keeps everything they say, in date order, and answers to their old titles.",
                group.len()
            ),
            notes: group.iter().map(|n| n.path.clone()).collect(),
            notes_removed: group.len() - 1,
            bytes_freed: freed,
            disk_freed: 0,
            preview: Some(text),
            status: Status::Pending,
            entry: None,
            error: None,
            plan: Plan::Topic {
                path,
                title,
                summary,
                fragments: group.iter().map(|n| n.path.clone()).collect(),
            },
        });
        claimed.extend(members);
    }

    // 3. Retire: notes finished long ago, never used, or empty.
    for (index, note) in notes.iter().enumerate() {
        if claimed.contains(&index) || protected(note) {
            continue;
        }
        if inbound.get(&note.path).is_some_and(|from| !from.is_empty()) {
            continue;
        }
        let last_use = usage
            .last_opened(&note.path)
            .map_or(note.touched, |opened| opened.max(note.touched));
        let idle = (now - last_use).num_days();
        let status = note.status();
        let reason = if features[index].words < 3 && note.links.is_empty() && idle >= 1 {
            Some("It's empty: no text worth keeping and no links.".to_owned())
        } else if DONE.contains(&status.as_str()) && idle >= settings.done_days {
            Some(format!(
                "Marked \"{status}\" and untouched for {idle} days, and no note links to it."
            ))
        } else if idle >= settings.stale_days {
            Some(format!(
                "Untouched for {idle} days, never opened from FastMind in that time, and no note links to it."
            ))
        } else {
            None
        };
        if let Some(reason) = reason {
            proposals.push(Proposal {
                id: String::new(),
                kind: Kind::Archive,
                title: format!("Archive \"{}\"", note.title),
                reason,
                notes: vec![note.path.clone()],
                notes_removed: 1,
                bytes_freed: note.size as i64,
                disk_freed: 0,
                preview: None,
                status: Status::Pending,
                entry: None,
                error: None,
                plan: Plan::Archive {
                    path: note.path.clone(),
                },
            });
        }
    }

    // Attachments nothing links to any more.
    let all_text: String = notes
        .iter()
        .map(|note| note.text.to_lowercase())
        .collect::<Vec<_>>()
        .join("\n");
    for (path, size) in nb.attachments() {
        let name = path.rsplit('/').next().unwrap_or(&path).to_owned();
        let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
        if !ATTACHMENT_EXT.contains(&ext.as_str()) {
            continue;
        }
        let lower = name.to_lowercase();
        if all_text.contains(&lower) || all_text.contains(&lower.replace(' ', "%20")) {
            continue;
        }
        let Ok(meta) = fs::metadata(nb.path(&path)?) else {
            continue;
        };
        let modified: DateTime<Local> = meta.modified().map(DateTime::from).unwrap_or(now);
        if (now - modified).num_days() < settings.done_days {
            continue;
        }
        proposals.push(Proposal {
            id: String::new(),
            kind: Kind::Archive,
            title: format!("Archive the unused file \"{name}\""),
            reason: format!("No note links to it any more ({}).", human_bytes(size)),
            notes: vec![path.clone()],
            notes_removed: 0,
            bytes_freed: size as i64,
            disk_freed: 0,
            preview: None,
            status: Status::Pending,
            entry: None,
            error: None,
            plan: Plan::Archive { path },
        });
    }

    // 4. Broken links that clearly meant an existing note.
    let resolver = Resolver::new(&notes);
    for note in &notes {
        let mut seen = HashSet::new();
        for target in &note.links {
            if !seen.insert(target.to_lowercase()) || resolver.resolve(target).is_some() {
                continue;
            }
            let last = target.rsplit('/').next().unwrap_or(target);
            if last.contains('.') && !last.to_ascii_lowercase().ends_with(".md") {
                continue;
            }
            let wanted = last.trim_end_matches(".md").to_lowercase();
            if wanted.chars().count() < 4 {
                continue;
            }
            let limit = if wanted.chars().count() >= 8 { 2 } else { 1 };
            let mut matches: Vec<&str> = resolver
                .names()
                .filter(|(name, _)| text::within_edits(name, &wanted, limit))
                .map(|(_, path)| *path)
                .collect();
            matches.sort();
            matches.dedup();
            if let [only] = matches.as_slice() {
                let to = notes
                    .iter()
                    .find(|n| n.path == *only)
                    .map(|n| n.stem().to_owned())
                    .unwrap_or_default();
                proposals.push(Proposal {
                    id: String::new(),
                    kind: Kind::FixLink,
                    title: format!("Point [[{target}]] in \"{}\" at [[{to}]]", note.title),
                    reason: format!(
                        "No note is called \"{target}\"; \"{to}\" is the only close match."
                    ),
                    notes: vec![note.path.clone(), (*only).to_owned()],
                    notes_removed: 0,
                    bytes_freed: 0,
                    disk_freed: 0,
                    preview: None,
                    status: Status::Pending,
                    entry: None,
                    error: None,
                    plan: Plan::FixLink {
                        note: note.path.clone(),
                        from: target.clone(),
                        to,
                    },
                });
            }
        }
    }

    // 5. Pack: old archived notes and old undo records go into one zip file.
    let index = Index::load(nb);
    let old_items: Vec<&archive::Item> = index
        .loose()
        .filter(|item| (now - item.archived_at).num_days() >= settings.pack_after_days)
        .collect();
    let old_entries: Vec<journal::Entry> = journal::history(nb)
        .into_iter()
        .filter(|entry| (now - entry.at).num_days() >= settings.undo_days)
        .collect();
    if !old_items.is_empty() || !old_entries.is_empty() {
        let loose: u64 = old_items.iter().map(|item| item.size).sum();
        let records: u64 = old_entries
            .iter()
            .map(|entry| entry_size(nb, &entry.id))
            .sum();
        let estimate = ((loose + records) as f64 * 0.65) as i64;
        let mut parts = Vec::new();
        if !old_items.is_empty() {
            parts.push(format!(
                "{} notes archived over {} days ago",
                old_items.len(),
                settings.pack_after_days
            ));
        }
        if !old_entries.is_empty() {
            parts.push(format!(
                "{} undo records over {} days old",
                old_entries.len(),
                settings.undo_days
            ));
        }
        proposals.push(Proposal {
            id: String::new(),
            kind: Kind::Pack,
            title: format!("Pack {} into a zip file", parts.join(" and ")),
            reason: "They're compressed into one zip in .fastmind/archive/packs. Archived notes can still be restored from it; packed undo records can't be undone any more.".into(),
            notes: old_items.iter().map(|item| item.original.clone()).collect(),
            notes_removed: 0,
            bytes_freed: 0,
            disk_freed: estimate,
            preview: None,
            status: Status::Pending,
            entry: None,
            error: None,
            plan: Plan::Pack {
                items: old_items.iter().map(|item| item.id.clone()).collect(),
                journal: old_entries.iter().map(|entry| entry.id.clone()).collect(),
            },
        });
    }

    for (n, proposal) in proposals.iter_mut().enumerate() {
        proposal.id = format!("p{}", n + 1);
    }
    let summary = summarize(&before, &proposals);
    let report = Report {
        id: journal::new_id(),
        ran_at: now,
        took_ms: started.elapsed().as_millis(),
        before,
        proposals,
        model: model.map(|_| "on".to_owned()),
        model_notes,
        summary,
    };
    save(nb, &report)?;
    Ok(report)
}

fn entry_size(nb: &Notebook, id: &str) -> u64 {
    let mut files = Vec::new();
    fsx::walk(&journal::journal_dir(nb).join(id), &|_| false, &mut files);
    files
        .iter()
        .filter_map(|path| fs::metadata(path).ok())
        .map(|meta| meta.len())
        .sum()
}

fn join(folder: &str, name: &str) -> String {
    if folder.is_empty() {
        name.to_owned()
    } else {
        format!("{folder}/{name}")
    }
}

/// Where a topic note goes: the folder most of its fragments live in, not counting the inbox.
fn majority_folder(notes: &[&Note]) -> String {
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for note in notes
        .iter()
        .filter(|note| note.status() != "inbox" && note.note_type() != "capture")
    {
        *counts.entry(note.folder()).or_default() += 1;
    }
    counts
        .into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(a.0)))
        .map(|(folder, _)| folder.to_owned())
        .unwrap_or_default()
}

fn name_topic(model: &Model, group: &[&Note]) -> Result<(String, String)> {
    let mut material = String::new();
    for note in group {
        material.push_str(&format!(
            "### {}\n{}\n\n",
            note.title,
            text::excerpt(note.body(), 1500)
        ));
    }
    let reply = model.complete(
        "You name topic notes in a personal Markdown notebook. Reply with exactly two lines:\n\
         TITLE: a short, specific title (2-6 words, no quotes, no trailing period)\n\
         SUMMARY: one or two plain sentences saying what these notes, together, establish. Use only what the notes say.",
        &format!("These notes are being gathered into one topic note:\n\n{material}"),
    )?;
    let title =
        model::field(&reply, "title").ok_or_else(|| Error::msg("the model gave no title"))?;
    let summary = model::field(&reply, "summary").unwrap_or_default();
    Ok((fsx::safe_stem(&title), summary))
}

pub fn human_bytes(bytes: u64) -> String {
    let b = bytes as f64;
    if b >= 1024.0 * 1024.0 {
        format!("{:.1} MB", b / 1024.0 / 1024.0)
    } else if b >= 1024.0 {
        format!("{:.1} KB", b / 1024.0)
    } else {
        format!("{bytes} bytes")
    }
}

fn summarize(before: &Totals, proposals: &[Proposal]) -> String {
    if proposals.is_empty() {
        return format!(
            "Dream looked at {} notes ({}) and found nothing to tidy.",
            before.notes,
            human_bytes(before.bytes)
        );
    }
    let count = |kind: Kind| proposals.iter().filter(|p| p.kind == kind).count();
    let mut parts = Vec::new();
    let merges = count(Kind::Merge);
    if merges > 0 {
        parts.push(format!(
            "{merges} duplicate{} to merge",
            if merges == 1 { "" } else { "s" }
        ));
    }
    let topics = count(Kind::Topic);
    if topics > 0 {
        parts.push(format!(
            "{topics} topic{} to gather",
            if topics == 1 { "" } else { "s" }
        ));
    }
    let archives = count(Kind::Archive);
    if archives > 0 {
        parts.push(format!("{archives} to archive"));
    }
    let links = count(Kind::FixLink);
    if links > 0 {
        parts.push(format!(
            "{links} broken link{} to fix",
            if links == 1 { "" } else { "s" }
        ));
    }
    if count(Kind::Pack) > 0 {
        parts.push("an archive to pack".into());
    }
    let removed: usize = proposals.iter().map(|p| p.notes_removed).sum();
    let freed: i64 = proposals.iter().map(|p| p.bytes_freed).sum();
    let disk: i64 = proposals.iter().map(|p| p.disk_freed).sum();
    let mut text = format!(
        "Dream looked at {} notes ({}): {}. Accepting everything takes the notebook to {} notes",
        before.notes,
        human_bytes(before.bytes + before.attachment_bytes),
        parts.join(", "),
        before.notes.saturating_sub(removed),
    );
    if freed > 0 {
        text.push_str(&format!(" and {} lighter", human_bytes(freed as u64)));
    }
    if disk > 0 {
        text.push_str(&format!(
            ", and frees about {} on disk",
            human_bytes(disk as u64)
        ));
    }
    text.push('.');
    text
}

// ---- Applying ----

/// Applies proposal `id` of the latest report.
pub fn apply(nb: &Notebook, id: &str, now: DateTime<Local>) -> Result<Proposal> {
    let mut report =
        latest(nb).ok_or_else(|| Error::msg("Dream hasn't run on this notebook yet"))?;
    let proposal = report
        .proposals
        .iter_mut()
        .find(|p| p.id == id)
        .ok_or_else(|| Error::msg(format!("no proposal {id}")))?;
    if proposal.status != Status::Pending {
        return Err(Error::msg(
            format!("proposal {id} is already {:?}", proposal.status).to_lowercase(),
        ));
    }
    match execute(nb, proposal, now) {
        Ok(entry) => {
            proposal.status = Status::Applied;
            proposal.entry = Some(entry);
            proposal.error = None;
        }
        Err(Outcome::Outdated(why)) => {
            proposal.status = Status::Outdated;
            proposal.error = Some(why);
        }
        Err(Outcome::Failed(error)) => {
            proposal.error = Some(error.to_string());
            let copy = proposal.clone();
            save(nb, &report)?;
            return Err(Error::msg(format!("{}: {}", copy.title, error)));
        }
    }
    let result = proposal.clone();
    save(nb, &report)?;
    Ok(result)
}

pub fn reject(nb: &Notebook, id: &str) -> Result<Proposal> {
    let mut report =
        latest(nb).ok_or_else(|| Error::msg("Dream hasn't run on this notebook yet"))?;
    let proposal = report
        .proposals
        .iter_mut()
        .find(|p| p.id == id)
        .ok_or_else(|| Error::msg(format!("no proposal {id}")))?;
    if proposal.status == Status::Pending {
        proposal.status = Status::Rejected;
    }
    let result = proposal.clone();
    save(nb, &report)?;
    Ok(result)
}

/// Applies every pending proposal of the given kinds (all kinds when empty), in order.
pub fn apply_all(nb: &Notebook, kinds: &[Kind], now: DateTime<Local>) -> Result<Vec<Proposal>> {
    let report = latest(nb).ok_or_else(|| Error::msg("Dream hasn't run on this notebook yet"))?;
    let ids: Vec<String> = report
        .pending()
        .filter(|p| kinds.is_empty() || kinds.contains(&p.kind))
        .map(|p| p.id.clone())
        .collect();
    let mut done = Vec::new();
    for id in ids {
        match apply(nb, &id, now) {
            Ok(proposal) => done.push(proposal),
            Err(error) => {
                if let Some(mut proposal) =
                    latest(nb).and_then(|r| r.proposals.into_iter().find(|p| p.id == id))
                {
                    proposal.error = Some(error.to_string());
                    done.push(proposal);
                }
            }
        }
    }
    Ok(done)
}

/// Applies the kinds `settings` lets Dream apply on its own.
pub fn auto_apply(
    nb: &Notebook,
    settings: &DreamSettings,
    now: DateTime<Local>,
) -> Result<Vec<Proposal>> {
    let kinds: Vec<Kind> = settings
        .auto_apply
        .iter()
        .filter_map(|name| Kind::parse(name))
        .collect();
    if kinds.is_empty() {
        return Ok(Vec::new());
    }
    apply_all(nb, &kinds, now)
}

/// Undoes an applied proposal through its journal entry.
pub fn undo(nb: &Notebook, id: &str) -> Result<Proposal> {
    let mut report =
        latest(nb).ok_or_else(|| Error::msg("Dream hasn't run on this notebook yet"))?;
    let proposal = report
        .proposals
        .iter_mut()
        .find(|p| p.id == id)
        .ok_or_else(|| Error::msg(format!("no proposal {id}")))?;
    let entry = proposal
        .entry
        .clone()
        .ok_or_else(|| Error::msg("that proposal wasn't applied"))?;
    journal::undo(nb, &entry)?;
    proposal.status = Status::Pending;
    proposal.entry = None;
    let result = proposal.clone();
    save(nb, &report)?;
    Ok(result)
}

enum Outcome {
    Outdated(String),
    Failed(Error),
}

impl From<Error> for Outcome {
    fn from(error: Error) -> Self {
        Outcome::Failed(error)
    }
}

fn execute(
    nb: &Notebook,
    proposal: &Proposal,
    now: DateTime<Local>,
) -> std::result::Result<String, Outcome> {
    let notes = nb.notes();
    let find = |path: &str| notes.iter().find(|n| n.path == path);
    let missing = |paths: &[String]| paths.iter().find(|p| find(p).is_none()).cloned();
    let mut index = Index::load(nb);
    let mut tx = Transaction::begin(nb, proposal.title.clone(), "dream")?;
    match &proposal.plan {
        Plan::Merge { keep, absorb } => {
            let mut all = vec![keep.clone()];
            all.extend(absorb.iter().cloned());
            if let Some(gone) = missing(&all) {
                return Err(Outcome::Outdated(format!("{gone} is gone")));
            }
            let keep_note = find(keep).expect("checked");
            let absorbed: Vec<&Note> = absorb.iter().filter_map(|p| find(p)).collect();
            let merged = compose::merge(keep_note, &absorbed, now);
            let gone: HashSet<String> = absorb.iter().cloned().collect();
            retarget_all(
                &mut tx,
                &notes,
                &gone,
                keep_note.stem(),
                Some((keep.as_str(), merged)),
            )?;
            for note in absorbed {
                archive::archive(
                    &mut tx,
                    &mut index,
                    &note.path,
                    &note.title,
                    &format!("merged into \"{}\"", keep_note.title),
                )?;
            }
        }
        Plan::Topic {
            path,
            title,
            summary,
            fragments,
        } => {
            if let Some(gone) = missing(fragments) {
                return Err(Outcome::Outdated(format!("{gone} is gone")));
            }
            let group: Vec<&Note> = fragments.iter().filter_map(|p| find(p)).collect();
            let text = compose::topic(title, summary, &group, now);
            let path = if nb.exists(path) {
                let (folder, name) = path.rsplit_once('/').unwrap_or(("", path));
                let stem = name.trim_end_matches(".md");
                join(
                    folder,
                    &fsx::free_name(stem, "md", |c| nb.exists(&join(folder, c))),
                )
            } else {
                path.clone()
            };
            let stem = path
                .rsplit('/')
                .next()
                .unwrap_or(&path)
                .trim_end_matches(".md")
                .to_owned();
            tx.write(&path, text.as_bytes())?;
            let gone: HashSet<String> = fragments.iter().cloned().collect();
            retarget_all(&mut tx, &notes, &gone, &stem, None)?;
            for note in group {
                archive::archive(
                    &mut tx,
                    &mut index,
                    &note.path,
                    &note.title,
                    &format!("gathered into \"{title}\""),
                )?;
            }
        }
        Plan::Archive { path } => {
            if !nb.exists(path) {
                return Err(Outcome::Outdated(format!("{path} is gone")));
            }
            let title = find(path)
                .map(|n| n.title.clone())
                .unwrap_or_else(|| path.rsplit('/').next().unwrap_or(path).to_owned());
            archive::archive(&mut tx, &mut index, path, &title, &proposal.reason)?;
        }
        Plan::FixLink { note, from, to } => {
            let Some(current) = find(note) else {
                return Err(Outcome::Outdated(format!("{note} is gone")));
            };
            if !current.links.iter().any(|l| l == from) {
                return Err(Outcome::Outdated("the link is no longer there".into()));
            }
            let body = links::retarget_with(current.body(), |target| target == from, to, false);
            let text = format!("{}{}", &current.text[..current.front.body_start], body);
            tx.write(note, text.as_bytes())?;
        }
        Plan::Pack {
            items,
            journal: entries,
        } => {
            let packable: Vec<archive::Item> = index
                .items
                .iter()
                .filter(|item| {
                    items.contains(&item.id) && item.pack.is_none() && nb.exists(&item.stored)
                })
                .cloned()
                .collect();
            let mut extra: Vec<(String, Vec<u8>)> = Vec::new();
            let mut journal_files: Vec<String> = Vec::new();
            for id in entries {
                let dir = journal::journal_dir(nb).join(id);
                let mut files = Vec::new();
                fsx::walk(&dir, &|_| false, &mut files);
                for file in files {
                    if let Some(rel) = fsx::relative(&nb.root, &file) {
                        extra.push((rel.clone(), fs::read(&file).map_err(Error::from)?));
                        journal_files.push(rel);
                    }
                }
            }
            if packable.is_empty() && extra.is_empty() {
                return Err(Outcome::Outdated("nothing left to pack".into()));
            }
            let refs: Vec<&archive::Item> = packable.iter().collect();
            let zip = archive::build_pack(nb, &refs, &extra)?;
            let base = format!("{STATE_DIR}/archive/packs/{}", now.format("%Y-%m-%d"));
            let pack = format!(
                "{}.zip",
                fsx::free_name(&base, "", |c| nb.exists(&format!("{c}.zip")))
            );
            tx.write(&pack, &zip)?;
            for item in &packable {
                tx.remove(&item.stored)?;
                if let Some(entry) = index.items.iter_mut().find(|i| i.id == item.id) {
                    entry.pack = Some(pack.clone());
                }
            }
            for file in journal_files {
                tx.remove(&file)?;
            }
        }
    }
    index.save(&mut tx)?;
    let entry = tx.commit()?;
    let mut usage = Usage::load(nb);
    if let Plan::Merge { keep, absorb } = &proposal.plan {
        for gone in absorb {
            usage.renamed(gone, keep);
        }
        let _ = usage.save(nb);
    }
    Ok(entry.id)
}

/// Points every link at a note in `gone` to `new_stem` instead, across the notebook.
/// `override_text` replaces one note's text before its links are changed (the merged note).
fn retarget_all(
    tx: &mut Transaction,
    notes: &[Note],
    gone: &HashSet<String>,
    new_stem: &str,
    override_text: Option<(&str, String)>,
) -> Result<()> {
    let resolver = Resolver::new(notes);
    let points_to = |target: &str| {
        resolver
            .resolve(target)
            .is_some_and(|path| gone.contains(path))
    };
    let mut overridden = false;
    for note in notes {
        if gone.contains(&note.path) {
            continue;
        }
        let text = match &override_text {
            Some((path, text)) if *path == note.path => {
                overridden = true;
                text.clone()
            }
            _ => note.text.clone(),
        };
        let front = crate::frontmatter::parse(&text);
        let body = &text[front.body_start..];
        let changed = links::retarget(body, &points_to, new_stem);
        let new_text = format!("{}{}", &text[..front.body_start], changed);
        if new_text != note.text {
            tx.write(&note.path, new_text.as_bytes())?;
        }
    }
    if let (Some((path, text)), false) = (&override_text, overridden) {
        tx.write(path, text.as_bytes())?;
    }
    Ok(())
}
