//! Which notes you actually open from FastMind, and when. Remember ranks often-used notes higher
//! and Dream never calls a note you opened recently stale.

use crate::notebook::{Notebook, STATE_DIR};
use crate::{Result, fsx};
use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Use {
    pub opened: u32,
    pub last: Option<DateTime<Local>>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Usage {
    pub notes: HashMap<String, Use>,
}

fn file(nb: &Notebook) -> std::path::PathBuf {
    nb.root.join(STATE_DIR).join("usage.json")
}

impl Usage {
    pub fn load(nb: &Notebook) -> Self {
        fs::read(file(nb))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, nb: &Notebook) -> Result<()> {
        fsx::write_atomic(&file(nb), &serde_json::to_vec(self)?)
    }

    pub fn record_open(nb: &Notebook, path: &str) -> Result<()> {
        let mut usage = Self::load(nb);
        let entry = usage.notes.entry(path.to_owned()).or_default();
        entry.opened += 1;
        entry.last = Some(Local::now());
        usage.save(nb)
    }

    pub fn last_opened(&self, path: &str) -> Option<DateTime<Local>> {
        self.notes.get(path).and_then(|entry| entry.last)
    }

    /// A gentle ranking boost for notes opened often.
    pub fn boost(&self, path: &str) -> f32 {
        self.notes
            .get(path)
            .map_or(1.0, |entry| 1.0 + (entry.opened as f32).ln_1p() * 0.1)
    }

    /// Keeps entries pointing at `to` after a note moves.
    pub fn renamed(&mut self, from: &str, to: &str) {
        if let Some(entry) = self.notes.remove(from) {
            self.notes.insert(to.to_owned(), entry);
        }
    }
}
