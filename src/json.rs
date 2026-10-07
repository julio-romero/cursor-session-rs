use serde::de::DeserializeOwned;

/// Parses JSON as Cursor writes it. JavaScript writes a string cut inside an
/// emoji with a lone surrogate escape (`"\ud83d"`), which serde_json rejects;
/// such an escape stands for U+FFFD instead, so it costs that character
/// rather than the whole chat, message or line.
pub(crate) fn from_str<T: DeserializeOwned>(json: &str) -> serde_json::Result<T> {
    serde_json::from_str(json).or_else(|err| match replace_lone_surrogates(json) {
        Some(repaired) => serde_json::from_str(&repaired),
        None => Err(err),
    })
}

/// `json` with each `\uXXXX` escape of a surrogate that is not part of a pair
/// replaced by `\ufffd`; `None` when there is none.
fn replace_lone_surrogates(json: &str) -> Option<String> {
    let bytes = json.as_bytes();
    let mut repaired = String::new();
    let mut copied = 0;
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] != b'\\' {
            at += 1;
            continue;
        }
        // Another escape, such as `\\` or `\"`, is two bytes long.
        let Some(unit) = escaped_unit(bytes, at) else {
            at += 2;
            continue;
        };
        let high = (0xD800..0xDC00).contains(&unit);
        let low = |unit: u16| (0xDC00..0xE000).contains(&unit);
        if high && escaped_unit(bytes, at + 6).is_some_and(low) {
            at += 12;
            continue;
        }
        if high || low(unit) {
            repaired.push_str(&json[copied..at]);
            repaired.push_str("\\ufffd");
            copied = at + 6;
        }
        at += 6;
    }
    if copied == 0 {
        return None;
    }
    repaired.push_str(&json[copied..]);
    Some(repaired)
}

/// The code unit of the `\uXXXX` escape at `at`.
fn escaped_unit(bytes: &[u8], at: usize) -> Option<u16> {
    let hex = bytes.get(at..at + 6)?.strip_prefix(b"\\u")?;
    if !hex.iter().all(u8::is_ascii_hexdigit) {
        return None;
    }
    u16::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok()
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::*;

    /// A JSON string of `parts`, each text or a code unit to escape as `\uXXXX`.
    fn json_string(parts: &[Part]) -> String {
        let mut json = String::from('"');
        for part in parts {
            match part {
                Part::Text(text) => json.push_str(text),
                Part::Unit(unit) => json.push_str(&format!("\\u{unit:04x}")),
            }
        }
        json.push('"');
        json
    }

    enum Part {
        Text(&'static str),
        Unit(u16),
    }

    #[test]
    fn lone_surrogates_become_replacement_characters() {
        use Part::{Text, Unit};
        let (high, low) = (0xD83D, 0xDE00);
        let parsed = |parts: &[Part]| from_str::<Value>(&json_string(parts)).unwrap();
        assert_eq!(parsed(&[Text("cut "), Unit(high)]), "cut \u{fffd}");
        assert_eq!(
            parsed(&[Text("cut "), Unit(low), Text(" here")]),
            "cut \u{fffd} here"
        );
        assert_eq!(
            parsed(&[Unit(high), Unit(high), Unit(low)]),
            "\u{fffd}\u{1f600}"
        );
        // Pairs, escaped backslashes and other escapes stay as they are.
        assert_eq!(
            parsed(&[
                Unit(high),
                Unit(low),
                Text(" \\\\ud83d \\n "),
                Unit(0xE9),
                Unit(low)
            ]),
            "\u{1f600} \\ud83d \n \u{e9}\u{fffd}"
        );
        let object = format!(r#"{{"text":{},"n":1}}"#, json_string(&[Unit(high)]));
        assert_eq!(
            from_str::<Value>(&object).unwrap(),
            serde_json::json!({"text": "\u{fffd}", "n": 1})
        );
        assert!(from_str::<Value>(&object[..object.len() - 6]).is_err());
        assert!(from_str::<Value>("{not json").is_err());
        assert_eq!(replace_lone_surrogates(&json_string(&[Unit(0xE9)])), None);
    }
}
