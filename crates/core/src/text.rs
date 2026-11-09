//! Words, as search and Dream compare them: lower case, common words left out, simple plurals
//! and verb endings folded so "notes", "noted" and "note" meet.

use std::collections::HashSet;
use std::sync::OnceLock;

const STOPWORDS: &str = "a about above after again against all am an and any are as at be because been before being below between both but by can could did do does doing down during each few for from further had has have having he her here hers herself him himself his how i if in into is it its itself just me more most my myself no nor not now of off on once only or other our ours ourselves out over own same she should so some such than that the their theirs them themselves then there these they this those through to too under until up very was we were what when where which while who whom why will with would you your yours yourself yourselves also get got one two use used using via etc like will can may might must shall http https www com";

fn stopwords() -> &'static HashSet<&'static str> {
    static SET: OnceLock<HashSet<&'static str>> = OnceLock::new();
    SET.get_or_init(|| STOPWORDS.split_whitespace().collect())
}

pub fn is_stopword(word: &str) -> bool {
    stopwords().contains(word)
}

/// The words of `text`, lower-cased and split on anything that isn't a letter or digit.
pub fn words(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|ch: char| !ch.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
}

/// The words worth matching: no stop words, no single letters, folded to a common stem.
pub fn terms(text: &str) -> Vec<String> {
    words(text)
        .filter(|word| word.chars().count() > 1 && !is_stopword(word))
        .filter(|word| !word.chars().all(|ch| ch.is_ascii_digit()) || word.len() >= 4)
        .map(|word| stem(&word))
        .collect()
}

/// A light stem: enough to make plurals and common endings meet without a dictionary.
pub fn stem(word: &str) -> String {
    if !word.is_ascii() || word.len() <= 3 {
        return word.to_owned();
    }
    let rules: [(&str, &str, usize); 9] = [
        ("ies", "y", 5),
        ("sses", "ss", 5),
        ("ing", "", 6),
        ("edly", "", 7),
        ("ed", "", 5),
        ("ers", "er", 5),
        ("es", "e", 5),
        ("ly", "", 6),
        ("s", "", 4),
    ];
    for (suffix, replacement, min_len) in rules {
        if word.len() >= min_len && word.ends_with(suffix) {
            if suffix == "s"
                && (word.ends_with("ss") || word.ends_with("us") || word.ends_with("is"))
            {
                return word.to_owned();
            }
            let mut stem = word[..word.len() - suffix.len()].to_owned();
            stem.push_str(replacement);
            return stem;
        }
    }
    word.to_owned()
}

/// Groups of `size` consecutive words, hashed: near-duplicate text shares most of them.
pub fn shingles(text: &str, size: usize) -> HashSet<u64> {
    let words: Vec<String> = words(text).collect();
    if words.len() < size {
        return words.iter().map(|word| fnv(word.as_bytes())).collect();
    }
    words
        .windows(size)
        .map(|window| fnv(window.join(" ").as_bytes()))
        .collect()
}

pub fn jaccard(a: &HashSet<u64>, b: &HashSet<u64>) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let shared = a.intersection(b).count();
    shared as f64 / (a.len() + b.len() - shared) as f64
}

/// A stable 64-bit hash (FNV-1a), the same on every run and every OS.
pub fn fnv(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

/// Text folded for comparing paragraphs: letters and digits only, lower case, single spaces.
pub fn normalize(text: &str) -> String {
    words(text).collect::<Vec<_>>().join(" ")
}

/// Edit distance, stopping early once it passes `limit`.
pub fn within_edits(a: &str, b: &str, limit: usize) -> bool {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.len().abs_diff(b.len()) > limit {
        return false;
    }
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut current = vec![i + 1; b.len() + 1];
        let mut row_min = current[0];
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            current[j + 1] = (previous[j] + cost)
                .min(previous[j + 1] + 1)
                .min(current[j] + 1);
            row_min = row_min.min(current[j + 1]);
        }
        if row_min > limit {
            return false;
        }
        previous = current;
    }
    previous[b.len()] <= limit
}

/// The first `max` characters of `text` on one line, cut at a word.
pub fn excerpt(text: &str, max: usize) -> String {
    let flat: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max {
        return flat;
    }
    let cut: String = flat.chars().take(max).collect();
    let cut = match cut.rfind(' ') {
        Some(space) if space > max / 2 => cut[..space].to_owned(),
        _ => cut,
    };
    format!("{cut}…")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terms_fold_plurals_and_skip_stopwords() {
        assert_eq!(terms("The Notes are notes"), vec!["note", "note"]);
        assert_eq!(terms("queries and query"), vec!["query", "query"]);
    }

    #[test]
    fn jaccard_of_identical_is_one() {
        let a = shingles("the quick brown fox jumps over", 3);
        assert!((jaccard(&a, &a) - 1.0).abs() < 1e-9);
        let b = shingles("something else entirely different here", 3);
        assert!(jaccard(&a, &b) < 0.01);
    }

    #[test]
    fn edits() {
        assert!(within_edits("postgres", "postgers", 2));
        assert!(!within_edits("postgres", "mysql", 2));
    }
}
