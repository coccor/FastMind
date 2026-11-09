//! Learn from AI sessions: the transcripts coding agents already keep on this computer (Claude
//! Code in `~/.claude/projects`, Codex in `~/.codex/sessions`) become notes — what was asked, how
//! it ended, which files changed, which commands ran. Secrets are taken out before anything is
//! written. Dream then compresses them like any other notes.
//!
//! Off until the user turns it on. Nothing leaves the computer unless a cloud model is chosen
//! to distil sessions.

use crate::frontmatter::{Value, quote, render};
use crate::model::{self, Model};
use crate::notebook::{Notebook, STATE_DIR};
use crate::{Result, fsx, text};
use chrono::{DateTime, Local};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AiSettings {
    /// Off until chosen: transcripts can hold anything.
    pub enabled: bool,
    /// `claude_code`, `codex`.
    pub sources: Vec<String>,
    /// Folder in the notebook for session notes.
    pub folder: String,
    /// A transcript untouched this long counts as finished.
    pub quiet_minutes: i64,
    /// On the first import, only sessions from the last this-many days.
    pub since_days: i64,
    /// Sessions shorter than this many prompts are left out.
    pub min_prompts: usize,
    /// Include sessions run by scripts and SDKs (headless `claude -p`, CI), not only ones a
    /// person had.
    pub include_automated: bool,
}

impl Default for AiSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            sources: vec!["claude_code".into(), "codex".into()],
            folder: "AI sessions".into(),
            quiet_minutes: 20,
            since_days: 30,
            min_prompts: 1,
            include_automated: false,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Session {
    pub source: String,
    pub id: String,
    pub cwd: String,
    pub branch: String,
    pub title: String,
    pub started: Option<DateTime<Local>>,
    pub ended: Option<DateTime<Local>>,
    pub prompts: Vec<String>,
    /// What the agent said, in order (its visible text, not its thinking).
    pub replies: Vec<String>,
    pub files: Vec<String>,
    pub commands: Vec<String>,
    /// How the session was started (`cli`, `claude-desktop`, `sdk-cli`…), when known.
    pub entrypoint: String,
}

impl Session {
    /// Run by a script or SDK rather than a person.
    pub fn is_automated(&self) -> bool {
        self.entrypoint.starts_with("sdk") || self.entrypoint == "headless"
    }

    pub fn project(&self) -> String {
        project_name(&self.cwd)
    }

    pub fn outcome(&self) -> &str {
        self.replies
            .iter()
            .rev()
            .find(|reply| text::words(reply).count() >= 8)
            .or(self.replies.last())
            .map(String::as_str)
            .unwrap_or("")
    }
}

/// The project a working folder belongs to: its last folder, seeing through agent worktrees
/// (`…/Payments/.claude/worktrees/fix-x` is Payments).
pub fn project_name(cwd: &str) -> String {
    let normal = cwd.replace('\\', "/");
    let base = ["/.claude/worktrees/", "/.worktrees/", "/worktrees/"]
        .iter()
        .find_map(|marker| normal.find(marker).map(|at| &normal[..at]))
        .unwrap_or(&normal);
    base.trim_end_matches('/')
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty() && !name.ends_with(':'))
        .unwrap_or("Other")
        .to_owned()
}

// ---- Finding transcripts ----

#[derive(Clone, Debug)]
pub struct Transcript {
    pub source: &'static str,
    pub path: PathBuf,
    pub size: u64,
    pub modified: DateTime<Local>,
}

fn home() -> Option<PathBuf> {
    std::env::var_os("FASTMIND_AI_HOME")
        .map(PathBuf::from)
        .or_else(|| directories::BaseDirs::new().map(|dirs| dirs.home_dir().to_path_buf()))
}

pub fn discover(settings: &AiSettings) -> Vec<Transcript> {
    let Some(home) = home() else {
        return Vec::new();
    };
    let mut found = Vec::new();
    let mut add = |source: &'static str, root: PathBuf| {
        let mut files = Vec::new();
        fsx::walk(&root, &|_| false, &mut files);
        for path in files {
            // Subagents' side conversations belong to their parent session.
            let side = path
                .components()
                .any(|part| part.as_os_str() == "subagents");
            if path.extension().is_some_and(|ext| ext == "jsonl")
                && !side
                && let Ok(meta) = fs::metadata(&path)
            {
                let modified = meta
                    .modified()
                    .map(DateTime::from)
                    .unwrap_or_else(|_| Local::now());
                found.push(Transcript {
                    source,
                    path,
                    size: meta.len(),
                    modified,
                });
            }
        }
    };
    if settings.sources.iter().any(|s| s == "claude_code") {
        add("claude_code", home.join(".claude").join("projects"));
    }
    if settings.sources.iter().any(|s| s == "codex") {
        add("codex", home.join(".codex").join("sessions"));
    }
    found.sort_by_key(|a| a.modified);
    found
}

pub fn read(transcript: &Transcript) -> Option<Session> {
    match transcript.source {
        "claude_code" => parse_claude(&transcript.path),
        "codex" => parse_codex(&transcript.path),
        _ => None,
    }
}

fn lines(path: &Path) -> impl Iterator<Item = Json> {
    let file = fs::File::open(path).ok();
    file.into_iter()
        .flat_map(|file| BufReader::new(file).lines().map_while(|line| line.ok()))
        .filter_map(|line| serde_json::from_str::<Json>(&line).ok())
}

fn time(value: &Json) -> Option<DateTime<Local>> {
    value
        .as_str()
        .and_then(|text| DateTime::parse_from_rfc3339(text).ok())
        .map(|t| t.with_timezone(&Local))
}

/// A prompt the person typed, without the harness's own tags and reminders.
fn clean_prompt(raw: &str) -> Option<String> {
    static TAGS: OnceLock<Regex> = OnceLock::new();
    let tags = TAGS.get_or_init(|| {
        Regex::new(r"(?s)<(system-reminder|local-command-[a-z]+|command-[a-z]+|ide_[a-z_]+|user-prompt-submit-hook)>.*?</[^>]+>").expect("valid")
    });
    let cleaned = tags.replace_all(raw, "");
    let cleaned = cleaned.trim();
    let recap = [
        "Background (from the session",
        "This session is being continued from a previous conversation",
    ];
    if cleaned.is_empty()
        || cleaned.starts_with('<')
        || cleaned.starts_with("Caveat:")
        || cleaned.starts_with("[Request interrupted")
        || recap.iter().any(|start| cleaned.starts_with(start))
    {
        return None;
    }
    Some(cleaned.to_owned())
}

fn record_tool(name: &str, input: &Json, session: &mut Session) {
    let lower = name.to_ascii_lowercase();
    for key in ["file_path", "path", "notebook_path"] {
        if let Some(path) = input[key].as_str()
            && matches!(
                lower.as_str(),
                "edit" | "write" | "multiedit" | "notebookedit" | "apply_patch"
            )
        {
            session.files.push(path.to_owned());
        }
    }
    if matches!(
        lower.as_str(),
        "bash" | "powershell" | "shell" | "exec_command" | "local_shell"
    ) {
        let command = match &input["command"] {
            Json::String(command) => Some(command.clone()),
            Json::Array(parts) => Some(
                parts
                    .iter()
                    .filter_map(Json::as_str)
                    .collect::<Vec<_>>()
                    .join(" "),
            ),
            _ => None,
        };
        if let Some(command) = command {
            session.commands.push(command);
        }
    }
}

pub fn parse_claude(path: &Path) -> Option<Session> {
    let mut session = Session {
        source: "claude_code".into(),
        ..Default::default()
    };
    for line in lines(path) {
        let kind = line["type"].as_str().unwrap_or("");
        if session.id.is_empty()
            && let Some(id) = line["sessionId"].as_str()
        {
            session.id = id.to_owned();
        }
        if kind == "ai-title" {
            if let Some(title) = line["aiTitle"].as_str() {
                session.title = title.to_owned();
            }
            continue;
        }
        if kind != "user" && kind != "assistant" {
            continue;
        }
        if line["isSidechain"].as_bool() == Some(true) || line["isMeta"].as_bool() == Some(true) {
            continue;
        }
        if let Some(cwd) = line["cwd"].as_str() {
            session.cwd = cwd.to_owned();
        }
        if let Some(entry) = line["entrypoint"].as_str() {
            session.entrypoint = entry.to_owned();
        }
        if let Some(branch) = line["gitBranch"].as_str().filter(|b| !b.is_empty()) {
            session.branch = branch.to_owned();
        }
        if let Some(at) = time(&line["timestamp"]) {
            session.started.get_or_insert(at);
            session.ended = Some(at);
        }
        let content = &line["message"]["content"];
        match (kind, content) {
            ("user", Json::String(raw)) => session.prompts.extend(clean_prompt(raw)),
            (_, Json::Array(blocks)) => {
                for block in blocks {
                    match (kind, block["type"].as_str().unwrap_or("")) {
                        ("user", "text") => session
                            .prompts
                            .extend(block["text"].as_str().and_then(clean_prompt)),
                        ("assistant", "text") => {
                            if let Some(reply) = block["text"]
                                .as_str()
                                .map(str::trim)
                                .filter(|t| !t.is_empty())
                            {
                                session.replies.push(reply.to_owned());
                            }
                        }
                        ("assistant", "tool_use") => record_tool(
                            block["name"].as_str().unwrap_or(""),
                            &block["input"],
                            &mut session,
                        ),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    finish(session)
}

/// Codex keeps `session_meta` then `response_item`s (messages and function calls).
pub fn parse_codex(path: &Path) -> Option<Session> {
    let mut session = Session {
        source: "codex".into(),
        ..Default::default()
    };
    for line in lines(path) {
        if let Some(at) = time(&line["timestamp"]) {
            session.started.get_or_insert(at);
            session.ended = Some(at);
        }
        let payload = &line["payload"];
        match line["type"].as_str().unwrap_or("") {
            "session_meta" => {
                session.id = payload["id"].as_str().unwrap_or("").to_owned();
                session.cwd = payload["cwd"].as_str().unwrap_or("").to_owned();
                if let Some(branch) = payload["git"]["branch"].as_str() {
                    session.branch = branch.to_owned();
                }
            }
            "response_item" => match payload["type"].as_str().unwrap_or("") {
                "message" => {
                    let role = payload["role"].as_str().unwrap_or("");
                    for block in payload["content"].as_array().into_iter().flatten() {
                        let Some(text) = block["text"].as_str() else {
                            continue;
                        };
                        match role {
                            "user" => session.prompts.extend(clean_prompt(text).filter(|t| {
                                !t.starts_with("# AGENTS.md")
                                    && !t.contains("<environment_context>")
                            })),
                            "assistant" => session.replies.push(text.trim().to_owned()),
                            _ => {}
                        }
                    }
                }
                "function_call" | "local_shell_call" | "custom_tool_call" => {
                    let name = payload["name"].as_str().unwrap_or("shell");
                    let input = match &payload["arguments"] {
                        Json::String(raw) => serde_json::from_str(raw).unwrap_or(Json::Null),
                        other if !other.is_null() => other.clone(),
                        _ => payload["action"].clone(),
                    };
                    record_tool(name, &input, &mut session);
                    if name == "apply_patch" {
                        let patch = payload["input"]
                            .as_str()
                            .or_else(|| input["input"].as_str())
                            .unwrap_or("");
                        for line in patch.lines() {
                            for prefix in ["*** Update File: ", "*** Add File: "] {
                                if let Some(file) = line.strip_prefix(prefix) {
                                    session.files.push(file.trim().to_owned());
                                }
                            }
                        }
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }
    finish(session)
}

fn finish(mut session: Session) -> Option<Session> {
    if session.prompts.is_empty() || session.id.is_empty() {
        return None;
    }
    let mut seen = HashSet::new();
    session.files.retain(|file| seen.insert(file.clone()));
    let mut seen = HashSet::new();
    session
        .commands
        .retain(|command| seen.insert(command.trim().to_owned()));
    session.title = humanize(&session.title);
    if session.title.trim().is_empty() {
        session.title = title_from_prompts(&session.prompts);
        let substantial = session.prompts.iter().any(|p| text::words(p).count() >= 5);
        if !substantial {
            let ending = prose(session.outcome());
            if text::words(&ending).count() >= 4 {
                session.title = title_from_prompts(&[ending]);
            }
        }
    }
    Some(session)
}

/// A title from what was asked: the first prompt of some substance, cut to its first sentence.
fn title_from_prompts(prompts: &[String]) -> String {
    let prompt = prompts
        .iter()
        .find(|p| text::words(p).count() >= 5)
        .or(prompts.first())
        .map(String::as_str)
        .unwrap_or("");
    let first = prompt
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim();
    let sentence = match first.find(['.', '?', '!']) {
        Some(at) if at >= 12 => &first[..at],
        _ => first,
    };
    let title = text::excerpt(sentence, 60)
        .trim_end_matches('…')
        .trim()
        .to_owned();
    let mut chars = title.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
        None => "AI session".to_owned(),
    }
}

/// `default-markdown-view-mode` reads as "Default markdown view mode".
fn humanize(title: &str) -> String {
    let title = title.trim();
    if title.contains(' ') || !title.contains(['-', '_']) {
        return title.to_owned();
    }
    let words = title.replace(['-', '_'], " ");
    let mut chars = words.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

// ---- Secrets out ----

/// Takes secrets out of `text`: API keys and tokens of the common services, private keys,
/// passwords in connection strings and `password=`/`token:` assignments.
pub fn redact(text: &str) -> String {
    static PATTERNS: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();
    let patterns = PATTERNS.get_or_init(|| {
        [
            (r"(?s)-----BEGIN [A-Z ]*PRIVATE KEY-----.*?-----END [A-Z ]*PRIVATE KEY-----", "<redacted private key>"),
            (r"sk-ant-[A-Za-z0-9_\-]{16,}", "<redacted>"),
            (r"\bsk-[A-Za-z0-9_\-]{20,}", "<redacted>"),
            (r"\b(ghp|gho|ghu|ghs|ghr)_[A-Za-z0-9]{20,}", "<redacted>"),
            (r"\bgithub_pat_[A-Za-z0-9_]{20,}", "<redacted>"),
            (r"\bxox[abprs]-[A-Za-z0-9\-]{10,}", "<redacted>"),
            (r"\bAKIA[0-9A-Z]{16}\b", "<redacted>"),
            (r"\bAIza[0-9A-Za-z_\-]{30,}", "<redacted>"),
            (r"\beyJ[A-Za-z0-9_\-]{8,}\.[A-Za-z0-9_\-]{8,}\.[A-Za-z0-9_\-]{8,}", "<redacted>"),
            (r"(?i)\b(bearer)\s+[A-Za-z0-9._\-]{16,}", "$1 <redacted>"),
            (r"(?i)(://[^:/\s@]+:)[^@\s/]+@", "${1}<redacted>@"),
            (
                r#"(?i)\b([a-z0-9_]*(password|passwd|pwd|secret|token|api[_-]?key|access[_-]?key|client[_-]?secret)[a-z0-9_]*)(\s*[:=]\s*)("[^"\s]{4,}"|'[^'\s]{4,}'|[^\s"',;]{4,})"#,
                "$1$3<redacted>",
            ),
        ]
        .into_iter()
        .map(|(pattern, replacement)| (Regex::new(pattern).expect("valid pattern"), replacement))
        .collect()
    });
    let mut out = text.to_owned();
    for (pattern, replacement) in patterns {
        out = pattern.replace_all(&out, *replacement).into_owned();
    }
    out
}

// ---- Writing notes ----

fn relative_to(cwd: &str, file: &str) -> String {
    let cwd = cwd.replace('\\', "/");
    let file = file.replace('\\', "/");
    let cwd_lower = cwd.trim_end_matches('/').to_lowercase();
    if file.to_lowercase().starts_with(&(cwd_lower.clone() + "/")) {
        file[cwd_lower.len() + 1..].to_owned()
    } else {
        file
    }
}

/// Text without code blocks, quotes of output or Markdown markers: what reads as a sentence.
fn prose(text: &str) -> String {
    let mut out = Vec::new();
    let mut fenced = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if fenced || line.trim_start().starts_with('|') {
            continue;
        }
        out.push(
            line.trim()
                .trim_start_matches(['#', '>', '-', '*'])
                .trim()
                .replace("**", "")
                .replace('`', ""),
        );
    }
    out.join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.trim().to_owned();
    }
    let cut: String = text.chars().take(max).collect();
    let cut = cut
        .rfind(['\n', '.', ' '])
        .map(|at| cut[..at].to_owned())
        .unwrap_or(cut);
    format!("{}…", cut.trim_end())
}

/// The note for a session. `distilled` is a model's summary, when one wrote it.
pub fn note_text(session: &Session, distilled: Option<&Distilled>) -> String {
    let started = session.started.unwrap_or_else(Local::now);
    let title = distilled
        .map(|d| d.title.clone())
        .unwrap_or_else(|| session.title.clone());
    let mut front = vec![
        "---".to_owned(),
        "type: ai-session".to_owned(),
        format!("source: {}", session.source.replace('_', "-")),
        format!("captured: {}", started.format("%Y-%m-%dT%H:%M")),
        format!("project: {}", quote(&session.project())),
        format!("cwd: {}", quote(&session.cwd.replace('\\', "/"))),
    ];
    if !session.branch.is_empty() {
        front.push(format!("branch: {}", quote(&session.branch)));
    }
    if let Some(ended) = session.ended {
        let minutes = (ended - started).num_minutes();
        front.push(format!("duration_min: {minutes}"));
    }
    front.push(format!("session: {}", quote(&session.id)));
    let tag = fsx::safe_stem(&session.project())
        .to_lowercase()
        .replace(' ', "-");
    front.push(render("tags", &Value::List(vec!["ai-session".into(), tag])));
    let files: Vec<String> = session
        .files
        .iter()
        .map(|f| relative_to(&session.cwd, f))
        .collect();
    if !files.is_empty() {
        front.push(render(
            "files",
            &Value::List(files.iter().take(30).cloned().collect()),
        ));
    }
    front.push("---".to_owned());

    let mut body = format!("# {title}\n\n");
    if let Some(distilled) = distilled {
        body.push_str(&format!("> {}\n\n", distilled.summary.replace('\n', " ")));
        body.push_str(distilled.body.trim());
        body.push_str("\n\n");
    } else {
        let gist = text::excerpt(&prose(session.outcome()), 240);
        if !gist.is_empty() {
            body.push_str(&format!("> {gist}\n\n"));
        }
    }
    body.push_str("## Asked\n\n");
    body.push_str(&clip(&session.prompts[0], 1500));
    body.push_str("\n\n");
    if session.prompts.len() > 1 {
        body.push_str("Then:\n\n");
        for prompt in session.prompts.iter().skip(1).take(10) {
            body.push_str(&format!("- {}\n", text::excerpt(prompt, 200)));
        }
        if session.prompts.len() > 11 {
            body.push_str(&format!("- … and {} more\n", session.prompts.len() - 11));
        }
        body.push('\n');
    }
    if distilled.is_none() && !session.outcome().is_empty() {
        body.push_str("## Outcome\n\n");
        body.push_str(&clip(session.outcome(), 3000));
        body.push_str("\n\n");
    }
    if !files.is_empty() {
        body.push_str("## Files changed\n\n");
        for file in files.iter().take(30) {
            body.push_str(&format!("- `{file}`\n"));
        }
        body.push('\n');
    }
    if !session.commands.is_empty() {
        body.push_str("## Commands\n\n```sh\n");
        for command in session
            .commands
            .iter()
            .rev()
            .take(12)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
        {
            body.push_str(&clip(command.lines().next().unwrap_or(""), 200));
            body.push('\n');
        }
        body.push_str("```\n");
    }
    redact(&format!(
        "{}\n{}",
        front.join("\n"),
        body.trim_end().to_owned() + "\n"
    ))
}

#[derive(Clone, Debug)]
pub struct Distilled {
    pub title: String,
    pub summary: String,
    pub body: String,
}

/// With a model: the session as a finding someone can use later.
pub fn distill(model: &Model, session: &Session) -> Result<Distilled> {
    let mut material = String::new();
    for (i, prompt) in session.prompts.iter().enumerate().take(20) {
        material.push_str(&format!("USER {}: {}\n\n", i + 1, clip(prompt, 2000)));
    }
    for reply in session
        .replies
        .iter()
        .rev()
        .take(6)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
    {
        material.push_str(&format!("AGENT: {}\n\n", clip(reply, 2500)));
    }
    if !session.files.is_empty() {
        material.push_str(&format!("FILES CHANGED: {}\n", session.files.join(", ")));
    }
    let material = redact(&material);
    let reply = model.complete(
        "You turn a coding session between a developer and an AI agent into a note for the developer's \
         notebook, read months later by someone without the conversation. Reply in this exact shape:\n\
         TITLE: the finding stated as a claim, under 12 words (not the topic)\n\
         SUMMARY: one sentence that answers the question on its own\n\
         BODY:\n\
         Markdown with only the sections that apply: '## Problem', '## What was found', '## Decision', \
         '## What worked' (commands, in fenced blocks), '## Watch out'. Be brief and concrete. Never include \
         secrets, tokens or passwords.",
        &material,
    )?;
    let title = model::field(&reply, "title")
        .ok_or_else(|| crate::Error::msg("the model gave no title"))?;
    let summary = model::field(&reply, "summary").unwrap_or_default();
    let body = reply
        .split_once("BODY:")
        .map(|(_, body)| body.trim().to_owned())
        .unwrap_or_default();
    Ok(Distilled {
        title: fsx::safe_stem(&title),
        summary,
        body,
    })
}

/// A session run in a temporary folder is a throwaway (tests, experiments).
fn is_temp_folder(cwd: &str) -> bool {
    let cwd = cwd.replace('\\', "/").to_lowercase();
    let temp = std::env::temp_dir()
        .to_string_lossy()
        .replace('\\', "/")
        .to_lowercase();
    (!temp.is_empty() && cwd.starts_with(temp.trim_end_matches('/')))
        || cwd.contains("/appdata/local/temp/")
        || cwd.starts_with("/tmp/")
        || cwd.starts_with("/var/folders/")
}

// ---- Importing ----

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Imported {
    size: u64,
    note: String,
    /// Hash of the note as FastMind wrote it; an edited note is never overwritten.
    written: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct State {
    transcripts: BTreeMap<String, Imported>,
}

fn state_file(nb: &Notebook) -> PathBuf {
    nb.root.join(STATE_DIR).join("ai-sessions.json")
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct ImportReport {
    pub found: usize,
    pub added: Vec<String>,
    pub updated: Vec<String>,
    /// Still running, too old for a first import, or too short.
    pub skipped: usize,
    pub notes: Vec<String>,
}

pub fn import(
    nb: &Notebook,
    settings: &AiSettings,
    model: Option<&Model>,
    model_budget: usize,
    now: DateTime<Local>,
) -> Result<ImportReport> {
    let mut state: State = fs::read(state_file(nb))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    let transcripts = discover(settings);
    let mut report = ImportReport {
        found: transcripts.len(),
        ..Default::default()
    };
    let mut budget = model_budget;
    for transcript in transcripts {
        let key = transcript.path.to_string_lossy().into_owned();
        let known = state.transcripts.get(&key).cloned();
        if known.as_ref().is_some_and(|k| k.size == transcript.size) {
            continue;
        }
        let quiet = (now - transcript.modified).num_minutes();
        let too_old =
            (now - transcript.modified).num_days() > settings.since_days && known.is_none();
        if quiet < settings.quiet_minutes || too_old {
            report.skipped += 1;
            continue;
        }
        let Some(session) = read(&transcript) else {
            report.skipped += 1;
            continue;
        };
        let throwaway = is_temp_folder(&session.cwd) && !settings.include_automated;
        if session.prompts.len() < settings.min_prompts
            || (session.is_automated() && !settings.include_automated)
            || throwaway
        {
            report.skipped += 1;
            continue;
        }
        // The note FastMind wrote before, if the user hasn't edited it since.
        let previous = known.as_ref().filter(|k| {
            nb.path(&k.note)
                .ok()
                .and_then(|p| fs::read(p).ok())
                .is_some_and(|bytes| text::fnv(&bytes) == k.written)
        });
        if known.is_some()
            && previous.is_none()
            && known.as_ref().is_some_and(|k| nb.exists(&k.note))
        {
            // Edited by hand: leave it, and stop following this transcript.
            state.transcripts.insert(
                key,
                Imported {
                    size: transcript.size,
                    ..known.unwrap_or_default()
                },
            );
            continue;
        }
        let distilled = match (model, budget > 0) {
            (Some(model), true) => {
                budget -= 1;
                match distill(model, &session) {
                    Ok(d) => Some(d),
                    Err(error) => {
                        report.notes.push(format!("{}: {error}", session.title));
                        None
                    }
                }
            }
            _ => None,
        };
        let text = note_text(&session, distilled.as_ref());
        let rel = match previous {
            Some(previous) => previous.note.clone(),
            None => {
                let folder = format!(
                    "{}/{}",
                    settings.folder.trim_matches('/'),
                    fsx::safe_stem(&session.project())
                );
                let date = session.started.unwrap_or(now).format("%Y-%m-%d");
                let title = distilled
                    .as_ref()
                    .map(|d| d.title.clone())
                    .unwrap_or_else(|| session.title.clone());
                let stem = fsx::safe_stem(&format!("{date} {title}"));
                let name = fsx::free_name(&stem, "md", |candidate| {
                    nb.exists(&format!("{folder}/{candidate}"))
                });
                format!("{folder}/{name}")
            }
        };
        let path = nb.path(&rel)?;
        fsx::write_atomic(&path, text.as_bytes())?;
        // The note is as old as the session, so Dream judges it by when the work happened.
        if let (Some(ended), Ok(file)) = (
            session.ended,
            fs::OpenOptions::new().write(true).open(&path),
        ) {
            let _ = file.set_modified(ended.into());
        }
        if previous.is_some() {
            report.updated.push(rel.clone());
        } else {
            report.added.push(rel.clone());
        }
        state.transcripts.insert(
            key,
            Imported {
                size: transcript.size,
                note: rel,
                written: text::fnv(text.as_bytes()),
            },
        );
    }
    fsx::write_atomic(&state_file(nb), &serde_json::to_vec_pretty(&state)?)?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_are_taken_out() {
        let text = "key sk-ant-api03-abcdefghijklmnopqrstuv and ghp_abcdefghijklmnopqrstuvwxyz0123 \
            postgres://app:hunter2@db:5432/x password=s3cretvalue api_key: \"abcd1234efgh\" Bearer abcdefghijklmnopqrstuvwx \
            AKIAABCDEFGHIJKLMNOP";
        let out = redact(text);
        for secret in [
            "sk-ant-api03",
            "ghp_abc",
            "hunter2",
            "s3cretvalue",
            "abcd1234efgh",
            "abcdefghijklmnopqrstuvwx",
            "AKIAABCD",
        ] {
            assert!(!out.contains(secret), "{secret} survived: {out}");
        }
        assert!(out.contains("postgres://app:<redacted>@db"));
        assert!(out.contains("password=<redacted>"));
        // Ordinary prose about passwords is left alone.
        assert_eq!(
            redact("Reset the password in settings."),
            "Reset the password in settings."
        );
    }

    #[test]
    fn projects_see_through_worktrees() {
        assert_eq!(project_name("C:\\Projects\\Payments"), "Payments");
        assert_eq!(
            project_name("C:/Projects/Payments/.claude/worktrees/fix-x"),
            "Payments"
        );
        assert_eq!(project_name("/home/a/code/api/"), "api");
    }

    #[test]
    fn prompts_lose_harness_tags() {
        assert_eq!(
            clean_prompt("<system-reminder>x</system-reminder>Fix the build"),
            Some("Fix the build".into())
        );
        assert_eq!(clean_prompt("<command-name>/clear</command-name>"), None);
        assert_eq!(
            clean_prompt("Background (from the session that started this one): …"),
            None
        );
        assert_eq!(
            humanize("default-markdown-view-mode"),
            "Default markdown view mode"
        );
        assert_eq!(humanize("Winget PR issue"), "Winget PR issue");
        assert_eq!(
            title_from_prompts(&[
                "ok".into(),
                "we are on a new machine. First make sure it builds".into()
            ]),
            "We are on a new machine"
        );
        assert_eq!(
            prose(
                "Done:
```
code
```
**All** tests pass."
            ),
            "Done: All tests pass."
        );
    }
}
