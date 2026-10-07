use std::env;
use std::ffi::OsStr;
use std::io::{self, ErrorKind, Write};

use cursor_session::ui;

use crate::cli::ColorChoice;

/// Rendering decisions for stdout, resolved once at startup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputOpts {
    /// Stdout is a terminal: table layout and the short default `show` window.
    pub tty: bool,
    pub color: bool,
    /// Terminal width, only queried when stdout is a terminal.
    pub width: Option<usize>,
}

impl OutputOpts {
    pub fn detect(choice: ColorChoice) -> OutputOpts {
        let tty = ui::stdout_is_tty();
        let no_color = env::var_os("NO_COLOR");
        let term = env::var_os("TERM");
        let mut color = use_color(tty, choice, no_color.as_deref(), term.as_deref());
        if tty {
            color = console_color(color, choice, enable_escape_sequences);
        }
        OutputOpts {
            tty,
            color,
            width: tty.then(ui::terminal_width),
        }
    }
}

/// A Windows console prints escape sequences as text until virtual terminal
/// processing is on. `enable` turns it on and says whether that worked; `auto`
/// then keeps color only if it did, `always` keeps it anyway.
fn console_color(color: bool, choice: ColorChoice, enable: impl FnOnce() -> bool) -> bool {
    color && (enable() || choice == ColorChoice::Always)
}

/// Turns on virtual terminal processing for the console, which crossterm
/// otherwise does only when it styles text itself (the table, not `show`).
#[cfg(windows)]
fn enable_escape_sequences() -> bool {
    crossterm::ansi_support::supports_ansi()
}

#[cfg(not(windows))]
fn enable_escape_sequences() -> bool {
    true
}

/// `auto` colors only a terminal, and only when NO_COLOR is unset or empty and
/// TERM is not `dumb`. `always` and `never` ignore the environment.
pub fn use_color(
    is_tty: bool,
    choice: ColorChoice,
    no_color: Option<&OsStr>,
    term: Option<&OsStr>,
) -> bool {
    match choice {
        ColorChoice::Always => true,
        ColorChoice::Never => false,
        ColorChoice::Auto => {
            is_tty
                && no_color.is_none_or(OsStr::is_empty)
                && term.is_none_or(|term| term != OsStr::new("dumb"))
        }
    }
}

/// Writer that remembers whether the reader went away (EPIPE), so the process
/// can exit quietly however the error was wrapped on its way up.
pub struct PipeWriter<W: Write> {
    inner: W,
    closed: bool,
}

impl<W: Write> PipeWriter<W> {
    pub fn new(inner: W) -> Self {
        Self {
            inner,
            closed: false,
        }
    }

    pub fn closed(&self) -> bool {
        self.closed
    }

    fn track<T>(&mut self, result: io::Result<T>) -> io::Result<T> {
        if let Err(error) = &result
            && error.kind() == ErrorKind::BrokenPipe
        {
            self.closed = true;
        }
        result
    }
}

impl<W: Write> Write for PipeWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let result = self.inner.write(buf);
        self.track(result)
    }

    fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        let result = self.inner.write_all(buf);
        self.track(result)
    }

    fn flush(&mut self) -> io::Result<()> {
        let result = self.inner.flush();
        self.track(result)
    }
}

/// Writer for a terminal: stored text goes through `ui::TerminalFilter`, so it
/// cannot move the cursor, retitle the window or ring the bell. `keep_color`
/// lets SGR styling through, the only escape sequences this program writes.
pub struct TerminalWriter<W: Write> {
    inner: W,
    filter: ui::TerminalFilter,
    buf: Vec<u8>,
}

impl<W: Write> TerminalWriter<W> {
    pub fn new(inner: W, keep_color: bool) -> Self {
        Self {
            inner,
            filter: ui::TerminalFilter::new(keep_color),
            buf: Vec::new(),
        }
    }
}

impl<W: Write> Write for TerminalWriter<W> {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.buf.clear();
        self.filter.push(data, &mut self.buf);
        self.inner.write_all(&self.buf)?;
        Ok(data.len())
    }

    /// Also resets styling that stored text left on, so it cannot reach the
    /// shell prompt.
    fn flush(&mut self) -> io::Result<()> {
        self.buf.clear();
        self.filter.reset_style(&mut self.buf);
        self.inner.write_all(&self.buf)?;
        self.inner.flush()
    }
}

/// Writer for diagnostics: a failing stderr must never fail or abort the program.
pub struct IgnoreErrors<W: Write>(pub W);

impl<W: Write> Write for IgnoreErrors<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let _ = self.0.write_all(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        let _ = self.0.flush();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::io::BufWriter;

    use super::*;

    const UNSET: Option<&OsStr> = None;

    fn os(value: &str) -> Option<&OsStr> {
        Some(OsStr::new(value))
    }

    #[test]
    fn auto_colors_only_a_capable_terminal() {
        assert!(use_color(
            true,
            ColorChoice::Auto,
            UNSET,
            os("xterm-256color")
        ));
        assert!(use_color(true, ColorChoice::Auto, UNSET, None));
        assert!(use_color(true, ColorChoice::Auto, os(""), os("xterm")));
        assert!(!use_color(false, ColorChoice::Auto, UNSET, os("xterm")));
        assert!(!use_color(true, ColorChoice::Auto, os("1"), os("xterm")));
        assert!(!use_color(true, ColorChoice::Auto, os("0"), os("xterm")));
        assert!(!use_color(true, ColorChoice::Auto, UNSET, os("dumb")));
    }

    #[test]
    fn always_and_never_ignore_the_environment() {
        for tty in [true, false] {
            for no_color in [UNSET, os("1")] {
                for term in [None, os("dumb"), os("xterm")] {
                    assert!(use_color(tty, ColorChoice::Always, no_color, term));
                    assert!(!use_color(tty, ColorChoice::Never, no_color, term));
                }
            }
        }
    }

    #[test]
    fn a_console_without_escape_sequences_gets_color_only_when_forced() {
        let unsupported = || false;
        assert!(!console_color(true, ColorChoice::Auto, unsupported));
        assert!(console_color(true, ColorChoice::Always, unsupported));
        assert!(console_color(true, ColorChoice::Auto, || true));
        // Without color the console is left as it is.
        let untouched = || panic!("console mode changed");
        assert!(!console_color(false, ColorChoice::Never, untouched));
        assert!(!console_color(false, ColorChoice::Auto, untouched));
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_environment_is_accepted() {
        use std::os::unix::ffi::OsStrExt;
        let raw = OsStr::from_bytes(b"\xff\xfe");
        assert!(!use_color(true, ColorChoice::Auto, Some(raw), None));
        assert!(use_color(true, ColorChoice::Auto, None, Some(raw)));
    }

    struct Failing(ErrorKind);

    impl Write for Failing {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(self.0.into())
        }

        fn flush(&mut self) -> io::Result<()> {
            Err(self.0.into())
        }
    }

    #[test]
    fn pipe_writer_records_a_closed_reader_on_flush() {
        let mut out = PipeWriter::new(BufWriter::new(Failing(ErrorKind::BrokenPipe)));
        assert!(write!(out, "buffered").is_ok());
        assert!(!out.closed());
        assert!(out.flush().is_err());
        assert!(out.closed());
    }

    #[test]
    fn pipe_writer_records_a_closed_reader_on_write() {
        let mut out = PipeWriter::new(Failing(ErrorKind::BrokenPipe));
        assert!(writeln!(out, "line").is_err());
        assert!(out.closed());
    }

    #[test]
    fn pipe_writer_ignores_other_failures() {
        let mut out = PipeWriter::new(Failing(ErrorKind::PermissionDenied));
        assert!(out.write_all(&[b'x'; 64 * 1024]).is_err());
        assert!(!out.closed());
    }

    #[test]
    fn terminal_writer_filters_across_writes() {
        let mut out = TerminalWriter::new(Vec::new(), false);
        for chunk in [
            "title \u{1b}]0;pw",
            "ned\u{7} ok\r",
            "\nred \u{1b}[3",
            "1mtext\u{1b}[0m\n",
        ] {
            out.write_all(chunk.as_bytes()).unwrap();
        }
        assert_eq!(out.inner, b"title  ok\r\nred text\n");

        let mut out = TerminalWriter::new(Vec::new(), true);
        write!(out, "\u{1b}[36magent\u{1b}[39m \u{1b}[2J").unwrap();
        assert_eq!(out.inner, b"\x1b[36magent\x1b[39m ");
    }

    #[test]
    fn terminal_writer_resets_stored_styling_on_flush() {
        let mut out = TerminalWriter::new(Vec::new(), true);
        write!(out, "visible \u{1b}[8mhidden \u{1b}[41;5mleft on").unwrap();
        out.flush().unwrap();
        assert!(out.inner.ends_with(b"left on\x1b[0m"));
        out.flush().unwrap();
        assert!(out.inner.ends_with(b"left on\x1b[0m"));

        let mut out = TerminalWriter::new(Vec::new(), true);
        write!(out, "\u{1b}[36magent\u{1b}[39m").unwrap();
        out.flush().unwrap();
        assert_eq!(out.inner, b"\x1b[36magent\x1b[39m");
    }

    #[test]
    fn ignore_errors_swallows_failures() {
        let mut err = IgnoreErrors(Failing(ErrorKind::BrokenPipe));
        assert!(writeln!(err, "warning: lost").is_ok());
        assert!(err.flush().is_ok());
    }
}
