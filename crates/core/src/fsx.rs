//! File helpers that behave the same on every OS: atomic writes, notebook-relative paths with
//! `/` separators, and free file names.

use crate::{Error, Result};
use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};

/// Writes `bytes` to `path` through a sibling temporary file, so a crash leaves the old file or
/// the new one, never half of either.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".into());
    let temp = path.with_file_name(format!(".{name}.{}.tmp", std::process::id()));
    {
        let mut file = fs::File::create(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    let mut last = None;
    for _ in 0..20 {
        match fs::rename(&temp, path) {
            Ok(()) => return Ok(()),
            Err(error) => {
                last = Some(error);
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
        }
    }
    let _ = fs::remove_file(&temp);
    Err(last
        .map(Error::from)
        .unwrap_or_else(|| Error::msg("could not replace file")))
}

/// `path` relative to `root`, with `/` between parts, or `None` when it isn't inside `root`.
pub fn relative(root: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(root).ok()?;
    let parts: Vec<String> = rel
        .components()
        .filter_map(|part| match part {
            Component::Normal(name) => Some(name.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    Some(parts.join("/"))
}

/// The absolute path of a notebook-relative `rel`. Refuses paths that would leave the notebook.
pub fn resolve(root: &Path, rel: &str) -> Result<PathBuf> {
    let mut path = root.to_path_buf();
    for part in rel.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => return Err(Error::msg(format!("path leaves the notebook: {rel}"))),
            part if part.contains(':') => {
                return Err(Error::msg(format!("not a notebook path: {rel}")));
            }
            part => path.push(part),
        }
    }
    Ok(path)
}

/// A file name for `stem` + `extension` that `exists` says is free: "Name.md", "Name 2.md", …
pub fn free_name(stem: &str, extension: &str, exists: impl Fn(&str) -> bool) -> String {
    let with_ext = |stem: &str| {
        if extension.is_empty() {
            stem.to_owned()
        } else {
            format!("{stem}.{extension}")
        }
    };
    let first = with_ext(stem);
    if !exists(&first) {
        return first;
    }
    (2..)
        .map(|n| with_ext(&format!("{stem} {n}")))
        .find(|name| !exists(name))
        .expect("an unused name")
}

/// A note title made safe as a file name on Windows, macOS and Linux.
pub fn safe_stem(title: &str) -> String {
    let cleaned: String = title
        .chars()
        .map(|ch| match ch {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '#' | '^' | '[' | ']' => ' ',
            ch if ch.is_control() => ' ',
            ch => ch,
        })
        .collect();
    let mut stem = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    stem = stem.trim_matches(['.', ' ']).to_owned();
    if stem.chars().count() > 80 {
        stem = stem
            .chars()
            .take(80)
            .collect::<String>()
            .trim_end()
            .to_owned();
    }
    let reserved = [
        "con", "prn", "aux", "nul", "com1", "com2", "com3", "lpt1", "lpt2", "lpt3",
    ];
    if stem.is_empty() || reserved.contains(&stem.to_ascii_lowercase().as_str()) {
        stem = format!("Note {stem}").trim().to_owned();
    }
    stem
}

/// Every file under `dir` (recursively), skipping folders `skip` rejects by name.
pub fn walk(dir: &Path, skip: &dyn Fn(&str) -> bool, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            if !skip(&entry.file_name().to_string_lossy()) {
                walk(&path, skip, out);
            }
        } else if kind.is_file() {
            out.push(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stems_are_portable() {
        assert_eq!(safe_stem("What: is *this*?"), "What is this");
        assert_eq!(safe_stem("  ...  "), "Note");
        assert_eq!(safe_stem("CON"), "Note CON");
    }

    #[test]
    fn resolve_refuses_escape() {
        let root = Path::new("root");
        assert!(resolve(root, "../x").is_err());
        assert_eq!(
            resolve(root, "a/b.md").unwrap(),
            root.join("a").join("b.md")
        );
    }

    #[test]
    fn free_names_count_up() {
        let taken = ["A.md", "A 2.md"];
        assert_eq!(free_name("A", "md", |name| taken.contains(&name)), "A 3.md");
    }
}
