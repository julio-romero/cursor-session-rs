use std::borrow::Cow;
use std::io::IsTerminal;

use comfy_table::{Attribute, Cell, Color, ContentArrangement, Table, presets};
use owo_colors::OwoColorize;

use crate::model::{Message, Session, Source};

pub const DEFAULT_TTY_SHOW_LIMIT: usize = 20;
const ID_FULL_WIDTH: usize = 36;
/// UUID prefix lengths that do not split a hex group: 8, 8-4, 8-4-4, 8-4-4-4, full.
const ID_PREFIX_WIDTHS: [usize; 5] = [8, 13, 18, 23, 36];
const SOURCE_WIDTH: usize = 6;
const MSGS_WIDTH: usize = 5;
const UPDATED_WIDTH: usize = 16;
const COL_GUTTER: usize = 2;
const TABLE_CHROME: usize = 12;
const MIN_TITLE_WIDTH: usize = 16;
const MIN_TERM_WIDTH: usize = 40;
const DEFAULT_TERM_WIDTH: usize = 80;

pub fn stdout_is_tty() -> bool {
    std::io::stdout().is_terminal()
}

/// Width of the controlling terminal. Only meaningful when stdout is a terminal.
pub fn terminal_width() -> usize {
    if let Ok((cols, _)) = crossterm::terminal::size()
        && cols > 0
    {
        return usize::from(cols).max(MIN_TERM_WIDTH);
    }
    std::env::var("COLUMNS")
        .ok()
        .and_then(|value| value.trim().parse().ok())
        .filter(|width| *width >= MIN_TERM_WIDTH)
        .unwrap_or(DEFAULT_TERM_WIDTH)
}

fn fixed_list_width() -> usize {
    SOURCE_WIDTH + MSGS_WIDTH + UPDATED_WIDTH + COL_GUTTER * 4 + TABLE_CHROME
}

/// How many leading ID characters fit in this terminal without crowding TITLE.
pub fn id_prefix_width(term_width: usize) -> usize {
    let available = term_width
        .saturating_sub(fixed_list_width() + MIN_TITLE_WIDTH)
        .max(ID_PREFIX_WIDTHS[0]);
    ID_PREFIX_WIDTHS
        .into_iter()
        .rev()
        .find(|&width| width <= available)
        .unwrap_or(ID_PREFIX_WIDTHS[0])
}

pub fn shorten_id(id: &str, width: usize) -> String {
    if id.chars().count() <= width {
        return id.to_string();
    }
    id.chars().take(width).collect()
}

pub fn paint_source(source: Source, use_color: bool) -> String {
    paint_source_text(source.as_str(), source, use_color)
}

fn paint_source_text(text: &str, source: Source, use_color: bool) -> String {
    if !use_color {
        return text.to_string();
    }
    match source {
        Source::Agent => text.cyan().to_string(),
        Source::Ide => text.magenta().to_string(),
    }
}

fn paint_bold(text: &str, use_color: bool) -> String {
    if !use_color {
        return text.to_string();
    }
    text.bold().to_string()
}

pub fn paint_role(role: &str, use_color: bool) -> String {
    if !use_color {
        return role.to_string();
    }
    match role {
        "user" => role.blue().bold().to_string(),
        "assistant" => role.magenta().bold().to_string(),
        _ => role.to_string(),
    }
}

pub fn paint_dim(text: &str, use_color: bool) -> String {
    if !use_color {
        return text.to_string();
    }
    text.dimmed().to_string()
}

pub fn truncate_chars(text: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// Replaces control characters (newlines, tabs, escape sequences) with spaces so
/// stored text stays on one line and cannot drive the terminal.
pub fn one_line(text: &str) -> Cow<'_, str> {
    if !text.chars().any(char::is_control) {
        return Cow::Borrowed(text);
    }
    Cow::Owned(
        text.chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .collect(),
    )
}

/// Drops control characters other than newline and tab from multi-line text.
pub fn printable(text: &str) -> Cow<'_, str> {
    let unsafe_control = |c: char| c.is_control() && c != '\n' && c != '\t';
    if !text.chars().any(unsafe_control) {
        return Cow::Borrowed(text);
    }
    Cow::Owned(text.chars().filter(|&c| !unsafe_control(c)).collect())
}

pub fn title_width(term_width: usize) -> usize {
    let used = id_prefix_width(term_width)
        + SOURCE_WIDTH
        + MSGS_WIDTH
        + UPDATED_WIDTH
        + COL_GUTTER * 4
        + TABLE_CHROME;
    term_width.saturating_sub(used).max(MIN_TITLE_WIDTH)
}

pub fn select_messages<T>(
    messages: &[T],
    is_tty: bool,
    limit: Option<usize>,
    all: bool,
) -> (&[T], Option<usize>) {
    if all {
        return (messages, None);
    }
    if let Some(n) = limit {
        let start = messages.len().saturating_sub(n);
        let hidden = if start > 0 { Some(start) } else { None };
        return (&messages[start..], hidden);
    }
    if !is_tty || messages.len() <= DEFAULT_TTY_SHOW_LIMIT {
        return (messages, None);
    }
    let start = messages.len() - DEFAULT_TTY_SHOW_LIMIT;
    (&messages[start..], Some(start))
}

pub fn format_message_header(role: &str, timestamp: Option<&str>, use_color: bool) -> String {
    let role = paint_role(&one_line(role), use_color);
    match timestamp {
        Some(ts) => format!(
            "[{role}{}]",
            paint_dim(&format!(" ({})", one_line(ts)), use_color)
        ),
        None => format!("[{role}]"),
    }
}

/// Renders the session list. `term_width` is the terminal width when stdout is a
/// terminal, which selects the fitted table; `None` selects the plain layout.
/// Color is independent of the layout.
pub fn render_list(sessions: &[Session], use_color: bool, term_width: Option<usize>) -> String {
    if sessions.is_empty() {
        return "No sessions found\n".to_string();
    }
    match term_width {
        Some(width) => render_list_table(sessions, use_color, width),
        None => render_list_plain(sessions, use_color),
    }
}

fn render_list_plain(sessions: &[Session], use_color: bool) -> String {
    let mut out = format!("Found {} session(s)\n\n", sessions.len());
    let header = format!(
        "{:<id_w$}  {:<src_w$}  {:>msgs_w$}  {:<upd_w$}  TITLE",
        "ID",
        "SOURCE",
        "MSGS",
        "UPDATED",
        id_w = ID_FULL_WIDTH,
        src_w = SOURCE_WIDTH,
        msgs_w = MSGS_WIDTH,
        upd_w = UPDATED_WIDTH
    );
    out.push_str(&paint_bold(&header, use_color));
    out.push('\n');
    out.push_str(&"-".repeat(100));
    out.push('\n');
    for session in sessions {
        let source = format!("{:<src_w$}", session.source.as_str(), src_w = SOURCE_WIDTH);
        let updated = format!(
            "{:<upd_w$}",
            session.updated_display(),
            upd_w = UPDATED_WIDTH
        );
        out.push_str(&format!(
            "{:<id_w$}  {}  {:>msgs_w$}  {}  {}\n",
            one_line(&session.id),
            paint_source_text(&source, session.source, use_color),
            session.message_count(),
            paint_dim(&updated, use_color),
            one_line(&session.title),
            id_w = ID_FULL_WIDTH,
            msgs_w = MSGS_WIDTH,
        ));
    }
    out
}

fn render_list_table(sessions: &[Session], use_color: bool, term_width: usize) -> String {
    let width = u16::try_from(term_width).unwrap_or(u16::MAX);
    let term_width = usize::from(width);
    let mut table = Table::new();
    table.load_preset(presets::UTF8_FULL_CONDENSED);
    table.set_content_arrangement(ContentArrangement::Dynamic);
    // Styling follows `use_color`, never comfy-table's own stdout check or
    // crossterm's NO_COLOR check: the caller already applied --color and NO_COLOR.
    table.force_no_tty();
    if use_color {
        table.enforce_styling();
        crossterm::style::force_color_output(true);
    }
    table.set_width(width);
    table.set_header(vec![
        header_cell("ID"),
        header_cell("SOURCE"),
        header_cell("MSGS"),
        header_cell("UPDATED"),
        header_cell("TITLE"),
    ]);

    let id_width = id_prefix_width(term_width);
    let max_title = title_width(term_width);
    for session in sessions {
        table.add_row(vec![
            Cell::new(shorten_id(&one_line(&session.id), id_width)),
            source_cell(session.source),
            Cell::new(session.message_count()),
            Cell::new(session.updated_display()).fg(Color::DarkGrey),
            Cell::new(truncate_chars(&one_line(&session.title), max_title)),
        ]);
    }

    let mut out = format!("Found {} session(s)\n\n{table}\n", sessions.len());
    if id_width < ID_FULL_WIDTH {
        out.push_str(&paint_dim(
            &format!("IDs shortened to {id_width} chars; `show` accepts a prefix."),
            use_color,
        ));
        out.push('\n');
    }
    out
}

fn header_cell(text: &str) -> Cell {
    Cell::new(text).add_attribute(Attribute::Bold)
}

fn source_cell(source: Source) -> Cell {
    let cell = Cell::new(source.as_str());
    match source {
        Source::Agent => cell.fg(Color::Cyan),
        Source::Ide => cell.fg(Color::Magenta),
    }
}

pub fn render_show_header(session: &Session, use_color: bool) -> String {
    let mut lines = Vec::new();
    lines.push(paint_bold(&one_line(&session.title), use_color));
    lines.push(format!("id:        {}", one_line(&session.id)));
    lines.push(format!(
        "source:    {}",
        paint_source(session.source, use_color)
    ));
    if let Some(workspace) = &session.workspace {
        lines.push(format!("workspace: {}", one_line(workspace)));
    }
    if let Some(model) = &session.model {
        lines.push(format!("model:     {}", one_line(model)));
    }
    lines.push(format!(
        "created:   {}",
        paint_dim(&session.created_display(), use_color)
    ));
    lines.push(format!(
        "updated:   {}",
        paint_dim(&session.updated_display(), use_color)
    ));
    lines.push(format!("messages:  {}", session.message_count()));
    lines.join("\n")
}

pub fn render_show(
    session: &Session,
    messages: &[Message],
    hidden: Option<usize>,
    use_color: bool,
) -> String {
    let mut out = render_show_header(session, use_color);
    out.push('\n');
    if let Some(hidden) = hidden {
        out.push('\n');
        out.push_str(&paint_dim(
            &format!("{hidden} earlier message(s) omitted. Use --limit N or --all to see more."),
            use_color,
        ));
        out.push('\n');
    }
    for message in messages {
        out.push('\n');
        out.push_str(&format_message_header(
            &message.role,
            message.timestamp.as_deref(),
            use_color,
        ));
        out.push('\n');
        out.push_str(&printable(&message.content));
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Source;

    fn sample_session() -> Session {
        Session {
            id: "f4eea6d2-d2d3-41ad-b290-824445295a15".into(),
            title: "Langfuse Semantic Layer".into(),
            source: Source::Agent,
            workspace: Some("/Users/manuel.romero".into()),
            workspace_hash: None,
            created_at_ms: Some(1_700_000_000_000),
            updated_at_ms: Some(1_700_000_100_000),
            model: Some("grok-4.6".into()),
            messages: vec![
                Message {
                    role: "user".into(),
                    content: "hello".into(),
                    timestamp: Some("Tue".into()),
                },
                Message {
                    role: "assistant".into(),
                    content: "hi".into(),
                    timestamp: None,
                },
            ],
        }
    }

    fn has_ansi(s: &str) -> bool {
        s.contains('\u{1b}')
    }

    #[test]
    fn colorless_helpers_emit_no_ansi() {
        assert!(!has_ansi(&paint_source(Source::Agent, false)));
        assert!(!has_ansi(&paint_role("user", false)));
        assert!(!has_ansi(&paint_dim("2026-09-17 23:14", false)));
        assert!(!has_ansi(&format_message_header(
            "assistant",
            Some("now"),
            false
        )));
        let session = sample_session();
        for width in [Some(120), None] {
            assert!(!has_ansi(&render_list(
                std::slice::from_ref(&session),
                false,
                width
            )));
        }
        assert!(!has_ansi(&render_show(
            &session,
            &session.messages,
            Some(5),
            false
        )));
    }

    #[test]
    fn color_helpers_emit_ansi() {
        assert!(has_ansi(&paint_source(Source::Agent, true)));
        assert!(has_ansi(&paint_role("user", true)));
        assert!(has_ansi(&format_message_header("user", Some("now"), true)));
    }

    #[test]
    fn piped_show_prints_all_without_limit() {
        let messages: Vec<u8> = (0..83).collect();
        let (shown, hidden) = select_messages(&messages, false, None, false);
        assert_eq!(shown.len(), 83);
        assert_eq!(hidden, None);
    }

    #[test]
    fn tty_show_defaults_to_last_twenty() {
        let messages: Vec<u8> = (0..83).collect();
        let (shown, hidden) = select_messages(&messages, true, None, false);
        assert_eq!(shown.len(), 20);
        assert_eq!(hidden, Some(63));
        assert_eq!(shown[0], 63);
    }

    #[test]
    fn all_flag_prints_everything_on_tty() {
        let messages: Vec<u8> = (0..83).collect();
        let (shown, hidden) = select_messages(&messages, true, None, true);
        assert_eq!(shown.len(), 83);
        assert_eq!(hidden, None);
    }

    #[test]
    fn plain_list_keeps_full_title() {
        let session = sample_session();
        let rendered = render_list(&[session], false, None);
        assert!(rendered.contains("Langfuse Semantic Layer"));
        assert!(rendered.contains("f4eea6d2-d2d3-41ad-b290-824445295a15"));
        assert!(!has_ansi(&rendered));
    }

    #[test]
    fn id_prefix_snaps_to_uuid_groups() {
        assert_eq!(id_prefix_width(200), 36);
        assert_eq!(id_prefix_width(80), 13);
        assert_eq!(id_prefix_width(50), 8);
        assert_eq!(
            shorten_id("f4eea6d2-d2d3-41ad-b290-824445295a15", 13),
            "f4eea6d2-d2d3"
        );
        assert_eq!(
            shorten_id("f4eea6d2-d2d3-41ad-b290-824445295a15", 8),
            "f4eea6d2"
        );
    }

    #[test]
    fn narrow_tty_list_shortens_id() {
        let session = sample_session();
        let rendered = render_list(std::slice::from_ref(&session), true, Some(80));
        assert!(rendered.contains("f4eea6d2-d2d3"));
        assert!(!rendered.contains("f4eea6d2-d2d3-41ad-b290-824445295a15"));
        assert!(rendered.contains("show` accepts a prefix"));
    }

    fn strip_ansi(s: &str) -> String {
        let mut out = String::new();
        let mut chars = s.chars();
        while let Some(c) = chars.next() {
            if c != '\u{1b}' {
                out.push(c);
                continue;
            }
            assert_eq!(chars.next(), Some('['));
            for c in chars.by_ref() {
                if ('@'..='~').contains(&c) {
                    break;
                }
            }
        }
        out
    }

    fn sample_sessions() -> Vec<Session> {
        let mut ide = sample_session();
        ide.id = "0b1c2d3e-aaaa-bbbb-cccc-1234567890ab".into();
        ide.title = "Refactor the configuration loader and add tests for it".into();
        ide.source = Source::Ide;
        vec![sample_session(), ide]
    }

    #[test]
    fn table_without_color_keeps_layout_and_drops_ansi() {
        let sessions = sample_sessions();
        for width in [40, 60, 80, 120, 200] {
            let colored = render_list(&sessions, true, Some(width));
            let plain = render_list(&sessions, false, Some(width));
            // Cyan SOURCE cell, also when the test runs under NO_COLOR.
            assert!(colored.contains("\u{1b}[38;5;14m"));
            assert!(!has_ansi(&plain));
            assert!(plain.contains('│'));
            assert_eq!(strip_ansi(&colored), plain);
        }
    }

    #[test]
    fn piped_color_keeps_plain_layout() {
        let sessions = sample_sessions();
        let colored = render_list(&sessions, true, None);
        let plain = render_list(&sessions, false, None);
        assert!(has_ansi(&colored));
        assert!(!plain.contains('│'));
        assert_eq!(strip_ansi(&colored), plain);

        let session = &sessions[0];
        let colored = render_show(session, &session.messages, Some(3), true);
        let plain = render_show(session, &session.messages, Some(3), false);
        assert!(has_ansi(&colored));
        assert_eq!(strip_ansi(&colored), plain);
    }

    #[test]
    fn extreme_widths_and_titles_render() {
        let titles = [
            String::new(),
            "x".into(),
            "a".repeat(5000),
            "日本語のタイトル".repeat(40),
            "family 👨‍👩‍👧‍👦 flag 🇪🇸 ".repeat(20),
            "e\u{301}\u{301}".repeat(100),
        ];
        let sessions: Vec<Session> = titles
            .iter()
            .map(|title| Session {
                title: title.clone(),
                ..sample_session()
            })
            .collect();
        let widths = [0, 1, 2, 10, 39, 40, 41, 65_535, 65_536, usize::MAX];
        for width in widths {
            for color in [true, false] {
                let rendered = render_list(&sessions, color, Some(width));
                assert!(rendered.starts_with("Found 6 session(s)"));
            }
        }
        assert!(title_width(usize::MAX) >= MIN_TITLE_WIDTH);
        assert_eq!(id_prefix_width(0), 8);
    }

    #[test]
    fn control_characters_are_neutralized() {
        let mut session = sample_session();
        session.title = "evil\u{1b}]0;pwned\u{7}\nsecond\tline".into();
        session.messages[0].content = "\u{1b}[2Jcleared\r\n\tindented\nnext\u{9b}31m".into();

        for width in [Some(80), None] {
            let rendered = render_list(std::slice::from_ref(&session), false, width);
            assert!(!rendered.chars().any(|c| c.is_control() && c != '\n'));
            assert!(rendered.contains("evil ]0;pwned  sec"));
        }
        let plain = render_list(std::slice::from_ref(&session), false, None);
        assert!(plain.contains("evil ]0;pwned  second line\n"));
        assert_eq!(plain.lines().count(), 5);

        let shown = render_show(&session, &session.messages, None, false);
        assert!(!has_ansi(&shown));
        assert!(!shown.contains('\r'));
        assert!(shown.contains("[2Jcleared\n\tindented\nnext31m"));
    }
}
