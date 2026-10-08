use std::collections::HashMap;
use std::fs;
use std::io::{self, BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use rusqlite::OptionalExtension;
use rusqlite::types::ValueRef;
use serde::Deserialize;
use serde::de::IgnoredAny;
use serde_json::Value;

use crate::detect::{ChatsScope, StoragePaths};
use crate::json::{self, lenient, lenient_ms};
use crate::model::{Message, MessagesAt, SessionSummary, Source, TOOL_ROLE, content_chars};
use crate::sqlite::with_readonly;
use crate::{Error, ReadOptions, Result, tools};

/// A session's `meta.json`. Each field is read leniently, so that one value of
/// an unexpected type costs that value, not the others.
#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct MetaJson {
    #[serde(default, deserialize_with = "lenient")]
    title: Option<String>,
    #[serde(default, deserialize_with = "lenient_ms")]
    created_at_ms: Option<i64>,
    #[serde(default, deserialize_with = "lenient_ms")]
    updated_at_ms: Option<i64>,
    #[serde(default, deserialize_with = "lenient")]
    cwd: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    has_conversation: Option<bool>,
}

/// The keys of `meta.json` that are read.
const META_KEYS: [&str; 5] = [
    "title",
    "createdAtMs",
    "updatedAtMs",
    "cwd",
    "hasConversation",
];

impl MetaJson {
    /// Parses a `meta.json`: `None` when it is blank or an empty object, which
    /// holds nothing to read, and `Err` with why its format is not one this
    /// version knows.
    fn parse(raw: &str) -> std::result::Result<Option<Self>, String> {
        if raw.trim().is_empty() {
            return Ok(None);
        }
        let value: Value = json::from_str(raw).map_err(|err| err.to_string())?;
        let Value::Object(fields) = &value else {
            return Err("not a JSON object".to_string());
        };
        if fields.is_empty() {
            return Ok(None);
        }
        if !META_KEYS.iter().any(|key| fields.contains_key(*key)) {
            return Err(format!("none of the keys {} found", META_KEYS.join(", ")));
        }
        Ok(Some(serde_json::from_value(value).unwrap_or_default()))
    }
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
    /// Anything else, which holds no text.
    Other(IgnoredAny),
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

/// Loads the Agent CLI sessions with their message counts; see [`index`] and
/// [`count`].
pub fn load_sessions(
    paths: &StoragePaths,
    warnings: &mut Vec<String>,
    notices: &mut Vec<String>,
) -> Result<Vec<SessionSummary>> {
    let index = index(paths, warnings, notices)?;
    Ok(count(&index, &|_| true, warnings))
}

/// The Agent CLI sessions, without their messages counted.
pub(crate) struct Index {
    pub sessions: Vec<SessionSummary>,
    /// The transcripts of each session that hold a message.
    transcripts: HashMap<String, Vec<TranscriptFile>>,
}

/// Finds the Agent CLI sessions, reading each transcript only up to its first
/// message. A location that cannot be read is reported in `notices` while the
/// other one loads; when no location that was found can be read, loading
/// fails, so that this is not mistaken for having no sessions. So does a
/// transcript format this version cannot read. Files that cannot be read are
/// reported in `warnings`.
pub(crate) fn index(
    paths: &StoragePaths,
    warnings: &mut Vec<String>,
    notices: &mut Vec<String>,
) -> Result<Index> {
    // Below the chats root (one workspace or one session), transcripts only
    // fill in the sessions found there.
    let scoped = paths.chats_dir.is_some() && paths.chats_scope != ChatsScope::All;
    let mut unreadable = Vec::new();

    let chats = paths.chats_dir.as_deref().and_then(|dir| {
        let scanned = scan_chats(dir, paths.chats_scope, warnings, notices);
        readable(dir, scanned, &mut unreadable)
    });
    let transcripts = paths.projects_dir.as_deref().and_then(|dir| {
        let scanned = scan_transcripts(dir, warnings);
        readable(dir, scanned, &mut unreadable).map(|found| (dir, found))
    });
    let read_any = chats.is_some() || transcripts.is_some();
    let mut unreadable = unreadable.into_iter();
    if !read_any && let Some((path, source)) = unreadable.next() {
        return Err(Error::AgentAccess { path, source });
    }
    for (dir, err) in unreadable {
        notices.push(format!("could not read {}: {err}", dir.display()));
    }
    let transcripts = match transcripts {
        Some((dir, found)) if found.unrecognized > 0 && found.unrecognized == found.read => {
            return Err(Error::SchemaMismatch {
                store: Source::Agent,
                path: dir.to_path_buf(),
                detail: match found.read {
                    1 => "its one transcript has no readable message".to_string(),
                    n => format!("none of its {n} transcripts has a readable message"),
                },
            });
        }
        Some((_, found)) => found.by_id,
        None => HashMap::new(),
    };

    let mut by_id: HashMap<String, SessionSummary> = HashMap::new();
    for session in chats.unwrap_or_default() {
        by_id.insert(session.id.clone(), session);
    }
    if !scoped {
        for id in transcripts.keys() {
            by_id
                .entry(id.clone())
                .or_insert_with(|| SessionSummary::new(id.clone(), id.clone(), Source::Agent));
        }
    }
    Ok(Index {
        sessions: by_id.into_values().collect(),
        transcripts,
    })
}

/// The sessions of `index` that `selected` accepts, each with the number of
/// messages in its transcript, the characters of their content, and where
/// that is. Of several copies of a transcript, the one with the most messages
/// is used. Transcripts that cannot be read are reported in `warnings`.
pub(crate) fn count(
    index: &Index,
    selected: &dyn Fn(&str) -> bool,
    warnings: &mut Vec<String>,
) -> Vec<SessionSummary> {
    let mut unreadable = Unreadable::new("transcript");
    let counted = index
        .sessions
        .iter()
        .filter(|session| selected(&session.id))
        .map(|session| {
            let mut session = session.clone();
            let files = index
                .transcripts
                .get(&session.id)
                .map_or(&[][..], Vec::as_slice);
            let mut best: Option<TranscriptCandidate> = None;
            for file in files {
                match scan_transcript(&file.path, None, false) {
                    // A copy that lost its messages since it was found is not one.
                    Ok(scan) if scan.messages > 0 => select_transcript(
                        &mut best,
                        TranscriptCandidate {
                            messages: scan.messages,
                            chars: scan.chars,
                            modified: file.modified,
                            path: file.path.clone(),
                        },
                    ),
                    Ok(_) => {}
                    Err(err) => unreadable.add(&file.path, reason(&err)),
                }
            }
            if let Some(best) = best {
                session.message_count = best.messages;
                session.content_chars = best.chars;
                session.messages_at = MessagesAt::Transcript(best.path);
            }
            session
        })
        .collect();
    unreadable.report(warnings);
    counted
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
    /// How many of them hold a format this version does not know.
    unrecognized: usize,
    /// How many files of this kind held something to read, readable or not.
    read: usize,
}

impl Unreadable {
    fn new(kind: &'static str) -> Self {
        Self {
            kind,
            details: Vec::new(),
            unrecognized: 0,
            read: 0,
        }
    }

    fn add(&mut self, path: &Path, reason: impl std::fmt::Display) {
        self.details.push(format!("{}: {reason}", path.display()));
    }

    fn add_unrecognized(&mut self, path: &Path, reason: impl std::fmt::Display) {
        self.unrecognized += 1;
        self.add(path, reason);
    }

    /// Reports a notice when every file of this kind that held something has
    /// a format this version does not know, and the files as warnings
    /// otherwise.
    fn report_format_change(
        self,
        dir: &Path,
        left_out: &str,
        warnings: &mut Vec<String>,
        notices: &mut Vec<String>,
    ) {
        if self.unrecognized > 0 && self.unrecognized == self.read {
            let kind = self.kind;
            notices.push(format!(
                "unrecognized {kind} format in {}: none of its {} {kind} files could be read \
                 ({}); {left_out}. Cursor may have changed its storage format.",
                dir.display(),
                self.read,
                self.details[0]
            ));
        } else {
            self.report(warnings);
        }
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
    notices: &mut Vec<String>,
) -> io::Result<Vec<SessionSummary>> {
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
    let sessions: Vec<SessionSummary> = session_dirs
        .iter()
        .filter_map(|(dir, hash)| load_chat_session(dir, hash, &mut meta_files, &mut stores))
        .collect();
    // Not one file of a kind in a format this version knows: rather than a
    // skipped file, the format changed, and what they hold is missing
    // throughout.
    meta_files.report_format_change(
        chats_dir,
        "the session titles, workspaces and times they hold are left out",
        warnings,
        notices,
    );
    stores.report_format_change(
        chats_dir,
        "the session names and models they hold are left out",
        warnings,
        notices,
    );
    Ok(sessions)
}

fn load_chat_session(
    session_dir: &Path,
    workspace_hash: &str,
    meta_files: &mut Unreadable,
    stores: &mut Unreadable,
) -> Option<SessionSummary> {
    let id = session_dir.file_name()?.to_str()?.to_string();
    let meta_path = session_dir.join("meta.json");
    let store_path = session_dir.join("store.db");
    let meta: MetaJson = if meta_path.is_file() {
        let raw = match fs::read_to_string(&meta_path) {
            Ok(raw) => raw,
            Err(err) => {
                meta_files.read += 1;
                meta_files.add(&meta_path, err);
                return None;
            }
        };
        match MetaJson::parse(&raw) {
            Ok(meta) => {
                meta_files.read += usize::from(meta.is_some());
                meta.unwrap_or_default()
            }
            Err(reason) => {
                meta_files.read += 1;
                meta_files.add_unrecognized(&meta_path, reason);
                MetaJson::default()
            }
        }
    } else if store_path.is_file() {
        MetaJson::default()
    } else {
        return None;
    };

    let title = meta
        .title
        .clone()
        .filter(|s| !s.is_empty())
        .unwrap_or_default();
    let mut session = SessionSummary {
        workspace: meta.cwd.clone(),
        workspace_hash: Some(workspace_hash.to_string()),
        created_at_ms: meta.created_at_ms,
        updated_at_ms: meta.updated_at_ms,
        ..SessionSummary::new(id, title, Source::Agent)
    };

    let store = read_store_meta(&store_path);
    stores.read += usize::from(!matches!(store, Ok(None)));
    match store {
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
        Err(StoreError::Format(reason)) => stores.add_unrecognized(&store_path, reason),
        Err(StoreError::Other(reason)) => stores.add(&store_path, reason),
    }

    if session.title.is_empty() {
        session.title = session.id.clone();
    }

    // Its transcript, if any, is read later, so this session is left out
    // even then; the transcript makes a session of its own.
    if meta.has_conversation == Some(false) && !store_path.is_file() {
        return None;
    }

    Some(session)
}

#[derive(Default)]
struct StoreMeta {
    name: Option<String>,
    model: Option<String>,
    created_at: Option<i64>,
}

/// Why an existing `store.db` is unusable.
enum StoreError {
    /// It has no `meta` table or column, or a value that is not JSON: a
    /// format this version does not know.
    Format(String),
    Other(String),
}

/// Reads the optional `meta` row of an agent `store.db`. `Ok(None)` means
/// there is no database to read: no file, an empty one, or one without
/// tables, as a session that was never used can leave.
fn read_store_meta(path: &Path) -> std::result::Result<Option<StoreMeta>, StoreError> {
    if !fs::metadata(path).is_ok_and(|meta| meta.is_file() && meta.len() > 0) {
        return Ok(None);
    }
    let value = with_readonly(path, |conn| {
        let db_err = |source| Error::Database {
            path: path.to_path_buf(),
            source,
        };
        let tables: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type = 'table'",
                [],
                |row| row.get(0),
            )
            .map_err(db_err)?;
        if tables == 0 {
            return Ok(None);
        }
        conn.query_row("SELECT value FROM meta WHERE key = '0'", [], |row| {
            Ok(match row.get_ref(0)? {
                ValueRef::Text(bytes) | ValueRef::Blob(bytes) => Some(bytes.to_vec()),
                _ => None,
            })
        })
        .optional()
        .map(|value| Some(value.flatten()))
        .map_err(db_err)
    })
    .map_err(|err| {
        if is_missing_schema(&err) {
            StoreError::Format(reason(&err))
        } else {
            StoreError::Other(reason(&err))
        }
    })?;
    let Some(value) = value else {
        return Ok(None);
    };
    let Some(value) = value else {
        return Ok(Some(StoreMeta::default()));
    };
    let json = decode_meta_json(&value).ok_or_else(|| {
        StoreError::Format("`meta` value is neither JSON nor hex-encoded JSON".to_string())
    })?;
    Ok(Some(StoreMeta {
        name: json.get("name").and_then(Value::as_str).map(str::to_string),
        model: json
            .get("lastUsedModel")
            .and_then(Value::as_str)
            .map(str::to_string),
        created_at: json.get("createdAt").and_then(Value::as_i64),
    }))
}

/// Whether a query failed for want of the table or column it names.
fn is_missing_schema(err: &Error) -> bool {
    matches!(
        err,
        Error::Database {
            source: rusqlite::Error::SqliteFailure(_, Some(message)),
            ..
        } if message.starts_with("no such table") || message.starts_with("no such column")
    )
}

fn decode_meta_json(raw: &[u8]) -> Option<Value> {
    let text = std::str::from_utf8(raw).ok()?;
    if let Ok(value) = json::from_str(text) {
        return Some(value);
    }
    let bytes = decode_hex(text)?;
    json::from_str(std::str::from_utf8(&bytes).ok()?).ok()
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
    messages: usize,
    /// The characters of their content.
    chars: usize,
    modified: Option<SystemTime>,
    path: PathBuf,
}

impl TranscriptCandidate {
    fn rank(&self) -> (usize, Option<SystemTime>, &Path) {
        (self.messages, self.modified, &self.path)
    }
}

/// Keeps the better of `best` and `candidate`: more messages, then the newer
/// file, then the greater path, so that the order files are found in does not
/// matter.
fn select_transcript(best: &mut Option<TranscriptCandidate>, candidate: TranscriptCandidate) {
    if best
        .as_ref()
        .is_none_or(|best| candidate.rank() > best.rank())
    {
        *best = Some(candidate);
    }
}

/// A transcript that holds a message.
struct TranscriptFile {
    path: PathBuf,
    modified: Option<SystemTime>,
}

/// What [`scan_transcripts`] found.
struct Transcripts {
    /// The transcripts of each session that hold a message.
    by_id: HashMap<String, Vec<TranscriptFile>>,
    /// Transcripts read that held messages or should have.
    read: usize,
    /// Those of them in which no message could be read.
    unrecognized: usize,
}

/// The transcripts under `projects_dir` that hold a message, by session ID,
/// each read only up to that message. Fails only when `projects_dir` itself
/// cannot be read.
fn scan_transcripts(projects_dir: &Path, warnings: &mut Vec<String>) -> io::Result<Transcripts> {
    let mut by_id: HashMap<String, Vec<TranscriptFile>> = HashMap::new();
    let mut unreadable = Unreadable::new("transcript");
    let mut read = 0;
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
            let scan = match scan_transcript(&path, None, true) {
                Ok(scan) => scan,
                Err(err) => {
                    unreadable.add(&path, reason(&err));
                    continue;
                }
            };
            if scan.unrecognized() {
                read += 1;
                unreadable.add_unrecognized(&path, "no user or assistant message could be read");
                continue;
            }
            if scan.messages > 0 {
                read += 1;
                let modified = fs::metadata(&path).and_then(|meta| meta.modified()).ok();
                by_id
                    .entry(id)
                    .or_default()
                    .push(TranscriptFile { path, modified });
            }
        }
    }
    let unrecognized = unreadable.unrecognized;
    unreadable.report(warnings);
    Ok(Transcripts {
        by_id,
        read,
        unrecognized,
    })
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
    // Only regular files: a FIFO or a device would never end.
    entries
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().and_then(|e| e.to_str()) == Some("jsonl") && p.is_file())
}

/// The messages of the transcript at `path`.
pub fn read_jsonl(path: &Path) -> Result<Vec<Message>> {
    read_jsonl_with(path, ReadOptions::default())
}

/// The messages of the transcript at `path`, with its tool calls and results
/// among them when `read.tools` is set (see [`with_tools`]).
pub fn read_jsonl_with(path: &Path, read: ReadOptions) -> Result<Vec<Message>> {
    let mut messages = Vec::new();
    let keep = Keep {
        messages: &mut messages,
        tools: read.tools,
    };
    scan_transcript(path, Some(keep), false)?;
    Ok(messages)
}

/// Where [`scan_transcript`] keeps the messages it reads.
struct Keep<'a> {
    messages: &'a mut Vec<Message>,
    /// Whether tool calls and results are kept too.
    tools: bool,
}

/// What a transcript holds, as far as it was read.
#[derive(Debug, Default)]
struct Scan {
    messages: usize,
    /// The characters of their content, as reading them builds it.
    chars: usize,
    /// Lines that should hold a message: the user's, the assistant's, and
    /// those this version cannot read, such as lines of an unknown role.
    expected: usize,
}

impl Scan {
    /// Whether it holds lines but no message could be read from them, which
    /// lines of the roles that are never shown (system, tool) alone do not
    /// make it.
    fn unrecognized(&self) -> bool {
        self.messages == 0 && self.expected > 0
    }
}

/// Reads the transcript at `path` line by line, keeping no more than one line
/// in memory. Its messages go to `keep` when given; otherwise they are only
/// counted, with the characters of their content. With `until_first`,
/// reading stops at the first message.
fn scan_transcript(path: &Path, mut keep: Option<Keep<'_>>, until_first: bool) -> Result<Scan> {
    let io_err = |source| Error::Io {
        path: path.to_path_buf(),
        source,
    };
    let file = fs::File::open(path).map_err(io_err)?;
    let mut scan = Scan::default();
    for line in BufReader::new(file).split(b'\n') {
        let line = line.map_err(io_err)?;
        // An invalid byte costs its character, not the rest of the transcript.
        let line = String::from_utf8_lossy(&line);
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut built = None;
        match read_line(line, keep.is_some()) {
            Line::Nothing => {}
            Line::NoMessage => scan.expected += 1,
            Line::Message { message, chars } => {
                scan.expected += 1;
                scan.messages += 1;
                scan.chars += chars;
                built = message;
            }
        }
        match &mut keep {
            Some(keep) if keep.tools => keep.messages.extend(with_tools(line, built)),
            Some(keep) => keep.messages.extend(built),
            None => {}
        }
        if until_first && scan.messages > 0 {
            break;
        }
    }
    Ok(scan)
}

/// What one transcript line holds.
enum Line {
    /// Nothing that should be a message: a system or tool line, or one of
    /// only tool calls or images.
    Nothing,
    /// A line that should hold a message but holds none this version can
    /// show: unreadable, of an unknown role, or without text.
    NoMessage,
    /// A message, built when asked for, and the characters of the content it
    /// shows.
    Message {
        message: Option<Message>,
        chars: usize,
    },
}

fn read_line(line: &str, build: bool) -> Line {
    let entry = json::from_str::<TranscriptLine>(line).ok();
    let Some((role, message)) = entry.and_then(|entry| Some((entry.role?, entry.message))) else {
        return Line::NoMessage;
    };
    if matches!(role.as_str(), "system" | "tool") {
        return Line::Nothing;
    }
    let known = role == "user" || role == "assistant";
    let content = message.map(|m| m.content).unwrap_or_default();
    // A line of only tool calls or images has nothing to show either.
    if known && holds_only_hidden(&content) {
        return Line::Nothing;
    }
    if !known {
        return Line::NoMessage;
    }
    if !build {
        return match shown_chars(&role, &content) {
            0 => Line::NoMessage,
            chars => Line::Message {
                message: None,
                chars,
            },
        };
    }
    let raw_content = extract_content(&content);
    let timestamp = extract_timestamp_tag(&raw_content);
    let content = if role == "user" {
        clean_user_text(&raw_content)
    } else {
        raw_content.trim().to_string()
    };
    if content.is_empty() {
        return Line::NoMessage;
    }
    Line::Message {
        chars: content_chars(&content),
        message: Some(Message {
            role,
            content,
            timestamp,
        }),
    }
}

/// The characters of the content [`read_line`] builds for a message of
/// `role` with `content`, counted without building it where that is cheaper:
/// 0 when it has no text to show.
fn shown_chars(role: &str, content: &TranscriptContent) -> usize {
    if role == "user" {
        return content_chars(&clean_user_text(&extract_content(content)));
    }
    match content {
        TranscriptContent::Text(text) => content_chars(text.trim()),
        TranscriptContent::Parts(parts) => joined_trimmed_chars(text_parts(parts)),
        TranscriptContent::Other(_) => 0,
    }
}

/// The characters of `texts` joined with blank lines, as [`extract_content`]
/// joins them, and then trimmed, without joining them.
fn joined_trimmed_chars<'a>(texts: impl Iterator<Item = &'a str>) -> usize {
    // Characters up to the last one that is not whitespace, and the
    // whitespace after it, which counts only when more text follows.
    let (mut shown, mut trailing) = (0, 0);
    let mut started = false;
    for text in texts {
        let text = if started {
            // The blank line before it.
            trailing += 2;
            text
        } else {
            text.trim_start()
        };
        let visible = text.trim_end();
        if visible.is_empty() {
            trailing += content_chars(text);
            continue;
        }
        if started {
            shown += trailing;
        }
        shown += content_chars(visible);
        trailing = content_chars(&text[visible.len()..]);
        started = true;
    }
    shown
}

/// A transcript line as read for its tool calls and results.
#[derive(Debug, Deserialize)]
struct ToolLine {
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    message: Option<ToolLineMessage>,
}

#[derive(Debug, Deserialize)]
struct ToolLineMessage {
    #[serde(default)]
    content: Value,
}

/// The messages of a transcript line with its tool calls and results:
/// `message`, the one [`read_line`] built from it, and a tool message for
/// each `tool_use` and `tool_result` part, in the order of the parts. Its
/// text parts make one message, as without the tools, which goes where the
/// first of them with text is. A line of the role `tool` is all tool
/// messages.
fn with_tools(line: &str, message: Option<Message>) -> Vec<Message> {
    let Ok(entry) = json::from_str::<ToolLine>(line) else {
        return message.into_iter().collect();
    };
    let role = entry.role.unwrap_or_default();
    let content = entry.message.map(|m| m.content).unwrap_or_default();
    // System lines are never shown, tools or not.
    if role == "system" {
        return message.into_iter().collect();
    }
    let mut messages = Vec::new();
    if role == TOOL_ROLE {
        match &content {
            Value::Array(parts) => {
                for part in parts {
                    let text = match part_kind(part) {
                        "text" => part.get("text").and_then(tools::result),
                        _ => tool_part(part),
                    };
                    messages.extend(text.and_then(tools::message));
                }
            }
            other => messages.extend(tools::result(other).and_then(tools::message)),
        }
        messages.extend(message);
        return messages;
    }
    let mut message = message;
    if let Value::Array(parts) = &content {
        for part in parts {
            match part_kind(part) {
                "tool_use" | "tool_result" => {
                    messages.extend(tool_part(part).and_then(tools::message));
                }
                "text"
                    if part
                        .get("text")
                        .and_then(Value::as_str)
                        .is_some_and(|text| !text.trim().is_empty()) =>
                {
                    messages.extend(message.take());
                }
                _ => {}
            }
        }
    }
    messages.extend(message);
    messages
}

/// The `type` of a content part; a part without one is text.
fn part_kind(part: &Value) -> &str {
    match part.get("type") {
        Some(Value::String(kind)) => kind,
        Some(_) => "",
        None => "text",
    }
}

/// A `tool_use` part as a call, and any other part as what a tool returned.
fn tool_part(part: &Value) -> Option<String> {
    if part_kind(part) == "tool_use" {
        let name = part.get("name").and_then(Value::as_str);
        let args = ["input", "arguments", "args"]
            .iter()
            .find_map(|key| part.get(*key));
        return Some(tools::call(name, args));
    }
    ["content", "result", "output", "text"]
        .iter()
        .find_map(|key| part.get(*key).and_then(tools::result))
}

/// Content parts that are never shown.
const HIDDEN_PARTS: [&str; 3] = ["tool_use", "tool_result", "image"];

/// Whether `content` has parts and all of them are of a kind never shown.
fn holds_only_hidden(content: &TranscriptContent) -> bool {
    matches!(content, TranscriptContent::Parts(parts) if !parts.is_empty()
        && parts.iter().all(|part| part.kind.as_deref().is_some_and(|kind| HIDDEN_PARTS.contains(&kind))))
}

/// The text of the parts that are text, which a part without a type is.
fn text_parts(parts: &[TranscriptPart]) -> impl Iterator<Item = &str> {
    parts
        .iter()
        .filter(|part| part.kind.as_deref().unwrap_or("text") == "text")
        .filter_map(|part| part.text.as_deref())
}

fn extract_content(content: &TranscriptContent) -> String {
    match content {
        TranscriptContent::Text(text) => text.clone(),
        TranscriptContent::Parts(parts) => text_parts(parts).collect::<Vec<_>>().join("\n\n"),
        TranscriptContent::Other(_) => String::new(),
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
            messages: count,
            chars: 0,
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
                let mut best = None;
                for (count, modified, path) in order {
                    select_transcript(&mut best, candidate(count, modified, path));
                }
                let expected = candidate(higher.0, higher.1, higher.2);
                assert_eq!(best.unwrap().rank(), expected.rank());
            }
        }
    }

    #[test]
    fn identical_transcripts_do_not_duplicate_messages() {
        let mut best = None;
        for _ in 0..2 {
            select_transcript(&mut best, candidate(3, Some(10), "same.jsonl"));
        }
        assert_eq!(best.unwrap().messages, 3);
    }

    /// Counting a transcript gives the number of messages reading it gives,
    /// and the characters of their content, whatever its lines hold.
    #[test]
    fn counted_messages_are_the_messages_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.jsonl");
        let lines = [
            r#"{"role":"user","message":{"content":"<timestamp>Mon</timestamp>\n<user_query>\nhi\n</user_query>"}}"#,
            r#"{"role":"user","message":{"content":"<timestamp>Mon</timestamp>"}}"#,
            r#"{"role":"user","message":{"content":[{"type":"text","text":"<timestamp>"},{"type":"text","text":"x</timestamp>"}]}}"#,
            r#"{"role":"assistant","message":{"content":[{"type":"text","text":"  "},{"text":"\n"}]}}"#,
            r#"{"role":"assistant","message":{"content":[{"type":"text","text":" "},{"text":"done"}]}}"#,
            r#"{"role":"assistant","message":{"content":[{"type":"tool_use","name":"Read","input":{}}]}}"#,
            r#"{"role":"assistant","message":{"content":[{"type":"text","text":7}]}}"#,
            r#"{"role":"assistant","message":{"content":{"text":"moved"}}}"#,
            r#"{"role":"assistant","message":{"content":null}}"#,
            r#"{"role":"assistant","message":{"content":"   "}}"#,
            r#"{"role":"assistant","message":{"content":"\u00a0plain\ud83d"}}"#,
            r#"{"role":"assistant","message":{"content":[{"text":" \n"},{"text":"  ünï "},{"type":"tool_use","name":"Read"},{"text":"\t"},{"text":"日本\u3000"},{"text":" "}]}}"#,
            r#"{"role":"assistant","message":{"content":[{"text":"👨‍👩‍👧 one"},{"type":"image"}]}}"#,
            r#"{"role":"system","message":{"content":"setup"}}"#,
            r#"{"role":"human","message":{"content":"hello"}}"#,
            r#"["user",{"content":"by position"}]"#,
            "{not json",
        ];
        fs::write(&path, lines.join("\n")).unwrap();
        let read = read_jsonl(&path).unwrap();
        let counted = scan_transcript(&path, None, false).unwrap();
        assert_eq!(counted.messages, read.len());
        assert_eq!(read.len(), 6, "{read:?}");
        let chars: usize = read.iter().map(|m| content_chars(&m.content)).sum();
        assert_eq!(counted.chars, chars);
        assert_eq!(read[3].content, "ünï \n\n\t\n\n日本", "{read:?}");
        let first = scan_transcript(&path, None, true).unwrap();
        assert_eq!((first.messages, first.expected), (1, 1));
    }

    #[test]
    fn joined_text_is_counted_as_joining_and_trimming_counts_it() {
        let texts: [&[&str]; 9] = [
            &[],
            &[""],
            &["  ", "\n"],
            &["a"],
            &[" a ", " b "],
            &["", "a", "", "b", ""],
            &["\u{a0}x", " \u{3000}", "y\u{2028}"],
            &["  ", "é", "  "],
            &["日本語", "\t\n", "👨‍👩‍👧"],
        ];
        for parts in texts {
            let joined = parts.join("\n\n");
            assert_eq!(
                joined_trimmed_chars(parts.iter().copied()),
                joined.trim().chars().count(),
                "{parts:?}"
            );
        }
    }

    fn tool_messages(lines: &[&str]) -> Vec<(String, String)> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.jsonl");
        fs::write(&path, lines.join("\n")).unwrap();
        let with = read_jsonl_with(&path, ReadOptions { tools: true }).unwrap();
        // The other messages are those read without the tools.
        let without: Vec<String> = read_jsonl(&path)
            .unwrap()
            .into_iter()
            .map(|m| format!("{}: {}", m.role, m.content))
            .collect();
        let others: Vec<String> = with
            .iter()
            .filter(|m| !m.is_tool())
            .map(|m| format!("{}: {}", m.role, m.content))
            .collect();
        assert_eq!(others, without);
        with.into_iter().map(|m| (m.role, m.content)).collect()
    }

    #[test]
    fn tool_calls_and_results_are_read_in_order_when_asked_for() {
        let read = tool_messages(&[
            r#"{"role":"user","message":{"content":"<user_query>find it</user_query>"}}"#,
            r#"{"role":"assistant","message":{"content":[{"type":"text","text":"Looking."},{"type":"tool_use","name":"Grep","input":{"pattern":"langfuse","path":"src"}},{"type":"tool_use","name":"Read","input":{"path":"a.rs"}}]}}"#,
            r#"{"role":"tool","message":{"content":[{"type":"tool_result","tool_use_id":"1","content":"src/a.rs:3: langfuse"},{"type":"tool_result","content":[{"type":"text","text":"fn main() {}"}]}]}}"#,
            r#"{"role":"tool","message":{"content":"plain output"}}"#,
            r#"{"role":"assistant","message":{"content":[{"type":"tool_use","name":"Shell","input":{"command":"ls"}},{"type":"text","text":"Then "},{"type":"tool_use","name":"Read"},{"type":"text","text":"done."}]}}"#,
            r#"{"role":"user","message":{"content":[{"type":"tool_result","content":"ok"},{"type":"text","text":"thanks"}]}}"#,
            r#"{"role":"tool","message":{"content":[{"type":"tool_result","content":"  "},{"type":"text","text":"note"}]}}"#,
            r#"{"role":"system","message":{"content":[{"type":"tool_use","name":"Setup"}]}}"#,
            r#"{"role":"tool","message":{"content":{"odd":true}}}"#,
            "{not json",
        ]);
        let expected = [
            ("user", "find it"),
            ("assistant", "Looking."),
            ("tool", r#"Grep {"pattern":"langfuse","path":"src"}"#),
            ("tool", r#"Read {"path":"a.rs"}"#),
            ("tool", "src/a.rs:3: langfuse"),
            ("tool", "fn main() {}"),
            ("tool", "plain output"),
            ("tool", r#"Shell {"command":"ls"}"#),
            // The text of a line stays one message, where its text starts.
            ("assistant", "Then \n\ndone."),
            ("tool", "Read"),
            ("tool", "ok"),
            ("user", "thanks"),
            ("tool", "note"),
            ("tool", r#"{"odd":true}"#),
        ];
        let read: Vec<(&str, &str)> = read
            .iter()
            .map(|(role, content)| (role.as_str(), content.as_str()))
            .collect();
        assert_eq!(read, expected);
    }

    #[test]
    fn tool_lines_still_count_for_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.jsonl");
        let lines = [
            r#"{"role":"user","message":{"content":"hi"}}"#,
            r#"{"role":"tool","message":{"content":"ran"}}"#,
            r#"{"role":"assistant","message":{"content":[{"type":"tool_use","name":"Read","input":{}}]}}"#,
        ];
        fs::write(&path, lines.join("\n")).unwrap();
        let counted = scan_transcript(&path, None, false).unwrap();
        assert_eq!((counted.messages, counted.chars), (1, 2));
        let with = read_jsonl_with(&path, ReadOptions { tools: true }).unwrap();
        assert_eq!(with.len(), 3);
        let session =
            crate::model::Session::new(SessionSummary::new("t", "t", Source::Agent), with);
        assert_eq!((session.message_count, session.content_chars), (1, 2));
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
    ) -> (Vec<SessionSummary>, Vec<String>) {
        let paths = StoragePaths {
            chats_dir: Some(chats_dir.to_path_buf()),
            chats_scope,
            projects_dir: projects_dir.map(Path::to_path_buf),
            ..Default::default()
        };
        let (mut warnings, mut notices) = (Vec::new(), Vec::new());
        let mut sessions = load_sessions(&paths, &mut warnings, &mut notices).unwrap();
        sessions.sort_by(|a, b| a.id.cmp(&b.id));
        warnings.extend(notices);
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
    fn meta_json_fields_are_read_one_by_one() {
        let dir = tempfile::tempdir().unwrap();
        let chats = dir.path().join("chats");
        write(
            &chats.join("ws").join("s1").join("meta.json"),
            r#"{"title":"Kept","createdAtMs":"2025-09-04T15:33:20Z","updatedAtMs":1757000000000.5,"cwd":42}"#,
        );
        let (sessions, warnings) = load(&chats, ChatsScope::All, None);
        assert!(warnings.is_empty(), "{warnings:?}");
        let session = &sessions[0];
        assert_eq!(session.title, "Kept");
        assert_eq!(session.created_at_ms, Some(1_757_000_000_000));
        assert_eq!(session.updated_at_ms, Some(1_757_000_000_000));
        assert_eq!(session.workspace, None);
    }

    #[test]
    fn meta_json_in_a_format_this_version_cannot_read_is_a_notice() {
        let dir = tempfile::tempdir().unwrap();
        let chats = dir.path().join("chats");
        let meta = |id: &str| chats.join("ws").join(id).join("meta.json");
        for id in ["s1", "s2"] {
            write(
                &meta(id),
                r#"{"schemaVersion":2,"name":"Renamed","workingDirectory":"/w"}"#,
            );
        }
        // One that holds nothing to read is no sign either way.
        write(&meta("s3"), "{}");
        let paths = StoragePaths {
            chats_dir: Some(chats.clone()),
            ..Default::default()
        };
        let (mut warnings, mut notices) = (Vec::new(), Vec::new());
        let sessions = load_sessions(&paths, &mut warnings, &mut notices).unwrap();
        // The sessions still load, under their IDs.
        assert_eq!(sessions.len(), 3);
        assert!(sessions.iter().all(|s| s.title == s.id));
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(notices.len(), 1);
        let notice = &notices[0];
        assert!(
            notice.starts_with(&format!(
                "unrecognized meta.json format in {}: none of its 2 meta.json files could be \
                 read (",
                chats.display()
            )) && notice.ends_with(
                ": none of the keys title, createdAtMs, updatedAtMs, cwd, hasConversation \
                 found); the session titles, workspaces and times they hold are left out. \
                 Cursor may have changed its storage format."
            ),
            "{notice}"
        );

        // With one in a known format, the others are skipped files.
        write(&meta("s3"), r#"{"title":"Known"}"#);
        let (sessions, warnings) = load(&chats, ChatsScope::All, None);
        assert_eq!(sessions[2].title, "Known");
        assert_eq!(warnings.len(), 1);
        assert!(
            warnings[0].starts_with("ignored 2 unreadable meta.json files (first: "),
            "{}",
            warnings[0]
        );
    }

    fn line(key: &str, role: &str, text: &str) -> String {
        format!("{{\"{key}\":\"{role}\",\"message\":{{\"content\":\"{text}\"}}}}\n")
    }

    fn load_transcripts(projects: &Path) -> Result<(Vec<SessionSummary>, Vec<String>)> {
        let paths = StoragePaths {
            projects_dir: Some(projects.to_path_buf()),
            ..Default::default()
        };
        let (mut warnings, mut notices) = (Vec::new(), Vec::new());
        let sessions = load_sessions(&paths, &mut warnings, &mut notices)?;
        assert!(notices.is_empty(), "{notices:?}");
        Ok((sessions, warnings))
    }

    #[test]
    fn transcripts_in_a_format_this_version_cannot_read_are_a_clear_error() {
        let dir = tempfile::tempdir().unwrap();
        let projects = dir.path().join("projects");
        let path = |id: &str| {
            projects
                .join("p")
                .join("agent-transcripts")
                .join(id)
                .join(format!("{id}.jsonl"))
        };
        // `role` became `type`.
        for id in ["a", "b"] {
            let text = line("type", "user", "hello") + &line("type", "assistant", "hi");
            write(&path(id), &text);
        }
        // Only tool output or tool calls: nothing to show, but nothing unknown
        // either.
        write(&path("tools"), &line("role", "tool", "ran"));
        write(
            &path("calls"),
            "{\"role\":\"assistant\",\"message\":{\"content\":[{\"type\":\"tool_use\",\
             \"name\":\"Shell\",\"input\":{}}]}}\n",
        );
        let err = load_transcripts(&projects).unwrap_err();
        assert_eq!(
            err.to_string(),
            format!(
                "unrecognized Cursor Agent CLI storage format in {}: none of its 2 transcripts \
                 has a readable message. Cursor may have changed its storage format.",
                projects.display()
            )
        );
        assert_eq!(err.skippable(), Some(Source::Agent));

        // With a transcript that still reads, the others are skipped files.
        write(&path("c"), &line("role", "user", "hello"));
        let (sessions, warnings) = load_transcripts(&projects).unwrap();
        let ids: Vec<&str> = sessions.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["c"]);
        assert_eq!(warnings.len(), 1);
        assert!(
            warnings[0].starts_with("ignored 2 unreadable transcript files (first: ")
                && warnings[0].ends_with(": no user or assistant message could be read)"),
            "{}",
            warnings[0]
        );

        // A message of a known role whose text moved elsewhere counts too,
        // and so do roles this version does not know.
        for id in ["a", "b"] {
            fs::remove_file(path(id)).unwrap();
        }
        for text in [
            "{\"role\":\"user\",\"message\":{\"parts\":[\"hello\"]}}\n".to_string(),
            line("role", "human", "hello") + &line("role", "ai", "hi"),
        ] {
            fs::remove_file(path("c")).unwrap();
            write(&path("c"), &text);
            let err = load_transcripts(&projects).unwrap_err();
            assert!(
                err.to_string()
                    .contains(": its one transcript has no readable message."),
                "{err}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn only_regular_files_are_read_as_transcripts() {
        let dir = tempfile::tempdir().unwrap();
        let projects = dir.path().join("projects");
        let session = projects.join("p").join("agent-transcripts").join("endless");
        fs::create_dir_all(&session).unwrap();
        // Read to its end, this would never finish.
        std::os::unix::fs::symlink("/dev/zero", session.join("a.jsonl")).unwrap();
        let (sessions, warnings) = load_transcripts(&projects).unwrap();
        assert!(sessions.is_empty() && warnings.is_empty());
    }

    #[test]
    fn store_dbs_in_a_format_this_version_cannot_read_are_a_notice() {
        let dir = tempfile::tempdir().unwrap();
        let chats = dir.path().join("chats");
        for id in ["s1", "s2"] {
            let session = chats.join("ws").join(id);
            fs::create_dir_all(&session).unwrap();
            let conn = rusqlite::Connection::open(session.join("store.db")).unwrap();
            conn.execute_batch("CREATE TABLE meta2 (key TEXT PRIMARY KEY, value BLOB);")
                .unwrap();
        }
        let paths = StoragePaths {
            chats_dir: Some(chats.clone()),
            ..Default::default()
        };
        let (mut warnings, mut notices) = (Vec::new(), Vec::new());
        let sessions = load_sessions(&paths, &mut warnings, &mut notices).unwrap();
        // The sessions still load, under their IDs.
        assert_eq!(sessions.len(), 2);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(notices.len(), 1);
        let notice = &notices[0];
        assert!(
            notice.starts_with(&format!(
                "unrecognized store.db format in {}: none of its 2 store.db files could be read (",
                chats.display()
            )) && notice.contains(
                ": no such table: meta); the session names and models they hold are left out."
            ),
            "{notice}"
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
            assert!(sessions.iter().all(|s| s.message_count == 1));
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
        let (mut warnings, mut notices) = (Vec::new(), Vec::new());
        let partial = load_sessions(&paths(&chats), &mut warnings, &mut notices);
        lock(&projects, 0o000);
        let failed = load_sessions(&paths(&chats), &mut Vec::new(), &mut Vec::new());
        // A location that is gone holds no sessions; the other one still fails.
        let gone = load_sessions(
            &paths(&dir.path().join("gone")),
            &mut Vec::new(),
            &mut Vec::new(),
        );
        lock(&chats, 0o755);
        lock(&projects, 0o755);

        let partial = partial.unwrap();
        assert_eq!(partial.len(), 1);
        assert_eq!(partial[0].id, "s2");
        // Not a skipped file: every session there is missing, so it is a notice.
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(notices.len(), 1);
        assert!(notices[0].starts_with(&format!("could not read {}: ", chats.display())));
        for (result, failing) in [(failed, &chats), (gone, &projects)] {
            assert!(
                matches!(&result, Err(Error::AgentAccess { path, .. }) if path == failing),
                "{result:?}"
            );
        }

        let gone = dir.path().join("gone");
        let empty = StoragePaths {
            chats_dir: Some(gone.clone()),
            projects_dir: Some(gone),
            ..Default::default()
        };
        assert!(
            load_sessions(&empty, &mut Vec::new(), &mut Vec::new())
                .unwrap()
                .is_empty()
        );
    }
}
