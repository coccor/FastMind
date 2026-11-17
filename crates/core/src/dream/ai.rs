//! Dream's judgment. Text statistics only find candidates, loosely and fast, so a notebook of
//! thousands of notes costs a few dozen model requests a night. The model decides: which notes
//! really say the same thing and how the merged note reads, whether scattered notes make one
//! subject and what the topic note says, whether an old note is still worth keeping, which note
//! a broken link meant, and where a fresh memory belongs.
//!
//! Verdicts are cached by the notes' content, so a night with nothing new asks nothing again.

use crate::notebook::{Note, Notebook};
use crate::{Error, Result, fsx, text};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Asks the model: system prompt, user turn and the JSON schema of the answer in, its JSON out.
pub type Ask<'a> = &'a (dyn Fn(&str, &str, &Value) -> Result<String> + Sync);

/// How many requests run at once.
const WORKERS: usize = 3;

/// The model, a request budget, a cache of earlier verdicts and what went wrong.
pub struct Judge<'a> {
    ask: Ask<'a>,
    nb: Notebook,
    preferences: String,
    /// The night's date, for the prompts.
    now: chrono::DateTime<chrono::Local>,
    budget: AtomicUsize,
    /// Requests held back from `budget` for the last step of the night.
    reserve: AtomicUsize,
    cache: Mutex<HashMap<String, String>>,
    cache_path: std::path::PathBuf,
    pub errors: Mutex<Vec<String>>,
    pub asked: AtomicUsize,
}

impl<'a> Judge<'a> {
    pub fn new(
        nb: &Notebook,
        ask: Ask<'a>,
        budget: usize,
        preferences: &str,
        now: chrono::DateTime<chrono::Local>,
    ) -> Self {
        let cache_path = nb.state_dir().join("dream").join("verdicts.json");
        let cache = std::fs::read(&cache_path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        Self {
            ask,
            nb: nb.clone(),
            preferences: preferences.to_owned(),
            now,
            budget: AtomicUsize::new(budget),
            reserve: AtomicUsize::new(0),
            cache: Mutex::new(cache),
            cache_path,
            errors: Mutex::new(Vec::new()),
            asked: AtomicUsize::new(0),
        }
    }

    /// Holds back up to `n` requests from the budget, until [`Judge::release`].
    pub fn hold(&self, n: usize) {
        let left = self
            .budget
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |left| {
                Some(left - left.min(n))
            })
            .unwrap_or_else(|left| left);
        self.reserve.fetch_add(left.min(n), Ordering::SeqCst);
    }

    /// Gives the requests held back to the budget again.
    pub fn release(&self) {
        let held = self.reserve.swap(0, Ordering::SeqCst);
        self.budget.fetch_add(held, Ordering::SeqCst);
    }

    /// The system prompt for skill `name`.
    fn prompt(&self, name: &str) -> String {
        self.prompt_with(name, &[])
    }

    /// The system prompt for skill `name`, followed by the `extra` skills (the jobs that draw
    /// get FastDraw's language).
    pub fn prompt_with(&self, name: &str, extra: &[&str]) -> String {
        crate::skills::system_at(
            Some(&self.nb),
            name,
            extra,
            &self.preferences,
            crate::skills::JSON,
            self.now,
        )
    }

    /// Keeps the verdicts for the notes still around, so the cache doesn't grow forever.
    pub fn save(&self, used: &std::collections::HashSet<String>) {
        let mut cache = self.cache.lock().expect("cache lock").clone();
        cache.retain(|key, _| used.contains(key));
        if let Ok(bytes) = serde_json::to_vec(&cache) {
            let _ = std::fs::create_dir_all(self.cache_path.parent().expect("has a parent"));
            let _ = fsx::write_atomic(&self.cache_path, &bytes);
        }
    }

    /// The cached answer for `key`, without asking; the key is kept for the next night.
    fn cached(
        &self,
        used: &Mutex<std::collections::HashSet<String>>,
        key: &str,
        system: &str,
    ) -> Option<Value> {
        let key = cache_key(key, system);
        let cached = self.cache.lock().expect("cache lock").get(&key).cloned();
        used.lock().expect("used lock").insert(key);
        serde_json::from_str(&cached?).ok()
    }

    /// The model's answer for `key`: from the cache when these exact notes were judged before.
    fn verdict(
        &self,
        used: &Mutex<std::collections::HashSet<String>>,
        key: &str,
        system: &str,
        user: &str,
        schema: &Value,
    ) -> Option<Value> {
        if let Some(cached) = self.cached(used, key, system) {
            return Some(cached);
        }
        let key = &cache_key(key, system);
        // Take one request from the budget, or give up on this one.
        if self
            .budget
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |left| {
                left.checked_sub(1)
            })
            .is_err()
        {
            return None;
        }
        self.asked.fetch_add(1, Ordering::SeqCst);
        let answer = (self.ask)(system, user, schema).and_then(|reply| {
            crate::write::json_in(&reply).map_err(|_| Error::msg("the model's reply wasn't JSON"))
        });
        match answer {
            Ok(value) => {
                self.cache
                    .lock()
                    .expect("cache lock")
                    .insert(key.to_owned(), value.to_string());
                Some(value)
            }
            Err(error) => {
                self.errors
                    .lock()
                    .expect("errors lock")
                    .push(error.to_string());
                None
            }
        }
    }
}

/// `key` with the system prompt it was asked with: a changed skill (or preferences) asks again,
/// but a new day doesn't, so the date line is left out.
fn cache_key(key: &str, system: &str) -> String {
    let undated: Vec<&str> = system
        .lines()
        .filter(|line| !line.starts_with(crate::skills::TODAY))
        .collect();
    format!(
        "{key}~{:x}",
        text::fnv(
            undated
                .join(
                    "
"
                )
                .as_bytes()
        )
    )
}

/// The cache key for judging `notes` for `task`: their paths and exact contents.
pub fn key(task: &str, notes: &[&Note]) -> String {
    let mut parts: Vec<String> = notes
        .iter()
        .map(|n| format!("{}#{:x}", n.path, text::fnv(n.text.as_bytes())))
        .collect();
    parts.sort();
    format!("{task}:{:x}", text::fnv(parts.join("|").as_bytes()))
}

/// Runs `f` over `items`, a few at a time.
pub fn in_parallel<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let next = AtomicUsize::new(0);
    let results: Mutex<Vec<(usize, R)>> = Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for _ in 0..WORKERS.min(items.len()) {
            scope.spawn(|| {
                loop {
                    let at = next.fetch_add(1, Ordering::SeqCst);
                    let Some(item) = items.get(at) else { break };
                    let result = f(item);
                    results.lock().expect("results lock").push((at, result));
                }
            });
        }
    });
    let mut results = results.into_inner().expect("results lock");
    results.sort_by_key(|(at, _)| *at);
    results.into_iter().map(|(_, r)| r).collect()
}

fn escape(text: &str) -> String {
    text.replace("</", "<\\/")
}

fn attribute(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
}

/// A note as the model reads it, with an id to answer by.
fn block(id: &str, note: &Note, max: usize, extra: &str) -> String {
    let body: String = note.body().chars().take(max).collect();
    let cut = if note.body().chars().count() > max {
        "\n[…cut]"
    } else {
        ""
    };
    format!(
        "<note id=\"{id}\" title=\"{}\" path=\"{}\" date=\"{}\"{extra}>\n{}{cut}\n</note>\n",
        attribute(&note.title),
        attribute(&note.path),
        note.created.unwrap_or(note.touched).format("%Y-%m-%d"),
        escape(body.trim())
    )
}

fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect()
}

fn id_index(id: &str) -> Option<usize> {
    id.trim()
        .trim_start_matches('n')
        .parse::<usize>()
        .ok()?
        .checked_sub(1)
}

// ---- Diagrams ----

/// The skills a job that may draw is given after its own.
const DRAWING: &[&str] = &["fastdraw-language"];

/// A diagram as the model drew it, with what's wrong with it.
#[derive(Clone, Debug)]
pub struct Drafted {
    pub caption: String,
    /// The FastDraw source, without a code fence.
    pub text: String,
    /// What keeps it from drawing (`Line N: …`); empty when it draws.
    pub errors: Vec<String>,
}

/// The diagram in an answer's `diagram` and `diagram_caption` fields, or `None` when it
/// has none.
pub fn drafted(text: &Value, caption: &Value) -> Option<Drafted> {
    let caption = caption
        .as_str()
        .unwrap_or("")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    drafted_from(text, caption)
}

fn drafted_from(text: &Value, caption: String) -> Option<Drafted> {
    let text = crate::diagram::unfence(text.as_str()?)?;
    let errors = crate::diagram::errors(&text);
    Some(Drafted {
        caption,
        text,
        errors,
    })
}

/// A diagram that doesn't draw, asked about once more: the job's own skill, the errors, and
/// "fix only these". `key` is the job's cache key; the fix is cached beside it. The caption
/// stays as it was.
pub fn fix(
    judge: &Judge,
    skill: &str,
    key: &str,
    drafted: &Drafted,
    used: &Mutex<std::collections::HashSet<String>>,
) -> Option<Drafted> {
    let user = format!(
        "The diagram you drew doesn't draw.

<diagram>
{}
</diagram>

<errors>
{}
</errors>

Fix only these errors; change nothing else.",
        escape(drafted.text.trim()),
        escape(&drafted.errors.join(
            "
"
        ))
    );
    let schema = json!({
        "type": "object", "additionalProperties": false, "required": ["diagram"],
        "properties": {"diagram": {"type": "string"}}
    });
    let system = judge.prompt_with(skill, DRAWING);
    let answer = judge.verdict(used, &format!("{key}~fix"), &system, &user, &schema)?;
    drafted_from(&answer["diagram"], drafted.caption.clone())
}

/// Whether a note was already asked about a diagram and it came to nothing: not worth one, no
/// diagram in the answer, or a diagram still broken after its fix. Asks nothing.
pub fn came_to_nothing<'j>(
    judge: &'j Judge,
    used: &'j Mutex<std::collections::HashSet<String>>,
) -> impl Fn(&Note) -> bool + 'j {
    let system = judge.prompt_with("draw-diagram", DRAWING);
    move |note| {
        let key = key("diagram", &[note]);
        let Some(answer) = judge.cached(used, &key, &system) else {
            return false;
        };
        if answer["worth"].as_bool() != Some(true) {
            return true;
        }
        let Some(drafted) = drafted(&answer["diagram"], &answer["caption"]) else {
            return true;
        };
        if drafted.errors.is_empty() {
            return false;
        }
        judge
            .cached(used, &format!("{key}~fix"), &system)
            .and_then(|fixed| drafted_from(&fixed["diagram"], String::new()))
            .is_some_and(|fixed| !fixed.errors.is_empty())
    }
}

/// What the model thinks of drawing a diagram for one note.
pub struct Drawing {
    pub worth: bool,
    pub caption: String,
    pub reason: String,
    /// The diagram, when it's worth one and the answer has one.
    pub diagram: Option<Drafted>,
}

fn draw_schema() -> Value {
    json!({
        "type": "object", "additionalProperties": false,
        "required": ["worth", "kind", "caption", "diagram", "reason"],
        "properties": {
            "worth": {"type": "boolean"}, "kind": {"type": "string"},
            "caption": {"type": "string"}, "diagram": {"type": "string"},
            "reason": {"type": "string"}
        }
    })
}

/// Whether `note` is worth a diagram, and the diagram.
pub fn draw(
    judge: &Judge,
    note: &Note,
    used: &Mutex<std::collections::HashSet<String>>,
) -> Option<Drawing> {
    let key = key("diagram", &[note]);
    let user = block("n1", note, 6000, "");
    let system = judge.prompt_with("draw-diagram", DRAWING);
    let answer = judge.verdict(used, &key, &system, &user, &draw_schema())?;
    let worth = answer["worth"].as_bool() == Some(true);
    let diagram = if worth {
        drafted(&answer["diagram"], &answer["caption"])
    } else {
        None
    };
    Some(Drawing {
        worth,
        caption: diagram
            .as_ref()
            .map(|d| d.caption.clone())
            .unwrap_or_default(),
        reason: answer["reason"].as_str().unwrap_or("").trim().to_owned(),
        diagram,
    })
}

// ---- Merges ----

pub struct Merge {
    /// Indexes into the group given.
    pub keep: usize,
    pub absorb: Vec<usize>,
    pub reason: String,
    /// The merged note's body, starting with its `# ` title.
    pub body: String,
    pub diagram: Option<Drafted>,
}

fn merge_schema() -> Value {
    json!({
        "type": "object", "additionalProperties": false, "required": ["groups"],
        "properties": {"groups": {"type": "array", "items": {
            "type": "object", "additionalProperties": false,
            "required": ["keep", "absorb", "reason", "text", "diagram", "diagram_caption"],
            "properties": {
                "keep": {"type": "string"}, "absorb": {"type": "array", "items": {"type": "string"}},
                "reason": {"type": "string"}, "text": {"type": "string"},
                "diagram": {"type": "string"}, "diagram_caption": {"type": "string"}
            }
        }}}
    })
}

/// Which of `group` are duplicates, and the merged notes. `links[i]` is how many notes link
/// to `group[i]`.
pub fn merges(
    judge: &Judge,
    group: &[&Note],
    links: &[usize],
    used: &Mutex<std::collections::HashSet<String>>,
) -> Vec<Merge> {
    let key = key("merge", group);
    let mut user = String::from("Notes that might be duplicates:\n\n");
    for (i, note) in group.iter().enumerate() {
        user.push_str(&block(
            &format!("n{}", i + 1),
            note,
            8000,
            &format!(" linked_from=\"{}\"", links[i]),
        ));
    }
    let system = judge.prompt_with("merge", DRAWING);
    let Some(answer) = judge.verdict(used, &key, &system, &user, &merge_schema()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut taken = std::collections::HashSet::new();
    for g in answer["groups"].as_array().into_iter().flatten() {
        let Some(keep) = g["keep"]
            .as_str()
            .and_then(id_index)
            .filter(|i| *i < group.len())
        else {
            continue;
        };
        let absorb: Vec<usize> = strings(&g["absorb"])
            .iter()
            .filter_map(|id| id_index(id))
            .filter(|i| *i < group.len() && *i != keep)
            .collect();
        let body = g["text"].as_str().unwrap_or("").trim().to_owned();
        if absorb.is_empty()
            || body.is_empty()
            || !taken.insert(keep)
            || absorb.iter().any(|i| !taken.insert(*i))
        {
            continue;
        }
        out.push(Merge {
            keep,
            absorb,
            reason: g["reason"].as_str().unwrap_or("").trim().to_owned(),
            body: ensure_title(&body, &group[keep].title),
            diagram: drafted(&g["diagram"], &g["diagram_caption"]),
        });
    }
    out
}

fn ensure_title(body: &str, title: &str) -> String {
    let body = body.replace("\r\n", "\n");
    let body = body.trim();
    if body.starts_with("# ") {
        format!("{body}\n")
    } else {
        format!("# {title}\n\n{body}\n")
    }
}

// ---- Topics ----

pub struct Topic {
    pub members: Vec<usize>,
    pub title: String,
    pub summary: String,
    pub body: String,
    pub reason: String,
    pub diagram: Option<Drafted>,
}

fn topic_schema() -> Value {
    json!({
        "type": "object", "additionalProperties": false,
        "required": ["members", "title", "summary", "body", "reason", "diagram", "diagram_caption"],
        "properties": {
            "members": {"type": "array", "items": {"type": "string"}},
            "title": {"type": "string"}, "summary": {"type": "string"},
            "body": {"type": "string"}, "reason": {"type": "string"},
            "diagram": {"type": "string"}, "diagram_caption": {"type": "string"}
        }
    })
}

/// The skill that judges a gathering task with `hint` (see [`topic`]).
pub fn topic_skill(hint: &str) -> &'static str {
    if hint.is_empty() {
        "gather-topic"
    } else {
        "gather-thread"
    }
}

/// Whether `group` makes one subject, and the topic note. `hint` says what kind of notes they
/// are when that helps ("AI sessions that may continue one piece of work").
pub fn topic(
    judge: &Judge,
    group: &[&Note],
    hint: &str,
    used: &Mutex<std::collections::HashSet<String>>,
) -> Option<Topic> {
    let key = key("topic", group);
    let mut user = String::new();
    for (i, note) in group.iter().enumerate() {
        user.push_str(&block(&format!("n{}", i + 1), note, 4000, ""));
    }
    let system = judge.prompt_with(topic_skill(hint), DRAWING);
    let answer = judge.verdict(used, &key, &system, &user, &topic_schema())?;
    let mut members: Vec<usize> = strings(&answer["members"])
        .iter()
        .filter_map(|id| id_index(id))
        .filter(|i| *i < group.len())
        .collect();
    members.sort();
    members.dedup();
    let title = fsx::safe_stem(
        answer["title"]
            .as_str()
            .unwrap_or("")
            .trim()
            .trim_start_matches('#')
            .trim(),
    );
    let body = answer["body"]
        .as_str()
        .unwrap_or("")
        .replace("\r\n", "\n")
        .trim()
        .to_owned();
    if members.len() < 2 || title.is_empty() || title == "Untitled" || body.is_empty() {
        return None;
    }
    Some(Topic {
        members,
        title,
        summary: answer["summary"].as_str().unwrap_or("").trim().to_owned(),
        body,
        reason: answer["reason"].as_str().unwrap_or("").trim().to_owned(),
        diagram: drafted(&answer["diagram"], &answer["diagram_caption"]),
    })
}

// ---- Archive ----

fn archive_schema() -> Value {
    json!({
        "type": "object", "additionalProperties": false, "required": ["decisions"],
        "properties": {"decisions": {"type": "array", "items": {
            "type": "object", "additionalProperties": false, "required": ["id", "archive", "reason"],
            "properties": {"id": {"type": "string"}, "archive": {"type": "boolean"}, "reason": {"type": "string"}}
        }}}
    })
}

/// For a batch of archive candidates (each with why the rules picked it): which to archive,
/// and why, by index into `batch`.
pub fn archive(
    judge: &Judge,
    batch: &[(&Note, String)],
    used: &Mutex<std::collections::HashSet<String>>,
) -> Vec<(usize, String)> {
    let notes: Vec<&Note> = batch.iter().map(|(n, _)| *n).collect();
    let key = key("archive", &notes);
    let mut user = String::from("Notes that could be archived:\n\n");
    for (i, (note, why)) in batch.iter().enumerate() {
        user.push_str(&block(
            &format!("n{}", i + 1),
            note,
            1500,
            &format!(" picked_because=\"{}\"", attribute(why)),
        ));
    }
    let system = judge.prompt("archive");
    let Some(answer) = judge.verdict(used, &key, &system, &user, &archive_schema()) else {
        return Vec::new();
    };
    answer["decisions"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|d| d["archive"].as_bool() == Some(true))
        .filter_map(|d| {
            let at = d["id"]
                .as_str()
                .and_then(id_index)
                .filter(|i| *i < batch.len())?;
            Some((at, d["reason"].as_str().unwrap_or("").trim().to_owned()))
        })
        .collect()
}

// ---- Links ----

/// A link that points nowhere, and the notes it might have meant.
pub struct Broken<'n> {
    pub note: &'n Note,
    pub target: String,
    /// The line the link is on.
    pub context: String,
    /// Names of notes it might mean.
    pub options: Vec<String>,
}

fn link_schema() -> Value {
    json!({
        "type": "object", "additionalProperties": false, "required": ["fixes"],
        "properties": {"fixes": {"type": "array", "items": {
            "type": "object", "additionalProperties": false, "required": ["id", "target", "reason"],
            "properties": {"id": {"type": "string"}, "target": {"anyOf": [{"type": "string"}, {"type": "null"}]}, "reason": {"type": "string"}}
        }}}
    })
}

/// For a batch of broken links: the option each one meant, by index into `batch`.
pub fn links(
    judge: &Judge,
    batch: &[Broken],
    used: &Mutex<std::collections::HashSet<String>>,
) -> Vec<(usize, String, String)> {
    let notes: Vec<&Note> = batch.iter().map(|b| b.note).collect();
    let key = format!(
        "{}:{:x}",
        key("links", &notes),
        text::fnv(
            batch
                .iter()
                .map(|b| format!("{}>{}", b.target, b.options.join(",")))
                .collect::<Vec<_>>()
                .join("|")
                .as_bytes()
        )
    );
    let mut user = String::new();
    for (i, broken) in batch.iter().enumerate() {
        user.push_str(&format!(
            "<link id=\"l{}\" in_note=\"{}\" target=\"{}\">\nline: {}\noptions: {}\n</link>\n",
            i + 1,
            attribute(&broken.note.title),
            attribute(&broken.target),
            escape(&broken.context),
            broken
                .options
                .iter()
                .map(|o| format!("\"{}\"", attribute(o)))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    let system = judge.prompt("fix-link");
    let Some(answer) = judge.verdict(used, &key, &system, &user, &link_schema()) else {
        return Vec::new();
    };
    answer["fixes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|f| {
            let at = f["id"]
                .as_str()?
                .trim()
                .trim_start_matches('l')
                .parse::<usize>()
                .ok()?
                .checked_sub(1)?;
            let broken = batch.get(at)?;
            let target = f["target"].as_str()?.trim();
            let option = broken
                .options
                .iter()
                .find(|o| o.eq_ignore_ascii_case(target))?;
            Some((
                at,
                option.clone(),
                f["reason"].as_str().unwrap_or("").trim().to_owned(),
            ))
        })
        .collect()
}

// ---- Settling ----

/// A fresh memory and the places it could settle.
pub struct Unsettled<'n> {
    pub note: &'n Note,
    /// Home notes, by name.
    pub homes: Vec<String>,
    /// Folders, as `work/db/`.
    pub folders: Vec<String>,
}

fn settle_schema() -> Value {
    json!({
        "type": "object", "additionalProperties": false, "required": ["placements"],
        "properties": {"placements": {"type": "array", "items": {
            "type": "object", "additionalProperties": false, "required": ["id", "kind", "name", "reason"],
            "properties": {
                "id": {"type": "string"}, "kind": {"type": "string", "enum": ["topic", "folder", "none"]},
                "name": {"type": "string"}, "reason": {"type": "string"}
            }
        }}}
    })
}

/// Where each fresh memory in `batch` belongs: `(index, is_home, name, reason)`.
pub fn homes(
    judge: &Judge,
    batch: &[Unsettled],
    used: &Mutex<std::collections::HashSet<String>>,
) -> Vec<(usize, bool, String, String)> {
    let notes: Vec<&Note> = batch.iter().map(|u| u.note).collect();
    let key = format!(
        "{}:{:x}",
        key("settle", &notes),
        text::fnv(
            batch
                .iter()
                .map(|u| format!("{}|{}", u.homes.join(","), u.folders.join(",")))
                .collect::<Vec<_>>()
                .join(";")
                .as_bytes()
        )
    );
    let mut user = String::new();
    for (i, item) in batch.iter().enumerate() {
        user.push_str(&block(&format!("n{}", i + 1), item.note, 1500, ""));
        user.push_str(&format!(
            "Home notes for n{}: {}\nFolders for n{}: {}\n\n",
            i + 1,
            if item.homes.is_empty() {
                "(none)".into()
            } else {
                item.homes
                    .iter()
                    .map(|h| format!("\"{}\"", attribute(h)))
                    .collect::<Vec<_>>()
                    .join(", ")
            },
            i + 1,
            if item.folders.is_empty() {
                "(none)".into()
            } else {
                item.folders.join(", ")
            },
        ));
    }
    let system = judge.prompt("settle");
    let Some(answer) = judge.verdict(used, &key, &system, &user, &settle_schema()) else {
        return Vec::new();
    };
    answer["placements"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| {
            let at = p["id"]
                .as_str()
                .and_then(id_index)
                .filter(|i| *i < batch.len())?;
            let name = p["name"].as_str()?.trim();
            let reason = p["reason"].as_str().unwrap_or("").trim().to_owned();
            match p["kind"].as_str()? {
                "topic" => batch[at]
                    .homes
                    .iter()
                    .find(|h| h.eq_ignore_ascii_case(name))
                    .map(|h| (at, true, h.clone(), reason)),
                "folder" => {
                    let wanted = format!("{}/", name.trim_matches('/'));
                    batch[at]
                        .folders
                        .iter()
                        .find(|f| f.eq_ignore_ascii_case(&wanted))
                        .map(|f| (at, false, f.clone(), reason))
                }
                _ => None,
            }
        })
        .collect()
}
