//! `search`: finding the sessions whose messages hold every term of a query,
//! and ranking them. Everything here is pure; the caller reads the messages
//! and feeds them in one at a time (see [`crate::search_session`]).

use std::cmp::Ordering;
use std::ops::{BitOr, Range};

use owo_colors::OwoColorize;
use regex::{Regex, RegexBuilder, RegexSet, RegexSetBuilder};
use serde::Serialize;

use crate::model::{Message, SessionSummary, Source};
use crate::ui;

/// The most terms a query may have, one per bit of a [`TermMask`].
pub const MAX_TERMS: usize = 64;

/// Characters of a message shown on each side of its first match.
pub const DEFAULT_CONTEXT: usize = 60;

/// Width of the source in a result's first line, the longest source name.
const SOURCE_WIDTH: usize = 5;
/// Width of the updated time in a result's first line.
const UPDATED_WIDTH: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum QueryError {
    #[error("the query has no terms")]
    Empty,
    #[error("the query has {0} terms; at most 64 are allowed")]
    TooManyTerms(usize),
    #[error("the query is too long")]
    TooLong,
}

/// A parsed query: its terms, and the matchers built from them. Every term is
/// a literal, matched case-insensitively with Unicode case folding.
#[derive(Debug, Clone)]
pub struct Query {
    terms: Vec<String>,
    /// One pattern per term, in order, so that a match's index is its bit.
    set: RegexSet,
    /// Any term, the longest first, to find what to highlight.
    any: Regex,
}

impl Query {
    pub fn terms(&self) -> &[String] {
        &self.terms
    }

    /// The matcher [`matches`] takes.
    pub fn set(&self) -> &RegexSet {
        &self.set
    }
}

/// The terms of `text`: its words, split on Unicode whitespace, and its
/// double-quoted phrases, each one term with the whitespace inside it kept.
/// A quote mark always starts or ends a phrase, also inside a word, and a
/// phrase left open runs to the end. Terms of only whitespace are dropped.
pub fn parse_terms(text: &str) -> Vec<String> {
    let mut terms = Vec::new();
    let mut term = String::new();
    let mut quoted = false;
    let mut end_term = |term: &mut String| {
        let trimmed = term.trim();
        if !trimmed.is_empty() {
            terms.push(trimmed.to_string());
        }
        term.clear();
    };
    for c in text.chars() {
        if c == '"' {
            end_term(&mut term);
            quoted = !quoted;
        } else if c.is_whitespace() && !quoted {
            end_term(&mut term);
        } else {
            term.push(c);
        }
    }
    end_term(&mut term);
    terms
}

/// Parses a query (see [`parse_terms`]) and builds its matchers: one
/// [`RegexSet`] of the escaped terms, case-insensitive. A query must have
/// from 1 to [`MAX_TERMS`] terms.
pub fn parse_query(text: &str) -> Result<Query, QueryError> {
    let terms = parse_terms(text);
    if terms.is_empty() {
        return Err(QueryError::Empty);
    }
    if terms.len() > MAX_TERMS {
        return Err(QueryError::TooManyTerms(terms.len()));
    }
    let patterns: Vec<String> = terms.iter().map(|term| regex::escape(term)).collect();
    let set = RegexSetBuilder::new(&patterns)
        .case_insensitive(true)
        .build()
        .map_err(|_| QueryError::TooLong)?;
    // Alternation takes the first term that matches at a position, so a
    // longer term is tried before a shorter one it starts with.
    let mut longest_first: Vec<&str> = patterns.iter().map(String::as_str).collect();
    longest_first.sort_by_key(|pattern| std::cmp::Reverse(pattern.len()));
    let any = RegexBuilder::new(&longest_first.join("|"))
        .case_insensitive(true)
        .build()
        .map_err(|_| QueryError::TooLong)?;
    Ok(Query { terms, set, any })
}

/// Which terms of a query a text holds: bit `i` for the query's term `i`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct TermMask(pub u64);

impl TermMask {
    /// The mask of every one of `terms` terms.
    pub fn all(terms: usize) -> TermMask {
        match terms {
            0 => TermMask(0),
            n if n >= MAX_TERMS => TermMask(u64::MAX),
            n => TermMask((1 << n) - 1),
        }
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// How many distinct terms it holds.
    pub fn count(self) -> u32 {
        self.0.count_ones()
    }

    /// Whether it holds every term `other` holds.
    pub fn covers(self, other: TermMask) -> bool {
        self.0 & other.0 == other.0
    }
}

impl BitOr for TermMask {
    type Output = TermMask;

    fn bitor(self, other: TermMask) -> TermMask {
        TermMask(self.0 | other.0)
    }
}

/// The terms of `set` that `text` holds, found in one pass over it.
pub fn matches(set: &RegexSet, text: &str) -> TermMask {
    set.matches(text)
        .iter()
        .filter(|&index| index < MAX_TERMS)
        .fold(TermMask(0), |mask, index| mask | TermMask(1 << index))
}

/// How well a session's messages match a query that they match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Score {
    /// Whether one message holds every term.
    pub all_in_one: bool,
    /// Messages that hold at least one term.
    pub matching_messages: usize,
    /// The message with the most distinct terms, the earliest of those that
    /// tie, as its index among the session's messages.
    pub best_message: usize,
    /// How many distinct terms that message holds.
    pub best_terms: u32,
}

/// Scores a session one message at a time, so that its messages need not
/// be held: [`score`] over the masks pushed so far.
#[derive(Debug, Clone)]
pub struct Scorer {
    all: TermMask,
    seen: TermMask,
    messages: usize,
    matching: usize,
    best: Option<(usize, u32)>,
}

impl Scorer {
    /// A scorer for a query of `terms` terms.
    pub fn new(terms: usize) -> Self {
        Self {
            all: TermMask::all(terms),
            seen: TermMask(0),
            messages: 0,
            matching: 0,
            best: None,
        }
    }

    /// Adds the next message's mask. Returns whether that message is now the
    /// best one, whose snippet the caller then keeps.
    pub fn push(&mut self, mask: TermMask) -> bool {
        let index = self.messages;
        self.messages += 1;
        let mask = TermMask(mask.0 & self.all.0);
        if mask.is_empty() {
            return false;
        }
        self.matching += 1;
        self.seen = self.seen | mask;
        let terms = mask.count();
        if self.best.is_some_and(|(_, best)| best >= terms) {
            return false;
        }
        self.best = Some((index, terms));
        true
    }

    /// The score, or `None` when some term is in no message.
    pub fn finish(&self) -> Option<Score> {
        if !self.seen.covers(self.all) {
            return None;
        }
        let (best_message, best_terms) = self.best?;
        Some(Score {
            all_in_one: best_terms == self.all.count(),
            matching_messages: self.matching,
            best_message,
            best_terms,
        })
    }
}

/// The score of a session whose messages, in order, hold the terms `masks`
/// give, for a query of `terms` terms; `None` when some term is in none of
/// them.
pub fn score(masks: impl IntoIterator<Item = TermMask>, terms: usize) -> Option<Score> {
    let mut scorer = Scorer::new(terms);
    for mask in masks {
        scorer.push(mask);
    }
    scorer.finish()
}

/// The part of a message that a result shows: on one line, around the first
/// match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snippet {
    pub role: String,
    /// Plain text, with `…` where the message was cut.
    pub text: String,
    /// The byte ranges of `text` that match a term.
    pub highlights: Vec<Range<usize>>,
}

/// The snippet of `message`: its content on one line, with runs of
/// whitespace made one space, cut to `context` characters on each side of
/// the first match, and the matches in it.
pub fn snippet(query: &Query, message: &Message, context: usize) -> Snippet {
    let line = collapse_whitespace(&ui::one_line(&message.content));
    let first = query.any.find(&line).map(|found| found.range());
    // Without a match on the line, as for a phrase with a tab in it, the
    // snippet is the start of the message.
    let (start, end) = match &first {
        Some(found) => (
            chars_before(&line, found.start, context),
            chars_after(&line, found.end, context),
        ),
        None => (0, chars_after(&line, 0, context.saturating_mul(2))),
    };
    // No space next to a `…`. Terms do not start or end with one, so the
    // match stays whole.
    let start = start + (line[start..end].len() - line[start..end].trim_start().len());
    let end = if end < line.len() {
        start + line[start..end].trim_end().len()
    } else {
        end
    };
    let mut text = String::new();
    if start > 0 {
        text.push('…');
    }
    let offset = text.len();
    text.push_str(&line[start..end]);
    if end < line.len() {
        text.push('…');
    }
    let highlights = query
        .any
        .find_iter(&line)
        .take_while(|found| found.start() < end)
        .filter_map(|found| {
            let (from, to) = (found.start().max(start), found.end().min(end));
            (from < to).then(|| from - start + offset..to - start + offset)
        })
        .collect();
    Snippet {
        role: message.role.clone(),
        text,
        highlights,
    }
}

fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The byte offset `count` characters before `at`, or 0.
fn chars_before(text: &str, at: usize, count: usize) -> usize {
    text[..at]
        .char_indices()
        .rev()
        .take(count)
        .last()
        .map_or(at, |(index, _)| index)
}

/// The byte offset `count` characters after `at`, or the end.
fn chars_after(text: &str, at: usize, count: usize) -> usize {
    text[at..]
        .char_indices()
        .nth(count)
        .map_or(text.len(), |(index, _)| at + index)
}

/// A session that matches a query, with its score and best message.
#[derive(Debug, Clone)]
pub struct Hit {
    pub session: SessionSummary,
    pub score: Score,
    pub snippet: Snippet,
}

/// Orders `hits` best first: those with every term in one message, then
/// those with more matching messages, then the most recently updated (by
/// the time `list` shows), then by ID, so that the order is total for
/// sessions of distinct IDs.
pub fn rank(mut hits: Vec<Hit>) -> Vec<Hit> {
    hits.sort_by(order);
    hits
}

fn order(a: &Hit, b: &Hit) -> Ordering {
    b.score
        .all_in_one
        .cmp(&a.score.all_in_one)
        .then(b.score.matching_messages.cmp(&a.score.matching_messages))
        .then(
            b.session
                .updated_or_created_ms()
                .cmp(&a.session.updated_or_created_ms()),
        )
        .then_with(|| a.session.id.cmp(&b.session.id))
}

/// Renders the results: one block per session, its ID, source, updated time
/// and title, then the role and snippet of its best message, the matches
/// highlighted with color. `term_width` is set when stdout is a terminal:
/// IDs are then shortened as `list` shortens them, distinct among `among`,
/// the IDs of every session `show` looks through.
pub fn render(hits: &[Hit], among: &[&str], use_color: bool, term_width: Option<usize>) -> String {
    let id_width = term_width.map(|width| ui::shown_id_width(among, width));
    let mut out = format!("Found {} matching session(s)\n", hits.len());
    for hit in hits {
        let session = &hit.session;
        let id = ui::one_line(&session.id);
        let id = match id_width {
            Some(width) => ui::shorten_id(&id, width),
            None => id.into_owned(),
        };
        let pad = " ".repeat(SOURCE_WIDTH.saturating_sub(session.source.as_str().len()));
        let updated = format!(
            "{:<width$}",
            session.updated_display(),
            width = UPDATED_WIDTH
        );
        out.push_str(&format!(
            "\n{id}  {}{pad}  {}  {}\n",
            ui::paint_source(session.source, use_color),
            ui::paint_dim(&updated, use_color),
            ui::one_line(&session.title),
        ));
        out.push_str(&format!(
            "  [{}] {}\n",
            ui::paint_role(&ui::one_line(&hit.snippet.role), use_color),
            highlighted(&hit.snippet, use_color)
        ));
    }
    if let Some(width) = id_width.filter(|&width| width < ui::ID_FULL_WIDTH) {
        out.push('\n');
        out.push_str(&ui::paint_dim(
            &format!("IDs shortened to {width} chars; `show` accepts a prefix."),
            use_color,
        ));
        out.push('\n');
    }
    out
}

/// The snippet's text with its matches in bold red, as ripgrep shows them,
/// when `use_color` is set.
fn highlighted(snippet: &Snippet, use_color: bool) -> String {
    if !use_color {
        return snippet.text.clone();
    }
    let mut out = String::with_capacity(snippet.text.len() + 16 * snippet.highlights.len());
    let mut at = 0;
    for range in &snippet.highlights {
        out.push_str(&snippet.text[at..range.start]);
        out.push_str(&(&snippet.text[range.clone()]).red().bold().to_string());
        at = range.end;
    }
    out.push_str(&snippet.text[at..]);
    out
}

impl Hit {
    /// The `search --json` entry.
    pub fn json(&self) -> HitJson<'_> {
        let summary = self.session.json();
        HitJson {
            id: summary.id,
            title: summary.title,
            source: summary.source,
            workspace: summary.workspace,
            updated_at: summary.updated_at,
            matching_messages: self.score.matching_messages,
            all_terms_in_one_message: self.score.all_in_one,
            snippet: SnippetJson {
                role: &self.snippet.role,
                text: &self.snippet.text,
                message_index: self.score.best_message,
            },
        }
    }
}

/// JSON shape of one `search --json` entry. Every key is always present.
#[derive(Debug, Clone, Serialize)]
pub struct HitJson<'a> {
    pub id: &'a str,
    pub title: &'a str,
    pub source: Source,
    pub workspace: Option<&'a str>,
    /// RFC 3339 UTC, whole seconds, as `list --json` gives it.
    pub updated_at: Option<String>,
    pub matching_messages: usize,
    pub all_terms_in_one_message: bool,
    pub snippet: SnippetJson<'a>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SnippetJson<'a> {
    pub role: &'a str,
    pub text: &'a str,
    /// The message's index among those `show --json` lists, from 0.
    pub message_index: usize,
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn query(text: &str) -> Query {
        parse_query(text).unwrap()
    }

    fn mask(text: &str, line: &str) -> u64 {
        matches(query(text).set(), line).0
    }

    fn message(role: &str, content: &str) -> Message {
        Message {
            role: role.into(),
            content: content.into(),
            timestamp: None,
        }
    }

    #[test]
    fn terms_split_on_unicode_whitespace_and_keep_quoted_phrases() {
        assert_eq!(parse_terms("retry  backoff"), ["retry", "backoff"]);
        // Tab, no-break space, ideographic space, line separator.
        assert_eq!(
            parse_terms("a\tb\u{a0}c\u{3000}d\u{2028}e\n"),
            ["a", "b", "c", "d", "e"]
        );
        assert_eq!(
            parse_terms(r#"pool "connection  timed out" retry"#),
            ["pool", "connection  timed out", "retry"]
        );
        assert_eq!(
            parse_terms(r#""a phrase""another""#),
            ["a phrase", "another"]
        );
        // A quote mark inside a word starts a phrase there.
        assert_eq!(
            parse_terms(r#"key"value pair" end"#),
            ["key", "value pair", "end"]
        );
        // A phrase left open runs to the end.
        assert_eq!(parse_terms(r#"one "two three"#), ["one", "two three"]);
        // Phrases lose the whitespace at their ends; empty terms are dropped.
        assert_eq!(parse_terms(r#"" padded "  """#), ["padded"]);
        for empty in ["", "   ", "\"\"", "\" \t \"", "\"", "\u{3000}"] {
            assert!(parse_terms(empty).is_empty(), "{empty:?}");
            assert_eq!(parse_query(empty).unwrap_err(), QueryError::Empty);
        }
        assert_eq!(parse_terms("日本語 データ"), ["日本語", "データ"]);
    }

    #[test]
    fn queries_have_one_to_64_terms() {
        let words = |n: usize| {
            (0..n)
                .map(|i| format!("w{i}"))
                .collect::<Vec<_>>()
                .join(" ")
        };
        assert_eq!(query(&words(64)).terms().len(), 64);
        assert_eq!(
            parse_query(&words(65)).unwrap_err(),
            QueryError::TooManyTerms(65)
        );
        // Each of 64 terms has a bit of its own.
        let all = matches(query(&words(64)).set(), &words(64));
        assert_eq!(all, TermMask::all(64));
        assert_eq!(all.count(), 64);
    }

    #[test]
    fn terms_match_case_insensitively_with_unicode_folding() {
        assert_eq!(mask("école", "L'ÉCOLE est fermée"), 1);
        assert_eq!(mask("ÉCOLE", "une école"), 1);
        assert_eq!(mask("σ", "ΣΊΣΥΦΟΣ"), 1);
        assert_eq!(mask("Σ", "σοφία"), 1);
        assert_eq!(mask("straße", "STRASSE"), 0, "no full case folding");
        assert_eq!(mask("Retry", "RETRY later"), 1);
        assert_eq!(mask("日本語", "日本語も大丈夫"), 1);
        assert_eq!(mask("a b", "A B"), 0b11);
        assert_eq!(mask("absent present", "present"), 0b10);
        assert_eq!(mask("\"two words\"", "TWO WORDS"), 1);
        assert_eq!(mask("\"two words\"", "two  words"), 0);
    }

    #[test]
    fn terms_are_literals_never_patterns() {
        assert_eq!(mask("a.b", "a.b"), 1);
        assert_eq!(mask("a.b", "axb"), 0);
        assert_eq!(mask("(x)", "f(x) = 1"), 1);
        assert_eq!(mask("(x)", "x"), 0);
        assert_eq!(mask("c++", "I like C++"), 1);
        assert_eq!(mask("c++", "c"), 0);
        for literal in [r"\d", "^start", "end$", "[ab]", "a|b", "x*", "{1}", "?"] {
            assert_eq!(
                mask(literal, &format!("has {literal} in it")),
                1,
                "{literal}"
            );
        }
        assert_eq!(mask(r"\d", "7"), 0);
        assert_eq!(mask("a|b", "a"), 0);
        assert_eq!(mask("[ab]", "a"), 0);
    }

    #[test]
    fn score_counts_messages_and_finds_the_best() {
        let m = |bits: &[u64]| bits.iter().map(|&bits| TermMask(bits)).collect::<Vec<_>>();
        assert_eq!(score(m(&[0b01, 0b10]), 3), None);
        assert_eq!(score(m(&[]), 1), None);
        assert_eq!(
            score(m(&[0, 0b01, 0b10, 0, 0b11, 0b11]), 2),
            Some(Score {
                all_in_one: true,
                matching_messages: 4,
                best_message: 4,
                best_terms: 2,
            })
        );
        assert_eq!(
            score(m(&[0b001, 0b110, 0b011]), 3),
            Some(Score {
                all_in_one: false,
                matching_messages: 3,
                best_message: 1,
                best_terms: 2,
            })
        );
        // Bits beyond the query's terms are not terms.
        assert_eq!(score(m(&[0b100]), 2), None);
    }

    #[test]
    fn snippets_cut_around_the_first_match() {
        let q = query("needle");
        let text = format!("{} NEEDLE {}", "a".repeat(100), "b".repeat(100));
        let snippet = snippet(&q, &message("user", &text), 5);
        assert_eq!(snippet.text, "…aaaa NEEDLE bbbb…");
        assert_eq!(snippet.role, "user");
        assert_eq!(&snippet.text[snippet.highlights[0].clone()], "NEEDLE");

        // Short messages are whole; lines and runs of whitespace become one space.
        let whole = super::snippet(
            &q,
            &message("assistant", "  the\n\nneedle\tis\x1b[31m here  "),
            60,
        );
        assert_eq!(whole.text, "the needle is here");
        assert_eq!(whole.highlights.len(), 1);
        assert_eq!(whole.highlights[0], 4..10);

        // Context counts characters, never splitting one.
        let wide = super::snippet(&query("ü"), &message("user", "日本語のüデータ"), 2);
        assert_eq!(wide.text, "…語のüデー…");
        assert_eq!(&wide.text[wide.highlights[0].clone()], "ü");

        // Nothing but the match with no context; every match in the window
        // is highlighted, also one the window cuts.
        let zero = super::snippet(&q, &message("user", "x needle y"), 0);
        assert_eq!(zero.text, "…needle…");
        let q2 = query("ab cd");
        let many = super::snippet(&q2, &message("user", "zz AB cd ab xxxxxx cd"), 7);
        assert_eq!(many.text, "zz AB cd ab…");
        let shown: Vec<&str> = many
            .highlights
            .iter()
            .map(|r| &many.text[r.clone()])
            .collect();
        assert_eq!(shown, ["AB", "cd", "ab"]);
        let cut = super::snippet(&query("ab xyzw"), &message("user", "ab xyzw"), 4);
        assert_eq!(cut.text, "ab xyz…");
        assert_eq!(cut.highlights, [0..2, 3..6]);

        // A longer term wins over a shorter one it starts with.
        let longest = super::snippet(&query("foo foobar"), &message("user", "a foobar"), 60);
        assert_eq!(longest.highlights.len(), 1);
        assert_eq!(longest.highlights[0], 2..8);

        // A match the one line loses, as of a phrase with a tab, shows the start.
        let lost = super::snippet(&query("\"a\tb\""), &message("user", "xyz a\tb"), 1);
        assert_eq!(lost.text, "xy…");
        assert!(lost.highlights.is_empty());
    }

    fn summary(id: &str, updated: Option<i64>) -> SessionSummary {
        SessionSummary {
            updated_at_ms: updated,
            ..SessionSummary::new(id, format!("title of {id}"), Source::Agent)
        }
    }

    fn hit(id: &str, updated: Option<i64>, score: Score) -> Hit {
        Hit {
            session: summary(id, updated),
            score,
            snippet: Snippet {
                role: "user".into(),
                text: format!("text of {id}"),
                highlights: Vec::new(),
            },
        }
    }

    fn scored(all_in_one: bool, matching_messages: usize) -> Score {
        Score {
            all_in_one,
            matching_messages,
            best_message: 0,
            best_terms: 1,
        }
    }

    fn ids(hits: &[Hit]) -> Vec<&str> {
        hits.iter().map(|hit| hit.session.id.as_str()).collect()
    }

    #[test]
    fn rank_orders_by_one_message_then_matches_then_time_then_id() {
        let hits = vec![
            hit("spread-many", Some(9), scored(false, 9)),
            hit("one-old", Some(1), scored(true, 1)),
            hit("one-new", Some(2), scored(true, 1)),
            hit("one-more", Some(0), scored(true, 3)),
            hit("untimed", None, scored(true, 1)),
            hit("tie-b", Some(2), scored(true, 1)),
            hit("spread-few", Some(9), scored(false, 2)),
        ];
        assert_eq!(
            ids(&rank(hits)),
            [
                "one-more",
                "one-new",
                "tie-b",
                "one-old",
                "untimed",
                "spread-many",
                "spread-few"
            ]
        );
    }

    #[test]
    fn render_shows_one_block_per_session() {
        let mut first = hit(
            "f4eea6d2-d2d3-41ad-b290-824445295a15",
            Some(1_757_003_600_000),
            scored(true, 2),
        );
        first.session.title = "Line\nbreak".into();
        first.snippet = super::snippet(
            &query("plan"),
            &message("assistant", "Here is the plan."),
            60,
        );
        let mut second = hit(
            "c0ffee00-0000-4000-8000-000000000001",
            None,
            scored(false, 1),
        );
        second.session.source = Source::Ide;
        let hits = [first, second];
        let among = [
            "f4eea6d2-d2d3-41ad-b290-824445295a15",
            "c0ffee00-0000-4000-8000-000000000001",
        ];
        assert_eq!(
            render(&hits, &among, false, None),
            "Found 2 matching session(s)\n\
             \n\
             f4eea6d2-d2d3-41ad-b290-824445295a15  agent  2025-09-04 16:33  Line break\n  \
             [assistant] Here is the plan.\n\
             \n\
             c0ffee00-0000-4000-8000-000000000001  ide    —                 title of c0ffee00-0000-4000-8000-000000000001\n  \
             [user] text of c0ffee00-0000-4000-8000-000000000001\n"
        );

        // In a terminal, IDs are shortened as `list` shortens them.
        let narrow = render(&hits, &among, false, Some(60));
        assert!(narrow.contains("\nf4eea6d2  agent  "), "{narrow}");
        assert!(
            narrow.ends_with("\nIDs shortened to 8 chars; `show` accepts a prefix.\n"),
            "{narrow}"
        );
        let close = [
            "f4eea6d2-d2d3-41ad-b290-824445295a15",
            "f4eea6d2-0000-4000-8000-000000000000",
        ];
        assert!(render(&hits[..1], &close, false, Some(60)).contains("\nf4eea6d2-d2d3  agent"));
        assert!(
            render(&hits[..1], &close, false, Some(200))
                .contains(&format!("\n{}  agent", close[0]))
        );

        // Color highlights the matches, bold red, and nothing without it.
        let colored = render(&hits[..1], &among, true, None);
        assert!(
            colored.contains(&format!("the {}.", "plan".red().bold())),
            "{colored:?}"
        );
        assert!(!render(&hits, &among, false, Some(200)).contains('\u{1b}'));
    }

    #[test]
    fn json_entries_have_the_documented_keys() {
        let mut entry = hit("abc", Some(1_757_003_600_000), scored(true, 2));
        entry.score.best_message = 3;
        let value = serde_json::to_value(entry.json()).unwrap();
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
                "updated_at",
                "matching_messages",
                "all_terms_in_one_message",
                "snippet"
            ]
        );
        assert_eq!(value["updated_at"], "2025-09-04T16:33:20Z");
        assert_eq!(value["workspace"], serde_json::Value::Null);
        assert_eq!(
            value["snippet"],
            serde_json::json!({"role": "user", "text": "text of abc", "message_index": 3})
        );
    }

    /// Masks of up to `terms` terms for a few messages.
    fn masks(terms: usize) -> impl Strategy<Value = Vec<TermMask>> {
        prop::collection::vec(0..=TermMask::all(terms).0, 0..12)
            .prop_map(|masks| masks.into_iter().map(TermMask).collect())
    }

    proptest! {
        #[test]
        fn score_is_none_exactly_when_a_term_is_missing(
            (terms, masks) in (1usize..=8).prop_flat_map(|terms| (Just(terms), masks(terms)))
        ) {
            let all = TermMask::all(terms);
            let seen = masks.iter().fold(TermMask(0), |seen, &mask| seen | mask);
            let scored = score(masks.clone(), terms);
            prop_assert_eq!(scored.is_none(), !seen.covers(all));
            if let Some(scored) = scored {
                let matching = masks.iter().filter(|mask| !mask.is_empty()).count();
                prop_assert_eq!(scored.matching_messages, matching);
                prop_assert_eq!(scored.all_in_one, masks.iter().any(|mask| mask.covers(all)));
                let most = masks.iter().map(|mask| mask.count()).max().unwrap_or(0);
                prop_assert_eq!(scored.best_terms, most);
                let first = masks.iter().position(|mask| mask.count() == most);
                prop_assert_eq!(Some(scored.best_message), first);
            }
        }

        #[test]
        fn all_terms_in_one_message_outranks_terms_spread_out(
            terms in 2usize..=6,
            one_extra in prop::collection::vec(any::<u64>(), 0..4),
            spread_extra in prop::collection::vec(any::<u64>(), 0..30),
            one_updated in any::<Option<i64>>(),
            spread_updated in any::<Option<i64>>(),
            one_id in "[a-z0-9]{1,8}",
            spread_id in "[a-z0-9]{1,8}",
        ) {
            prop_assume!(one_id != spread_id);
            let all = TermMask::all(terms);
            // Every term in one message, plus any others.
            let mut one: Vec<TermMask> = vec![all];
            one.extend(one_extra.iter().map(|&bits| TermMask(bits & all.0)));
            // Each term alone, plus any messages that lack at least one term.
            let mut spread: Vec<TermMask> = (0..terms).map(|term| TermMask(1 << term)).collect();
            spread.extend(spread_extra.iter().map(|&bits| TermMask(bits & all.0 & !1)));
            let one_score = score(one, terms).unwrap();
            let spread_score = score(spread, terms).unwrap();
            prop_assert!(one_score.all_in_one);
            prop_assert!(!spread_score.all_in_one);
            for hits in [
                vec![hit(&spread_id, spread_updated, spread_score), hit(&one_id, one_updated, one_score)],
                vec![hit(&one_id, one_updated, one_score), hit(&spread_id, spread_updated, spread_score)],
            ] {
                let ranked = rank(hits);
                prop_assert_eq!(&ranked[0].session.id, &one_id);
            }
        }

        #[test]
        fn rank_does_not_depend_on_the_order_given(
            (hits, shuffled) in prop::collection::vec(
                (any::<bool>(), 1usize..6, prop::option::of(0i64..4)),
                0..16,
            )
            .prop_flat_map(|rows| {
                let hits: Vec<(String, bool, usize, Option<i64>)> = rows
                    .into_iter()
                    .enumerate()
                    .map(|(n, (one, matching, updated))| (format!("s{n:02}"), one, matching, updated))
                    .collect();
                (Just(hits.clone()), Just(hits).prop_shuffle())
            })
        ) {
            let build = |rows: &[(String, bool, usize, Option<i64>)]| {
                rows.iter()
                    .map(|(id, one, matching, updated)| hit(id, *updated, scored(*one, *matching)))
                    .collect::<Vec<_>>()
            };
            let ranked = rank(build(&hits));
            let reranked = rank(build(&shuffled));
            prop_assert_eq!(ids(&ranked), ids(&reranked));
            // Every hit stays, ordered as the rules say.
            prop_assert_eq!(ranked.len(), hits.len());
            for pair in ranked.windows(2) {
                prop_assert_ne!(order(&pair[0], &pair[1]), Ordering::Greater);
            }
        }
    }
}
