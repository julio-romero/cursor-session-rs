use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Agent,
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
