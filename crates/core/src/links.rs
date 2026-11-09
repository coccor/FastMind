//! Links between notes: `[[wikilinks]]` (with `#heading` and `|display` parts) and Markdown
//! links to `.md` files. Text in code fences is never a link.

use crate::notebook::Note;
use std::collections::HashMap;

/// The link targets in `body`, as written (no `#heading` or `|display`).
pub fn targets(body: &str) -> Vec<String> {
    let mut found = Vec::new();
    for_each_link(body, |link| found.push(link.target.to_owned()));
    found
}

struct Link<'a> {
    /// Byte range of the target inside the text.
    start: usize,
    end: usize,
    target: &'a str,
    /// The whole `[[...]]` has its own display text.
    has_display: bool,
    wiki: bool,
}

fn for_each_link<'a>(body: &'a str, mut visit: impl FnMut(Link<'a>)) {
    let mut fenced = false;
    let mut offset = 0;
    for line in body.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fenced = !fenced;
        } else if !fenced {
            scan_line(line, offset, &mut visit);
        }
        offset += line.len();
    }
}

fn scan_line<'a>(line: &'a str, offset: usize, visit: &mut impl FnMut(Link<'a>)) {
    let bytes = line.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'`' {
            match line[at + 1..].find('`') {
                Some(close) => {
                    at += close + 2;
                    continue;
                }
                None => return,
            }
        }
        if line[at..].starts_with("[[")
            && let Some(close) = line[at + 2..].find("]]")
        {
            let inner = &line[at + 2..at + 2 + close];
            let target_len = inner.find(['|', '#', '^']).unwrap_or(inner.len());
            let target = inner[..target_len].trim_end();
            if !target.is_empty() && !inner.contains('\n') {
                visit(Link {
                    start: offset + at + 2,
                    end: offset + at + 2 + target.len(),
                    target,
                    has_display: inner.contains('|'),
                    wiki: true,
                });
            }
            at += close + 4;
            continue;
        }
        if bytes[at] == b']'
            && line[at..].starts_with("](")
            && let Some(close) = line[at + 2..].find(')')
        {
            let raw = &line[at + 2..at + 2 + close];
            let raw = raw.split_whitespace().next().unwrap_or("");
            let path = raw.split('#').next().unwrap_or("");
            if path.to_ascii_lowercase().ends_with(".md") && !path.contains("://") {
                visit(Link {
                    start: offset + at + 2,
                    end: offset + at + 2 + path.len(),
                    target: path,
                    has_display: true,
                    wiki: false,
                });
            }
            at += close + 3;
            continue;
        }
        at += line[at..].chars().next().map_or(1, char::len_utf8);
    }
}

/// Finds the note a link points to by file name, path, title or alias, ignoring case.
pub struct Resolver<'a> {
    by_name: HashMap<String, &'a str>,
    by_path: HashMap<String, &'a str>,
}

impl<'a> Resolver<'a> {
    pub fn new(notes: &'a [Note]) -> Self {
        let mut by_name = HashMap::new();
        let mut by_path = HashMap::new();
        // File names win over titles and aliases: insert them last.
        for note in notes {
            for name in note.names() {
                by_name.entry(name).or_insert(note.path.as_str());
            }
        }
        for note in notes {
            by_name.insert(note.stem().to_lowercase(), note.path.as_str());
            let lower = note.path.to_lowercase();
            by_path.insert(lower.trim_end_matches(".md").to_owned(), note.path.as_str());
        }
        Self { by_name, by_path }
    }

    pub fn resolve(&self, target: &str) -> Option<&'a str> {
        let decoded = decode(target);
        let lower = decoded.trim().to_lowercase();
        let lower = lower.trim_start_matches("./");
        let without_ext = lower.strip_suffix(".md").unwrap_or(lower);
        if let Some(path) = self.by_path.get(without_ext) {
            return Some(path);
        }
        let name = without_ext.rsplit('/').next().unwrap_or(without_ext);
        if let Some(path) = self.by_name.get(name) {
            return Some(path);
        }
        self.by_name.get(lower).copied()
    }

    /// Every name a link may use, for suggesting a fix to a broken link.
    pub fn names(&self) -> impl Iterator<Item = (&String, &&'a str)> {
        self.by_name.iter()
    }
}

/// `body` with every link that `points_to` accepts pointed at `new_stem` instead. A wikilink
/// keeps what it showed: `[[Old]]` becomes `[[New|Old]]`.
pub fn retarget(body: &str, points_to: impl Fn(&str) -> bool, new_stem: &str) -> String {
    retarget_with(body, points_to, new_stem, true)
}

/// Like `retarget`; with `keep_shown` false a plain `[[Typo]]` becomes plain `[[New]]` (fixing a
/// mistyped link shouldn't keep showing the typo).
pub fn retarget_with(
    body: &str,
    points_to: impl Fn(&str) -> bool,
    new_stem: &str,
    keep_shown: bool,
) -> String {
    rewrite(
        body,
        points_to,
        new_stem,
        keep_shown,
        &encode(&format!("{new_stem}.md")),
    )
}

/// Points links in the note at `from_path` that `points_to` accepts at the note now at
/// `new_path`: wikilinks by its name, Markdown links by its path relative to `from_path`.
pub fn retarget_note(
    body: &str,
    points_to: impl Fn(&str) -> bool,
    new_path: &str,
    from_path: &str,
) -> String {
    let name = new_path.rsplit('/').next().unwrap_or(new_path);
    let stem = name.strip_suffix(".md").unwrap_or(name);
    rewrite(
        body,
        points_to,
        stem,
        true,
        &encode(&relative_path(from_path, new_path)),
    )
}

/// `to` as a path relative to the folder of `from` (both notebook-relative, `/`-separated).
pub fn relative_path(from: &str, to: &str) -> String {
    let from_dir: Vec<&str> = from
        .split('/')
        .collect::<Vec<_>>()
        .split_last()
        .map(|(_, dir)| dir.to_vec())
        .unwrap_or_default();
    let to_parts: Vec<&str> = to.split('/').collect();
    let common = from_dir
        .iter()
        .zip(&to_parts)
        .take_while(|(a, b)| a == b)
        .count();
    let mut parts: Vec<&str> = vec![".."; from_dir.len() - common];
    parts.extend(&to_parts[common..]);
    parts.join("/")
}

fn rewrite(
    body: &str,
    points_to: impl Fn(&str) -> bool,
    new_stem: &str,
    keep_shown: bool,
    md_target: &str,
) -> String {
    let mut edits: Vec<(usize, usize, String)> = Vec::new();
    for_each_link(body, |link| {
        if !points_to(link.target) {
            return;
        }
        let replacement = if link.wiki {
            if link.has_display || !keep_shown || link.target.eq_ignore_ascii_case(new_stem) {
                new_stem.to_owned()
            } else {
                // Put the display after any #heading: insert it at the closing brackets instead.
                format!("{new_stem}\u{0}{}", link.target)
            }
        } else {
            md_target.to_owned()
        };
        edits.push((link.start, link.end, replacement));
    });
    let mut out = body.to_owned();
    for (start, end, replacement) in edits.into_iter().rev() {
        if let Some((stem, display)) = replacement.split_once('\u{0}') {
            let close = out[end..].find("]]").map(|at| end + at).unwrap_or(end);
            out.replace_range(close..close, &format!("|{display}"));
            out.replace_range(start..end, stem);
        } else {
            out.replace_range(start..end, &replacement);
        }
    }
    out
}

fn decode(text: &str) -> String {
    if !text.contains('%') {
        return text.to_owned();
    }
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Some(byte) = text
                .get(i + 1..i + 3)
                .and_then(|hex| u8::from_str_radix(hex, 16).ok())
        {
            out.push(byte);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn encode(text: &str) -> String {
    text.replace('%', "%25")
        .replace(' ', "%20")
        .replace('(', "%28")
        .replace(')', "%29")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notebook::build_note;
    use chrono::Local;

    fn note(path: &str, text: &str) -> Note {
        build_note(path.into(), text.into(), text.len() as u64, Local::now())
    }

    #[test]
    fn finds_wiki_and_markdown_links_outside_code() {
        let body = "See [[Alpha]], [[Beta#Part|b]] and [g](sub/Gamma%20One.md).\n```\n[[NotALink]]\n```\n`[[no]]`";
        assert_eq!(targets(body), vec!["Alpha", "Beta", "sub/Gamma%20One.md"]);
    }

    #[test]
    fn resolves_by_name_title_alias_and_path() {
        let notes = vec![
            note("a/Alpha.md", "# The Alpha\n"),
            note("b/Beta.md", "---\naliases: [B note]\n---\n"),
            note("sub/Gamma One.md", ""),
        ];
        let resolver = Resolver::new(&notes);
        assert_eq!(resolver.resolve("alpha"), Some("a/Alpha.md"));
        assert_eq!(resolver.resolve("The Alpha"), Some("a/Alpha.md"));
        assert_eq!(resolver.resolve("B note"), Some("b/Beta.md"));
        assert_eq!(
            resolver.resolve("sub/Gamma%20One.md"),
            Some("sub/Gamma One.md")
        );
        assert_eq!(resolver.resolve("missing"), None);
    }

    #[test]
    fn markdown_links_get_paths_relative_to_their_note() {
        assert_eq!(
            relative_path("Index.md", "work/Plan 2.md"),
            "work/Plan 2.md"
        );
        assert_eq!(relative_path("ideas/a.md", "work/b.md"), "../work/b.md");
        assert_eq!(relative_path("work/x/a.md", "work/b.md"), "../b.md");
        let out = retarget_note(
            "[p](inbox/Plan.md) [[Plan]]",
            |t| t.contains("Plan"),
            "work/Plan 2.md",
            "Index.md",
        );
        assert_eq!(out, "[p](work/Plan%202.md) [[Plan 2|Plan]]");
    }

    #[test]
    fn retarget_keeps_display_text() {
        let body = "[[Old]] [[Old#H]] [[Old|shown]] [x](Old.md) [[Other]]";
        let out = retarget(body, |t| t.trim_end_matches(".md") == "Old", "New Name");
        assert_eq!(
            out,
            "[[New Name|Old]] [[New Name#H|Old]] [[New Name|shown]] [x](New%20Name.md) [[Other]]"
        );
    }
}
