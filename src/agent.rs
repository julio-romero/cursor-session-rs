use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::Deserialize;
use serde_json::Value;

use crate::detect::StoragePaths;
use crate::model::{Message, Session, Source};
use crate::sqlite::open_readonly;
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

pub fn load_sessions(paths: &StoragePaths, _warnings: &mut Vec<String>) -> Result<Vec<Session>> {
    let mut by_id: HashMap<String, Session> = HashMap::new();

    if let Some(chats_dir) = &paths.chats_dir {
        for session in scan_chats(chats_dir)? {
            by_id.insert(session.id.clone(), session);
        }
    }

    if let Some(projects_dir) = &paths.projects_dir {
        for (id, messages) in scan_transcripts(projects_dir)? {
            if let Some(session) = by_id.get_mut(&id) {
                if session.messages.is_empty() {
                    session.messages = messages;
                }
            } else {
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
    }

    Ok(by_id.into_values().collect())
}

fn scan_chats(chats_dir: &Path) -> Result<Vec<Session>> {
    let mut sessions = Vec::new();
    let Ok(workspace_dirs) = fs::read_dir(chats_dir) else {
        return Ok(sessions);
    };

    for workspace in workspace_dirs.flatten() {
        let workspace_path = workspace.path();
        if !workspace_path.is_dir() {
            continue;
        }
        let workspace_hash = workspace
            .file_name()
            .to_str()
            .unwrap_or_default()
            .to_string();
        let Ok(session_dirs) = fs::read_dir(&workspace_path) else {
            continue;
        };
        for session_dir in session_dirs.flatten() {
            let path = session_dir.path();
            if !path.is_dir() {
                continue;
            }
            if let Some(session) = load_chat_session(&path, &workspace_hash) {
                sessions.push(session);
            }
        }
    }
    Ok(sessions)
}

fn load_chat_session(session_dir: &Path, workspace_hash: &str) -> Option<Session> {
    let id = session_dir.file_name()?.to_str()?.to_string();
    let meta_path = session_dir.join("meta.json");
    let meta: MetaJson = if meta_path.is_file() {
        let raw = fs::read_to_string(&meta_path).ok()?;
        serde_json::from_str(&raw).unwrap_or_default()
    } else if session_dir.join("store.db").is_file() {
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

    if let Some(store_meta) = read_store_meta(&session_dir.join("store.db")) {
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

    if session.title.is_empty() {
        session.title = session.id.clone();
    }

    if meta.has_conversation == Some(false)
        && !session_dir.join("store.db").is_file()
        && session.messages.is_empty()
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

fn read_store_meta(path: &Path) -> Option<StoreMeta> {
    if !path.is_file() {
        return None;
    }
    let conn = open_readonly(path).ok()?;
    let value: String = conn
        .query_row("SELECT value FROM meta WHERE key = '0'", [], |row| {
            row.get(0)
        })
        .ok()?;
    let json = decode_meta_json(&value)?;
    Some(StoreMeta {
        name: json.get("name").and_then(Value::as_str).map(str::to_string),
        model: json
            .get("lastUsedModel")
            .and_then(Value::as_str)
            .map(str::to_string),
        created_at: json.get("createdAt").and_then(Value::as_i64),
    })
}

fn decode_meta_json(raw: &str) -> Option<Value> {
    if let Ok(value) = serde_json::from_str::<Value>(raw) {
        return Some(value);
    }
    let bytes = decode_hex(raw)?;
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

fn scan_transcripts(projects_dir: &Path) -> Result<HashMap<String, Vec<Message>>> {
    let mut candidates = HashMap::new();
    let Ok(projects) = fs::read_dir(projects_dir) else {
        return Ok(HashMap::new());
    };
    for project in projects.flatten() {
        let transcripts = project.path().join("agent-transcripts");
        if !transcripts.is_dir() {
            continue;
        }
        let Ok(sessions) = fs::read_dir(&transcripts) else {
            continue;
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
            let jsonl = transcript_path(&dir, &id);
            if let Some(path) = jsonl
                && let Ok(messages) = read_jsonl(&path)
                && !messages.is_empty()
            {
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
    Ok(candidates
        .into_iter()
        .map(|(id, candidate)| (id, candidate.messages))
        .collect())
}

fn transcript_path(dir: &Path, id: &str) -> Option<PathBuf> {
    let named = dir.join(format!("{id}.jsonl"));
    if named.is_file() {
        return Some(named);
    }
    if dir.is_file() && dir.extension().and_then(|e| e.to_str()) == Some("jsonl") {
        return Some(dir.to_path_buf());
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return None;
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
    for line in reader.lines() {
        let line = line.map_err(io_err)?;
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
}
