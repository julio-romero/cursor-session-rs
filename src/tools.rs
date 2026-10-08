//! Tool calls and results as messages of the role [`TOOL_ROLE`], which the
//! readers build only when asked to (see [`crate::ReadOptions`]). Both stores
//! keep them as JSON of their own; this renders them as readable text.

use serde_json::{Map, Value};

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

/// The marker of a tool call that ended as `status` says, when that was not
/// a success: `error` for one that failed, `cancelled` for one stopped, and
/// any other status that is a word as it is, such as `rejected`. `None` for a
/// call that succeeded, or a status that is no word.
pub(crate) fn status_marker(status: &str) -> Option<String> {
    let status = status.trim().to_ascii_lowercase();
    match status.as_str() {
        "" | "completed" | "complete" | "success" | "succeeded" | "successful" | "done" | "ok"
        | "finished" => None,
        "error" | "errored" | "failed" | "failure" | "fail" => Some("error".to_string()),
        "cancelled" | "canceled" | "aborted" => Some("cancelled".to_string()),
        other => is_marker_word(other).then(|| other.to_string()),
    }
}

/// The longest status marker.
const MARKER_MAX_CHARS: usize = 24;

/// Whether `word` can be a status marker: a few lowercase letters, digits,
/// `_` or `-`.
fn is_marker_word(word: &str) -> bool {
    !word.is_empty()
        && word.len() <= MARKER_MAX_CHARS
        && word
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'-'))
}

/// The text of a tool message of a call that ended as `marker` says, such as
/// `not found (error)`: `text` followed by the marker in parentheses, or the
/// marker alone without text.
pub(crate) fn marked(text: Option<String>, marker: &str) -> String {
    match text {
        Some(text) if !text.trim().is_empty() => format!("{} ({marker})", text.trim_end()),
        _ => format!("({marker})"),
    }
}

/// A tool message's `content` split into its text and the status marker it
/// ends with (see [`marked`]), such as `("not found", Some("(error)"))`.
pub(crate) fn split_marker(content: &str) -> (&str, Option<&str>) {
    let Some(open) = content
        .strip_suffix(')')
        .and_then(|inside| inside.rfind('('))
    else {
        return (content, None);
    };
    let word = &content[open + 1..content.len() - 1];
    let text = &content[..open];
    if !is_marker_word(word) {
        return (content, None);
    }
    match text.strip_suffix(' ') {
        Some(text) => (text, Some(&content[open..])),
        None if text.is_empty() => (text, Some(content)),
        None => (content, None),
    }
}

/// What a tool returned, as text, or `None` for nothing:
///
/// - a string as it is, unless it holds a JSON object, as the IDE stores
///   results; that is shown as an object is;
/// - parts (an array) each as text, joined with blank lines;
/// - an object by its text (see [`TEXT_FIELDS`]), or else as indented JSON;
/// - images and binary data as a placeholder such as `[image]`, never their
///   bytes.
pub(crate) fn result(value: &Value) -> Option<String> {
    let text = match value {
        Value::Null => return None,
        Value::String(text) => {
            let parsed = text
                .trim_start()
                .starts_with('{')
                .then(|| crate::json::from_str::<Value>(text).ok())
                .flatten();
            match parsed {
                Some(Value::Object(fields)) => object(&fields),
                _ => part_text(value),
            }
        }
        Value::Array(parts) => parts
            .iter()
            .filter_map(|part| match part {
                Value::Null => None,
                Value::Array(_) => result(part),
                other => Some(part_text(other)),
            })
            .collect::<Vec<_>>()
            .join("\n\n"),
        other => part_text(other),
    };
    (!text.trim().is_empty()).then_some(text)
}

/// Fields that hold the text of a result object, such as the output of a
/// terminal command (`output`) or the file read (`contents`), first first.
const TEXT_FIELDS: [&str; 8] = [
    "output", "stdout", "contents", "content", "text", "result", "message", "error",
];

/// What stands for binary data.
const BINARY: &str = "[binary data]";

/// One part of a result as text.
fn part_text(part: &Value) -> String {
    match part {
        Value::String(text) if looks_binary(text) => BINARY.to_string(),
        Value::String(text) => text.clone(),
        Value::Object(fields) => object(fields),
        other => other.to_string(),
    }
}

/// An object of a result as text: a placeholder for an image or other
/// binary content, the first of its [`TEXT_FIELDS`] with text, or else the
/// whole object as indented JSON with binary data left out.
fn object(fields: &Map<String, Value>) -> String {
    if let Some(placeholder) = hidden(fields) {
        return placeholder;
    }
    for field in TEXT_FIELDS {
        let text = match fields.get(field) {
            Some(text @ Value::String(_)) => part_text(text),
            Some(parts @ Value::Array(_)) => result(parts).unwrap_or_default(),
            _ => continue,
        };
        if !text.trim().is_empty() {
            return text;
        }
    }
    let shown = Value::Object(fields.clone());
    serde_json::to_string_pretty(&without_binary(&shown)).unwrap_or_else(|_| shown.to_string())
}

/// The placeholder for a content part that is never shown as text: an image,
/// or another kind of binary content.
fn hidden(fields: &Map<String, Value>) -> Option<String> {
    let kind = fields.get("type").and_then(Value::as_str).unwrap_or("");
    if kind == "image" || kind == "image_url" {
        return Some("[image]".to_string());
    }
    if matches!(kind, "audio" | "document" | "file" | "blob") {
        return Some(format!("[{kind}]"));
    }
    None
}

/// `value` with images and binary data, at any depth, as placeholders.
fn without_binary(value: &Value) -> Value {
    match value {
        Value::String(text) if looks_binary(text) => Value::String(BINARY.to_string()),
        Value::Array(items) => Value::Array(items.iter().map(without_binary).collect()),
        Value::Object(fields) => match hidden(fields) {
            Some(placeholder) => Value::String(placeholder),
            None => Value::Object(
                fields
                    .iter()
                    .map(|(key, value)| (key.clone(), without_binary(value)))
                    .collect(),
            ),
        },
        other => other.clone(),
    }
}

/// Characters of base64 from which a string is taken for binary data.
const BINARY_MIN_CHARS: usize = 256;

/// Whether `text` looks like encoded binary data rather than text: a `data:`
/// URL, or a long run of base64 without a space.
fn looks_binary(text: &str) -> bool {
    let text = text.trim();
    if text.starts_with("data:")
        && text
            .split_once(',')
            .is_some_and(|(head, _)| head.ends_with(";base64"))
    {
        return true;
    }
    text.len() >= BINARY_MIN_CHARS
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'=' | b'-' | b'_'))
        // Words of letters alone, such as a long identifier, are text.
        && text.bytes().any(|b| b.is_ascii_digit() || matches!(b, b'+' | b'/'))
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
            Some("a\n\nb\n\n{\n  \"x\": 1\n}")
        );
        assert_eq!(
            result(&json!({"ok": true})).as_deref(),
            Some("{\n  \"ok\": true\n}")
        );
        assert_eq!(result(&json!(7)).as_deref(), Some("7"));
        for nothing in [json!(null), json!(""), json!(" \n"), json!([])] {
            assert_eq!(result(&nothing), None, "{nothing}");
        }
    }

    /// Results as the IDE stores them: JSON in a string.
    #[test]
    fn stored_result_objects_show_their_text() {
        let stored = |value: Value| result(&Value::String(value.to_string()));
        // run_terminal_cmd
        assert_eq!(
            stored(json!({"output": "a\nb\n", "exitCodeV2": 0, "rejected": false})).as_deref(),
            Some("a\nb\n")
        );
        // read_file
        assert_eq!(
            stored(json!({"contents": "fn main() {}", "didDowngradeToLineRange": false}))
                .as_deref(),
            Some("fn main() {}")
        );
        assert_eq!(stored(json!({"stdout": "ok"})).as_deref(), Some("ok"));
        // MCP: text parts under `content`.
        assert_eq!(
            stored(
                json!({"content": [{"type": "text", "text": "a"}, {"type": "text", "text": "b"}]})
            )
            .as_deref(),
            Some("a\n\nb")
        );
        // An empty text field is passed over for the next one.
        assert_eq!(
            stored(json!({"output": "", "error": "denied"})).as_deref(),
            Some("denied")
        );
        // Without one, the object is indented JSON.
        assert_eq!(
            stored(json!({"files": ["Cargo.toml", "src"]})).as_deref(),
            Some("{\n  \"files\": [\n    \"Cargo.toml\",\n    \"src\"\n  ]\n}")
        );
        // A string that is no JSON object stays as it is.
        for text in ["plain text", "[1, 2]", "{not json", " 42"] {
            assert_eq!(result(&json!(text)).as_deref(), Some(text));
        }
    }

    #[test]
    fn images_and_binary_data_are_placeholders() {
        let base64 = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg=="
            .repeat(10);
        let image = json!({"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": base64}});
        let parts = json!([{"type": "text", "text": "Screenshot:"}, image]);
        let shown = result(&parts).unwrap();
        assert_eq!(shown, "Screenshot:\n\n[image]");
        let shortened = crate::view::preview(&shown);
        assert_eq!(shortened, "Screenshot: [image]");
        // Nested in an object without text, or a bare string of it.
        let nested = result(&json!({"screenshot": {"data": base64}, "ok": true})).unwrap();
        assert!(
            nested.contains("[binary data]") && !nested.contains("iVBOR"),
            "{nested}"
        );
        assert_eq!(result(&json!(base64)).as_deref(), Some("[binary data]"));
        let url = format!("data:image/png;base64,{base64}");
        assert_eq!(result(&json!([url])).as_deref(), Some("[binary data]"));
        assert_eq!(
            result(&json!([{"type": "image_url"}])).as_deref(),
            Some("[image]")
        );
        // Long text that is not base64 stays.
        let words = "word ".repeat(100);
        assert_eq!(result(&json!(words)).as_deref(), Some(words.as_str()));
        let long_name = "a".repeat(300);
        assert_eq!(
            result(&json!(long_name)).as_deref(),
            Some(long_name.as_str())
        );
    }

    #[test]
    fn failed_calls_are_marked() {
        assert_eq!(status_marker("completed"), None);
        assert_eq!(status_marker(" Success "), None);
        assert_eq!(status_marker(""), None);
        assert_eq!(status_marker("error").as_deref(), Some("error"));
        assert_eq!(status_marker("FAILED").as_deref(), Some("error"));
        assert_eq!(status_marker("canceled").as_deref(), Some("cancelled"));
        assert_eq!(status_marker("rejected").as_deref(), Some("rejected"));
        assert_eq!(status_marker("not a (word)"), None);
        assert_eq!(marked(Some("gone\n".into()), "error"), "gone (error)");
        assert_eq!(marked(None, "cancelled"), "(cancelled)");
        assert_eq!(marked(Some(" ".into()), "error"), "(error)");
        assert_eq!(split_marker("gone (error)"), ("gone", Some("(error)")));
        assert_eq!(split_marker("(cancelled)"), ("", Some("(cancelled)")));
        for unmarked in ["gone", "f(x)", "a (two words)", "a (Error)", "()", "a ()"] {
            assert_eq!(split_marker(unmarked), (unmarked, None), "{unmarked}");
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
