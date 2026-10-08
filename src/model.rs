use std::borrow::Cow;

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

impl Message {
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
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
    pub messages: Vec<Message>,
}

impl Session {
    pub fn message_count(&self) -> usize {
        self.messages.len()
    }

    pub fn created_display(&self) -> String {
        format_ms(self.created_at_ms)
    }

    pub fn updated_display(&self) -> String {
        format_ms(self.updated_or_created_ms())
    }

    /// [`Session::created_display`] marked as UTC, for text that also shows
    /// local times.
    pub fn created_utc(&self) -> String {
        marked_utc(self.created_display())
    }

    /// [`Session::updated_display`] marked as UTC.
    pub fn updated_utc(&self) -> String {
        marked_utc(self.updated_display())
    }

    /// The time the UPDATED column shows, which also orders the list. A time
    /// it cannot show (see [`utc`]) is taken as missing.
    fn updated_or_created_ms(&self) -> Option<i64> {
        let shown = |ms: Option<i64>| ms.filter(|&ms| utc(Some(ms)).is_some());
        shown(self.updated_at_ms).or(shown(self.created_at_ms))
    }

    pub fn summary(&self) -> SessionSummary<'_> {
        SessionSummary {
            id: &self.id,
            title: &self.title,
            source: self.source,
            workspace: self.workspace.as_deref(),
            workspace_hash: self.workspace_hash.as_deref(),
            model: self.model.as_deref(),
            created_at: rfc3339(self.created_at_ms),
            updated_at: rfc3339(self.updated_at_ms),
            message_count: self.message_count(),
        }
    }

    /// The summary plus `messages`, which may be a tail of `self.messages`.
    pub fn detail<'a>(&'a self, messages: &'a [Message]) -> SessionDetail<'a> {
        SessionDetail {
            summary: self.summary(),
            messages: messages
                .iter()
                .map(|message| MessageDetail {
                    role: &message.role,
                    content: &message.content,
                    timestamp: message.timestamp.as_deref(),
                })
                .collect(),
        }
    }
}

/// JSON shape of one `list --json` entry. Every key is always present.
#[derive(Debug, Clone, Serialize)]
pub struct SessionSummary<'a> {
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
}

/// JSON shape of `show --json`.
#[derive(Debug, Clone, Serialize)]
pub struct SessionDetail<'a> {
    #[serde(flatten)]
    pub summary: SessionSummary<'a>,
    pub messages: Vec<MessageDetail<'a>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MessageDetail<'a> {
    pub role: &'a str,
    pub content: &'a str,
    /// As stored by Cursor; not normalised.
    pub timestamp: Option<&'a str>,
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

pub fn merge_sessions(mut sessions: Vec<Session>) -> Vec<Session> {
    sessions.sort_by(|a, b| a.id.cmp(&b.id));
    let mut merged: Vec<Session> = Vec::new();
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

fn merge_into(dst: &mut Session, src: Session) {
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
    if dst.messages.is_empty() && !src.messages.is_empty() {
        dst.messages = src.messages;
    } else if dst.source == Source::Ide && src.source == Source::Agent && !src.messages.is_empty() {
        dst.messages = src.messages;
        dst.source = Source::Agent;
    }
    if dst.source == Source::Ide && src.source == Source::Agent {
        dst.source = Source::Agent;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> Session {
        Session {
            id: "f4eea6d2-d2d3-41ad-b290-824445295a15".into(),
            title: "Langfuse Semantic Layer".into(),
            source: Source::Ide,
            workspace: Some("/Users/manuel.romero".into()),
            workspace_hash: None,
            created_at_ms: Some(1_700_000_000_000),
            updated_at_ms: Some(1_700_000_100_123),
            model: None,
            messages: vec![
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
        }
    }

    #[test]
    fn summary_json_has_stable_keys_and_rfc3339_times() {
        let session = session();
        let value = serde_json::to_value(session.summary()).unwrap();
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
                "message_count"
            ]
        );
        assert_eq!(value["source"], "ide");
        assert_eq!(value["workspace_hash"], serde_json::Value::Null);
        assert_eq!(value["model"], serde_json::Value::Null);
        assert_eq!(value["created_at"], "2023-11-14T22:13:20Z");
        assert_eq!(value["updated_at"], "2023-11-14T22:15:00Z");
        assert_eq!(value["message_count"], 2);

        let bare = Session {
            created_at_ms: None,
            updated_at_ms: None,
            ..session
        };
        let value = serde_json::to_value(bare.summary()).unwrap();
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
        assert_eq!(object.len(), 10);
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
        let at = |id: &str, created_at_ms, updated_at_ms| Session {
            id: id.into(),
            created_at_ms,
            updated_at_ms,
            ..session()
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
        let shown: Vec<String> = merged.iter().map(Session::updated_display).collect();
        assert!(shown[..4].is_sorted_by(|a, b| a >= b), "{shown:?}");
    }

    #[test]
    fn a_time_that_cannot_be_shown_does_not_order_the_list() {
        // Microseconds where milliseconds belong: the year 57650.
        let micro = Session {
            id: "micro".into(),
            created_at_ms: Some(1_757_000_000_000),
            updated_at_ms: Some(1_757_000_000_000_000),
            ..session()
        };
        let newer = Session {
            id: "newer".into(),
            created_at_ms: Some(1_757_500_000_000),
            updated_at_ms: None,
            ..session()
        };
        let merged = merge_sessions(vec![micro, newer]);
        let ids: Vec<&str> = merged.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["newer", "micro"]);
        // It shows its creation time, as for a session never updated.
        assert_eq!(merged[1].updated_display(), "2025-09-04 15:33");
        assert_eq!(merged[1].updated_utc(), "2025-09-04 15:33 UTC");
        assert_eq!(
            serde_json::to_value(merged[1].summary()).unwrap()["updated_at"],
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
        let bare = Session {
            created_at_ms: None,
            ..session()
        };
        assert_eq!(bare.created_utc(), "—");
    }

    #[test]
    fn export_json_shape_is_unchanged() {
        let mut session = session();
        session.messages.clear();
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
