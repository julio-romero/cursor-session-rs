use std::path::PathBuf;

use clap::{Args, CommandFactory, Parser, Subcommand, ValueEnum, ValueHint};
use cursor_session::export::Format;
use cursor_session::model::Source;

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
  cursor-session show f4eea6d2 --json --limit 5";

const EXPORT_EXAMPLES: &str = "\
Examples:
  cursor-session export
  cursor-session export --format json --session-id f4eea6d2 --out sessions
  cursor-session export --workspace ~/src/billing-api --limit 5";

const HEALTHCHECK_EXAMPLES: &str = "\
Examples:
  cursor-session healthcheck
  cursor-session -v healthcheck";

const COMPLETIONS_EXAMPLES: &str = "\
Examples:
  cursor-session completions bash > ~/.local/share/bash-completion/completions/cursor-session
  cursor-session completions zsh > ~/.zfunc/_cursor-session
  cursor-session completions fish > ~/.config/fish/completions/cursor-session.fish";

const MAN_EXAMPLES: &str = "\
Examples:
  cursor-session man > cursor-session.1 && man ./cursor-session.1
  cursor-session man list > cursor-session-list.1";

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
    #[command(
        after_help = LIST_EXAMPLES,
        after_long_help = format!("{LIST_EXAMPLES}\n\n{EXIT_CODES}")
    )]
    List(ListArgs),
    /// Show messages from a session
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
    /// Check that session stores can be found and loaded
    #[command(
        after_help = HEALTHCHECK_EXAMPLES,
        after_long_help = format!("{HEALTHCHECK_EXAMPLES}\n\n{EXIT_CODES}")
    )]
    Healthcheck(HealthcheckArgs),
    /// Print a shell completion script
    #[command(
        after_help = COMPLETIONS_EXAMPLES,
        after_long_help = format!("{COMPLETIONS_EXAMPLES}\n\n{EXIT_CODES}")
    )]
    Completions(CompletionsArgs),
    /// Print a man page in roff format
    #[command(
        hide = true,
        after_help = MAN_EXAMPLES,
        after_long_help = format!("{MAN_EXAMPLES}\n\n{EXIT_CODES}")
    )]
    Man(ManArgs),
}

impl Commands {
    /// Whether the command reads session stores. The others only print what
    /// the binary knows about itself, so `--storage` and a missing home do
    /// not concern them.
    pub fn reads_storage(&self) -> bool {
        !matches!(self, Self::Completions(_) | Self::Man(_))
    }
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
    #[arg(
        long,
        value_name = "DIR",
        default_value = "exports",
        value_hint = ValueHint::DirPath
    )]
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
pub struct HealthcheckArgs {
    #[arg(from_global)]
    pub verbose: bool,
    #[arg(from_global)]
    pub storage: Option<PathBuf>,
}

#[derive(Args)]
pub struct CompletionsArgs {
    /// The shell to complete in
    #[arg(value_enum)]
    pub shell: Shell,
}

/// The shells `completions` writes scripts for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Shell {
    Bash,
    Zsh,
    Fish,
}

#[derive(Args)]
pub struct ManArgs {
    /// Print the page of this command instead of the overview
    #[arg(value_name = "COMMAND", value_parser = documented_command)]
    pub command: Option<String>,
}

/// The visible subcommands, the ones with a man page of their own.
pub fn documented_commands() -> Vec<String> {
    Cli::command()
        .get_subcommands()
        .filter(|sub| !sub.is_hide_set())
        .map(|sub| sub.get_name().to_string())
        .collect()
}

fn documented_command(value: &str) -> Result<String, String> {
    let names = documented_commands();
    if names.iter().any(|name| name == value) {
        return Ok(value.to_string());
    }
    Err(format!("expected one of: {}", names.join(", ")))
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
        for command in ["list", "show", "export"] {
            let args = |limit| {
                let id = (command == "show").then_some("abc");
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
