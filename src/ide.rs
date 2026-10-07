use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::Path;

use rusqlite::types::ValueRef;
use rusqlite::{Connection, OptionalExtension};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer};
use serde_json::Value;

use crate::detect::StoragePaths;
use crate::json::{self, lenient, lenient_ms};
use crate::model::{Message, Session, Source};
use crate::sqlite::with_readonly;
use crate::{Error, Result};

const KV_TABLE: &str = "cursorDiskKV";

// Every field is optional and read leniently (see `lenient`), so that one
// value of an unexpected type costs that value, not its chat or message.

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Composer {
    #[serde(default, deserialize_with = "lenient")]
    composer_id: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    name: Option<String>,
    #[serde(default, deserialize_with = "lenient_ms")]
    created_at: Option<i64>,
    #[serde(default, deserialize_with = "lenient_ms")]
    last_updated_at: Option<i64>,
    #[serde(default, deserialize_with = "lenient")]
    full_conversation_headers_only: Vec<ConversationHeader>,
    /// Older Cursor versions keep the messages in the composer itself.
    #[serde(default, deserialize_with = "lenient")]
    conversation: Vec<Value>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConversationHeader {
    #[serde(default, deserialize_with = "lenient")]
    bubble_id: Option<String>,
    #[serde(rename = "type", default, deserialize_with = "kind")]
    kind: Option<Kind>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Bubble {
    #[serde(default, deserialize_with = "lenient")]
    bubble_id: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    text: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    rich_text: Option<String>,
    #[serde(default, deserialize_with = "timestamp")]
    timestamp: Option<String>,
    #[serde(rename = "type", default, deserialize_with = "kind")]
    kind: Option<Kind>,
    #[serde(default, deserialize_with = "lenient")]
    code_blocks: Vec<CodeBlock>,
    /// A tool call, which is never shown.
    #[serde(default)]
    tool_former_data: Option<Value>,
    /// Images, which are never shown.
    #[serde(default, deserialize_with = "lenient")]
    images: Vec<Value>,
}

impl Bubble {
    /// Whether it holds something that is never shown, such as a tool call or
    /// an image, so that having no text to show is no sign of a new format.
    fn has_hidden_content(&self) -> bool {
        self.tool_former_data.is_some() || !self.images.is_empty() || !self.code_blocks.is_empty()
    }
}

/// Who wrote a message, from its `type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// 1, or `"user"`.
    User,
    /// 2, or `"assistant"`.
    Assistant,
    /// Any other number.
    Other,
}

#[derive(Debug, Deserialize, Default)]
struct CodeBlock {
    #[serde(default, deserialize_with = "lenient")]
    language: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    content: Option<String>,
}

/// A message `type`: `None` when it is neither a number nor a known name.
fn kind<'de, D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Option<Kind>, D::Error> {
    Ok(match Value::deserialize(deserializer)? {
        Value::Number(n) => Some(match n.as_i64() {
            Some(1) => Kind::User,
            Some(2) => Kind::Assistant,
            _ => Kind::Other,
        }),
        Value::String(name) if name == "user" => Some(Kind::User),
        Value::String(name) if name == "assistant" => Some(Kind::Assistant),
        _ => None,
    })
}

/// A message time as stored: a number as its digits, or a string as it is.
fn timestamp<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<String>, D::Error> {
    Ok(match Value::deserialize(deserializer)? {
        Value::Number(n) => Some(n.to_string()),
        Value::String(text) if !text.is_empty() => Some(text),
        _ => None,
    })
}

pub fn load_sessions(
    paths: &StoragePaths,
    warnings: &mut Vec<String>,
    notices: &mut Vec<String>,
) -> Result<Vec<Session>> {
    let Some(db_path) = &paths.global_storage_db else {
        return Ok(Vec::new());
    };
    load_from_db(db_path, warnings, notices)
}

/// Loads composer sessions from a `state.vscdb`. A missing file holds no
/// sessions; rows that cannot be decoded are skipped and reported in
/// `warnings`. Chats and messages that load in part, as when Cursor changes
/// their format for new chats only, are reported in `notices`.
pub fn load_from_db(
    db_path: &Path,
    warnings: &mut Vec<String>,
    notices: &mut Vec<String>,
) -> Result<Vec<Session>> {
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
    // warnings and notices are kept.
    let (sessions, found, noticed) = with_readonly(db_path, |conn| {
        let (mut found, mut noticed) = (Vec::new(), Vec::new());
        let sessions = read_sessions(conn, db_path, &mut found, &mut noticed)?;
        Ok((sessions, found, noticed))
    })?;
    warnings.extend(found);
    notices.extend(noticed);
    Ok(sessions)
}

fn read_sessions(
    conn: &Connection,
    db_path: &Path,
    warnings: &mut Vec<String>,
    notices: &mut Vec<String>,
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
    let mut read = 0;
    let skipped_bubbles = read_rows(conn, db_path, "bubbleId:", |key, value| {
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
        read += 1;
        true
    })?;
    #[cfg(test)]
    AFTER_BUBBLES.with_borrow_mut(|hook| hook.as_mut().map(|hook| hook()));

    let mut sessions = Vec::new();
    // Chats with stored messages, and those of them that lead to none.
    let (mut stored, mut unlinked) = (0, 0);
    // Chats that list messages and should have one to show, and those of
    // them that do.
    let (mut listing, mut shown) = (0, 0);
    // Messages shown without a known type.
    let mut untyped = 0;
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
        let lists = !composer.full_conversation_headers_only.is_empty()
            || !composer.conversation.is_empty();
        // Each chat takes its messages, so their text is not held twice.
        let chat_bubbles = bubbles.remove(&id);
        let has_stored = chat_bubbles.is_some();
        let chat = composer_session(id, composer, chat_bubbles, &mut untyped);
        if has_stored {
            stored += 1;
            unlinked += usize::from(!chat.linked);
        }
        if lists && !chat.only_hidden {
            listing += 1;
            shown += usize::from(!chat.session.messages.is_empty());
        }
        sessions.push(chat.session);
        true
    })?;
    // What is left are messages whose chat has no row that was read.
    let orphans = find_orphans(conn, db_path, bubbles)?;

    // Rows that are all unreadable, messages without a chat or a type, or
    // chats that all lead to none of their messages, mean the format changed
    // rather than that there is nothing.
    let mismatch = |detail| Error::SchemaMismatch {
        store: Source::Ide,
        path: db_path.to_path_buf(),
        detail,
    };
    if sessions.is_empty() && skipped > 0 {
        return Err(mismatch(match skipped {
            1 => "its composerData row could not be read".to_string(),
            _ => format!("none of its {skipped} composerData rows could be read"),
        }));
    }
    if sessions.is_empty()
        && let Some(prefix) = &orphans.moved_to
    {
        return Err(mismatch(match read {
            1 => format!(
                "its bubbleId row belongs to no composerData row, and a `{prefix}:` row names \
                 its chat"
            ),
            _ => format!(
                "none of its {read} bubbleId rows belongs to a composerData row, and \
                 `{prefix}:` rows name their chats"
            ),
        }));
    }
    if read == 0 && skipped_bubbles > 0 {
        return Err(mismatch(match skipped_bubbles {
            1 => "its bubbleId row could not be read".to_string(),
            _ => format!("none of its {skipped_bubbles} bubbleId rows could be read"),
        }));
    }
    if unlinked > 0 && unlinked == stored {
        return Err(mismatch(
            "no chat lists the messages stored for it".to_string(),
        ));
    }
    if listing > 0 && shown == 0 {
        return Err(mismatch(match listing {
            1 => "its one chat that lists messages has no readable message".to_string(),
            _ => format!("none of its {listing} chats that list messages has a readable message"),
        }));
    }
    let messages: usize = sessions.iter().map(|session| session.messages.len()).sum();
    if untyped > 0 && untyped == messages {
        return Err(mismatch(match messages {
            1 => "its one message has no known type".to_string(),
            _ => format!("none of its {messages} messages has a known type"),
        }));
    }
    // Not one chat or message row: either there are no chats, or their rows
    // are all under keys this version does not know.
    if sessions.is_empty() && read + skipped + skipped_bubbles == 0 {
        let mut others = key_prefixes(conn).map_err(|source| Error::Database {
            path: db_path.to_path_buf(),
            source,
        })?;
        others.retain(|prefix| prefix != "composerData" && prefix != "bubbleId");
        // Rows named like chats or messages are most likely those, moved.
        let (alike, unlike): (Vec<String>, Vec<String>) = others.into_iter().partition(|prefix| {
            let prefix = prefix.to_ascii_lowercase();
            prefix.contains("composer") || prefix.contains("bubble")
        });
        let named = |prefixes: &[String]| {
            prefixes
                .iter()
                .map(|prefix| format!("`{prefix}:`"))
                .collect::<Vec<_>>()
                .join(", ")
        };
        if !alike.is_empty() {
            notices.push(format!(
                "{} has no composerData or bubbleId rows, but rows under {}, which are left \
                 out. Cursor may have changed its storage format.",
                db_path.display(),
                named(&alike)
            ));
        } else if !unlike.is_empty() {
            warnings.push(format!(
                "{} has no composerData or bubbleId rows, only rows under {}",
                db_path.display(),
                named(&unlike)
            ));
        }
    }
    warn_skipped(warnings, skipped_bubbles, "message", db_path);
    warn_skipped(warnings, skipped, "composer", db_path);
    if orphans.left.chats > 0 {
        warnings.push(format!(
            "skipped {} message row(s) of {} chat(s) in {} that have no composerData row",
            orphans.left.rows,
            orphans.left.chats,
            db_path.display()
        ));
    }
    if unlinked > 0 {
        warnings.push(format!(
            "{unlinked} chat(s) in {} list none of their stored messages",
            db_path.display()
        ));
    }
    // Loading went on, but what is shown is missing chats or may be wrong.
    if let Some(prefix) = &orphans.moved_to {
        notices.push(format!(
            "left out {} chat(s) with {} message row(s) in {}: they have no composerData row, \
             but `{prefix}:` rows name them. Cursor may have changed its storage format.",
            orphans.moved.chats,
            orphans.moved.rows,
            db_path.display()
        ));
    }
    if untyped > 0 {
        notices.push(format!(
            "{untyped} message(s) in {} have a type this version does not know and are shown \
             as `unknown`. Cursor may have changed its storage format.",
            db_path.display()
        ));
    }
    Ok(sessions)
}

/// Chats with stored messages but without a chat row that was read.
#[derive(Debug, Default)]
struct Orphans {
    /// Those that a row under another key prefix names, as when Cursor
    /// moves its chat rows, and the first such prefix.
    moved: Count,
    moved_to: Option<String>,
    /// Those that no other row names, as a deleted chat leaves them.
    left: Count,
}

#[derive(Debug, Default)]
struct Count {
    chats: usize,
    rows: usize,
}

impl Count {
    fn add(&mut self, rows: usize) {
        self.chats += 1;
        self.rows += rows;
    }
}

/// Sorts the chats of `bubbles` by whether a `<prefix>:<chat id>` row of
/// another prefix exists. Those with a `composerData` row of their own, which
/// could not be read and is reported as such, are left out.
fn find_orphans(
    conn: &Connection,
    db_path: &Path,
    bubbles: HashMap<String, HashMap<String, Bubble>>,
) -> Result<Orphans> {
    let mut orphans = Orphans::default();
    if bubbles.is_empty() {
        return Ok(orphans);
    }
    let db_err = |source| Error::Database {
        path: db_path.to_path_buf(),
        source,
    };
    let mut prefixes = key_prefixes(conn).map_err(db_err)?;
    prefixes.retain(|prefix| prefix != "bubbleId");
    // An unreadable chat row of its own explains a chat first.
    prefixes.sort_by_key(|prefix| prefix != "composerData");
    let mut exists = conn
        .prepare(&format!("SELECT 1 FROM {KV_TABLE} WHERE key = ?1"))
        .map_err(db_err)?;
    let mut chats: Vec<(String, usize)> = bubbles
        .into_iter()
        .map(|(chat, messages)| (chat, messages.len()))
        .collect();
    chats.sort();
    for (chat, rows) in chats {
        let mut named_by = None;
        if !chat.is_empty() {
            for prefix in &prefixes {
                if exists
                    .exists([format!("{prefix}:{chat}")])
                    .map_err(db_err)?
                {
                    named_by = Some(prefix);
                    break;
                }
            }
        }
        match named_by {
            Some(prefix) if prefix == "composerData" => {}
            Some(prefix) => {
                orphans.moved.add(rows);
                orphans.moved_to.get_or_insert_with(|| prefix.clone());
            }
            None => orphans.left.add(rows),
        }
    }
    Ok(orphans)
}

/// The prefixes, up to the first `:`, of the TEXT keys of `cursorDiskKV`. It
/// skips through the key index from one prefix to the next rather than reading
/// every key.
fn key_prefixes(conn: &Connection) -> rusqlite::Result<Vec<String>> {
    // `X''`, the smallest BLOB, sorts after every TEXT. The bound is a BLOB
    // cast to TEXT so that a key that is not UTF-8 can be skipped too.
    let query = |op: &str| {
        format!(
            "SELECT key FROM {KV_TABLE} WHERE key {op} CAST(?1 AS TEXT) AND key < X'' \
             ORDER BY key LIMIT 1"
        )
    };
    let mut from = conn.prepare(&query(">="))?;
    let mut after = conn.prepare(&query(">"))?;
    let first = |stmt: &mut rusqlite::Statement<'_>, bound: &[u8]| {
        stmt.query_row([bound], |row| match row.get_ref(0)? {
            ValueRef::Text(key) => Ok(Some(key.to_vec())),
            _ => Ok(None),
        })
        .optional()
        .map(Option::flatten)
    };
    let mut prefixes = Vec::new();
    let mut key = first(&mut from, b"")?;
    while let Some(found) = key {
        key = match found.iter().position(|&byte| byte == b':') {
            // Every key with this prefix sorts before `<prefix>;`.
            Some(colon) => {
                prefixes.push(String::from_utf8_lossy(&found[..colon]).into_owned());
                let mut end = found[..colon].to_vec();
                end.push(b';');
                first(&mut from, &end)?
            }
            None => first(&mut after, &found)?,
        };
    }
    Ok(prefixes)
}

#[cfg(test)]
thread_local! {
    /// Runs on this thread between reading the messages and reading the chats.
    #[allow(clippy::type_complexity)]
    static AFTER_BUBBLES: std::cell::RefCell<Option<Box<dyn FnMut()>>> =
        const { std::cell::RefCell::new(None) };
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
        store: Source::Ide,
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
    let mut stmt = conn.prepare(&rows_query()).map_err(db_err)?;
    let mut rows = stmt.query([prefix, &prefix_end(prefix)]).map_err(db_err)?;
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

/// The rows whose key is in `?1..?2`, a range that, unlike `LIKE`, SQLite
/// finds through the index on `key`. A key stored as a BLOB sorts after
/// every TEXT, so the range is searched once as TEXT and once as BLOB.
fn rows_query() -> String {
    format!(
        "SELECT key, value FROM {KV_TABLE} WHERE (key >= ?1 AND key < ?2 \
         OR key >= CAST(?1 AS BLOB) AND key < CAST(?2 AS BLOB)) AND value IS NOT NULL"
    )
}

/// The first key after all those that start with `prefix`, which ends in `:`
/// as every key prefix does: `bubbleId;` for `bubbleId:`.
fn prefix_end(prefix: &str) -> String {
    format!("{};", prefix.strip_suffix(':').unwrap_or(prefix))
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
    json::from_str(json).ok()
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

/// A chat read from its composer row.
struct Chat {
    session: Session,
    /// Whether its conversation led to any message: one of the `bubbles`
    /// stored for it, or one kept inline.
    linked: bool,
    /// Whether it has no message to show because every message it lists is
    /// there and holds only what is never shown, such as tool calls.
    only_hidden: bool,
}

/// The chat for `composer`, with its messages from `bubbles`, its stored
/// messages, or those kept inline, as older Cursor versions did for composers
/// without conversation headers. Messages without a known type are counted in
/// `untyped`.
fn composer_session(
    id: String,
    composer: Composer,
    mut bubbles: Option<HashMap<String, Bubble>>,
    untyped: &mut usize,
) -> Chat {
    let mut messages = Vec::new();
    let mut linked = false;
    // Whether every message it lists is there and holds something hidden.
    let mut all_hidden = true;
    for header in &composer.full_conversation_headers_only {
        let bubble = header
            .bubble_id
            .as_ref()
            .and_then(|bubble_id| bubbles.as_mut()?.remove(bubble_id));
        let Some(bubble) = bubble else {
            all_hidden = false;
            continue;
        };
        linked = true;
        all_hidden &= bubble.has_hidden_content();
        messages.extend(message(header.kind.or(bubble.kind), bubble, untyped));
    }
    if composer.full_conversation_headers_only.is_empty() {
        for value in composer.conversation {
            let Ok(bubble) = serde_json::from_value::<Bubble>(value) else {
                all_hidden = false;
                continue;
            };
            linked = true;
            all_hidden &= bubble.has_hidden_content();
            messages.extend(message(bubble.kind, bubble, untyped));
        }
    }
    let title = composer
        .name
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Untitled".to_string());
    let only_hidden = messages.is_empty() && linked && all_hidden;
    Chat {
        session: Session {
            id,
            title,
            source: Source::Ide,
            workspace: None,
            workspace_hash: None,
            created_at_ms: composer.created_at,
            updated_at_ms: composer.last_updated_at,
            model: None,
            messages,
        },
        linked,
        only_hidden,
    }
}

/// A message from `bubble`, unless it has no text or code. One without a
/// known `kind` is counted in `untyped` and goes on with the role `unknown`,
/// rather than a guess that may be wrong.
fn message(kind: Option<Kind>, mut bubble: Bubble, untyped: &mut usize) -> Option<Message> {
    let timestamp = bubble.timestamp.take();
    let content = extract_bubble_text(bubble);
    if content.is_empty() {
        return None;
    }
    let role = match kind {
        Some(Kind::User) => "user",
        Some(Kind::Assistant) => "assistant",
        Some(Kind::Other) | None => {
            *untyped += 1;
            "unknown"
        }
    };
    Some(Message {
        role: role.to_string(),
        content,
        timestamp,
    })
}

fn extract_bubble_text(bubble: Bubble) -> String {
    let mut parts = Vec::new();
    if let Some(text) = bubble.text {
        let trimmed = text.trim();
        if trimmed.len() == text.len() && !text.is_empty() {
            parts.push(text);
        } else if !trimmed.is_empty() {
            parts.push(trimmed.to_string());
        }
    }
    if parts.is_empty()
        && let Some(rich) = bubble.rich_text.as_deref()
        && !rich.is_empty()
        && let Ok(value) = json::from_str::<Value>(rich)
    {
        let extracted = collect_json_text(&value);
        if !extracted.is_empty() {
            parts.push(extracted);
        }
    }
    for block in bubble.code_blocks {
        if let Some(content) = block.content {
            if content.is_empty() {
                continue;
            }
            let lang = block.language.as_deref().unwrap_or("");
            parts.push(format!("```{lang}\n{content}\n```"));
        }
    }
    // The text alone, as most messages are, is moved rather than copied.
    if parts.len() == 1 {
        return parts.pop().unwrap_or_default();
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
        let (mut warnings, mut notices) = (Vec::new(), Vec::new());
        let sessions = load_from_db(path, &mut warnings, &mut notices);
        assert!(notices.is_empty(), "{notices:?}");
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
            ("bubbleId:c1:bad", SqlValue::Text(r#"{"bubbleId":"#.into())),
            // Odd field types cost only those fields.
            (
                "bubbleId:c1:odd",
                SqlValue::Text(r#"{"bubbleId":5,"text":["a"],"codeBlocks":null}"#.into()),
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
    fn chats_and_their_messages_come_from_one_commit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.vscdb");
        create_db(&path, &well_formed(false));
        // Cursor has the database open, so it is read in place while it writes.
        let writer = Connection::open(&path).unwrap();
        writer.pragma_update(None, "journal_mode", "wal").unwrap();
        writer
            .execute("INSERT INTO ItemTable VALUES ('open', 'yes')", [])
            .unwrap();
        // A chat and its message, committed between reading messages and chats.
        AFTER_BUBBLES.set(Some(Box::new(move || {
            writer
                .execute_batch(
                    r#"INSERT INTO cursorDiskKV VALUES ('bubbleId:late:b1', '{"type":1,"text":"late"}');
                       INSERT INTO cursorDiskKV VALUES ('composerData:late', '{"fullConversationHeadersOnly":[{"bubbleId":"b1"}]}');"#,
                )
                .unwrap();
        })));
        let (sessions, warnings) = load(&path);
        AFTER_BUBBLES.set(None);
        let sessions = sessions.unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        let ids: Vec<&str> = sessions.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["c1"]);
        assert_eq!(load(&path).0.unwrap().len(), 2);
    }

    #[test]
    fn rows_are_found_through_the_key_index() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.vscdb");
        let mut rows = well_formed(false);
        rows.extend([
            ("bubbleId", SqlValue::Text("{}".into())),
            ("bubbleId;", SqlValue::Text("{}".into())),
            ("bubbleIdX:c1:b1", SqlValue::Text("{}".into())),
            ("checkpointId:c1:k1", SqlValue::Text("{}".into())),
        ]);
        create_db(&path, &rows);
        let conn = Connection::open(&path).unwrap();
        // A key stored as a BLOB, as a binding of bytes leaves it.
        conn.execute(
            "INSERT INTO cursorDiskKV VALUES (?1, '{}')",
            [SqlValue::Blob(b"bubbleId:c1:b3".to_vec())],
        )
        .unwrap();
        // Every step searches the index; none scans the table.
        let plan = |sql: &str, params: &[&dyn rusqlite::ToSql]| -> Vec<String> {
            let mut stmt = conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
            stmt.query_map(params, |row| row.get(3))
                .unwrap()
                .map(Result::unwrap)
                .filter(|step: &String| step.contains(KV_TABLE))
                .collect()
        };
        for (sql, steps) in [
            (rows_query(), plan(&rows_query(), &[&"a", &"b"])),
            (
                "prefix scan".to_string(),
                plan(
                    &format!(
                        "SELECT key FROM {KV_TABLE} WHERE key >= CAST(?1 AS TEXT) AND key < X'' \
                         ORDER BY key LIMIT 1"
                    ),
                    &[&b"a".as_slice()],
                ),
            ),
        ] {
            assert!(!steps.is_empty(), "{sql}");
            for step in steps {
                assert!(
                    step.starts_with("SEARCH") && step.contains("INDEX"),
                    "{sql}: {step}"
                );
            }
        }

        assert_eq!(prefix_end("bubbleId:"), "bubbleId;");
        let mut keys = Vec::new();
        read_rows(&conn, &path, "bubbleId:", |key, _| {
            keys.push(key.to_string());
            true
        })
        .unwrap();
        keys.sort();
        assert_eq!(keys, ["bubbleId:c1:b1", "bubbleId:c1:b2", "bubbleId:c1:b3"]);
        assert_eq!(
            key_prefixes(&conn).unwrap(),
            ["bubbleId", "bubbleIdX", "checkpointId", "composerData"]
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
