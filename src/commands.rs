use std::fs;
use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result, bail};
use cursor_session::detect::StoragePaths;
use cursor_session::export::{self, Format};
use cursor_session::model::{Session, Source};
use cursor_session::ui;
use cursor_session::{LoadOptions, filter_workspace, find_session, load_sessions};

use crate::cli::Commands;
use crate::output::OutputOpts;

/// Runs one subcommand. Normal output goes to `out`, diagnostics such as load
/// warnings go to `err`.
pub fn run(
    command: Commands,
    paths: &StoragePaths,
    opts: &OutputOpts,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<()> {
    match command {
        Commands::List => cmd_list(paths, opts, out, err),
        Commands::Show {
            session_id,
            limit,
            all,
        } => cmd_show(paths, opts, out, err, &session_id, limit, all),
        Commands::Export {
            format,
            out: out_dir,
            session_id,
            workspace,
        } => cmd_export(
            paths,
            out,
            err,
            format,
            &out_dir,
            session_id.as_deref(),
            workspace.as_deref(),
        ),
        Commands::Healthcheck => cmd_healthcheck(paths, out, err),
    }
}

fn load(paths: &StoragePaths, opts: &LoadOptions, err: &mut dyn Write) -> Result<Vec<Session>> {
    let loaded = load_sessions(paths, opts)?;
    for warning in &loaded.warnings {
        writeln!(err, "warning: {warning}")?;
    }
    Ok(loaded.sessions)
}

fn cmd_list(
    paths: &StoragePaths,
    opts: &OutputOpts,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<()> {
    let sessions = load(paths, &LoadOptions::default(), err)?;
    write!(
        out,
        "{}",
        ui::render_list(&sessions, opts.color, opts.width)
    )?;
    Ok(())
}

fn cmd_show(
    paths: &StoragePaths,
    opts: &OutputOpts,
    out: &mut dyn Write,
    err: &mut dyn Write,
    session_id: &str,
    limit: Option<usize>,
    all: bool,
) -> Result<()> {
    let sessions = load(paths, &LoadOptions::default(), err)?;
    let Some(session) = find_session(&sessions, session_id) else {
        bail!("session not found: {session_id}\nUse `cursor-session list` to see IDs.");
    };
    let (messages, hidden) = ui::select_messages(&session.messages, opts.tty, limit, all);
    write!(
        out,
        "{}",
        ui::render_show(session, messages, hidden, opts.color)
    )?;
    Ok(())
}

fn cmd_export(
    paths: &StoragePaths,
    out: &mut dyn Write,
    err: &mut dyn Write,
    format: Format,
    out_dir: &Path,
    session_id: Option<&str>,
    workspace: Option<&str>,
) -> Result<()> {
    let sessions = load(paths, &LoadOptions::default(), err)?;
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
    fs::create_dir_all(out_dir)?;
    for session in selected {
        let path = export::export_path(out_dir, session, format);
        let mut file = fs::File::create(&path)?;
        export::export_session(session, format, &mut file)?;
        writeln!(out, "wrote {}", path.display())?;
    }
    Ok(())
}

fn cmd_healthcheck(paths: &StoragePaths, out: &mut dyn Write, err: &mut dyn Write) -> Result<()> {
    writeln!(out, "Cursor session healthcheck\n")?;
    match &paths.chats_dir {
        Some(dir) => writeln!(out, "agent chats: {} (ok)", dir.display())?,
        None => writeln!(out, "agent chats: not found (~/.cursor/chats)")?,
    }
    match &paths.projects_dir {
        Some(dir) => writeln!(
            out,
            "transcripts: {}/{{project}}/agent-transcripts (ok)",
            dir.display()
        )?,
        None => writeln!(out, "transcripts: not found (~/.cursor/projects)")?,
    }
    match &paths.global_storage_db {
        Some(db) => writeln!(out, "ide db: {} (ok)", db.display())?,
        None => writeln!(out, "ide db: not found (state.vscdb)")?,
    }

    let sessions = load(paths, &LoadOptions::default(), err)?;
    let agent = sessions
        .iter()
        .filter(|s| s.source == Source::Agent)
        .count();
    let ide = sessions.len() - agent;
    writeln!(
        out,
        "\nsessions loaded: {} (agent: {agent}, ide: {ide})",
        sessions.len()
    )?;
    if sessions.is_empty() {
        writeln!(out, "warning: no sessions were parsed")?;
    }
    Ok(())
}
