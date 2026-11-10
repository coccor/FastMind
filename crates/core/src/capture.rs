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
    /// Files pasted or dropped into a window that can't give their paths (a browser): the
    /// bytes, base64-encoded.
    #[serde(default)]
    pub files: Vec<InlineFile>,
    /// Write the note with the model before returning, instead of in the background.
    #[serde(default)]
    pub wait: bool,
    /// Already written (an agent's finding, or saved with `--raw`): kept as it is.
    #[serde(default)]
    pub as_written: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct InlineFile {
    pub name: String,
    /// Base64, with or without a `data:…;base64,` prefix.
    pub data: String,
}

enum Source<'a> {
    Path(&'a std::path::Path),
    Bytes(Vec<u8>),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Saved {
    pub path: String,
    pub title: String,
    /// Hash of the note as saved: a later background update only applies to it unchanged.
    #[serde(default)]
    pub written: u64,
    /// The raw capture kept for the writing step, in `.fastmind/raw/<id>.json`.
    #[serde(default)]
    pub capture_id: Option<String>,
}

/// What you captured, exactly as you gave it: kept so a model can write the note from it, and
/// write it again later. The note itself is yours to edit; this record never changes, except
/// to say which note was written from it.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Raw {
    pub id: String,
    /// `YYYY-MM-DDTHH:MM`.
    pub captured: String,
    pub text: String,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    /// The stored files, relative to the notebook.
    #[serde(default)]
    pub attachments: Vec<String>,
    /// The note now holding this capture, relative to the notebook.
    pub note: String,
    /// The note's hash as FastMind last wrote it: a note with another hash was edited.
    pub note_hash: u64,
    /// Which model wrote the note, once one has.
    #[serde(default)]
    pub written_by: Option<String>,
    /// Why writing failed, the last time it was tried.
    #[serde(default)]
    pub failed: Option<String>,
}

pub fn raw_dir(nb: &Notebook) -> PathBuf {
    nb.state_dir().join("raw")
}

impl Raw {
    pub fn load(nb: &Notebook, id: &str) -> Result<Raw> {
        if id.contains(['/', '\\', '.']) {
            return Err(Error::msg("bad capture id"));
        }
        let bytes = fs::read(raw_dir(nb).join(format!("{id}.json")))
            .map_err(|_| Error::msg("the original capture isn't kept for this note"))?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    pub fn save(&self, nb: &Notebook) -> Result<()> {
        let dir = raw_dir(nb);
        fs::create_dir_all(&dir)?;
        fsx::write_atomic(
            &dir.join(format!("{}.json", self.id)),
            &serde_json::to_vec_pretty(self)?,
        )
    }

    /// Every kept capture, newest first.
    pub fn all(nb: &Notebook) -> Vec<Raw> {
        let mut out: Vec<Raw> = fs::read_dir(raw_dir(nb))
            .into_iter()
            .flatten()
            .flatten()
            .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
            .filter_map(|entry| serde_json::from_slice(&fs::read(entry.path()).ok()?).ok())
            .collect();
        out.sort_by(|a: &Raw, b: &Raw| b.id.cmp(&a.id));
        out
    }
}

/// A free file name in `dir`, claimed by creating it empty: another capture at the same
/// moment can't take the same name.
fn reserve(dir: &std::path::Path, stem: &str, ext: &str) -> Result<String> {
    for _ in 0..50 {
        let name = fsx::free_name(stem, ext, |name| dir.join(name).exists());
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(dir.join(&name))
        {
            Ok(_) => return Ok(name),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }
    Err(Error::msg("couldn't find a free file name"))
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
    let mut sources: Vec<(String, Source)> = capture
        .attachments
        .iter()
        .map(|path| {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "file".into());
            (name, Source::Path(path.as_path()))
        })
        .collect();
    for file in &capture.files {
        let data = file
            .data
            .split_once(";base64,")
            .map_or(file.data.as_str(), |(_, data)| data);
        let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, data.trim())
            .map_err(|_| Error::msg(format!("{} couldn't be read", file.name)))?;
        if bytes.len() as u64 > MAX_ATTACHMENT {
            return Err(Error::msg(format!("{} is over 50 MB", file.name)));
        }
        let name = if file.name.trim().is_empty() {
            "pasted.png".to_owned()
        } else {
            file.name.clone()
        };
        sources.push((name, Source::Bytes(bytes)));
    }
    if body_text.is_empty() && url.is_none() && sources.is_empty() {
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
    let capture_id = crate::journal::new_id();
    let names: Vec<PathBuf> = sources
        .iter()
        .map(|(name, _)| PathBuf::from(name))
        .collect();
    let title = title_for(&body_text, url.as_deref(), &names)
        .unwrap_or_else(|| format!("Capture {}", now.format("%Y-%m-%d %H.%M")));
    let folder = capture
        .folder
        .as_deref()
        .map(|folder| folder.trim_matches(['/', '\\']).to_owned())
        .filter(|folder| !folder.is_empty())
        .unwrap_or_else(|| inbox.to_owned());
    let dir = nb.path(&folder)?;
    fs::create_dir_all(&dir)?;
    let name = reserve(&dir, &fsx::safe_stem(&title), "md")?;
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
        format!("capture_id: {capture_id}"),
        "status: inbox".to_owned(),
    ];
    if let Some(url) = &url {
        front.push(format!("source_url: {}", quote(url)));
    }
    if !tags.is_empty() {
        front.push(render("tags", &Value::List(tags.clone())));
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
    let mut stored: Vec<String> = Vec::new();
    if !sources.is_empty() {
        let attach_dir = dir.join("attachments");
        fs::create_dir_all(&attach_dir)?;
        body.push_str("## Attachments\n\n");
        for (original, source) in &sources {
            let (stem, ext) = match original.rfind('.') {
                Some(at) => (original[..at].to_owned(), original[at + 1..].to_owned()),
                None => (original.clone(), String::new()),
            };
            let free = match reserve(&attach_dir, &fsx::safe_stem(&stem), &ext) {
                Ok(free) => free,
                Err(error) => {
                    for path in written {
                        let _ = fs::remove_file(path);
                    }
                    let _ = fs::remove_file(dir.join(&name));
                    return Err(error);
                }
            };
            let target = attach_dir.join(&free);
            let result = match source {
                Source::Path(path) => fs::copy(path, &target).map(|_| ()),
                Source::Bytes(bytes) => fs::write(&target, bytes),
            };
            if let Err(error) = result {
                let _ = fs::remove_file(&target);
                for path in written {
                    let _ = fs::remove_file(path);
                }
                let _ = fs::remove_file(dir.join(&name));
                return Err(error.into());
            }
            written.push(target);
            stored.push(if folder.is_empty() {
                format!("attachments/{free}")
            } else {
                format!("{folder}/attachments/{free}")
            });
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
        let _ = fs::remove_file(dir.join(&name));
        return Err(error);
    }
    let hash = text::fnv(note.as_bytes());
    let raw = Raw {
        id: capture_id.clone(),
        captured: now.format("%Y-%m-%dT%H:%M").to_string(),
        text: capture.text.clone(),
        url,
        tags,
        attachments: stored,
        note: rel.clone(),
        note_hash: hash,
        written_by: None,
        failed: None,
    };
    // The note is saved either way; without this record it just can't be written by a model.
    let kept = raw.save(nb).is_ok();
    Ok(Saved {
        path: rel,
        title,
        written: hash,
        capture_id: kept.then_some(capture_id),
    })
}

/// What a web page says it is: its title and description, from `<title>`, Open Graph and
/// `description` meta tags. Fetched once, like a browser but without cookies, within a few
/// seconds and 1 MB.
pub fn page_info(url: &str) -> Option<(String, Option<String>)> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(4))
        .timeout(std::time::Duration::from_secs(8))
        .redirects(5)
        .user_agent("Mozilla/5.0 (FastMind; +https://github.com/coccor/FastMind)")
        .build();
    let response = agent.get(url).call().ok()?;
    if !response.content_type().contains("html") {
        return None;
    }
    let mut html = String::new();
    std::io::Read::read_to_string(
        &mut std::io::Read::take(response.into_reader(), 1_000_000),
        &mut html,
    )
    .ok()?;
    let head = &html[..html.find("</head>").unwrap_or(html.len().min(200_000))];
    let meta = |names: &[&str]| -> Option<String> {
        static TAG: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
        let tag = TAG.get_or_init(|| regex::Regex::new(r#"(?is)<meta\s[^>]*>"#).expect("valid"));
        for found in tag.find_iter(head) {
            let tag = found.as_str();
            let attr = |name: &str| {
                let pattern = format!(r#"(?is)\b{name}\s*=\s*("([^"]*)"|'([^']*)')"#);
                regex::Regex::new(&pattern)
                    .ok()?
                    .captures(tag)
                    .and_then(|c| c.get(2).or(c.get(3)))
                    .map(|m| m.as_str().to_owned())
            };
            let key = attr("property")
                .or_else(|| attr("name"))
                .unwrap_or_default()
                .to_lowercase();
            if names.contains(&key.as_str())
                && let Some(content) = attr("content").filter(|c| !c.trim().is_empty())
            {
                return Some(unescape(content.trim()));
            }
        }
        None
    };
    let title = meta(&["og:title", "twitter:title"]).or_else(|| {
        let start = head.to_lowercase().find("<title")?;
        let open = head[start..].find('>')? + start + 1;
        let close = head[open..].to_lowercase().find("</title>")? + open;
        Some(unescape(head[open..close].trim()))
    })?;
    let title: String = title.split_whitespace().collect::<Vec<_>>().join(" ");
    if title.is_empty() {
        return None;
    }
    let description = meta(&["og:description", "description", "twitter:description"]);
    Some((title, description))
}

/// A page as the writing step reads it: what it says it is, and its main text.
#[derive(Clone, Debug, Default)]
pub struct Page {
    pub title: Option<String>,
    pub site: Option<String>,
    pub text: String,
}

/// The page at `url`, read once (8 seconds, 2 MB): its title, site name and main text, the
/// article or main element when it has one, without scripts, styles and navigation.
pub fn page_text(url: &str) -> std::result::Result<Page, String> {
    let mut url = url.to_owned();
    // A page that only redirects with `<meta http-equiv="refresh">` is followed, twice at most.
    for _ in 0..3 {
        match fetch_page(&url)? {
            Fetched::Page(page) => return Ok(page),
            Fetched::Refresh(next) => url = next,
        }
    }
    Err("redirects too many times".into())
}

enum Fetched {
    Page(Page),
    Refresh(String),
}

/// The target of a `<meta http-equiv="refresh" content="0; url=…">`, made absolute.
pub fn meta_refresh(html: &str, base: &str) -> Option<String> {
    static REFRESH: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let pattern = REFRESH.get_or_init(|| {
        regex::Regex::new(r#"(?is)<meta[^>]*http-equiv\s*=\s*["']?refresh["']?[^>]*content\s*=\s*["'][^"']*?url\s*=\s*([^"'\s>]+)"#)
            .expect("valid")
    });
    let target = pattern
        .captures(html)?
        .get(1)?
        .as_str()
        .trim_matches(['\'', '"'])
        .to_owned();
    if target.starts_with("http://") || target.starts_with("https://") {
        return Some(target);
    }
    let origin_end = base
        .find("://")
        .map(|at| at + 3)
        .and_then(|at| base[at..].find('/').map(|p| p + at))
        .unwrap_or(base.len());
    if target.starts_with('/') {
        Some(format!("{}{target}", &base[..origin_end]))
    } else {
        let dir_end = base
            .rfind('/')
            .filter(|at| *at >= origin_end)
            .unwrap_or(origin_end);
        Some(format!("{}/{target}", &base[..dir_end]))
    }
}

fn fetch_page(url: &str) -> std::result::Result<Fetched, String> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(4))
        .timeout(std::time::Duration::from_secs(8))
        .redirects(5)
        .user_agent("Mozilla/5.0 (FastMind; +https://github.com/coccor/FastMind)")
        .build();
    let response = agent.get(url).call().map_err(|error| match error {
        ureq::Error::Status(code, _) => format!("failed ({code})"),
        ureq::Error::Transport(_) => "couldn't be reached".to_owned(),
    })?;
    let kind = response.content_type().to_owned();
    let mut body = String::new();
    std::io::Read::read_to_string(
        &mut std::io::Read::take(response.into_reader(), 2_000_000),
        &mut body,
    )
    .map_err(|_| "couldn't be read".to_owned())?;
    if kind.starts_with("text/plain") || kind.contains("markdown") {
        return Ok(Fetched::Page(Page {
            title: None,
            site: None,
            text: body.chars().take(20_000).collect(),
        }));
    }
    if !kind.contains("html") {
        return Err(format!("not a web page ({kind})"));
    }
    if body.len() < 4000
        && let Some(next) = meta_refresh(&body, url)
    {
        return Ok(Fetched::Refresh(next));
    }
    Ok(Fetched::Page(html_to_page(&body)))
}

/// The readable part of an HTML page, as plain text with paragraphs kept.
pub fn html_to_page(html: &str) -> Page {
    use regex::Regex;
    use std::sync::OnceLock;
    static PARTS: OnceLock<(Vec<Regex>, Regex, Regex, Regex, Regex)> = OnceLock::new();
    let (noise, block, tag, blank, site) = PARTS.get_or_init(|| {
        let noise = [
            "script", "style", "noscript", "svg", "nav", "header", "footer", "aside", "form",
            "template", "iframe",
        ]
        .iter()
        .map(|name| Regex::new(&format!(r"(?is)<{name}\b.*?</{name}\s*>")).expect("valid"))
        .collect();
        (
            noise,
            Regex::new(r"(?i)<(/?(p|div|section|article|li|ul|ol|h[1-6]|tr|table|blockquote|pre)\b[^>]*|br\s*/?)>")
                .expect("valid"),
            Regex::new(r"(?s)<[^>]*>").expect("valid"),
            Regex::new(r"\n\s*\n(\s*\n)+").expect("valid"),
            Regex::new(r#"(?is)<meta\s[^>]*property\s*=\s*["']og:site_name["'][^>]*content\s*=\s*["']([^"']*)["']"#)
                .expect("valid"),
        )
    });
    let lower = html.to_lowercase();
    let title = {
        let start = lower.find("<title");
        start.and_then(|start| {
            let open = html[start..].find('>')? + start + 1;
            let close = lower[open..].find("</title>")? + open;
            let title = unescape(html[open..close].trim());
            let title: String = title.split_whitespace().collect::<Vec<_>>().join(" ");
            (!title.is_empty()).then_some(title)
        })
    };
    let site_name = site
        .captures(html)
        .and_then(|c| c.get(1))
        .map(|m| unescape(m.as_str().trim()));
    // The article, or the main element, or the body.
    let pick = |open: &str, close: &str| -> Option<&str> {
        let start = lower.find(open)?;
        let end = lower.rfind(close)?;
        (end > start).then(|| &html[start..end])
    };
    let mut part = pick("<article", "</article>")
        .or_else(|| pick("<main", "</main>"))
        .or_else(|| pick("<body", "</body>"))
        .unwrap_or(html)
        .to_owned();
    for pattern in noise {
        part = pattern.replace_all(&part, " ").into_owned();
    }
    let part = block.replace_all(&part, "\n");
    let part = tag.replace_all(&part, "");
    let lines: Vec<String> = unescape(&part)
        .lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect();
    let text = blank
        .replace_all(&lines.join("\n"), "\n\n")
        .trim()
        .to_owned();
    Page {
        title,
        site: site_name,
        text: text.chars().take(20_000).collect(),
    }
}

fn unescape(text: &str) -> String {
    text.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&nbsp;", " ")
        .replace("&#160;", " ")
        .replace("&mdash;", "—")
        .replace("&ndash;", "–")
        .replace("&rsquo;", "’")
        .replace("&lsquo;", "‘")
        .replace("&ldquo;", "“")
        .replace("&rdquo;", "”")
        .replace("&hellip;", "…")
}

/// After a link capture is saved: the page's own title and description go into the note, if
/// the note is still as it was saved.
pub fn enrich_link(nb: &Notebook, saved: &Saved, url: &str) -> Result<bool> {
    let Some((page_title, description)) = page_info(url) else {
        return Ok(false);
    };
    let path = nb.path(&saved.path)?;
    let text = fs::read_to_string(&path)?;
    let heading = format!("# {}\n", saved.title);
    // Only the note exactly as it was saved: once the user has touched it, it's theirs.
    if text::fnv(text.as_bytes()) != saved.written || !text.contains(&heading) {
        return Ok(false);
    }
    let title = text::excerpt(&page_title, 120);
    let mut replacement = format!("# {title}\n");
    if let Some(description) = description {
        replacement.push_str(&format!("\n> {}\n", text::excerpt(&description, 400)));
    }
    let updated = text.replacen(&heading, &replacement, 1);
    // Checked again right before writing: the page fetch took seconds.
    if fs::read(&path).map(|now| text::fnv(&now)).ok() != Some(saved.written) {
        return Ok(false);
    }
    fsx::write_atomic(&path, updated.as_bytes())?;
    Ok(true)
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

    #[test]
    fn a_page_is_read_without_its_chrome() {
        let html = "<html><head><title>Lag &amp; you</title><meta property=\"og:site_name\" content=\"Ex\"></head><body><nav>Home | About</nav><article><h1>Lag</h1><p>Lag grows when<br>consumers stall.</p><script>alert(1)</script><p>Second.</p></article><footer>(c)</footer></body></html>";
        let page = html_to_page(html);
        assert_eq!(page.title.as_deref(), Some("Lag & you"));
        assert_eq!(page.site.as_deref(), Some("Ex"));
        assert!(
            page.text.contains("Lag grows when\nconsumers stall."),
            "{}",
            page.text
        );
        assert!(page.text.contains("Second."));
        assert!(
            !page.text.contains("alert")
                && !page.text.contains("About")
                && !page.text.contains("(c)")
        );
    }

    #[test]
    fn meta_refresh_targets_are_made_absolute() {
        let html = "<noscript><meta http-equiv=\"refresh\" content=\"0; url=https://b.example/x/\"></noscript>";
        assert_eq!(
            meta_refresh(html, "https://a.example/y").as_deref(),
            Some("https://b.example/x/")
        );
        let html = "<meta http-equiv='refresh' content='0;URL=/new/page'>";
        assert_eq!(
            meta_refresh(html, "https://a.example/old/p.html").as_deref(),
            Some("https://a.example/new/page")
        );
        let html = "<meta http-equiv=refresh content=\"5; url=next.html\">";
        assert_eq!(
            meta_refresh(html, "https://a.example/d/p.html").as_deref(),
            Some("https://a.example/d/next.html")
        );
        assert_eq!(meta_refresh("<p>no</p>", "https://a.example"), None);
    }
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
    fn pasted_files_arrive_as_attachments() {
        let dir = tempfile::tempdir().unwrap();
        let nb = Notebook::open(dir.path()).unwrap();
        let saved = save(
            &nb,
            "inbox",
            &Capture {
                text: "The broken dialog".into(),
                files: vec![InlineFile {
                    name: "shot.png".into(),
                    data: "data:image/png;base64,iVBORw0KGgo=".into(),
                }],
                ..Default::default()
            },
        )
        .unwrap();
        let text = fs::read_to_string(nb.root.join(&saved.path)).unwrap();
        assert!(text.contains("![shot.png](attachments/shot.png)"), "{text}");
        assert_eq!(
            fs::read(nb.root.join("inbox/attachments/shot.png")).unwrap()[..4],
            [0x89, b'P', b'N', b'G']
        );
    }

    #[test]
    fn empty_capture_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let nb = Notebook::open(dir.path()).unwrap();
        assert!(save(&nb, "inbox", &Capture::default()).is_err());
    }
}
