use std::io;
use std::path::{Path, PathBuf};

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// How many candidates an ambiguous ID hint lists before summarising the rest.
const MAX_ID_CANDIDATES: usize = 10;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("could not determine home directory")]
    NoHome,

    #[error("storage path does not exist: {}", path.display())]
    StorageNotFound { path: PathBuf, source: io::Error },

    #[error("unrecognized storage location: {}", path.display())]
    UnsupportedStorage { path: PathBuf },

    #[error("session not found: {query}")]
    SessionNotFound { query: String },

    #[error("session id prefix \"{query}\" is ambiguous ({} matches)", matches.len())]
    AmbiguousId {
        query: String,
        /// One `id  source  title` line per matching session, most recent first.
        matches: Vec<String>,
    },

    #[error(
        "unrecognized Cursor IDE storage format in {}: {detail}. Cursor may have changed its storage format.",
        path.display()
    )]
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

    /// A database whose `-wal` a crash left behind is read from a copy.
    #[error("could not copy {} to a temporary directory for reading", path.display())]
    Snapshot { path: PathBuf, source: io::Error },

    #[error("no Cursor session storage found")]
    NoStorage,

    #[error("session id is empty")]
    EmptyId,
}

impl Error {
    /// `StorageNotFound` when `path` is missing, `Io` for any other failure.
    pub(crate) fn access(path: &Path, source: io::Error) -> Self {
        let path = path.to_path_buf();
        if source.kind() == io::ErrorKind::NotFound {
            Error::StorageNotFound { path, source }
        } else {
            Error::Io { path, source }
        }
    }

    /// Extra guidance lines to show under the error message.
    pub fn hints(&self) -> Vec<String> {
        match self {
            Error::NoHome => vec!["Set HOME or pass --storage <path>.".to_string()],
            Error::UnsupportedStorage { .. } => vec![
                "Pass a home or .cursor directory, ~/.cursor/chats, a session directory, store.db, \
                 state.vscdb, or the directory that contains state.vscdb."
                    .to_string(),
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
            Error::SchemaMismatch { .. } => vec![
                "Rerun with `--source agent` to skip IDE sessions.".to_string(),
                "Please report it at https://github.com/julio-romero/cursor-session-rs/issues \
                 and include your Cursor version."
                    .to_string(),
            ],
            Error::Database { source, .. }
                if matches!(
                    source.sqlite_error_code(),
                    Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked)
                ) =>
            {
                vec!["Cursor may be writing to it right now; try again in a moment.".to_string()]
            }
            Error::Snapshot { .. } => vec![
                format!(
                    "make sure {} is writable and has room for a copy of the database, or point \
                     TMPDIR (TMP on Windows) elsewhere",
                    std::env::temp_dir().display()
                ),
                "if Cursor crashed, start and quit it once; a database it closed cleanly is read \
                 without a copy"
                    .to_string(),
            ],
            Error::NoStorage => {
                vec!["pass --storage <path> if your Cursor data lives elsewhere".to_string()]
            }
            _ => Vec::new(),
        }
    }
}
