use std::ffi::OsString;
use std::path::PathBuf;

use clap::{Args, CommandFactory, Parser, Subcommand, ValueEnum, ValueHint};
use cursor_session::export::Format;
use cursor_session::model::Source;
use cursor_session::search::{self, DEFAULT_CONTEXT, MAX_CONTEXT};
use cursor_session::since::{Since, parse_since};
use cursor_session::view::Role;

const LONG_ABOUT: &str = "\
List, show, and export Cursor IDE and Agent CLI chat sessions.

`search` finds the sessions whose messages hold a set of words, and `handoff`
copies a session's transcript for another agent to continue it.

Reads two local stores and never writes to them:
  Agent CLI  ~/.cursor/chats plus transcripts in ~/.cursor/projects/*/agent-transcripts
  IDE        Cursor/User/globalStorage/state.vscdb in Cursor's config directory";

const EXAMPLES: &str = "\
Examples:
  cursor-session list
  cursor-session list --source agent --limit 10
  cursor-session show f4eea6d2
  cursor-session show f4eea6d2 --json | jq -r '.messages[].content'
  cursor-session search retry backoff --since 30d
  cursor-session handoff f4eea6d2
  cursor-session export --format md --session-id f4eea6d2";

const LIST_EXAMPLES: &str = "\
Examples:
  cursor-session list
  cursor-session list --source ide --limit 5
  cursor-session list --since 7d
  cursor-session list --json | jq -r '.[].id'";

const SHOW_EXAMPLES: &str = "\
Examples:
  cursor-session show f4eea6d2
  cursor-session show f4eea6d2 --all
  cursor-session show f4eea6d2 --json --limit 5
  cursor-session show f4eea6d2 --only user,assistant --short
  cursor-session show f4eea6d2 --only tool --limit 10";

const SEARCH_EXAMPLES: &str = "\
Examples:
  cursor-session search retry backoff
  cursor-session search \"connection pool\" timeout --since 30d
  cursor-session search migration --source ide -n 5 --context 120
  cursor-session search flaky ci --json | jq -r '.[].id'
  cursor-session search -- --force-with-lease";

const EXPORT_EXAMPLES: &str = "\
Examples:
  cursor-session export
  cursor-session export --format json --session-id f4eea6d2 --out sessions
  cursor-session export --workspace ~/src/billing-api --limit 5
  cursor-session export --since 2w --format json";

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
  1  Error: session not found or deleted while being read, no sessions match (search), unreadable storage, failed healthcheck
  2  Usage error: unknown command or flag, invalid value";

const SEARCH_EXIT_CODES: &str = "\
Exit codes:
  0  Success, also when output is cut short by a closed pipe (e.g. `| head`)
  1  Error: no sessions match, unreadable storage
  2  Usage error: unknown flag, invalid value, a query without terms or with more than 64";

const SEARCH_ABOUT: &str = "\
Find the sessions whose messages hold every word of a query.

Every term must appear somewhere in a session's messages, in any order and
any case (Unicode-aware). A word with spaces in it, as the shell passes
\"connection pool\", and a phrase in double quotes inside the query, as in
'\"connection pool\" timeout', are one term, whose words match with any
whitespace between them. Every term is matched as written, never as a
pattern. Titles are not searched. Put -- before a query that starts with -.

Sessions with every term in one message come first, then those with more
matching messages, then the most recently updated. Each result shows the
message with the most terms, cut around its first match, with the matches
highlighted when color is on.";

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

    /// Print the storage paths in use and the rows, lines and files that were skipped to stderr
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
    /// Find the sessions whose messages hold every word of a query
    #[command(
        long_about = SEARCH_ABOUT,
        after_help = SEARCH_EXAMPLES,
        after_long_help = format!("{SEARCH_EXAMPLES}\n\n{SEARCH_EXIT_CODES}")
    )]
    Search(SearchArgs),
    /// Export sessions to files
    ///
    /// Each session's messages are read only to write its file. An IDE chat
    /// that Cursor deletes after the sessions were listed is skipped with a
    /// warning, and the others are still written. With --session-id, such a
    /// chat is an error instead.
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
    /// on Windows through the system clipboard; on Linux through X11, where a
    /// copy of this program serves it in the background until something else
    /// is copied, for at most 12 hours. Without an X11 display (DISPLAY not
    /// set, as in a Wayland session without XWayland, where `--stdout |
    /// wl-copy` copies it), or when copying fails, it is printed to stdout
    /// instead, with a warning on stderr, and the command still succeeds. A
    /// session without user or assistant messages is not copied; a warning
    /// says so.
    ///
    /// On X11, handoff waits up to 5 seconds for that background copy to own
    /// the clipboard. Interrupted in that time, it prints nothing, but the
    /// background copy may still take the clipboard and serve the transcript.
    /// Over `ssh -X` it keeps the connection to the X server open until
    /// something else is copied, which can keep the session from closing:
    /// there, use --stdout, or copy something else before logging out.
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
    /// the binary knows about itself, or hold the clipboard, so `--storage`
    /// and a missing home do not concern them.
    pub fn reads_storage(&self) -> bool {
        !matches!(
            self,
            Self::Completions(_) | Self::Man(_) | Self::ServeClipboard
        )
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
    /// Keep only the sessions updated within this long: a number and s, m, h, d or w (30d, 12h)
    #[arg(
        long,
        value_name = "DURATION",
        value_parser = parse_since
    )]
    pub since: Option<Since>,
    /// Print a JSON array of session summaries
    #[arg(long)]
    pub json: bool,
    #[arg(from_global)]
    pub verbose: bool,
}

#[derive(Args)]
pub struct SearchArgs {
    /// Terms that must all appear in a session's messages; a quoted "phrase" is one term (put -- before a term that starts with -)
    #[arg(required = true, value_name = "QUERY")]
    pub query: Vec<String>,
    /// Only read this store; the other one is never opened
    #[arg(long, value_enum)]
    pub source: Option<Source>,
    /// Keep only the N best matching sessions
    #[arg(
        short = 'n',
        long,
        value_name = "N",
        value_parser = at_least_one,
        allow_negative_numbers = true
    )]
    pub limit: Option<usize>,
    /// Characters of the best message to show on each side of its first match, at most 1000
    #[arg(
        long,
        value_name = "CHARS",
        value_parser = context_chars,
        allow_negative_numbers = true,
        default_value_t = DEFAULT_CONTEXT
    )]
    pub context: usize,
    /// Search only the sessions updated within this long: a number and s, m, h, d or w (30d, 12h)
    #[arg(
        long,
        value_name = "DURATION",
        value_parser = parse_since
    )]
    pub since: Option<Since>,
    /// Print a JSON array of the matching sessions, best first
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
    /// Export only the sessions updated within this long: a number and s, m, h, d or w (30d, 12h)
    #[arg(
        long,
        value_name = "DURATION",
        value_parser = parse_since,
        conflicts_with = "session_id"
    )]
    pub since: Option<Since>,
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
    /// Start the transcript with TEXT instead of the default preamble (TEXT
    /// may start with "-"; one of handoff's options is text only attached, as
    /// in --preamble=--stdout)
    #[arg(
        long,
        value_name = "TEXT",
        value_parser = not_blank,
        allow_hyphen_values = true,
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

/// A whole number from 0 to [`MAX_CONTEXT`].
fn context_chars(value: &str) -> Result<usize, String> {
    let invalid = || format!("expected a whole number from 0 to {MAX_CONTEXT}");
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(invalid());
    }
    match value.parse::<usize>() {
        Ok(n) if n <= MAX_CONTEXT => Ok(n),
        _ => Err(invalid()),
    }
}

impl Cli {
    /// Checks what clap cannot check one value at a time: that the words of
    /// a search query, together, make a query (see
    /// [`search::parse_query`]). The error names the subcommand and gives a
    /// usage error's message.
    pub fn check(&self) -> Result<(), (&'static str, String)> {
        if let Commands::Search(args) = &self.command {
            let query = search::query_text(&args.query);
            if let Err(error) = search::parse_query(&query) {
                let message = format!("invalid value '{query}' for '<QUERY>...': {error}");
                return Err(("search", message));
            }
        }
        Ok(())
    }

    /// Checks what only the raw arguments `args` show: that `handoff
    /// --preamble` did not take one of handoff's own options for its text,
    /// as in `--preamble --stdout`. A preamble may start with `-`, so clap
    /// takes the next word whatever it is; given as `--preamble=--stdout`
    /// the option is meant as text and is kept. The error is as
    /// [`Cli::check`] gives one.
    pub fn check_args(&self, args: &[OsString]) -> Result<(), (&'static str, String)> {
        let Commands::Handoff(handoff) = &self.command else {
            return Ok(());
        };
        let Some(preamble) = &handoff.preamble else {
            return Ok(());
        };
        let separate = args
            .windows(2)
            .any(|pair| pair[0] == "--preamble" && pair[1] == preamble.as_str());
        if separate && is_handoff_option(preamble) {
            let message = format!(
                "invalid value '{preamble}' for '--preamble <TEXT>': it is an option of \
                 handoff, not text; give the text before it, or write \
                 --preamble='{preamble}' to start the transcript with it"
            );
            return Err(("handoff", message));
        }
        Ok(())
    }
}

/// The arguments `args` with a negative span given to `--since` attached
/// to it: `--since -1d` becomes `--since=-1d`, so that clap reports the span
/// as a `--since` value that is not valid rather than as an option it does
/// not know. Only a word of `-` and a digit is attached, so a flag that
/// follows a `--since` without a value is still read as a flag, and clap
/// says the value is missing. Nothing after `--` is changed, nor a
/// `--since` that is itself the text of `--preamble`.
pub fn attach_negative_since(args: impl IntoIterator<Item = OsString>) -> Vec<OsString> {
    let mut attached: Vec<OsString> = Vec::new();
    let mut after_dashes = false;
    for arg in args {
        let follows_since = !after_dashes
            && attached.last().is_some_and(|last| last == "--since")
            && attached
                .len()
                .checked_sub(2)
                .is_none_or(|before| attached[before] != "--preamble");
        if follows_since && is_negative_span(&arg) {
            if let Some(since) = attached.last_mut() {
                since.push("=");
                since.push(&arg);
            }
            continue;
        }
        after_dashes |= arg == "--";
        attached.push(arg);
    }
    attached
}

/// Whether `word` reads as a negative number: `-` and then a digit.
fn is_negative_span(word: &OsString) -> bool {
    let bytes = word.as_encoded_bytes();
    bytes.len() > 1 && bytes[0] == b'-' && bytes[1].is_ascii_digit()
}

/// Whether `word` is one of the options `handoff` takes, its own or a
/// global one, alone or with an `=value`.
fn is_handoff_option(word: &str) -> bool {
    const SHORT: [&str; 2] = ["-v", "-h"];
    const LONG: [&str; 9] = [
        "--stdout",
        "--no-preamble",
        "--preamble",
        "--limit",
        "--source",
        "--storage",
        "--color",
        "--verbose",
        "--help",
    ];
    let name = word.split_once('=').map_or(word, |(name, _)| name);
    SHORT.contains(&word) || LONG.contains(&name)
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
    fn search_limit_must_be_positive_and_context_a_whole_number() {
        let parse = |argv: &[&str]| {
            Cli::try_parse_from(["cursor-session", "search", "x"].iter().chain(argv))
        };
        for argv in [&["-n", "0"][..], &["--limit", "-1"], &["--limit", "x"]] {
            let err = parse(argv).err().unwrap();
            assert_eq!(
                err.kind(),
                clap::error::ErrorKind::ValueValidation,
                "{argv:?}"
            );
        }
        for argv in [
            &["--context", "-1"][..],
            &["--context", ""],
            &["--context", "1.5"],
            &["--context", "1001"],
            &["--context", "99999999999999999999999"],
        ] {
            let err = parse(argv).err().unwrap();
            assert_eq!(
                err.kind(),
                clap::error::ErrorKind::ValueValidation,
                "{argv:?}"
            );
            assert!(
                err.to_string()
                    .contains(": expected a whole number from 0 to 1000"),
                "{err}"
            );
        }
        let context = |argv: &[&str]| match parse(argv).unwrap().command {
            Commands::Search(args) => (args.context, args.limit),
            _ => panic!("expected search"),
        };
        assert_eq!(context(&[]), (60, None));
        assert_eq!(context(&["--context", "0", "-n", "3"]), (0, Some(3)));
        assert_eq!(context(&["--context", "1000"]), (1000, None));
    }

    #[test]
    fn search_query_words_join_into_one_query() {
        let cli =
            Cli::try_parse_from(["cursor-session", "search", "\"two", "words\"", "more"]).unwrap();
        assert!(cli.check().is_ok());
        let Commands::Search(args) = cli.command else {
            panic!("expected search");
        };
        let query = search::parse_query(&search::query_text(&args.query)).unwrap();
        assert_eq!(query.terms(), ["two words", "more"]);

        // A word with spaces in it, as the shell passes a quoted phrase, is
        // one; one with quote marks is parsed as written.
        let terms = |words: &[&str]| {
            let words: Vec<String> = words.iter().map(|word| word.to_string()).collect();
            search::parse_query(&search::query_text(&words))
                .unwrap()
                .terms()
                .to_vec()
        };
        assert_eq!(terms(&["connection pool", "x"]), ["connection pool", "x"]);
        assert_eq!(terms(&["a\"b c\" d"]), ["a", "b c", "d"]);
        assert_eq!(terms(&["-x"]), ["-x"]);
        let dashed = Cli::try_parse_from(["cursor-session", "search", "--", "-x"]).unwrap();
        let Commands::Search(args) = dashed.command else {
            panic!("expected search");
        };
        assert_eq!(args.query, ["-x"]);

        // Clap requires a word; the check, that the words make a term.
        assert!(Cli::try_parse_from(["cursor-session", "search"]).is_err());
        for empty in [&[""][..], &["  "], &["\"\""], &["\"", "\""]] {
            let argv = ["cursor-session", "search"].iter().chain(empty);
            let (name, message) = Cli::try_parse_from(argv).unwrap().check().unwrap_err();
            assert_eq!(name, "search");
            assert!(message.ends_with("': the query has no terms"), "{message}");
        }
        let many: Vec<String> = (0..65).map(|n| format!("t{n}")).collect();
        let argv = ["cursor-session", "search"]
            .into_iter()
            .chain(many.iter().map(String::as_str));
        let (_, message) = Cli::try_parse_from(argv).unwrap().check().unwrap_err();
        assert!(
            message.ends_with("the query has 65 terms; at most 64 are allowed"),
            "{message}"
        );
    }

    #[test]
    fn a_preamble_is_not_one_of_handoffs_options() {
        let check = |argv: &[&str]| {
            let args: Vec<OsString> = argv.iter().map(OsString::from).collect();
            let cli = Cli::try_parse_from(&args).unwrap();
            cli.check_args(&args)
        };
        for option in [
            "--stdout",
            "--no-preamble",
            "--preamble",
            "--limit",
            "--limit=5",
            "--source=ide",
            "--storage=/tmp",
            "--color",
            "--color=never",
            "-v",
            "--verbose",
            "-h",
            "--help",
        ] {
            for argv in [
                &["cursor-session", "handoff", "abc", "--preamble", option][..],
                &["cursor-session", "handoff", "--preamble", option, "abc"],
            ] {
                let (name, message) = check(argv).unwrap_err();
                assert_eq!(name, "handoff");
                assert!(
                    message.starts_with(&format!(
                        "invalid value '{option}' for '--preamble <TEXT>': it is an option"
                    )),
                    "{message}"
                );
                assert!(
                    message.ends_with(&format!(
                        "write --preamble='{option}' to start the transcript with it"
                    )),
                    "{message}"
                );
            }
            // Attached, it is text.
            let attached = format!("--preamble={option}");
            assert!(check(&["cursor-session", "handoff", "abc", &attached]).is_ok());
        }
        // Text that only starts with a hyphen, and words that are no option
        // of handoff's.
        for text in [
            "- Continue the refactor",
            "--- context ---",
            "--stdout please",
            "--json",
            "--all",
            "-x",
            "-vv",
        ] {
            assert!(
                check(&["cursor-session", "handoff", "abc", "--preamble", text]).is_ok(),
                "{text}"
            );
        }
        // Other commands and handoff without --preamble pass.
        assert!(check(&["cursor-session", "handoff", "abc", "--stdout"]).is_ok());
        assert!(check(&["cursor-session", "show", "abc"]).is_ok());
    }

    #[test]
    fn attach_negative_since_attaches_only_a_negative_span() {
        let attach = |argv: &[&str]| -> Vec<String> {
            attach_negative_since(argv.iter().map(OsString::from))
                .into_iter()
                .map(|arg| arg.into_string().unwrap())
                .collect()
        };
        assert_eq!(
            attach(&["cs", "list", "--since", "-1d", "--json"]),
            ["cs", "list", "--since=-1d", "--json"]
        );
        for argv in [
            &["cs", "list", "--since", "1d"][..],
            &["cs", "list", "--since", "--json"],
            &["cs", "list", "--since", "-v"],
            &["cs", "list", "--since", "-"],
            &["cs", "list", "--since=-1d"],
            &["cs", "list", "--since"],
            &["cs", "search", "--", "--since", "-1d"],
            &["cs", "handoff", "abc", "--preamble", "--since", "-1d"],
        ] {
            assert_eq!(attach(argv), argv, "{argv:?}");
        }
    }

    #[test]
    fn since_takes_a_duration_and_not_with_one_session() {
        for command in [&["list"][..], &["export"], &["search", "x"]] {
            let parse = |since: &str| {
                let argv: Vec<&str> = ["cursor-session"]
                    .into_iter()
                    .chain(command.iter().copied())
                    .chain(["--since", since])
                    .collect();
                Cli::try_parse_from(attach_negative_since(argv.iter().map(OsString::from)))
            };
            for since in ["30d", "12h", "90m", "45s", "2w"] {
                assert!(parse(since).is_ok(), "{command:?} {since}");
            }
            for since in ["", "0d", "30", "d", "1.5h", "30y", "30 d", "30D"] {
                let err = parse(since).err().unwrap();
                assert_eq!(
                    err.kind(),
                    clap::error::ErrorKind::ValueValidation,
                    "{since:?}"
                );
                assert_eq!(err.exit_code(), 2);
            }
            // A negative span is a span that is not valid, not an unknown option.
            for since in ["-1d", "-30", "-0d"] {
                let err = parse(since).err().unwrap();
                assert_eq!(err.kind(), clap::error::ErrorKind::ValueValidation);
                assert_eq!(err.exit_code(), 2);
                assert!(err.to_string().contains("'--since <DURATION>'"), "{err}");
            }
            // A flag after a --since without a value stays a flag: the value
            // is missing, whether the flag takes a value or not.
            let own: &[&str] = if command[0] == "export" {
                &["--out", "x"]
            } else {
                &["--json"]
            };
            for next in [
                own,
                &["--limit", "3"],
                &["--source", "agent"],
                &["-v"],
                &["-h"],
                &["--help"],
            ] {
                let argv: Vec<&str> = ["cursor-session"]
                    .into_iter()
                    .chain(command.iter().copied())
                    .chain(["--since"])
                    .chain(next.iter().copied())
                    .collect();
                let err =
                    Cli::try_parse_from(attach_negative_since(argv.iter().map(OsString::from)))
                        .err()
                        .unwrap();
                assert_eq!(err.kind(), clap::error::ErrorKind::InvalidValue, "{argv:?}");
                let message = err.to_string();
                assert!(
                    message.contains("a value is required for '--since <DURATION>'"),
                    "{argv:?}: {message}"
                );
            }
        }
        let err = Cli::try_parse_from([
            "cursor-session",
            "export",
            "--since",
            "1d",
            "--session-id",
            "abc",
        ])
        .err()
        .unwrap();
        assert_eq!(err.kind(), clap::error::ErrorKind::ArgumentConflict);
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
