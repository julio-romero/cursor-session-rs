use std::io;
use std::path::{Path, PathBuf};

pub type Result<T, E = Error> = std::result::Result<T, E>;

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

    #[error("session id `{query}` is ambiguous ({} matches)", matches.len())]
    AmbiguousId { query: String, matches: Vec<String> },

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

    #[error("could not copy {} to a temporary directory for reading", path.display())]
    Snapshot { path: PathBuf, source: io::Error },
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
            Error::SessionNotFound { .. } => {
                vec!["Use `cursor-session list` to see IDs.".to_string()]
            }
            Error::AmbiguousId { matches, .. } => {
                let mut hints = matches.clone();
                hints.push("Use a longer prefix or the full ID.".to_string());
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
            Error::Snapshot { .. } => vec![format!(
                "Make sure {} is writable and has free space, or point TMPDIR (TEMP on Windows) \
                 elsewhere.",
                std::env::temp_dir().display()
            )],
            _ => Vec::new(),
        }
    }
}
