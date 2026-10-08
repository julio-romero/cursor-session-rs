//! The transcript `handoff` builds for pasting into another agent: a
//! preamble, a session's user and assistant messages cut short as
//! `show --short --only user,assistant` prints them, and a trailer with the
//! transcript's token estimate. A function of the messages alone, so that
//! the clipboard and stdout get the same text.

use crate::model::{Message, content_chars, token_estimate};
use crate::ui;
use crate::view::{Role, View};

/// The preamble a transcript starts with unless it is replaced or left out.
pub const DEFAULT_PREAMBLE: &str = "The following is a transcript from a Cursor session that ran \
     out of credits. Continue from where it ended; do not summarize it back.";

/// What a transcript holds besides its messages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandoffOptions {
    /// The paragraph before the messages; `None` leaves it out.
    pub preamble: Option<String>,
    /// Only the last N of the messages given.
    pub limit: Option<usize>,
}

impl Default for HandoffOptions {
    fn default() -> Self {
        Self {
            preamble: Some(DEFAULT_PREAMBLE.to_string()),
            limit: None,
        }
    }
}

/// A transcript and what its trailer says of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Handoff {
    /// The whole transcript, trailer included, without escape sequences.
    pub text: String,
    /// The messages in it.
    pub messages: usize,
    /// The tokens `text` is estimated to take: ceil(characters / 4),
    /// counting the trailer that states it.
    pub token_estimate: usize,
}

/// The messages a transcript is built from, and how they are read: those
/// `show --short --only user,assistant` prints. Tool calls and results are
/// never read.
pub fn view() -> View {
    View {
        only: vec![Role::User, Role::Assistant],
        short: true,
    }
}

/// The transcript of `messages`, which are those [`view`] keeps of a
/// session, in order: the preamble, each message under a `[role]` line (as
/// `show` prints it, without the time), then a trailer that counts the
/// messages and estimates the transcript's tokens. `opts.limit` keeps only
/// the last messages. Escape sequences and control characters other than
/// line breaks and tabs are removed from every part, the preamble too.
pub fn render_handoff(messages: &[Message], opts: &HandoffOptions) -> Handoff {
    let start = opts
        .limit
        .map_or(0, |limit| messages.len().saturating_sub(limit));
    let messages = &messages[start..];
    let mut text = String::new();
    if let Some(preamble) = &opts.preamble {
        text.push_str(&ui::plain_text(preamble));
        text.push_str("\n\n");
    }
    for message in messages {
        text.push('[');
        text.push_str(&ui::plain_text(&message.role));
        text.push_str("]\n");
        text.push_str(&ui::plain_text(&message.content));
        text.push_str("\n\n");
    }
    let tokens = trailer_tokens(content_chars(&text), messages.len());
    text.push_str(&trailer(messages.len(), tokens));
    Handoff {
        text,
        messages: messages.len(),
        token_estimate: tokens,
    }
}

/// The last line of a transcript of `messages` messages, which estimates its
/// `tokens`.
fn trailer(messages: usize, tokens: usize) -> String {
    format!(
        "[end of transcript: {}, ~{tokens} tokens (estimate)]\n",
        count(messages, "message")
    )
}

/// The token estimate of a transcript whose text before the trailer has
/// `chars` characters, the trailer that states it included. The trailer
/// grows only with the digits of the estimate, so starting from the text
/// alone the estimate settles within a few rounds.
fn trailer_tokens(chars: usize, messages: usize) -> usize {
    let mut tokens = token_estimate(chars);
    // Each round adds at most one digit; 20 cover any `usize`.
    for _ in 0..20 {
        let total = token_estimate(chars + content_chars(&trailer(messages, tokens)));
        if total == tokens {
            break;
        }
        tokens = total;
    }
    tokens
}

/// `n` and `noun`, plural unless `n` is 1.
pub fn count(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(role: &str, content: &str) -> Message {
        Message {
            role: role.into(),
            content: content.into(),
            timestamp: Some("Mon".into()),
        }
    }

    fn conversation() -> Vec<Message> {
        vec![
            message("user", "Where is langfuse configured?"),
            message("assistant", "In src/config.rs."),
            message("user", "Keep it read-only."),
        ]
    }

    fn estimate_of(text: &str) -> usize {
        text.chars().count().div_ceil(4)
    }

    #[test]
    fn preamble_messages_then_trailer() {
        let handoff = render_handoff(&conversation(), &HandoffOptions::default());
        let expected = format!(
            "{DEFAULT_PREAMBLE}\n\n\
             [user]\nWhere is langfuse configured?\n\n\
             [assistant]\nIn src/config.rs.\n\n\
             [user]\nKeep it read-only.\n\n\
             [end of transcript: 3 messages, ~{} tokens (estimate)]\n",
            handoff.token_estimate
        );
        assert_eq!(handoff.text, expected);
        assert_eq!(handoff.messages, 3);
        assert_eq!(handoff.token_estimate, estimate_of(&handoff.text));
        assert_eq!(
            DEFAULT_PREAMBLE,
            "The following is a transcript from a Cursor session that ran out of credits. \
             Continue from where it ended; do not summarize it back."
        );
    }

    #[test]
    fn preamble_can_be_replaced_or_left_out() {
        let custom = HandoffOptions {
            preamble: Some("Pick up the refactor.".into()),
            limit: None,
        };
        let handoff = render_handoff(&conversation(), &custom);
        assert!(
            handoff
                .text
                .starts_with("Pick up the refactor.\n\n[user]\nWhere is langfuse")
        );
        assert!(!handoff.text.contains(DEFAULT_PREAMBLE));

        let none = HandoffOptions {
            preamble: None,
            limit: None,
        };
        let handoff = render_handoff(&conversation(), &none);
        assert!(
            handoff
                .text
                .starts_with("[user]\nWhere is langfuse configured?\n\n")
        );
        assert_eq!(handoff.token_estimate, estimate_of(&handoff.text));
    }

    #[test]
    fn limit_keeps_the_last_messages() {
        let last = |limit| {
            render_handoff(
                &conversation(),
                &HandoffOptions {
                    limit: Some(limit),
                    ..HandoffOptions::default()
                },
            )
        };
        let one = last(1);
        assert_eq!(one.messages, 1);
        assert!(
            one.text.ends_with(&format!(
                "\n\n[user]\nKeep it read-only.\n\n\
                 [end of transcript: 1 message, ~{} tokens (estimate)]\n",
                one.token_estimate
            )),
            "{}",
            one.text
        );
        assert!(!one.text.contains("[assistant]"));
        let two = last(2);
        assert_eq!(two.messages, 2);
        assert!(
            two.text
                .contains("\n\n[assistant]\nIn src/config.rs.\n\n[user]\n")
        );
        assert_eq!(last(usize::MAX), last(3));
        assert_eq!(
            last(3),
            render_handoff(&conversation(), &HandoffOptions::default())
        );
    }

    #[test]
    fn the_trailer_estimates_the_whole_transcript() {
        // Around each length where the estimate gains a digit, which
        // lengthens the trailer that states it.
        for chars in (0..60).chain(3_960..4_060).chain(39_960..40_060) {
            let text = "x".repeat(chars);
            for preamble in [None, Some(text)] {
                let handoff = render_handoff(
                    &[],
                    &HandoffOptions {
                        preamble,
                        limit: None,
                    },
                );
                assert_eq!(
                    handoff.token_estimate,
                    estimate_of(&handoff.text),
                    "{chars}"
                );
                assert!(handoff.text.ends_with(&format!(
                    "[end of transcript: 0 messages, ~{} tokens (estimate)]\n",
                    handoff.token_estimate
                )));
            }
        }
        // Characters are Unicode scalar values, as for the session estimate.
        let handoff = render_handoff(
            &[message("user", &"é".repeat(400))],
            &HandoffOptions::default(),
        );
        assert_eq!(handoff.token_estimate, estimate_of(&handoff.text));
        assert!(handoff.token_estimate < handoff.text.len().div_ceil(4));
    }

    #[test]
    fn no_escape_sequence_reaches_the_transcript() {
        let messages = [
            message(
                "user",
                "copy \u{1b}]52;c;aGk=\u{7} bell \u{7} clear \u{1b}[2J done\r\nnext",
            ),
            message(
                "assistant",
                "csi \u{9b}2J here, DEL \u{7f} \u{1b}[31mred\u{1b}[0m",
            ),
        ];
        let opts = HandoffOptions {
            preamble: Some("Go \u{1b}[41mon\u{1b}[0m.".into()),
            limit: None,
        };
        let handoff = render_handoff(&messages, &opts);
        assert!(
            !handoff
                .text
                .chars()
                .any(|c| c.is_control() && !matches!(c, '\n' | '\r')),
            "{:?}",
            handoff.text
        );
        assert!(handoff.text.starts_with(
            "Go on.\n\n[user]\ncopy  bell  clear  done\r\nnext\n\n\
             [assistant]\ncsi  here, DEL  red\n\n"
        ));
        assert_eq!(handoff.token_estimate, estimate_of(&handoff.text));
    }

    #[test]
    fn the_view_is_short_user_and_assistant_messages() {
        let view = view();
        assert!(view.short);
        assert!(!view.read_options().tools);
        let kept = view.apply(vec![
            message("user", "q"),
            message("tool", "Grep {}"),
            message("unknown", "?"),
            message("assistant", &"a".repeat(301)),
        ]);
        let roles: Vec<&str> = kept.iter().map(|m| m.role.as_str()).collect();
        assert_eq!(roles, ["user", "assistant"]);
        assert_eq!(kept[1].content, format!("{}…", "a".repeat(300)));
    }

    #[test]
    fn counts_are_plural_unless_one() {
        assert_eq!(count(0, "message"), "0 messages");
        assert_eq!(count(1, "message"), "1 message");
        assert_eq!(count(2, "message"), "2 messages");
    }
}
