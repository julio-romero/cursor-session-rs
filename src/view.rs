//! Which of a session's messages to print, and how much of each: what
//! `show --only` and `show --short` do, as functions of the messages alone so
//! that other views can build the same text.

use std::borrow::Cow;

use crate::ReadOptions;
use crate::model::{Message, TOOL_ROLE};
use crate::ui;

/// Characters (Unicode scalar values) of a message that `--short` keeps.
pub const SHORT_CHARS: usize = 300;
/// Characters of a tool message's one-line preview that `--short` keeps.
pub const TOOL_PREVIEW_CHARS: usize = 120;

/// A role `--only` selects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, clap::ValueEnum)]
pub enum Role {
    /// What the user wrote
    User,
    /// The model's replies
    Assistant,
    /// Tool calls and their results
    Tool,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Tool => TOOL_ROLE,
        }
    }
}

/// Which messages to print and how.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct View {
    /// Only messages of these roles; empty for those `show` prints by
    /// default, every role but tool calls and results.
    pub only: Vec<Role>,
    /// Cut each message short (see [`shorten`]).
    pub short: bool,
}

impl View {
    /// How to read the messages this view prints: tool messages are built
    /// only when it selects them.
    pub fn read_options(&self) -> ReadOptions {
        ReadOptions {
            tools: self.only.contains(&Role::Tool),
        }
    }

    /// Whether this view prints a message of `role`. Without `only`, that is
    /// every role but [`TOOL_ROLE`], also `unknown`; with it, only the roles
    /// it names.
    pub fn shows(&self, role: &str) -> bool {
        if self.only.is_empty() {
            return role != TOOL_ROLE;
        }
        self.only.iter().any(|wanted| wanted.as_str() == role)
    }

    /// The messages this view prints, in order, cut short if it says so.
    pub fn apply(&self, messages: Vec<Message>) -> Vec<Message> {
        messages
            .into_iter()
            .filter(|message| self.shows(&message.role))
            .map(|message| {
                if self.short {
                    shorten(message)
                } else {
                    message
                }
            })
            .collect()
    }
}

/// `message` cut short: its first [`SHORT_CHARS`] characters, then `…` when
/// there were more; a tool message as a one-line preview (see
/// [`preview`]).
pub fn shorten(mut message: Message) -> Message {
    let short = if message.role == TOOL_ROLE {
        Cow::Owned(preview(&message.content))
    } else {
        cut(&message.content, SHORT_CHARS)
    };
    if let Cow::Owned(short) = short {
        message.content = short;
    }
    message
}

/// `text` on one line, without escape sequences and with each run of
/// whitespace a single space, cut to its first [`TOOL_PREVIEW_CHARS`]
/// characters, then `…` when there were more.
pub fn preview(text: &str) -> String {
    let line = ui::one_line(text);
    let line = line.split_whitespace().collect::<Vec<_>>().join(" ");
    match line.char_indices().nth(TOOL_PREVIEW_CHARS) {
        None => line,
        Some((end, _)) => format!("{}…", line[..end].trim_end()),
    }
}

/// The first `max` characters (Unicode scalar values) of `text`, followed by
/// `…` when there were more; `text` itself when there were not.
pub fn cut(text: &str, max: usize) -> Cow<'_, str> {
    match text.char_indices().nth(max) {
        None => Cow::Borrowed(text),
        Some((end, _)) => Cow::Owned(format!("{}…", &text[..end])),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(role: &str, content: &str) -> Message {
        Message {
            role: role.into(),
            content: content.into(),
            timestamp: Some("t".into()),
        }
    }

    fn roles(messages: &[Message]) -> Vec<&str> {
        messages.iter().map(|m| m.role.as_str()).collect()
    }

    fn conversation() -> Vec<Message> {
        vec![
            message("user", "q"),
            message("assistant", "a"),
            message("tool", "Grep {}"),
            message("tool", "found"),
            message("unknown", "?"),
            message("assistant", "b"),
        ]
    }

    #[test]
    fn only_keeps_the_roles_named_in_order() {
        let view = |only: &[Role]| View {
            only: only.to_vec(),
            short: false,
        };
        assert_eq!(
            roles(&View::default().apply(conversation())),
            ["user", "assistant", "unknown", "assistant"]
        );
        assert_eq!(
            roles(&view(&[Role::Assistant]).apply(conversation())),
            ["assistant", "assistant"]
        );
        assert_eq!(
            roles(&view(&[Role::Tool, Role::User]).apply(conversation())),
            ["user", "tool", "tool"]
        );
        // `unknown` shows only without --only.
        assert_eq!(
            roles(&view(&[Role::User, Role::Assistant, Role::Tool]).apply(conversation())),
            ["user", "assistant", "tool", "tool", "assistant"]
        );
        assert!(view(&[Role::Tool]).read_options().tools);
        assert!(!view(&[Role::User, Role::Assistant]).read_options().tools);
        assert!(!View::default().read_options().tools);
    }

    #[test]
    fn short_cuts_at_300_characters() {
        let at = |n: usize| "é".repeat(n);
        let shortened = |text: &str| shorten(message("user", text)).content;
        assert_eq!(shortened(&at(299)), at(299));
        assert_eq!(shortened(&at(300)), at(300));
        assert_eq!(shortened(&at(301)), format!("{}…", at(300)));
        assert_eq!(shortened(""), "");
        // Characters are Unicode scalar values: an emoji of several is cut
        // between them, never inside one.
        let family = "👨\u{200d}👩\u{200d}👧";
        let text = format!("{}{family}", "x".repeat(298));
        assert_eq!(shortened(&text), format!("{}👨\u{200d}…", "x".repeat(298)));
        // The rest of the message stays.
        let short = shorten(message("assistant", &"a".repeat(400)));
        assert_eq!(short.role, "assistant");
        assert_eq!(short.timestamp.as_deref(), Some("t"));
        assert_eq!(short.content.chars().count(), 301);
        // Line breaks stay in a message that is not a tool's.
        assert_eq!(shortened("a\nb"), "a\nb");
    }

    #[test]
    fn short_tool_messages_are_a_one_line_preview() {
        let tool = |text: &str| shorten(message("tool", text)).content;
        assert_eq!(tool("line one\n  line\ttwo\r\n"), "line one line two");
        assert_eq!(tool("\u{1b}[31mred\u{1b}[0m"), "red");
        let long = "ü".repeat(500);
        assert_eq!(tool(&long), format!("{}…", "ü".repeat(TOOL_PREVIEW_CHARS)));
        assert_eq!(tool(&"x".repeat(120)), "x".repeat(120));
        // A cut does not leave a space before the ellipsis.
        let words = "word ".repeat(30);
        assert_eq!(tool(&words), format!("{}…", "word ".repeat(24).trim_end()));
    }

    #[test]
    fn short_view_shortens_what_it_keeps() {
        let view = View {
            only: vec![Role::Tool],
            short: true,
        };
        let mut messages = conversation();
        messages[3].content = "a\nb".into();
        let shown = view.apply(messages);
        assert_eq!(roles(&shown), ["tool", "tool"]);
        assert_eq!(shown[1].content, "a b");
    }

    #[test]
    fn cut_borrows_what_it_keeps_whole() {
        assert!(matches!(cut("abc", 3), Cow::Borrowed("abc")));
        assert_eq!(cut("abcd", 3), "abc…");
        assert_eq!(cut("abc", 0), "…");
        assert_eq!(cut("", 0), "");
    }
}
