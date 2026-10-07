mod cli;
mod commands;
mod output;

use std::env;
use std::error::Error as StdError;
use std::ffi::OsString;
use std::io::{self, BufWriter, ErrorKind, Write};
use std::path::Path;
use std::process::ExitCode;

use anyhow::Result;
use clap::{CommandFactory, FromArgMatches};
use cursor_session::detect::StoragePaths;

use crate::cli::Cli;
use crate::output::{IgnoreErrors, OutputOpts, PipeWriter, TerminalWriter};

fn main() -> ExitCode {
    let cli = match parse_cli(env::args_os().collect()) {
        Ok(cli) => cli,
        Err(error) => {
            // Help and version go to stdout and exit 0; usage errors exit 2.
            let _ = error.print();
            return ExitCode::from(u8::try_from(error.exit_code()).unwrap_or(2));
        }
    };

    let opts = OutputOpts::detect(cli.color);
    if opts.color {
        // `opts.color` already applied --color and NO_COLOR. crossterm, which
        // colors the table, would otherwise apply NO_COLOR again.
        crossterm::style::force_color_output(true);
    }
    let mut out = PipeWriter::new(stdout_sink(&opts));
    let mut err = diagnostics(io::stderr().lock());
    let result = run(cli, &opts, &mut out, &mut err).and_then(|()| Ok(out.flush()?));
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) if out.closed() || is_broken_pipe(&error) => ExitCode::SUCCESS,
        Err(error) => {
            let _ = out.flush();
            report(&error, &mut err);
            ExitCode::FAILURE
        }
    }
}

/// Parses the command line. clap renders help and usage errors before `--color`
/// has a parsed value, so its styling is taken from the raw arguments.
fn parse_cli(args: Vec<OsString>) -> Result<Cli, clap::Error> {
    let mut command = Cli::command().color(clap_color(&args));
    let mut matches = command.try_get_matches_from_mut(args)?;
    Cli::from_arg_matches_mut(&mut matches).map_err(|error| error.format(&mut command))
}

/// The last `--color WHEN` or `--color=WHEN` before `--`; anything unexpected is
/// left to clap to report.
fn clap_color(args: &[OsString]) -> clap::ColorChoice {
    let mut choice = clap::ColorChoice::Auto;
    let mut args = args.iter().skip(1).map(|arg| arg.to_str());
    while let Some(arg) = args.next() {
        let value = match arg {
            Some("--") => break,
            Some("--color") => args.next().flatten(),
            Some(arg) => arg.strip_prefix("--color="),
            None => None,
        };
        choice = match value {
            Some("auto") => clap::ColorChoice::Auto,
            Some("always") => clap::ColorChoice::Always,
            Some("never") => clap::ColorChoice::Never,
            _ => choice,
        };
    }
    choice
}

/// A terminal gets std's line buffering, so progress lines show as they are
/// written, and only text that is safe to display. A pipe gets every byte,
/// fully buffered.
fn stdout_sink(opts: &OutputOpts) -> Box<dyn Write> {
    let stdout = io::stdout().lock();
    if opts.tty {
        Box::new(TerminalWriter::new(stdout, opts.color))
    } else {
        Box::new(BufWriter::new(stdout))
    }
}

/// Stderr never carries styling of ours, so every escape sequence in stored text
/// (titles in hints, paths in warnings) is removed.
fn diagnostics<W: Write>(stderr: W) -> IgnoreErrors<TerminalWriter<W>> {
    IgnoreErrors(TerminalWriter::new(stderr, false))
}

fn run(cli: Cli, opts: &OutputOpts, out: &mut dyn Write, err: &mut dyn Write) -> Result<()> {
    let paths = resolve_paths(cli.storage.as_deref())?;
    if cli.verbose {
        writeln!(err, "chats: {:?}", paths.chats_dir)?;
        writeln!(err, "projects: {:?}", paths.projects_dir)?;
        writeln!(err, "ide db: {:?}", paths.global_storage_db)?;
    }

    commands::run(cli.command, &paths, opts, out, err)
}

fn resolve_paths(storage: Option<&Path>) -> Result<StoragePaths> {
    let paths = match storage {
        Some(path) => StoragePaths::from_custom(path, None)?,
        None => StoragePaths::detect()?,
    };
    Ok(paths)
}

/// Prints the error, its causes, and any hints from the library error. Write
/// failures are ignored: there is nowhere left to report them.
fn report(error: &anyhow::Error, err: &mut dyn Write) {
    let _ = writeln!(err, "error: {error}");
    for cause in error.chain().skip(1) {
        let _ = writeln!(err, "  caused by: {cause}");
    }
    let hints = error
        .chain()
        .find_map(|cause| cause.downcast_ref::<cursor_session::Error>())
        .map(cursor_session::Error::hints)
        .unwrap_or_default();
    for hint in hints {
        let _ = writeln!(err, "{hint}");
    }
}

fn is_broken_pipe(error: &anyhow::Error) -> bool {
    error
        .chain()
        .any(|cause| io_error_kind(cause) == Some(ErrorKind::BrokenPipe))
}

/// The io::ErrorKind behind a cause, including errors that wrap an io::Error
/// transparently and so never expose it through `source()`.
fn io_error_kind(cause: &(dyn StdError + 'static)) -> Option<ErrorKind> {
    if let Some(error) = cause.downcast_ref::<io::Error>() {
        return Some(error.kind());
    }
    if let Some(error) = cause.downcast_ref::<serde_json::Error>() {
        return error.io_error_kind();
    }
    match cause.downcast_ref::<cursor_session::Error>()? {
        cursor_session::Error::Write(error) => Some(error.kind()),
        cursor_session::Error::Json(error) => error.io_error_kind(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use anyhow::Context;
    use cursor_session::Error;

    use super::*;

    fn reported(error: &anyhow::Error) -> String {
        let mut buf = Vec::new();
        report(error, &mut buf);
        String::from_utf8(buf).unwrap()
    }

    fn args(args: &[&str]) -> Vec<OsString> {
        std::iter::once("cursor-session")
            .chain(args.iter().copied())
            .map(OsString::from)
            .collect()
    }

    #[test]
    fn usage_errors_exit_2_and_help_exits_0() {
        let code = |argv: &[&str]| parse_cli(args(argv)).err().map(|e| e.exit_code());
        assert_eq!(code(&["list", "--bogus"]), Some(2));
        assert_eq!(code(&["--color", "sometimes", "list"]), Some(2));
        assert_eq!(code(&[]), Some(2));
        assert_eq!(code(&["--help"]), Some(0));
        assert_eq!(code(&["--version"]), Some(0));
        assert_eq!(code(&["list", "--color", "never"]), None);
    }

    #[test]
    fn clap_styling_follows_the_color_flag() {
        let color = |argv: &[&str]| clap_color(&args(argv));
        assert_eq!(color(&["list"]), clap::ColorChoice::Auto);
        assert_eq!(
            color(&["--color", "never", "--help"]),
            clap::ColorChoice::Never
        );
        assert_eq!(
            color(&["list", "--bogus", "--color=never"]),
            clap::ColorChoice::Never
        );
        assert_eq!(
            color(&["--color", "always", "list"]),
            clap::ColorChoice::Always
        );
        assert_eq!(
            color(&["--color=never", "--color", "auto"]),
            clap::ColorChoice::Auto
        );
        assert_eq!(color(&["--color", "sometimes"]), clap::ColorChoice::Auto);
        assert_eq!(color(&["--color"]), clap::ColorChoice::Auto);
        assert_eq!(
            color(&["show", "--", "--color=never"]),
            clap::ColorChoice::Auto
        );

        let cli = parse_cli(args(&["--color=never", "list"])).unwrap();
        assert_eq!(cli.color, crate::cli::ColorChoice::Never);
    }

    #[test]
    fn report_prints_message_then_each_cause() {
        let error = anyhow::Error::from(io::Error::new(ErrorKind::NotFound, "no such file"))
            .context("reading /nope")
            .context("could not load sessions");
        assert_eq!(
            reported(&error),
            "error: could not load sessions\n  caused by: reading /nope\n  caused by: no such file\n"
        );
    }

    fn hint_lines(error: &Error) -> String {
        let hints = error.hints();
        assert!(!hints.is_empty());
        hints.iter().map(|hint| format!("{hint}\n")).collect()
    }

    #[test]
    fn report_prints_hints_after_causes() {
        let ambiguous = || Error::AmbiguousId {
            query: "f4".into(),
            matches: vec!["f4aa".into(), "f4bb".into()],
        };
        let error = anyhow::Error::from(ambiguous());
        let expected = format!("error: {error}\n{}", hint_lines(&ambiguous()));
        assert_eq!(reported(&error), expected);
        assert!(expected.contains("\n  f4aa\n  f4bb\nuse more characters of the ID\n"));

        let not_found = || Error::SessionNotFound {
            query: "abc".into(),
        };
        let error = Err::<(), _>(not_found())
            .context("could not show session")
            .unwrap_err();
        assert_eq!(
            reported(&error),
            format!(
                "error: could not show session\n  caused by: {}\n{}",
                not_found(),
                hint_lines(&not_found())
            )
        );
    }

    #[test]
    fn diagnostics_strip_escape_sequences_from_stored_text() {
        let error = anyhow::Error::from(Error::AmbiguousId {
            query: "cc".into(),
            matches: vec![
                "cc01  agent  Edge \u{1b}]0;pwned\u{7} title".into(),
                "cc02  agent  Second \u{1b}[41mEVIL\u{1b}[0m\u{7} title".into(),
            ],
        })
        .context("query \u{1b}[2Jcc");
        let mut buf = Vec::new();
        report(&error, &mut diagnostics(&mut buf));
        let text = String::from_utf8(buf).unwrap();
        assert!(!text.contains(['\u{1b}', '\u{7}']), "{text:?}");
        assert!(text.starts_with("error: query cc\n  caused by: "));
        assert!(text.contains("\n  cc01  agent  Edge  title\n"));
        assert!(text.contains("\n  cc02  agent  Second EVIL title\n"));
    }

    #[test]
    fn report_never_fails_on_a_broken_stderr() {
        struct Closed;
        impl Write for Closed {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(ErrorKind::BrokenPipe.into())
            }
            fn flush(&mut self) -> io::Result<()> {
                Err(ErrorKind::BrokenPipe.into())
            }
        }
        report(&anyhow::anyhow!("boom"), &mut Closed);
    }

    #[test]
    fn broken_pipe_is_found_anywhere_in_the_chain() {
        let pipe = || io::Error::from(ErrorKind::BrokenPipe);
        let cases = [
            anyhow::Error::from(pipe()),
            anyhow::Error::from(pipe()).context("writing output"),
            anyhow::Error::from(Error::Write(pipe())),
            anyhow::Error::from(Error::Json(serde_json::Error::io(pipe()))),
            anyhow::Error::from(serde_json::Error::io(pipe())).context("serializing"),
            anyhow::Error::from(Error::Io {
                path: PathBuf::from("out.md"),
                source: pipe(),
            }),
        ];
        for error in &cases {
            assert!(is_broken_pipe(error), "{error:#}");
        }

        let others = [
            anyhow::anyhow!("session not found: x"),
            anyhow::Error::from(io::Error::from(ErrorKind::PermissionDenied)),
            anyhow::Error::from(Error::Write(io::Error::from(ErrorKind::WriteZero))),
        ];
        for error in &others {
            assert!(!is_broken_pipe(error), "{error:#}");
        }
    }
}
