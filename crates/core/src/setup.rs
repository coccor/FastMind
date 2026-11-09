//! First run: where are your notes? FastMind looks for folders you already keep notes in
//! (Obsidian vaults, FastCapture's notebooks) and offers them beside a new `~/FastMind`.

use crate::config;
use crate::fsx;
use crate::notebook::is_note_file;
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize)]
pub struct Candidate {
    pub path: PathBuf,
    /// `obsidian`, `fastcapture` or `new`.
    pub kind: String,
    pub label: String,
    pub notes: usize,
}

fn count_notes(dir: &Path) -> usize {
    let mut files = Vec::new();
    fsx::walk(dir, &crate::notebook::skip_dir, &mut files);
    files.iter().filter(|p| is_note_file(p)).count()
}

/// Obsidian vaults (folders holding `.obsidian`) within a few levels of the usual places.
fn obsidian_vaults(home: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let roots = [
        home.to_path_buf(),
        home.join("Documents"),
        home.join("OneDrive").join("Documents"),
        home.join("Dropbox"),
        home.join("iCloud Drive"),
    ];
    fn look(dir: &Path, depth: usize, found: &mut Vec<PathBuf>) {
        if found.len() >= 6 {
            return;
        }
        if dir.join(".obsidian").is_dir() {
            found.push(dir.to_path_buf());
            return;
        }
        if depth == 0 {
            return;
        }
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.')
                || matches!(
                    name.as_str(),
                    "AppData" | "Library" | "node_modules" | "target" | "Applications"
                )
            {
                continue;
            }
            if entry.file_type().is_ok_and(|t| t.is_dir()) {
                look(&entry.path(), depth - 1, found);
            }
        }
    }
    for root in roots {
        look(&root, 2, &mut found);
    }
    found.sort();
    found.dedup();
    found
}

/// Notebooks FastCapture knows (Windows): `notebook=` and `recent_notebook=` in its settings.
fn fastcapture_notebooks() -> Vec<PathBuf> {
    let Some(appdata) = std::env::var_os("APPDATA") else {
        return Vec::new();
    };
    let Ok(text) = fs::read_to_string(
        PathBuf::from(appdata)
            .join("FastCapture")
            .join("fastcapture.ini"),
    ) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|line| line.split_once('='))
        .filter(|(key, _)| matches!(key.trim(), "notebook" | "recent_notebook"))
        .map(|(_, value)| PathBuf::from(value.trim()))
        .filter(|path| path.is_dir())
        .collect()
}

pub fn candidates() -> Vec<Candidate> {
    let home = directories::UserDirs::new().map(|d| d.home_dir().to_path_buf());
    let mut out = Vec::new();
    for path in fastcapture_notebooks() {
        if out.iter().any(|c: &Candidate| c.path == path) {
            continue;
        }
        out.push(Candidate {
            notes: count_notes(&path),
            label: format!("FastCapture notebook · {}", path.display()),
            path,
            kind: "fastcapture".into(),
        });
    }
    if let Some(home) = &home {
        for path in obsidian_vaults(home) {
            if out.iter().any(|c| c.path == path) {
                continue;
            }
            out.push(Candidate {
                notes: count_notes(&path),
                label: format!("Obsidian vault · {}", path.display()),
                path,
                kind: "obsidian".into(),
            });
        }
    }
    let fresh = config::default_notebook();
    if !out.iter().any(|c| c.path == fresh) {
        let notes = if fresh.is_dir() {
            count_notes(&fresh)
        } else {
            0
        };
        // A notebook FastMind used before keeps its state in `.fastmind`: offer it first.
        let used = fresh.join(crate::notebook::STATE_DIR).is_dir();
        let candidate = Candidate {
            label: if used || notes > 0 {
                format!("{}", fresh.display())
            } else {
                format!("A new notebook at {}", fresh.display())
            },
            path: fresh,
            kind: if used {
                "fastmind".into()
            } else {
                "new".into()
            },
            notes,
        };
        if used {
            out.insert(0, candidate);
        } else {
            out.push(candidate);
        }
    }
    out
}
