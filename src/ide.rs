use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::Path;

use rusqlite::Connection;
use rusqlite::types::ValueRef;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::detect::StoragePaths;
use crate::model::{Message, Session, Source};
use crate::sqlite::with_readonly;
use crate::{Error, Result};

const KV_TABLE: &str = "cursorDiskKV";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Composer {
    #[serde(default)]
    composer_id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    created_at: Option<i64>,
    #[serde(default)]
    last_updated_at: Option<i64>,
    #[serde(default)]
    full_conversation_headers_only: Vec<ConversationHeader>,
    /// Older Cursor versions keep the messages in the composer itself.
    #[serde(default)]
    conversation: Vec<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConversationHeader {
    bubble_id: Option<String>,
    #[serde(rename = "type")]
    kind: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Bubble {
    #[serde(default)]
    bubble_id: Option<String>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    rich_text: Option<String>,
    #[serde(default)]
    timestamp: Option<i64>,
    #[serde(rename = "type")]
    kind: Option<i64>,
    #[serde(default)]
    code_blocks: Vec<CodeBlock>,
}

#[derive(Debug, Deserialize, Default)]
struct CodeBlock {
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    content: Option<String>,
}

pub fn load_sessions(paths: &StoragePaths, warnings: &mut Vec<String>) -> Result<Vec<Session>> {
    let Some(db_path) = &paths.global_storage_db else {
        return Ok(Vec::new());
    };
    load_from_db(db_path, warnings)
}

/// Loads composer sessions from a `state.vscdb`. A missing file holds no
/// sessions; rows that cannot be decoded are skipped and reported in
/// `warnings`.
pub fn load_from_db(db_path: &Path, warnings: &mut Vec<String>) -> Result<Vec<Session>> {
    match fs::metadata(db_path) {
        Ok(meta) if meta.len() == 0 => {
            warnings.push(format!(
                "{} is empty; no Cursor IDE sessions loaded",
                db_path.display()
            ));
            return Ok(Vec::new());
        }
        Ok(_) => {}
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => return Err(Error::access(db_path, source)),
    }
    // The read may run twice (see `with_readonly`); only the last one's
    // warnings are kept.
    let (sessions, found) = with_readonly(db_path, |conn| {
        let mut found = Vec::new();
        let sessions = read_sessions(conn, db_path, &mut found)?;
        Ok((sessions, found))
    })?;
    warnings.extend(found);
    Ok(sessions)
}

fn read_sessions(
    conn: &Connection,
    db_path: &Path,
    warnings: &mut Vec<String>,
) -> Result<Vec<Session>> {
    // One read transaction, so that a chat and its messages come from the same
    // commit while Cursor writes. Dropping it ends the read.
    let _snapshot = conn
        .unchecked_transaction()
        .map_err(|source| Error::Database {
            path: db_path.to_path_buf(),
            source,
        })?;
    if !check_schema(conn, db_path)? {
        warnings.push(format!(
            "{} has no tables; no Cursor IDE sessions loaded",
            db_path.display()
        ));
        return Ok(Vec::new());
    }

    // `bubbleId:<chat id>:<message id>` rows, by chat and then message ID.
    let mut bubbles: HashMap<String, HashMap<String, Bubble>> = HashMap::new();
    let skipped = read_rows(conn, db_path, "bubbleId:", |key, value| {
        let Some(bubble) = parse_object::<Bubble>(value) else {
            return false;
        };
        let rest = key.strip_prefix("bubbleId:").unwrap_or(key);
        let (chat, bubble_key) = rest.split_once(':').unwrap_or_default();
        let id = bubble
            .bubble_id
            .clone()
            .unwrap_or_else(|| bubble_key.to_string());
        bubbles
            .entry(chat.to_string())
            .or_default()
            .insert(id, bubble);
        true
    })?;
    warn_skipped(warnings, skipped, "message", db_path);

    let mut sessions = Vec::new();
    // Chats with stored messages, and those of them that lead to none.
    let (mut stored, mut unlinked) = (0, 0);
    let skipped = read_rows(conn, db_path, "composerData:", |key, value| {
        let Some(composer) = parse_object::<Composer>(value) else {
            return false;
        };
        // A blank ID could not be looked up with `show`.
        let named = |id: &str| !id.trim().is_empty();
        let id = composer
            .composer_id
            .clone()
            .filter(|id| named(id))
            .or_else(|| {
                key.strip_prefix("composerData:")
                    .filter(|id| named(id))
                    .map(str::to_string)
            });
        let Some(id) = id else {
            return false;
        };
        let chat_bubbles = bubbles.get(&id);
        let (session, linked) = composer_session(id, composer, chat_bubbles);
        if chat_bubbles.is_some() {
            stored += 1;
            unlinked += usize::from(!linked);
        }
        sessions.push(session);
        true
    })?;

    // Rows that are all unreadable, or chats that all lead to none of their
    // messages, mean the format changed rather than that there is nothing.
    let mismatch = |detail| Error::SchemaMismatch {
        path: db_path.to_path_buf(),
        detail,
    };
    if sessions.is_empty() && skipped > 0 {
        return Err(mismatch(match skipped {
            1 => "its composerData row could not be read".to_string(),
            _ => format!("none of its {skipped} composerData rows could be read"),
        }));
    }
    if unlinked > 0 && unlinked == stored {
        return Err(mismatch(
            "no chat lists the messages stored for it".to_string(),
        ));
    }
    warn_skipped(warnings, skipped, "composer", db_path);
    if unlinked > 0 {
        warnings.push(format!(
            "{unlinked} chat(s) in {} list none of their stored messages",
            db_path.display()
        ));
    }
    Ok(sessions)
}

/// Confirms `cursorDiskKV(key, value)` exists. `Ok(false)` means the database
/// has no tables at all.
fn check_schema(conn: &Connection, db_path: &Path) -> Result<bool> {
    let db_err = |source| Error::Database {
        path: db_path.to_path_buf(),
        source,
    };
    let tables = column_strings(
        conn,
        "SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name",
    )
    .map_err(db_err)?;
    if tables.is_empty() {
        return Ok(false);
    }
    let mismatch = |detail| Error::SchemaMismatch {
        path: db_path.to_path_buf(),
        detail,
    };
    if !tables.iter().any(|t| t.eq_ignore_ascii_case(KV_TABLE)) {
        return Err(mismatch(format!(
            "table `{KV_TABLE}` not found (tables present: {})",
            tables.join(", ")
        )));
    }
    let columns = column_strings(
        conn,
        &format!("SELECT name FROM pragma_table_info('{KV_TABLE}')"),
    )
    .map_err(db_err)?;
    for wanted in ["key", "value"] {
        if !columns.iter().any(|c| c.eq_ignore_ascii_case(wanted)) {
            return Err(mismatch(format!(
                "table `{KV_TABLE}` has no `{wanted}` column (columns: {})",
                columns.join(", ")
            )));
        }
    }
    Ok(true)
}

fn column_strings(conn: &Connection, sql: &str) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
    rows.collect()
}

/// Calls `visit` with the key and value of every non-NULL row whose key starts
/// with `prefix`. Values may be stored as TEXT or BLOB. Returns how many rows
/// could not be decoded, either here or by `visit` returning `false`.
fn read_rows(
    conn: &Connection,
    db_path: &Path,
    prefix: &str,
    mut visit: impl FnMut(&str, &str) -> bool,
) -> Result<usize> {
    let db_err = |source| Error::Database {
        path: db_path.to_path_buf(),
        source,
    };
    let mut stmt = conn
        .prepare(&format!(
            "SELECT key, value FROM {KV_TABLE} WHERE key LIKE ?1 AND value IS NOT NULL"
        ))
        .map_err(db_err)?;
    let mut rows = stmt.query([format!("{prefix}%")]).map_err(db_err)?;
    let mut skipped = 0;
    while let Some(row) = rows.next().map_err(db_err)? {
        let decoded = match (as_text(row.get_ref(0)), as_text(row.get_ref(1))) {
            (Some(key), Some(value)) => visit(key, value),
            _ => false,
        };
        if !decoded {
            skipped += 1;
        }
    }
    Ok(skipped)
}

fn as_text(value: rusqlite::Result<ValueRef<'_>>) -> Option<&str> {
    match value.ok()? {
        ValueRef::Text(bytes) | ValueRef::Blob(bytes) => std::str::from_utf8(bytes).ok(),
        _ => None,
    }
}

/// Parses a JSON object. serde would also fill a struct from a JSON array, by
/// field position.
fn parse_object<T: DeserializeOwned>(json: &str) -> Option<T> {
    if !json.trim_start().starts_with('{') {
        return None;
    }
    serde_json::from_str(json).ok()
}

fn warn_skipped(warnings: &mut Vec<String>, skipped: usize, kind: &str, db_path: &Path) {
    if skipped > 0 {
        let rows = if skipped == 1 { "row" } else { "rows" };
        warnings.push(format!(
            "skipped {skipped} unreadable {kind} {rows} in {}",
            db_path.display()
        ));
    }
}

/// The session for `composer`, and whether its conversation led to any
/// message: one of `bubbles`, its stored messages, or one kept inline, as
/// older Cursor versions did for composers without conversation headers.
fn composer_session(
    id: String,
    composer: Composer,
    bubbles: Option<&HashMap<String, Bubble>>,
) -> (Session, bool) {
    let mut messages = Vec::new();
    let mut linked = false;
    for header in &composer.full_conversation_headers_only {
        let bubble = header
            .bubble_id
            .as_ref()
            .and_then(|bubble_id| bubbles?.get(bubble_id));
        if let Some(bubble) = bubble {
            linked = true;
            messages.extend(message(header.kind.or(bubble.kind), bubble));
        }
    }
    if composer.full_conversation_headers_only.is_empty() {
        for value in &composer.conversation {
            if let Ok(bubble) = Bubble::deserialize(value) {
                linked = true;
                messages.extend(message(bubble.kind, &bubble));
            }
        }
    }
    let title = composer
        .name
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Untitled".to_string());
    let session = Session {
        id,
        title,
        source: Source::Ide,
        workspace: None,
        workspace_hash: None,
        created_at_ms: composer.created_at,
        updated_at_ms: composer.last_updated_at,
        model: None,
        messages,
    };
    (session, linked)
}

/// A message from `bubble`, unless it has no text or code.
fn message(kind: Option<i64>, bubble: &Bubble) -> Option<Message> {
    let content = extract_bubble_text(bubble);
    if content.is_empty() {
        return None;
    }
    let role = match kind.unwrap_or(1) {
        1 => "user",
        _ => "assistant",
    };
    Some(Message {
        role: role.to_string(),
        content,
        timestamp: bubble.timestamp.map(|ms| ms.to_string()),
    })
}

fn extract_bubble_text(bubble: &Bubble) -> String {
    let mut parts = Vec::new();
    if let Some(text) = bubble.text.as_deref() {
        let text = text.trim();
        if !text.is_empty() {
            parts.push(text.to_string());
        }
    }
    if parts.is_empty()
        && let Some(rich) = bubble.rich_text.as_deref()
        && !rich.is_empty()
        && let Ok(value) = serde_json::from_str::<Value>(rich)
    {
        let extracted = collect_json_text(&value);
        if !extracted.is_empty() {
            parts.push(extracted);
        }
    }
    for block in &bubble.code_blocks {
        if let Some(content) = block.content.as_deref() {
            if content.is_empty() {
                continue;
            }
            let lang = block.language.as_deref().unwrap_or("");
            parts.push(format!("```{lang}\n{content}\n```"));
        }
    }
    parts.join("\n\n")
}

fn collect_json_text(value: &Value) -> String {
    let mut out = Vec::new();
    walk_text(value, &mut out);
    out.join("")
}

fn walk_text(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            if let Some(Value::String(text)) = map.get("text")
                && !text.is_empty()
            {
                out.push(text.clone());
            }
            for (key, nested) in map {
                if key != "text" {
                    walk_text(nested, out);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                walk_text(item, out);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use rusqlite::types::Value as SqlValue;

    use super::*;

    const COMPOSER: &str = r#"{"composerId":"c1","name":"Plan","createdAt":1700000000000,"lastUpdatedAt":1700000500000,"fullConversationHeadersOnly":[{"bubbleId":"b1","type":1},{"bubbleId":"b2","type":2}]}"#;
    const BUBBLE_1: &str = r#"{"bubbleId":"b1","type":1,"text":"question"}"#;
    const BUBBLE_2: &str = r#"{"bubbleId":"b2","type":2,"text":"answer"}"#;

    fn create_db(path: &Path, rows: &[(&str, SqlValue)]) {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            "CREATE TABLE ItemTable (key TEXT UNIQUE ON CONFLICT REPLACE, value BLOB);
             CREATE TABLE cursorDiskKV (key TEXT UNIQUE ON CONFLICT REPLACE, value BLOB);",
        )
        .unwrap();
        for (key, value) in rows {
            conn.execute(
                "INSERT INTO cursorDiskKV (key, value) VALUES (?1, ?2)",
                rusqlite::params![key, value],
            )
            .unwrap();
        }
    }

    fn well_formed(as_blob: bool) -> Vec<(&'static str, SqlValue)> {
        let value = |json: &str| {
            if as_blob {
                SqlValue::Blob(json.as_bytes().to_vec())
            } else {
                SqlValue::Text(json.to_string())
            }
        };
        vec![
            ("composerData:c1", value(COMPOSER)),
            ("bubbleId:c1:b1", value(BUBBLE_1)),
            ("bubbleId:c1:b2", value(BUBBLE_2)),
        ]
    }

    fn load(path: &Path) -> (Result<Vec<Session>>, Vec<String>) {
        let mut warnings = Vec::new();
        let sessions = load_from_db(path, &mut warnings);
        (sessions, warnings)
    }

    #[test]
    fn text_and_blob_values_load_the_same_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let mut loaded = Vec::new();
        for as_blob in [false, true] {
            let path = dir.path().join(format!("{as_blob}.vscdb"));
            create_db(&path, &well_formed(as_blob));
            let (sessions, warnings) = load(&path);
            assert_eq!(warnings, Vec::<String>::new());
            loaded.push(serde_json::to_string(&sessions.unwrap()).unwrap());
        }
        assert_eq!(loaded[0], loaded[1]);
        let sessions: Vec<Session> = serde_json::from_str(&loaded[0]).unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].title, "Plan");
        let contents: Vec<_> = sessions[0].messages.iter().map(|m| &m.content).collect();
        assert_eq!(contents, ["question", "answer"]);
    }

    #[test]
    fn malformed_rows_are_skipped_and_counted() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.vscdb");
        let mut rows = well_formed(false);
        rows.extend([
            ("composerData:bad-json", SqlValue::Text("{not json".into())),
            (
                "composerData:bad-utf8",
                SqlValue::Blob(vec![b'{', 0xff, b'}']),
            ),
            ("composerData:number", SqlValue::Integer(7)),
            ("composerData:null", SqlValue::Null),
            (
                "bubbleId:c1:bad",
                SqlValue::Text(r#"{"bubbleId":5}"#.into()),
            ),
        ]);
        create_db(&path, &rows);

        let (sessions, warnings) = load(&path);
        let sessions = sessions.unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].messages.len(), 2);
        assert_eq!(
            warnings,
            [
                format!("skipped 1 unreadable message row in {}", path.display()),
                format!("skipped 3 unreadable composer rows in {}", path.display()),
            ]
        );
    }

    #[test]
    fn missing_kv_table_is_a_schema_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.vscdb");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch("CREATE TABLE ItemTable (key TEXT, value BLOB);")
            .unwrap();
        drop(conn);

        let err = load(&path).0.unwrap_err();
        assert!(matches!(err, Error::SchemaMismatch { .. }));
        assert_eq!(
            err.to_string(),
            format!(
                "unrecognized Cursor IDE storage format in {}: table `cursorDiskKV` not found \
                 (tables present: ItemTable). Cursor may have changed its storage format.",
                path.display()
            )
        );
        assert!(err.hints()[0].contains("--source agent"));

        let conn = Connection::open(&path).unwrap();
        conn.execute_batch("CREATE TABLE cursorDiskKV (k TEXT, value BLOB);")
            .unwrap();
        drop(conn);
        let err = load(&path).0.unwrap_err();
        assert!(
            err.to_string()
                .contains("table `cursorDiskKV` has no `key` column (columns: k, value)"),
            "{err}"
        );
    }

    #[test]
    fn missing_and_empty_files_hold_no_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.vscdb");
        let (sessions, warnings) = load(&path);
        assert!(sessions.unwrap().is_empty());
        assert!(warnings.is_empty());

        fs::write(&path, b"").unwrap();
        let (sessions, warnings) = load(&path);
        assert!(sessions.unwrap().is_empty());
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("is empty"));
    }

    #[test]
    fn non_sqlite_file_is_a_database_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.vscdb");
        fs::write(
            &path,
            "plain text that is long enough to fill a sqlite header",
        )
        .unwrap();
        let err = load(&path).0.unwrap_err();
        assert!(matches!(&err, Error::Database { path: p, .. } if *p == path));
        let source = std::error::Error::source(&err).unwrap().to_string();
        assert!(source.contains("not a database"), "{source}");
    }
}
