//! Tool calls and results as messages of the role [`TOOL_ROLE`], which the
//! readers build only when asked to (see [`crate::ReadOptions`]). Both stores
//! keep them as JSON of their own; this renders them as readable text.

use serde_json::Value;

use crate::model::{Message, TOOL_ROLE};

/// A tool message with `content`, or `None` when it has no text.
pub(crate) fn message(content: String) -> Option<Message> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return None;
    }
    let content = if trimmed.len() == content.len() {
        content
    } else {
        trimmed.to_string()
    };
    Some(Message {
        role: TOOL_ROLE.to_string(),
        content,
        timestamp: None,
    })
}

/// A tool call: its name, then its arguments as compact JSON, such as
/// `Grep {"pattern":"langfuse"}`. Arguments stored as a JSON string, as the
/// IDE stores them, are parsed first; text that is not JSON is kept as it is.
pub(crate) fn call(name: Option<&str>, args: Option<&Value>) -> String {
    let name = name.map(str::trim).filter(|name| !name.is_empty());
    let name = name.unwrap_or("tool");
    let args = args.and_then(|args| match args {
        Value::Null => None,
        Value::String(text) => Some(match crate::json::from_str::<Value>(text) {
            Ok(parsed @ (Value::Object(_) | Value::Array(_))) => parsed.to_string(),
            _ => text.trim().to_string(),
        }),
        other => Some(other.to_string()),
    });
    match args {
        Some(args) if !args.is_empty() => format!("{name} {args}"),
        _ => name.to_string(),
    }
}

/// What a tool returned, as text: a string as it is, text parts joined with
/// blank lines, and anything else as compact JSON. `None` for nothing.
pub(crate) fn result(value: &Value) -> Option<String> {
    let text = match value {
        Value::Null => return None,
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|part| match part {
                Value::String(text) => Some(text.clone()),
                Value::Object(fields) => match fields.get("text") {
                    Some(Value::String(text)) => Some(text.clone()),
                    _ => Some(part.to_string()),
                },
                Value::Null => None,
                other => Some(other.to_string()),
            })
            .collect::<Vec<_>>()
            .join("\n\n"),
        other => other.to_string(),
    };
    (!text.trim().is_empty()).then_some(text)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn calls_show_their_name_and_compact_arguments() {
        assert_eq!(
            call(
                Some("Grep"),
                Some(&json!({"pattern": "langfuse", "path": "src"}))
            ),
            r#"Grep {"pattern":"langfuse","path":"src"}"#
        );
        // As the IDE stores them: JSON in a string, maybe pretty-printed.
        assert_eq!(
            call(
                Some("read_file"),
                Some(&json!("{\n  \"target_file\": \"a.rs\"\n}"))
            ),
            r#"read_file {"target_file":"a.rs"}"#
        );
        assert_eq!(
            call(Some("Shell"), Some(&json!(" ls -la "))),
            "Shell ls -la"
        );
        assert_eq!(call(Some("Read"), None), "Read");
        assert_eq!(call(Some("Read"), Some(&Value::Null)), "Read");
        assert_eq!(call(Some(" "), Some(&json!({}))), "tool {}");
        assert_eq!(call(None, Some(&json!([1, 2]))), "tool [1,2]");
    }

    #[test]
    fn results_are_text() {
        assert_eq!(result(&json!("found 3")).as_deref(), Some("found 3"));
        assert_eq!(
            result(&json!([{"type": "text", "text": "a"}, "b", {"x": 1}, null])).as_deref(),
            Some("a\n\nb\n\n{\"x\":1}")
        );
        assert_eq!(
            result(&json!({"ok": true})).as_deref(),
            Some(r#"{"ok":true}"#)
        );
        assert_eq!(result(&json!(7)).as_deref(), Some("7"));
        for nothing in [json!(null), json!(""), json!(" \n"), json!([])] {
            assert_eq!(result(&nothing), None, "{nothing}");
        }
    }

    #[test]
    fn tool_messages_have_text() {
        let built = message("  out\n".to_string()).unwrap();
        assert_eq!(
            (built.role.as_str(), built.content.as_str(), built.timestamp),
            ("tool", "out", None)
        );
        assert!(message(" \n\t".to_string()).is_none());
    }
}
