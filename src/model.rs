use std::borrow::Cow;
use std::ops::{Deref, DerefMut};
use std::path::PathBuf;

use chrono::{DateTime, Datelike, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    /// Cursor Agent CLI chats (~/.cursor/chats and agent transcripts)
    Agent,
    /// Cursor IDE composer chats (state.vscdb)
    Ide,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Agent => "agent",
            Source::Ide => "ide",
        }
    }

    /// The store this one is not.
    pub fn other(self) -> Source {
        match self {
            Source::Agent => Source::Ide,
            Source::Ide => Source::Agent,
        }
    }

    /// The store's name in messages.
    pub fn name(self) -> &'static str {
        match self {
            Source::Agent => "Agent CLI",
            Source::Ide => "IDE",
        }
    }

    /// The Cursor product that writes this store.
    pub fn product(self) -> &'static str {
        match self {
            Source::Agent => "Cursor Agent CLI",
            Source::Ide => "Cursor IDE",
        }
    }

    /// The `--source` option that leaves this store unread, and what for.
    pub fn skip_option(self) -> &'static str {
        match self {
            Source::Agent => "`--source ide` to skip Agent CLI sessions",
            Source::Ide => "`--source agent` to skip IDE sessions",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
}

/// The role of the tool calls and results that reading builds only when
/// asked to (see [`crate::ReadOptions`]). They are no part of a session's
/// `message_count` or `content_chars`.
pub const TOOL_ROLE: &str = "tool";

impl Message {
    /// Whether this is a tool call or result rather than a message `show`
    /// prints by default.
    pub fn is_tool(&self) -> bool {
        self.role == TOOL_ROLE
    }

    /// The time to show with the message: epoch milliseconds, as the IDE
    /// stores them, as a UTC time; anything else as stored.
    pub fn timestamp_display(&self) -> Option<Cow<'_, str>> {
        let raw = self.timestamp.as_deref()?;
        let ms = raw
            .bytes()
            .all(|byte| byte.is_ascii_digit())
            .then(|| raw.parse().ok())
            .flatten();
        Some(match utc(ms) {
            Some(time) => Cow::Owned(format!("{} UTC", time.format("%Y-%m-%d %H:%M"))),
            None => Cow::Borrowed(raw),
        })
    }
}

/// Where a session's messages are, so that they can be read after listing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum MessagesAt {
    /// The session has no messages.
    #[default]
    Nowhere,
    /// An Agent CLI transcript.
    Transcript(PathBuf),
    /// An IDE chat: the row with `key` in the `state.vscdb` at `db`, a key
    /// stored as a BLOB when `blob_key` is set.
    IdeChat {
        db: PathBuf,
        key: String,
        blob_key: bool,
    },
}

/// A session without its messages: what `list` shows. Serialized, it is the
/// session part of an export.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSummary {
    pub id: String,
    pub title: String,
    pub source: Source,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// How many messages `show` and `export` read from `messages_at`.
    #[serde(skip)]
    pub message_count: usize,
    /// The characters (Unicode scalar values) of the content of those
    /// messages, for [`SessionSummary::token_estimate`].
    #[serde(skip)]
    pub content_chars: usize,
    #[serde(skip)]
    pub messages_at: MessagesAt,
}

impl SessionSummary {
    /// A summary with only these fields known and no messages.
    pub fn new(id: impl Into<String>, title: impl Into<String>, source: Source) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            source,
            workspace: None,
            workspace_hash: None,
            created_at_ms: None,
            updated_at_ms: None,
            model: None,
            message_count: 0,
            content_chars: 0,
            messages_at: MessagesAt::Nowhere,
        }
    }

    pub fn created_display(&self) -> String {
        format_ms(self.created_at_ms)
    }

    pub fn updated_display(&self) -> String {
        format_ms(self.updated_or_created_ms())
    }

    /// [`SessionSummary::created_display`] marked as UTC, for text that also
    /// shows local times.
    pub fn created_utc(&self) -> String {
        marked_utc(self.created_display())
    }

    /// [`SessionSummary::updated_display`] marked as UTC.
    pub fn updated_utc(&self) -> String {
        marked_utc(self.updated_display())
    }

    /// The time the UPDATED column shows, which also orders the list. A time
    /// it cannot show (see [`utc`]) is taken as missing.
    fn updated_or_created_ms(&self) -> Option<i64> {
        let shown = |ms: Option<i64>| ms.filter(|&ms| utc(Some(ms)).is_some());
        shown(self.updated_at_ms).or(shown(self.created_at_ms))
    }

    /// An estimate of how many tokens the messages `show` prints by default
    /// take: one per four characters, rounded up. It is no tokenizer's count.
    pub fn token_estimate(&self) -> usize {
        token_estimate(self.content_chars)
    }

    /// The `list --json` entry.
    pub fn json(&self) -> SummaryJson<'_> {
        SummaryJson {
            id: &self.id,
            title: &self.title,
            source: self.source,
            workspace: self.workspace.as_deref(),
            workspace_hash: self.workspace_hash.as_deref(),
            model: self.model.as_deref(),
            created_at: rfc3339(self.created_at_ms),
            updated_at: rfc3339(self.updated_at_ms),
            message_count: self.message_count,
            token_estimate: self.token_estimate(),
        }
    }
}

/// A session with its messages: what `show` and `export` print. It derefs to
/// its summary, whose `message_count` is the number of `messages` and
/// `content_chars` the characters of their content, tool messages left out.
/// Read with its tool messages, an Agent CLI session's text that tool calls
/// separate is several messages, which count as the one they are without.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(from = "StoredSession")]
pub struct Session {
    #[serde(flatten)]
    pub summary: SessionSummary,
    pub messages: Vec<Message>,
}

/// A session as an export stores it.
#[derive(Deserialize)]
struct StoredSession {
    #[serde(flatten)]
    summary: SessionSummary,
    messages: Vec<Message>,
}

impl From<StoredSession> for Session {
    fn from(stored: StoredSession) -> Self {
        Session::new(stored.summary, stored.messages)
    }
}

impl Session {
    pub fn new(mut summary: SessionSummary, messages: Vec<Message>) -> Self {
        let counted = || messages.iter().filter(|message| !message.is_tool());
        summary.message_count = counted().count();
        summary.content_chars = counted()
            .map(|message| content_chars(&message.content))
            .sum();
        Self { summary, messages }
    }

    /// The `show --json` object: the summary plus `messages`, which may be a
    /// tail of `self.messages`.
    pub fn detail<'a>(&'a self, messages: &'a [Message]) -> DetailJson<'a> {
        DetailJson {
            summary: self.summary.json(),
            messages: messages
                .iter()
                .map(|message| MessageJson {
                    role: &message.role,
                    content: &message.content,
                    timestamp: message.timestamp.as_deref(),
                })
                .collect(),
        }
    }
}

impl Deref for Session {
    type Target = SessionSummary;

    fn deref(&self) -> &SessionSummary {
        &self.summary
    }
}

impl DerefMut for Session {
    fn deref_mut(&mut self) -> &mut SessionSummary {
        &mut self.summary
    }
}

/// JSON shape of one `list --json` entry. Every key is always present.
#[derive(Debug, Clone, Serialize)]
pub struct SummaryJson<'a> {
    pub id: &'a str,
    pub title: &'a str,
    pub source: Source,
    pub workspace: Option<&'a str>,
    pub workspace_hash: Option<&'a str>,
    pub model: Option<&'a str>,
    /// RFC 3339 UTC, whole seconds.
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub message_count: usize,
    /// See [`SessionSummary::token_estimate`].
    pub token_estimate: usize,
}

/// JSON shape of `show --json`.
#[derive(Debug, Clone, Serialize)]
pub struct DetailJson<'a> {
    #[serde(flatten)]
    pub summary: SummaryJson<'a>,
    pub messages: Vec<MessageJson<'a>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MessageJson<'a> {
    pub role: &'a str,
    pub content: &'a str,
    /// As stored by Cursor; not normalised.
    pub timestamp: Option<&'a str>,
}

/// The characters of a message's content, as [`SessionSummary`] counts them:
/// Unicode scalar values.
pub fn content_chars(content: &str) -> usize {
    content.chars().count()
}

/// The tokens `chars` characters are estimated to take: `ceil(chars / 4)`.
pub fn token_estimate(chars: usize) -> usize {
    chars.div_ceil(4)
}

/// `ms` as a UTC time in the years 0000 to 9999, which RFC 3339 and the
/// UPDATED column can show. Anything else is taken as no time at all.
fn utc(ms: Option<i64>) -> Option<DateTime<Utc>> {
    let dt = DateTime::from_timestamp_millis(ms?)?;
    (0..=9999).contains(&dt.year()).then_some(dt)
}

fn rfc3339(ms: Option<i64>) -> Option<String> {
    utc(ms).map(|dt| dt.to_rfc3339_opts(SecondsFormat::Secs, true))
}

fn format_ms(ms: Option<i64>) -> String {
    utc(ms).map_or_else(
        || "—".to_string(),
        |dt| dt.format("%Y-%m-%d %H:%M").to_string(),
    )
}

fn marked_utc(shown: String) -> String {
    if shown == "—" {
        shown
    } else {
        format!("{shown} UTC")
    }
}

/// Merges the summaries of one session from several stores, which share its
/// ID, and sorts the result newest first. The first copy of a session keeps
/// its fields and takes those it lacks from the others; an Agent CLI copy
/// makes the session `agent`, and its messages win unless it has none.
pub fn merge_sessions(mut sessions: Vec<SessionSummary>) -> Vec<SessionSummary> {
    sessions.sort_by(|a, b| a.id.cmp(&b.id));
    let mut merged: Vec<SessionSummary> = Vec::new();
    for session in sessions {
        if let Some(existing) = merged.last_mut()
            && existing.id == session.id
        {
            merge_into(existing, session);
            continue;
        }
        merged.push(session);
    }
    merged.sort_by(|a, b| {
        b.updated_or_created_ms()
            .cmp(&a.updated_or_created_ms())
            .then(a.id.cmp(&b.id))
    });
    merged
}

fn merge_into(dst: &mut SessionSummary, src: SessionSummary) {
    if (dst.title.is_empty() || dst.title == dst.id) && !src.title.is_empty() {
        dst.title = src.title;
    }
    if dst.workspace.is_none() {
        dst.workspace = src.workspace;
    }
    if dst.workspace_hash.is_none() {
        dst.workspace_hash = src.workspace_hash;
    }
    if dst.created_at_ms.is_none() {
        dst.created_at_ms = src.created_at_ms;
    }
    if dst.updated_at_ms.is_none() || src.updated_at_ms > dst.updated_at_ms {
        dst.updated_at_ms = src.updated_at_ms;
    }
    if dst.model.is_none() {
        dst.model = src.model;
    }
    let takes_messages = src.message_count > 0
        && (dst.message_count == 0 || dst.source == Source::Ide && src.source == Source::Agent);
    if takes_messages {
        dst.message_count = src.message_count;
        dst.content_chars = src.content_chars;
        dst.messages_at = src.messages_at;
    }
    if dst.source == Source::Ide && src.source == Source::Agent {
        dst.source = Source::Agent;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary() -> SessionSummary {
        SessionSummary {
            workspace: Some("/Users/manuel.romero".into()),
            created_at_ms: Some(1_700_000_000_000),
            updated_at_ms: Some(1_700_000_100_123),
            ..SessionSummary::new(
                "f4eea6d2-d2d3-41ad-b290-824445295a15",
                "Langfuse Semantic Layer",
                Source::Ide,
            )
        }
    }

    fn session() -> Session {
        Session::new(
            summary(),
            vec![
                Message {
                    role: "user".into(),
                    content: "hello".into(),
                    timestamp: Some("1700000000000".into()),
                },
                Message {
                    role: "assistant".into(),
                    content: "hi".into(),
                    timestamp: None,
                },
            ],
        )
    }

    #[test]
    fn summary_json_has_stable_keys_and_rfc3339_times() {
        let session = session();
        let value = serde_json::to_value(session.json()).unwrap();
        let keys: Vec<&str> = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            keys,
            [
                "id",
                "title",
                "source",
                "workspace",
                "workspace_hash",
                "model",
                "created_at",
                "updated_at",
                "message_count",
                "token_estimate"
            ]
        );
        assert_eq!(value["source"], "ide");
        assert_eq!(value["workspace_hash"], serde_json::Value::Null);
        assert_eq!(value["model"], serde_json::Value::Null);
        assert_eq!(value["created_at"], "2023-11-14T22:13:20Z");
        assert_eq!(value["updated_at"], "2023-11-14T22:15:00Z");
        assert_eq!(value["message_count"], 2);
        // "hello" and "hi": seven characters.
        assert_eq!(value["token_estimate"], 2);

        let bare = SessionSummary {
            created_at_ms: None,
            updated_at_ms: None,
            ..session.summary
        };
        let value = serde_json::to_value(bare.json()).unwrap();
        assert_eq!(value["created_at"], serde_json::Value::Null);
        assert_eq!(value["updated_at"], serde_json::Value::Null);
    }

    #[test]
    fn out_of_range_times_serialize_as_null() {
        assert_eq!(rfc3339(Some(-1)).unwrap(), "1969-12-31T23:59:59Z");
        assert_eq!(
            rfc3339(Some(253_402_300_799_999)).unwrap(),
            "9999-12-31T23:59:59Z"
        );
        // Microseconds stored where milliseconds are expected land in year 55840.
        assert_eq!(rfc3339(Some(1_700_000_000_000_000)), None);
        assert_eq!(rfc3339(Some(253_402_300_800_000)), None);
        assert_eq!(rfc3339(Some(i64::MAX)), None);

        // The UPDATED column, 16 wide, shows those as missing too.
        assert_eq!(format_ms(Some(-1)), "1969-12-31 23:59");
        for ms in [1_700_000_000_000_000, i64::MAX, i64::MIN] {
            assert_eq!(format_ms(Some(ms)), "—");
        }
    }

    #[test]
    fn detail_json_adds_messages_with_nullable_timestamps() {
        let session = session();
        let value = serde_json::to_value(session.detail(&session.messages[1..])).unwrap();
        let object = value.as_object().unwrap();
        assert_eq!(object.len(), 11);
        assert_eq!(object.keys().next_back().unwrap(), "messages");
        assert_eq!(value["message_count"], 2);
        assert_eq!(
            value["messages"],
            serde_json::json!([{"role": "assistant", "content": "hi", "timestamp": null}])
        );

        let value = serde_json::to_value(session.detail(&session.messages)).unwrap();
        assert_eq!(value["messages"][0]["timestamp"], "1700000000000");
    }

    #[test]
    fn sessions_sort_newest_first_by_the_updated_time_shown() {
        let at = |id: &str, created_at_ms, updated_at_ms| SessionSummary {
            id: id.into(),
            created_at_ms,
            updated_at_ms,
            ..summary()
        };
        let sessions = vec![
            at("no-times", None, None),
            at("updated-2", Some(1), Some(2)),
            at("created-3", Some(3), None),
            at("tie-b", None, Some(2)),
            at("tie-a", Some(2), None),
        ];
        let merged = merge_sessions(sessions);
        let ids: Vec<&str> = merged.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(
            ids,
            ["created-3", "tie-a", "tie-b", "updated-2", "no-times"]
        );
        let shown: Vec<String> = merged.iter().map(SessionSummary::updated_display).collect();
        assert!(shown[..4].is_sorted_by(|a, b| a >= b), "{shown:?}");
    }

    #[test]
    fn a_time_that_cannot_be_shown_does_not_order_the_list() {
        // Microseconds where milliseconds belong: the year 57650.
        let micro = SessionSummary {
            id: "micro".into(),
            created_at_ms: Some(1_757_000_000_000),
            updated_at_ms: Some(1_757_000_000_000_000),
            ..summary()
        };
        let newer = SessionSummary {
            id: "newer".into(),
            created_at_ms: Some(1_757_500_000_000),
            updated_at_ms: None,
            ..summary()
        };
        let merged = merge_sessions(vec![micro, newer]);
        let ids: Vec<&str> = merged.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["newer", "micro"]);
        // It shows its creation time, as for a session never updated.
        assert_eq!(merged[1].updated_display(), "2025-09-04 15:33");
        assert_eq!(merged[1].updated_utc(), "2025-09-04 15:33 UTC");
        assert_eq!(
            serde_json::to_value(merged[1].json()).unwrap()["updated_at"],
            serde_json::Value::Null
        );
    }

    #[test]
    fn epoch_milliseconds_show_as_utc_and_other_times_as_stored() {
        let at = |timestamp: Option<&str>| Message {
            role: "user".into(),
            content: "x".into(),
            timestamp: timestamp.map(str::to_string),
        };
        let shown = |timestamp| at(timestamp).timestamp_display().map(Cow::into_owned);
        assert_eq!(
            shown(Some("1757200000000")).as_deref(),
            Some("2025-09-06 23:06 UTC")
        );
        for stored in [
            "Friday, Oct 2, 2026, 10:29 AM (UTC+2)",
            "2023-11-14T22:13:20.000Z",
            "1757200000000.5",
            "-1",
            "1757200000000000000000",
        ] {
            assert_eq!(shown(Some(stored)).as_deref(), Some(stored));
        }
        assert_eq!(shown(None), None);
        let bare = SessionSummary {
            created_at_ms: None,
            ..summary()
        };
        assert_eq!(bare.created_utc(), "—");
    }

    #[test]
    fn tokens_are_estimated_as_a_quarter_of_the_characters_rounded_up() {
        for (chars, tokens) in [(0, 0), (1, 1), (4, 1), (5, 2), (8, 2), (1001, 251)] {
            assert_eq!(token_estimate(chars), tokens, "{chars}");
        }
        // Characters are Unicode scalar values, not bytes.
        assert_eq!(content_chars("ünï日本👨‍👩‍👧"), 10);

        let message = |role: &str, content: &str| Message {
            role: role.into(),
            content: content.into(),
            timestamp: None,
        };
        let session = Session::new(
            summary(),
            vec![
                message("user", "héllo"),
                message(TOOL_ROLE, "Grep {\"pattern\":\"x\"}"),
                message("unknown", "?"),
            ],
        );
        // Tool messages count for neither.
        assert_eq!((session.message_count, session.content_chars), (2, 6));
        assert_eq!(session.token_estimate(), 2);
        assert_eq!(
            serde_json::to_value(session.json()).unwrap()["token_estimate"],
            2
        );
    }

    #[test]
    fn merged_sessions_keep_the_characters_of_the_messages_they_take() {
        let copy = |source, count, chars| SessionSummary {
            source,
            message_count: count,
            content_chars: chars,
            ..summary()
        };
        let merged = merge_sessions(vec![copy(Source::Ide, 3, 30), copy(Source::Agent, 2, 20)]);
        assert_eq!((merged[0].message_count, merged[0].content_chars), (2, 20));
        let merged = merge_sessions(vec![copy(Source::Ide, 3, 30), copy(Source::Agent, 0, 0)]);
        assert_eq!((merged[0].message_count, merged[0].content_chars), (3, 30));
    }

    #[test]
    fn export_json_shape_is_unchanged() {
        let session = Session::new(summary(), Vec::new());
        let value = serde_json::to_value(&session).unwrap();
        let keys: Vec<&str> = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            keys,
            [
                "id",
                "title",
                "source",
                "workspace",
                "created_at_ms",
                "updated_at_ms",
                "messages"
            ]
        );
    }
}
