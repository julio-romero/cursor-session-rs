pub mod agent;
pub mod detect;
mod error;
pub mod export;
pub mod ide;
mod json;
pub mod model;
mod sqlite;
pub mod ui;

pub use crate::error::{Error, Result};
pub use crate::sqlite::remove_stale_snapshot_copies;

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
    /// Rows and files that were skipped; the binary prints them with `-v`.
    pub warnings: Vec<String>,
    /// Problems that change what is shown although loading went on, such as
    /// a location that could not be read; the binary always prints them.
    pub notices: Vec<String>,
}

pub fn load_sessions(paths: &StoragePaths, opts: &LoadOptions) -> Result<Loaded> {
    let mut sessions = Vec::new();
    let mut warnings = Vec::new();
    let mut notices = Vec::new();
    if opts.source.is_none_or(|source| source == Source::Agent) {
        sessions.extend(agent::load_sessions(paths, &mut warnings, &mut notices)?);
    }
    if opts.source.is_none_or(|source| source == Source::Ide) {
        sessions.extend(ide::load_sessions(paths, &mut warnings, &mut notices)?);
    }
    Ok(Loaded {
        sessions: model::merge_sessions(sessions),
        warnings,
        notices,
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
            unsearched: None,
        }),
        ([only], _) | (_, [only]) => Ok(only),
        _ => Err(Error::AmbiguousId {
            query: query.to_string(),
            matches: matches
                .iter()
                .map(|s| {
                    format!(
                        "{}  {:<5}  {}",
                        ui::one_line(&s.id),
                        s.source.as_str(),
                        ui::truncate_chars(&ui::one_line(&s.title), CANDIDATE_TITLE_WIDTH)
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

/// The sessions of a workspace, which `workspace` names by its path or a
/// directory above it (absolute), by whole directory names in its path
/// (`api`, `src/api`), or by the MD5 hash of its path that names its
/// directory under `~/.cursor/chats`. `/` and `\` both separate directories,
/// and a trailing one is ignored.
pub fn filter_workspace<'a>(sessions: &'a [Session], workspace: &str) -> Vec<&'a Session> {
    let wanted = separated(workspace);
    sessions
        .iter()
        .filter(|session| {
            session
                .workspace
                .as_deref()
                .is_some_and(|cwd| in_workspace(&separated(cwd), &wanted))
                || session
                    .workspace_hash
                    .as_deref()
                    .is_some_and(|hash| hash == workspace)
        })
        .collect()
}

/// `path` with `/` as its only separator and without a trailing one, unless
/// it is the root.
fn separated(path: &str) -> String {
    let path = path.replace('\\', "/");
    match path.trim_end_matches('/') {
        "" if path.starts_with('/') => "/".to_string(),
        trimmed => trimmed.to_string(),
    }
}

fn in_workspace(cwd: &str, wanted: &str) -> bool {
    let absolute = wanted.starts_with('/')
        || wanted.as_bytes().get(1) == Some(&b':') && wanted.as_bytes()[0].is_ascii_alphabetic();
    if absolute {
        let below = format!("{}/", wanted.trim_end_matches('/'));
        cwd == wanted || cwd.starts_with(&below)
    } else {
        !wanted.is_empty() && format!("/{cwd}/").contains(&format!("/{wanted}/"))
    }
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

        // An exact match beats one that differs only in case.
        let cased = [session("abcd", Source::Ide), session("ABCD", Source::Ide)];
        assert_eq!(find_session(&cased, "ABCD").unwrap().id, "ABCD");
        assert_eq!(find_session(&cased, "abcd").unwrap().id, "abcd");
        assert!(matches!(
            find_session(&cased, "AbCd"),
            Err(Error::AmbiguousId { .. })
        ));
    }

    #[test]
    fn workspaces_match_by_whole_directories() {
        let at = |id: &str, cwd: &str| Session {
            workspace: Some(cwd.to_string()),
            workspace_hash: Some(format!("hash-{id}")),
            ..session(id, Source::Agent)
        };
        let sessions = [
            at("api", "/Users/dana/src/api"),
            at("nested", "/Users/dana/src/api/tools"),
            at("gateway", "/Users/dana/src/api-gateway"),
            at("dotted", "/Users/dana/v1.2"),
            at("windows", r"C:\Users\dana\api"),
        ];
        let found = |workspace: &str| -> Vec<&str> {
            filter_workspace(&sessions, workspace)
                .iter()
                .map(|s| s.id.as_str())
                .collect()
        };
        assert_eq!(found("/Users/dana/src/api"), ["api", "nested"]);
        // A tab-completed directory ends in a separator.
        assert_eq!(found("/Users/dana/src/api/"), ["api", "nested"]);
        assert_eq!(found("api"), ["api", "nested", "windows"]);
        assert_eq!(found("src/api"), ["api", "nested"]);
        assert_eq!(found("/Users/dana/src/ap"), Vec::<&str>::new());
        assert_eq!(found(r"C:\Users\dana\"), ["windows"]);
        assert_eq!(found("/"), ["api", "nested", "gateway", "dotted"]);
        assert_eq!(found("hash-gateway"), ["gateway"]);
        for nothing in [".", "", "v1", "dana/src/ap"] {
            assert_eq!(found(nothing), Vec::<&str>::new(), "{nothing:?}");
        }
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
            r#"session ID prefix "AB" is ambiguous (12 matches)"#
        );
        let hints = err.hints();
        assert_eq!(hints.len(), 12);
        assert_eq!(hints[0], "  abc00  agent  title of abc00");
        assert_eq!(hints[9], "  abc09  agent  title of abc09");
        assert_eq!(hints[10], "  and 2 more");
        assert_eq!(hints[11], "use more characters of the ID");

        // A title on several lines stays on its candidate's line.
        let mut multiline = sessions[..2].to_vec();
        multiline[0].title = "first line\nsecond\tline\r\n".into();
        let hints = find_session(&multiline, "abc").unwrap_err().hints();
        assert_eq!(hints[0], "  abc00  agent  first line second line  ");

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
