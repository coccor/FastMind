//! FastMind's engine: a plain-Markdown notebook, captured into, searched, and tidied overnight
//! by Dream. No UI and nothing OS-specific, so the desktop app, the CLI and tests share it.

pub mod api;
pub mod archive;
pub mod capture;
pub mod config;
pub mod demo;
pub mod dream;
pub mod filing;
pub mod frontmatter;
pub mod fsx;
pub mod journal;
pub mod links;
pub mod model;
pub mod notebook;
pub mod remember;
pub mod search;
pub mod sessions;
pub mod setup;
pub mod text;
pub mod usage;
pub mod write;

pub use notebook::{Note, Notebook};

#[derive(Debug)]
pub struct Error(pub String);

impl Error {
    pub fn msg(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self(error.to_string())
    }
}

impl From<serde_json::Error> for Error {
    fn from(error: serde_json::Error) -> Self {
        Self(error.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;
