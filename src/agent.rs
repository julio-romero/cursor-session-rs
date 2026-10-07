use std::collections::HashMap;
use std::fs;
use std::io::{self, BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use rusqlite::OptionalExtension;
use rusqlite::types::ValueRef;
use serde::Deserialize;
use serde_json::Value;

use crate::detect::{ChatsScope, StoragePaths};
use crate::model::{Message, Session, Source};
use crate::sqlite::with_readonly;
use crate::{Error, Result};

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct MetaJson {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    created_at_ms: Option<i64>,
    #[serde(default)]
    updated_at_ms: Option<i64>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    has_conversation: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct TranscriptLine {
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    message: Option<TranscriptMessage>,
}

#[derive(Debug, Deserialize)]
struct TranscriptMessage {
    #[serde(default)]
    content: TranscriptContent,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum TranscriptContent {
    Text(String),
    Parts(Vec<TranscriptPart>),
    Other(Value),
}

impl Default for TranscriptContent {
    fn default() -> Self {
        Self::Parts(Vec::new())
    }
}

#[derive(Debug, Deserialize)]
struct TranscriptPart {
    #[serde(rename = "type")]
    kind: Option<String>,
    text: Option<String>,
}

/// Loads the Agent CLI sessions. A location that cannot be read is reported in
/// `warnings` while the other one loads; when no location that was found can be
/// read, loading fails, so that this is not mistaken for having no sessions.
pub fn load_sessions(paths: &StoragePaths, warnings: &mut Vec<String>) -> Result<Vec<Session>> {
    let mut by_id: HashMap<String, Session> = HashMap::new();
    // Below the chats root (one workspace or one session), transcripts only
    // fill in the sessions found there.
    let scoped = paths.chats_dir.is_some() && paths.chats_scope != ChatsScope::All;
    let mut unreadable = Vec::new();

    let chats = paths.chats_dir.as_deref().and_then(|dir| {
        let scanned = scan_chats(dir, paths.chats_scope, warnings);
        readable(dir, scanned, &mut unreadable)
    });
    let transcripts = paths.projects_dir.as_deref().and_then(|dir| {
        let scanned = scan_transcripts(dir, warnings);
        readable(dir, scanned, &mut unreadable)
    });
    let read_any = chats.is_some() || transcripts.is_some();
    let mut unreadable = unreadable.into_iter();
    if !read_any && let Some((dir, err)) = unreadable.next() {
        return Err(Error::access(&dir, err));
    }
    for (dir, err) in unreadable {
        warnings.push(format!("could not read {}: {err}", dir.display()));
    }

    for session in chats.unwrap_or_default() {
        by_id.insert(session.id.clone(), session);
    }
    for (id, messages) in transcripts.unwrap_or_default() {
        if let Some(session) = by_id.get_mut(&id) {
            if session.messages.is_empty() {
                session.messages = messages;
            }
        } else if !scoped {
            by_id.insert(
                id.clone(),
                Session {
                    title: id.clone(),
                    id,
                    source: Source::Agent,
                    workspace: None,
                    workspace_hash: None,
                    created_at_ms: None,
                    updated_at_ms: None,
                    model: None,
                    messages,
                },
            );
        }
    }

    Ok(by_id.into_values().collect())
}

/// What scanning the location `dir` found, or `None` when `dir` could not be
/// read, which `unreadable` then records. A location removed since it was
/// found is not recorded.
fn readable<T>(
    dir: &Path,
    scanned: io::Result<T>,
    unreadable: &mut Vec<(PathBuf, io::Error)>,
) -> Option<T> {
    match scanned {
        Ok(found) => Some(found),
        Err(err) => {
            if err.kind() != io::ErrorKind::NotFound {
                unreadable.push((dir.to_path_buf(), err));
            }
            None
        }
    }
}

/// Files of one kind that could not be read, reported as a single warning.
struct Unreadable {
    kind: &'static str,
    details: Vec<String>,
}

impl Unreadable {
    fn new(kind: &'static str) -> Self {
        Self {
            kind,
            details: Vec::new(),
        }
    }

    fn add(&mut self, path: &Path, reason: impl std::fmt::Display) {
        self.details.push(format!("{}: {reason}", path.display()));
    }

    fn report(self, warnings: &mut Vec<String>) {
        let kind = self.kind;
        match self.details.as_slice() {
            [] => {}
            [only] => warnings.push(format!("ignored unreadable {kind} {only}")),
            [first, ..] => warnings.push(format!(
                "ignored {} unreadable {kind} files (first: {first})",
                self.details.len()
            )),
        }
    }
}

fn read_subdirs(dir: &Path) -> io::Result<Vec<PathBuf>> {
    Ok(fs::read_dir(dir)?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect())
}

/// Subdirectories of `dir`. An unreadable `dir` is reported in `warnings`.
fn subdirs(dir: &Path, warnings: &mut Vec<String>) -> Vec<PathBuf> {
    read_subdirs(dir).unwrap_or_else(|err| {
        warnings.push(format!("could not read {}: {err}", dir.display()));
        Vec::new()
    })
}

/// The underlying cause of `err`, for warnings that already name the file.
fn reason(err: &Error) -> String {
    if let Error::Snapshot { source, .. } = err {
        // The file is fine; the temporary directory is not.
        return format!(
            "could not copy it to {} for reading: {source}",
            std::env::temp_dir().display()
        );
    }
    std::error::Error::source(err).map_or_else(|| err.to_string(), ToString::to_string)
}

fn dir_name(dir: Option<&Path>) -> String {
    dir.and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_string()
}

/// Loads the sessions under `chats_dir`, which is the `scope` part of the
/// chats tree. Fails only when `chats_dir` itself cannot be read.
fn scan_chats(
    chats_dir: &Path,
    scope: ChatsScope,
    warnings: &mut Vec<String>,
) -> io::Result<Vec<Session>> {
    let mut session_dirs = Vec::new();
    match scope {
        ChatsScope::Session => {
            session_dirs.push((chats_dir.to_path_buf(), dir_name(chats_dir.parent())))
        }
        ChatsScope::Workspace => {
            let workspace_hash = dir_name(Some(chats_dir));
            for dir in read_subdirs(chats_dir)? {
                session_dirs.push((dir, workspace_hash.clone()));
            }
        }
        ChatsScope::All => {
            for workspace in read_subdirs(chats_dir)? {
                let workspace_hash = dir_name(Some(&workspace));
                for dir in subdirs(&workspace, warnings) {
                    session_dirs.push((dir, workspace_hash.clone()));
                }
            }
        }
    }

    let mut meta_files = Unreadable::new("meta.json");
    let mut stores = Unreadable::new("store.db");
    let sessions = session_dirs
        .iter()
        .filter_map(|(dir, hash)| load_chat_session(dir, hash, &mut meta_files, &mut stores))
        .collect();
    meta_files.report(warnings);
    stores.report(warnings);
    Ok(sessions)
}

fn load_chat_session(
    session_dir: &Path,
    workspace_hash: &str,
    meta_files: &mut Unreadable,
    stores: &mut Unreadable,
) -> Option<Session> {
    let id = session_dir.file_name()?.to_str()?.to_string();
    let meta_path = session_dir.join("meta.json");
    let store_path = session_dir.join("store.db");
    let meta: MetaJson = if meta_path.is_file() {
        let raw = match fs::read_to_string(&meta_path) {
            Ok(raw) => raw,
            Err(err) => {
                meta_files.add(&meta_path, err);
                return None;
            }
        };
        serde_json::from_str(&raw).unwrap_or_else(|err| {
            meta_files.add(&meta_path, err);
            MetaJson::default()
        })
    } else if store_path.is_file() {
        MetaJson::default()
    } else {
        return None;
    };

    let mut session = Session {
        id,
        title: meta
            .title
            .clone()
            .filter(|s| !s.is_empty())
            .unwrap_or_default(),
        source: Source::Agent,
        workspace: meta.cwd.clone(),
        workspace_hash: Some(workspace_hash.to_string()),
        created_at_ms: meta.created_at_ms,
        updated_at_ms: meta.updated_at_ms,
        model: None,
        messages: Vec::new(),
    };

    match read_store_meta(&store_path) {
        Ok(Some(store_meta)) => {
            if session.title.is_empty()
                && let Some(name) = store_meta.name
            {
                session.title = name;
            }
            if session.created_at_ms.is_none() {
                session.created_at_ms = store_meta.created_at;
            }
            session.model = store_meta.model;
        }
        Ok(None) => {}
        Err(reason) => stores.add(&store_path, reason),
    }

    if session.title.is_empty() {
        session.title = session.id.clone();
    }

    if meta.has_conversation == Some(false) && !store_path.is_file() && session.messages.is_empty()
    {
        return None;
    }

    Some(session)
}

struct StoreMeta {
    name: Option<String>,
    model: Option<String>,
    created_at: Option<i64>,
}

/// Reads the optional `meta` row of an agent `store.db`. `Ok(None)` means
/// there is nothing to read; `Err` carries why an existing store is unusable.
fn read_store_meta(path: &Path) -> std::result::Result<Option<StoreMeta>, String> {
    if !path.is_file() {
        return Ok(None);
    }
    let value = with_readonly(path, |conn| {
        conn.query_row("SELECT value FROM meta WHERE key = '0'", [], |row| {
            Ok(match row.get_ref(0)? {
                ValueRef::Text(bytes) | ValueRef::Blob(bytes) => Some(bytes.to_vec()),
                _ => None,
            })
        })
        .optional()
        .map_err(|source| Error::Database {
            path: path.to_path_buf(),
            source,
        })
    })
    .map_err(|err| reason(&err))?
    .flatten();
    let Some(value) = value else {
        return Ok(None);
    };
    let json =
        decode_meta_json(&value).ok_or("`meta` value is neither JSON nor hex-encoded JSON")?;
    Ok(Some(StoreMeta {
        name: json.get("name").and_then(Value::as_str).map(str::to_string),
        model: json
            .get("lastUsedModel")
            .and_then(Value::as_str)
            .map(str::to_string),
        created_at: json.get("createdAt").and_then(Value::as_i64),
    }))
}

fn decode_meta_json(raw: &[u8]) -> Option<Value> {
    if let Ok(value) = serde_json::from_slice::<Value>(raw) {
        return Some(value);
    }
    let bytes = decode_hex(std::str::from_utf8(raw).ok()?)?;
    serde_json::from_slice(&bytes).ok()
}

fn decode_hex(s: &str) -> Option<Vec<u8>> {
    let s = s.trim();
    if !s.len().is_multiple_of(2) || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

struct TranscriptCandidate {
    messages: Vec<Message>,
    modified: Option<SystemTime>,
    path: PathBuf,
}

impl TranscriptCandidate {
    fn rank(&self) -> (usize, Option<SystemTime>, &Path) {
        (self.messages.len(), self.modified, &self.path)
    }
}

fn select_transcript(
    candidates: &mut HashMap<String, TranscriptCandidate>,
    id: String,
    candidate: TranscriptCandidate,
) {
    use std::collections::hash_map::Entry;

    match candidates.entry(id) {
        Entry::Occupied(mut entry) => {
            if candidate.rank() > entry.get().rank() {
                entry.insert(candidate);
            }
        }
        Entry::Vacant(entry) => {
            entry.insert(candidate);
        }
    }
}

/// The messages of each transcript under `projects_dir`, by session ID. Fails
/// only when `projects_dir` itself cannot be read.
fn scan_transcripts(
    projects_dir: &Path,
    warnings: &mut Vec<String>,
) -> io::Result<HashMap<String, Vec<Message>>> {
    let mut candidates = HashMap::new();
    let mut unreadable = Unreadable::new("transcript");
    for project in read_subdirs(projects_dir)? {
        let transcripts = project.join("agent-transcripts");
        if !transcripts.is_dir() {
            continue;
        }
        let sessions = match fs::read_dir(&transcripts) {
            Ok(sessions) => sessions,
            Err(err) => {
                warnings.push(format!("could not read {}: {err}", transcripts.display()));
                continue;
            }
        };
        for session in sessions.flatten() {
            let dir = session.path();
            let id =
                if dir.is_file() && dir.extension().and_then(|ext| ext.to_str()) == Some("jsonl") {
                    dir.file_stem()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string()
                } else {
                    session.file_name().to_string_lossy().to_string()
                };
            let Some(path) = transcript_path(&dir, &id, warnings) else {
                continue;
            };
            let messages = match read_jsonl(&path) {
                Ok(messages) => messages,
                Err(err) => {
                    unreadable.add(&path, reason(&err));
                    continue;
                }
            };
            if !messages.is_empty() {
                let modified = fs::metadata(&path).and_then(|meta| meta.modified()).ok();
                select_transcript(
                    &mut candidates,
                    id,
                    TranscriptCandidate {
                        messages,
                        modified,
                        path,
                    },
                );
            }
        }
    }
    unreadable.report(warnings);
    Ok(candidates
        .into_iter()
        .map(|(id, candidate)| (id, candidate.messages))
        .collect())
}

fn transcript_path(dir: &Path, id: &str, warnings: &mut Vec<String>) -> Option<PathBuf> {
    if dir.is_file() {
        return (dir.extension().and_then(|e| e.to_str()) == Some("jsonl"))
            .then(|| dir.to_path_buf());
    }
    let named = dir.join(format!("{id}.jsonl"));
    if named.is_file() {
        return Some(named);
    }
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) => {
            warnings.push(format!("could not read {}: {err}", dir.display()));
            return None;
        }
    };
    entries
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().and_then(|e| e.to_str()) == Some("jsonl"))
}

pub fn read_jsonl(path: &Path) -> Result<Vec<Message>> {
    let io_err = |source| Error::Io {
        path: path.to_path_buf(),
        source,
    };
    let file = fs::File::open(path).map_err(io_err)?;
    let reader = BufReader::new(file);
    let mut messages = Vec::new();
    for line in reader.split(b'\n') {
        let line = line.map_err(io_err)?;
        // An invalid byte costs its character, not the rest of the transcript.
        let line = String::from_utf8_lossy(&line);
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(entry) = serde_json::from_str::<TranscriptLine>(line) else {
            continue;
        };
        let Some(role) = entry.role else {
            continue;
        };
        if role != "user" && role != "assistant" {
            continue;
        }
        let raw_content = entry
            .message
            .map(|m| extract_content(&m.content))
            .unwrap_or_default();
        let timestamp = extract_timestamp_tag(&raw_content);
        let content = if role == "user" {
            clean_user_text(&raw_content)
        } else {
            raw_content.trim().to_string()
        };
        if content.is_empty() {
            continue;
        }
        messages.push(Message {
            role,
            content,
            timestamp,
        });
    }
    Ok(messages)
}

fn extract_content(content: &TranscriptContent) -> String {
    match content {
        TranscriptContent::Text(text) => text.clone(),
        TranscriptContent::Parts(parts) => parts
            .iter()
            .filter(|part| part.kind.as_deref().unwrap_or("text") == "text")
            .filter_map(|part| part.text.as_deref())
            .collect::<Vec<_>>()
            .join("\n\n"),
        TranscriptContent::Other(value) => value.as_str().map(str::to_string).unwrap_or_default(),
    }
}

pub fn clean_user_text(raw: &str) -> String {
    let mut text = raw.to_string();
    while let Some(start) = text.find("<timestamp>") {
        if let Some(rel_end) = text[start..].find("</timestamp>") {
            let end = start + rel_end + "</timestamp>".len();
            text.replace_range(start..end, "");
        } else {
            break;
        }
    }
    if let Some(start) = text.find("<user_query>")
        && let Some(rel_end) = text[start..].find("</user_query>")
    {
        let inner_start = start + "<user_query>".len();
        let inner_end = start + rel_end;
        text = text[inner_start..inner_end].to_string();
    }
    text.trim().to_string()
}

fn extract_timestamp_tag(raw: &str) -> Option<String> {
    let start = raw.find("<timestamp>")? + "<timestamp>".len();
    let end = raw.find("</timestamp>")?;
    if end <= start {
        return None;
    }
    Some(raw[start..end].trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failed_copy_blames_the_temporary_directory() {
        let denied = || std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        let snapshot = Error::Snapshot {
            path: PathBuf::from("store.db"),
            source: denied(),
        };
        let text = reason(&snapshot);
        assert!(
            text.starts_with(&format!(
                "could not copy it to {} for reading: ",
                std::env::temp_dir().display()
            )),
            "{text}"
        );
        let io = Error::Io {
            path: PathBuf::from("store.db"),
            source: denied(),
        };
        assert_eq!(reason(&io), denied().to_string());
    }

    #[test]
    fn invalid_utf8_in_a_transcript_line_keeps_the_others() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.jsonl");
        let mut bytes = b"{\"role\":\"user\",\"message\":{\"content\":\"first\"}}\r\n".to_vec();
        bytes.extend(b"{\"role\":\"assistant\",\"message\":{\"content\":\"caf\xe9\"}}\n");
        bytes.extend(b"{\"role\":\"user\",\"message\":{\"content\":\"last\"}}");
        fs::write(&path, bytes).unwrap();
        let contents: Vec<String> = read_jsonl(&path)
            .unwrap()
            .into_iter()
            .map(|m| m.content)
            .collect();
        assert_eq!(contents, ["first", "caf\u{fffd}", "last"]);
    }

    #[test]
    fn strips_user_query_wrapper() {
        let raw = "<timestamp>Tue</timestamp>\n<user_query>\nhello world\n</user_query>";
        assert_eq!(clean_user_text(raw), "hello world");
    }

    fn candidate(count: usize, modified: Option<u64>, path: &str) -> TranscriptCandidate {
        TranscriptCandidate {
            messages: (0..count)
                .map(|index| Message {
                    role: "user".into(),
                    content: format!("message {index}"),
                    timestamp: None,
                })
                .collect(),
            modified: modified
                .map(|seconds| SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(seconds)),
            path: path.into(),
        }
    }

    #[test]
    fn duplicate_transcripts_are_selected_independently_of_discovery_order() {
        let cases = [
            // More messages wins even when the shorter copy is newer.
            ((13, Some(20), "z"), (40, Some(10), "a")),
            ((2, Some(10), "z"), (2, Some(20), "a")),
            ((2, None, "z"), (2, Some(10), "a")),
            ((2, Some(10), "a"), (2, Some(10), "z")),
            ((2, None, "a"), (2, None, "z")),
        ];
        for (lower, higher) in cases {
            for order in [[lower, higher], [higher, lower]] {
                let mut candidates = HashMap::new();
                for (count, modified, path) in order {
                    select_transcript(
                        &mut candidates,
                        "session".into(),
                        candidate(count, modified, path),
                    );
                }
                let expected = candidate(higher.0, higher.1, higher.2);
                assert_eq!(candidates["session"].rank(), expected.rank());
                assert_eq!(candidates.len(), 1);
            }
        }
    }

    #[test]
    fn identical_transcripts_do_not_duplicate_messages() {
        let mut candidates = HashMap::new();
        for _ in 0..2 {
            select_transcript(
                &mut candidates,
                "session".into(),
                candidate(3, Some(10), "same.jsonl"),
            );
        }
        assert_eq!(candidates["session"].messages.len(), 3);
    }

    fn write(path: &Path, contents: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    fn transcript(projects: &Path, id: &str) {
        write(
            &projects
                .join("p")
                .join("agent-transcripts")
                .join(id)
                .join(format!("{id}.jsonl")),
            &format!("{{\"role\":\"user\",\"message\":{{\"content\":\"hello {id}\"}}}}\n"),
        );
    }

    fn load(
        chats_dir: &Path,
        chats_scope: ChatsScope,
        projects_dir: Option<&Path>,
    ) -> (Vec<Session>, Vec<String>) {
        let paths = StoragePaths {
            chats_dir: Some(chats_dir.to_path_buf()),
            chats_scope,
            projects_dir: projects_dir.map(Path::to_path_buf),
            ..Default::default()
        };
        let mut warnings = Vec::new();
        let mut sessions = load_sessions(&paths, &mut warnings).unwrap();
        sessions.sort_by(|a, b| a.id.cmp(&b.id));
        (sessions, warnings)
    }

    #[test]
    fn store_db_is_optional_enrichment() {
        let dir = tempfile::tempdir().unwrap();
        let chats = dir.path().join("chats");
        let workspace = chats.join("0123abcd");
        for id in ["s1", "s2", "s3"] {
            write(
                &workspace.join(id).join("meta.json"),
                r#"{"createdAtMs":1}"#,
            );
        }
        write(&workspace.join("s4").join("meta.json"), "{not json");

        // Hex-encoded JSON stored as a BLOB.
        let hex: String = r#"{"name":"Named","lastUsedModel":"gpt-5"}"#
            .bytes()
            .map(|b| format!("{b:02x}"))
            .collect();
        let conn = rusqlite::Connection::open(workspace.join("s1").join("store.db")).unwrap();
        conn.execute_batch("CREATE TABLE meta (key TEXT PRIMARY KEY, value BLOB);")
            .unwrap();
        conn.execute(
            "INSERT INTO meta VALUES ('0', ?1)",
            [rusqlite::types::Value::Blob(hex.into_bytes())],
        )
        .unwrap();
        drop(conn);
        let conn = rusqlite::Connection::open(workspace.join("s2").join("store.db")).unwrap();
        conn.execute_batch("CREATE TABLE blobs (id TEXT PRIMARY KEY, data BLOB);")
            .unwrap();
        drop(conn);
        write(
            &workspace.join("s3").join("store.db"),
            "not a sqlite database, just text",
        );

        let (sessions, warnings) = load(&chats, ChatsScope::All, None);
        let titles: Vec<_> = sessions.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(titles, ["Named", "s2", "s3", "s4"]);
        assert_eq!(sessions[0].model.as_deref(), Some("gpt-5"));
        assert_eq!(sessions[1].created_at_ms, Some(1));

        assert_eq!(warnings.len(), 2, "{warnings:?}");
        assert!(warnings[0].starts_with("ignored unreadable meta.json "));
        assert!(warnings[0].contains("s4"));
        assert!(
            warnings[1].starts_with("ignored 2 unreadable store.db files (first: "),
            "{}",
            warnings[1]
        );
    }

    #[test]
    fn workspace_and_session_directories_scope_the_transcripts() {
        let dir = tempfile::tempdir().unwrap();
        let chats = dir.path().join("chats");
        let projects = dir.path().join("projects");
        let workspace = chats.join("0123abcd");
        for id in ["s1", "s2"] {
            write(&workspace.join(id).join("meta.json"), r#"{"title":"t"}"#);
            transcript(&projects, id);
        }
        transcript(&projects, "transcript-only");

        for (chats_dir, scope, expected) in [
            (
                chats.clone(),
                ChatsScope::All,
                &["s1", "s2", "transcript-only"][..],
            ),
            (workspace.clone(), ChatsScope::Workspace, &["s1", "s2"][..]),
            (workspace.join("s1"), ChatsScope::Session, &["s1"][..]),
        ] {
            let (sessions, warnings) = load(&chats_dir, scope, Some(&projects));
            let ids: Vec<_> = sessions.iter().map(|s| s.id.as_str()).collect();
            assert_eq!(ids, expected, "{}", chats_dir.display());
            assert!(sessions.iter().all(|s| s.messages.len() == 1));
            assert!(
                sessions
                    .iter()
                    .filter(|s| s.id != "transcript-only")
                    .all(|s| s.workspace_hash.as_deref() == Some("0123abcd"))
            );
            assert!(warnings.is_empty());
        }
    }

    #[test]
    fn stray_files_in_a_workspace_do_not_hide_its_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let chats = dir.path().join("chats");
        write(&chats.join("ws1").join("s1").join("meta.json"), "{}");
        write(&chats.join("ws2").join("s2").join("meta.json"), "{}");
        write(&chats.join("ws2").join("meta.json"), "{}");

        let (sessions, warnings) = load(&chats, ChatsScope::All, None);
        let ids: Vec<_> = sessions.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["s1", "s2"]);
        assert!(warnings.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_directories_are_reported() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let chats = dir.path().join("chats");
        write(&chats.join("ok").join("s1").join("meta.json"), "{}");
        let locked = chats.join("locked");
        write(&locked.join("s2").join("meta.json"), "{}");
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
        let readable = fs::read_dir(&locked).is_ok(); // root ignores permissions
        let (sessions, warnings) = load(&chats, ChatsScope::All, None);
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
        if readable {
            eprintln!("skipped: permissions are not enforced for this user (root)");
            return;
        }

        assert_eq!(sessions.len(), 1);
        assert_eq!(warnings.len(), 1);
        assert!(
            warnings[0].starts_with(&format!("could not read {}: ", locked.display())),
            "{}",
            warnings[0]
        );
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_locations_fail_only_when_no_other_one_loads() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let chats = dir.path().join("chats");
        let projects = dir.path().join("projects");
        write(&chats.join("ws").join("s1").join("meta.json"), "{}");
        transcript(&projects, "s2");
        let paths = |chats_dir: &Path| StoragePaths {
            chats_dir: Some(chats_dir.to_path_buf()),
            projects_dir: Some(projects.clone()),
            ..Default::default()
        };
        let lock = |path: &Path, mode| {
            fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
        };

        lock(&chats, 0o000);
        if fs::read_dir(&chats).is_ok() {
            lock(&chats, 0o755);
            eprintln!("skipped: permissions are not enforced for this user (root)");
            return;
        }
        let mut warnings = Vec::new();
        let partial = load_sessions(&paths(&chats), &mut warnings);
        lock(&projects, 0o000);
        let failed = load_sessions(&paths(&chats), &mut Vec::new());
        // A location that is gone holds no sessions; the other one still fails.
        let gone = load_sessions(&paths(&dir.path().join("gone")), &mut Vec::new());
        lock(&chats, 0o755);
        lock(&projects, 0o755);

        let partial = partial.unwrap();
        assert_eq!(partial.len(), 1);
        assert_eq!(partial[0].id, "s2");
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].starts_with(&format!("could not read {}: ", chats.display())));
        for (result, failing) in [(failed, &chats), (gone, &projects)] {
            assert!(
                matches!(&result, Err(Error::Io { path, .. }) if path == failing),
                "{result:?}"
            );
        }

        let gone = dir.path().join("gone");
        let empty = StoragePaths {
            chats_dir: Some(gone.clone()),
            projects_dir: Some(gone),
            ..Default::default()
        };
        assert!(load_sessions(&empty, &mut Vec::new()).unwrap().is_empty());
    }
}
