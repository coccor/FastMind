//! FastMind's settings, in one JSON file in the user's config folder
//! (`%AppData%\coccor\FastMind\config`, `~/Library/Application Support/dev.coccor.FastMind`,
//! `~/.config/fastmind`).
//! `FASTMIND_CONFIG` points somewhere else, which tests and portable installs use.

use crate::model::ModelSettings;
use crate::{Result, fsx};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Settings {
    /// The notebook folder. Empty until chosen; the app suggests `~/FastMind`.
    pub notebook: Option<PathBuf>,
    /// Where captures land, inside the notebook.
    pub inbox: String,
    pub hotkeys: Hotkeys,
    pub dream: DreamSettings,
    pub model: ModelSettings,
    /// Start with the computer.
    pub autostart: bool,
    /// A captured link gets its page's title and description (one request to that page).
    pub fetch_link_titles: bool,
    /// Learn from AI sessions (off until chosen).
    pub ai: crate::sessions::AiSettings,
    /// The second step of a capture: a model writes the note.
    pub writing: crate::write::WritingSettings,
    /// Notebooks used before, most recent first, for switching between them.
    pub recent_notebooks: Vec<PathBuf>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Hotkeys {
    pub capture: String,
    pub remember: String,
}

impl Hotkeys {
    /// A hotkey is an accelerator like "CommandOrControl+Alt+Space": letters, digits and `+`.
    /// The desktop app writes them into other programs' config (Hyprland's), so nothing else
    /// may get in.
    pub fn check(&self) -> std::result::Result<(), String> {
        for (name, keys) in [("capture", &self.capture), ("remember", &self.remember)] {
            let fine =
                keys.len() <= 64 && keys.chars().all(|c| c.is_ascii_alphanumeric() || c == '+');
            if !fine {
                return Err(format!(
                    "the {name} hotkey {keys:?} isn't a key combination (only letters, digits and +)"
                ));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct DreamSettings {
    /// Dream runs on its own once a night, inside the window.
    pub nightly: bool,
    /// Window start and end, 24-hour `HH:MM` local time.
    pub window_start: String,
    pub window_end: String,
    /// A note untouched, unlinked and unopened this long is proposed for the archive.
    pub stale_days: i64,
    /// A note marked done (status done, finished, obsolete…) this long is proposed for the archive.
    pub done_days: i64,
    /// Archived notes are packed into zip files after this many days.
    pub pack_after_days: i64,
    /// Undo records older than this are packed too, and can no longer be undone.
    pub undo_days: i64,
    /// How alike two notes must be to count as duplicates (0–1).
    pub duplicate_similarity: f64,
    /// How alike short notes must be to be gathered into one topic note (0–1).
    pub topic_similarity: f64,
    /// Smallest group of short notes worth gathering into a topic note.
    pub topic_min_notes: usize,
    /// A note with fewer words than this counts as a short fragment.
    pub fragment_words: usize,
    /// Kinds of proposal applied without asking (`pack`, `fix_link`, `archive`, `merge`, `topic`).
    pub auto_apply: Vec<String>,
    /// New memories (captures still in the inbox) are settled into their home once they're
    /// this many hours old.
    pub settle_after_hours: i64,
    /// Most model requests one night may make.
    pub model_requests: usize,
    /// AI session notes nobody opened in this many days are proposed for the archive.
    pub ai_session_days: i64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            notebook: None,
            inbox: "Fresh memories".into(),
            hotkeys: Hotkeys::default(),
            dream: DreamSettings::default(),
            model: ModelSettings::None,
            autostart: false,
            fetch_link_titles: true,
            ai: crate::sessions::AiSettings::default(),
            writing: crate::write::WritingSettings::default(),
            recent_notebooks: Vec::new(),
        }
    }
}

impl Default for Hotkeys {
    fn default() -> Self {
        Self {
            capture: "CommandOrControl+Alt+Space".into(),
            remember: "CommandOrControl+Alt+R".into(),
        }
    }
}

impl Default for DreamSettings {
    fn default() -> Self {
        Self {
            nightly: true,
            window_start: "02:00".into(),
            window_end: "06:00".into(),
            stale_days: 180,
            done_days: 30,
            pack_after_days: 30,
            undo_days: 30,
            duplicate_similarity: 0.55,
            topic_similarity: 0.22,
            topic_min_notes: 3,
            fragment_words: 120,
            // Dream clears the inbox on its own: settling only moves notes, and History undoes it.
            auto_apply: vec!["pack".into(), "settle".into()],
            settle_after_hours: 6,
            model_requests: 20,
            ai_session_days: 90,
        }
    }
}

pub fn config_file() -> PathBuf {
    if let Some(path) = std::env::var_os("FASTMIND_CONFIG") {
        return PathBuf::from(path);
    }
    directories::ProjectDirs::from("dev", "coccor", "FastMind")
        .map(|dirs| dirs.config_dir().join("settings.json"))
        .unwrap_or_else(|| PathBuf::from("fastmind-settings.json"))
}

/// Where a notebook keeps fresh captures: what FastPad's `capture_folder` says, else a folder
/// it already uses (`Fresh memories`, or FastCapture's `inbox`), else `Fresh memories`.
pub fn capture_folder_for(notebook: &std::path::Path) -> String {
    let ini =
        std::fs::read_to_string(notebook.join(".fastpad").join("notebook.ini")).unwrap_or_default();
    if let Some(folder) = ini.lines().find_map(|line| {
        let (key, value) = line.split_once('=')?;
        (key.trim() == "capture_folder").then(|| value.trim().trim_matches(['/', '\\']).to_owned())
    }) && !folder.is_empty()
        && !folder.contains("..")
    {
        return folder;
    }
    for folder in ["Fresh memories", "inbox"] {
        if notebook.join(folder).is_dir() {
            return folder.to_owned();
        }
    }
    "Fresh memories".to_owned()
}

/// The folder suggested for a first notebook: `~/FastMind`.
pub fn default_notebook() -> PathBuf {
    directories::UserDirs::new()
        .map(|dirs| dirs.home_dir().join("FastMind"))
        .unwrap_or_else(|| PathBuf::from("FastMind"))
}

impl Settings {
    pub fn load() -> Self {
        fs::read(config_file())
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<()> {
        fsx::write_atomic(&config_file(), &serde_json::to_vec_pretty(self)?)
    }

    pub fn notebook_or_default(&self) -> PathBuf {
        self.notebook.clone().unwrap_or_else(default_notebook)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hotkeys_are_only_key_combinations() {
        let ok = Hotkeys::default();
        assert!(ok.check().is_ok());
        let empty = Hotkeys {
            capture: String::new(),
            ..Hotkeys::default()
        };
        assert!(empty.check().is_ok());
        for bad in [
            "X\")) os.execute(\"id\") --",
            "Ctrl+Alt+ Space",
            "Ctrl+Alt+Space\nexec",
            "Ctrl,Alt",
        ] {
            let hotkeys = Hotkeys {
                capture: bad.into(),
                ..Hotkeys::default()
            };
            assert!(hotkeys.check().is_err(), "{bad:?} got through");
        }
    }

    #[test]
    fn missing_fields_take_defaults() {
        let settings: Settings =
            serde_json::from_str(r#"{"inbox":"in","dream":{"stale_days":10}}"#).unwrap();
        assert_eq!(settings.inbox, "in");
        assert_eq!(settings.dream.stale_days, 10);
        assert_eq!(settings.dream.done_days, 30);
        assert_eq!(settings.model, ModelSettings::None);
    }
}
