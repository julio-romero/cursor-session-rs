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

use std::collections::HashSet;

use crate::detect::StoragePaths;
use crate::model::{Message, MessagesAt, Session, SessionSummary, Source};

#[derive(Debug, Clone, Default)]
pub struct LoadOptions {
    /// Only open this store; `None` loads both.
    pub source: Option<Source>,
    /// Count the messages of only this many sessions, the most recently
    /// updated; `None` counts them all.
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Default)]
pub struct Loaded {
    /// Newest first, with their message counts: every session, or the
    /// `limit` most recently updated.
    pub sessions: Vec<SessionSummary>,
    /// The IDs of every session, newest first, also of those `limit` leaves
    /// out.
    pub ids: Vec<String>,
    /// Rows and files that were skipped; the binary prints them with `-v`.
    pub warnings: Vec<String>,
    /// Problems that change what is shown although loading went on, such as
    /// a location that could not be read; the binary always prints them.
    pub notices: Vec<String>,
}

/// Lists the sessions of both stores, or of `opts.source`, newest first, with
/// how many messages each has. No session's messages are kept: transcripts
/// are read a line at a time and IDE chats one at a time. With `opts.limit`,
/// only the messages of the sessions listed are counted.
pub fn load_sessions(paths: &StoragePaths, opts: &LoadOptions) -> Result<Loaded> {
    let (mut warnings, mut notices) = (Vec::new(), Vec::new());
    let index = Index::new(paths, opts.source, &mut warnings, &mut notices)?;
    let ids: Vec<String> = model::merge_sessions(index.sessions())
        .into_iter()
        .map(|session| session.id)
        .collect();
    let listed: HashSet<&str> = ids
        .iter()
        .take(opts.limit.unwrap_or(usize::MAX))
        .map(String::as_str)
        .collect();
    let counted = index.count(&|id| listed.contains(id), &mut warnings, &mut notices)?;
    Ok(Loaded {
        sessions: model::merge_sessions(counted),
        ids,
        warnings,
        notices,
    })
}

/// Loads the session that `query` names, as [`find_session`] looks it up
/// among those [`load_sessions`] lists, with its messages. Only its own
/// messages are read. What loading skipped or left out goes to `warnings` and
/// `notices` even when it fails.
pub fn load_session(
    paths: &StoragePaths,
    opts: &LoadOptions,
    query: &str,
    warnings: &mut Vec<String>,
    notices: &mut Vec<String>,
) -> Result<Session> {
    let index = Index::new(paths, opts.source, warnings, notices)?;
    let listed = model::merge_sessions(index.sessions());
    let id = find_session(&listed, query)?.id.clone();
    let counted = index.count(&|candidate| candidate == id, warnings, notices)?;
    let summary = model::merge_sessions(counted)
        .into_iter()
        .next()
        .ok_or_else(|| Error::SessionNotFound {
            query: query.to_string(),
            unsearched: None,
        })?;
    load_messages(&summary)
}

/// The session `summary` lists, with its messages read from where the
/// summary says they are.
pub fn load_messages(summary: &SessionSummary) -> Result<Session> {
    let messages = match &summary.messages_at {
        MessagesAt::Nowhere => Vec::new(),
        MessagesAt::Transcript(path) => agent::read_jsonl(path)?,
        MessagesAt::IdeChat { db, key, blob_key } => {
            ide::read_messages(db, key, *blob_key, &summary.id)?
        }
    };
    Ok(Session::new(summary.clone(), messages))
}

/// Calls `visit` with each message of the session `summary` lists, in order:
/// the messages [`load_messages`] returns, without holding them all. A
/// transcript is read a line at a time; an IDE chat is read whole, as one
/// chat at a time is.
pub fn visit_messages(summary: &SessionSummary, visit: &mut dyn FnMut(Message)) -> Result<()> {
    match &summary.messages_at {
        MessagesAt::Nowhere => Ok(()),
        MessagesAt::Transcript(path) => agent::visit_jsonl(path, visit),
        MessagesAt::IdeChat { db, key, blob_key } => {
            ide::read_messages(db, key, *blob_key, &summary.id)?
                .into_iter()
                .for_each(visit);
            Ok(())
        }
    }
}

/// The sessions of the stores to load, found without counting messages.
struct Index {
    agent: Option<agent::Index>,
    ide: Option<ide::Index>,
}

impl Index {
    fn new(
        paths: &StoragePaths,
        source: Option<Source>,
        warnings: &mut Vec<String>,
        notices: &mut Vec<String>,
    ) -> Result<Self> {
        let wanted = |store| source.is_none_or(|source| source == store);
        let agent = if wanted(Source::Agent) {
            Some(agent::index(paths, warnings, notices)?)
        } else {
            None
        };
        let ide = match &paths.global_storage_db {
            Some(db) if wanted(Source::Ide) => ide::index(db, warnings)?,
            _ => None,
        };
        Ok(Self { agent, ide })
    }

    /// Every session found, with no messages counted.
    fn sessions(&self) -> Vec<SessionSummary> {
        let agent = self.agent.iter().flat_map(|index| &index.sessions);
        let ide = self.ide.iter().flat_map(|index| &index.sessions);
        agent.chain(ide).cloned().collect()
    }

    /// The sessions whose ID `selected` accepts, from each store, with their
    /// messages counted.
    fn count(
        &self,
        selected: &dyn Fn(&str) -> bool,
        warnings: &mut Vec<String>,
        notices: &mut Vec<String>,
    ) -> Result<Vec<SessionSummary>> {
        let mut counted = Vec::new();
        if let Some(index) = &self.agent {
            counted.extend(agent::count(index, selected, warnings));
        }
        if let Some(index) = &self.ide {
            counted.extend(ide::count(index, selected, warnings, notices)?);
        }
        Ok(counted)
    }
}

/// Title width in ambiguous-ID candidate lines.
const CANDIDATE_TITLE_WIDTH: usize = 60;

/// Looks up a session by exact ID, then by a unique case-insensitive ID prefix.
pub fn find_session<'a>(sessions: &'a [SessionSummary], query: &str) -> Result<&'a SessionSummary> {
    let query = query.trim();
    if query.is_empty() {
        return Err(Error::EmptyId);
    }
    if let Some(exact) = sessions.iter().find(|s| s.id == query) {
        return Ok(exact);
    }
    let matches: Vec<&SessionSummary> = sessions
        .iter()
        .filter(|s| has_prefix_ignore_case(&s.id, query))
        .collect();
    // A full ID typed in another case still beats longer IDs sharing it as a prefix.
    let exact: Vec<&SessionSummary> = matches
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
/// and a trailing one is ignored. A session is also found by the MD5 hash of
/// the exact path given, so that one whose `meta.json` records no path is too.
pub fn filter_workspace<'a>(
    sessions: &'a [SessionSummary],
    workspace: &str,
) -> Vec<&'a SessionSummary> {
    let wanted = separated(workspace);
    let hashed = is_absolute(&wanted).then(|| {
        let path = match workspace.trim_end_matches(['/', '\\']) {
            "" => workspace,
            trimmed => trimmed,
        };
        detect::workspace_md5(path)
    });
    sessions
        .iter()
        .filter(|session| {
            session
                .workspace
                .as_deref()
                .is_some_and(|cwd| in_workspace(&separated(cwd), &wanted))
                || session.workspace_hash.as_deref().is_some_and(|hash| {
                    hash == workspace || hashed.as_deref().is_some_and(|hashed| hash == hashed)
                })
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

/// Whether `path`, with `/` as its separator, is absolute on Unix or Windows.
fn is_absolute(path: &str) -> bool {
    path.starts_with('/')
        || path.as_bytes().get(1) == Some(&b':') && path.as_bytes()[0].is_ascii_alphabetic()
}

fn in_workspace(cwd: &str, wanted: &str) -> bool {
    if is_absolute(wanted) {
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
            ..Default::default()
        };
        let loaded = load_sessions(&paths, &agent_only).unwrap();
        assert_eq!(loaded.sessions.len(), 1);
        assert_eq!(loaded.sessions[0].source, Source::Agent);

        let ide_only = LoadOptions {
            source: Some(Source::Ide),
            ..Default::default()
        };
        assert!(matches!(
            load_sessions(&paths, &ide_only),
            Err(Error::Database { .. })
        ));
        assert!(load_sessions(&paths, &LoadOptions::default()).is_err());
    }

    fn session(id: &str, source: Source) -> SessionSummary {
        SessionSummary::new(id, format!("title of {id}"), source)
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
        let at = |id: &str, cwd: &str| SessionSummary {
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

        // Without its path, a session is found by the hash of the exact path.
        let unrecorded = [SessionSummary {
            workspace_hash: Some(detect::workspace_md5("/Users/dana/src/web")),
            ..session("unrecorded", Source::Agent)
        }];
        let found = |workspace: &str| filter_workspace(&unrecorded, workspace).len();
        assert_eq!(found("/Users/dana/src/web"), 1);
        assert_eq!(found("/Users/dana/src/web/"), 1);
        for nothing in ["/Users/dana/src", "web", "src/web"] {
            assert_eq!(found(nothing), 0, "{nothing:?}");
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
        let sessions: Vec<SessionSummary> = (0..12)
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
