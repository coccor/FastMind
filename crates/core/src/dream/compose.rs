//! How Dream writes notes: a merged note keeps everything the copies said that it didn't, and a
//! topic note gathers fragments under one title. Paragraph by paragraph, nothing new is invented
//! unless a model writes the title and summary.

use crate::frontmatter::{self, Value};
use crate::notebook::Note;
use crate::text;
use chrono::{DateTime, Local};
use std::collections::HashSet;

fn paragraphs(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut fenced = false;
    for line in body.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
        }
        if !fenced && line.trim().is_empty() {
            if !current.trim().is_empty() {
                out.push(current.trim_end().to_owned());
            }
            current.clear();
        } else {
            current.push_str(line);
            current.push('\n');
        }
    }
    if !current.trim().is_empty() {
        out.push(current.trim_end().to_owned());
    }
    out
}

/// The paragraphs of `note` worth keeping beside `known`: not its title heading, not already
/// said (word for word or nearly).
fn new_paragraphs(
    note: &Note,
    known: &mut HashSet<String>,
    known_shingles: &mut HashSet<u64>,
) -> Vec<String> {
    let mut kept = Vec::new();
    for paragraph in paragraphs(note.body()) {
        let trimmed = paragraph.trim();
        if let Some(heading) = trimmed.strip_prefix("# ")
            && (heading.trim().eq_ignore_ascii_case(&note.title) || !trimmed.contains('\n'))
        {
            continue;
        }
        let normal = text::normalize(trimmed);
        if normal.is_empty() || known.contains(&normal) {
            continue;
        }
        let shingles = text::shingles(trimmed, 3);
        if shingles.len() >= 4 {
            let seen = shingles
                .iter()
                .filter(|s| known_shingles.contains(s))
                .count();
            if seen as f64 / shingles.len() as f64 >= 0.7 {
                continue;
            }
        }
        known.insert(normal);
        known_shingles.extend(shingles);
        kept.push(trimmed.to_owned());
    }
    kept
}

fn seed_known(note: &Note) -> (HashSet<String>, HashSet<u64>) {
    let mut known = HashSet::new();
    let mut shingles = HashSet::new();
    for paragraph in paragraphs(note.body()) {
        known.insert(text::normalize(&paragraph));
        shingles.extend(text::shingles(&paragraph, 3));
    }
    (known, shingles)
}

fn date_of(note: &Note) -> String {
    note.created
        .unwrap_or(note.touched)
        .format("%Y-%m-%d")
        .to_string()
}

fn merged_list(base: Vec<String>, more: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen: HashSet<String> = base.iter().map(|item| item.to_lowercase()).collect();
    let mut out = base;
    for item in more {
        if !item.trim().is_empty() && seen.insert(item.to_lowercase()) {
            out.push(item);
        }
    }
    out
}

/// The Markdown link to a diagram file in `attachments/`, spaces written as `%20`.
pub fn diagram_link(file_name: &str) -> String {
    format!("attachments/{}", file_name.replace(' ', "%20"))
}

/// `note_text` with the diagram placed after its title and the summary quote that follows it,
/// or first in the body when it has no title.
pub fn with_diagram(note_text: &str, caption: &str, link: &str) -> String {
    let caption = caption.replace(['[', ']', '\n', '\r'], " ");
    let image = format!("![{}]({link})", caption.trim());
    let start = frontmatter::parse(note_text).body_start;
    let (front, body) = note_text.split_at(start);
    let mut offset = 0;
    let mut title_end = None;
    for line in body.split_inclusive('\n') {
        offset += line.len();
        if line.starts_with("# ") {
            title_end = Some(offset);
            break;
        }
    }
    let Some(mut at) = title_end else {
        return format!("{front}{image}\n\n{body}");
    };
    // A blockquote directly after the title (blank lines between allowed) stays above the image.
    let mut quote_end = None;
    let mut pos = at;
    for line in body[at..].split_inclusive('\n') {
        if line.trim().is_empty() && quote_end.is_none() {
            pos += line.len();
        } else if line.starts_with('>') {
            pos += line.len();
            quote_end = Some(pos);
        } else {
            break;
        }
    }
    if let Some(end) = quote_end {
        at = end;
    }
    let mut head = body[..at].to_owned();
    if !head.ends_with('\n') {
        head.push('\n');
    }
    format!("{front}{head}\n{image}\n{}", &body[at..])
}

/// `keep` with what `absorb` added, their titles as aliases so links and searches still find them.
pub fn merge(keep: &Note, absorb: &[&Note], now: DateTime<Local>) -> String {
    let (mut known, mut known_shingles) = seed_known(keep);
    let mut text = keep.text.trim_end().to_owned();
    text.push('\n');
    for other in absorb {
        let added = new_paragraphs(other, &mut known, &mut known_shingles);
        if !added.is_empty() {
            text.push_str(&format!(
                "\n## From \"{}\" ({})\n\n",
                other.title,
                date_of(other)
            ));
            text.push_str(&added.join("\n\n"));
            text.push('\n');
        }
    }
    let aliases = merged_list(
        keep.front.aliases(),
        absorb.iter().flat_map(|other| {
            let mut names = vec![other.title.clone()];
            if other.stem() != other.title {
                names.push(other.stem().to_owned());
            }
            names.extend(other.front.aliases());
            names
        }),
    );
    let aliases: Vec<String> = aliases
        .into_iter()
        .filter(|alias| alias != &keep.title && alias != keep.stem())
        .collect();
    let tags = merged_list(
        keep.front.tags(),
        absorb.iter().flat_map(|other| other.front.tags()),
    );
    if !aliases.is_empty() {
        text = frontmatter::set(&text, "aliases", &Value::List(aliases));
    }
    if !tags.is_empty() {
        text = frontmatter::set(&text, "tags", &Value::List(tags));
        text = frontmatter::remove(&text, "tag");
    }
    frontmatter::set(
        &text,
        "updated",
        &Value::Text(now.format("%Y-%m-%d").to_string()),
    )
}

/// A topic note gathering `fragments` under `title`, with `summary` on top.
pub fn topic(title: &str, summary: &str, fragments: &[&Note], now: DateTime<Local>) -> String {
    let tags = merged_list(
        Vec::new(),
        fragments.iter().flat_map(|note| note.front.tags()),
    );
    let aliases = merged_list(Vec::new(), fragments.iter().map(|note| note.title.clone()));
    let mut front = vec![
        "---".to_owned(),
        "type: topic".to_owned(),
        format!("created: {}", now.format("%Y-%m-%d")),
        frontmatter::render("aliases", &Value::List(aliases)),
    ];
    if !tags.is_empty() {
        front.push(frontmatter::render("tags", &Value::List(tags)));
    }
    front.push("dreamed: true".to_owned());
    front.push("---".to_owned());

    let mut body = format!("# {title}\n\n");
    if !summary.trim().is_empty() {
        body.push_str(&format!("> {}\n\n", summary.trim().replace('\n', "\n> ")));
    }
    let mut known = HashSet::new();
    let mut known_shingles = HashSet::new();
    let mut ordered: Vec<&&Note> = fragments.iter().collect();
    ordered.sort_by_key(|note| note.created.unwrap_or(note.touched));
    for note in ordered {
        let kept = new_paragraphs(note, &mut known, &mut known_shingles);
        body.push_str(&format!("## {}\n\n*{}*\n\n", note.title, date_of(note)));
        if kept.is_empty() {
            body.push_str("(Nothing beyond what's above.)\n\n");
        } else {
            body.push_str(&kept.join("\n\n"));
            body.push_str("\n\n");
        }
    }
    format!(
        "{}\n{}",
        front.join("\n"),
        body.trim_end().to_owned() + "\n"
    )
}

/// Sentences closest to what the group is about, as a short summary written from the notes.
pub fn extract_summary(fragments: &[&Note], words: &[String], max: usize) -> String {
    let wanted: HashSet<String> = words
        .iter()
        .map(|word| text::stem(&word.to_lowercase()))
        .collect();
    let mut sentences: Vec<(f64, usize, String)> = Vec::new();
    for (order, note) in fragments.iter().enumerate() {
        let body = note.body();
        for raw in body.split(['.', '!', '?', '\n']) {
            let sentence = raw
                .trim()
                .trim_start_matches(['#', '-', '*', '>', ' '])
                .trim();
            let count = text::words(sentence).count();
            if !(5..=40).contains(&count) || sentence.eq_ignore_ascii_case(&note.title) {
                continue;
            }
            let terms: HashSet<String> = text::terms(sentence).into_iter().collect();
            let hits = terms.intersection(&wanted).count() as f64;
            if hits == 0.0 {
                continue;
            }
            sentences.push((hits / (count as f64).sqrt(), order, sentence.to_owned()));
        }
    }
    sentences.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
    let mut seen = HashSet::new();
    let mut from = HashSet::new();
    let picked: Vec<String> = sentences
        .into_iter()
        .filter(|(_, order, sentence)| {
            from.insert(*order) && seen.insert(text::normalize(sentence))
        })
        .take(max)
        .map(|(_, _, sentence)| format!("{sentence}."))
        .collect();
    picked.join(" ")
}

/// A title from naming words: "Kubernetes, probes and pods".
pub fn title_from_words(words: &[String]) -> String {
    let mut words: Vec<String> = words
        .iter()
        .filter(|word| !word.is_empty())
        .cloned()
        .collect();
    if words.is_empty() {
        return "Gathered notes".into();
    }
    if let Some(first) = words.first_mut() {
        let mut chars = first.chars();
        if let Some(head) = chars.next() {
            *first = head.to_uppercase().collect::<String>() + chars.as_str();
        }
    }
    match words.len() {
        1 => words.remove(0),
        2 => format!("{} and {}", words[0], words[1]),
        _ => {
            let last = words.pop().unwrap_or_default();
            format!("{} and {last}", words.join(", "))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notebook::build_note;

    fn note(path: &str, text: &str) -> Note {
        build_note(path.into(), text.into(), text.len() as u64, Local::now())
    }

    #[test]
    fn with_diagram_after_title_and_summary() {
        assert_eq!(
            with_diagram("# T\n\n> s\n\nbody", "c", "attachments/T.fdraw"),
            "# T\n\n> s\n\n![c](attachments/T.fdraw)\n\nbody"
        );
        assert_eq!(
            with_diagram("# T\n\nbody", "c", "attachments/T.fdraw"),
            "# T\n\n![c](attachments/T.fdraw)\n\nbody"
        );
    }

    #[test]
    fn with_diagram_no_heading() {
        assert_eq!(
            with_diagram("---\ntags: [a]\n---\nbody\n", "c", "attachments/x.fdraw"),
            "---\ntags: [a]\n---\n![c](attachments/x.fdraw)\n\nbody\n"
        );
    }

    #[test]
    fn diagram_link_encodes_spaces() {
        assert_eq!(diagram_link("Note 2.fdraw"), "attachments/Note%202.fdraw");
    }

    #[test]
    fn merge_adds_only_what_is_new_and_keeps_names() {
        let keep = note(
            "Redis.md",
            "---\ntags: [redis]\n---\n# Redis eviction\n\nUse allkeys-lru.\n\nSet maxmemory.\n",
        );
        let other = note(
            "Redis copy.md",
            "---\ntags: [cache]\n---\n# Redis eviction notes\n\nUse allkeys-lru.\n\nWatch evicted_keys in INFO stats.\n",
        );
        let merged = merge(&keep, &[&other], Local::now());
        let front = frontmatter::parse(&merged);
        assert_eq!(front.tags(), vec!["redis", "cache"]);
        assert!(front.aliases().contains(&"Redis eviction notes".to_owned()));
        assert_eq!(merged.matches("Use allkeys-lru.").count(), 1);
        assert!(merged.contains("## From \"Redis eviction notes\""));
        assert!(merged.contains("Watch evicted_keys in INFO stats."));
    }

    #[test]
    fn topic_gathers_fragments() {
        let a = note("a.md", "# Probe A\n\nReadiness probes gate traffic.\n");
        let b = note("b.md", "# Probe B\n\nLiveness probes restart pods.\n");
        let text = topic(
            "Kubernetes probes",
            "Probes keep pods healthy.",
            &[&a, &b],
            Local::now(),
        );
        let front = frontmatter::parse(&text);
        assert_eq!(front.text("type"), Some("topic"));
        assert_eq!(front.aliases(), vec!["Probe A", "Probe B"]);
        assert!(text.contains("## Probe A") && text.contains("Liveness probes restart pods."));
        assert!(text.contains("> Probes keep pods healthy."));
    }

    #[test]
    fn titles_read_naturally() {
        assert_eq!(
            title_from_words(&["kubernetes".into(), "probes".into(), "pods".into()]),
            "Kubernetes, probes and pods"
        );
        assert_eq!(title_from_words(&["redis".into()]), "Redis");
    }
}
