use std::collections::HashMap;
use std::path::Path;

use serde::Deserialize;
use serde_json::Value;

use crate::detect::StoragePaths;
use crate::model::{Message, Session, Source};
use crate::sqlite::open_readonly;
use crate::{Error, Result};

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

pub fn load_from_db(db_path: &Path, _warnings: &mut Vec<String>) -> Result<Vec<Session>> {
    let conn = match open_readonly(db_path) {
        Ok(conn) => conn,
        Err(_) => return Ok(Vec::new()),
    };
    let db_err = |source| Error::Database {
        path: db_path.to_path_buf(),
        source,
    };

    let mut bubble_map: HashMap<String, Bubble> = HashMap::new();
    {
        let mut stmt = conn
            .prepare(
                "SELECT key, value FROM cursorDiskKV WHERE key LIKE 'bubbleId:%' AND value IS NOT NULL",
            )
            .map_err(db_err)?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(db_err)?;
        for row in rows.flatten() {
            let (key, value) = row;
            if let Ok(bubble) = serde_json::from_str::<Bubble>(&value) {
                let id = bubble
                    .bubble_id
                    .clone()
                    .or_else(|| key.rsplit(':').next().map(str::to_string));
                if let Some(id) = id {
                    bubble_map.insert(id, bubble);
                }
            }
        }
    }

    let mut sessions = Vec::new();
    {
        let mut stmt = conn
            .prepare(
                "SELECT key, value FROM cursorDiskKV WHERE key LIKE 'composerData:%' AND value IS NOT NULL",
            )
            .map_err(db_err)?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(db_err)?;
        for row in rows.flatten() {
            let (key, value) = row;
            let Ok(composer) = serde_json::from_str::<Composer>(&value) else {
                continue;
            };
            let id = composer
                .composer_id
                .clone()
                .or_else(|| key.strip_prefix("composerData:").map(str::to_string));
            let Some(id) = id else {
                continue;
            };
            let mut messages = Vec::new();
            for header in &composer.full_conversation_headers_only {
                let Some(bubble_id) = &header.bubble_id else {
                    continue;
                };
                let Some(bubble) = bubble_map.get(bubble_id) else {
                    continue;
                };
                let content = extract_bubble_text(bubble);
                if content.is_empty() {
                    continue;
                }
                let role = match header.kind.or(bubble.kind).unwrap_or(1) {
                    1 => "user",
                    2 => "assistant",
                    _ => "assistant",
                };
                messages.push(Message {
                    role: role.to_string(),
                    content,
                    timestamp: bubble.timestamp.map(|ms| ms.to_string()),
                });
            }
            let title = composer
                .name
                .clone()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "Untitled".to_string());
            sessions.push(Session {
                id,
                title,
                source: Source::Ide,
                workspace: None,
                workspace_hash: None,
                created_at_ms: composer.created_at,
                updated_at_ms: composer.last_updated_at.or(composer.created_at),
                model: None,
                messages,
            });
        }
    }
    Ok(sessions)
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
