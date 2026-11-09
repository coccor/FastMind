//! Remember's search: ranked full-text search over titles, aliases, tags and bodies (BM25), with
//! typo tolerance and prefix matching for the word being typed. Built in memory from the files,
//! so it can always be rebuilt and never needs a database.

use crate::notebook::Note;
use crate::text;
use chrono::{DateTime, Local};
use serde::Serialize;
use std::collections::HashMap;

#[derive(Clone, Debug)]
pub struct Doc {
    pub path: String,
    pub title: String,
    pub archived: Option<String>,
    pub touched: DateTime<Local>,
    pub body: String,
    /// Line in the file where the body starts.
    body_line: usize,
    terms: HashMap<String, f32>,
    len: f32,
    tags: Vec<String>,
    kind: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Hit {
    pub path: String,
    pub title: String,
    pub score: f32,
    pub snippet: String,
    /// 1-based line in the file the snippet comes from.
    pub line: usize,
    /// The archive item id, when the hit is an archived note.
    pub archived: Option<String>,
    pub tags: Vec<String>,
    /// The note's `type:` (`ai-session`, `topic`, …), empty when it has none.
    pub kind: String,
}

#[derive(Default)]
pub struct Index {
    docs: Vec<Doc>,
    df: HashMap<String, usize>,
    avg_len: f32,
    /// Every known word, sorted, for prefix and typo lookups.
    vocab: Vec<String>,
}

const TITLE: f32 = 4.0;
const ALIAS: f32 = 3.0;
const TAG: f32 = 2.5;

impl Index {
    pub fn build(notes: &[Note], archived: Vec<(String, Note)>) -> Self {
        let mut docs: Vec<Doc> = notes.iter().map(|note| doc(note, None)).collect();
        docs.extend(
            archived
                .iter()
                .map(|(id, note)| doc(note, Some(id.clone()))),
        );
        Self::from_docs(docs)
    }

    /// An index over documents already made, so unchanged notes needn't be read again.
    pub fn from_docs(docs: Vec<Doc>) -> Self {
        let mut df: HashMap<String, usize> = HashMap::new();
        for doc in &docs {
            for term in doc.terms.keys() {
                *df.entry(term.clone()).or_default() += 1;
            }
        }
        let avg_len = if docs.is_empty() {
            1.0
        } else {
            docs.iter().map(|doc| doc.len).sum::<f32>() / docs.len() as f32
        };
        let mut vocab: Vec<String> = df.keys().cloned().collect();
        vocab.sort();
        Self {
            docs,
            df,
            avg_len: avg_len.max(1.0),
            vocab,
        }
    }

    /// Known words starting with `prefix`, from the sorted vocabulary.
    fn with_prefix<'a>(&'a self, prefix: &'a str) -> impl Iterator<Item = &'a String> + 'a {
        let start = self.vocab.partition_point(|word| word.as_str() < prefix);
        self.vocab[start..]
            .iter()
            .take_while(move |word| word.starts_with(prefix))
    }

    pub fn len(&self) -> usize {
        self.docs
            .iter()
            .filter(|doc| doc.archived.is_none())
            .count()
    }

    pub fn is_empty(&self) -> bool {
        self.docs.is_empty()
    }

    pub fn doc(&self, path: &str) -> Option<&Doc> {
        self.docs
            .iter()
            .find(|doc| doc.path == path && doc.archived.is_none())
    }

    /// Query terms with the weight each counts for: exact words first, then words the index
    /// knows within a typo or two, and words starting with the last, unfinished word.
    fn expand(&self, query: &str) -> Vec<(String, f32)> {
        let words: Vec<String> = text::words(query).collect();
        let mut expanded: Vec<(String, f32)> = Vec::new();
        let last = words.len().saturating_sub(1);
        for (index, word) in words.iter().enumerate() {
            if word.chars().count() < 2 || text::is_stopword(word) {
                continue;
            }
            let term = text::stem(word);
            if self.df.contains_key(&term) {
                expanded.push((term.clone(), 1.0));
            } else if word.chars().count() >= 4 {
                // A typo: words starting with the same letter, about as long.
                let limit = if word.chars().count() >= 7 { 2 } else { 1 };
                let length = term.chars().count();
                let first: String = term.chars().take(1).collect();
                for known in self.with_prefix(&first) {
                    if known.chars().count().abs_diff(length) <= limit
                        && text::within_edits(known, &term, limit)
                    {
                        expanded.push((known.clone(), 0.6));
                    }
                }
            }
            if index == last && word.chars().count() >= 3 {
                // The word being typed: everything it starts.
                for known in self.with_prefix(word).take(50) {
                    if known.len() > word.len() && *known != term {
                        expanded.push((known.clone(), 0.5));
                    }
                }
            }
        }
        expanded.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.total_cmp(&a.1)));
        expanded.dedup_by(|a, b| a.0 == b.0);
        expanded
    }

    pub fn search(&self, query: &str, limit: usize, boosts: &dyn Fn(&str) -> f32) -> Vec<Hit> {
        let (tag_filter, rest): (Vec<String>, Vec<&str>) = {
            let mut tags = Vec::new();
            let mut rest = Vec::new();
            for word in query.split_whitespace() {
                match word.strip_prefix('#') {
                    Some(tag) if !tag.is_empty() => tags.push(tag.to_lowercase()),
                    _ => rest.push(word),
                }
            }
            (tags, rest)
        };
        let query = rest.join(" ");
        let terms = self.expand(&query);
        let phrase = query.trim().to_lowercase();
        let n = self.docs.len() as f32;
        let now = Local::now();
        let mut scored: Vec<(f32, &Doc)> = Vec::new();
        for doc in &self.docs {
            if !tag_filter
                .iter()
                .all(|tag| doc.tags.iter().any(|have| have.to_lowercase() == *tag))
            {
                continue;
            }
            let mut score = 0.0;
            for (term, weight) in &terms {
                let Some(tf) = doc.terms.get(term) else {
                    continue;
                };
                let df = *self.df.get(term).unwrap_or(&1) as f32;
                let idf = ((n - df + 0.5) / (df + 0.5) + 1.0).ln();
                let k1 = 1.2;
                let b = 0.75;
                score += weight * idf * (tf * (k1 + 1.0))
                    / (tf + k1 * (1.0 - b + b * doc.len / self.avg_len));
            }
            if terms.is_empty() && !tag_filter.is_empty() {
                score = 1.0;
            }
            if score <= 0.0 {
                continue;
            }
            if phrase.len() > 3 && doc.title.to_lowercase().contains(&phrase) {
                score += 3.0;
            }
            let age_days = (now - doc.touched).num_days().max(0) as f32;
            score *= 1.0 + 0.15 * (-age_days / 120.0).exp();
            if doc.archived.is_some() {
                score *= 0.5;
            }
            score *= boosts(&doc.path);
            scored.push((score, doc));
        }
        scored.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.path.cmp(&b.1.path)));
        scored.truncate(limit);
        // Snippets only for the hits shown: finding the best line costs more than scoring.
        scored
            .into_iter()
            .map(|(score, doc)| {
                let (snippet, line) = snippet(doc, &terms);
                Hit {
                    path: doc.path.clone(),
                    title: doc.title.clone(),
                    score,
                    snippet,
                    line,
                    archived: doc.archived.clone(),
                    tags: doc.tags.clone(),
                    kind: doc.kind.clone(),
                }
            })
            .collect()
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Similar {
    pub path: String,
    pub title: String,
    pub kind: String,
    pub score: f32,
}

impl Index {
    /// Notes most like `path`: scored on its own strongest words. Archived notes are left out.
    pub fn similar(&self, path: &str, limit: usize) -> Vec<Similar> {
        let Some(source) = self.doc(path) else {
            return Vec::new();
        };
        let n = self.docs.len() as f32;
        let idf = |term: &str| {
            let df = *self.df.get(term).unwrap_or(&1) as f32;
            ((n - df + 0.5) / (df + 0.5) + 1.0).ln()
        };
        let mut strongest: Vec<(&String, f32)> = source
            .terms
            .iter()
            .map(|(t, tf)| (t, tf * idf(t)))
            .collect();
        strongest.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(b.0)));
        strongest.truncate(14);
        let mut out: Vec<Similar> = self
            .docs
            .iter()
            .filter(|doc| doc.archived.is_none() && doc.path != path)
            .filter_map(|doc| {
                let mut score = 0.0;
                for (term, weight) in &strongest {
                    let Some(tf) = doc.terms.get(*term) else {
                        continue;
                    };
                    let (k1, b) = (1.2, 0.75);
                    score += weight.sqrt() * idf(term) * (tf * (k1 + 1.0))
                        / (tf + k1 * (1.0 - b + b * doc.len / self.avg_len));
                }
                (score > 0.0).then(|| Similar {
                    path: doc.path.clone(),
                    title: doc.title.clone(),
                    kind: doc.kind.clone(),
                    score,
                })
            })
            .collect();
        out.sort_by(|a, b| b.score.total_cmp(&a.score).then(a.path.cmp(&b.path)));
        out.truncate(limit);
        out
    }
}

/// A note as search sees it.
pub fn doc(note: &Note, archived: Option<String>) -> Doc {
    let mut terms: HashMap<String, f32> = HashMap::new();
    let mut add = |source: &str, weight: f32| {
        for term in text::terms(source) {
            *terms.entry(term).or_default() += weight;
        }
    };
    add(&note.title, TITLE);
    add(&note.stem().replace(['-', '_'], " "), 1.0);
    for alias in note.front.aliases() {
        add(&alias, ALIAS);
    }
    let tags = note.front.tags();
    for tag in &tags {
        add(&tag.replace(['-', '_', '/'], " "), TAG);
    }
    let body = note.body();
    add(body, 1.0);
    let len = terms.values().sum::<f32>();
    let body_line = note.text[..note.front.body_start].lines().count();
    Doc {
        path: note.path.clone(),
        title: note.title.clone(),
        archived,
        touched: note.touched,
        body: body.to_owned(),
        body_line,
        terms,
        len,
        tags,
        kind: note.note_type(),
    }
}

fn snippet(doc: &Doc, terms: &[(String, f32)]) -> (String, usize) {
    let mut best: Option<(f32, usize, &str)> = None;
    for (index, line) in doc.body.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("# ") || trimmed == "---" {
            continue;
        }
        let line_terms: Vec<String> = text::terms(trimmed);
        let score: f32 = terms
            .iter()
            .filter(|(term, _)| line_terms.contains(term))
            .map(|(_, weight)| weight)
            .sum();
        let score = score + 0.01;
        if best.is_none_or(|(have, _, _)| score > have) {
            best = Some((score, index, trimmed));
        }
    }
    match best {
        Some((_, index, line)) => (
            text::excerpt(line.trim_start_matches(['#', '>', '-', '*', ' ']), 180),
            doc.body_line + index + 1,
        ),
        None => (String::new(), doc.body_line + 1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notebook::build_note;

    fn note(path: &str, text: &str) -> Note {
        build_note(path.into(), text.into(), text.len() as u64, Local::now())
    }

    fn index() -> Index {
        Index::build(
            &[
                note(
                    "Postgres vacuum.md",
                    "---\ntags: [db]\n---\n# Postgres vacuum\n\nAutovacuum stalls when tables are huge.\n",
                ),
                note(
                    "Kubernetes.md",
                    "# Kubernetes deploys\n\nRolling deploys with readiness probes.\n",
                ),
                note("Shopping.md", "# Shopping\n\nMilk, eggs.\n"),
            ],
            Vec::new(),
        )
    }

    #[test]
    fn finds_by_title_and_body() {
        let hits = index().search("autovacuum", 5, &|_| 1.0);
        assert_eq!(hits[0].path, "Postgres vacuum.md");
        assert_eq!(hits[0].snippet, "Autovacuum stalls when tables are huge.");
        assert_eq!(hits[0].line, 6);
    }

    #[test]
    fn tolerates_typos_and_prefixes() {
        assert_eq!(
            index().search("kubernetse", 5, &|_| 1.0)[0].path,
            "Kubernetes.md"
        );
        assert_eq!(
            index().search("readin", 5, &|_| 1.0)[0].path,
            "Kubernetes.md"
        );
    }

    #[test]
    fn filters_by_tag() {
        let hits = index().search("#db", 5, &|_| 1.0);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].path, "Postgres vacuum.md");
    }

    #[test]
    fn nothing_for_unknown_words() {
        assert!(index().search("zebra", 5, &|_| 1.0).is_empty());
    }
}
