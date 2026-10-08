use std::borrow::Cow;
use std::collections::HashSet;
use std::io::IsTerminal;

use comfy_table::{Attribute, Cell, Color, ContentArrangement, Table, presets};
use owo_colors::OwoColorize;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::model::{Message, Session, SessionSummary, Source, TOOL_ROLE};

pub const DEFAULT_TTY_SHOW_LIMIT: usize = 20;
const ID_FULL_WIDTH: usize = 36;
/// UUID prefix lengths that do not split a hex group: 8, 8-4, 8-4-4, 8-4-4-4, full.
const ID_PREFIX_WIDTHS: [usize; 5] = [8, 13, 18, 23, 36];
const SOURCE_WIDTH: usize = 6;
const MSGS_WIDTH: usize = 5;
const TOKENS_WIDTH: usize = 6;
/// TOKENS in the plain layout, whose columns do not fit their contents: wide
/// enough to keep 99,999,999 tokens aligned.
const PLAIN_TOKENS_WIDTH: usize = 8;
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
    let cols = crossterm::terminal::size().ok().map(|(cols, _)| cols);
    width_from(cols, std::env::var("COLUMNS").ok().as_deref())
}

/// The width the terminal reports, else the one COLUMNS gives, but never
/// below the narrowest table; 80 when neither gives one.
fn width_from(cols: Option<u16>, columns: Option<&str>) -> usize {
    if let Some(cols) = cols.filter(|&cols| cols > 0) {
        return usize::from(cols).max(MIN_TERM_WIDTH);
    }
    columns
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|&width| width > 0)
        .map_or(DEFAULT_TERM_WIDTH, |width| width.max(MIN_TERM_WIDTH))
}

/// The width of the table's columns other than ID and TITLE, with TOKENS when
/// `tokens` is set.
fn fixed_list_width(tokens: bool) -> usize {
    let tokens = if tokens { TOKENS_WIDTH + COL_GUTTER } else { 0 };
    SOURCE_WIDTH + MSGS_WIDTH + tokens + UPDATED_WIDTH + COL_GUTTER * 4 + TABLE_CHROME
}

/// Whether the table has a TOKENS column in this terminal: only when it fits
/// beside the shortest IDs and titles, so that a narrow terminal keeps the
/// columns it has room for.
pub fn shows_tokens(term_width: usize) -> bool {
    term_width >= fixed_list_width(true) + ID_PREFIX_WIDTHS[0] + MIN_TITLE_WIDTH
}

/// How many leading ID characters fit in this terminal without crowding TITLE.
pub fn id_prefix_width(term_width: usize) -> usize {
    let fixed = fixed_list_width(shows_tokens(term_width));
    let available = term_width
        .saturating_sub(fixed + MIN_TITLE_WIDTH)
        .max(ID_PREFIX_WIDTHS[0]);
    ID_PREFIX_WIDTHS
        .into_iter()
        .rev()
        .find(|&width| width <= available)
        .unwrap_or(ID_PREFIX_WIDTHS[0])
}

/// The narrowest UUID prefix length, from `width` up, at which the shortened
/// IDs of `sessions` differ, so that each is a prefix `show` accepts; the
/// full width when none is.
fn distinct_id_width(ids: &[&str], width: usize) -> usize {
    ID_PREFIX_WIDTHS
        .into_iter()
        .filter(|&candidate| candidate >= width)
        .find(|&candidate| {
            let mut seen = HashSet::new();
            ids.iter()
                .all(|id| seen.insert(shorten_id(id, candidate).to_lowercase()))
        })
        .unwrap_or(ID_FULL_WIDTH)
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
        TOOL_ROLE => role.cyan().to_string(),
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

/// Like [`truncate_chars`], but `max` counts terminal columns, as the table does.
pub fn truncate_width(text: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    if text.width() <= max {
        return text.to_string();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in text.chars() {
        used += c.width().unwrap_or(0);
        if used >= max {
            break;
        }
        out.push(c);
    }
    // A few sequences, such as an emoji presentation selector, are wider than
    // their characters.
    while out.width() >= max {
        out.pop();
    }
    out.push('…');
    out
}

/// Removes escape sequences and turns the remaining control characters (newlines,
/// tabs) into spaces, so stored text fits on one table row.
pub fn one_line(text: &str) -> Cow<'_, str> {
    if !text.chars().any(char::is_control) {
        return Cow::Borrowed(text);
    }
    let mut filter = TerminalFilter::new(false);
    let mut safe = Vec::with_capacity(text.len());
    filter.push(text.as_bytes(), &mut safe);
    Cow::Owned(
        String::from_utf8_lossy(&safe)
            .chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .collect(),
    )
}

/// Longest SGR parameter list passed through; longer ones are dropped.
const MAX_SGR_LEN: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FilterState {
    Ground,
    /// After `\r`, which is kept only as part of `\r\n`.
    CarriageReturn,
    /// After 0xC2, the UTF-8 lead byte of the C1 controls.
    C1Lead,
    Escape,
    EscapeIntermediate,
    /// Control sequence (`ESC [` or C1 CSI); its parameters collect in `csi`.
    Csi,
    /// Control string (OSC, DCS, SOS, PM, APC), dropped up to its terminator.
    ControlString,
    ControlStringEscape,
    ControlStringC1Lead,
}

/// Streaming filter that makes text safe for a terminal: it removes escape
/// sequences whole (CSI, OSC and other control strings, two-byte escapes, their
/// C1 forms) and control characters other than `\n`, `\t` and the `\r` of `\r\n`.
/// SGR styling (`ESC [ ... m`) is kept when `keep_sgr` is set, without blinking
/// or hidden text; styling still on at the end of a line is reset there, so
/// stored text cannot style what follows. Input may be split anywhere, also
/// inside a sequence or a UTF-8 character.
#[derive(Debug, Clone)]
pub struct TerminalFilter {
    keep_sgr: bool,
    state: FilterState,
    csi: Vec<u8>,
    /// Whether the control sequence being read may pass: only the 7-bit
    /// `ESC [` form, which is what this program writes.
    csi_kept: bool,
    /// SGR attribute groups switched on by the sequences passed through.
    style: u16,
}

impl TerminalFilter {
    pub fn new(keep_sgr: bool) -> Self {
        Self {
            keep_sgr,
            state: FilterState::Ground,
            csi: Vec::new(),
            csi_kept: false,
            style: 0,
        }
    }

    /// Appends the safe part of `input` to `out`.
    pub fn push(&mut self, input: &[u8], out: &mut Vec<u8>) {
        for &byte in input {
            self.byte(byte, out);
        }
    }

    /// Appends a full reset when styling that passed through is still on.
    pub fn reset_style(&mut self, out: &mut Vec<u8>) {
        if self.style != 0 {
            self.style = 0;
            out.extend_from_slice(b"\x1b[0m");
        }
    }

    fn byte(&mut self, byte: u8, out: &mut Vec<u8>) {
        match self.state {
            FilterState::Ground => self.ground(byte, out),
            FilterState::CarriageReturn => {
                self.state = FilterState::Ground;
                if byte == b'\n' {
                    self.reset_style(out);
                    out.push(b'\r');
                }
                self.ground(byte, out);
            }
            FilterState::C1Lead => {
                self.state = FilterState::Ground;
                match byte {
                    0x9b => self.start_csi(false),
                    0x90 | 0x98 | 0x9d..=0x9f => self.state = FilterState::ControlString,
                    0x80..=0x9f => {}
                    _ => {
                        out.push(0xc2);
                        self.ground(byte, out);
                    }
                }
            }
            FilterState::Escape => self.escape(byte, out),
            FilterState::EscapeIntermediate => match byte {
                0x20..=0x2f => {}
                0x30..=0x7e => self.state = FilterState::Ground,
                _ => {
                    self.state = FilterState::Ground;
                    self.ground(byte, out);
                }
            },
            FilterState::Csi => match byte {
                0x20..=0x3f => {
                    if self.csi.len() <= MAX_SGR_LEN {
                        self.csi.push(byte);
                    }
                }
                0x40..=0x7e => {
                    self.state = FilterState::Ground;
                    if byte == b'm'
                        && self.keep_sgr
                        && self.csi_kept
                        && is_sgr(&self.csi)
                        && let Some(params) = visible_sgr(&self.csi)
                    {
                        out.extend_from_slice(b"\x1b[");
                        out.extend_from_slice(&params);
                        out.push(b'm');
                        self.csi = params;
                        self.track_sgr();
                    }
                }
                _ => {
                    self.state = FilterState::Ground;
                    self.ground(byte, out);
                }
            },
            FilterState::ControlString => self.control_string(byte, out),
            FilterState::ControlStringEscape => {
                if byte == b'\\' {
                    self.state = FilterState::Ground;
                } else {
                    self.escape(byte, out);
                }
            }
            FilterState::ControlStringC1Lead => match byte {
                0x9c => self.state = FilterState::Ground,
                0x80..=0xbf => self.state = FilterState::ControlString,
                _ => {
                    self.state = FilterState::ControlString;
                    self.control_string(byte, out);
                }
            },
        }
    }

    fn ground(&mut self, byte: u8, out: &mut Vec<u8>) {
        match byte {
            0x1b => self.state = FilterState::Escape,
            b'\r' => self.state = FilterState::CarriageReturn,
            b'\n' => {
                self.reset_style(out);
                out.push(byte);
            }
            b'\t' => out.push(byte),
            0x00..=0x1f | 0x7f => {}
            0xc2 => self.state = FilterState::C1Lead,
            _ => out.push(byte),
        }
    }

    fn escape(&mut self, byte: u8, out: &mut Vec<u8>) {
        self.state = FilterState::Ground;
        match byte {
            b'[' => self.start_csi(true),
            b']' | b'P' | b'X' | b'^' | b'_' => self.state = FilterState::ControlString,
            0x20..=0x2f => self.state = FilterState::EscapeIntermediate,
            0x30..=0x7e => {}
            _ => self.ground(byte, out),
        }
    }

    fn start_csi(&mut self, kept: bool) {
        self.csi.clear();
        self.csi_kept = kept;
        self.state = FilterState::Csi;
    }

    fn track_sgr(&mut self) {
        let mut params = self.csi.split(|&byte| byte == b';');
        while let Some(param) = params.next() {
            let mut parts = param.split(|&byte| byte == b':').map(sgr_number);
            let code = parts.next().unwrap_or(0);
            let sub = parts.next();
            if code == 0 {
                self.style = 0;
                continue;
            }
            if matches!(code, 38 | 48 | 58) && sub.is_none() {
                // `38;5;n` and `38;2;r;g;b` carry the color in the next parameters.
                let skip = match params.next().map(sgr_number) {
                    Some(5) => 1,
                    Some(2) => 3,
                    _ => 0,
                };
                params.by_ref().take(skip).for_each(drop);
            }
            let (group, on) = sgr_group(code);
            if on && !(code == 4 && sub == Some(0)) {
                self.style |= group;
            } else {
                self.style &= !group;
            }
        }
    }

    fn control_string(&mut self, byte: u8, out: &mut Vec<u8>) {
        match byte {
            0x07 => self.state = FilterState::Ground,
            0x1b => self.state = FilterState::ControlStringEscape,
            0xc2 => self.state = FilterState::ControlStringC1Lead,
            // Any other control ends an unterminated string, so it hides at
            // most the rest of its line.
            0x00..=0x1f | 0x7f => {
                self.state = FilterState::Ground;
                self.ground(byte, out);
            }
            _ => {}
        }
    }
}

fn is_sgr(params: &[u8]) -> bool {
    params.len() <= MAX_SGR_LEN
        && params
            .iter()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b';' | b':'))
}

/// SGR parameters without blinking (5, 6) or hidden text (8), which stored
/// text must not switch on; `None` when nothing else is left. A color given
/// as `38;5;n` or `38;2;r;g;b` keeps all of its parameters.
fn visible_sgr(params: &[u8]) -> Option<Vec<u8>> {
    let mut kept: Vec<&[u8]> = Vec::new();
    let mut dropped = false;
    let mut params = params.split(|&byte| byte == b';').peekable();
    while let Some(param) = params.next() {
        let mut parts = param.split(|&byte| byte == b':');
        let code = parts.next().map_or(0, sgr_number);
        let colon_form = parts.next().is_some();
        if matches!(code, 38 | 48 | 58) && !colon_form {
            kept.push(param);
            let color = match params.peek().map(|next| sgr_number(next)) {
                Some(5) => 2,
                Some(2) => 4,
                _ => 0,
            };
            kept.extend(params.by_ref().take(color));
        } else if matches!(code, 5 | 6 | 8) {
            dropped = true;
        } else {
            kept.push(param);
        }
    }
    (!dropped || !kept.is_empty()).then(|| kept.join(&b';'))
}

fn sgr_number(digits: &[u8]) -> u32 {
    digits.iter().fold(0, |number: u32, digit| {
        number
            .saturating_mul(10)
            .saturating_add(u32::from(digit.wrapping_sub(b'0')))
    })
}

/// The attribute group an SGR code switches, as a bit, and whether it switches
/// it on. Codes without a reset of their own share a group only `0` clears.
fn sgr_group(code: u32) -> (u16, bool) {
    let (bit, on) = match code {
        1 | 2 => (0, true),
        22 => (0, false),
        3 | 20 => (1, true),
        23 => (1, false),
        4 | 21 => (2, true),
        24 => (2, false),
        5 | 6 => (3, true),
        25 => (3, false),
        7 => (4, true),
        27 => (4, false),
        8 => (5, true),
        28 => (5, false),
        9 => (6, true),
        29 => (6, false),
        30..=38 | 90..=97 => (7, true),
        39 => (7, false),
        40..=48 | 100..=107 => (8, true),
        49 => (8, false),
        58 => (9, true),
        59 => (9, false),
        _ => (10, true),
    };
    (1 << bit, on)
}

pub fn title_width(term_width: usize) -> usize {
    title_width_beside(term_width, id_prefix_width(term_width))
}

/// How wide titles may be next to IDs `id_width` wide.
fn title_width_beside(term_width: usize, id_width: usize) -> usize {
    let used = id_width + fixed_list_width(shows_tokens(term_width));
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

/// Renders the session list. `term_width` is the terminal width when stdout is a
/// terminal, which selects the fitted table; `None` selects the plain layout.
/// Both show each session's [`SessionSummary::token_estimate`] as TOKENS, the
/// table only where it fits (see [`shows_tokens`]).
/// Color is independent of the layout. The plain layout and `render_show` keep
/// stored text byte for byte; a terminal writer applies [`TerminalFilter`].
///
/// The table's colors come from crossterm, which also drops them while
/// NO_COLOR is set, unless the program called
/// `crossterm::style::force_color_output(true)` as the binary does.
pub fn render_list(
    sessions: &[SessionSummary],
    use_color: bool,
    term_width: Option<usize>,
) -> String {
    let ids: Vec<&str> = sessions.iter().map(|s| s.id.as_str()).collect();
    render_list_among(sessions, &ids, use_color, term_width)
}

/// Renders `sessions` as [`render_list`] does, with the table's IDs shortened
/// only as far as they stay distinct among the IDs `among`: those of every
/// session `show` looks through, of which `--limit` lists only the first.
pub fn render_list_among(
    sessions: &[SessionSummary],
    among: &[&str],
    use_color: bool,
    term_width: Option<usize>,
) -> String {
    if sessions.is_empty() {
        return "No sessions found\n".to_string();
    }
    match term_width {
        Some(width) => render_list_table(sessions, among, use_color, width),
        None => render_list_plain(sessions, use_color),
    }
}

fn render_list_plain(sessions: &[SessionSummary], use_color: bool) -> String {
    let mut out = format!("Found {} session(s)\n\n", sessions.len());
    let header = format!(
        "{:<id_w$}  {:<src_w$}  {:>msgs_w$}  {:>tok_w$}  {:<upd_w$}  TITLE",
        "ID",
        "SOURCE",
        "MSGS",
        "TOKENS",
        "UPDATED",
        id_w = ID_FULL_WIDTH,
        src_w = SOURCE_WIDTH,
        msgs_w = MSGS_WIDTH,
        tok_w = PLAIN_TOKENS_WIDTH,
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
            "{:<id_w$}  {}  {:>msgs_w$}  {:>tok_w$}  {}  {}\n",
            session.id,
            paint_source_text(&source, session.source, use_color),
            session.message_count,
            session.token_estimate(),
            paint_dim(&updated, use_color),
            session.title,
            id_w = ID_FULL_WIDTH,
            msgs_w = MSGS_WIDTH,
            tok_w = PLAIN_TOKENS_WIDTH,
        ));
    }
    out
}

fn render_list_table(
    sessions: &[SessionSummary],
    among: &[&str],
    use_color: bool,
    term_width: usize,
) -> String {
    let width = u16::try_from(term_width).unwrap_or(u16::MAX);
    let term_width = usize::from(width);
    let mut table = Table::new();
    table.load_preset(presets::UTF8_FULL_CONDENSED);
    table.set_content_arrangement(ContentArrangement::Dynamic);
    // Styling follows `use_color`, never comfy-table's own stdout check.
    table.force_no_tty();
    if use_color {
        table.enforce_styling();
    }
    table.set_width(width);
    let tokens = shows_tokens(term_width);
    let mut header = vec![
        header_cell("ID"),
        header_cell("SOURCE"),
        header_cell("MSGS"),
    ];
    if tokens {
        header.push(header_cell("TOKENS"));
    }
    header.extend([header_cell("UPDATED"), header_cell("TITLE")]);
    table.set_header(header);

    let id_width = distinct_id_width(among, id_prefix_width(term_width));
    let max_title = title_width_beside(term_width, id_width);
    for session in sessions {
        let mut row = vec![
            Cell::new(shorten_id(&one_line(&session.id), id_width)),
            source_cell(session.source),
            Cell::new(session.message_count),
        ];
        if tokens {
            row.push(Cell::new(session.token_estimate()));
        }
        row.extend([
            Cell::new(session.updated_display()).fg(Color::DarkGrey),
            Cell::new(truncate_width(&one_line(&session.title), max_title)),
        ]);
        table.add_row(row);
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
    lines.push(paint_bold(&session.title, use_color));
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
        paint_dim(&session.created_utc(), use_color)
    ));
    lines.push(format!(
        "updated:   {}",
        paint_dim(&session.updated_utc(), use_color)
    ));
    lines.push(format!("messages:  {}", session.message_count));
    lines.push(format!(
        "tokens:    ~{} (estimate)",
        session.token_estimate()
    ));
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
            message.timestamp_display().as_deref(),
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

    fn sample_summary() -> SessionSummary {
        SessionSummary {
            workspace: Some("/Users/manuel.romero".into()),
            created_at_ms: Some(1_700_000_000_000),
            updated_at_ms: Some(1_700_000_100_000),
            model: Some("grok-4.6".into()),
            ..SessionSummary::new(
                "f4eea6d2-d2d3-41ad-b290-824445295a15",
                "Langfuse Semantic Layer",
                Source::Agent,
            )
        }
    }

    fn sample_session() -> Session {
        Session::new(
            sample_summary(),
            vec![
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
        )
    }

    fn summaries(sessions: &[Session]) -> Vec<SessionSummary> {
        sessions
            .iter()
            .map(|session| session.summary.clone())
            .collect()
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
                std::slice::from_ref(&session.summary),
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
        assert!(has_ansi(&paint_role("tool", true)));
        assert!(!has_ansi(&paint_role("tool", false)));
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
        let rendered = render_list(&[sample_summary()], false, None);
        assert!(rendered.contains("Langfuse Semantic Layer"));
        assert!(rendered.contains("f4eea6d2-d2d3-41ad-b290-824445295a15"));
        assert!(!has_ansi(&rendered));
    }

    #[test]
    fn id_prefix_snaps_to_uuid_groups() {
        assert_eq!(id_prefix_width(200), 36);
        assert_eq!(id_prefix_width(85), 13);
        // TOKENS takes what a longer ID would.
        assert_eq!(id_prefix_width(80), 8);
        assert_eq!(id_prefix_width(50), 8);
        // Without room for it, there is no TOKENS column to make room for.
        assert!(shows_tokens(79) && !shows_tokens(78));
        assert_eq!(id_prefix_width(78), 13);
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
    fn shortened_ids_stay_distinct() {
        let with_id = |id: &str| SessionSummary {
            id: id.into(),
            ..sample_summary()
        };
        let sessions = [
            with_id("f4eea6d2-d2d3-41ad-b290-824445295a15"),
            with_id("f4eea6d2-d2d3-9999-b290-824445295a15"),
        ];
        let rendered = render_list(&sessions, false, Some(80));
        assert!(rendered.contains("│ f4eea6d2-d2d3-41ad ┆"), "{rendered}");
        assert!(rendered.contains("│ f4eea6d2-d2d3-9999 ┆"), "{rendered}");
        assert!(
            rendered.contains("IDs shortened to 18 chars;"),
            "{rendered}"
        );
        // `show` matches a prefix in any case: only the full IDs tell these apart.
        let cased = [
            "abcdef00-0000-4000-8000-000000000001",
            "ABCDEF00-0000-4000-8000-000000000001",
        ];
        assert_eq!(distinct_id_width(&cased, 8), ID_FULL_WIDTH);
        assert_eq!(distinct_id_width(&[sessions[0].id.as_str()], 8), 8);
        // Listing only the first, its ID stays distinct from the second's.
        let ids = [sessions[0].id.as_str(), sessions[1].id.as_str()];
        let first = render_list_among(&sessions[..1], &ids, false, Some(80));
        assert!(first.contains("│ f4eea6d2-d2d3-41ad ┆"), "{first}");
        assert!(!first.contains("9999"), "{first}");
    }

    #[test]
    fn plain_list_keeps_large_token_counts_aligned() {
        let sized = |id: &str, chars: usize| SessionSummary {
            message_count: 12_345,
            content_chars: chars,
            ..SessionSummary::new(id, "t", Source::Agent)
        };
        let sessions = [
            sized("a", 7),
            sized("b", 4_000_000),
            sized("c", 399_999_996),
        ];
        let plain = render_list(&sessions, false, None);
        let lines: Vec<&str> = plain
            .lines()
            .skip(2)
            .filter(|l| !l.starts_with('-'))
            .collect();
        assert!(lines[3].contains("  99999999  "), "{plain}");
        // Every row's UPDATED starts where its header does.
        let updated = lines[0].find("UPDATED").unwrap();
        for row in &lines[1..] {
            assert_eq!(row.find('—'), Some(updated), "{plain}");
        }
    }

    #[test]
    fn narrow_tty_list_shortens_id() {
        let rendered = render_list(&[sample_summary()], true, Some(89));
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
        // As in the binary, so the test also passes under NO_COLOR.
        crossterm::style::force_color_output(true);
        let sessions = sample_sessions();
        for width in [40, 60, 80, 120, 200] {
            let colored = render_list(&summaries(&sessions), true, Some(width));
            let plain = render_list(&summaries(&sessions), false, Some(width));
            // Cyan SOURCE cell.
            assert!(colored.contains("\u{1b}[38;5;14m"));
            assert!(!has_ansi(&plain));
            assert!(plain.contains('│'));
            assert_eq!(strip_ansi(&colored), plain);
        }
    }

    #[test]
    fn piped_color_keeps_plain_layout() {
        let sessions = sample_sessions();
        let colored = render_list(&summaries(&sessions), true, None);
        let plain = render_list(&summaries(&sessions), false, None);
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
        let sessions: Vec<SessionSummary> = titles
            .iter()
            .map(|title| SessionSummary {
                title: title.clone(),
                ..sample_summary()
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
    fn titles_are_cut_by_terminal_columns() {
        let cases = [
            ("Langfuse", 8, "Langfuse"),
            ("Langfuse", 5, "Lang…"),
            ("データパイプラインの再設計", 17, "データパイプライ…"),
            ("データパイプライン", 4, "デ…"),
            ("e\u{301}e\u{301}e\u{301}", 2, "e\u{301}…"),
            ("ok \u{2764}\u{fe0f} done", 6, "ok \u{2764}\u{fe0f}…"),
            ("ok \u{2764}\u{fe0f} done", 5, "ok \u{2764}…"),
            ("abc", 0, ""),
        ];
        for (title, max, expected) in cases {
            let cut = truncate_width(title, max);
            assert_eq!(cut, expected, "{title:?} at {max}");
            assert!(cut.width() <= max, "{cut:?}");
        }
    }

    #[test]
    fn wide_title_leaves_the_full_id_on_one_line() {
        let mut session = sample_session();
        session.title = "データパイプラインの再設計レビュー 🚀 — überprüfe den Ablauf".into();
        for width in [80, 100, 120] {
            let rendered = render_list(std::slice::from_ref(&session.summary), false, Some(width));
            let id = shorten_id(&session.id, id_prefix_width(width));
            assert!(rendered.contains(&format!("│ {id} ┆")), "{rendered}");
            // Found, blank line, top border, header, separator, row, bottom border.
            assert!(
                rendered.lines().nth(6).unwrap().starts_with('└'),
                "{rendered}"
            );
            assert!(rendered.lines().all(|line| line.width() <= width));
        }
    }

    fn control_session() -> Session {
        let mut session = sample_session();
        session.title = "Edge \u{1b}]0;pwned\u{7} title\twith\nnewline".into();
        session.messages = vec![
            Message {
                role: "user".into(),
                content: "line one\r\nline two\r\n".into(),
                timestamp: None,
            },
            Message {
                role: "assistant".into(),
                content: "colored \u{1b}[31mred\u{1b}[0m output and bell \u{7} done".into(),
                timestamp: None,
            },
        ];
        session
    }

    #[test]
    fn plain_layout_and_show_keep_stored_text_verbatim() {
        let session = control_session();
        let plain = render_list(std::slice::from_ref(&session.summary), false, None);
        let row = format!(
            "{}  agent       2         2  {:<16}  Edge \u{1b}]0;pwned\u{7} title\twith\nnewline\n",
            session.id,
            session.updated_display()
        );
        assert!(plain.ends_with(&row), "{plain:?}");

        let shown = render_show(&session, &session.messages, None, false);
        assert!(shown.starts_with("Edge \u{1b}]0;pwned\u{7} title\twith\nnewline\nid: "));
        assert!(shown.ends_with(
            "messages:  2\n\
             tokens:    ~2 (estimate)\n\
             \n[user]\nline one\r\nline two\r\n\n\
             \n[assistant]\ncolored \u{1b}[31mred\u{1b}[0m output and bell \u{7} done\n"
        ));
    }

    #[test]
    fn table_rows_drop_escape_sequences_and_stay_on_one_line() {
        assert_eq!(
            one_line("Edge \u{1b}]0;pwned\u{7} title\twith\nnewline"),
            "Edge  title with newline"
        );
        assert_eq!(one_line("plain title"), "plain title");

        let session = control_session();
        let rendered = render_list(std::slice::from_ref(&session.summary), false, Some(200));
        assert!(!rendered.chars().any(|c| c.is_control() && c != '\n'));
        assert!(
            rendered.contains("┆ Edge  title with newline │"),
            "{rendered}"
        );
        assert_eq!(rendered.lines().count(), 7);
    }

    fn filtered(text: &str, keep_sgr: bool) -> String {
        let mut whole = Vec::new();
        TerminalFilter::new(keep_sgr).push(text.as_bytes(), &mut whole);
        let mut split = Vec::new();
        let mut filter = TerminalFilter::new(keep_sgr);
        for byte in text.as_bytes() {
            filter.push(std::slice::from_ref(byte), &mut split);
        }
        assert_eq!(whole, split, "{text:?}");
        String::from_utf8(whole).unwrap()
    }

    #[test]
    fn terminal_filter_removes_whole_sequences() {
        let cases = [
            ("line one\r\nline two\r\n", "line one\r\nline two\r\n"),
            (
                "colored \u{1b}[31mred\u{1b}[0m output and bell \u{7} done",
                "colored red output and bell  done",
            ),
            (
                "form\u{c}feed, lone\rcr, del\u{7f}, nul\0",
                "formfeed, lonecr, del, nul",
            ),
            ("Edge \u{1b}]0;pwned\u{7} title", "Edge  title"),
            (
                "osc8 \u{1b}]8;;http://x\u{1b}\\link\u{1b}]8;;\u{1b}\\ end",
                "osc8 link end",
            ),
            (
                "c1 \u{9b}2Jcsi \u{9d}0;t\u{9c}ok \u{85}nel",
                "c1 csi ok nel",
            ),
            (
                "\u{1b}[2J\u{1b}[Hx \u{1b}[?25ly \u{1b}(Bz \u{1b}cw",
                "x y z w",
            ),
            (
                "unterminated \u{1b}]0;title\nnext line",
                "unterminated \nnext line",
            ),
            ("dcs \u{1b}Pq#0\u{1b}\\ done", "dcs  done"),
            (
                "tab\tand 日本語 é ñ 👨‍👩‍👧 \u{a0}nbsp",
                "tab\tand 日本語 é ñ 👨‍👩‍👧 \u{a0}nbsp",
            ),
            ("trailing\r", "trailing"),
        ];
        for (input, expected) in cases {
            assert_eq!(filtered(input, false), expected, "{input:?}");
        }
    }

    #[test]
    fn terminal_filter_keeps_only_sgr_when_asked() {
        assert_eq!(
            filtered(
                "a \u{1b}[1;38;5;14mb\u{1b}[0m c \u{1b}[2J\u{1b}[>4;2m",
                true
            ),
            "a \u{1b}[1;38;5;14mb\u{1b}[0m c "
        );
        // Only the 7-bit form passes, as this program writes it.
        assert_eq!(filtered("x\u{9b}31my", true), "xy");
        // Text cannot blink or hide; colors whose parameters are 5 or 8 stay.
        assert_eq!(
            filtered(
                "a\u{1b}[1;5;31mb\u{1b}[6mc\u{1b}[8md\u{1b}[38;5;8;48;2;5;6;8me\u{1b}[m",
                true
            ),
            "a\u{1b}[1;31mbcd\u{1b}[38;5;8;48;2;5;6;8me\u{1b}[m"
        );
        assert_eq!(visible_sgr(b"5;8"), None);
        assert_eq!(visible_sgr(b"").as_deref(), Some(&b""[..]));
        assert_eq!(visible_sgr(b"38:5:8;8").as_deref(), Some(&b"38:5:8"[..]));
        let long = format!("\u{1b}[{}m", "1;".repeat(MAX_SGR_LEN));
        assert_eq!(filtered(&long, true), "");

        let session = sample_session();
        let colored = render_list(std::slice::from_ref(&session.summary), true, Some(120));
        assert_eq!(filtered(&colored, true), colored);
        let colored = render_show(&session, &session.messages, Some(2), true);
        assert_eq!(filtered(&colored, true), colored);
    }

    #[test]
    fn terminal_filter_ends_stored_styling_at_the_line_end() {
        let cases = [
            (
                "visible \u{1b}[8mhidden\r\n\u{1b}[41;5mleft on\nnext",
                "visible hidden\r\n\u{1b}[41mleft on\u{1b}[0m\nnext",
            ),
            (
                "\u{1b}[1;31mx\u{1b}[22;39m\n",
                "\u{1b}[1;31mx\u{1b}[22;39m\n",
            ),
            ("\u{1b}[38;5;8mx\u{1b}[39m\n", "\u{1b}[38;5;8mx\u{1b}[39m\n"),
            (
                "\u{1b}[38;2;1;2;3mx\u{1b}[39m\n",
                "\u{1b}[38;2;1;2;3mx\u{1b}[39m\n",
            ),
            ("\u{1b}[4:3mx\u{1b}[4:0m\n", "\u{1b}[4:3mx\u{1b}[4:0m\n"),
            ("\u{1b}[7mx\u{1b}[m\n", "\u{1b}[7mx\u{1b}[m\n"),
            ("\u{1b}[7mx\u{1b}[39m\n", "\u{1b}[7mx\u{1b}[39m\u{1b}[0m\n"),
            (
                "\u{1b}[53mx\u{1b}[55m\n",
                "\u{1b}[53mx\u{1b}[55m\u{1b}[0m\n",
            ),
        ];
        for (input, expected) in cases {
            assert_eq!(filtered(input, true), expected, "{input:?}");
        }
        assert_eq!(filtered("\u{1b}[8mx\n", false), "x\n");

        let mut filter = TerminalFilter::new(true);
        let mut out = Vec::new();
        filter.push(b"\x1b[41mred", &mut out);
        filter.reset_style(&mut out);
        filter.reset_style(&mut out);
        assert_eq!(out, b"\x1b[41mred\x1b[0m");
    }

    #[test]
    fn terminal_width_falls_back_to_columns_then_80() {
        assert_eq!(width_from(Some(120), Some("60")), 120);
        assert_eq!(width_from(Some(10), None), MIN_TERM_WIDTH);
        assert_eq!(width_from(Some(0), Some(" 60 ")), 60);
        assert_eq!(width_from(None, Some("1")), MIN_TERM_WIDTH);
        for columns in [None, Some(""), Some("0"), Some("wide"), Some("-5")] {
            assert_eq!(width_from(None, columns), DEFAULT_TERM_WIDTH, "{columns:?}");
        }
    }

    #[test]
    fn comfy_table_shares_our_crossterm() {
        // The binary's `force_color_output` reaches comfy-table only when both
        // resolve to the same crossterm.
        let lock =
            std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.lock")).unwrap();
        let crossterms = lock
            .lines()
            .filter(|line| line.trim_end() == r#"name = "crossterm""#)
            .count();
        assert_eq!(crossterms, 1);
    }
}
