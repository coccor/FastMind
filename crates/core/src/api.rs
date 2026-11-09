//! One command surface for every front end. The desktop app, the CLI's `serve` mode and tests
//! all call `App::call(name, args)` with JSON in and JSON out, so they can't drift apart.

use crate::archive::{self, Index as ArchiveIndex};
use crate::capture::{self, Capture};
use crate::config::Settings;
use crate::dream::{self, Kind};
use crate::frontmatter::{self, Value as FmValue};
use crate::journal::{self, Transaction};
use crate::model::Model;
use crate::notebook::Notebook;
use crate::search::Index;
use crate::usage::Usage;
use crate::{Error, Result, demo, remember, text};
use chrono::Local;
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub struct App {
    settings: Mutex<Settings>,
    index: Mutex<Option<(PathBuf, Instant, Index)>>,
    /// Set while Dream runs, so a second run waits its turn.
    dreaming: Mutex<()>,
    persist: bool,
}

/// How long a search index is reused before the files are read again.
const INDEX_FRESH: Duration = Duration::from_secs(5);

fn arg<T: for<'de> Deserialize<'de>>(args: &Value, key: &str) -> Result<T> {
    serde_json::from_value(args.get(key).cloned().unwrap_or(Value::Null))
        .map_err(|_| Error::msg(format!("missing or bad argument: {key}")))
}

impl App {
    /// An app that saves settings changes to the settings file.
    pub fn new(settings: Settings) -> Self {
        Self {
            settings: Mutex::new(settings),
            index: Mutex::new(None),
            dreaming: Mutex::new(()),
            persist: true,
        }
    }

    /// An app whose settings live only in memory (tests, one-off CLI runs).
    pub fn ephemeral(settings: Settings) -> Self {
        Self {
            persist: false,
            ..Self::new(settings)
        }
    }

    pub fn settings(&self) -> Settings {
        self.settings.lock().expect("settings lock").clone()
    }

    pub fn notebook(&self) -> Result<Notebook> {
        Notebook::open(self.settings().notebook_or_default())
    }

    fn model(&self) -> Option<Model> {
        Model::from_settings(&self.settings().model)
    }

    pub fn invalidate(&self) {
        *self.index.lock().expect("index lock") = None;
    }

    fn with_index<T>(&self, nb: &Notebook, use_it: impl FnOnce(&Index) -> T) -> T {
        let mut guard = self.index.lock().expect("index lock");
        let stale = match &*guard {
            Some((root, built, _)) => *root != nb.root || built.elapsed() > INDEX_FRESH,
            None => true,
        };
        if stale {
            *guard = Some((nb.root.clone(), Instant::now(), remember::build_index(nb)));
        }
        use_it(&guard.as_ref().expect("built").2)
    }

    pub fn call(&self, name: &str, args: Value) -> Result<Value> {
        // These don't need the notebook, so they never create its folder.
        if matches!(name, "settings_get" | "settings_set" | "demo_create") {
            return self.call_without_notebook(name, args);
        }
        let nb = self.notebook()?;
        let now = Local::now();
        Ok(match name {
            "status" => self.status(&nb),
            "capture" => {
                let request: Capture =
                    serde_json::from_value(args).map_err(|e| Error::msg(e.to_string()))?;
                let saved = capture::save(&nb, &self.settings().inbox, &request)?;
                self.invalidate();
                json!(saved)
            }
            "search" => {
                let query: String = arg(&args, "query")?;
                let limit: usize = arg(&args, "limit").unwrap_or(20);
                if query.trim().is_empty() {
                    return Ok(json!(self.recent(&nb, limit)));
                }
                json!(self.with_index(&nb, |index| remember::search(&nb, index, &query, limit)))
            }
            "ask" => {
                let question: String = arg(&args, "question")?;
                let model = self.model();
                json!(self.with_index(&nb, |index| remember::ask(
                    &nb,
                    index,
                    &question,
                    model.as_ref()
                )))
            }
            "read" => {
                let path: String = arg(&args, "path")?;
                let note = nb.read(&path)?;
                json!({ "path": note.path, "title": note.title, "text": note.text, "abs_path": nb.path(&path)? })
            }
            "open" => {
                let path: String = arg(&args, "path")?;
                Usage::record_open(&nb, &path)?;
                json!({ "abs_path": nb.path(&path)? })
            }
            "inbox" => json!(self.inbox(&nb)),
            "ai_status" => {
                let settings = self.settings().ai;
                let transcripts = crate::sessions::discover(&crate::sessions::AiSettings {
                    sources: vec!["claude_code".into(), "codex".into()],
                    ..settings.clone()
                });
                let count =
                    |source: &str| transcripts.iter().filter(|t| t.source == source).count();
                json!({
                    "enabled": settings.enabled,
                    "claude_code": count("claude_code"),
                    "codex": count("codex"),
                    "folder": settings.folder,
                })
            }
            "ai_import" => {
                let settings = self.settings();
                let model = self.model();
                let report = crate::sessions::import(
                    &nb,
                    &settings.ai,
                    model.as_ref(),
                    settings.dream.model_requests,
                    now,
                )?;
                self.invalidate();
                json!(report)
            }
            "pin_note" => {
                // Keep forever: Dream never merges, gathers or archives a pinned note.
                let path: String = arg(&args, "path")?;
                let note = nb.read(&path)?;
                let changed = frontmatter::set(&note.text, "pin", &FmValue::Text("true".into()));
                let mut tx =
                    Transaction::begin(&nb, format!("Keep \"{}\" forever", note.title), "dream")?;
                tx.write(&path, changed.as_bytes())?;
                let entry = tx.commit()?;
                // Proposals about it are settled: mark them skipped.
                if let Some(report) = dream::latest(&nb) {
                    for proposal in report.pending().filter(|p| p.notes.contains(&path)) {
                        dream::reject(&nb, &proposal.id)?;
                    }
                }
                self.invalidate();
                json!({ "entry": entry.id })
            }
            "dream_forget_skips" => {
                dream::forget_skips(&nb)?;
                json!({ "ok": true })
            }
            "inbox_keep" => {
                let path: String = arg(&args, "path")?;
                let note = nb.read(&path)?;
                let changed = frontmatter::set(&note.text, "status", &FmValue::Text("kept".into()));
                let mut tx = Transaction::begin(&nb, format!("Keep \"{}\"", note.title), "inbox")?;
                tx.write(&path, changed.as_bytes())?;
                let entry = tx.commit()?;
                self.invalidate();
                json!({ "entry": entry.id })
            }
            "archive_note" => {
                let path: String = arg(&args, "path")?;
                let note = nb.read(&path)?;
                let mut index = ArchiveIndex::load(&nb);
                let mut tx =
                    Transaction::begin(&nb, format!("Archive \"{}\"", note.title), "archive")?;
                archive::archive(&mut tx, &mut index, &path, &note.title, "archived by hand")?;
                index.save(&mut tx)?;
                let entry = tx.commit()?;
                self.invalidate();
                json!({ "entry": entry.id })
            }
            "archive_list" => {
                let mut items = ArchiveIndex::load(&nb).items;
                items.sort_by_key(|item| std::cmp::Reverse(item.archived_at));
                json!({ "items": items, "disk_bytes": archive::disk_size(&nb) })
            }
            "archive_restore" => {
                let id: String = arg(&args, "id")?;
                let path = archive::restore(&nb, &id)?;
                self.invalidate();
                json!({ "path": path })
            }
            "dream_run" => {
                let _running = self.dreaming.lock().expect("dream lock");
                let settings = self.settings();
                let model = self.model();
                // Fresh AI sessions first, so tonight's Dream compresses them too.
                if settings.ai.enabled {
                    crate::sessions::import(
                        &nb,
                        &settings.ai,
                        model.as_ref(),
                        settings.dream.model_requests,
                        now,
                    )?;
                }
                dream::run(&nb, &settings.dream, model.as_ref(), now)?;
                let auto = dream::auto_apply(&nb, &settings.dream, now)?;
                self.invalidate();
                let report = dream::latest(&nb);
                json!({ "report": report, "auto_applied": auto.len() })
            }
            "dream_latest" => json!(dream::latest(&nb)),
            "dream_apply" => {
                let id: String = arg(&args, "id")?;
                let proposal = dream::apply(&nb, &id, now)?;
                self.invalidate();
                json!(proposal)
            }
            "dream_reject" => {
                let id: String = arg(&args, "id")?;
                json!(dream::reject(&nb, &id)?)
            }
            "dream_apply_all" => {
                let names: Vec<String> = arg(&args, "kinds").unwrap_or_default();
                let kinds: Vec<Kind> = names.iter().filter_map(|name| Kind::parse(name)).collect();
                let done = dream::apply_all(&nb, &kinds, now)?;
                self.invalidate();
                json!(done)
            }
            "dream_undo" => {
                let id: String = arg(&args, "id")?;
                let proposal = dream::undo(&nb, &id)?;
                self.invalidate();
                json!(proposal)
            }
            "history" => {
                let entries: Vec<Value> = journal::history(&nb)
                    .into_iter()
                    .take(100)
                    .map(|entry| {
                        let conflicts = if entry.undone { Vec::new() } else { journal::conflicts(&nb, &entry) };
                        json!({
                            "id": entry.id, "at": entry.at, "label": entry.label, "source": entry.source,
                            "undone": entry.undone, "files": entry.paths(), "can_undo": !entry.undone && conflicts.is_empty(),
                        })
                    })
                    .collect();
                json!(entries)
            }
            "undo" => {
                let id: String = arg(&args, "id")?;
                let entry = journal::undo(&nb, &id)?;
                // A proposal undone from the history goes back to pending.
                if let Some(mut report) = dream::latest(&nb) {
                    let mut changed = false;
                    for proposal in &mut report.proposals {
                        if proposal.entry.as_deref() == Some(id.as_str()) {
                            proposal.entry = None;
                            proposal.status = dream::Status::Pending;
                            changed = true;
                        }
                    }
                    if changed {
                        let bytes = serde_json::to_vec_pretty(&report)?;
                        crate::fsx::write_atomic(
                            &nb.state_dir().join("dream").join("latest.json"),
                            &bytes,
                        )?;
                    }
                }
                self.invalidate();
                json!({ "undone": entry.label })
            }
            other => return Err(Error::msg(format!("unknown command: {other}"))),
        })
    }

    fn call_without_notebook(&self, name: &str, args: Value) -> Result<Value> {
        Ok(match name {
            "settings_get" => {
                json!({ "settings": self.settings(), "file": crate::config::config_file() })
            }
            "settings_set" => {
                let settings: Settings = arg(&args, "settings")?;
                self.replace_settings(settings)?;
                json!({ "ok": true })
            }
            "demo_create" => {
                let path: PathBuf = arg::<Option<PathBuf>>(&args, "path")?.unwrap_or_else(|| {
                    crate::config::default_notebook().with_file_name("FastMind Demo")
                });
                let report = demo::create(&path)?;
                let mut settings = self.settings();
                settings.notebook = Some(path.clone());
                self.replace_settings(settings)?;
                json!({ "path": path, "notes": report })
            }
            other => return Err(Error::msg(format!("unknown command: {other}"))),
        })
    }

    pub fn replace_settings(&self, settings: Settings) -> Result<()> {
        if self.persist {
            settings.save()?;
        }
        *self.settings.lock().expect("settings lock") = settings;
        self.invalidate();
        Ok(())
    }

    fn status(&self, nb: &Notebook) -> Value {
        let settings = self.settings();
        let notes = nb.notes();
        let inbox = notes.iter().filter(|note| note.status() == "inbox").count();
        let latest = dream::latest(nb);
        let pending = latest.as_ref().map_or(0, |report| report.pending().count());
        json!({
            "notebook": nb.root,
            "notes": notes.len(),
            "bytes": notes.iter().map(|n| n.size).sum::<u64>(),
            "inbox": inbox,
            "archived": ArchiveIndex::load(nb).items.len(),
            "dream": latest.map(|report| json!({
                "ran_at": report.ran_at, "summary": report.summary, "pending": pending,
            })),
            "model": settings.model.label(),
            "hotkeys": settings.hotkeys,
        })
    }

    fn inbox(&self, nb: &Notebook) -> Vec<Value> {
        let inbox = self.settings().inbox;
        let mut notes: Vec<_> = nb
            .notes()
            .into_iter()
            .filter(|note| {
                note.status() == "inbox" || (note.folder() == inbox && note.status().is_empty())
            })
            .collect();
        notes.sort_by(|a, b| {
            b.created
                .unwrap_or(b.touched)
                .cmp(&a.created.unwrap_or(a.touched))
        });
        notes
            .iter()
            .map(|note| {
                json!({
                    "path": note.path,
                    "title": note.title,
                    "captured": note.created.unwrap_or(note.touched),
                    "snippet": text::excerpt(&note.body().replace(&format!("# {}", note.title), ""), 200),
                    "tags": note.front.tags(),
                })
            })
            .collect()
    }

    fn recent(&self, nb: &Notebook, limit: usize) -> Vec<Value> {
        let mut notes = nb.notes();
        notes.sort_by_key(|note| std::cmp::Reverse(note.touched));
        notes
            .iter()
            .take(limit)
            .map(|note| {
                json!({
                    "path": note.path, "title": note.title, "score": 0.0,
                    "snippet": text::excerpt(&note.body().replace(&format!("# {}", note.title), ""), 180),
                    "line": 1, "archived": null, "tags": note.front.tags(), "kind": note.note_type(),
                })
            })
            .collect()
    }
}
