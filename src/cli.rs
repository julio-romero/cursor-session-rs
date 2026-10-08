use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};
use cursor_session::export::Format;
use cursor_session::model::Source;
use cursor_session::view::Role;

const LONG_ABOUT: &str = "\
List, show, and export Cursor IDE and Agent CLI chat sessions.

Reads two local stores and never writes to them:
  Agent CLI  ~/.cursor/chats plus transcripts in ~/.cursor/projects/*/agent-transcripts
  IDE        Cursor/User/globalStorage/state.vscdb in Cursor's config directory";

const EXAMPLES: &str = "\
Examples:
  cursor-session list
  cursor-session list --source agent --limit 10
  cursor-session show f4eea6d2
  cursor-session show f4eea6d2 --json | jq -r '.messages[].content'
  cursor-session handoff f4eea6d2
  cursor-session export --format md --session-id f4eea6d2";

const LIST_EXAMPLES: &str = "\
Examples:
  cursor-session list
  cursor-session list --source ide --limit 5
  cursor-session list --json | jq -r '.[].id'";

const SHOW_EXAMPLES: &str = "\
Examples:
  cursor-session show f4eea6d2
  cursor-session show f4eea6d2 --all
  cursor-session show f4eea6d2 --json --limit 5
  cursor-session show f4eea6d2 --only user,assistant --short
  cursor-session show f4eea6d2 --only tool --limit 10";

const EXPORT_EXAMPLES: &str = "\
Examples:
  cursor-session export
  cursor-session export --format json --session-id f4eea6d2 --out sessions
  cursor-session export --workspace ~/src/billing-api --limit 5";

const HANDOFF_EXAMPLES: &str = "\
Examples:
  cursor-session handoff f4eea6d2
  cursor-session handoff f4eea6d2 --limit 30
  cursor-session handoff f4eea6d2 --stdout > handoff.txt
  cursor-session handoff f4eea6d2 --preamble 'Continue this refactor in the same repository.'
  cursor-session handoff f4eea6d2 --no-preamble --stdout
  cursor-session handoff f4eea6d2 --stdout | wl-copy";

const SERVE_CLIPBOARD_EXAMPLES: &str = "\
Examples:
  cursor-session serve-clipboard < transcript.txt";

const HEALTHCHECK_EXAMPLES: &str = "\
Examples:
  cursor-session healthcheck
  cursor-session -v healthcheck";

const EXIT_CODES: &str = "\
Exit codes:
  0  Success, also when output is cut short by a closed pipe (e.g. `| head`)
  1  Error: session not found, unreadable storage, failed healthcheck
  2  Usage error: unknown command or flag, invalid value";

#[derive(Parser)]
#[command(
    name = "cursor-session",
    version,
    about = "List, show, and export Cursor IDE and Agent CLI chat sessions",
    long_about = LONG_ABOUT,
    after_help = EXAMPLES,
    after_long_help = format!("{EXAMPLES}\n\n{EXIT_CODES}"),
    next_help_heading = "Global Options"
)]
pub struct Cli {
    /// Read only this location: a home, .cursor, chats, workspace, session or projects
    /// directory, a store.db or state.vscdb file, or the directory that holds state.vscdb
    #[arg(long, global = true, value_name = "PATH")]
    pub storage: Option<PathBuf>,

    /// Print the storage paths in use and the rows and files that were skipped to stderr
    #[arg(short, long, global = true)]
    pub verbose: bool,

    /// When to use color
    #[arg(
        long,
        global = true,
        value_name = "WHEN",
        value_enum,
        default_value_t = ColorChoice::Auto
    )]
    pub color: ColorChoice,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ColorChoice {
    Auto,
    Always,
    Never,
}

#[derive(Subcommand)]
pub enum Commands {
    /// List sessions, most recently updated first
    ///
    /// TOKENS (`token_estimate` in --json) estimates how many tokens the
    /// messages `show` prints by default take: ceil(characters / 4). It is an
    /// estimate, not any model's tokenizer count. A terminal's table has the
    /// column from 79 columns wide; piped output and --json always have it.
    #[command(
        after_help = LIST_EXAMPLES,
        after_long_help = format!("{LIST_EXAMPLES}\n\n{EXIT_CODES}")
    )]
    List(ListArgs),
    /// Show messages from a session
    ///
    /// The header's `tokens:` line (`token_estimate` in --json) estimates how
    /// many tokens the messages `show` prints by default take: ceil(characters
    /// / 4). Tool calls and results are not counted. It is an estimate, not
    /// any model's tokenizer count.
    ///
    /// The header's `messages:` and `tokens:` lines (`message_count` and
    /// `token_estimate` in --json) are always those of the whole session,
    /// whatever --only, --short or --limit print.
    #[command(
        after_help = SHOW_EXAMPLES,
        after_long_help = format!("{SHOW_EXAMPLES}\n\n{EXIT_CODES}")
    )]
    Show(ShowArgs),
    /// Export sessions to files
    #[command(
        after_help = EXPORT_EXAMPLES,
        after_long_help = format!("{EXPORT_EXAMPLES}\n\n{EXIT_CODES}")
    )]
    Export(ExportArgs),
    /// Copy a session's transcript, to continue it with another agent
    ///
    /// The transcript is a preamble, then the session's user and assistant
    /// messages as `show --short --only user,assistant` prints them, each
    /// under a `[user]` or `[assistant]` line, then a trailer with the number
    /// of messages and the transcript's token estimate: ceil(characters / 4),
    /// an estimate, not any model's tokenizer count. Escape sequences in
    /// stored text are removed. The default preamble reads: "The following is
    /// a transcript from a Cursor session that ran out of credits. Continue
    /// from where it ended; do not summarize it back."
    ///
    /// It is copied to the system clipboard: on macOS with /usr/bin/pbcopy;
    /// on Linux through X11, where a copy of this program serves it in the
    /// background until something else is copied, for at most 12 hours.
    /// Without an X11 display (DISPLAY not set, as in a Wayland session
    /// without XWayland, where `--stdout | wl-copy` copies it), or when
    /// copying fails, it is printed to stdout instead, with a warning on
    /// stderr, and the command still succeeds. A session without user or
    /// assistant messages is not copied; a warning says so.
    #[command(
        after_help = HANDOFF_EXAMPLES,
        after_long_help = format!("{HANDOFF_EXAMPLES}\n\n{EXIT_CODES}")
    )]
    Handoff(HandoffArgs),
    /// Keep the text on stdin on the clipboard until something else is copied
    ///
    /// `handoff` runs this in the background on X11, where the program that
    /// copied text must keep serving it. It prints `ok` once the text is on
    /// the clipboard, or `error: ...`.
    #[command(
        hide = true,
        after_help = SERVE_CLIPBOARD_EXAMPLES,
        after_long_help = format!("{SERVE_CLIPBOARD_EXAMPLES}\n\n{EXIT_CODES}")
    )]
    ServeClipboard,
    /// Check that session stores can be found and loaded
    #[command(
        after_help = HEALTHCHECK_EXAMPLES,
        after_long_help = format!("{HEALTHCHECK_EXAMPLES}\n\n{EXIT_CODES}")
    )]
    Healthcheck(HealthcheckArgs),
}

#[derive(Args)]
pub struct ListArgs {
    /// Only read this store; the other one is never opened
    #[arg(long, value_enum)]
    pub source: Option<Source>,
    /// Keep only the N most recently updated sessions
    #[arg(
        long,
        value_name = "N",
        value_parser = at_least_one,
        allow_negative_numbers = true
    )]
    pub limit: Option<usize>,
    /// Print a JSON array of session summaries
    #[arg(long)]
    pub json: bool,
    #[arg(from_global)]
    pub verbose: bool,
}

#[derive(Args)]
pub struct ShowArgs {
    /// Session ID, or a unique prefix of one (case-insensitive)
    #[arg(value_parser = not_blank)]
    pub session_id: String,
    /// Only read this store; the other one is never opened
    #[arg(long, value_enum)]
    pub source: Option<Source>,
    /// Print only the last N messages [default: 20 in a terminal, all when piped or with --json]
    #[arg(
        long,
        value_name = "N",
        value_parser = at_least_one,
        allow_negative_numbers = true,
        conflicts_with = "all"
    )]
    pub limit: Option<usize>,
    /// Print the full transcript
    #[arg(long)]
    pub all: bool,
    /// Print only messages of these roles, comma-separated; `tool` adds the
    /// tool calls and results, which are left out otherwise, a call that
    /// failed or was stopped marked `(error)` or `(cancelled)`. --limit counts
    /// only the messages printed
    #[arg(long, value_name = "ROLES", value_enum, value_delimiter = ',')]
    pub only: Vec<Role>,
    /// Cut each message to its first 300 characters, and tool calls and
    /// results to a one-line preview
    #[arg(long)]
    pub short: bool,
    /// Print the session and its messages as JSON
    #[arg(long)]
    pub json: bool,
    #[arg(from_global)]
    pub verbose: bool,
}

#[derive(Args)]
pub struct ExportArgs {
    /// Output file format
    #[arg(long, value_enum, default_value_t = Format::Md)]
    pub format: Format,
    /// Directory to write into (created if missing)
    #[arg(long, value_name = "DIR", default_value = "exports")]
    pub out: PathBuf,
    /// Export only this session (ID or unique prefix)
    #[arg(long, conflicts_with = "workspace", value_parser = not_blank)]
    pub session_id: Option<String>,
    /// Export the Agent CLI sessions of a workspace: its path or a directory
    /// above it, directory names in its path, or the MD5 hash of its path
    #[arg(long, value_parser = not_blank)]
    pub workspace: Option<String>,
    /// Only read this store; the other one is never opened
    #[arg(long, value_enum)]
    pub source: Option<Source>,
    /// Export only the N most recently updated of the selected sessions
    #[arg(
        long,
        value_name = "N",
        value_parser = at_least_one,
        allow_negative_numbers = true,
        conflicts_with = "session_id"
    )]
    pub limit: Option<usize>,
    #[arg(from_global)]
    pub verbose: bool,
}

#[derive(Args)]
pub struct HandoffArgs {
    /// Session ID, or a unique prefix of one (case-insensitive)
    #[arg(value_parser = not_blank)]
    pub session_id: String,
    /// Only read this store; the other one is never opened
    #[arg(long, value_enum)]
    pub source: Option<Source>,
    /// Keep only the last N messages [default: all]
    #[arg(
        long,
        value_name = "N",
        value_parser = at_least_one,
        allow_negative_numbers = true
    )]
    pub limit: Option<usize>,
    /// Print the transcript instead of copying it
    #[arg(long)]
    pub stdout: bool,
    /// Start the transcript with TEXT instead of the default preamble
    #[arg(
        long,
        value_name = "TEXT",
        value_parser = not_blank,
        conflicts_with = "no_preamble"
    )]
    pub preamble: Option<String>,
    /// Start the transcript with the first message
    #[arg(long)]
    pub no_preamble: bool,
    #[arg(from_global)]
    pub verbose: bool,
}

#[derive(Args)]
pub struct HealthcheckArgs {
    #[arg(from_global)]
    pub verbose: bool,
    #[arg(from_global)]
    pub storage: Option<PathBuf>,
}

/// A whole number of at least 1. One too large to store asks for everything.
fn at_least_one(value: &str) -> Result<usize, String> {
    let invalid = || "expected a whole number of at least 1".to_string();
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(invalid());
    }
    match value.parse::<usize>() {
        Ok(0) => Err(invalid()),
        Ok(n) => Ok(n),
        Err(_) => Ok(usize::MAX),
    }
}

fn not_blank(value: &str) -> Result<String, String> {
    if value.trim().is_empty() {
        return Err("must not be empty".to_string());
    }
    Ok(value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn verbose_reaches_subcommands_from_either_position() {
        for argv in [
            ["cursor-session", "-v", "healthcheck"],
            ["cursor-session", "healthcheck", "-v"],
        ] {
            let cli = Cli::try_parse_from(argv).unwrap();
            assert!(cli.verbose);
            let Commands::Healthcheck(args) = cli.command else {
                panic!("expected healthcheck");
            };
            assert!(args.verbose);
        }
    }

    #[test]
    fn limit_must_be_positive() {
        for command in ["list", "show", "export", "handoff"] {
            let args = |limit| {
                let id = ["show", "handoff"].contains(&command).then_some("abc");
                ["cursor-session", command]
                    .into_iter()
                    .chain(id)
                    .chain(["--limit", limit])
                    .collect::<Vec<_>>()
            };
            for limit in ["0", "-1", "", " 3", "+3", "abc", "1.5", "000"] {
                let err = Cli::try_parse_from(args(limit)).err().unwrap();
                assert_eq!(
                    err.kind(),
                    clap::error::ErrorKind::ValueValidation,
                    "{command} {limit:?}"
                );
                assert!(
                    err.to_string()
                        .contains(": expected a whole number of at least 1"),
                    "{err}"
                );
            }
            assert!(Cli::try_parse_from(args("3")).is_ok());
        }
        assert_eq!(at_least_one("007"), Ok(7));
        assert_eq!(at_least_one("18446744073709551616"), Ok(usize::MAX));
    }

    #[test]
    fn blank_ids_and_workspaces_are_usage_errors() {
        for argv in [
            &["cursor-session", "show", ""][..],
            &["cursor-session", "show", " \t"],
            &["cursor-session", "export", "--session-id", ""],
            &["cursor-session", "export", "--workspace", " "],
            &["cursor-session", "handoff", ""],
            &["cursor-session", "handoff", "abc", "--preamble", " "],
        ] {
            let err = Cli::try_parse_from(argv).err().unwrap();
            assert_eq!(
                err.kind(),
                clap::error::ErrorKind::ValueValidation,
                "{argv:?}"
            );
            assert_eq!(err.exit_code(), 2);
        }
    }

    #[test]
    fn every_subcommand_has_examples_and_exit_codes() {
        let mut command = Cli::command();
        for sub in command.get_subcommands_mut() {
            let name = sub.get_name().to_string();
            let short = sub.render_help().to_string();
            let long = sub.render_long_help().to_string();
            assert!(
                short.contains(&format!("Examples:\n  cursor-session {name}")),
                "{short}"
            );
            assert!(!short.contains("Exit codes:"), "{name}");
            assert!(long.contains("Exit codes:\n  0  Success"), "{long}");
        }
    }
}
