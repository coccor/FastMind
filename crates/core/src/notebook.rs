//! A notebook: a folder of Markdown notes. FastMind keeps its own state in `.fastmind/` inside
//! it (Dream's proposals, the undo journal, the archive), so the notebook carries everything with
//! it and any editor still sees plain files.

use crate::frontmatter::{self, FrontMatter};
use crate::{Result, fsx, links, text};
use chrono::{DateTime, Local, NaiveDate, NaiveDateTime, TimeZone};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

pub const STATE_DIR: &str = ".fastmind";

/// Note types that organise other notes; Dream never archives or absorbs them.
pub const HOME_TYPES: [&str; 5] = ["topic", "project", "moc", "index", "home"];

#[derive(Clone, Debug)]
pub struct Notebook {
    pub root: PathBuf,
}

#[derive(Clone, Debug)]
pub struct Note {
    /// Relative to the notebook, `/` between folders.
    pub path: String,
    pub title: String,
    pub front: FrontMatter,
    pub text: String,
    pub size: u64,
    /// The file's modification time.
    pub modified: DateTime<Local>,
    /// The newest of the file's modification time and the dates in its front matter.
    pub touched: DateTime<Local>,
    /// The date the note says it was made, if it says.
    pub created: Option<DateTime<Local>>,
    pub links: Vec<String>,
}

impl Note {
    pub fn body(&self) -> &str {
        &self.text[self.front.body_start..]
    }

    pub fn stem(&self) -> &str {
        let name = self.path.rsplit('/').next().unwrap_or(&self.path);
        name.strip_suffix(".md").unwrap_or(name)
    }

    pub fn folder(&self) -> &str {
        match self.path.rfind('/') {
            Some(at) => &self.path[..at],
            None => "",
        }
    }

    pub fn note_type(&self) -> String {
        self.front.text("type").unwrap_or("").to_ascii_lowercase()
    }

    pub fn status(&self) -> String {
        self.front.text("status").unwrap_or("").to_ascii_lowercase()
    }

    pub fn is_home(&self) -> bool {
        HOME_TYPES.contains(&self.note_type().as_str())
    }

    /// A template: in a templates folder or `type: template`. A tool, not a note.
    pub fn is_template(&self) -> bool {
        self.note_type() == "template"
            || self.path.split('/').rev().skip(1).any(|folder| {
                matches!(
                    folder.to_ascii_lowercase().as_str(),
                    "templates" | "_templates" | "template"
                )
            })
    }

    /// A record of a day or an event (daily notes, journals, meetings, logs). Records are
    /// history: Dream never retires them for being old.
    pub fn is_record(&self) -> bool {
        let kind = self.note_type();
        if matches!(
            kind.as_str(),
            "daily" | "journal" | "meeting" | "log" | "diary" | "weekly"
        ) {
            return true;
        }
        if kind.starts_with("ai-") {
            return false;
        }
        // A daily note's name is its date, perhaps with the weekday: "2026-11-09", "2026-11-09 Monday".
        let stem = self.stem();
        let rest = stem.get(10..).unwrap_or("").trim().to_ascii_lowercase();
        let only_a_date = rest.is_empty()
            || (!rest.contains(' ')
                && ["mon", "tue", "wed", "thu", "fri", "sat", "sun"]
                    .iter()
                    .any(|day| rest.starts_with(day)));
        let dated = only_a_date
            && stem.len() >= 10
            && stem.as_bytes()[..10]
                .iter()
                .enumerate()
                .all(|(i, b)| match i {
                    4 | 7 => *b == b'-' || *b == b'.' || *b == b'_',
                    _ => b.is_ascii_digit(),
                });
        dated
            || self.path.split('/').rev().skip(1).any(|folder| {
                matches!(
                    folder.to_ascii_lowercase().as_str(),
                    "daily" | "journal" | "journals" | "diary" | "dailies" | "daily notes"
                )
            })
    }

    /// Pinned notes are never archived or merged away.
    pub fn is_pinned(&self) -> bool {
        let yes = |key: &str| {
            self.front.text(key).is_some_and(|value| {
                matches!(
                    value.to_ascii_lowercase().as_str(),
                    "true" | "yes" | "forever"
                )
            })
        };
        yes("pin") || yes("pinned") || yes("keep")
    }

    pub fn word_count(&self) -> usize {
        text::words(self.body()).count()
    }

    /// Names a link can use for this note: its file name, title and aliases, lower-cased.
    pub fn names(&self) -> Vec<String> {
        let mut names = vec![self.stem().to_lowercase(), self.title.to_lowercase()];
        names.extend(
            self.front
                .aliases()
                .iter()
                .map(|alias| alias.to_lowercase()),
        );
        names.sort();
        names.dedup();
        names
    }
}

impl Notebook {
    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        fs::create_dir_all(&root)?;
        let root = root.canonicalize().unwrap_or(root);
        let root = strip_verbatim(root);
        Ok(Self { root })
    }

    pub fn state_dir(&self) -> PathBuf {
        self.root.join(STATE_DIR)
    }

    pub fn path(&self, rel: &str) -> Result<PathBuf> {
        fsx::resolve(&self.root, rel)
    }

    pub fn exists(&self, rel: &str) -> bool {
        self.path(rel).map(|path| path.exists()).unwrap_or(false)
    }

    /// Every note in the notebook, sorted by path. Hidden folders (`.git`, `.obsidian`,
    /// `.fastmind`) and build folders are left out.
    pub fn notes(&self) -> Vec<Note> {
        let mut files = Vec::new();
        fsx::walk(&self.root, &skip_dir, &mut files);
        let mut notes: Vec<Note> = files
            .iter()
            .filter(|path| is_note_file(path))
            .filter_map(|path| self.read_path(path).ok())
            .collect();
        notes.sort_by(|a, b| a.path.cmp(&b.path));
        notes
    }

    /// Every file that isn't a note: images, PDFs and other attachments.
    pub fn attachments(&self) -> Vec<(String, u64)> {
        let mut files = Vec::new();
        fsx::walk(&self.root, &skip_dir, &mut files);
        files
            .iter()
            .filter(|path| !is_note_file(path))
            .filter(|path| {
                !path
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with('.'))
            })
            .filter_map(|path| {
                let rel = fsx::relative(&self.root, path)?;
                let size = fs::metadata(path).ok()?.len();
                Some((rel, size))
            })
            .collect()
    }

    pub fn read(&self, rel: &str) -> Result<Note> {
        self.read_path(&self.path(rel)?)
    }

    fn read_path(&self, path: &Path) -> Result<Note> {
        let bytes = fs::read(path)?;
        let text = String::from_utf8_lossy(&bytes).into_owned();
        let meta = fs::metadata(path)?;
        let modified: DateTime<Local> = meta
            .modified()
            .map(DateTime::from)
            .unwrap_or_else(|_| Local::now());
        let rel =
            fsx::relative(&self.root, path).unwrap_or_else(|| path.to_string_lossy().into_owned());
        Ok(build_note(rel, text, bytes.len() as u64, modified))
    }
}

pub fn build_note(path: String, text: String, size: u64, modified: DateTime<Local>) -> Note {
    let front = frontmatter::parse(&text);
    let body = &text[front.body_start..];
    let stem = {
        let name = path.rsplit('/').next().unwrap_or(&path);
        name.strip_suffix(".md").unwrap_or(name).to_owned()
    };
    let title = front
        .text("title")
        .map(str::to_owned)
        .or_else(|| first_heading(body))
        .unwrap_or(stem);
    let dates: Vec<DateTime<Local>> = ["updated", "modified", "captured", "created", "date"]
        .iter()
        .filter_map(|key| front.text(key).and_then(parse_date))
        .collect();
    let created = ["created", "captured", "date"]
        .iter()
        .find_map(|key| front.text(key).and_then(parse_date));
    let touched = dates
        .iter()
        .copied()
        .chain([modified])
        .max()
        .unwrap_or(modified);
    let links = links::targets(body);
    Note {
        path,
        title,
        text: text.clone(),
        front,
        size,
        modified,
        touched,
        created,
        links,
    }
}

fn first_heading(body: &str) -> Option<String> {
    body.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .and_then(|line| line.strip_prefix("# "))
        .map(|heading| heading.trim().to_owned())
        .filter(|heading| !heading.is_empty())
}

/// Reads `2026-11-09`, `2026-11-09T22:10`, `2026-11-09 22:10:05` and RFC 3339.
pub fn parse_date(text: &str) -> Option<DateTime<Local>> {
    let text = text.trim();
    if let Ok(date) = DateTime::parse_from_rfc3339(text) {
        return Some(date.with_timezone(&Local));
    }
    for format in [
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%dT%H:%M",
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%d %H:%M",
    ] {
        if let Ok(naive) = NaiveDateTime::parse_from_str(text, format) {
            return Local.from_local_datetime(&naive).earliest();
        }
    }
    let date = NaiveDate::parse_from_str(text.get(..10)?, "%Y-%m-%d").ok()?;
    Local
        .from_local_datetime(&date.and_hms_opt(12, 0, 0)?)
        .earliest()
}

pub fn skip_dir(name: &str) -> bool {
    name.starts_with('.') || matches!(name, "node_modules" | "target" | "__pycache__")
}

pub fn is_note_file(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("md") || ext.eq_ignore_ascii_case("markdown"))
        && !path
            .file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with('.'))
}

fn strip_verbatim(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy();
    match text.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with("UNC") => PathBuf::from(rest),
        _ => path,
    }
}

/// Which notes each note is linked from (by path), counting only links that resolve.
pub fn backlinks(notes: &[Note]) -> HashMap<String, Vec<String>> {
    let resolver = links::Resolver::new(notes);
    let mut inbound: HashMap<String, Vec<String>> = HashMap::new();
    for note in notes {
        for target in &note.links {
            if let Some(to) = resolver.resolve(target)
                && to != note.path
            {
                inbound
                    .entry(to.to_owned())
                    .or_default()
                    .push(note.path.clone());
            }
        }
    }
    for from in inbound.values_mut() {
        from.sort();
        from.dedup();
    }
    inbound
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_come_from_front_matter_heading_or_name() {
        let now = Local::now();
        let note = build_note("a/b.md".into(), "---\ntitle: T\n---\n# H\n".into(), 1, now);
        assert_eq!(note.title, "T");
        let note = build_note("a/b.md".into(), "\n# Heading\ntext".into(), 1, now);
        assert_eq!(note.title, "Heading");
        let note = build_note("a/b.md".into(), "text".into(), 1, now);
        assert_eq!(note.title, "b");
        assert_eq!(note.folder(), "a");
    }

    #[test]
    fn touched_is_the_newest_date() {
        let old = parse_date("2020-01-01").unwrap();
        let note = build_note(
            "x.md".into(),
            "---\nupdated: 2024-05-01\n---\n".into(),
            1,
            old,
        );
        assert_eq!(note.touched, parse_date("2024-05-01").unwrap());
    }
}
