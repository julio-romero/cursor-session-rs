use std::io;
use std::path::PathBuf;

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// How many candidates an ambiguous ID hint lists before summarising the rest.
const MAX_ID_CANDIDATES: usize = 10;

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

    #[error("session id prefix \"{query}\" is ambiguous ({} matches)", matches.len())]
    AmbiguousId {
        query: String,
        /// One `id  source  title` line per matching session, most recent first.
        matches: Vec<String>,
    },

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

    #[error("no Cursor session storage found")]
    NoStorage,

    #[error("session id is empty")]
    EmptyId,
}

impl Error {
    /// Extra guidance lines to show under the error message.
    pub fn hints(&self) -> Vec<String> {
        match self {
            Error::NoHome => vec!["Set HOME or pass --storage <path>.".to_string()],
            Error::UnsupportedStorage { .. } => vec![
                "Pass ~/.cursor/chats, a session directory, store.db, or state.vscdb.".to_string(),
            ],
            Error::SessionNotFound { .. } | Error::EmptyId => {
                vec!["run `cursor-session list` to see session IDs".to_string()]
            }
            Error::AmbiguousId { matches, .. } => {
                let mut hints: Vec<String> =
                    matches.iter().take(MAX_ID_CANDIDATES).cloned().collect();
                if matches.len() > MAX_ID_CANDIDATES {
                    hints.push(format!("and {} more", matches.len() - MAX_ID_CANDIDATES));
                }
                hints.push("use more characters of the ID".to_string());
                hints
            }
            Error::NoStorage => {
                vec!["pass --storage <path> if your Cursor data lives elsewhere".to_string()]
            }
            _ => Vec::new(),
        }
    }
}
