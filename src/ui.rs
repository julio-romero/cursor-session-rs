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

pub fn stdout_is_tty() -> bool {
    std::io::stdout().is_terminal()
}

pub fn terminal_width() -> usize {
    if let Ok((cols, _)) = crossterm::terminal::size() {
        return (cols as usize).max(40);
    }
    std::env::var("COLUMNS")
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|width| *width >= 40)
        .unwrap_or(80)
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
    let label = source.as_str();
    if !use_color {
        return label.to_string();
    }
    match source {
        Source::Agent => label.cyan().to_string(),
        Source::Ide => label.magenta().to_string(),
    }
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
    let role = paint_role(role, use_color);
    match timestamp {
        Some(ts) => format!("[{role}{}]", paint_dim(&format!(" ({ts})"), use_color)),
        None => format!("[{role}]"),
    }
}

pub fn render_list(sessions: &[Session], use_color: bool, term_width: usize) -> String {
    if sessions.is_empty() {
        return "No sessions found\n".to_string();
    }
    if use_color {
        render_list_table(sessions, term_width)
    } else {
        render_list_plain(sessions)
    }
}

fn render_list_plain(sessions: &[Session]) -> String {
    let mut out = format!("Found {} session(s)\n\n", sessions.len());
    out.push_str(&format!(
        "{:<id_w$}  {:<src_w$}  {:>msgs_w$}  {:<upd_w$}  TITLE\n",
        "ID",
        "SOURCE",
        "MSGS",
        "UPDATED",
        id_w = ID_FULL_WIDTH,
        src_w = SOURCE_WIDTH,
        msgs_w = MSGS_WIDTH,
        upd_w = UPDATED_WIDTH
    ));
    out.push_str(&"-".repeat(100));
    out.push('\n');
    for session in sessions {
        out.push_str(&format!(
            "{:<id_w$}  {:<src_w$}  {:>msgs_w$}  {:<upd_w$}  {}\n",
            session.id,
            session.source.as_str(),
            session.message_count(),
            session.updated_display(),
            session.title,
            id_w = ID_FULL_WIDTH,
            src_w = SOURCE_WIDTH,
            msgs_w = MSGS_WIDTH,
            upd_w = UPDATED_WIDTH
        ));
    }
    out
}

fn render_list_table(sessions: &[Session], term_width: usize) -> String {
    let mut table = Table::new();
    table.load_preset(presets::UTF8_FULL_CONDENSED);
    table.set_content_arrangement(ContentArrangement::Dynamic);
    table.set_width(term_width as u16);
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
            Cell::new(shorten_id(&session.id, id_width)),
            source_cell(session.source),
            Cell::new(session.message_count()),
            Cell::new(session.updated_display()).fg(Color::DarkGrey),
            Cell::new(truncate_chars(&session.title, max_title)),
        ]);
    }

    let mut out = format!("Found {} session(s)\n\n{table}\n", sessions.len());
    if id_width < ID_FULL_WIDTH {
        out.push_str(&paint_dim(
            &format!("IDs shortened to {id_width} chars; `show` accepts a prefix."),
            true,
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
    lines.push(if use_color {
        session.title.bold().to_string()
    } else {
        session.title.clone()
    });
    lines.push(format!("id:        {}", session.id));
    lines.push(format!(
        "source:    {}",
        paint_source(session.source, use_color)
    ));
    if let Some(workspace) = &session.workspace {
        lines.push(format!("workspace: {workspace}"));
    }
    if let Some(model) = &session.model {
        lines.push(format!("model:     {model}"));
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
        out.push_str(&message.content);
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
        assert!(!has_ansi(&render_list(
            std::slice::from_ref(&session),
            false,
            120
        )));
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
        let rendered = render_list(&[session], false, 40);
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
        let rendered = render_list(std::slice::from_ref(&session), true, 80);
        assert!(rendered.contains("f4eea6d2-d2d3"));
        assert!(!rendered.contains("f4eea6d2-d2d3-41ad-b290-824445295a15"));
        assert!(rendered.contains("show` accepts a prefix"));
    }
}
