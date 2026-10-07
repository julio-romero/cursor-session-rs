use chrono::{DateTime, Datelike, SecondsFormat};
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
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
        format_ms(self.updated_at_ms.or(self.created_at_ms))
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

fn rfc3339(ms: Option<i64>) -> Option<String> {
    let dt = DateTime::from_timestamp_millis(ms?)?;
    // RFC 3339 has no form for years outside 0000-9999.
    (0..=9999)
        .contains(&dt.year())
        .then(|| dt.to_rfc3339_opts(SecondsFormat::Secs, true))
}

fn format_ms(ms: Option<i64>) -> String {
    let Some(ms) = ms else {
        return "—".to_string();
    };
    let secs = ms.div_euclid(1000);
    let nsecs = (ms.rem_euclid(1000) * 1_000_000) as u32;
    match chrono::DateTime::from_timestamp(secs, nsecs) {
        Some(dt) => dt.format("%Y-%m-%d %H:%M").to_string(),
        None => ms.to_string(),
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
    merged.sort_by(|a, b| b.updated_at_ms.cmp(&a.updated_at_ms).then(a.id.cmp(&b.id)));
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
