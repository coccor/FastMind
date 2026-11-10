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
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

type Live = Arc<Mutex<(Option<Instant>, remember::LiveIndex)>>;

pub struct App {
    settings: Mutex<Settings>,
    index: Live,
    /// Captures a model is writing right now, by capture id.
    writing: Arc<Mutex<HashSet<String>>>,
    /// Set while Dream runs, so a second run waits its turn.
    dreaming: Mutex<()>,
    persist: bool,
}

/// How long a search index is reused before the files are read again.
const INDEX_FRESH: Duration = Duration::from_secs(1);

/// One capture through the writing step, with the index behind `live`. Marks it busy while the
/// model works, so the inbox can say so.
fn write_with(
    nb: &Notebook,
    live: &Live,
    writing: &Arc<Mutex<HashSet<String>>>,
    settings: &Settings,
    id: &str,
    force: bool,
) -> Result<crate::write::Outcome> {
    let model =
        Model::from_settings(&settings.model).ok_or_else(|| Error::msg("no model is chosen"))?;
    writing.lock().expect("writing lock").insert(id.to_owned());
    let with_index = |use_it: &mut dyn FnMut(&Index, &[crate::Note])| {
        let mut guard = live.lock().expect("index lock");
        let (checked, index) = &mut *guard;
        *checked = Some(Instant::now());
        index.refresh(nb);
        let notes = index.notes();
        let built = index.refresh_if_built(nb);
        use_it(built, &notes);
    };
    let schema = crate::write::schema();
    let ask = |system: &str, user: &str, images: &[crate::model::Image]| {
        model.complete_json(system, user, images, &schema)
    };
    let result = crate::write::write_capture(
        nb,
        id,
        &settings.writing,
        &settings.inbox,
        &settings.ai.folder,
        &settings.model.id(),
        &with_index,
        &|url| crate::capture::page_text(url),
        &ask,
        force,
    );
    writing.lock().expect("writing lock").remove(id);
    result
}

fn arg<T: for<'de> Deserialize<'de>>(args: &Value, key: &str) -> Result<T> {
    serde_json::from_value(args.get(key).cloned().unwrap_or(Value::Null))
        .map_err(|_| Error::msg(format!("missing or bad argument: {key}")))
}

impl App {
    /// An app that saves settings changes to the settings file.
    pub fn new(settings: Settings) -> Self {
        Self {
            settings: Mutex::new(settings),
            index: Arc::new(Mutex::new((None, remember::LiveIndex::default()))),
            writing: Arc::new(Mutex::new(HashSet::new())),
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

    /// The chosen notebook. Before one is chosen there's none: FastMind never makes a folder
    /// the user didn't pick.
    pub fn notebook(&self) -> Result<Notebook> {
        match self.settings().notebook {
            Some(path) => Notebook::open(path),
            None => Err(Error::msg(
                "no notebook chosen yet: open FastMind to choose one, or run `fastmind setup <folder>`",
            )),
        }
    }

    fn model(&self) -> Option<Model> {
        Model::from_settings(&self.settings().model)
    }

    pub fn invalidate(&self) {
        self.index.lock().expect("index lock").0 = None;
    }

    /// The search index, refreshed from the files at most every second (only changed notes are
    /// read again), or right away after FastMind itself changed something.
    fn with_index<T>(&self, nb: &Notebook, use_it: impl FnOnce(&Index) -> T) -> T {
        let mut guard = self.index.lock().expect("index lock");
        let (checked, live) = &mut *guard;
        let index = if checked.is_none_or(|at| at.elapsed() > INDEX_FRESH) {
            *checked = Some(Instant::now());
            live.refresh(nb)
        } else {
            live.refresh_if_built(nb)
        };
        use_it(index)
    }

    /// Every note, from the same cache as search: only notes changed since are read again.
    fn notes(&self, nb: &Notebook) -> Vec<crate::Note> {
        let mut guard = self.index.lock().expect("index lock");
        let (checked, live) = &mut *guard;
        if checked.is_none_or(|at| at.elapsed() > INDEX_FRESH) {
            *checked = Some(Instant::now());
            live.refresh(nb);
        } else {
            live.refresh_if_built(nb);
        }
        live.notes()
    }

    pub fn call(&self, name: &str, args: Value) -> Result<Value> {
        // These don't need the notebook, so they never create its folder.
        if matches!(
            name,
            "settings_get" | "settings_set" | "demo_create" | "first_run" | "setup" | "agent_setup"
        ) {
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
                // A link gets its page's title and description in the background: saving
                // never waits on the network.
                let link = request
                    .url
                    .clone()
                    .filter(|u| !u.trim().is_empty())
                    .or_else(|| {
                        capture::is_url(request.text.trim()).then(|| request.text.trim().to_owned())
                    });
                let settings = self.settings();
                let writes = settings.model.is_on() && settings.writing.enabled;
                if let (true, Some(id)) = (writes, saved.capture_id.clone()) {
                    // The model reads the page itself, so no separate title fetch.
                    if request.wait {
                        let outcome = self.write_capture(&nb, &id, false);
                        let mut out = json!(saved);
                        match outcome {
                            Ok(outcome) => {
                                out["path"] = json!(outcome.path);
                                out["title"] = json!(outcome.title);
                                out["written_by"] = json!(settings.model.id());
                            }
                            Err(error) => out["write_error"] = json!(error.to_string()),
                        }
                        return Ok(out);
                    }
                    self.write_in_background(&nb, vec![id]);
                    let mut out = json!(saved);
                    out["writing"] = json!(true);
                    return Ok(out);
                } else if let (Some(url), true) = (link, settings.fetch_link_titles) {
                    let (nb, saved) = (nb.clone(), saved.clone());
                    std::thread::spawn(move || {
                        let _ = capture::enrich_link(&nb, &saved, &url);
                    });
                }
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
                let capture_id = note.front.text("capture_id").map(str::to_owned);
                let kept = capture_id
                    .as_deref()
                    .is_some_and(|id| crate::capture::Raw::load(&nb, id).is_ok());
                let writing = capture_id
                    .as_deref()
                    .is_some_and(|id| self.writing.lock().expect("writing lock").contains(id));
                json!({
                    "path": note.path, "title": note.title, "text": note.text, "abs_path": nb.path(&path)?,
                    "capture_id": capture_id.filter(|_| kept), "written_by": note.front.text("written_by"),
                    "writing": writing,
                })
            }
            "open" => {
                let path: String = arg(&args, "path")?;
                Usage::record_open(&nb, &path)?;
                json!({ "abs_path": nb.path(&path)? })
            }
            "inbox" => json!(self.inbox(&nb)),
            "rewrite_note" => {
                // Write (or write again) the note from the capture it was made from. Asked for
                // by the user, so it replaces the note even if edited: History undoes it.
                let path: String = arg(&args, "path")?;
                let id = self.capture_id_of(&nb, &path)?;
                if !self.settings().model.is_on() {
                    return Err(Error::msg("choose a model in Settings to write notes"));
                }
                // The note may have been filed since: write it where it is now.
                let mut raw = crate::capture::Raw::load(&nb, &id)?;
                if raw.note != path {
                    raw.note = path.clone();
                    raw.save(&nb)?;
                }
                let outcome = self.write_capture(&nb, &id, true)?;
                json!(outcome)
            }
            "write_pending" => {
                let ids = if self.settings().model.is_on() && self.settings().writing.enabled {
                    crate::write::pending(&nb, 7)
                } else {
                    Vec::new()
                };
                let count = ids.len();
                self.write_in_background(&nb, ids);
                json!({ "started": count })
            }
            "capture_original" => {
                let path: String = arg(&args, "path")?;
                let id = self.capture_id_of(&nb, &path)?;
                let raw = crate::capture::Raw::load(&nb, &id)?;
                json!({
                    "captured": raw.captured, "text": raw.text, "url": raw.url, "tags": raw.tags,
                    "attachments": raw.attachments, "written_by": raw.written_by, "failed": raw.failed,
                    "writing": self.writing.lock().expect("writing lock").contains(&id),
                })
            }
            "home" => self.home(&nb, now),
            "image" => {
                // An image inside the notebook, for showing in a note. Nothing else is served.
                let path: String = arg(&args, "path")?;
                let ext = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
                let mime = match ext.as_str() {
                    "png" => "image/png",
                    "jpg" | "jpeg" => "image/jpeg",
                    "gif" => "image/gif",
                    "webp" => "image/webp",
                    "bmp" => "image/bmp",
                    "svg" => "image/svg+xml",
                    _ => return Err(Error::msg("not an image")),
                };
                let file = nb.path(&path)?;
                let size = std::fs::metadata(&file).map(|m| m.len()).unwrap_or(0);
                if size > 15 * 1024 * 1024 {
                    return Err(Error::msg("image too large to show"));
                }
                let bytes = std::fs::read(&file)?;
                let data =
                    base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes);
                json!({ "data": format!("data:{mime};base64,{data}") })
            }
            "suggest_homes" => {
                let path: String = arg(&args, "path")?;
                let settings = self.settings();
                let mut homes = self.with_index(&nb, |index| {
                    crate::filing::suggest(index, &path, &settings.inbox, &settings.ai.folder)
                });
                if let Some(home) = self.written_home(&nb, &path) {
                    homes.retain(|h| !(h.kind == home.kind && h.target == home.target));
                    homes.insert(0, home);
                }
                json!(homes)
            }
            "folders" => json!(crate::filing::folders(&self.notes(&nb))),
            "file_note" => {
                let path: String = arg(&args, "path")?;
                let folder: Option<String> = arg(&args, "folder").unwrap_or(None);
                let home: Option<String> = arg(&args, "home").unwrap_or(None);
                let filed = crate::filing::file(&nb, &path, folder.as_deref(), home.as_deref())?;
                self.invalidate();
                json!({ "path": filed })
            }
            "related" => {
                let path: String = arg(&args, "path")?;
                let notes = self.notes(&nb);
                let inbound = crate::notebook::backlinks(&notes);
                let linked_from: Vec<Value> = inbound
                    .get(&path)
                    .into_iter()
                    .flatten()
                    .filter_map(|from| notes.iter().find(|n| &n.path == from))
                    .map(|n| json!({ "path": n.path, "title": n.title }))
                    .collect();
                let similar = self.with_index(&nb, |index| index.similar(&path, 6));
                json!({ "linked_from": linked_from, "similar": similar })
            }
            "write_note" => {
                // Saves an edit made in the app. `base` is the text the edit started from: if the
                // file changed since (another editor, a sync), nothing is overwritten.
                let path: String = arg(&args, "path")?;
                let new_text: String = arg(&args, "text")?;
                let base: Option<String> = arg(&args, "base").unwrap_or(None);
                let current = std::fs::read_to_string(nb.path(&path)?).unwrap_or_default();
                if base.as_deref().is_some_and(|base| base != current) {
                    return Err(Error::msg(
                        "the note changed on disk since you opened it; reopen it to see the latest",
                    ));
                }
                let title =
                    crate::notebook::build_note(path.clone(), current.clone(), 0, now).title;
                let mut tx = Transaction::begin(&nb, format!("Edit \"{title}\""), "edit")?;
                tx.write(&path, new_text.as_bytes())?;
                let entry = tx.commit()?;
                self.invalidate();
                json!({ "entry": entry.id })
            }
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
                // One Dream change at a time: a double click can't apply a proposal twice.
                let _one_at_a_time = self.dreaming.lock().expect("dream lock");
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
                let _one_at_a_time = self.dreaming.lock().expect("dream lock");
                let names: Vec<String> = arg(&args, "kinds").unwrap_or_default();
                let kinds: Vec<Kind> = names.iter().filter_map(|name| Kind::parse(name)).collect();
                let done = dream::apply_all(&nb, &kinds, now)?;
                self.invalidate();
                json!(done)
            }
            "dream_undo" => {
                // One Dream change at a time: a double click can't apply a proposal twice.
                let _one_at_a_time = self.dreaming.lock().expect("dream lock");
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
            "agent_setup" => {
                // The `fastmind` command beside this program, for coding agents to start.
                let name = if cfg!(windows) {
                    "fastmind.exe"
                } else {
                    "fastmind"
                };
                let beside = std::env::current_exe()
                    .ok()
                    .map(|exe| exe.with_file_name(name))
                    .filter(|p| p.exists());
                let command = beside
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "fastmind".into());
                let quoted = if command.contains(' ') {
                    format!("\"{command}\"")
                } else {
                    command.clone()
                };
                json!({
                    "command": command,
                    "claude_code": format!("claude mcp add --scope user fastmind -- {quoted} mcp"),
                    "codex": format!("[mcp_servers.fastmind]\ncommand = {}\nargs = [\"mcp\"]", serde_json::to_string(&command)?),
                })
            }
            "first_run" => {
                let settings = self.settings();
                let transcripts =
                    crate::sessions::discover(&crate::sessions::AiSettings::default());
                json!({
                    "needs_setup": settings.notebook.is_none(),
                    "candidates": crate::setup::candidates(),
                    "ai_sessions": transcripts.len(),
                    "hotkeys": settings.hotkeys,
                })
            }
            "setup" => {
                let notebook: PathBuf = arg(&args, "notebook")?;
                let ai: bool = arg(&args, "ai").unwrap_or(false);
                std::fs::create_dir_all(&notebook)?;
                let mut settings = self.settings();
                settings.notebook = Some(notebook);
                settings.ai.enabled = ai;
                self.replace_settings(settings)?;
                json!({ "ok": true })
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
        let notes = self.notes(nb);
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
            "model_on": settings.model.is_on(),
            "writes": settings.model.is_on() && settings.writing.enabled,
            "writing": self.writing.lock().expect("writing lock").len(),
            "hotkeys": settings.hotkeys,
        })
    }

    /// The morning page: what Dream did and proposes, what's waiting, what's recent, and two
    /// older notes brought back so they aren't forgotten.
    fn home(&self, nb: &Notebook, now: chrono::DateTime<Local>) -> Value {
        let notes = self.notes(nb);
        let latest = dream::latest(nb);
        let dream = latest.as_ref().map(|report| {
            let applied: Vec<&dream::Proposal> =
                report.proposals.iter().filter(|p| p.status == dream::Status::Applied).collect();
            json!({
                "ran_at": report.ran_at,
                "summary": report.summary,
                "pending": report.pending().count(),
                "applied": applied.len(),
                "before": report.before.notes,
                "after": report.before.notes.saturating_sub(report.proposals.iter().filter(|p| p.status != dream::Status::Rejected).map(|p| p.notes_removed).sum::<usize>()),
                "kinds": report.pending().fold(std::collections::BTreeMap::<String, usize>::new(), |mut acc, p| {
                    *acc.entry(format!("{:?}", p.kind).to_lowercase()).or_default() += 1;
                    acc
                }),
            })
        });
        let inbox = self.inbox(nb);
        let mut recent: Vec<&crate::Note> =
            notes.iter().filter(|n| n.status() != "inbox").collect();
        recent.sort_by_key(|n| std::cmp::Reverse(n.touched));
        let card = |n: &crate::Note| {
            json!({
                "path": n.path, "title": n.title, "kind": n.note_type(), "touched": n.touched,
                "snippet": text::excerpt(&n.body().replace(&format!("# {}", n.title), ""), 160),
            })
        };
        // Brought back: notes over a month old with something in them, not records or homes,
        // picked by the date so the choice holds all day and changes tomorrow.
        let usage = Usage::load(nb);
        let mut old: Vec<&crate::Note> = notes
            .iter()
            .filter(|n| (now - n.touched).num_days() >= 30)
            .filter(|n| n.word_count() >= 25 && !n.is_record() && !n.is_home() && !n.is_template())
            .filter(|n| {
                usage
                    .last_opened(&n.path)
                    .is_none_or(|at| (now - at).num_days() >= 30)
            })
            .collect();
        old.sort_by_key(|n| n.path.clone());
        let day = now.date_naive().to_string();
        let mut resurfaced: Vec<Value> = Vec::new();
        if !old.is_empty() {
            let seed = text::fnv(day.as_bytes()) as usize;
            for k in 0..old.len().min(2) {
                let note = old[(seed + k * 7919) % old.len()];
                if !resurfaced.iter().any(|r| r["path"] == json!(note.path)) {
                    let mut value = card(note);
                    value["age_days"] = json!((now - note.touched).num_days());
                    resurfaced.push(value);
                }
            }
        }
        let ai_week = notes
            .iter()
            .filter(|n| {
                n.note_type() == "ai-session"
                    && (now - n.created.unwrap_or(n.touched)).num_days() < 7
            })
            .count();
        json!({
            "notes": notes.len(),
            "dream": dream,
            "inbox_count": inbox.len(),
            "inbox": inbox.into_iter().take(3).collect::<Vec<_>>(),
            "recent": recent.iter().take(6).map(|n| card(n)).collect::<Vec<_>>(),
            "resurfaced": resurfaced,
            "ai_week": ai_week,
            "ai_enabled": self.settings().ai.enabled,
        })
    }

    /// The capture a note was made from.
    fn capture_id_of(&self, nb: &Notebook, path: &str) -> Result<String> {
        let note = nb.read(path)?;
        note.front
            .text("capture_id")
            .map(str::to_owned)
            .ok_or_else(|| {
                Error::msg(
                    "this note wasn't captured with FastMind, so there's nothing to write it from",
                )
            })
    }

    /// The home the model suggested when it wrote the note, as a filing choice.
    fn written_home(&self, nb: &Notebook, path: &str) -> Option<crate::filing::Home> {
        let note = nb.read(path).ok()?;
        let home = note.front.text("suggested_home")?.trim().to_owned();
        let because: Vec<String> = note
            .front
            .text("suggested_because")
            .map(|b| vec![b.to_owned()])
            .unwrap_or_default();
        if let Some(name) = home.strip_prefix("[[").and_then(|h| h.strip_suffix("]]")) {
            let target = self.notes(nb).into_iter().find(|n| n.stem() == name)?.path;
            Some(crate::filing::Home {
                kind: "note".into(),
                label: name.to_owned(),
                target,
                because,
                score: f32::MAX,
            })
        } else {
            let folder = home.trim_matches('/').to_owned();
            nb.path(&folder).ok().filter(|p| p.is_dir())?;
            Some(crate::filing::Home {
                kind: "folder".into(),
                label: format!("{folder}/"),
                target: folder,
                because,
                score: f32::MAX,
            })
        }
    }

    /// Writes the note for capture `id` now, with the chosen model.
    pub fn write_capture(
        &self,
        nb: &Notebook,
        id: &str,
        force: bool,
    ) -> Result<crate::write::Outcome> {
        let result = write_with(nb, &self.index, &self.writing, &self.settings(), id, force);
        self.invalidate();
        result
    }

    /// Writes the notes for `ids` one after another on a thread of their own.
    fn write_in_background(&self, nb: &Notebook, ids: Vec<String>) {
        if ids.is_empty() {
            return;
        }
        let (nb, live, writing, settings) = (
            nb.clone(),
            self.index.clone(),
            self.writing.clone(),
            self.settings(),
        );
        {
            let mut busy = writing.lock().expect("writing lock");
            busy.extend(ids.iter().cloned());
        }
        std::thread::spawn(move || {
            for id in ids {
                let _ = write_with(&nb, &live, &writing, &settings, &id, false);
                live.lock().expect("index lock").0 = None;
            }
        });
    }

    fn inbox(&self, nb: &Notebook) -> Vec<Value> {
        let inbox = self.settings().inbox;
        let model_on = self.settings().model.is_on();
        let mut notes: Vec<_> = self
            .notes(nb)
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
                let id = note.front.text("capture_id").map(str::to_owned);
                let raw = id
                    .as_deref()
                    .and_then(|id| crate::capture::Raw::load(nb, id).ok());
                let writing = id
                    .as_deref()
                    .is_some_and(|id| self.writing.lock().expect("writing lock").contains(id));
                let summary = note.front.text("summary").map(str::to_owned);
                let snippet = summary.unwrap_or_else(|| {
                    text::excerpt(&note.body().replace(&format!("# {}", note.title), ""), 200)
                });
                json!({
                    "path": note.path,
                    "title": note.title,
                    "captured": note.created.unwrap_or(note.touched),
                    "snippet": snippet,
                    "tags": note.front.tags(),
                    "writing": writing,
                    "written_by": note.front.text("written_by"),
                    "write_failed": raw.as_ref().and_then(|r| r.failed.clone()),
                    "can_write": raw.is_some() && model_on,
                })
            })
            .collect()
    }

    fn recent(&self, nb: &Notebook, limit: usize) -> Vec<Value> {
        let mut notes = self.notes(nb);
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
