use std::io;
use std::path::PathBuf;

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("could not determine home directory")]
    NoHome,

    #[error("storage path does not exist: {}", path.display())]
    StorageNotFound { path: PathBuf, source: io::Error },

    #[error("unsupported storage file (expected state.vscdb or store.db)")]
    UnsupportedStorage { path: PathBuf },

    #[error("session not found: {query}")]
    SessionNotFound { query: String },

    #[error("session id `{query}` is ambiguous ({} matches)", matches.len())]
    AmbiguousId { query: String, matches: Vec<String> },

    #[error("unexpected schema in {}: {detail}", path.display())]
    SchemaMismatch { path: PathBuf, detail: String },

    #[error("failed to read sqlite database: {}", path.display())]
    Database {
        path: PathBuf,
        source: rusqlite::Error,
    },

    #[error("could not access {}", path.display())]
    Io { path: PathBuf, source: io::Error },

    #[error(transparent)]
    Json(#[from] serde_json::Error),

    #[error(transparent)]
    Yaml(#[from] serde_yaml::Error),

    /// Writing exported output failed; the writer has no path of its own.
    #[error(transparent)]
    Write(io::Error),
}

impl Error {
    /// Extra guidance lines to show under the error message.
    pub fn hints(&self) -> Vec<String> {
        match self {
            Error::NoHome => vec!["Set HOME or pass --storage <path>.".to_string()],
            Error::UnsupportedStorage { .. } => vec![
                "Pass ~/.cursor/chats, a session directory, store.db, or state.vscdb.".to_string(),
            ],
            Error::SessionNotFound { .. } => {
                vec!["Use `cursor-session list` to see IDs.".to_string()]
            }
            Error::AmbiguousId { matches, .. } => {
                let mut hints = matches.clone();
                hints.push("Use a longer prefix or the full ID.".to_string());
                hints
            }
            _ => Vec::new(),
        }
    }
}
