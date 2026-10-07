use std::io;
use std::path::{Path, PathBuf};

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// How many candidates an ambiguous ID hint lists before summarising the rest.
const MAX_ID_CANDIDATES: usize = 10;
const TRY_AGAIN: &str = "try again in a moment; Cursor may be writing to it right now";
const SKIP_IDE: &str = "rerun with `--source agent` to skip IDE sessions";

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

    /// A database read without locks changed during two reads in a row.
    #[error("{} changed while it was being read", path.display())]
    Changed { path: PathBuf },

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

    /// Extra guidance lines to show under the error message: lowercase
    /// imperatives without a closing period, after any indented candidates.
    pub fn hints(&self) -> Vec<String> {
        match self {
            Error::NoHome => vec!["set HOME or pass --storage <path>".to_string()],
            Error::UnsupportedStorage { .. } => vec![
                "pass a home or .cursor directory, ~/.cursor/chats, a session directory, store.db, \
                 state.vscdb, or the directory that contains state.vscdb"
                    .to_string(),
            ],
            Error::SessionNotFound { .. } | Error::EmptyId => {
                vec!["run `cursor-session list` to see session IDs".to_string()]
            }
            Error::AmbiguousId { matches, .. } => {
                let mut hints: Vec<String> = matches
                    .iter()
                    .take(MAX_ID_CANDIDATES)
                    .map(|candidate| format!("  {candidate}"))
                    .collect();
                if matches.len() > MAX_ID_CANDIDATES {
                    hints.push(format!("  and {} more", matches.len() - MAX_ID_CANDIDATES));
                }
                hints.push("use more characters of the ID".to_string());
                hints
            }
            Error::SchemaMismatch { .. } => vec![
                SKIP_IDE.to_string(),
                "report it at https://github.com/julio-romero/cursor-session-rs/issues and \
                 include your Cursor version"
                    .to_string(),
            ],
            Error::Database { source, .. }
                if matches!(
                    source.sqlite_error_code(),
                    Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked)
                ) =>
            {
                vec![TRY_AGAIN.to_string()]
            }
            Error::Changed { .. } => vec![TRY_AGAIN.to_string()],
            Error::Database { path, .. } | Error::Io { path, .. } if is_ide_db(path) => {
                vec![SKIP_IDE.to_string()]
            }
            Error::Snapshot { .. } => vec![
                format!(
                    "make sure {} is writable and has room for a copy of the database, or point \
                     TMPDIR (TMP on Windows) elsewhere",
                    std::env::temp_dir().display()
                ),
                "start and quit Cursor once if it crashed; a database it closed cleanly is read \
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

/// Whether `path` is an IDE database, which `--source agent` leaves unread.
fn is_ide_db(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with(".vscdb") || name.ends_with(".vscdb.backup"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hints_are_lowercase_imperatives_without_a_period() {
        let path = PathBuf::from("state.vscdb");
        let busy = rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_BUSY),
            None,
        );
        let errors = [
            Error::NoHome,
            Error::UnsupportedStorage { path: path.clone() },
            Error::SessionNotFound { query: "x".into() },
            Error::EmptyId,
            Error::AmbiguousId {
                query: "a".into(),
                matches: (0..12).map(|n| format!("a{n}  agent  Title")).collect(),
            },
            Error::SchemaMismatch {
                path: path.clone(),
                detail: "no table".into(),
            },
            Error::Database {
                path: path.clone(),
                source: busy,
            },
            Error::Snapshot {
                path: path.clone(),
                source: io::Error::other("disk full"),
            },
            Error::Changed { path },
            Error::NoStorage,
        ];
        let ide_db = Error::Io {
            path: PathBuf::from("/Cursor/User/globalStorage/state.vscdb"),
            source: io::Error::from(io::ErrorKind::PermissionDenied),
        };
        assert_eq!(ide_db.hints(), [SKIP_IDE]);
        let chats = Error::Io {
            path: PathBuf::from("/home/.cursor/chats"),
            source: io::Error::from(io::ErrorKind::PermissionDenied),
        };
        assert!(chats.hints().is_empty());
        let not_a_database = Error::Database {
            path: PathBuf::from("backup.vscdb.backup"),
            source: rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_NOTADB),
                None,
            ),
        };
        assert_eq!(not_a_database.hints(), [SKIP_IDE]);

        for error in errors.iter().chain([&ide_db]) {
            let hints = error.hints();
            let (candidates, advice): (Vec<_>, Vec<_>) =
                hints.iter().partition(|hint| hint.starts_with("  "));
            assert!(!advice.is_empty(), "{error}");
            for hint in advice {
                assert!(hint.starts_with(|c: char| c.is_ascii_lowercase()), "{hint}");
                assert!(!hint.ends_with('.'), "{hint}");
            }
            if let Error::AmbiguousId { .. } = error {
                assert_eq!(candidates.len(), 11);
                assert!(hints[..11].iter().all(|hint| hint.starts_with("  ")));
            }
        }
    }
}
