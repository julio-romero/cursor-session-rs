pub mod agent;
pub mod detect;
mod error;
pub mod export;
pub mod ide;
pub mod model;
mod sqlite;
pub mod ui;

pub use crate::error::{Error, Result};

use crate::detect::StoragePaths;
use crate::model::{Session, Source};

#[derive(Debug, Clone, Default)]
pub struct LoadOptions {
    /// Only open this store; `None` loads both.
    pub source: Option<Source>,
}

#[derive(Debug, Clone, Default)]
pub struct Loaded {
    pub sessions: Vec<Session>,
    pub warnings: Vec<String>,
}

pub fn load_sessions(paths: &StoragePaths, opts: &LoadOptions) -> Result<Loaded> {
    let mut sessions = Vec::new();
    let mut warnings = Vec::new();
    if opts.source.is_none_or(|source| source == Source::Agent) {
        sessions.extend(agent::load_sessions(paths, &mut warnings)?);
    }
    if opts.source.is_none_or(|source| source == Source::Ide) {
        sessions.extend(ide::load_sessions(paths, &mut warnings)?);
    }
    Ok(Loaded {
        sessions: model::merge_sessions(sessions),
        warnings,
    })
}

pub fn find_session<'a>(sessions: &'a [Session], query: &str) -> Option<&'a Session> {
    if let Some(exact) = sessions.iter().find(|s| s.id == query) {
        return Some(exact);
    }
    let matches: Vec<_> = sessions
        .iter()
        .filter(|s| s.id.starts_with(query))
        .collect();
    if matches.len() == 1 {
        Some(matches[0])
    } else {
        None
    }
}

pub fn filter_workspace<'a>(sessions: &'a [Session], workspace: &str) -> Vec<&'a Session> {
    sessions
        .iter()
        .filter(|session| {
            session
                .workspace
                .as_deref()
                .is_some_and(|cwd| cwd == workspace || cwd.contains(workspace))
                || session
                    .workspace_hash
                    .as_deref()
                    .is_some_and(|hash| hash == workspace)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_filter_only_opens_that_store() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("state.vscdb");
        std::fs::write(
            &db,
            "not a sqlite database, only some text to fill the header",
        )
        .unwrap();
        let paths = StoragePaths {
            projects_dir: Some(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/fixtures/home/.cursor/projects"),
            ),
            global_storage_db: Some(db),
            ..Default::default()
        };

        let agent_only = LoadOptions {
            source: Some(Source::Agent),
        };
        let loaded = load_sessions(&paths, &agent_only).unwrap();
        assert_eq!(loaded.sessions.len(), 1);
        assert_eq!(loaded.sessions[0].source, Source::Agent);

        let ide_only = LoadOptions {
            source: Some(Source::Ide),
        };
        assert!(matches!(
            load_sessions(&paths, &ide_only),
            Err(Error::Database { .. })
        ));
        assert!(load_sessions(&paths, &LoadOptions::default()).is_err());
    }
}
