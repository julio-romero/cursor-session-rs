use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};
use cursor_session::export::Format;

#[derive(Parser)]
#[command(
    name = "cursor-session",
    version,
    about = "List, show, and export Cursor IDE and Agent CLI chat sessions"
)]
pub struct Cli {
    /// Path to ~/.cursor/chats, a session directory, store.db, or state.vscdb
    #[arg(long, global = true)]
    pub storage: Option<PathBuf>,

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
    /// List available sessions
    List,
    /// Show messages from a session
    Show {
        session_id: String,
        /// Maximum number of messages to print (from the end)
        #[arg(long, conflicts_with = "all")]
        limit: Option<usize>,
        /// Print the full transcript
        #[arg(long)]
        all: bool,
    },
    /// Export sessions to files
    Export {
        #[arg(long, value_enum, default_value_t = Format::Md)]
        format: Format,
        #[arg(long, default_value = "exports")]
        out: PathBuf,
        #[arg(long)]
        session_id: Option<String>,
        /// Filter by workspace path or MD5 hash
        #[arg(long)]
        workspace: Option<String>,
    },
    /// Check that session stores can be found
    Healthcheck,
}
