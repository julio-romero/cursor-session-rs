use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use rusqlite::types::ValueRef;
use rusqlite::{Connection, OptionalExtension};
use serde::de::{DeserializeOwned, IgnoredAny};
use serde::{Deserialize, Deserializer};
use serde_json::Value;

use crate::detect::StoragePaths;
use crate::json::{self, lenient, lenient_ms};
use crate::model::{Message, MessagesAt, SessionSummary, Source, content_chars};
use crate::sqlite::with_readonly;
use crate::{Error, ReadOptions, Result, tools};

const KV_TABLE: &str = "cursorDiskKV";

// Every field is optional and read leniently (see `lenient`), so that one
// value of an unexpected type costs that value, not its chat or message.

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Composer {
    #[serde(default, deserialize_with = "lenient")]
    full_conversation_headers_only: Vec<ConversationHeader>,
    /// Older Cursor versions keep the messages in the composer itself.
    #[serde(default, deserialize_with = "lenient")]
    conversation: Vec<Value>,
}

/// A composer row as listing reads it: everything but its conversation.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ComposerMeta {
    #[serde(default, deserialize_with = "lenient")]
    composer_id: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    name: Option<String>,
    #[serde(default, deserialize_with = "lenient_ms")]
    created_at: Option<i64>,
    #[serde(default, deserialize_with = "lenient_ms")]
    last_updated_at: Option<i64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConversationHeader {
    #[serde(default, deserialize_with = "lenient")]
    bubble_id: Option<String>,
    #[serde(rename = "type", default, deserialize_with = "kind")]
    kind: Option<Kind>,
}

/// A message. Its tool call, `toolFormerData`, is read as a `T`: skipped
/// unread ([`IgnoredAny`]) unless tool messages are asked for ([`ToolCall`]).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", bound(deserialize = "T: Deserialize<'de>"))]
struct Bubble<T = IgnoredAny> {
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
    /// A tool call, shown only when asked for.
    #[serde(default)]
    tool_former_data: Option<T>,
    /// Images, which are never shown.
    #[serde(default, deserialize_with = "lenient")]
    images: Vec<Value>,
}

impl<T> Bubble<T> {
    /// Whether it holds something that is never shown, such as a tool call or
    /// an image, so that having no text to show is no sign of a new format.
    fn has_hidden_content(&self) -> bool {
        self.tool_former_data.is_some() || !self.images.is_empty() || !self.code_blocks.is_empty()
    }
}

/// What a message's `toolFormerData` is read as.
trait ToolData: DeserializeOwned {
    /// The tool messages it makes, in order.
    fn messages(self) -> impl Iterator<Item = Message>;
}

impl ToolData for IgnoredAny {
    fn messages(self) -> impl Iterator<Item = Message> {
        std::iter::empty()
    }
}

/// A tool call and what it returned, from a message's `toolFormerData`. It
/// is read leniently: a value of another shape makes no tool message, and
/// costs neither the message nor its chat.
#[derive(Debug, Default)]
struct ToolCall {
    name: Option<String>,
    /// `rawArgs`, else `params`: usually JSON in a string.
    args: Option<Value>,
    /// Usually JSON in a string.
    result: Option<Value>,
}

impl<'de> Deserialize<'de> for ToolCall {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let Value::Object(mut fields) = Value::deserialize(deserializer)? else {
            return Ok(Self::default());
        };
        let given = |value: &Value| match value {
            Value::Null => false,
            Value::String(text) => !text.trim().is_empty(),
            _ => true,
        };
        let name = match fields.remove("name") {
            Some(Value::String(name)) => Some(name),
            _ => None,
        };
        let args = ["rawArgs", "params"]
            .iter()
            .find_map(|key| fields.remove(*key).filter(given));
        let result = fields.remove("result").filter(given);
        Ok(Self { name, args, result })
    }
}

impl ToolData for ToolCall {
    /// The call, unless nothing names it or its arguments, then the result.
    fn messages(self) -> impl Iterator<Item = Message> {
        let call = (self.name.is_some() || self.args.is_some())
            .then(|| tools::call(self.name.as_deref(), self.args.as_ref()));
        let result = self.result.as_ref().and_then(tools::result);
        call.into_iter().chain(result).filter_map(tools::message)
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
) -> Result<Vec<SessionSummary>> {
    let Some(db_path) = &paths.global_storage_db else {
        return Ok(Vec::new());
    };
    load_from_db(db_path, warnings, notices)
}

/// Loads the composer sessions of a `state.vscdb` with their message counts;
/// see [`index`] and [`count`].
pub fn load_from_db(
    db_path: &Path,
    warnings: &mut Vec<String>,
    notices: &mut Vec<String>,
) -> Result<Vec<SessionSummary>> {
    match index(db_path, warnings)? {
        Some(index) => count(&index, &|_| true, warnings, notices),
        None => Ok(Vec::new()),
    }
}

/// The chats of a `state.vscdb`, without their messages counted.
pub(crate) struct Index {
    db: PathBuf,
    pub sessions: Vec<SessionSummary>,
    /// Chat rows that could not be read.
    skipped: usize,
    /// The message rows of chats without a chat row that was read.
    orphan_rows: Rows,
    orphans: Orphans,
    /// In a database without chat or message rows, the prefixes of the keys
    /// it has instead.
    other_prefixes: Vec<String>,
}

/// Message rows that were read and that could not be.
#[derive(Debug, Default, Clone, Copy)]
struct Rows {
    read: usize,
    skipped: usize,
}

/// Lists the chats of a `state.vscdb` from their rows, and finds the messages
/// stored for chats without one. `None` when there is nothing to read: no
/// file, an empty one, or one without tables, which `warnings` reports.
/// Fails when the rows show that the format changed.
pub(crate) fn index(db_path: &Path, warnings: &mut Vec<String>) -> Result<Option<Index>> {
    match fs::metadata(db_path) {
        Ok(meta) if meta.len() == 0 => {
            warnings.push(format!(
                "{} is empty; no Cursor IDE sessions loaded",
                db_path.display()
            ));
            return Ok(None);
        }
        Ok(_) => {}
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(source) => return Err(Error::access(db_path, source)),
    }
    // The read may run twice (see `with_readonly`); only the last one's
    // warnings are kept.
    let (index, found) = with_readonly(db_path, |conn| {
        let mut found = Vec::new();
        let index = read_index(conn, db_path, &mut found)?;
        Ok((index, found))
    })?;
    warnings.extend(found);
    Ok(index)
}

fn read_index(
    conn: &Connection,
    db_path: &Path,
    warnings: &mut Vec<String>,
) -> Result<Option<Index>> {
    // One read transaction, so that the messages without a chat are those of
    // the chats listed.
    let _snapshot = transaction(conn, db_path)?;
    if !check_schema(conn, db_path)? {
        warnings.push(format!(
            "{} has no tables; no Cursor IDE sessions loaded",
            db_path.display()
        ));
        return Ok(None);
    }

    let mut sessions = Vec::new();
    let mut ids = HashSet::new();
    let skipped = read_rows(conn, db_path, "composerData:", |key, blob_key, value| {
        let Some(composer) = parse_object::<ComposerMeta>(value) else {
            return false;
        };
        // A blank ID could not be looked up with `show`.
        let named = |id: &str| !id.trim().is_empty();
        let id = composer.composer_id.filter(|id| named(id)).or_else(|| {
            key.strip_prefix("composerData:")
                .filter(|id| named(id))
                .map(str::to_string)
        });
        let Some(id) = id else {
            return false;
        };
        ids.insert(id.clone());
        let title = composer
            .name
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "Untitled".to_string());
        sessions.push(SessionSummary {
            created_at_ms: composer.created_at,
            updated_at_ms: composer.last_updated_at,
            messages_at: MessagesAt::IdeChat {
                db: db_path.to_path_buf(),
                key: key.to_string(),
                blob_key,
            },
            ..SessionSummary::new(id, title, Source::Ide)
        });
        true
    })?;
    #[cfg(test)]
    AFTER_CHATS.with_borrow_mut(|hook| hook.as_mut().map(|hook| hook()));

    let (orphan_rows, orphan_chats) = orphan_messages(conn, db_path, &ids)?;
    let orphans = find_orphans(conn, db_path, orphan_chats)?;

    // Rows that are all unreadable, or messages whose chats are all under
    // another key, mean the format changed rather than that there is nothing.
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
        return Err(mismatch(match orphan_rows.read {
            1 => format!(
                "its bubbleId row belongs to no composerData row, and a `{prefix}:` row names \
                 its chat"
            ),
            read => format!(
                "none of its {read} bubbleId rows belongs to a composerData row, and \
                 `{prefix}:` rows name their chats"
            ),
        }));
    }
    // Not one chat or message row: either there are no chats, or their rows
    // are all under keys this version does not know.
    let mut other_prefixes = Vec::new();
    if sessions.is_empty() && orphan_rows.read + skipped + orphan_rows.skipped == 0 {
        other_prefixes = key_prefixes(conn).map_err(|source| Error::Database {
            path: db_path.to_path_buf(),
            source,
        })?;
        other_prefixes.retain(|prefix| prefix != "composerData" && prefix != "bubbleId");
    }
    Ok(Some(Index {
        db: db_path.to_path_buf(),
        sessions,
        skipped,
        orphan_rows,
        orphans,
        other_prefixes,
    }))
}

/// The chats of `index` that `selected` accepts, each with the number of
/// messages it shows. Chats are read one at a time, so no more than one
/// chat's messages are held. Messages that cannot be read are reported in
/// `warnings`; chats and messages that load only in part, as when Cursor
/// changes their format for new chats only, in `notices`. When the messages
/// read show that the format changed, loading fails; a count of only some
/// chats then counts them all to decide, as counting them all would.
pub(crate) fn count(
    index: &Index,
    selected: &dyn Fn(&str) -> bool,
    warnings: &mut Vec<String>,
    notices: &mut Vec<String>,
) -> Result<Vec<SessionSummary>> {
    if index.sessions.is_empty() {
        report(index, &Stats::default(), warnings, notices)?;
        return Ok(Vec::new());
    }
    let (sessions, mut stats) =
        with_readonly(&index.db, |conn| count_chats(conn, index, selected))?;
    if sessions.len() < index.sessions.len() && stats.looks_changed(index.orphan_rows) {
        stats = with_readonly(&index.db, |conn| count_chats(conn, index, &|_| true))?.1;
    }
    report(index, &stats, warnings, notices)?;
    Ok(sessions)
}

/// What counting the messages of chats found.
#[derive(Debug, Default, Clone, Copy)]
struct Stats {
    /// The message rows of the chats counted.
    rows: Rows,
    /// Chats with stored messages, and those of them that lead to none.
    stored: usize,
    unlinked: usize,
    /// Chats that list messages and should have one to show, and those of
    /// them that do.
    listing: usize,
    shown: usize,
    /// Messages shown, and those of them without a known type.
    messages: usize,
    untyped: usize,
}

impl Stats {
    /// Whether these chats, with the messages of chats without a chat row,
    /// look like a format this version does not know (see [`report`]).
    fn looks_changed(&self, orphan_rows: Rows) -> bool {
        let read = self.rows.read + orphan_rows.read;
        let skipped = self.rows.skipped + orphan_rows.skipped;
        (read == 0 && skipped > 0)
            || (self.unlinked > 0 && self.unlinked == self.stored)
            || (self.listing > 0 && self.shown == 0)
            || (self.untyped > 0 && self.untyped == self.messages)
    }
}

fn count_chats(
    conn: &Connection,
    index: &Index,
    selected: &dyn Fn(&str) -> bool,
) -> Result<(Vec<SessionSummary>, Stats)> {
    // One read transaction, so that a chat and its messages come from the
    // same commit while Cursor writes.
    let _snapshot = transaction(conn, &index.db)?;
    let mut stats = Stats::default();
    // Of several rows naming one chat, the first takes its stored messages.
    let mut given = HashSet::new();
    let mut counted = Vec::new();
    for session in index
        .sessions
        .iter()
        .filter(|session| selected(&session.id))
    {
        let mut session = session.clone();
        if let MessagesAt::IdeChat { key, blob_key, .. } = &session.messages_at
            && let Some(composer) = read_composer(conn, &index.db, key, *blob_key)?
        {
            let lists = !composer.full_conversation_headers_only.is_empty()
                || !composer.conversation.is_empty();
            let bubbles = if given.insert(session.id.clone()) {
                chat_bubbles(conn, &index.db, &session.id, &mut stats.rows)?
            } else {
                None
            };
            let has_stored = bubbles.is_some();
            let chat = chat_messages::<IgnoredAny>(composer, bubbles, &mut stats.untyped);
            if has_stored {
                stats.stored += 1;
                stats.unlinked += usize::from(!chat.linked);
            }
            if lists && !chat.only_hidden {
                stats.listing += 1;
                stats.shown += usize::from(!chat.messages.is_empty());
            }
            stats.messages += chat.messages.len();
            session.message_count = chat.messages.len();
            session.content_chars = chat
                .messages
                .iter()
                .map(|message| content_chars(&message.content))
                .sum();
        }
        counted.push(session);
    }
    Ok((counted, stats))
}

/// Fails when the rows read show that the format changed, and reports what
/// was left out.
fn report(
    index: &Index,
    stats: &Stats,
    warnings: &mut Vec<String>,
    notices: &mut Vec<String>,
) -> Result<()> {
    let db_path = &index.db;
    let read = stats.rows.read + index.orphan_rows.read;
    let skipped_bubbles = stats.rows.skipped + index.orphan_rows.skipped;
    // Messages that are all unreadable, chats that all lead to none of their
    // messages, or messages all without a type mean the format changed
    // rather than that there is nothing.
    let mismatch = |detail| Error::SchemaMismatch {
        store: Source::Ide,
        path: db_path.to_path_buf(),
        detail,
    };
    if read == 0 && skipped_bubbles > 0 {
        return Err(mismatch(match skipped_bubbles {
            1 => "its bubbleId row could not be read".to_string(),
            _ => format!("none of its {skipped_bubbles} bubbleId rows could be read"),
        }));
    }
    if stats.unlinked > 0 && stats.unlinked == stats.stored {
        return Err(mismatch(
            "no chat lists the messages stored for it".to_string(),
        ));
    }
    if stats.listing > 0 && stats.shown == 0 {
        return Err(mismatch(match stats.listing {
            1 => "its one chat that lists messages has no readable message".to_string(),
            listing => {
                format!("none of its {listing} chats that list messages has a readable message")
            }
        }));
    }
    if stats.untyped > 0 && stats.untyped == stats.messages {
        return Err(mismatch(match stats.messages {
            1 => "its one message has no known type".to_string(),
            messages => format!("none of its {messages} messages has a known type"),
        }));
    }
    if !index.other_prefixes.is_empty() {
        // Rows named like chats or messages are most likely those, moved.
        let (alike, unlike): (Vec<&String>, Vec<&String>) =
            index.other_prefixes.iter().partition(|prefix| {
                let prefix = prefix.to_ascii_lowercase();
                prefix.contains("composer") || prefix.contains("bubble")
            });
        let named = |prefixes: &[&String]| {
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
        } else {
            warnings.push(format!(
                "{} has no composerData or bubbleId rows, only rows under {}",
                db_path.display(),
                named(&unlike)
            ));
        }
    }
    warn_skipped(warnings, skipped_bubbles, "message", db_path);
    warn_skipped(warnings, index.skipped, "composer", db_path);
    let orphans = &index.orphans;
    if orphans.left.chats > 0 {
        warnings.push(format!(
            "skipped {} message row(s) of {} chat(s) in {} that have no composerData row",
            orphans.left.rows,
            orphans.left.chats,
            db_path.display()
        ));
    }
    if stats.unlinked > 0 {
        warnings.push(format!(
            "{} chat(s) in {} list none of their stored messages",
            stats.unlinked,
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
    if stats.untyped > 0 {
        notices.push(format!(
            "{} message(s) in {} have a type this version does not know and are shown as \
             `unknown`. Cursor may have changed its storage format.",
            stats.untyped,
            db_path.display()
        ));
    }
    Ok(())
}

/// The messages of the chat whose row has `key` (a BLOB when `blob_key` is
/// set), with the messages stored for chat `id`, and their tool calls and
/// results when `read.tools` is set.
pub(crate) fn read_messages(
    db_path: &Path,
    key: &str,
    blob_key: bool,
    id: &str,
    read: ReadOptions,
) -> Result<Vec<Message>> {
    if read.tools {
        read_chat::<ToolCall>(db_path, key, blob_key, id)
    } else {
        read_chat::<IgnoredAny>(db_path, key, blob_key, id)
    }
}

fn read_chat<T: ToolData>(
    db_path: &Path,
    key: &str,
    blob_key: bool,
    id: &str,
) -> Result<Vec<Message>> {
    with_readonly(db_path, |conn| {
        let _snapshot = transaction(conn, db_path)?;
        let Some(composer) = read_composer(conn, db_path, key, blob_key)? else {
            return Ok(Vec::new());
        };
        let bubbles = chat_bubbles::<T>(conn, db_path, id, &mut Rows::default())?;
        Ok(chat_messages(composer, bubbles, &mut 0).messages)
    })
}

fn transaction<'a>(conn: &'a Connection, db_path: &Path) -> Result<rusqlite::Transaction<'a>> {
    // Dropping it ends the read.
    conn.unchecked_transaction()
        .map_err(|source| Error::Database {
            path: db_path.to_path_buf(),
            source,
        })
}

/// The chat row with `key`, if it is still there and readable.
fn read_composer(
    conn: &Connection,
    db_path: &Path,
    key: &str,
    blob_key: bool,
) -> Result<Option<Composer>> {
    let mut stmt = conn
        .prepare_cached(&format!(
            "SELECT value FROM {KV_TABLE} WHERE key = ?1 AND value IS NOT NULL"
        ))
        .map_err(|source| Error::Database {
            path: db_path.to_path_buf(),
            source,
        })?;
    let read = |row: &rusqlite::Row<'_>| Ok(as_text(row.get_ref(0)).map(str::to_string));
    let value = if blob_key {
        stmt.query_row([key.as_bytes()], read)
    } else {
        stmt.query_row([key], read)
    };
    let value = value
        .optional()
        .map_err(|source| Error::Database {
            path: db_path.to_path_buf(),
            source,
        })?
        .flatten();
    Ok(value.and_then(|value| parse_object(&value)))
}

/// The messages stored for chat `id`, its `bubbleId:<id>:<message>` rows, by
/// message ID; `None` when none of them could be read. Rows are counted in
/// `rows`.
fn chat_bubbles<T: DeserializeOwned>(
    conn: &Connection,
    db_path: &Path,
    id: &str,
    rows: &mut Rows,
) -> Result<Option<HashMap<String, Bubble<T>>>> {
    let mut bubbles = HashMap::new();
    for_chat_rows(
        conn,
        db_path,
        id.as_bytes(),
        |bubble_id, bubble| match bubble {
            Some(bubble) => {
                rows.read += 1;
                let id = bubble.bubble_id.clone().unwrap_or(bubble_id);
                bubbles.insert(id, bubble);
            }
            None => rows.skipped += 1,
        },
    )?;
    Ok((!bubbles.is_empty()).then_some(bubbles))
}

const BUBBLE_PREFIX: &[u8] = b"bubbleId:";

/// Calls `visit` with the message ID from the key and the message of each
/// non-NULL `bubbleId:<chat>:<message>` row of `chat`; `None` for a row that
/// cannot be read.
fn for_chat_rows<T: DeserializeOwned>(
    conn: &Connection,
    db_path: &Path,
    chat: &[u8],
    mut visit: impl FnMut(String, Option<Bubble<T>>),
) -> Result<()> {
    let bound = |end: u8| [BUBBLE_PREFIX, chat, &[end]].concat();
    scan_rows(conn, db_path, &bound(b':'), &bound(b';'), |row| {
        // The chat ID ends at the first `:`, so an ID with one of its own has
        // the keys of another chat in its range.
        if chat_of(row.key) != Some(chat) {
            return;
        }
        let message = row.text.and_then(|(key, value)| {
            let message_id = key.split_once(':')?.1.split_once(':')?.1;
            Some((message_id.to_string(), parse_object::<Bubble<T>>(value)?))
        });
        match message {
            Some((message_id, bubble)) => visit(message_id, Some(bubble)),
            None => visit(String::new(), None),
        }
    })
}

/// The chat ID of a `bubbleId:` key, which ends at its next `:`.
fn chat_of(key: &[u8]) -> Option<&[u8]> {
    let rest = key.strip_prefix(BUBBLE_PREFIX)?;
    rest.iter()
        .position(|&byte| byte == b':')
        .map(|colon| &rest[..colon])
}

/// The messages stored for chats that no chat row read names, by chat: how
/// many rows were read and could not be, and for each chat with a readable
/// one, how many messages it has. Only the rows of those chats are read.
fn orphan_messages(
    conn: &Connection,
    db_path: &Path,
    ids: &HashSet<String>,
) -> Result<(Rows, Vec<(String, usize)>)> {
    let db_err = |source| Error::Database {
        path: db_path.to_path_buf(),
        source,
    };
    let (chats, loose) = message_chats(conn).map_err(db_err)?;
    let mut rows = Rows::default();
    let mut orphans: BTreeMap<String, HashSet<String>> = BTreeMap::new();
    let mut add = |chat: &str, message: Option<(String, Bubble)>, rows: &mut Rows| match message {
        Some((message_id, bubble)) => {
            rows.read += 1;
            let id = bubble.bubble_id.unwrap_or(message_id);
            orphans.entry(chat.to_string()).or_default().insert(id);
        }
        None => rows.skipped += 1,
    };
    for chat in chats {
        let id = std::str::from_utf8(&chat).ok();
        if id.is_some_and(|id| ids.contains(id)) {
            continue;
        }
        for_chat_rows(conn, db_path, &chat, |message_id, bubble| {
            // A chat ID that is not UTF-8 has only keys that are not either.
            let chat = id.unwrap_or_default();
            add(chat, bubble.map(|bubble| (message_id, bubble)), &mut rows);
        })?;
    }
    // Keys without a chat ID belong to the chat ``, as do `bubbleId::` keys.
    for (key, blob_key) in loose {
        let mut stmt = conn
            .prepare_cached(&format!(
                "SELECT value FROM {KV_TABLE} WHERE key = CAST(?1 AS {}) AND value IS NOT NULL",
                if blob_key { "BLOB" } else { "TEXT" }
            ))
            .map_err(db_err)?;
        let value = stmt
            .query_row(
                [&key],
                |row| Ok(as_text(row.get_ref(0)).map(str::to_string)),
            )
            .optional()
            .map_err(db_err)?;
        let Some(value) = value else {
            continue;
        };
        let message = std::str::from_utf8(&key)
            .ok()
            .zip(value)
            .and_then(|(_, value)| parse_object::<Bubble>(&value))
            .map(|bubble| (String::new(), bubble));
        add("", message, &mut rows);
    }
    let orphans = orphans
        .into_iter()
        .map(|(chat, messages)| (chat, messages.len()))
        .collect();
    Ok((rows, orphans))
}

/// The chat IDs, as bytes, of the `bubbleId:<chat>:<message>` keys, and the
/// `bubbleId:` keys without a second `:`, with whether each is a BLOB. It
/// skips through the key index from one chat to the next rather than reading
/// every key.
#[allow(clippy::type_complexity)]
fn message_chats(conn: &Connection) -> rusqlite::Result<(BTreeSet<Vec<u8>>, Vec<(Vec<u8>, bool)>)> {
    let mut chats = BTreeSet::new();
    let mut loose = Vec::new();
    // TEXT keys sort before BLOB keys, so each is searched on its own.
    for (cast, blob_key) in [("TEXT", false), ("BLOB", true)] {
        let query = |op: &str| {
            format!(
                "SELECT key FROM {KV_TABLE} WHERE key {op} CAST(?1 AS {cast}) \
                 AND key < CAST(?2 AS {cast}) ORDER BY key LIMIT 1"
            )
        };
        let mut from = conn.prepare(&query(">="))?;
        let mut after = conn.prepare(&query(">"))?;
        let end = b"bubbleId;".as_slice();
        let first = |stmt: &mut rusqlite::Statement<'_>, bound: &[u8]| {
            stmt.query_row([bound, end], |row| match row.get_ref(0)? {
                ValueRef::Text(key) | ValueRef::Blob(key) => Ok(Some(key.to_vec())),
                _ => Ok(None),
            })
            .optional()
            .map(Option::flatten)
        };
        let mut key = first(&mut from, BUBBLE_PREFIX)?;
        while let Some(found) = key {
            key = match chat_of(&found) {
                // Every key of this chat sorts before `bubbleId:<chat>;`.
                Some(chat) => {
                    let next = [BUBBLE_PREFIX, chat, b";"].concat();
                    chats.insert(chat.to_vec());
                    first(&mut from, &next)?
                }
                None => {
                    let next = first(&mut after, &found)?;
                    loose.push((found, blob_key));
                    next
                }
            };
        }
    }
    Ok((chats, loose))
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

/// Sorts `chats`, with how many messages each has, by whether a
/// `<prefix>:<chat id>` row of another prefix exists. Those with a
/// `composerData` row of their own, which could not be read and is reported
/// as such, are left out.
fn find_orphans(conn: &Connection, db_path: &Path, chats: Vec<(String, usize)>) -> Result<Orphans> {
    let mut orphans = Orphans::default();
    if chats.is_empty() {
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
    /// Runs on this thread between reading the chat rows and looking for
    /// messages without a chat.
    #[allow(clippy::type_complexity)]
    static AFTER_CHATS: std::cell::RefCell<Option<Box<dyn FnMut()>>> =
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

/// Calls `visit` with the key, whether it is a BLOB, and the value of every
/// non-NULL row whose key starts with `prefix`. Values may be stored as TEXT
/// or BLOB. Returns how many rows could not be decoded, either here or by
/// `visit` returning `false`.
fn read_rows(
    conn: &Connection,
    db_path: &Path,
    prefix: &str,
    mut visit: impl FnMut(&str, bool, &str) -> bool,
) -> Result<usize> {
    let mut skipped = 0;
    scan_rows(
        conn,
        db_path,
        prefix.as_bytes(),
        prefix_end(prefix).as_bytes(),
        |row| {
            let decoded = match row.text {
                Some((key, value)) => visit(key, row.blob_key, value),
                None => false,
            };
            if !decoded {
                skipped += 1;
            }
        },
    )?;
    Ok(skipped)
}

/// A row as [`scan_rows`] finds it.
struct Row<'a> {
    key: &'a [u8],
    blob_key: bool,
    /// The key and the value, when both are UTF-8.
    text: Option<(&'a str, &'a str)>,
}

/// Calls `visit` with every non-NULL row whose key, as TEXT or as a BLOB, is
/// in `lower..upper`.
fn scan_rows(
    conn: &Connection,
    db_path: &Path,
    lower: &[u8],
    upper: &[u8],
    mut visit: impl FnMut(Row<'_>),
) -> Result<()> {
    let db_err = |source| Error::Database {
        path: db_path.to_path_buf(),
        source,
    };
    let mut stmt = conn.prepare_cached(&rows_query()).map_err(db_err)?;
    let mut rows = stmt.query([lower, upper]).map_err(db_err)?;
    while let Some(row) = rows.next().map_err(db_err)? {
        let (key, blob_key) = match row.get_ref(0) {
            Ok(ValueRef::Text(key)) => (key, false),
            Ok(ValueRef::Blob(key)) => (key, true),
            _ => (&[][..], false),
        };
        let text = std::str::from_utf8(key).ok().zip(as_text(row.get_ref(1)));
        visit(Row {
            key,
            blob_key,
            text,
        });
    }
    Ok(())
}

/// The rows whose key is in `?1..?2`, a range that, unlike `LIKE`, SQLite
/// finds through the index on `key`. A key stored as a BLOB sorts after
/// every TEXT, so the range is searched once as TEXT and once as BLOB. The
/// bounds are bound as BLOBs, so that they can be any bytes.
fn rows_query() -> String {
    format!(
        "SELECT key, value FROM {KV_TABLE} WHERE (key >= CAST(?1 AS TEXT) AND key < CAST(?2 AS TEXT) \
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

/// The messages of a chat, read from its composer row and its stored
/// messages.
struct ChatMessages {
    messages: Vec<Message>,
    /// Whether its conversation led to any message: one of the `bubbles`
    /// stored for it, or one kept inline.
    linked: bool,
    /// Whether it has no message to show because every message it lists is
    /// there and holds only what is never shown, such as tool calls.
    only_hidden: bool,
}

/// The messages of `composer`, from `bubbles`, its stored messages, or those
/// kept inline, as older Cursor versions did for composers without
/// conversation headers. Messages without a known type are counted in
/// `untyped`. Each message is followed by the tool messages of its tool call,
/// which only a `T` other than [`IgnoredAny`] makes.
fn chat_messages<T: ToolData>(
    composer: Composer,
    mut bubbles: Option<HashMap<String, Bubble<T>>>,
    untyped: &mut usize,
) -> ChatMessages {
    let mut messages = Vec::new();
    let mut linked = false;
    // Whether every message it lists is there and holds something hidden.
    let mut all_hidden = true;
    for header in &composer.full_conversation_headers_only {
        let bubble = header
            .bubble_id
            .as_ref()
            .and_then(|bubble_id| bubbles.as_mut()?.remove(bubble_id));
        let Some(mut bubble) = bubble else {
            all_hidden = false;
            continue;
        };
        linked = true;
        all_hidden &= bubble.has_hidden_content();
        let tool = bubble.tool_former_data.take();
        messages.extend(message(header.kind.or(bubble.kind), bubble, untyped));
        messages.extend(tool.into_iter().flat_map(ToolData::messages));
    }
    if composer.full_conversation_headers_only.is_empty() {
        for value in composer.conversation {
            let Ok(mut bubble) = serde_json::from_value::<Bubble<T>>(value) else {
                all_hidden = false;
                continue;
            };
            linked = true;
            all_hidden &= bubble.has_hidden_content();
            let tool = bubble.tool_former_data.take();
            messages.extend(message(bubble.kind, bubble, untyped));
            messages.extend(tool.into_iter().flat_map(ToolData::messages));
        }
    }
    let only_hidden = messages.is_empty() && linked && all_hidden;
    ChatMessages {
        messages,
        linked,
        only_hidden,
    }
}

/// A message from `bubble`, unless it has no text or code. One without a
/// known `kind` is counted in `untyped` and goes on with the role `unknown`,
/// rather than a guess that may be wrong.
fn message<T>(kind: Option<Kind>, mut bubble: Bubble<T>, untyped: &mut usize) -> Option<Message> {
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

fn extract_bubble_text<T>(bubble: Bubble<T>) -> String {
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

    use crate::model::Session;

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

    /// The sessions with their messages, which number what listing counted.
    fn load(path: &Path) -> (Result<Vec<Session>>, Vec<String>) {
        let (mut warnings, mut notices) = (Vec::new(), Vec::new());
        let sessions = load_from_db(path, &mut warnings, &mut notices).and_then(|summaries| {
            summaries
                .iter()
                .map(|summary| {
                    let session = crate::load_messages(summary)?;
                    assert_eq!(
                        session.messages.len(),
                        summary.message_count,
                        "{}",
                        summary.id
                    );
                    assert_eq!(session.content_chars, summary.content_chars);
                    Ok(session)
                })
                .collect()
        });
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
            let sessions = sessions.unwrap();
            loaded.push((serde_json::to_string(&sessions).unwrap(), sessions));
        }
        assert_eq!(loaded[0].0, loaded[1].0);
        let sessions = &loaded[0].1;
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].title, "Plan");
        let contents: Vec<_> = sessions[0].messages.iter().map(|m| &m.content).collect();
        assert_eq!(contents, ["question", "answer"]);
    }

    #[test]
    fn tool_calls_are_read_only_when_asked_for() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.vscdb");
        let headers: Vec<String> = (1..=8)
            .map(|n| format!(r#"{{"bubbleId":"b{n}","type":{}}}"#, 2 - n % 2))
            .collect();
        let composer = format!(
            r#"{{"composerId":"c1","name":"Tools","fullConversationHeadersOnly":[{}]}}"#,
            headers.join(",")
        );
        let bubbles = [
            r#"{"type":1,"text":"question"}"#,
            // Text, then the call it makes and what that returned.
            r#"{"type":2,"text":"Let me look.","toolFormerData":{"tool":5,"name":"read_file","rawArgs":"{\"target_file\": \"a.rs\"}","params":"{}","result":"{\"contents\":\"fn main() {}\"}","status":"completed"}}"#,
            r#"{"type":2,"text":"","toolFormerData":{"name":"run_terminal_cmd","params":{"command":"ls"},"status":"cancelled"}}"#,
            // Shapes this version does not know cost only the tool message.
            r#"{"type":1,"text":"","toolFormerData":"not an object"}"#,
            r#"{"type":2,"text":"","toolFormerData":{"name":7,"result":["a",{"text":"b"}]}}"#,
            r#"{"type":1,"text":"","toolFormerData":{}}"#,
            r#"{"type":2,"text":"","toolFormerData":null}"#,
            r#"{"type":2,"text":"answer"}"#,
        ];
        let mut rows = vec![("composerData:c1".to_string(), SqlValue::Text(composer))];
        for (n, bubble) in bubbles.iter().enumerate() {
            let key = format!("bubbleId:c1:b{}", n + 1);
            rows.push((key, SqlValue::Text((*bubble).to_string())));
        }
        let rows: Vec<(&str, SqlValue)> = rows
            .iter()
            .map(|(key, value)| (key.as_str(), value.clone()))
            .collect();
        create_db(&path, &rows);

        let (sessions, warnings) = load(&path);
        assert!(warnings.is_empty(), "{warnings:?}");
        let session = &sessions.unwrap()[0];
        let shown = |messages: &[Message]| -> Vec<(String, String)> {
            messages
                .iter()
                .map(|m| (m.role.clone(), m.content.clone()))
                .collect()
        };
        assert_eq!(session.message_count, 3);
        let key = "composerData:c1";
        let with = read_messages(&path, key, false, "c1", ReadOptions { tools: true }).unwrap();
        let expected = [
            ("user", "question"),
            ("assistant", "Let me look."),
            ("tool", r#"read_file {"target_file":"a.rs"}"#),
            ("tool", r#"{"contents":"fn main() {}"}"#),
            ("tool", r#"run_terminal_cmd {"command":"ls"}"#),
            ("tool", "a\n\nb"),
            ("assistant", "answer"),
        ];
        let expected: Vec<(String, String)> = expected
            .iter()
            .map(|(role, content)| (role.to_string(), content.to_string()))
            .collect();
        assert_eq!(shown(&with), expected);
        // The other messages are those read without the tools.
        let others: Vec<Message> = with.iter().filter(|m| !m.is_tool()).cloned().collect();
        assert_eq!(shown(&others), shown(&session.messages));
        let with = Session::new(session.summary.clone(), with);
        assert_eq!(
            (with.message_count, with.content_chars),
            (session.message_count, session.content_chars)
        );
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
        AFTER_CHATS.set(Some(Box::new(move || {
            writer
                .execute_batch(
                    r#"INSERT INTO cursorDiskKV VALUES ('bubbleId:late:b1', '{"type":1,"text":"late"}');
                       INSERT INTO cursorDiskKV VALUES ('composerData:late', '{"fullConversationHeadersOnly":[{"bubbleId":"b1"}]}');"#,
                )
                .unwrap();
        })));
        let (sessions, warnings) = load(&path);
        AFTER_CHATS.set(None);
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
        read_rows(&conn, &path, "bubbleId:", |key, _, _| {
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
