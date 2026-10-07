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

/// Title width in ambiguous-ID candidate lines.
const CANDIDATE_TITLE_WIDTH: usize = 60;

/// Looks up a session by exact ID, then by a unique case-insensitive ID prefix.
pub fn find_session<'a>(sessions: &'a [Session], query: &str) -> Result<&'a Session> {
    let query = query.trim();
    if query.is_empty() {
        return Err(Error::EmptyId);
    }
    if let Some(exact) = sessions.iter().find(|s| s.id == query) {
        return Ok(exact);
    }
    let matches: Vec<&Session> = sessions
        .iter()
        .filter(|s| has_prefix_ignore_case(&s.id, query))
        .collect();
    // A full ID typed in another case still beats longer IDs sharing it as a prefix.
    let exact: Vec<&Session> = matches
        .iter()
        .copied()
        .filter(|s| s.id.len() == query.len())
        .collect();
    match (matches.as_slice(), exact.as_slice()) {
        ([], _) => Err(Error::SessionNotFound {
            query: query.to_string(),
        }),
        ([only], _) | (_, [only]) => Ok(only),
        _ => Err(Error::AmbiguousId {
            query: query.to_string(),
            matches: matches
                .iter()
                .map(|s| {
                    format!(
                        "{}  {:<5}  {}",
                        s.id,
                        s.source.as_str(),
                        ui::truncate_chars(&s.title, CANDIDATE_TITLE_WIDTH)
                    )
                })
                .collect(),
        }),
    }
}

fn has_prefix_ignore_case(id: &str, prefix: &str) -> bool {
    id.get(..prefix.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
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

    fn session(id: &str, source: Source) -> Session {
        Session {
            id: id.to_string(),
            title: format!("title of {id}"),
            source,
            workspace: None,
            workspace_hash: None,
            created_at_ms: None,
            updated_at_ms: None,
            model: None,
            messages: Vec::new(),
        }
    }

    #[test]
    fn find_session_prefers_exact_then_unique_prefix() {
        let sessions = [
            session("f4eea6d2-d2d3-41ad-b290-824445295a15", Source::Agent),
            session("abc", Source::Ide),
            session("abcdef", Source::Ide),
        ];
        let found = |query| find_session(&sessions, query).map(|s| s.id.as_str());
        assert_eq!(found("abc").unwrap(), "abc");
        assert_eq!(found("abcd").unwrap(), "abcdef");
        assert_eq!(found("f4eea6d2").unwrap(), sessions[0].id);
        assert_eq!(found("F4EEA6D2-D2D3").unwrap(), sessions[0].id);
        assert_eq!(found("  f4eea6d2\n").unwrap(), sessions[0].id);
        assert_eq!(found("ABC").unwrap(), "abc");
    }

    #[test]
    fn find_session_rejects_empty_and_unknown_queries() {
        let sessions = [session("abc", Source::Agent)];
        assert!(matches!(find_session(&sessions, ""), Err(Error::EmptyId)));
        assert!(matches!(
            find_session(&sessions, " \t"),
            Err(Error::EmptyId)
        ));

        let err = find_session(&sessions, "zzz").unwrap_err();
        assert_eq!(err.to_string(), "session not found: zzz");
        assert_eq!(
            err.hints(),
            ["run `cursor-session list` to see session IDs"]
        );
        assert!(matches!(
            find_session(&[], "abc"),
            Err(Error::SessionNotFound { .. })
        ));
    }

    #[test]
    fn ambiguous_prefix_lists_ten_candidates() {
        let sessions: Vec<Session> = (0..12)
            .map(|n| session(&format!("abc{n:02}"), Source::Agent))
            .collect();
        let err = find_session(&sessions, "AB").unwrap_err();
        assert_eq!(
            err.to_string(),
            r#"session id prefix "AB" is ambiguous (12 matches)"#
        );
        let hints = err.hints();
        assert_eq!(hints.len(), 12);
        assert_eq!(hints[0], "  abc00  agent  title of abc00");
        assert_eq!(hints[9], "  abc09  agent  title of abc09");
        assert_eq!(hints[10], "  and 2 more");
        assert_eq!(hints[11], "use more characters of the ID");

        let err = find_session(&sessions[..2], "abc").unwrap_err();
        assert_eq!(
            err.hints(),
            [
                "  abc00  agent  title of abc00",
                "  abc01  agent  title of abc01",
                "use more characters of the ID",
            ]
        );
    }
}
