use std::io;
use std::path::{Path, PathBuf};

use crate::model::Source;

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// How many candidates an ambiguous ID hint lists before summarising the rest.
const MAX_ID_CANDIDATES: usize = 10;
const TRY_AGAIN: &str = "try again in a moment; Cursor may be writing to it right now";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("could not determine home directory")]
    NoHome,

    #[error("storage path does not exist: {}", path.display())]
    StorageNotFound { path: PathBuf },

    #[error("unrecognized storage location: {}", path.display())]
    UnsupportedStorage { path: PathBuf },

    #[error("session not found: {query}")]
    SessionNotFound {
        query: String,
        /// A store that was found but, by `--source`, not searched.
        unsearched: Option<Source>,
    },

    #[error("session ID prefix \"{query}\" is ambiguous ({} matches)", matches.len())]
    AmbiguousId {
        query: String,
        /// One `id  source  title` line per matching session, most recent first.
        matches: Vec<String>,
    },

    #[error(
        "unrecognized {} storage format in {}: {detail}. Cursor may have changed its storage format.",
        store.product(),
        path.display()
    )]
    SchemaMismatch {
        store: Source,
        path: PathBuf,
        detail: String,
    },

    #[error("could not read SQLite database {}", path.display())]
    Database {
        path: PathBuf,
        source: rusqlite::Error,
    },

    #[error("could not access {}", path.display())]
    Io { path: PathBuf, source: io::Error },

    /// None of the Agent CLI locations that were found could be read.
    #[error("could not access {}", path.display())]
    AgentAccess { path: PathBuf, source: io::Error },

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

    /// `export --workspace` selected no session.
    #[error("no sessions matched workspace `{workspace}`")]
    NoWorkspaceMatch { workspace: String },

    #[error("session ID is empty")]
    EmptyId,
}

impl Error {
    /// `StorageNotFound` when `path` is missing, `Io` for any other failure.
    pub(crate) fn access(path: &Path, source: io::Error) -> Self {
        let path = path.to_path_buf();
        if source.kind() == io::ErrorKind::NotFound {
            Error::StorageNotFound { path }
        } else {
            Error::Io { path, source }
        }
    }

    /// The store this error comes from, when `--source` with the other store
    /// gets past it. [`Error::hints`] then starts with how.
    pub fn skippable(&self) -> Option<Source> {
        match self {
            Error::SchemaMismatch { store, .. } => Some(*store),
            Error::AgentAccess { .. } => Some(Source::Agent),
            Error::Database { source, .. } if is_busy(source) => None,
            Error::Database { path, .. }
            | Error::Io { path, .. }
            | Error::Snapshot { path, .. }
                if is_ide_db(path) =>
            {
                Some(Source::Ide)
            }
            _ => None,
        }
    }

    /// Extra guidance lines to show under the error message: lowercase
    /// imperatives without a closing period, after any indented candidates.
    pub fn hints(&self) -> Vec<String> {
        let mut hints: Vec<String> = self
            .skippable()
            .map(|store| format!("rerun with {}", store.skip_option()))
            .into_iter()
            .collect();
        hints.extend(self.advice());
        hints
    }

    fn advice(&self) -> Vec<String> {
        match self {
            Error::NoHome => vec!["set HOME or pass `--storage <path>`".to_string()],
            Error::UnsupportedStorage { .. } => vec![
                "pass a home, .cursor, chats, workspace, session or projects directory, a \
                 store.db or state.vscdb file, or the directory that holds state.vscdb"
                    .to_string(),
            ],
            Error::SessionNotFound {
                unsearched: Some(store),
                ..
            } => vec![format!(
                "rerun without `--source {}` to search {} sessions too",
                store.other().as_str(),
                store.name()
            )],
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
                "report it at https://github.com/julio-romero/cursor-session-rs/issues and \
                 include your Cursor version"
                    .to_string(),
            ],
            Error::Database { source, .. } if is_busy(source) => vec![TRY_AGAIN.to_string()],
            Error::Changed { .. } => vec![TRY_AGAIN.to_string()],
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
                vec!["pass `--storage <path>` if your Cursor data lives elsewhere".to_string()]
            }
            Error::NoWorkspaceMatch { .. } => vec![
                "list the recorded workspaces with `cursor-session list --json | jq -r \
                 '.[].workspace // empty' | sort -u`; IDE sessions record none"
                    .to_string(),
            ],
            _ => Vec::new(),
        }
    }
}

fn is_busy(source: &rusqlite::Error) -> bool {
    matches!(
        source.sqlite_error_code(),
        Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked)
    )
}

/// Whether `path` is an IDE database or its journal, which `--source agent`
/// leaves unread.
fn is_ide_db(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            let db = ["-wal", "-journal"]
                .iter()
                .find_map(|suffix| name.strip_suffix(suffix))
                .unwrap_or(name);
            db.ends_with(".vscdb") || db.ends_with(".vscdb.backup")
        })
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
            Error::SessionNotFound {
                query: "x".into(),
                unsearched: None,
            },
            Error::SessionNotFound {
                query: "x".into(),
                unsearched: Some(Source::Agent),
            },
            Error::EmptyId,
            Error::AmbiguousId {
                query: "a".into(),
                matches: (0..12).map(|n| format!("a{n}  agent  Title")).collect(),
            },
            Error::SchemaMismatch {
                store: Source::Ide,
                path: path.clone(),
                detail: "no table".into(),
            },
            Error::SchemaMismatch {
                store: Source::Agent,
                path: PathBuf::from("projects"),
                detail: "no transcript".into(),
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
            Error::NoWorkspaceMatch {
                workspace: "api".into(),
            },
        ];
        let skip_ide = "rerun with `--source agent` to skip IDE sessions";
        let ide_db = Error::Io {
            path: PathBuf::from("/Cursor/User/globalStorage/state.vscdb"),
            source: io::Error::from(io::ErrorKind::PermissionDenied),
        };
        assert_eq!(ide_db.hints(), [skip_ide]);
        assert_eq!(ide_db.skippable(), Some(Source::Ide));
        let chats = Error::AgentAccess {
            path: PathBuf::from("/home/.cursor/chats"),
            source: io::Error::from(io::ErrorKind::PermissionDenied),
        };
        assert_eq!(
            chats.hints(),
            ["rerun with `--source ide` to skip Agent CLI sessions"]
        );
        // `--storage` naming a path that cannot be read: no store to skip.
        let storage = Error::Io {
            path: PathBuf::from("/home/.cursor/chats"),
            source: io::Error::from(io::ErrorKind::PermissionDenied),
        };
        assert!(storage.hints().is_empty());
        let not_a_database = Error::Database {
            path: PathBuf::from("backup.vscdb.backup"),
            source: rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_NOTADB),
                None,
            ),
        };
        assert_eq!(not_a_database.hints(), [skip_ide]);
        // A crash left a journal that cannot be read, or the copy failed.
        let journal = Error::Io {
            path: PathBuf::from("/Cursor/User/globalStorage/state.vscdb-wal"),
            source: io::Error::from(io::ErrorKind::PermissionDenied),
        };
        assert_eq!(journal.hints(), [skip_ide]);
        let copy = Error::Snapshot {
            path: PathBuf::from("state.vscdb"),
            source: io::Error::other("disk full"),
        };
        assert_eq!(copy.hints()[0], skip_ide);
        let store_db_journal = Error::Io {
            path: PathBuf::from("store.db-journal"),
            source: io::Error::from(io::ErrorKind::PermissionDenied),
        };
        assert_eq!(store_db_journal.skippable(), None);

        for error in errors.iter().chain([&ide_db, &chats]) {
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
            if let Some(store) = error.skippable() {
                assert_eq!(hints[0], format!("rerun with {}", store.skip_option()));
            }
        }
    }

    #[test]
    fn schema_mismatch_names_the_store() {
        let mismatch = |store| Error::SchemaMismatch {
            store,
            path: PathBuf::from("x"),
            detail: "detail".into(),
        };
        assert!(
            mismatch(Source::Agent)
                .to_string()
                .starts_with("unrecognized Cursor Agent CLI storage format in x: detail.")
        );
        assert!(
            mismatch(Source::Ide)
                .to_string()
                .starts_with("unrecognized Cursor IDE storage format in x: detail.")
        );
    }
}
