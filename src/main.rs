use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use cursor_session::detect::StoragePaths;
use cursor_session::export::{self, Format};
use cursor_session::model::Session;
use cursor_session::ui::{self, stdout_is_tty, terminal_width};
use cursor_session::{filter_workspace, find_session, load_sessions};

#[derive(Parser)]
#[command(
    name = "cursor-session",
    version,
    about = "List, show, and export Cursor IDE and Agent CLI chat sessions"
)]
struct Cli {
    /// Path to ~/.cursor/chats, a session directory, store.db, or state.vscdb
    #[arg(long, global = true)]
    storage: Option<PathBuf>,

    #[arg(short, long, global = true)]
    verbose: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
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

fn main() -> Result<()> {
    let cli = Cli::parse();
    let paths = resolve_paths(cli.storage.as_deref())?;
    if cli.verbose {
        eprintln!("chats: {:?}", paths.chats_dir);
        eprintln!("projects: {:?}", paths.projects_dir);
        eprintln!("ide db: {:?}", paths.global_storage_db);
    }

    match cli.command {
        Commands::List => cmd_list(&paths),
        Commands::Show {
            session_id,
            limit,
            all,
        } => cmd_show(&paths, &session_id, limit, all),
        Commands::Export {
            format,
            out,
            session_id,
            workspace,
        } => cmd_export(
            &paths,
            format,
            &out,
            session_id.as_deref(),
            workspace.as_deref(),
        ),
        Commands::Healthcheck => cmd_healthcheck(&paths),
    }
}

fn resolve_paths(storage: Option<&std::path::Path>) -> Result<StoragePaths> {
    match storage {
        Some(path) => StoragePaths::from_custom(path, None),
        None => StoragePaths::detect(),
    }
}

fn cmd_list(paths: &StoragePaths) -> Result<()> {
    let sessions = load_sessions(paths)?;
    print!(
        "{}",
        ui::render_list(&sessions, stdout_is_tty(), terminal_width())
    );
    Ok(())
}

fn cmd_show(paths: &StoragePaths, session_id: &str, limit: Option<usize>, all: bool) -> Result<()> {
    let sessions = load_sessions(paths)?;
    let Some(session) = find_session(&sessions, session_id) else {
        bail!("session not found: {session_id}\nUse `cursor-session list` to see IDs.");
    };
    let tty = stdout_is_tty();
    let (messages, hidden) = ui::select_messages(&session.messages, tty, limit, all);
    print!("{}", ui::render_show(session, messages, hidden, tty));
    Ok(())
}

fn cmd_export(
    paths: &StoragePaths,
    format: Format,
    out: &std::path::Path,
    session_id: Option<&str>,
    workspace: Option<&str>,
) -> Result<()> {
    let sessions = load_sessions(paths)?;
    let selected: Vec<&Session> = if let Some(id) = session_id {
        let session =
            find_session(&sessions, id).with_context(|| format!("session not found: {id}"))?;
        vec![session]
    } else if let Some(workspace) = workspace {
        filter_workspace(&sessions, workspace)
    } else {
        sessions.iter().collect()
    };
    if selected.is_empty() {
        bail!("no sessions matched");
    }
    fs::create_dir_all(out)?;
    for session in selected {
        let path = export::export_path(out, session, format);
        let mut file = fs::File::create(&path)?;
        export::export_session(session, format, &mut file)?;
        println!("wrote {}", path.display());
    }
    Ok(())
}

fn cmd_healthcheck(paths: &StoragePaths) -> Result<()> {
    println!("Cursor session healthcheck\n");
    match &paths.chats_dir {
        Some(dir) => println!("agent chats: {} (ok)", dir.display()),
        None => println!("agent chats: not found (~/.cursor/chats)"),
    }
    match &paths.projects_dir {
        Some(dir) => println!(
            "transcripts: {}/{{project}}/agent-transcripts (ok)",
            dir.display()
        ),
        None => println!("transcripts: not found (~/.cursor/projects)"),
    }
    match &paths.global_storage_db {
        Some(db) => println!("ide db: {} (ok)", db.display()),
        None => println!("ide db: not found (state.vscdb)"),
    }

    let sessions = load_sessions(paths)?;
    let agent = sessions
        .iter()
        .filter(|s| s.source == cursor_session::model::Source::Agent)
        .count();
    let ide = sessions.len() - agent;
    println!(
        "\nsessions loaded: {} (agent: {agent}, ide: {ide})",
        sessions.len()
    );
    if sessions.is_empty() {
        println!("warning: no sessions were parsed");
    }
    Ok(())
}
