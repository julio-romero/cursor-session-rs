use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};
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
    /// Read only this location: a home, .cursor, chats or session directory, store.db, or
    /// state.vscdb
    #[arg(long, global = true, value_name = "PATH")]
    pub storage: Option<PathBuf>,

    /// Print the storage paths in use and loader warnings to stderr
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
    List(ListArgs),
    /// Show messages from a session
    Show(ShowArgs),
    /// Export sessions to files
    Export(ExportArgs),
    /// Check that session stores can be found and loaded
    Healthcheck(HealthcheckArgs),
}

#[derive(Args)]
pub struct ListArgs {
    /// Only read this store; the other one is never opened
    #[arg(long, value_enum)]
    pub source: Option<Source>,
    /// Keep only the N most recently updated sessions
    #[arg(long, value_name = "N", value_parser = at_least_one)]
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
    pub session_id: String,
    /// Only read this store; the other one is never opened
    #[arg(long, value_enum)]
    pub source: Option<Source>,
    /// Maximum number of messages to print (from the end)
    #[arg(long, value_name = "N", conflicts_with = "all")]
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
    #[arg(long, default_value = "exports")]
    pub out: PathBuf,
    /// Export only this session (ID or unique prefix)
    #[arg(long)]
    pub session_id: Option<String>,
    /// Filter by workspace path or MD5 hash
    #[arg(long)]
    pub workspace: Option<String>,
    /// Only read this store; the other one is never opened
    #[arg(long, value_enum)]
    pub source: Option<Source>,
    #[arg(from_global)]
    pub verbose: bool,
}

#[derive(Args)]
pub struct HealthcheckArgs {
    #[arg(from_global)]
    pub verbose: bool,
}

fn at_least_one(value: &str) -> Result<usize, String> {
    match value.parse::<usize>() {
        Ok(0) => Err("must be at least 1".to_string()),
        Ok(n) => Ok(n),
        Err(err) => Err(err.to_string()),
    }
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
    fn list_limit_must_be_positive() {
        let err = Cli::try_parse_from(["cursor-session", "list", "--limit", "0"])
            .err()
            .unwrap();
        assert_eq!(err.kind(), clap::error::ErrorKind::ValueValidation);
        assert!(Cli::try_parse_from(["cursor-session", "list", "--limit", "3"]).is_ok());
    }
}
