mod cli;
mod commands;
mod output;

use std::error::Error as StdError;
use std::io::{self, ErrorKind, Write};
use std::path::Path;
use std::process::ExitCode;

use anyhow::Result;
use clap::Parser;
use cursor_session::detect::StoragePaths;

use crate::cli::Cli;
use crate::output::{IgnoreErrors, OutputOpts, PipeWriter};

fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => {
            // Help and version go to stdout and exit 0; usage errors exit 2.
            let _ = error.print();
            return ExitCode::from(u8::try_from(error.exit_code()).unwrap_or(2));
        }
    };

    let mut out = PipeWriter::new(io::stdout().lock());
    let mut err = IgnoreErrors(io::stderr().lock());
    let result = run(cli, &mut out, &mut err).and_then(|()| Ok(out.flush()?));
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

fn run(cli: Cli, out: &mut dyn Write, err: &mut dyn Write) -> Result<()> {
    let paths = resolve_paths(cli.storage.as_deref())?;
    if cli.verbose {
        writeln!(err, "chats: {:?}", paths.chats_dir)?;
        writeln!(err, "projects: {:?}", paths.projects_dir)?;
        writeln!(err, "ide db: {:?}", paths.global_storage_db)?;
    }

    let opts = OutputOpts::detect(cli.color);
    commands::run(cli.command, &paths, &opts, out, err)
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

    #[test]
    fn usage_errors_exit_2_and_help_exits_0() {
        let code = |args: &[&str]| Cli::try_parse_from(args).err().map(|e| e.exit_code());
        assert_eq!(code(&["cursor-session", "list", "--bogus"]), Some(2));
        assert_eq!(
            code(&["cursor-session", "--color", "sometimes", "list"]),
            Some(2)
        );
        assert_eq!(code(&["cursor-session"]), Some(2));
        assert_eq!(code(&["cursor-session", "--help"]), Some(0));
        assert_eq!(code(&["cursor-session", "--version"]), Some(0));
        assert_eq!(code(&["cursor-session", "list", "--color", "never"]), None);
    }

    #[test]
    fn report_prints_message_causes_and_hints() {
        let error = anyhow::Error::from(Error::StorageNotFound {
            path: PathBuf::from("/nope"),
            source: io::Error::new(ErrorKind::NotFound, "no such file"),
        });
        assert_eq!(
            reported(&error),
            "error: storage path does not exist: /nope\n  caused by: no such file\n"
        );

        let error = anyhow::Error::from(Error::AmbiguousId {
            query: "f4".into(),
            matches: vec!["f4aa".into(), "f4bb".into()],
        });
        assert_eq!(
            reported(&error),
            "error: session id `f4` is ambiguous (2 matches)\nf4aa\nf4bb\nUse a longer prefix or the full ID.\n"
        );
    }

    #[test]
    fn report_finds_hints_under_context() {
        let error = Err::<(), _>(Error::SessionNotFound {
            query: "abc".into(),
        })
        .context("could not show session")
        .unwrap_err();
        assert_eq!(
            reported(&error),
            "error: could not show session\n  caused by: session not found: abc\nUse `cursor-session list` to see IDs.\n"
        );
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
