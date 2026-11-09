//! What Dream measures: how alike notes are (shared phrases and shared vocabulary), which short
//! notes circle the same subject, and which notes nothing uses any more.

use crate::notebook::Note;
use crate::text;
use std::collections::{HashMap, HashSet};

pub struct Features {
    /// tf-idf weights, normalised to length 1.
    pub vector: HashMap<String, f64>,
    pub shingles: HashSet<u64>,
    pub words: usize,
    /// For each stem, the word as it was most often written.
    pub surface: HashMap<String, String>,
}

pub fn features(notes: &[Note]) -> Vec<Features> {
    let mut counts: Vec<HashMap<String, f64>> = Vec::with_capacity(notes.len());
    let mut surfaces: Vec<HashMap<String, String>> = Vec::with_capacity(notes.len());
    let mut df: HashMap<String, usize> = HashMap::new();
    for note in notes {
        let mut tf: HashMap<String, f64> = HashMap::new();
        let mut seen_forms: HashMap<String, HashMap<String, usize>> = HashMap::new();
        let mut add = |source: &str, weight: f64| {
            for word in text::words(source) {
                if word.chars().count() < 2 || text::is_stopword(&word) {
                    continue;
                }
                if word.chars().all(|ch| ch.is_ascii_digit()) {
                    continue;
                }
                let stem = text::stem(&word);
                *tf.entry(stem.clone()).or_default() += weight;
                *seen_forms.entry(stem).or_default().entry(word).or_default() += 1;
            }
        };
        add(&note.title, 2.0);
        add(&strip_markup(note.body()), 1.0);
        for term in tf.keys() {
            *df.entry(term.clone()).or_default() += 1;
        }
        let surface = seen_forms
            .into_iter()
            .map(|(stem, forms)| {
                let best = forms
                    .into_iter()
                    .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)))
                    .map(|(form, _)| form)
                    .unwrap_or_default();
                (stem, best)
            })
            .collect();
        counts.push(tf);
        surfaces.push(surface);
    }
    let n = notes.len().max(1) as f64;
    notes
        .iter()
        .zip(counts)
        .zip(surfaces)
        .map(|((note, tf), surface)| {
            let mut vector: HashMap<String, f64> = tf
                .into_iter()
                .map(|(term, count)| {
                    let idf = (n / *df.get(&term).unwrap_or(&1) as f64).ln() + 1.0;
                    let weight = (1.0 + count.ln()) * idf;
                    (term, weight)
                })
                .collect();
            let length = vector.values().map(|w| w * w).sum::<f64>().sqrt();
            if length > 0.0 {
                for weight in vector.values_mut() {
                    *weight /= length;
                }
            }
            let body = strip_markup(note.body());
            Features {
                vector,
                shingles: text::shingles(&body, 3),
                words: text::words(&body).count(),
                surface,
            }
        })
        .collect()
}

/// The body without front-matter-like noise: headings markers, link brackets and URLs.
pub fn strip_markup(body: &str) -> String {
    body.lines()
        .map(|line| {
            line.split_whitespace()
                .filter(|word| {
                    !word.starts_with("http://")
                        && !word.starts_with("https://")
                        && !word.starts_with("<http")
                })
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect::<Vec<_>>()
        .join("\n")
        .replace("[[", "")
        .replace("]]", "")
}

pub fn cosine(a: &HashMap<String, f64>, b: &HashMap<String, f64>) -> f64 {
    let (small, large) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    small
        .iter()
        .filter_map(|(term, w)| large.get(term).map(|v| w * v))
        .sum()
}

/// How much of the smaller note's phrasing appears in the larger one (0–1).
pub fn containment(a: &HashSet<u64>, b: &HashSet<u64>) -> f64 {
    let (small, large) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    if small.is_empty() {
        return 0.0;
    }
    small.intersection(large).count() as f64 / small.len() as f64
}

/// Pairs of notes worth comparing: those sharing one of their strongest words.
pub fn candidate_pairs(features: &[Features], allowed: &[bool]) -> Vec<(usize, usize)> {
    let mut by_term: HashMap<&str, Vec<usize>> = HashMap::new();
    for (index, feature) in features.iter().enumerate() {
        if !allowed[index] {
            continue;
        }
        let mut top: Vec<(&String, &f64)> = feature.vector.iter().collect();
        top.sort_by(|a, b| b.1.total_cmp(a.1).then(a.0.cmp(b.0)));
        for (term, _) in top.into_iter().take(12) {
            by_term.entry(term.as_str()).or_default().push(index);
        }
    }
    let mut pairs = HashSet::new();
    for indices in by_term.values() {
        if indices.len() > 200 {
            continue;
        }
        for (i, a) in indices.iter().enumerate() {
            for b in &indices[i + 1..] {
                pairs.insert(((*a).min(*b), (*a).max(*b)));
            }
        }
    }
    let mut pairs: Vec<_> = pairs.into_iter().collect();
    pairs.sort();
    pairs
}

/// Whether two notes say the same thing: most of the shorter one's phrasing is in the longer,
/// and they share their vocabulary.
pub fn duplicates(a: &Features, b: &Features, threshold: f64) -> Option<f64> {
    let cos = cosine(&a.vector, &b.vector);
    if cos < 0.45 {
        return None;
    }
    let small = a.shingles.len().min(b.shingles.len());
    let contained = containment(&a.shingles, &b.shingles);
    if small >= 6 && contained >= threshold {
        return Some(contained.max(cos));
    }
    if small < 6 && cos >= 0.92 {
        return Some(cos);
    }
    None
}

pub struct UnionFind(Vec<usize>);

impl UnionFind {
    pub fn new(size: usize) -> Self {
        Self((0..size).collect())
    }

    pub fn find(&mut self, x: usize) -> usize {
        let mut root = x;
        while self.0[root] != root {
            root = self.0[root];
        }
        let mut current = x;
        while self.0[current] != root {
            let next = self.0[current];
            self.0[current] = root;
            current = next;
        }
        root
    }

    pub fn union(&mut self, a: usize, b: usize) {
        let (a, b) = (self.find(a), self.find(b));
        if a != b {
            self.0[a.max(b)] = a.min(b);
        }
    }

    pub fn groups(&mut self) -> Vec<Vec<usize>> {
        let mut groups: HashMap<usize, Vec<usize>> = HashMap::new();
        for x in 0..self.0.len() {
            let root = self.find(x);
            groups.entry(root).or_default().push(x);
        }
        let mut groups: Vec<Vec<usize>> = groups
            .into_values()
            .filter(|group| group.len() > 1)
            .collect();
        groups.sort();
        groups
    }
}

/// Clusters of at least `min` fragments around a seed, each member alike the seed and the group.
/// `pairs` are the pairs worth comparing (those sharing a strong word); comparing every pair
/// would grow with the square of the notebook.
pub fn topic_clusters(
    candidates: &[usize],
    pairs: &[(usize, usize)],
    similarity: &dyn Fn(usize, usize) -> f64,
    threshold: f64,
    min: usize,
) -> Vec<Vec<usize>> {
    let mut neighbours: HashMap<usize, Vec<(usize, f64)>> = HashMap::new();
    for (a, b) in pairs {
        let alike = similarity(*a, *b);
        if alike >= threshold {
            neighbours.entry(*a).or_default().push((*b, alike));
            neighbours.entry(*b).or_default().push((*a, alike));
        }
    }
    let mut order: Vec<usize> = candidates.to_vec();
    order.sort_by(|a, b| {
        let na = neighbours.get(a).map_or(0, Vec::len);
        let nb = neighbours.get(b).map_or(0, Vec::len);
        nb.cmp(&na).then(a.cmp(b))
    });
    let mut taken: HashSet<usize> = HashSet::new();
    let mut clusters = Vec::new();
    for seed in order {
        if taken.contains(&seed) {
            continue;
        }
        let mut members = vec![seed];
        let mut near: Vec<(usize, f64)> = neighbours
            .get(&seed)
            .map(|list| {
                list.iter()
                    .copied()
                    .filter(|(other, _)| !taken.contains(other))
                    .collect()
            })
            .unwrap_or_default();
        near.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        for (other, _) in near {
            // Each newcomer must be alike most of the group, not only the seed.
            let alike = members
                .iter()
                .filter(|member| similarity(**member, other) >= threshold * 0.7)
                .count();
            if alike * 2 >= members.len() {
                members.push(other);
            }
        }
        if members.len() >= min {
            members.sort();
            taken.extend(members.iter().copied());
            clusters.push(members);
        }
    }
    clusters
}

/// A two-word title from what the notes share: the word most of their titles use ("probe"),
/// with the word most of them mention ("Kubernetes"), in the order the notes write them.
pub fn naming_title(
    notes: &[&crate::notebook::Note],
    features: &[Features],
    members: &[usize],
) -> Option<String> {
    let mut in_titles: HashMap<String, usize> = HashMap::new();
    for note in notes {
        let terms: HashSet<String> = text::terms(&note.title).into_iter().collect();
        for term in terms {
            *in_titles.entry(term).or_default() += 1;
        }
    }
    let weight = |term: &str| {
        members
            .iter()
            .map(|m| features[*m].vector.get(term).copied().unwrap_or(0.0))
            .sum::<f64>()
    };
    let presence = |term: &str| {
        members
            .iter()
            .filter(|m| features[**m].vector.contains_key(term))
            .count()
    };
    let head = in_titles
        .iter()
        .filter(|(_, count)| **count * 2 >= notes.len())
        .max_by(|a, b| {
            a.1.cmp(b.1)
                .then(weight(a.0).total_cmp(&weight(b.0)))
                .then(b.0.cmp(a.0))
        })
        .map(|(term, _)| term.clone())?;
    let mut candidates: Vec<String> = members
        .iter()
        .flat_map(|m| features[*m].vector.keys().cloned())
        .collect::<HashSet<_>>()
        .into_iter()
        .filter(|term| *term != head && presence(term) * 2 > members.len())
        .collect();
    // A tag most of the notes carry names their subject best.
    let mut tag_count: HashMap<String, usize> = HashMap::new();
    for note in notes {
        for tag in note.front.tags() {
            for term in text::terms(&tag) {
                *tag_count.entry(term).or_default() += 1;
            }
        }
    }
    let tagged = |term: &str| {
        tag_count
            .get(term)
            .is_some_and(|count| count * 2 >= notes.len())
    };
    candidates.sort_by(|a, b| {
        tagged(b)
            .cmp(&tagged(a))
            .then(presence(b).cmp(&presence(a)))
            .then(weight(b).total_cmp(&weight(a)))
            .then(a.cmp(b))
    });
    let surface = |term: &str| {
        members
            .iter()
            .find_map(|m| features[*m].surface.get(term).cloned())
            .unwrap_or_else(|| term.to_owned())
    };
    let Some(modifier) = candidates.into_iter().next() else {
        return Some(surface(&head));
    };
    let mut before = 0;
    let mut after = 0;
    for note in notes {
        let all: Vec<String> = text::words(&format!(
            "{}
{}",
            note.title,
            note.body()
        ))
        .map(|w| text::stem(&w))
        .collect();
        for pair in all.windows(2) {
            if pair[0] == modifier && pair[1] == head {
                before += 1;
            } else if pair[0] == head && pair[1] == modifier {
                after += 1;
            }
        }
    }
    Some(if after > before {
        format!("{} {}", surface(&head), surface(&modifier))
    } else {
        format!("{} {}", surface(&modifier), surface(&head))
    })
}

/// The words that best name a group: those weighing most across its members, as written.
pub fn naming_words(features: &[Features], members: &[usize], count: usize) -> Vec<String> {
    let mut weight: HashMap<&str, f64> = HashMap::new();
    let mut present: HashMap<&str, usize> = HashMap::new();
    for member in members {
        for (term, w) in &features[*member].vector {
            *weight.entry(term.as_str()).or_default() += w;
            *present.entry(term.as_str()).or_default() += 1;
        }
    }
    let mut terms: Vec<(&str, f64)> = weight
        .into_iter()
        .filter(|(term, _)| present.get(term).copied().unwrap_or(0) * 2 >= members.len().max(2))
        .collect();
    terms.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(b.0)));
    terms
        .into_iter()
        .take(count)
        .map(|(term, _)| {
            members
                .iter()
                .find_map(|member| features[*member].surface.get(term).cloned())
                .unwrap_or_else(|| term.to_owned())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notebook::build_note;
    use chrono::Local;

    fn notes(texts: &[&str]) -> Vec<Note> {
        texts
            .iter()
            .enumerate()
            .map(|(i, text)| {
                build_note(
                    format!("n{i}.md"),
                    (*text).into(),
                    text.len() as u64,
                    Local::now(),
                )
            })
            .collect()
    }

    #[test]
    fn near_copies_are_duplicates_and_strangers_are_not() {
        let notes = notes(&[
            "# Redis eviction\nRedis evicts keys with allkeys-lru when maxmemory is reached; set maxmemory-policy explicitly in production.",
            "# Redis eviction policy\nRedis evicts keys with allkeys-lru when maxmemory is reached; set maxmemory-policy explicitly in production. Also watch evicted_keys.",
            "# Sourdough\nFeed the starter twice a day and keep it warm; bake when it doubles in four hours.",
        ]);
        let f = features(&notes);
        assert!(duplicates(&f[0], &f[1], 0.55).is_some());
        assert!(duplicates(&f[0], &f[2], 0.55).is_none());
    }

    #[test]
    fn related_fragments_cluster() {
        let notes = notes(&[
            "# Kubernetes probes\nReadiness probes gate traffic to kubernetes pods.",
            "# Liveness probe\nKubernetes liveness probes restart stuck pods.",
            "# Pod restarts\nPods restart when kubernetes probes fail repeatedly.",
            "# Pasta\nBoil water, salt it, cook pasta eight minutes.",
        ]);
        let f = features(&notes);
        let pairs: Vec<(usize, usize)> = (0..4)
            .flat_map(|a| (a + 1..4).map(move |b| (a, b)))
            .collect();
        let clusters = topic_clusters(
            &[0, 1, 2, 3],
            &pairs,
            &|a, b| cosine(&f[a].vector, &f[b].vector),
            0.2,
            3,
        );
        assert_eq!(clusters, vec![vec![0, 1, 2]]);
        let words = naming_words(&f, &clusters[0], 2);
        assert!(
            words.contains(&"kubernetes".to_owned()) || words.contains(&"probes".to_owned()),
            "{words:?}"
        );
    }
}
