//! Learn: saves what you caught as a note in the notebook's inbox, instantly and with no model.
//! The note is plain Markdown with front matter any editor reads.

use crate::frontmatter::{Value, quote, render};
use crate::notebook::Notebook;
use crate::{Error, Result, fsx, text};
use chrono::Local;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Capture {
    pub text: String,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    /// Files to copy in beside the note.
    #[serde(default)]
    pub attachments: Vec<PathBuf>,
    /// Folder inside the notebook; the inbox by default.
    #[serde(default)]
    pub folder: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Saved {
    pub path: String,
    pub title: String,
}

pub const MAX_TEXT: usize = 1_000_000;
pub const MAX_ATTACHMENT: u64 = 50 * 1024 * 1024;

pub fn save(nb: &Notebook, inbox: &str, capture: &Capture) -> Result<Saved> {
    let body_text = capture.text.trim();
    let url = capture
        .url
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty());
    // A capture that is only a link is recorded as one.
    let (body_text, url) = match url {
        Some(url) => (body_text.to_owned(), Some(url.to_owned())),
        None if is_url(body_text) => (String::new(), Some(body_text.to_owned())),
        None => (body_text.to_owned(), None),
    };
    if body_text.is_empty() && url.is_none() && capture.attachments.is_empty() {
        return Err(Error::msg("nothing to capture"));
    }
    if body_text.len() > MAX_TEXT {
        return Err(Error::msg("the text is over 1 MB"));
    }
    for file in &capture.attachments {
        let meta =
            fs::metadata(file).map_err(|_| Error::msg(format!("can't read {}", file.display())))?;
        if meta.len() > MAX_ATTACHMENT {
            return Err(Error::msg(format!("{} is over 50 MB", file.display())));
        }
    }

    let now = Local::now();
    let title = title_for(&body_text, url.as_deref(), &capture.attachments)
        .unwrap_or_else(|| format!("Capture {}", now.format("%Y-%m-%d %H.%M")));
    let folder = capture
        .folder
        .as_deref()
        .map(|folder| folder.trim_matches(['/', '\\']).to_owned())
        .filter(|folder| !folder.is_empty())
        .unwrap_or_else(|| inbox.to_owned());
    let dir = nb.path(&folder)?;
    fs::create_dir_all(&dir)?;
    let name = fsx::free_name(&fsx::safe_stem(&title), "md", |name| {
        dir.join(name).exists()
    });
    let rel = if folder.is_empty() {
        name.clone()
    } else {
        format!("{folder}/{name}")
    };

    let mut tags: Vec<String> = capture
        .tags
        .iter()
        .map(|tag| tag.trim_start_matches('#').to_owned())
        .collect();
    tags.extend(hashtags(&body_text));
    tags.sort();
    tags.dedup();

    let mut front = vec![
        "---".to_owned(),
        "type: capture".to_owned(),
        format!("captured: {}", now.format("%Y-%m-%dT%H:%M")),
        "status: inbox".to_owned(),
    ];
    if let Some(url) = &url {
        front.push(format!("source_url: {}", quote(url)));
    }
    if !tags.is_empty() {
        front.push(render("tags", &Value::List(tags)));
    }
    front.push("---".to_owned());

    let mut body = format!("# {title}\n\n");
    let first_line = body_text.lines().next().unwrap_or("").trim();
    let rest = if clean_line(first_line) == title {
        body_text[body_text.find('\n').unwrap_or(body_text.len())..].trim()
    } else {
        body_text.as_str()
    };
    if !rest.is_empty() {
        body.push_str(rest);
        body.push_str("\n\n");
    }
    if let Some(url) = &url {
        body.push_str(&format!("Source: <{url}>\n\n"));
    }

    // Attachments are written first; if the note then fails, they are removed again.
    let mut written: Vec<PathBuf> = Vec::new();
    if !capture.attachments.is_empty() {
        let attach_dir = dir.join("attachments");
        fs::create_dir_all(&attach_dir)?;
        body.push_str("## Attachments\n\n");
        for file in &capture.attachments {
            let original = file
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            let (stem, ext) = match original.rfind('.') {
                Some(at) => (original[..at].to_owned(), original[at + 1..].to_owned()),
                None => (original.clone(), String::new()),
            };
            let free = fsx::free_name(&fsx::safe_stem(&stem), &ext, |name| {
                attach_dir.join(name).exists()
            });
            let target = attach_dir.join(&free);
            if let Err(error) = fs::copy(file, &target) {
                for path in written {
                    let _ = fs::remove_file(path);
                }
                return Err(error.into());
            }
            written.push(target);
            let link = format!("attachments/{}", free.replace(' ', "%20"));
            if is_image(&ext) {
                body.push_str(&format!("![{free}]({link})\n"));
            } else {
                body.push_str(&format!("- [{free}]({link})\n"));
            }
        }
    }

    let note = format!(
        "{}\n{}",
        front.join("\n"),
        body.trim_end().to_owned() + "\n"
    );
    if let Err(error) = fsx::write_atomic(&dir.join(&name), note.as_bytes()) {
        for path in written {
            let _ = fs::remove_file(path);
        }
        return Err(error);
    }
    Ok(Saved { path: rel, title })
}

fn title_for(text: &str, url: Option<&str>, attachments: &[PathBuf]) -> Option<String> {
    if let Some(line) = text.lines().map(clean_line).find(|line| !line.is_empty()) {
        return Some(shorten(&line, 70));
    }
    if let Some(url) = url {
        let host = url
            .split("://")
            .nth(1)
            .unwrap_or(url)
            .split('/')
            .next()
            .unwrap_or(url);
        let path = url
            .split("://")
            .nth(1)
            .and_then(|rest| rest.split_once('/'))
            .map(|(_, path)| path)
            .unwrap_or("");
        let last = path
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or("")
            .replace(['-', '_'], " ");
        let last = last
            .split(['?', '#', '.'])
            .next()
            .unwrap_or("")
            .trim()
            .to_owned();
        return Some(if last.is_empty() {
            host.to_owned()
        } else {
            format!("{} ({host})", capitalize(&last))
        });
    }
    attachments
        .first()
        .and_then(|file| file.file_stem())
        .map(|stem| stem.to_string_lossy().into_owned())
}

fn clean_line(line: &str) -> String {
    let line = line
        .trim()
        .trim_start_matches(['#', '>', '-', '*', ' '])
        .trim();
    let line = line.replace("**", "").replace("__", "").replace('`', "");
    line.split_whitespace()
        .filter(|word| !word.starts_with('#') || word.len() == 1)
        .collect::<Vec<_>>()
        .join(" ")
}

fn shorten(text: &str, max: usize) -> String {
    let sentence_end = text.find(['.', '?', '!']).filter(|at| *at > 10);
    let text = match sentence_end {
        Some(at) if at < max => &text[..at],
        _ => text,
    };
    text::excerpt(text, max)
        .trim_end_matches('…')
        .trim()
        .to_owned()
}

fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

pub fn is_url(text: &str) -> bool {
    !text.contains(char::is_whitespace)
        && (text.starts_with("http://") || text.starts_with("https://"))
}

fn is_image(ext: &str) -> bool {
    matches!(
        ext.to_ascii_lowercase().as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "svg"
    )
}

/// `#tags` written in the text.
pub fn hashtags(text: &str) -> Vec<String> {
    text.split_whitespace()
        .filter_map(|word| word.strip_prefix('#'))
        .map(|tag| {
            tag.trim_end_matches(|ch: char| {
                !ch.is_alphanumeric() && ch != '-' && ch != '_' && ch != '/'
            })
        })
        .filter(|tag| !tag.is_empty() && tag.chars().next().is_some_and(char::is_alphabetic))
        .map(str::to_lowercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontmatter;

    #[test]
    fn saves_a_note_in_the_inbox() {
        let dir = tempfile::tempdir().unwrap();
        let nb = Notebook::open(dir.path()).unwrap();
        let saved = save(
            &nb,
            "inbox",
            &Capture {
                text: "Postgres vacuum stalls on big tables. #db\nCheck autovacuum_naptime.".into(),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(saved.title, "Postgres vacuum stalls on big tables");
        assert_eq!(saved.path, "inbox/Postgres vacuum stalls on big tables.md");
        let text = fs::read_to_string(nb.root.join(&saved.path)).unwrap();
        let front = frontmatter::parse(&text);
        assert_eq!(front.text("status"), Some("inbox"));
        assert_eq!(front.tags(), vec!["db"]);
        assert!(text.contains("Check autovacuum_naptime."));

        let again = save(
            &nb,
            "inbox",
            &Capture {
                text: "Postgres vacuum stalls on big tables".into(),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            again.path,
            "inbox/Postgres vacuum stalls on big tables 2.md"
        );
    }

    #[test]
    fn a_bare_link_is_a_link_capture() {
        let dir = tempfile::tempdir().unwrap();
        let nb = Notebook::open(dir.path()).unwrap();
        let saved = save(
            &nb,
            "inbox",
            &Capture {
                text: "https://example.com/blog/rust-error-handling".into(),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(saved.title, "Rust error handling (example.com)");
        let text = fs::read_to_string(nb.root.join(&saved.path)).unwrap();
        assert_eq!(
            frontmatter::parse(&text).text("source_url"),
            Some("https://example.com/blog/rust-error-handling")
        );
    }

    #[test]
    fn attachments_are_copied_and_linked() {
        let dir = tempfile::tempdir().unwrap();
        let nb = Notebook::open(dir.path().join("nb")).unwrap();
        let shot = dir.path().join("screen shot.png");
        fs::write(&shot, b"png").unwrap();
        let saved = save(
            &nb,
            "inbox",
            &Capture {
                text: "Bug in dialog".into(),
                attachments: vec![shot],
                ..Default::default()
            },
        )
        .unwrap();
        let text = fs::read_to_string(nb.root.join(&saved.path)).unwrap();
        assert!(
            text.contains("![screen shot.png](attachments/screen%20shot.png)"),
            "{text}"
        );
        assert!(nb.root.join("inbox/attachments/screen shot.png").exists());
    }

    #[test]
    fn empty_capture_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let nb = Notebook::open(dir.path()).unwrap();
        assert!(save(&nb, "inbox", &Capture::default()).is_err());
    }
}
