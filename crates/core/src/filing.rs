//! Filing: an inbox note goes to the place it belongs. FastMind suggests where, from the notes it
//! is most like (their folders, and the home notes among them), and filing it is one key: the
//! note moves, a home note gets a link to it, and the change can be undone.

use crate::frontmatter::{self, Value};
use crate::journal::Transaction;
use crate::links::{self, Resolver};
use crate::notebook::{HOME_TYPES, Notebook};
use crate::search::Index;
use crate::{Error, Result, fsx};
use serde::Serialize;
use std::collections::HashMap;

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Home {
    /// `folder` or `note`.
    pub kind: String,
    /// The folder, or the home note's path.
    pub target: String,
    pub label: String,
    /// The notes there it's most like, for the "why".
    pub because: Vec<String>,
    pub score: f32,
}

fn folder_of(path: &str) -> &str {
    path.rfind('/').map_or("", |at| &path[..at])
}

/// Where `path` probably belongs, best first: up to three folders and two home notes.
pub fn suggest(index: &Index, path: &str, inbox: &str, ai_folder: &str) -> Vec<Home> {
    let similar = index.similar(path, 20);
    let mut folders: HashMap<String, (f32, Vec<String>)> = HashMap::new();
    let mut homes: Vec<Home> = Vec::new();
    let skip = |folder: &str| {
        folder == inbox || folder.starts_with(&format!("{inbox}/")) || folder.starts_with(ai_folder)
    };
    for hit in &similar {
        if HOME_TYPES.contains(&hit.kind.as_str()) {
            homes.push(Home {
                kind: "note".into(),
                target: hit.path.clone(),
                label: hit.title.clone(),
                because: Vec::new(),
                score: hit.score * 1.5,
            });
        }
        let folder = folder_of(&hit.path);
        if skip(folder) {
            continue;
        }
        let entry = folders.entry(folder.to_owned()).or_default();
        entry.0 += hit.score;
        if entry.1.len() < 3 {
            entry.1.push(hit.title.clone());
        }
    }
    let mut out: Vec<Home> = folders
        .into_iter()
        .map(|(folder, (score, because))| Home {
            kind: "folder".into(),
            label: if folder.is_empty() {
                "Notebook root".into()
            } else {
                format!("{folder}/")
            },
            target: folder,
            because,
            score,
        })
        .collect();
    out.sort_by(|a, b| b.score.total_cmp(&a.score).then(a.target.cmp(&b.target)));
    out.truncate(3);
    homes.sort_by(|a, b| b.score.total_cmp(&a.score));
    homes.truncate(2);
    out.extend(homes);
    out.sort_by(|a, b| b.score.total_cmp(&a.score).then(a.target.cmp(&b.target)));
    out
}

/// Every folder in the notebook, for filing somewhere else.
pub fn folders(notes: &[crate::Note]) -> Vec<String> {
    let mut out: Vec<String> = notes
        .iter()
        .map(|note| note.folder().to_owned())
        .flat_map(|folder| {
            let mut all = Vec::new();
            let mut current = folder.as_str();
            loop {
                all.push(current.to_owned());
                match current.rfind('/') {
                    Some(at) => current = &current[..at],
                    None => break,
                }
            }
            all
        })
        .filter(|folder| !folder.is_empty())
        .collect();
    out.sort();
    out.dedup();
    out
}

/// Files `path` into `folder`, or beside the home note `home` with a link added to it.
/// Returns where the note is now.
pub fn file(nb: &Notebook, path: &str, folder: Option<&str>, home: Option<&str>) -> Result<String> {
    let note = nb.read(path)?;
    let notes = nb.notes();
    let (target_folder, home_note) = match (folder, home) {
        (_, Some(home)) => {
            let home_note = notes
                .iter()
                .find(|n| n.path == home)
                .ok_or_else(|| Error::msg("that home note is gone"))?;
            (home_note.folder().to_owned(), Some(home_note.clone()))
        }
        (Some(folder), None) => (folder.trim_matches(['/', '\\']).to_owned(), None),
        (None, None) => return Err(Error::msg("file it where? give a folder or a home note")),
    };
    if target_folder
        .split('/')
        .any(|part| part == ".." || part.starts_with('.'))
    {
        return Err(Error::msg("not a folder FastMind can file into"));
    }
    let name = path.rsplit('/').next().unwrap_or(path);
    let (stem, _) = name.rsplit_once('.').unwrap_or((name, "md"));
    let join = |file: &str| {
        if target_folder.is_empty() {
            file.to_owned()
        } else {
            format!("{target_folder}/{file}")
        }
    };
    let free = fsx::free_name(stem, "md", |candidate| {
        let rel = join(candidate);
        rel != path && nb.exists(&rel)
    });
    let target = join(&free);
    let new_stem = free.trim_end_matches(".md").to_owned();

    let mut text = frontmatter::remove(&note.text, "status");
    if frontmatter::parse(&text).entries.is_empty() && frontmatter::parse(&text).present {
        // Nothing left in the block: drop it rather than leave an empty one.
        let body = frontmatter::body(&text).to_owned();
        text = body;
    }
    let label = home_note
        .as_ref()
        .map_or(target_folder.clone(), |h| h.title.clone());
    let mut tx = Transaction::begin(
        nb,
        format!(
            "File \"{}\" in {}",
            note.title,
            if label.is_empty() {
                "the notebook root"
            } else {
                &label
            }
        ),
        "inbox",
    )?;
    if target != path {
        tx.write(&target, text.as_bytes())?;
        tx.remove(path)?;
    } else {
        tx.write(path, text.as_bytes())?;
    }
    // Links that named the note by a file name it no longer has follow it.
    if new_stem != note.stem() {
        let resolver = Resolver::new(&notes);
        for other in notes.iter().filter(|n| n.path != path) {
            let body = links::retarget_note(
                other.body(),
                |t| resolver.resolve(t) == Some(path),
                &target,
                &other.path,
            );
            if body != other.body() {
                tx.write(
                    &other.path,
                    format!("{}{}", &other.text[..other.front.body_start], body).as_bytes(),
                )?;
            }
        }
    }
    if let Some(home) = home_note {
        let current = nb.read(&home.path)?;
        let link = format!("- [[{new_stem}]]");
        if !current.text.contains(&format!("[[{new_stem}]]")) {
            let text = add_under_notes(&current.text, &link);
            tx.write(&home.path, text.as_bytes())?;
        }
    }
    tx.commit()?;
    Ok(target)
}

/// `text` with `line` added to its "## Notes" list (made at the end when there's none).
fn add_under_notes(text: &str, line: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    if let Some(start) = lines.iter().position(|l| {
        let l = l.trim().to_lowercase();
        l == "## notes" || l == "## related" || l == "## links"
    }) {
        let mut end = start + 1;
        while end < lines.len() && !lines[end].starts_with("## ") && !lines[end].starts_with("# ") {
            end += 1;
        }
        // After the section's last non-empty line.
        let mut insert = end;
        while insert > start + 1 && lines[insert - 1].trim().is_empty() {
            insert -= 1;
        }
        let mut out: Vec<String> = lines.iter().map(|l| (*l).to_owned()).collect();
        if insert == start + 1 {
            out.insert(insert, String::new());
            insert += 1;
        }
        out.insert(insert, line.to_owned());
        return out.join("\n") + "\n";
    }
    format!("{}\n\n## Notes\n\n{line}\n", text.trim_end())
}

#[allow(dead_code)]
fn set_status(text: &str, status: &str) -> String {
    frontmatter::set(text, "status", &Value::Text(status.into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remember;
    use std::fs;

    #[test]
    fn suggests_where_similar_notes_live_and_files_there() {
        let dir = tempfile::tempdir().unwrap();
        let nb = Notebook::open(dir.path()).unwrap();
        let w = |rel: &str, text: &str| {
            let path = nb.root.join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        };
        w(
            "work/db/Postgres vacuum.md",
            "# Postgres vacuum\n\nAutovacuum and dead tuples on big postgres tables.\n",
        );
        w(
            "work/db/Postgres indexes.md",
            "# Postgres indexes\n\nPartial indexes in postgres speed up queue tables.\n",
        );
        w(
            "work/Databases.md",
            "---\ntype: topic\n---\n# Databases\n\nPostgres, autovacuum and indexes.\n\n## Notes\n\n- [[Postgres vacuum]]\n",
        );
        w("home/Bread.md", "# Bread\n\nKnead and proof.\n");
        w(
            "inbox/Vacuum freeze.md",
            "---\nstatus: inbox\n---\n# Vacuum freeze\n\nPostgres autovacuum freeze runs on old tables; watch dead tuples.\n",
        );
        w("ideas/Later.md", "See [[Vacuum freeze]].\n");

        let index = remember::build_index(&nb);
        let homes = suggest(&index, "inbox/Vacuum freeze.md", "inbox", "AI sessions");
        assert_eq!(homes[0].target, "work/db", "{homes:#?}");
        assert!(
            homes
                .iter()
                .any(|h| h.kind == "note" && h.target == "work/Databases.md")
        );
        assert!(!homes.iter().any(|h| h.target == "home"));

        let filed = file(
            &nb,
            "inbox/Vacuum freeze.md",
            None,
            Some("work/Databases.md"),
        )
        .unwrap();
        assert_eq!(filed, "work/Vacuum freeze.md");
        let moved = fs::read_to_string(nb.root.join(&filed)).unwrap();
        assert!(
            !moved.contains("status: inbox") && moved.starts_with("# Vacuum freeze"),
            "{moved}"
        );
        let home = fs::read_to_string(nb.root.join("work/Databases.md")).unwrap();
        assert!(
            home.contains("- [[Postgres vacuum]]\n- [[Vacuum freeze]]"),
            "{home}"
        );
        assert!(!nb.root.join("inbox/Vacuum freeze.md").exists());
        assert_eq!(
            folders(&nb.notes()),
            vec!["home", "ideas", "work", "work/db"]
        );
    }

    #[test]
    fn a_taken_name_moves_links_with_the_note() {
        let dir = tempfile::tempdir().unwrap();
        let nb = Notebook::open(dir.path()).unwrap();
        fs::create_dir_all(nb.root.join("inbox")).unwrap();
        fs::create_dir_all(nb.root.join("work")).unwrap();
        fs::write(nb.root.join("work/Plan.md"), "# Plan\n\nOld plan.\n").unwrap();
        fs::write(nb.root.join("inbox/Plan.md"), "# Plan\n\nNew plan.\n").unwrap();
        fs::write(
            nb.root.join("Index.md"),
            "See [inbox plan](inbox/Plan.md).\n",
        )
        .unwrap();
        let filed = file(&nb, "inbox/Plan.md", Some("work"), None).unwrap();
        assert_eq!(filed, "work/Plan 2.md");
        assert!(
            fs::read_to_string(nb.root.join("Index.md"))
                .unwrap()
                .contains("(work/Plan%202.md)")
        );
    }
}
