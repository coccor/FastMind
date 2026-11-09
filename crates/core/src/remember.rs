//! Remember: find a note, or ask the notebook a question. Answers come from your notes and say
//! which ones: with a model, it writes the answer and cites them; without one, the passages that
//! answer best are quoted, each with its note.

use crate::archive::{self, Index as ArchiveIndex};
use crate::model::Model;
use crate::notebook::{Notebook, build_note};
use crate::search::{Hit, Index};
use crate::text;
use crate::usage::Usage;
use serde::Serialize;
use std::collections::HashSet;

pub fn build_index(nb: &Notebook) -> Index {
    let notes = nb.notes();
    let archive = ArchiveIndex::load(nb);
    let archived = archive
        .loose()
        .filter(|item| item.stored.ends_with(".md"))
        .filter_map(|item| {
            let bytes = archive::read(nb, item).ok()?;
            let text = String::from_utf8_lossy(&bytes).into_owned();
            let note = build_note(item.original.clone(), text, item.size, item.archived_at);
            Some((item.id.clone(), note))
        })
        .collect();
    Index::build(&notes, archived)
}

pub fn search(nb: &Notebook, index: &Index, query: &str, limit: usize) -> Vec<Hit> {
    let usage = Usage::load(nb);
    index.search(query, limit, &|path| usage.boost(path))
}

#[derive(Clone, Debug, Serialize)]
pub struct Source {
    pub n: usize,
    pub path: String,
    pub title: String,
    pub line: usize,
    pub archived: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Answer {
    pub text: String,
    pub sources: Vec<Source>,
    pub by_model: bool,
    /// Why the model wasn't used, when one is set but failed.
    pub note: Option<String>,
}

pub fn ask(nb: &Notebook, index: &Index, question: &str, model: Option<&Model>) -> Answer {
    let hits = search(nb, index, question, 6);
    if hits.is_empty() {
        return Answer {
            text: "Nothing in the notebook mentions that yet.".into(),
            sources: Vec::new(),
            by_model: false,
            note: None,
        };
    }
    let sources: Vec<Source> = hits
        .iter()
        .enumerate()
        .map(|(i, hit)| Source {
            n: i + 1,
            path: hit.path.clone(),
            title: hit.title.clone(),
            line: hit.line,
            archived: hit.archived.clone(),
        })
        .collect();
    let bodies: Vec<String> = hits
        .iter()
        .map(|hit| match index.doc(&hit.path) {
            Some(doc) => doc.body.clone(),
            None => hit.snippet.clone(),
        })
        .collect();

    let mut note = None;
    if let Some(model) = model {
        let mut material = String::new();
        for (source, body) in sources.iter().zip(&bodies) {
            material.push_str(&format!(
                "[{}] {}\n{}\n\n",
                source.n,
                source.title,
                text::excerpt(body, 4000)
            ));
        }
        let system = "You answer questions from the user's own notes. Use only the numbered notes given. \
            Cite each claim with its note number in square brackets, like [2]. If the notes don't answer \
            the question, say so plainly in one sentence. Be brief: a few sentences or a short list.";
        match model.complete(system, &format!("Notes:\n\n{material}Question: {question}")) {
            Ok(reply) => {
                let used: HashSet<usize> = (1..=sources.len())
                    .filter(|n| reply.contains(&format!("[{n}]")))
                    .collect();
                let sources = sources
                    .into_iter()
                    .filter(|s| used.contains(&s.n) || used.is_empty())
                    .collect();
                return Answer {
                    text: reply,
                    sources,
                    by_model: true,
                    note: None,
                };
            }
            Err(error) => {
                note = Some(format!(
                    "The model couldn't answer ({error}); these are the closest passages."
                ))
            }
        }
    }

    // Without a model: the sentences sharing the most of the question's words.
    let wanted: HashSet<String> = text::terms(question).into_iter().collect();
    let mut scored: Vec<(f64, usize, String)> = Vec::new();
    for (source, body) in sources.iter().zip(&bodies) {
        for raw in body.split(['\n']) {
            for sentence in split_sentences(raw) {
                let sentence = sentence
                    .trim()
                    .trim_start_matches(['#', '-', '*', '>', ' '])
                    .trim();
                let count = text::words(sentence).count();
                if count < 3 || count > 60 {
                    continue;
                }
                let terms: HashSet<String> = text::terms(sentence).into_iter().collect();
                let hits = terms.intersection(&wanted).count() as f64;
                if hits == 0.0 {
                    continue;
                }
                let rank_bonus = 1.0 / (source.n as f64 + 1.0);
                scored.push((
                    hits / (count as f64).sqrt() + rank_bonus,
                    source.n,
                    sentence.to_owned(),
                ));
            }
        }
    }
    scored.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
    let mut seen = HashSet::new();
    let picked: Vec<(usize, String)> = scored
        .into_iter()
        .filter(|(_, _, sentence)| seen.insert(text::normalize(sentence)))
        .take(4)
        .map(|(_, n, sentence)| (n, sentence))
        .collect();
    let text = if picked.is_empty() {
        format!(
            "These notes mention it; the closest is \"{}\" [1].",
            sources[0].title
        )
    } else {
        picked
            .iter()
            .map(|(n, sentence)| {
                format!(
                    "- {} [{n}]",
                    sentence.trim_end_matches('.').to_owned() + "."
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let used: HashSet<usize> = picked.iter().map(|(n, _)| *n).chain([1]).collect();
    Answer {
        text,
        sources: sources
            .into_iter()
            .filter(|s| used.contains(&s.n))
            .collect(),
        by_model: false,
        note,
    }
}

fn split_sentences(line: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let bytes = line.as_bytes();
    for (i, byte) in bytes.iter().enumerate() {
        if matches!(byte, b'.' | b'!' | b'?') && bytes.get(i + 1).is_none_or(|next| *next == b' ') {
            out.push(&line[start..=i]);
            start = i + 1;
        }
    }
    if start < line.len() {
        out.push(&line[start..]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn answers_from_notes_with_sources() {
        let dir = tempfile::tempdir().unwrap();
        let nb = Notebook::open(dir.path()).unwrap();
        fs::write(
            nb.root.join("Postgres.md"),
            "# Postgres vacuum\n\nAutovacuum falls behind on huge tables. Lower autovacuum_vacuum_scale_factor to 0.02 for them.\n",
        )
        .unwrap();
        fs::write(nb.root.join("Bread.md"), "# Bread\n\nKnead ten minutes.\n").unwrap();
        let index = build_index(&nb);
        let answer = ask(&nb, &index, "how do I fix autovacuum falling behind?", None);
        assert!(
            answer.text.contains("autovacuum_vacuum_scale_factor"),
            "{}",
            answer.text
        );
        assert!(answer.text.contains("[1]"));
        assert_eq!(answer.sources[0].path, "Postgres.md");
        assert!(!answer.by_model);

        let none = ask(&nb, &index, "quantum chromodynamics", None);
        assert!(none.sources.is_empty());
    }
}
