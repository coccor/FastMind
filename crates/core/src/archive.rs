//! The archive: where notes go when Dream or the user retires them. Archived notes leave the
//! notebook's folders but stay in `.fastmind/archive/` and can be restored. After a while
//! they're packed into zip files, which is where the disk space actually comes back.

use crate::journal::Transaction;
use crate::notebook::{Notebook, STATE_DIR};
use crate::{Error, Result, fsx};
use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::io::{Read, Write};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Item {
    pub id: String,
    /// Where the file was in the notebook.
    pub original: String,
    /// Where it is now: a loose file under `.fastmind/archive/files/`, or its name inside `pack`.
    pub stored: String,
    pub title: String,
    pub reason: String,
    pub archived_at: DateTime<Local>,
    pub size: u64,
    #[serde(default)]
    pub pack: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Index {
    pub items: Vec<Item>,
}

/// Each archived item is described by its own small file, so undoing one archive change never
/// collides with another.
pub fn items_dir() -> String {
    format!("{STATE_DIR}/archive/items")
}

fn item_path(id: &str) -> String {
    format!("{}/{id}.json", items_dir())
}

impl Index {
    pub fn load(nb: &Notebook) -> Self {
        let Ok(dir) = nb.path(&items_dir()) else {
            return Self::default();
        };
        let Ok(entries) = fs::read_dir(dir) else {
            return Self::default();
        };
        let mut items: Vec<Item> = entries
            .flatten()
            .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
            .filter_map(|entry| fs::read(entry.path()).ok())
            .filter_map(|bytes| serde_json::from_slice(&bytes).ok())
            .collect();
        items.sort_by(|a, b| a.archived_at.cmp(&b.archived_at).then(a.id.cmp(&b.id)));
        Self { items }
    }

    /// Writes the items that changed and removes those that left, inside `tx`.
    pub fn save(&self, tx: &mut Transaction) -> Result<()> {
        let nb = tx.notebook().clone();
        let before = Index::load(&nb);
        for item in &self.items {
            let bytes = serde_json::to_vec_pretty(item)?;
            let path = item_path(&item.id);
            let current = nb.path(&path).ok().and_then(|p| fs::read(p).ok());
            if current.as_deref() != Some(bytes.as_slice()) {
                tx.write(&path, &bytes)?;
            }
        }
        for old in before.items {
            if !self.items.iter().any(|item| item.id == old.id) {
                tx.remove(&item_path(&old.id))?;
            }
        }
        Ok(())
    }

    pub fn loose(&self) -> impl Iterator<Item = &Item> {
        self.items.iter().filter(|item| item.pack.is_none())
    }
}

/// Moves `rel` into the archive inside `tx`.
pub fn archive(
    tx: &mut Transaction,
    index: &mut Index,
    rel: &str,
    title: &str,
    reason: &str,
) -> Result<()> {
    let nb = tx.notebook().clone();
    let size = fs::metadata(nb.path(rel)?)?.len();
    let month = Local::now().format("%Y-%m").to_string();
    let (folder, name) = match rel.rfind('/') {
        Some(at) => (format!("/{}", &rel[..at]), &rel[at + 1..]),
        None => (String::new(), rel),
    };
    let base = format!("{STATE_DIR}/archive/files/{month}{folder}");
    let (stem, ext) = match name.rfind('.') {
        Some(at) => (&name[..at], &name[at + 1..]),
        None => (name, ""),
    };
    let taken: HashSet<String> = index.items.iter().map(|item| item.stored.clone()).collect();
    let file = fsx::free_name(stem, ext, |candidate| {
        let stored = format!("{base}/{candidate}");
        taken.contains(&stored) || nb.exists(&stored)
    });
    let stored = format!("{base}/{file}");
    tx.rename(rel, &stored)?;
    index.items.push(Item {
        id: crate::journal::new_id(),
        original: rel.to_owned(),
        stored,
        title: title.to_owned(),
        reason: reason.to_owned(),
        archived_at: Local::now(),
        size,
        pack: None,
    });
    Ok(())
}

/// Puts archived item `id` back where it was (or beside it, when that name is taken now).
pub fn restore(nb: &Notebook, id: &str) -> Result<String> {
    let mut index = Index::load(nb);
    let position = index
        .items
        .iter()
        .position(|item| item.id == id)
        .ok_or_else(|| Error::msg("no such archived note"))?;
    let item = index.items.remove(position);
    let bytes = match &item.pack {
        None => fs::read(nb.path(&item.stored)?)?,
        Some(pack) => read_from_pack(nb, pack, &item.stored)?,
    };
    let (folder, name) = match item.original.rfind('/') {
        Some(at) => (&item.original[..at + 1], &item.original[at + 1..]),
        None => ("", item.original.as_str()),
    };
    let (stem, ext) = match name.rfind('.') {
        Some(at) => (&name[..at], &name[at + 1..]),
        None => (name, ""),
    };
    let free = fsx::free_name(stem, ext, |candidate| {
        nb.exists(&format!("{folder}{candidate}"))
    });
    let target = format!("{folder}{free}");

    let mut tx = Transaction::begin(
        nb,
        format!("Restore \"{}\" from the archive", item.title),
        "archive",
    )?;
    tx.write(&target, &bytes)?;
    if item.pack.is_none() {
        tx.remove(&item.stored)?;
    }
    index.save(&mut tx)?;
    tx.commit()?;
    Ok(target)
}

/// Reads one archived note's text, loose or packed.
pub fn read(nb: &Notebook, item: &Item) -> Result<Vec<u8>> {
    match &item.pack {
        None => Ok(fs::read(nb.path(&item.stored)?)?),
        Some(pack) => read_from_pack(nb, pack, &item.stored),
    }
}

fn read_from_pack(nb: &Notebook, pack: &str, name: &str) -> Result<Vec<u8>> {
    let file = fs::File::open(nb.path(pack)?)?;
    let mut zip = zip::ZipArchive::new(file).map_err(|error| Error::msg(error.to_string()))?;
    let mut entry = zip
        .by_name(name)
        .map_err(|error| Error::msg(error.to_string()))?;
    let mut bytes = Vec::new();
    entry.read_to_end(&mut bytes)?;
    Ok(bytes)
}

/// Builds a zip holding `items`' loose files, keyed by their stored path. Returns the zip's bytes.
pub fn build_pack(nb: &Notebook, items: &[&Item], extra: &[(String, Vec<u8>)]) -> Result<Vec<u8>> {
    let mut buffer = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut buffer);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .compression_level(Some(9));
        for item in items {
            let bytes = fs::read(nb.path(&item.stored)?)?;
            zip.start_file(item.stored.as_str(), options)
                .map_err(|error| Error::msg(error.to_string()))?;
            zip.write_all(&bytes)?;
        }
        for (name, bytes) in extra {
            zip.start_file(name.as_str(), options)
                .map_err(|error| Error::msg(error.to_string()))?;
            zip.write_all(bytes)?;
        }
        zip.finish()
            .map_err(|error| Error::msg(error.to_string()))?;
    }
    Ok(buffer.into_inner())
}

/// Bytes the archive takes on disk: loose files and packs.
pub fn disk_size(nb: &Notebook) -> u64 {
    let mut files = Vec::new();
    fsx::walk(
        &nb.root.join(STATE_DIR).join("archive"),
        &|_| false,
        &mut files,
    );
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
    fn archive_and_restore_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let nb = Notebook::open(dir.path()).unwrap();
        fs::create_dir_all(nb.root.join("ideas")).unwrap();
        fs::write(nb.root.join("ideas/Old.md"), "# Old\nstale").unwrap();

        let mut index = Index::load(&nb);
        let mut tx = Transaction::begin(&nb, "archive", "test").unwrap();
        archive(&mut tx, &mut index, "ideas/Old.md", "Old", "stale").unwrap();
        index.save(&mut tx).unwrap();
        tx.commit().unwrap();
        assert!(!nb.root.join("ideas/Old.md").exists());
        let index = Index::load(&nb);
        assert_eq!(index.items.len(), 1);
        assert!(nb.root.join(&index.items[0].stored).exists());

        let restored = restore(&nb, &index.items[0].id).unwrap();
        assert_eq!(restored, "ideas/Old.md");
        assert_eq!(
            fs::read_to_string(nb.root.join("ideas/Old.md")).unwrap(),
            "# Old\nstale"
        );
        assert!(Index::load(&nb).items.is_empty());
    }
}
