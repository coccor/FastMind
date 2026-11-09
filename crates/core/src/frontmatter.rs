//! Front matter: the YAML block between `---` lines at the top of a note.
//!
//! Only the subset notes use is read: `key: value`, quoted strings, flow lists (`[a, b]`) and
//! block lists (`- a`). Anything else is kept as written for its key and is never an error.
//! Editing rewrites only the lines of the key being changed, so the rest of the block stays as
//! the user or another tool wrote it.

const MAX_LINES: usize = 200;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    Text(String),
    List(Vec<String>),
    Raw(String),
}

impl Value {
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Value::Text(text) => Some(text),
            _ => None,
        }
    }

    pub fn items(&self) -> Vec<String> {
        match self {
            Value::List(items) => items.clone(),
            Value::Text(text) => text
                .split([',', ' '])
                .map(|item| item.trim().to_owned())
                .filter(|item| !item.is_empty())
                .collect(),
            Value::Raw(_) => Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FrontMatter {
    pub entries: Vec<(String, Value)>,
    /// Byte offset where the body starts (0 when the note has no front matter).
    pub body_start: usize,
    pub present: bool,
}

impl FrontMatter {
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.entries
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(key))
            .map(|(_, value)| value)
    }

    pub fn text(&self, key: &str) -> Option<&str> {
        self.get(key)
            .and_then(Value::as_text)
            .filter(|text| !text.is_empty())
    }

    pub fn list(&self, key: &str) -> Vec<String> {
        self.get(key).map(Value::items).unwrap_or_default()
    }

    pub fn tags(&self) -> Vec<String> {
        let mut tags = self.list("tags");
        tags.extend(self.list("tag"));
        tags.into_iter()
            .map(|tag| tag.trim_start_matches('#').to_owned())
            .filter(|tag| !tag.is_empty())
            .collect()
    }

    pub fn aliases(&self) -> Vec<String> {
        let mut aliases = self.list_exact("aliases");
        aliases.extend(self.list_exact("alias"));
        aliases
    }

    /// A list key whose plain-text form is one item, not split on spaces (`alias: My note`).
    fn list_exact(&self, key: &str) -> Vec<String> {
        match self.get(key) {
            Some(Value::List(items)) => items.clone(),
            Some(Value::Text(text)) if !text.is_empty() => vec![text.clone()],
            _ => Vec::new(),
        }
    }
}

/// Reads the front matter at the top of `text`. A note without one gets an empty, absent block.
pub fn parse(text: &str) -> FrontMatter {
    let offset = if text.starts_with('\u{feff}') { 3 } else { 0 };
    let mut lines = Lines { text, at: offset };
    match lines.next() {
        Some((_, "---")) => {}
        _ => return FrontMatter::default(),
    }
    let mut block = Vec::new();
    for (index, (end, line)) in lines.by_ref().enumerate() {
        if index >= MAX_LINES {
            return FrontMatter::default();
        }
        if line == "---" || line == "..." {
            return FrontMatter {
                entries: entries(&block),
                body_start: end,
                present: true,
            };
        }
        block.push(line);
    }
    FrontMatter::default()
}

/// The note's body, after its front matter.
pub fn body(text: &str) -> &str {
    &text[parse(text).body_start..]
}

/// `text` with `key` set to `value`, replacing the key's current lines or adding it at the end
/// of the block. A note with no front matter gets one.
pub fn set(text: &str, key: &str, value: &Value) -> String {
    let rendered = render(key, value);
    let front = parse(text);
    if !front.present {
        let body = text.strip_prefix('\u{feff}').unwrap_or(text);
        return format!("---\n{rendered}\n---\n{body}");
    }
    let (head, body) = text.split_at(front.body_start);
    let mut out: Vec<String> = Vec::new();
    let mut replaced = false;
    let mut skipping = false;
    let lines: Vec<&str> = head.lines().collect();
    let last = lines.len().saturating_sub(1);
    for (index, line) in lines.iter().enumerate() {
        if index == 0 || index == last {
            if index == last && !replaced {
                out.push(rendered.clone());
                replaced = true;
            }
            out.push((*line).to_owned());
            continue;
        }
        if skipping {
            if line.starts_with([' ', '\t'])
                || line.trim_start().starts_with("- ")
                || line.trim() == "-"
            {
                continue;
            }
            skipping = false;
        }
        if key_of(line).is_some_and(|(name, _)| name.eq_ignore_ascii_case(key)) {
            if !replaced {
                out.push(rendered.clone());
                replaced = true;
            }
            skipping = true;
            continue;
        }
        out.push((*line).to_owned());
    }
    format!("{}\n{}", out.join("\n"), body)
}

/// `text` without `key` in its front matter.
pub fn remove(text: &str, key: &str) -> String {
    let front = parse(text);
    if !front.present || front.get(key).is_none() {
        return text.to_owned();
    }
    let (head, body) = text.split_at(front.body_start);
    let mut out = Vec::new();
    let mut skipping = false;
    for line in head.lines() {
        if skipping {
            if line.starts_with([' ', '\t'])
                || line.trim_start().starts_with("- ")
                || line.trim() == "-"
            {
                continue;
            }
            skipping = false;
        }
        if key_of(line).is_some_and(|(name, _)| name.eq_ignore_ascii_case(key)) {
            skipping = true;
            continue;
        }
        out.push(line);
    }
    format!("{}\n{}", out.join("\n"), body)
}

pub fn render(key: &str, value: &Value) -> String {
    match value {
        Value::Text(text) => format!("{key}: {}", quote(text)),
        Value::List(items) => {
            let items: Vec<String> = items.iter().map(|item| quote(item)).collect();
            format!("{key}: [{}]", items.join(", "))
        }
        Value::Raw(raw) => format!("{key}:{raw}"),
    }
}

/// A scalar as YAML: bare when that reads back the same, otherwise double-quoted.
pub fn quote(text: &str) -> String {
    let plain = !text.is_empty()
        && !text.starts_with([
            ' ', '"', '\'', '[', ']', '{', '}', '#', '&', '*', '!', '|', '>', '%', '@', '`', '-',
            '?',
        ])
        && !text.ends_with(' ')
        && !text.contains(": ")
        && !text.contains(" #")
        && !text.contains([',', '\n', '\r', '\t'])
        && !matches!(
            text.to_ascii_lowercase().as_str(),
            "true" | "false" | "null" | "yes" | "no" | "~"
        );
    if plain {
        text.to_owned()
    } else {
        let escaped = text
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
            .replace('\r', "")
            .replace('\t', "\\t");
        format!("\"{escaped}\"")
    }
}

struct Lines<'a> {
    text: &'a str,
    at: usize,
}

impl<'a> Iterator for Lines<'a> {
    type Item = (usize, &'a str);

    fn next(&mut self) -> Option<Self::Item> {
        if self.at >= self.text.len() {
            return None;
        }
        let rest = &self.text[self.at..];
        let (line, advance) = match rest.find('\n') {
            Some(newline) => (&rest[..newline], newline + 1),
            None => (rest, rest.len()),
        };
        self.at += advance;
        Some((self.at, line.strip_suffix('\r').unwrap_or(line)))
    }
}

fn key_of(line: &str) -> Option<(&str, &str)> {
    if line.starts_with([' ', '\t', '-', '#']) {
        return None;
    }
    let colon = line.find(':')?;
    let key = &line[..colon];
    if key.is_empty() || key.contains([' ', '\t', '#']) {
        return None;
    }
    let rest = &line[colon + 1..];
    if !rest.is_empty() && !rest.starts_with([' ', '\t']) {
        return None;
    }
    Some((key, rest))
}

fn entries(block: &[&str]) -> Vec<(String, Value)> {
    let mut entries = Vec::new();
    let mut index = 0;
    while index < block.len() {
        let Some((key, rest)) = key_of(block[index]) else {
            index += 1;
            continue;
        };
        index += 1;
        let rest = strip_comment(rest.trim());
        if rest.is_empty() {
            let mut items = Vec::new();
            let mut raw = String::new();
            while index < block.len()
                && (block[index].starts_with([' ', '\t', '-']) || block[index].is_empty())
            {
                let line = block[index];
                let trimmed = line.trim();
                if let Some(item) =
                    trimmed
                        .strip_prefix("- ")
                        .or(if trimmed == "-" { Some("") } else { None })
                {
                    items.push(unquote(item.trim()));
                } else if !trimmed.is_empty() {
                    raw.push('\n');
                    raw.push_str(line);
                }
                index += 1;
            }
            let value = if !raw.is_empty() {
                Value::Raw(raw)
            } else if !items.is_empty() {
                Value::List(items.into_iter().filter(|item| !item.is_empty()).collect())
            } else {
                Value::Text(String::new())
            };
            entries.push((key.to_owned(), value));
        } else if rest.starts_with('[') && rest.ends_with(']') {
            let items = split_flow(&rest[1..rest.len() - 1]);
            entries.push((key.to_owned(), Value::List(items)));
        } else if rest.starts_with(['|', '>', '{']) {
            let mut raw = format!(" {rest}");
            while index < block.len()
                && (block[index].starts_with([' ', '\t']) || block[index].is_empty())
            {
                raw.push('\n');
                raw.push_str(block[index]);
                index += 1;
            }
            entries.push((key.to_owned(), Value::Raw(raw)));
        } else {
            entries.push((key.to_owned(), Value::Text(unquote(rest))));
        }
    }
    entries
}

fn strip_comment(text: &str) -> &str {
    if text.starts_with(['"', '\'']) {
        return text;
    }
    match text.find(" #") {
        Some(at) => text[..at].trim_end(),
        None => text,
    }
}

fn split_flow(inner: &str) -> Vec<String> {
    let mut items = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for ch in inner.chars() {
        if let Some(q) = quote {
            current.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' && q == '"' {
                escaped = true;
            } else if ch == q {
                quote = None;
            }
            continue;
        }
        match ch {
            '"' | '\'' => {
                quote = Some(ch);
                current.push(ch);
            }
            ',' => {
                items.push(unquote(current.trim()));
                current.clear();
            }
            _ => current.push(ch),
        }
    }
    if !current.trim().is_empty() {
        items.push(unquote(current.trim()));
    }
    items.into_iter().filter(|item| !item.is_empty()).collect()
}

fn unquote(text: &str) -> String {
    if text.len() >= 2 && text.starts_with('"') && text.ends_with('"') {
        let inner = &text[1..text.len() - 1];
        let mut out = String::new();
        let mut chars = inner.chars();
        while let Some(ch) = chars.next() {
            if ch == '\\' {
                match chars.next() {
                    Some('n') => out.push('\n'),
                    Some('t') => out.push('\t'),
                    Some(other) => out.push(other),
                    None => {}
                }
            } else {
                out.push(ch);
            }
        }
        out
    } else if text.len() >= 2 && text.starts_with('\'') && text.ends_with('\'') {
        text[1..text.len() - 1].replace("''", "'")
    } else {
        text.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOTE: &str = "---\ntitle: \"A: note\"\ntags: [rust, \"db\"]\naliases:\n  - Old name\nstatus: inbox\n---\n# Body\n";

    #[test]
    fn reads_scalars_and_lists() {
        let front = parse(NOTE);
        assert!(front.present);
        assert_eq!(front.text("title"), Some("A: note"));
        assert_eq!(front.tags(), vec!["rust", "db"]);
        assert_eq!(front.aliases(), vec!["Old name"]);
        assert_eq!(body(NOTE), "# Body\n");
    }

    #[test]
    fn set_replaces_and_appends() {
        let changed = set(NOTE, "status", &Value::Text("kept".into()));
        assert_eq!(parse(&changed).text("status"), Some("kept"));
        assert_eq!(parse(&changed).aliases(), vec!["Old name"]);
        let changed = set(
            &changed,
            "aliases",
            &Value::List(vec!["Old name".into(), "Other".into()]),
        );
        assert_eq!(parse(&changed).aliases(), vec!["Old name", "Other"]);
        assert_eq!(body(&changed), "# Body\n");
        let added = set(&changed, "archived", &Value::Text("2026-01-01".into()));
        assert_eq!(parse(&added).text("archived"), Some("2026-01-01"));
        assert_eq!(parse(&added).text("title"), Some("A: note"));
    }

    #[test]
    fn set_creates_front_matter() {
        let changed = set("# Hi\n", "status", &Value::Text("inbox".into()));
        assert_eq!(changed, "---\nstatus: inbox\n---\n# Hi\n");
    }

    #[test]
    fn remove_drops_block_lists() {
        let changed = remove(NOTE, "aliases");
        let front = parse(&changed);
        assert!(front.get("aliases").is_none());
        assert_eq!(front.text("status"), Some("inbox"));
    }

    #[test]
    fn remove_takes_bare_dashes_with_the_list() {
        let text = "---\ntags:\n- a\n-\ntitle: x\n---\nbody";
        assert_eq!(remove(text, "tags"), "---\ntitle: x\n---\nbody");
    }

    #[test]
    fn quoting_round_trips() {
        for text in [
            "plain",
            "a: b",
            "true",
            "x, y",
            "#tag",
            "quote \" inside",
            "- dash",
        ] {
            let note = set("", "k", &Value::Text(text.into()));
            assert_eq!(parse(&note).text("k"), Some(text), "{note}");
        }
    }

    #[test]
    fn unclosed_block_is_body() {
        let text = "---\ntitle: x\nno end";
        assert!(!parse(text).present);
        assert_eq!(body(text), text);
    }
}
