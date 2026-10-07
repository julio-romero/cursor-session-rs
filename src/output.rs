use std::env;
use std::ffi::OsStr;
use std::io::{self, BufWriter, ErrorKind, Write};

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
        OutputOpts {
            tty,
            color: use_color(tty, choice, no_color.as_deref(), term.as_deref()),
            width: tty.then(ui::terminal_width),
        }
    }
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

/// Buffered writer that remembers whether the reader went away (EPIPE), so the
/// process can exit quietly however the error was wrapped on its way up.
pub struct PipeWriter<W: Write> {
    inner: BufWriter<W>,
    closed: bool,
}

impl<W: Write> PipeWriter<W> {
    pub fn new(inner: W) -> Self {
        Self {
            inner: BufWriter::new(inner),
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
        let mut out = PipeWriter::new(Failing(ErrorKind::BrokenPipe));
        assert!(write!(out, "buffered").is_ok());
        assert!(!out.closed());
        assert!(out.flush().is_err());
        assert!(out.closed());
    }

    #[test]
    fn pipe_writer_ignores_other_failures() {
        let mut out = PipeWriter::new(Failing(ErrorKind::PermissionDenied));
        assert!(out.write_all(&[b'x'; 64 * 1024]).is_err());
        assert!(!out.closed());
    }

    #[test]
    fn ignore_errors_swallows_failures() {
        let mut err = IgnoreErrors(Failing(ErrorKind::BrokenPipe));
        assert!(writeln!(err, "warning: lost").is_ok());
        assert!(err.flush().is_ok());
    }
}
