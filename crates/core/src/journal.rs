//! The undo journal. Every change FastMind makes to a notebook runs in a transaction that keeps
//! a copy of each file as it was. Undo puts those copies back, but only while the files are
//! still exactly as FastMind left them: an edit made since is never overwritten.

use crate::notebook::{Notebook, STATE_DIR};
use crate::{Error, Result, fsx, text};
use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Change {
    pub path: String,
    /// The saved copy of the file before the change, or `None` when it didn't exist.
    pub before: Option<String>,
    /// The file's hash after the change, or `None` when the change removed it.
    pub after: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    pub at: DateTime<Local>,
    pub label: String,
    /// What made the change: "dream", "inbox", "archive", "capture".
    pub source: String,
    pub changes: Vec<Change>,
    #[serde(default)]
    pub undone: bool,
}

impl Entry {
    pub fn paths(&self) -> Vec<&str> {
        self.changes
            .iter()
            .map(|change| change.path.as_str())
            .collect()
    }
}

pub fn journal_dir(nb: &Notebook) -> PathBuf {
    nb.root.join(STATE_DIR).join("journal")
}

pub struct Transaction<'a> {
    nb: &'a Notebook,
    dir: PathBuf,
    entry: Entry,
    seen: HashSet<String>,
    done: bool,
}

static COUNTER: AtomicU32 = AtomicU32::new(0);

pub fn new_id() -> String {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed) % 1000;
    format!("{}-{n:03}", Local::now().format("%Y%m%d-%H%M%S-%3f"))
}

impl<'a> Transaction<'a> {
    pub fn begin(nb: &'a Notebook, label: impl Into<String>, source: &str) -> Result<Self> {
        let id = new_id();
        let dir = journal_dir(nb).join(&id);
        fs::create_dir_all(dir.join("files"))?;
        Ok(Self {
            nb,
            dir,
            entry: Entry {
                id,
                at: Local::now(),
                label: label.into(),
                source: source.to_owned(),
                changes: Vec::new(),
                undone: false,
            },
            seen: HashSet::new(),
            done: false,
        })
    }

    pub fn notebook(&self) -> &Notebook {
        self.nb
    }

    fn snapshot(&mut self, rel: &str) -> Result<usize> {
        if let Some(index) = self
            .entry
            .changes
            .iter()
            .position(|change| change.path == rel)
        {
            return Ok(index);
        }
        let path = self.nb.path(rel)?;
        let before = if path.is_file() {
            let name = format!("{}", self.entry.changes.len());
            fs::copy(&path, self.dir.join("files").join(&name))?;
            Some(name)
        } else {
            None
        };
        let after = before.as_ref().map(|_| hash_file(&path)).transpose()?;
        self.seen.insert(rel.to_owned());
        self.entry.changes.push(Change {
            path: rel.to_owned(),
            before,
            after,
        });
        Ok(self.entry.changes.len() - 1)
    }

    pub fn write(&mut self, rel: &str, bytes: &[u8]) -> Result<()> {
        let index = self.snapshot(rel)?;
        fsx::write_atomic(&self.nb.path(rel)?, bytes)?;
        self.entry.changes[index].after = Some(text::fnv(bytes));
        Ok(())
    }

    pub fn remove(&mut self, rel: &str) -> Result<()> {
        let index = self.snapshot(rel)?;
        let path = self.nb.path(rel)?;
        if path.exists() {
            fs::remove_file(&path)?;
            remove_empty_parents(&self.nb.root, &path);
        }
        self.entry.changes[index].after = None;
        Ok(())
    }

    pub fn rename(&mut self, from: &str, to: &str) -> Result<()> {
        let bytes = fs::read(self.nb.path(from)?)?;
        self.write(to, &bytes)?;
        self.remove(from)
    }

    pub fn is_empty(&self) -> bool {
        self.entry.changes.is_empty()
    }

    pub fn commit(mut self) -> Result<Entry> {
        self.done = true;
        let manifest = serde_json::to_vec_pretty(&self.entry)?;
        fsx::write_atomic(&self.dir.join("manifest.json"), &manifest)?;
        Ok(self.entry.clone())
    }

    /// Puts back everything changed so far and forgets the transaction.
    pub fn abort(mut self) {
        self.done = true;
        let _ = restore(self.nb, &self.dir, &self.entry);
        let _ = fs::remove_dir_all(&self.dir);
    }
}

impl Drop for Transaction<'_> {
    fn drop(&mut self) {
        if !self.done {
            let _ = restore(self.nb, &self.dir, &self.entry);
            let _ = fs::remove_dir_all(&self.dir);
        }
    }
}

fn restore(nb: &Notebook, dir: &std::path::Path, entry: &Entry) -> Result<()> {
    for change in entry.changes.iter().rev() {
        let path = nb.path(&change.path)?;
        match &change.before {
            Some(copy) => {
                let bytes = fs::read(dir.join("files").join(copy))?;
                fsx::write_atomic(&path, &bytes)?;
            }
            None => {
                if path.exists() {
                    fs::remove_file(&path)?;
                    remove_empty_parents(&nb.root, &path);
                }
            }
        }
    }
    Ok(())
}

fn remove_empty_parents(root: &std::path::Path, path: &std::path::Path) {
    let mut dir = path.parent();
    while let Some(current) = dir {
        if current == root || !current.starts_with(root) {
            break;
        }
        if fs::remove_dir(current).is_err() {
            break;
        }
        dir = current.parent();
    }
}

fn hash_file(path: &std::path::Path) -> Result<u64> {
    Ok(text::fnv(&fs::read(path)?))
}

/// Every journal entry, newest first.
pub fn history(nb: &Notebook) -> Vec<Entry> {
    let Ok(dirs) = fs::read_dir(journal_dir(nb)) else {
        return Vec::new();
    };
    let mut entries: Vec<Entry> = dirs
        .flatten()
        .filter_map(|dir| fs::read(dir.path().join("manifest.json")).ok())
        .filter_map(|bytes| serde_json::from_slice(&bytes).ok())
        .collect();
    entries.sort_by(|a, b| b.id.cmp(&a.id));
    entries
}

/// Which files of `entry` changed since FastMind wrote them.
pub fn conflicts(nb: &Notebook, entry: &Entry) -> Vec<String> {
    entry
        .changes
        .iter()
        .filter(|change| {
            let Ok(path) = nb.path(&change.path) else {
                return true;
            };
            match change.after {
                Some(hash) => hash_file(&path).map_or(true, |now| now != hash),
                None => path.exists(),
            }
        })
        .map(|change| change.path.clone())
        .collect()
}

/// Undoes entry `id`. Refuses, changing nothing, when any of its files was edited since.
pub fn undo(nb: &Notebook, id: &str) -> Result<Entry> {
    let dir = journal_dir(nb).join(id);
    let bytes = fs::read(dir.join("manifest.json"))
        .map_err(|_| Error::msg(format!("no change {id} to undo")))?;
    let mut entry: Entry = serde_json::from_slice(&bytes)?;
    if entry.undone {
        return Err(Error::msg("that change is already undone"));
    }
    let changed = conflicts(nb, &entry);
    if !changed.is_empty() {
        return Err(Error::msg(format!(
            "not undone: changed since FastMind wrote it: {}",
            changed.join(", ")
        )));
    }
    restore(nb, &dir, &entry)?;
    entry.undone = true;
    fsx::write_atomic(
        &dir.join("manifest.json"),
        &serde_json::to_vec_pretty(&entry)?,
    )?;
    Ok(entry)
}

/// Bytes the journal's saved copies take.
pub fn size(nb: &Notebook) -> u64 {
    let mut files = Vec::new();
    fsx::walk(&journal_dir(nb), &|_| false, &mut files);
    files
        .iter()
        .filter_map(|path| fs::metadata(path).ok())
        .map(|meta| meta.len())
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undo_restores_and_refuses_after_edits() {
        let dir = tempfile::tempdir().unwrap();
        let nb = Notebook::open(dir.path()).unwrap();
        fs::write(nb.root.join("a.md"), "one").unwrap();

        let mut tx = Transaction::begin(&nb, "test", "test").unwrap();
        tx.write("a.md", b"two").unwrap();
        tx.rename("a.md", "moved/a.md").unwrap();
        tx.write("b.md", b"new").unwrap();
        let entry = tx.commit().unwrap();
        assert!(!nb.root.join("a.md").exists());

        undo(&nb, &entry.id).unwrap();
        assert_eq!(fs::read_to_string(nb.root.join("a.md")).unwrap(), "one");
        assert!(!nb.root.join("b.md").exists());
        assert!(!nb.root.join("moved").exists());
        assert!(undo(&nb, &entry.id).is_err());

        let mut tx = Transaction::begin(&nb, "again", "test").unwrap();
        tx.write("a.md", b"three").unwrap();
        let entry = tx.commit().unwrap();
        fs::write(nb.root.join("a.md"), "edited by hand").unwrap();
        assert!(undo(&nb, &entry.id).is_err());
        assert_eq!(
            fs::read_to_string(nb.root.join("a.md")).unwrap(),
            "edited by hand"
        );
    }

    #[test]
    fn dropped_transaction_rolls_back() {
        let dir = tempfile::tempdir().unwrap();
        let nb = Notebook::open(dir.path()).unwrap();
        fs::write(nb.root.join("a.md"), "one").unwrap();
        {
            let mut tx = Transaction::begin(&nb, "test", "test").unwrap();
            tx.write("a.md", b"two").unwrap();
        }
        assert_eq!(fs::read_to_string(nb.root.join("a.md")).unwrap(), "one");
        assert!(history(&nb).is_empty());
    }
}
