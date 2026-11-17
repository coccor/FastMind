//! Dream: while you sleep, FastMind compresses the notebook. It finds notes that say the same
//! thing and merges them, gathers scattered fragments into topic notes, retires notes nothing
//! uses any more, mends broken links and packs old archives into zip files.
//!
//! Dream proposes; you decide. Every proposal says what it will do and what it frees, and every
//! accepted one can be undone from the journal. Only the kinds you allow (packing, by default)
//! are applied on their own.

pub mod ai;
pub mod analyze;
pub mod compose;

use crate::archive::{self, Index};
use crate::config::DreamSettings;
use crate::journal::{self, Transaction};
use crate::links::{self, Resolver};
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
    /// New memories moved from the inbox to where they belong.
    Settle,
    /// A diagram added to a note.
    Diagram,
}

impl Kind {
    pub fn parse(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().replace('-', "_").as_str() {
            "merge" => Some(Kind::Merge),
            "topic" => Some(Kind::Topic),
            "archive" => Some(Kind::Archive),
            "fix_link" | "link" => Some(Kind::FixLink),
            "pack" => Some(Kind::Pack),
            "settle" | "file" => Some(Kind::Settle),
            "diagram" | "draw" => Some(Kind::Diagram),
            _ => None,
        }
    }
}

/// A FastDraw diagram a plan adds to a note.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Diagram {
    /// The `.fdraw` file as the preview names it (decoded, with real spaces). When the plan is
    /// applied the file is named afresh, beside the note.
    pub file: String,
    pub caption: String,
    /// The FastDraw source.
    pub text: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Plan {
    Merge {
        keep: String,
        absorb: Vec<String>,
        /// The merged note as the model wrote it.
        #[serde(default)]
        text: Option<String>,
        #[serde(default)]
        diagram: Option<Diagram>,
    },
    Topic {
        path: String,
        title: String,
        summary: String,
        fragments: Vec<String>,
        /// The topic note as the model wrote it.
        #[serde(default)]
        text: Option<String>,
        #[serde(default)]
        diagram: Option<Diagram>,
    },
    Diagram {
        note: String,
        diagram: Diagram,
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
    Settle {
        /// The folder the notes go into, or the home note they go beside.
        folder: Option<String>,
        home: Option<String>,
        paths: Vec<String>,
    },
}

/// Where the notebook keeps new captures and AI sessions: not places to settle notes into.
#[derive(Clone, Debug)]
pub struct Places {
    pub inbox: String,
    pub ai_folder: String,
    /// The user's preferences for how notes are written, given to every skill.
    pub preferences: String,
}

impl Default for Places {
    fn default() -> Self {
        Self {
            inbox: "Fresh memories".into(),
            ai_folder: "AI sessions".into(),
            preferences: String::new(),
        }
    }
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
    /// The notes a model-written proposal was written from, with their hashes: it applies only
    /// while they're unchanged.
    #[serde(default)]
    pub sources: Vec<(String, u64)>,
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

/// Saves `report` as the latest one.
pub fn save_report(nb: &Notebook, report: &Report) -> Result<()> {
    save(nb, report)
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
const ATTACHMENT_EXT: [&str; 17] = [
    "png", "jpg", "jpeg", "gif", "webp", "bmp", "svg", "fdraw", "pdf", "docx", "xlsx", "pptx",
    "zip", "mp3", "mp4", "mov", "wav",
];

/// Looks at the whole notebook and writes proposals. Changes nothing in the notes.
/// Dream with text statistics alone. FastMind itself always dreams with a model ([`run_in`]);
/// this is for tests and tools that must not call one.
pub fn run(
    nb: &Notebook,
    settings: &DreamSettings,
    ai: Option<ai::Ask>,
    now: DateTime<Local>,
) -> Result<Report> {
    run_in(nb, settings, &Places::default(), ai, now)
}

/// Dream: statistics find candidates; with `ai`, the model decides each one and writes the
/// merged and topic notes.
pub fn run_in(
    nb: &Notebook,
    settings: &DreamSettings,
    places: &Places,
    ai: Option<ai::Ask>,
    now: DateTime<Local>,
) -> Result<Report> {
    let judge = ai.map(|ask| ai::Judge::new(nb, ask, settings.model_requests, &places.preferences));
    let used_keys: std::sync::Mutex<HashSet<String>> = std::sync::Mutex::new(HashSet::new());
    let fingerprint = |paths: &[&Note]| -> Vec<(String, u64)> {
        paths
            .iter()
            .map(|n| (n.path.clone(), text::fnv(n.text.as_bytes())))
            .collect()
    };
    let started = std::time::Instant::now();
    let notes = nb.notes();
    let before = totals(nb, &notes);
    let usage = Usage::load(nb);
    let inbound = notebook::backlinks(&notes);
    let features = analyze::features(&notes);
    let mut proposals: Vec<Proposal> = Vec::new();
    let mut claimed: HashSet<usize> = HashSet::new();
    let mut model_notes = Vec::new();
    // Diagrams the model drew that don't draw, by the proposal they belong to.
    let mut held_diagrams: Vec<(usize, ai::Drafted)> = Vec::new();

    let protected = |note: &Note| {
        note.is_home()
            || note.is_pinned()
            || note.is_template()
            || matches!(
                note.stem().to_ascii_lowercase().as_str(),
                "readme" | "index" | "home"
            )
    };
    let recent = |note: &Note| now - note.touched < Duration::days(2);

    // 1. Duplicates: notes that say the same thing. The best-connected copy stays.
    let allowed: Vec<bool> = notes
        .iter()
        .map(|note| !protected(note) && !recent(note) && !note.is_record())
        .collect();
    if let Some(judge) = &judge {
        // Loose candidates: notes sharing much of their vocabulary. The model decides which
        // really say the same thing, and writes the merged note.
        let mut loose = analyze::UnionFind::new(notes.len());
        let mut strength: HashMap<usize, f64> = HashMap::new();
        for (a, b) in analyze::candidate_pairs(&features, &allowed) {
            let cos = analyze::cosine(&features[a].vector, &features[b].vector);
            if cos >= LOOSE_DUPLICATE {
                loose.union(a, b);
                for i in [a, b] {
                    let best = strength.entry(i).or_default();
                    *best = best.max(cos);
                }
            }
        }
        let mut candidates: Vec<Vec<usize>> = Vec::new();
        for mut group in loose.groups().into_iter().filter(|g| g.len() >= 2) {
            // A long chain is judged in parts, closest notes together.
            group.sort_by(|x, y| strength[y].total_cmp(&strength[x]));
            candidates.extend(
                group
                    .chunks(6)
                    .filter(|c| c.len() >= 2)
                    .map(<[usize]>::to_vec),
            );
        }
        let skips = skipped(nb);
        candidates.retain(|group| {
            let mut paths: Vec<String> = group.iter().map(|i| notes[*i].path.clone()).collect();
            paths.sort();
            !skips.contains(&format!("{:?}:{}", Kind::Merge, paths.join("|")))
        });
        let best = |g: &Vec<usize>| g.iter().map(|i| strength[i]).fold(0.0, f64::max);
        candidates.sort_by(|x, y| best(y).total_cmp(&best(x)));
        let decided = ai::in_parallel(&candidates, |group| {
            let members: Vec<&Note> = group.iter().map(|i| &notes[*i]).collect();
            let links: Vec<usize> = group
                .iter()
                .map(|i| inbound.get(&notes[*i].path).map_or(0, Vec::len))
                .collect();
            ai::merges(judge, &members, &links, &used_keys)
        });
        for (group, merges) in candidates.iter().zip(decided) {
            for merge in merges {
                let keep = group[merge.keep];
                let absorb: Vec<usize> = merge.absorb.iter().map(|i| group[*i]).collect();
                let absorbed: Vec<&Note> = absorb.iter().map(|i| &notes[*i]).collect();
                // Front matter as the offline merge makes it (aliases, tags); the model's body.
                let base = compose::merge(&notes[keep], &absorbed, now);
                let head_end = crate::frontmatter::parse(&base).body_start;
                let merged = format!("{}{}", &base[..head_end], merge.body);
                let mut all: Vec<&Note> = vec![&notes[keep]];
                all.extend(absorbed.iter().copied());
                let freed = all.iter().map(|n| n.size as i64).sum::<i64>() - merged.len() as i64;
                let names: Vec<String> = absorbed
                    .iter()
                    .map(|n| format!("\"{}\"", n.title))
                    .collect();
                let (diagram, preview) = with_drafted(
                    merge.diagram,
                    notes[keep].stem(),
                    &merged,
                    proposals.len(),
                    &mut held_diagrams,
                );
                proposals.push(Proposal {
                    id: String::new(),
                    kind: Kind::Merge,
                    title: format!("Merge {} into \"{}\"", names.join(", "), notes[keep].title),
                    reason: if merge.reason.is_empty() {
                        "They say the same thing.".into()
                    } else {
                        merge.reason
                    },
                    notes: all.iter().map(|n| n.path.clone()).collect(),
                    notes_removed: absorb.len(),
                    bytes_freed: freed,
                    disk_freed: 0,
                    preview: Some(preview),
                    status: Status::Pending,
                    entry: None,
                    error: None,
                    plan: Plan::Merge {
                        keep: notes[keep].path.clone(),
                        absorb: absorbed.iter().map(|n| n.path.clone()).collect(),
                        text: Some(merged),
                        diagram,
                    },
                    sources: fingerprint(&all),
                });
                claimed.insert(keep);
                claimed.extend(absorb);
            }
        }
    } else {
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
            let freed = absorbed.iter().map(|n| n.size as i64).sum::<i64>()
                + notes[keep].size as i64
                - merged.len() as i64;
            let names: Vec<String> = absorbed
                .iter()
                .map(|n| format!("\"{}\"", n.title))
                .collect();
            proposals.push(Proposal {
                id: String::new(),
                kind: Kind::Merge,
                title: if absorbed.iter().all(|n| n.title == notes[keep].title) {
                    format!("Merge {} copies of \"{}\"", group.len(), notes[keep].title)
                } else {
                    format!("Merge {} into \"{}\"", names.join(", "), notes[keep].title)
                },
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
                    text: None,
                    diagram: None,
                },
                sources: Vec::new(),
            });
            claimed.extend(group);
        }
    }

    // 2. Topics: short notes circling one subject, gathered into one note. Threads: AI
    //    sessions that continued the same piece of work.
    if let Some(judge) = &judge {
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
            analyze::cosine(&features[a].vector, &features[b].vector)
                + if tags[a].is_disjoint(&tags[b]) {
                    0.0
                } else {
                    0.1
                }
        };
        let mut is_fragment = vec![false; notes.len()];
        for i in &fragments {
            is_fragment[*i] = true;
        }
        let pairs = analyze::candidate_pairs(&features, &is_fragment);
        let mut tasks: Vec<(Vec<usize>, &str)> = analyze::topic_clusters(
            &fragments,
            &pairs,
            &similarity,
            settings.topic_similarity * LOOSE_TOPIC,
            settings.topic_min_notes.max(2),
        )
        .into_iter()
        .map(|members| (members.into_iter().take(12).collect(), ""))
        .collect();
        let in_tasks: HashSet<usize> = tasks.iter().flat_map(|(m, _)| m.iter().copied()).collect();
        let sessions: Vec<usize> = (0..notes.len())
            .filter(|i| {
                !claimed.contains(i)
                    && !in_tasks.contains(i)
                    && allowed[*i]
                    && notes[*i].note_type() == "ai-session"
            })
            .collect();
        let mut threads = analyze::UnionFind::new(notes.len());
        for (k, a) in sessions.iter().enumerate() {
            for b in &sessions[k + 1..] {
                let (na, nb_) = (&notes[*a], &notes[*b]);
                let close =
                    analyze::cosine(&features[*a].vector, &features[*b].vector) >= LOOSE_DUPLICATE;
                if na.front.text("project") == nb_.front.text("project")
                    && (same_thread(&na.title, &nb_.title) || close)
                {
                    threads.union(*a, *b);
                }
            }
        }
        for members in threads.groups().into_iter().filter(|g| g.len() >= 2) {
            // A thread still being worked on waits until it has been quiet for a few days.
            let newest = members
                .iter()
                .map(|i| notes[*i].touched)
                .max()
                .unwrap_or(now);
            if now - newest >= Duration::days(3) {
                tasks.push((members.into_iter().take(12).collect(), THREAD_HINT));
            }
        }
        let skips = skipped(nb);
        tasks.retain(|(members, _)| {
            let mut paths: Vec<String> = members.iter().map(|i| notes[*i].path.clone()).collect();
            paths.sort();
            !skips.contains(&format!("{:?}:{}", Kind::Topic, paths.join("|")))
        });
        let decided = ai::in_parallel(&tasks, |(members, hint)| {
            let group: Vec<&Note> = members.iter().map(|i| &notes[*i]).collect();
            ai::topic(judge, &group, hint, &used_keys)
        });
        let mut planned_paths: HashSet<String> = HashSet::new();
        for ((members, hint), topic) in tasks.iter().zip(decided) {
            let Some(topic) = topic else { continue };
            let chosen: Vec<usize> = topic.members.iter().map(|i| members[*i]).collect();
            if chosen.iter().any(|i| claimed.contains(i)) {
                continue;
            }
            let group: Vec<&Note> = chosen.iter().map(|i| &notes[*i]).collect();
            let thread = !hint.is_empty();
            let folder = if thread {
                group[0].folder().to_owned()
            } else {
                majority_folder(&group)
            };
            let name = fsx::free_name(&topic.title, "md", |candidate| {
                let rel = join(&folder, candidate);
                nb.exists(&rel) || planned_paths.contains(&rel)
            });
            let path = join(&folder, &name);
            planned_paths.insert(path.clone());
            let text = written_topic(
                &topic.title,
                &topic.summary,
                &topic.body,
                &group,
                thread,
                now,
            );
            let freed = group.iter().map(|n| n.size as i64).sum::<i64>() - text.len() as i64;
            let (diagram, preview) = with_drafted(
                topic.diagram,
                name.strip_suffix(".md").unwrap_or(&name),
                &text,
                proposals.len(),
                &mut held_diagrams,
            );
            proposals.push(Proposal {
                id: String::new(),
                kind: Kind::Topic,
                title: format!(
                    "Gather {} {} into \"{}\"",
                    group.len(),
                    if thread { "AI sessions" } else { "notes" },
                    topic.title
                ),
                reason: if topic.reason.is_empty() {
                    "They're about the same subject.".into()
                } else {
                    topic.reason
                },
                notes: group.iter().map(|n| n.path.clone()).collect(),
                notes_removed: group.len() - 1,
                bytes_freed: freed,
                disk_freed: 0,
                preview: Some(preview),
                status: Status::Pending,
                entry: None,
                error: None,
                plan: Plan::Topic {
                    path,
                    title: topic.title,
                    summary: topic.summary,
                    fragments: group.iter().map(|n| n.path.clone()).collect(),
                    text: Some(text),
                    diagram,
                },
                sources: fingerprint(&group),
            });
            claimed.extend(chosen);
        }
    } else {
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
        let is_fragment: Vec<bool> = {
            let mut flags = vec![false; notes.len()];
            for i in &fragments {
                flags[*i] = true;
            }
            flags
        };
        let fragment_pairs = analyze::candidate_pairs(&features, &is_fragment);
        let clusters = analyze::topic_clusters(
            &fragments,
            &fragment_pairs,
            &similarity,
            settings.topic_similarity,
            settings.topic_min_notes.max(2),
        );
        let mut planned_paths: HashSet<String> = HashSet::new();
        for members in clusters {
            let words = analyze::naming_words(&features, &members, 3);
            let group: Vec<&Note> = members.iter().map(|i| &notes[*i]).collect();
            let title = match analyze::naming_title(&group, &features, &members) {
                Some(phrase) => compose::title_from_words(&[phrase]),
                None => compose::title_from_words(&words),
            };
            let summary = compose::extract_summary(&group, &words, 2);
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
                    text: None,
                    diagram: None,
                },
                sources: Vec::new(),
            });
            claimed.extend(members);
        }

        // 2b. Threads: AI sessions that continued the same piece of work, gathered into one note.
        let sessions: Vec<usize> = (0..notes.len())
            .filter(|i| {
                !claimed.contains(i) && allowed[*i] && notes[*i].note_type() == "ai-session"
            })
            .collect();
        let mut threads = analyze::UnionFind::new(notes.len());
        for (k, a) in sessions.iter().enumerate() {
            for b in &sessions[k + 1..] {
                let (na, nb_) = (&notes[*a], &notes[*b]);
                if na.front.text("project") == nb_.front.text("project")
                    && same_thread(&na.title, &nb_.title)
                {
                    threads.union(*a, *b);
                }
            }
        }
        for members in threads.groups() {
            // A thread still being worked on waits until it has been quiet for a few days.
            let newest = members
                .iter()
                .map(|i| notes[*i].touched)
                .max()
                .unwrap_or(now);
            if now - newest < Duration::days(3) {
                continue;
            }
            let group: Vec<&Note> = members.iter().map(|i| &notes[*i]).collect();
            let title = thread_title(&group);
            let folder = group[0].folder().to_owned();
            let stem = fsx::safe_stem(&title);
            let name = fsx::free_name(&stem, "md", |candidate| {
                let rel = join(&folder, candidate);
                nb.exists(&rel) || planned_paths.contains(&rel)
            });
            let path = join(&folder, &name);
            planned_paths.insert(path.clone());
            let summary = format!(
                "{} AI sessions on this, from {} to {}.",
                group.len(),
                group
                    .iter()
                    .filter_map(|n| n.created)
                    .min()
                    .unwrap_or(now)
                    .format("%Y-%m-%d"),
                group
                    .iter()
                    .filter_map(|n| n.created)
                    .max()
                    .unwrap_or(now)
                    .format("%Y-%m-%d"),
            );
            let text = compose::topic(&title, &summary, &group, now);
            let text = crate::frontmatter::set(
                &text,
                "type",
                &crate::frontmatter::Value::Text("ai-thread".into()),
            );
            let freed = group.iter().map(|n| n.size as i64).sum::<i64>() - text.len() as i64;
            proposals.push(Proposal {
                id: String::new(),
                kind: Kind::Topic,
                title: format!("Gather {} AI sessions into \"{title}\"", group.len()),
                reason: format!(
                    "{} sessions continued the same piece of work. One note keeps each session's question, outcome, files and commands, in date order.",
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
                    text: None,
                    diagram: None,
                },
                sources: Vec::new(),
            });
            claimed.extend(members);
        }
    }

    // 3. Retire: notes finished long ago, never used, or empty. With a model, these are only
    //    candidates: it keeps what's still worth having.
    let archive_from = proposals.len();
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
        } else if note.note_type() == "ai-session" && idle >= settings.ai_session_days {
            Some(format!(
                "An AI session from {idle} days ago that nobody has opened or linked to since."
            ))
        } else if note.is_record() {
            // Daily notes, meetings and logs are history, kept however old.
            None
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
                sources: Vec::new(),
            });
        }
    }

    if let Some(judge) = &judge {
        let candidates: Vec<Proposal> = proposals.drain(archive_from..).collect();
        let skips = skipped(nb);
        let candidates: Vec<Proposal> = candidates
            .into_iter()
            .filter(|p| !skips.contains(&signature(p)))
            .collect();
        let found: Vec<(&Note, String)> = candidates
            .iter()
            .filter_map(|p| {
                notes
                    .iter()
                    .find(|n| n.path == p.notes[0])
                    .map(|n| (n, p.reason.clone()))
            })
            .collect();
        let batches: Vec<&[(&Note, String)]> = found.chunks(12).collect();
        let decided = ai::in_parallel(&batches, |batch| ai::archive(judge, batch, &used_keys));
        for (batch, chosen) in batches.iter().zip(decided) {
            for (at, reason) in chosen {
                let (note, rule) = &batch[at];
                if let Some(mut proposal) =
                    candidates.iter().find(|p| p.notes[0] == note.path).cloned()
                {
                    proposal.reason = if reason.is_empty() {
                        rule.clone()
                    } else {
                        format!("{reason} ({})", rule.trim_end_matches('.'))
                    };
                    proposals.push(proposal);
                }
            }
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
            sources: Vec::new(),
        });
    }

    // 4. Broken links that clearly meant an existing note. Names are grouped by first letter:
    // a typo rarely hits the first one, and comparing every name with every broken link
    // would take minutes in a large notebook.
    let resolver = Resolver::new(&notes);
    let mut names_by_first: HashMap<Option<char>, Vec<(&String, &str)>> = HashMap::new();
    for (name, path) in resolver.names() {
        names_by_first
            .entry(name.chars().next())
            .or_default()
            .push((name, *path));
    }
    if let Some(judge) = &judge {
        let names: Vec<(&String, &str)> = resolver.names().map(|(n, p)| (n, *p)).collect();
        let mut broken: Vec<ai::Broken> = Vec::new();
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
                if wanted.chars().count() < 3 {
                    continue;
                }
                let wanted_words: HashSet<String> = text::words(&wanted).collect();
                let mut options: Vec<(usize, String)> = names
                    .iter()
                    .filter_map(|(name, path)| {
                        let shown = notes
                            .iter()
                            .find(|n| n.path == *path)
                            .map(|n| n.stem().to_owned())?;
                        let name_words: HashSet<String> = text::words(name).collect();
                        let shared = wanted_words.intersection(&name_words).count();
                        let rank = if text::within_edits(name, &wanted, 3) {
                            0
                        } else if name.contains(&wanted) || wanted.contains(name.as_str()) {
                            1
                        } else if shared * 2 >= wanted_words.len().max(1) && shared > 0 {
                            2
                        } else {
                            return None;
                        };
                        Some((rank, shown))
                    })
                    .collect();
                options.sort();
                let mut unique: Vec<String> = Vec::new();
                for (_, name) in options {
                    if !unique.contains(&name) {
                        unique.push(name);
                    }
                }
                unique.truncate(6);
                if unique.is_empty() {
                    continue;
                }
                let context = note
                    .text
                    .lines()
                    .find(|line| line.contains(target.as_str()))
                    .unwrap_or("")
                    .trim()
                    .chars()
                    .take(300)
                    .collect();
                broken.push(ai::Broken {
                    note,
                    target: target.clone(),
                    context,
                    options: unique,
                });
            }
        }
        let batches: Vec<&[ai::Broken]> = broken.chunks(15).collect();
        let decided = ai::in_parallel(&batches, |batch| ai::links(judge, batch, &used_keys));
        for (batch, fixes) in batches.iter().zip(decided) {
            for (at, to, reason) in fixes {
                let item = &batch[at];
                let target = &item.target;
                let note = item.note;
                proposals.push(Proposal {
                    id: String::new(),
                    kind: Kind::FixLink,
                    title: format!("Point [[{target}]] in \"{}\" at [[{to}]]", note.title),
                    reason: if reason.is_empty() {
                        format!("No note is called \"{target}\".")
                    } else {
                        reason
                    },
                    notes: vec![note.path.clone()],
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
                    sources: Vec::new(),
                });
            }
        }
    } else {
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
                let first = wanted.chars().next();
                let length = wanted.chars().count();
                let mut matches: Vec<&str> = names_by_first
                    .get(&first)
                    .into_iter()
                    .flatten()
                    .filter(|(name, _)| name.chars().count().abs_diff(length) <= limit)
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
                        sources: Vec::new(),
                    });
                }
            }
        }
    }

    // 5. Settle: fresh memories go where they belong, the way sleep consolidates the day. The
    //    home the model suggested when it wrote the note comes first; else the folder (or
    //    home note) holding the notes most like it, when at least two agree. A note with no
    //    clear home stays new.
    let busy: HashSet<String> = proposals
        .iter()
        .flat_map(|p| p.notes.iter().cloned())
        .collect();
    let settling: Vec<&Note> = notes
        .iter()
        .filter(|n| n.status() == "inbox" && !protected(n) && !busy.contains(&n.path))
        .filter(|n| now - n.touched >= Duration::hours(settings.settle_after_hours))
        .collect();
    if !settling.is_empty() {
        let search = crate::search::Index::build(&notes, Vec::new());
        let mut unsettled: Vec<ai::Unsettled> = Vec::new();
        let all_folders: Vec<String> = {
            let mut counts: HashMap<String, usize> = HashMap::new();
            for n in &notes {
                let folder = n.folder();
                if !folder.is_empty()
                    && folder != places.inbox
                    && !folder.starts_with(&format!("{}/", places.inbox))
                    && !folder.starts_with(&places.ai_folder)
                    && !folder.contains("attachments")
                {
                    *counts.entry(format!("{folder}/")).or_default() += 1;
                }
            }
            let mut list: Vec<(String, usize)> = counts.into_iter().collect();
            list.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
            list.into_iter().map(|(f, _)| f).collect()
        };
        // (folder, home) and the notes going there, each with why.
        type Place = (Option<String>, Option<String>);
        let mut groups: Vec<(Place, Vec<(String, String)>)> = Vec::new();
        for note in settling {
            let suggested = note
                .front
                .text("suggested_home")
                .map(str::trim)
                .unwrap_or("");
            let because = note
                .front
                .text("suggested_because")
                .unwrap_or("")
                .trim()
                .to_owned();
            let from_model = if let Some(name) = suggested
                .strip_prefix("[[")
                .and_then(|h| h.strip_suffix("]]"))
            {
                notes
                    .iter()
                    .find(|n| n.stem() == name && n.is_home())
                    .map(|h| ((None, Some(h.path.clone())), because.clone()))
            } else if !suggested.is_empty() {
                let folder = suggested.trim_matches('/').to_owned();
                nb.path(&folder)
                    .ok()
                    .filter(|p| p.is_dir())
                    .map(|_| ((Some(folder), None), because.clone()))
            } else {
                None
            };
            if from_model.is_none() && judge.is_some() {
                // Asked below, with every note that needs a home in one go.
                let offered =
                    crate::filing::suggest(&search, &note.path, &places.inbox, &places.ai_folder);
                let mut homes: Vec<String> = offered
                    .iter()
                    .filter(|h| h.kind == "note")
                    .filter_map(|h| {
                        notes
                            .iter()
                            .find(|n| n.path == h.target)
                            .map(|n| n.stem().to_owned())
                    })
                    .collect();
                for home in notes
                    .iter()
                    .filter(|n| n.is_home() && !n.path.starts_with(&format!("{}/", places.inbox)))
                {
                    if homes.len() >= 12 {
                        break;
                    }
                    if !homes.contains(&home.stem().to_owned()) {
                        homes.push(home.stem().to_owned());
                    }
                }
                let mut folders: Vec<String> = offered
                    .iter()
                    .filter(|h| h.kind == "folder" && !h.target.is_empty())
                    .map(|h| format!("{}/", h.target))
                    .collect();
                for folder in &all_folders {
                    if folders.len() >= 25 {
                        break;
                    }
                    if !folders.contains(folder) {
                        folders.push(folder.clone());
                    }
                }
                unsettled.push(ai::Unsettled {
                    note,
                    homes,
                    folders,
                });
                continue;
            }
            let choice = from_model.or_else(|| {
                let homes =
                    crate::filing::suggest(&search, &note.path, &places.inbox, &places.ai_folder);
                let best = homes.first()?;
                if best.kind == "note" {
                    Some((
                        (None, Some(best.target.clone())),
                        format!("it's most like notes linked from \"{}\"", best.label),
                    ))
                } else if best.because.len() >= 2 {
                    Some((
                        (Some(best.target.clone()), None),
                        format!("beside {}", best.because.join(", ")),
                    ))
                } else {
                    None
                }
            });
            let Some((place, why)) = choice else { continue };
            // Already there: nothing to move.
            if place.0.as_deref() == Some(note.folder()) {
                continue;
            }
            match groups.iter_mut().find(|(p, _)| *p == place) {
                Some((_, members)) => members.push((note.path.clone(), why)),
                None => groups.push((place, vec![(note.path.clone(), why)])),
            }
        }
        if let Some(judge) = &judge {
            let batches: Vec<&[ai::Unsettled]> = unsettled.chunks(10).collect();
            let decided = ai::in_parallel(&batches, |batch| ai::homes(judge, batch, &used_keys));
            for (batch, placed) in batches.iter().zip(decided) {
                for (at, is_home, name, why) in placed {
                    let note = batch[at].note;
                    let place = if is_home {
                        match notes.iter().find(|n| n.stem() == name && n.is_home()) {
                            Some(home) => (None, Some(home.path.clone())),
                            None => continue,
                        }
                    } else {
                        (Some(name.trim_matches('/').to_owned()), None)
                    };
                    if place.0.as_deref() == Some(note.folder()) {
                        continue;
                    }
                    match groups.iter_mut().find(|(p, _)| *p == place) {
                        Some((_, members)) => members.push((note.path.clone(), why)),
                        None => groups.push((place, vec![(note.path.clone(), why)])),
                    }
                }
            }
        }
        for ((folder, home), members) in groups {
            let titles: Vec<String> = members
                .iter()
                .filter_map(|(path, _)| notes.iter().find(|n| &n.path == path))
                .map(|n| format!("\"{}\"", n.title))
                .collect();
            let place = match (&folder, &home) {
                (_, Some(home)) => notes
                    .iter()
                    .find(|n| &n.path == home)
                    .map_or(home.clone(), |h| format!("\"{}\"", h.title)),
                (Some(folder), None) if folder.is_empty() => "the notebook root".into(),
                (Some(folder), None) => format!("{folder}/"),
                (None, None) => continue,
            };
            let count = members.len();
            let reasons: Vec<String> = members
                .iter()
                .zip(&titles)
                .map(|((_, why), title)| {
                    if why.is_empty() {
                        title.clone()
                    } else {
                        format!("{title}: {why}")
                    }
                })
                .collect();
            proposals.push(Proposal {
                id: String::new(),
                kind: Kind::Settle,
                title: if count == 1 {
                    format!("Settle {} in {place}", titles[0])
                } else {
                    format!("Settle {count} fresh memories in {place}")
                },
                reason: format!(
                    "{}. {}",
                    reasons.join("; "),
                    if home.is_some() {
                        "They move beside it and it links to them."
                    } else {
                        "They move into that folder; links to them follow."
                    }
                ),
                notes: members.iter().map(|(p, _)| p.clone()).collect(),
                notes_removed: 0,
                bytes_freed: 0,
                disk_freed: 0,
                preview: None,
                status: Status::Pending,
                entry: None,
                error: None,
                plan: Plan::Settle {
                    folder,
                    home,
                    paths: members.into_iter().map(|(p, _)| p).collect(),
                },
                sources: Vec::new(),
            });
        }
    }

    // 6. Pack: old archived notes and old undo records go into one zip file.
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
            sources: Vec::new(),
        });
    }

    if let Some(judge) = &judge {
        judge.save(&used_keys.lock().expect("used lock"));
        let errors = judge.errors.lock().expect("errors lock").clone();
        let mut seen = HashSet::new();
        for error in errors {
            if seen.insert(error.clone()) {
                model_notes.push(error);
            }
        }
    }
    let failed_questions = model_notes.len();
    // Diagrams that still don't draw are left out; the notes they came with stay.
    if let Some((_, first)) = held_diagrams.first() {
        let dropped = held_diagrams.len();
        model_notes.push(format!(
            "{dropped} diagram{} dropped: {}",
            if dropped == 1 { "" } else { "s" },
            first
                .errors
                .first()
                .map_or("it doesn't draw", String::as_str)
        ));
    }
    let skips = skipped(nb);
    proposals.retain(|proposal| !skips.contains(&signature(proposal)));
    for (n, proposal) in proposals.iter_mut().enumerate() {
        proposal.id = format!("p{}", n + 1);
    }
    let mut summary = summarize(&before, &proposals);
    if let Some(judge) = &judge
        && failed_questions > 0
    {
        let asked = judge.asked.load(std::sync::atomic::Ordering::SeqCst);
        summary = format!(
            "{summary} The model couldn't answer {} of {asked} questions ({}), so some candidates wait for the next night.",
            failed_questions.min(asked),
            model_notes[0]
        );
    }
    let report = Report {
        id: journal::new_id(),
        ran_at: now,
        took_ms: started.elapsed().as_millis(),
        before,
        proposals,
        model: judge.as_ref().map(|j| {
            format!(
                "{} questions",
                j.asked.load(std::sync::atomic::Ordering::SeqCst)
            )
        }),
        model_notes,
        summary,
    };
    save(nb, &report)?;
    Ok(report)
}

/// A title without a trailing copy number ("Backlog creation 2").
fn base_title(title: &str) -> String {
    let lower = title.trim().to_lowercase();
    match lower.rsplit_once(' ') {
        Some((head, tail)) if tail.chars().all(|c| c.is_ascii_digit()) => head.to_owned(),
        _ => lower,
    }
}

/// Two AI sessions continue one piece of work when their titles match, or share most of their
/// words (at least two).
fn same_thread(a: &str, b: &str) -> bool {
    let (a, b) = (base_title(a), base_title(b));
    if a == b {
        return true;
    }
    let ta: HashSet<String> = text::terms(&a).into_iter().collect();
    let tb: HashSet<String> = text::terms(&b).into_iter().collect();
    let shared = ta.intersection(&tb).count();
    shared >= 2 && shared * 3 >= ta.len().max(tb.len()) * 2
}

fn thread_title(group: &[&Note]) -> String {
    let mut titles: Vec<String> = group.iter().map(|n| n.title.trim().to_owned()).collect();
    titles.sort_by_key(|t| (t.len(), t.clone()));
    let base = base_title(&titles[0]);
    let mut chars = base.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
        None => "AI sessions".to_owned(),
    }
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

/// The diagram the model drew for the note at proposal `index` (named after `stem`), and the
/// note's preview with it. A diagram that doesn't draw is held for the fix pass, so the note
/// goes without it until then.
fn with_drafted(
    drafted: Option<ai::Drafted>,
    stem: &str,
    text: &str,
    index: usize,
    held: &mut Vec<(usize, ai::Drafted)>,
) -> (Option<Diagram>, String) {
    let Some(drafted) = drafted else {
        return (None, text.to_owned());
    };
    if !drafted.errors.is_empty() {
        held.push((index, drafted));
        return (None, text.to_owned());
    }
    let file = format!("{stem}.fdraw");
    let preview = compose::with_diagram(text, &drafted.caption, &compose::diagram_link(&file));
    let diagram = Diagram {
        file: format!("attachments/{file}"),
        caption: drafted.caption,
        text: drafted.text,
    };
    (Some(diagram), preview)
}

/// A topic note the model wrote: front matter as gathering makes it, then its summary, body
/// and where it was gathered from.
fn written_topic(
    title: &str,
    summary: &str,
    body: &str,
    group: &[&Note],
    thread: bool,
    now: DateTime<Local>,
) -> String {
    let base = compose::topic(title, summary, group, now);
    let base = if thread {
        crate::frontmatter::set(
            &base,
            "type",
            &crate::frontmatter::Value::Text("ai-thread".into()),
        )
    } else {
        base
    };
    let head_end = crate::frontmatter::parse(&base).body_start;
    let mut text = format!("{}# {title}\n", &base[..head_end]);
    if !summary.trim().is_empty() {
        text.push_str(&format!("\n> {}\n", summary.trim().replace('\n', "\n> ")));
    }
    text.push_str(&format!("\n{}\n\n## Gathered from\n\n", body.trim()));
    let mut ordered: Vec<&&Note> = group.iter().collect();
    ordered.sort_by_key(|n| n.created.unwrap_or(n.touched));
    for note in ordered {
        text.push_str(&format!(
            "- {} ({})\n",
            note.title,
            note.created.unwrap_or(note.touched).format("%Y-%m-%d")
        ));
    }
    text
}

/// Notes this alike in vocabulary are shown to the model as possible duplicates.
const LOOSE_DUPLICATE: f64 = 0.3;
/// With a model, fragments are clustered this much more loosely than the offline threshold.
const LOOSE_TOPIC: f64 = 0.7;
/// Marks a gathering task as AI sessions: the gather-thread skill judges it.
const THREAD_HINT: &str = "thread";

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
    let diagrams = count(Kind::Diagram);
    if diagrams > 0 {
        parts.push(format!(
            "{diagrams} diagram{} to draw",
            if diagrams == 1 { "" } else { "s" }
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
    let settled: usize = proposals
        .iter()
        .filter(|p| p.kind == Kind::Settle)
        .map(|p| p.notes.len())
        .sum();
    if settled > 0 {
        parts.push(format!(
            "{settled} fresh memor{} to settle",
            if settled == 1 { "y" } else { "ies" }
        ));
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
        remember_skip(nb, proposal)?;
    }
    let result = proposal.clone();
    save(nb, &report)?;
    Ok(result)
}

/// What a proposal is about, independent of the night it was made: its kind and its notes.
/// A skipped proposal with the same signature isn't made again. When the notes change (a new
/// copy turns up), the signature changes and Dream may ask again.
pub fn signature(proposal: &Proposal) -> String {
    let mut notes = proposal.notes.clone();
    notes.sort();
    format!("{:?}:{}", proposal.kind, notes.join("|"))
}

fn skipped_file(nb: &Notebook) -> std::path::PathBuf {
    dream_dir(nb).join("skipped.json")
}

fn skipped(nb: &Notebook) -> HashSet<String> {
    fs::read(skipped_file(nb))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Vec<String>>(&bytes).ok())
        .unwrap_or_default()
        .into_iter()
        .collect()
}

fn remember_skip(nb: &Notebook, proposal: &Proposal) -> Result<()> {
    let mut all: Vec<String> = skipped(nb).into_iter().collect();
    all.push(signature(proposal));
    all.sort();
    all.dedup();
    fsx::write_atomic(&skipped_file(nb), &serde_json::to_vec_pretty(&all)?)
}

/// Forgets every skip, so Dream may propose them again.
pub fn forget_skips(nb: &Notebook) -> Result<()> {
    let path = skipped_file(nb);
    if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
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
    // A model-written note is applied as previewed, only if its notes are as they were.
    for (path, hash) in &proposal.sources {
        match find(path) {
            Some(note) if text::fnv(note.text.as_bytes()) == *hash => {}
            Some(note) => {
                return Err(Outcome::Outdated(format!(
                    "\"{}\" changed since Dream wrote this; it'll be looked at again tonight",
                    note.title
                )));
            }
            None => return Err(Outcome::Outdated(format!("{path} is gone"))),
        }
    }
    let mut tx = Transaction::begin(nb, proposal.title.clone(), "dream")?;
    match &proposal.plan {
        // Settling files each note in this same change, below.
        Plan::Settle { .. } => {}
        Plan::Merge {
            keep,
            absorb,
            text,
            diagram,
        } => {
            let mut all = vec![keep.clone()];
            all.extend(absorb.iter().cloned());
            if let Some(gone) = missing(&all) {
                return Err(Outcome::Outdated(format!("{gone} is gone")));
            }
            let keep_note = find(keep).expect("checked");
            let absorbed: Vec<&Note> = absorb.iter().filter_map(|p| find(p)).collect();
            let merged = text
                .clone()
                .unwrap_or_else(|| compose::merge(keep_note, &absorbed, now));
            let merged = match diagram {
                Some(d) => write_diagram(&mut tx, nb, keep, &merged, d)?,
                None => merged,
            };
            let gone: HashSet<String> = absorb.iter().cloned().collect();
            retarget_all(&mut tx, &notes, &gone, keep, Some((keep.as_str(), merged)))?;
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
            text: written,
            diagram,
        } => {
            if let Some(gone) = missing(fragments) {
                return Err(Outcome::Outdated(format!("{gone} is gone")));
            }
            let group: Vec<&Note> = fragments.iter().filter_map(|p| find(p)).collect();
            let text = written
                .clone()
                .unwrap_or_else(|| compose::topic(title, summary, &group, now));
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
            let text = match diagram {
                Some(d) => write_diagram(&mut tx, nb, &path, &text, d)?,
                None => text,
            };
            tx.write(&path, text.as_bytes())?;
            let gone: HashSet<String> = fragments.iter().cloned().collect();
            retarget_all(&mut tx, &notes, &gone, &path, None)?;
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
        Plan::Diagram { note, diagram } => {
            let Some(current) = find(note) else {
                return Err(Outcome::Outdated(format!("{note} is gone")));
            };
            let text = write_diagram(&mut tx, nb, note, &current.text, diagram)?;
            tx.write(note, text.as_bytes())?;
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
    if let Plan::Settle {
        folder,
        home,
        paths,
    } = &proposal.plan
    {
        if let Some(home) = home
            && find(home).is_none()
        {
            return Err(Outcome::Outdated(format!("{home} is gone")));
        }
        let mut moved = 0;
        for path in paths {
            // Gone, or settled by hand since: nothing to do for it.
            if find(path).is_none_or(|n| n.status() != "inbox") {
                continue;
            }
            crate::filing::file_in(&mut tx, path, folder.as_deref(), home.as_deref())?;
            moved += 1;
        }
        if moved == 0 {
            return Err(Outcome::Outdated(
                "those notes were settled or moved already".into(),
            ));
        }
    }
    index.save(&mut tx)?;
    let entry = tx.commit()?;
    let mut usage = Usage::load(nb);
    if let Plan::Merge { keep, absorb, .. } = &proposal.plan {
        for gone in absorb {
            usage.renamed(gone, keep);
        }
        let _ = usage.save(nb);
    }
    Ok(entry.id)
}

/// Writes the diagram's `.fdraw` into `attachments/` beside the note at `note_path`, under a
/// free name made from the note's name, and returns `note_text` with the link to it added.
fn write_diagram(
    tx: &mut Transaction,
    nb: &Notebook,
    note_path: &str,
    note_text: &str,
    d: &Diagram,
) -> Result<String> {
    let (folder, name) = note_path.rsplit_once('/').unwrap_or(("", note_path));
    let stem = name.strip_suffix(".md").unwrap_or(name);
    let attachments = join(folder, "attachments");
    let file = fsx::free_name(stem, "fdraw", |c| nb.exists(&join(&attachments, c)));
    tx.write(&join(&attachments, &file), d.text.as_bytes())?;
    Ok(compose::with_diagram(
        note_text,
        &d.caption,
        &compose::diagram_link(&file),
    ))
}

/// Points every link at a note in `gone` to `new_stem` instead, across the notebook.
/// `override_text` replaces one note's text before its links are changed (the merged note).
fn retarget_all(
    tx: &mut Transaction,
    notes: &[Note],
    gone: &HashSet<String>,
    new_path: &str,
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
        let changed = links::retarget_note(body, points_to, new_path, &note.path);
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
