use std::borrow::Cow;
use std::fs;
use std::io::{BufWriter, ErrorKind, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use cursor_session::detect::{self, Env, StoragePaths};
use cursor_session::export::{self, Format};
use cursor_session::model::{self, Session, SessionSummary, Source, SummaryJson};
use cursor_session::ui;
use cursor_session::view::View;
use cursor_session::{
    Error, LoadOptions, Loaded, ReadOptions, filter_workspace, load_messages, load_session_with,
    load_sessions,
};
use serde::Serialize;

use crate::cli::{Commands, ExportArgs, HealthcheckArgs, ListArgs, ShowArgs};
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
        Commands::List(args) => cmd_list(paths, opts, out, err, &args),
        Commands::Show(args) => cmd_show(paths, opts, out, err, &args),
        Commands::Export(args) => cmd_export(paths, out, err, &args),
        Commands::Healthcheck(args) => cmd_healthcheck(paths, out, err, &args),
    }
}

fn load(
    paths: &StoragePaths,
    opts: &LoadOptions,
    verbose: bool,
    err: &mut dyn Write,
) -> Result<Loaded> {
    warn_unread_source(paths, opts.source, err)?;
    let loaded = load_sessions(paths, opts)?;
    print_warnings(&loaded.notices, true, err)?;
    print_warnings(&loaded.warnings, verbose, err)?;
    Ok(loaded)
}

/// Loads the session `query` names (see [`load_session_with`]), with its
/// messages read as `read` says, printing what loading reported also when it
/// fails. When it is not found, the error names the store that `--source`
/// left unsearched, if it was found.
fn load_one(
    paths: &StoragePaths,
    source: Option<Source>,
    query: &str,
    read: ReadOptions,
    verbose: bool,
    err: &mut dyn Write,
) -> Result<Session> {
    warn_unread_source(paths, source, err)?;
    let opts = LoadOptions {
        source,
        ..Default::default()
    };
    let loaded = load_session_with(paths, &opts, query, read);
    print_warnings(&loaded.notices, true, err)?;
    print_warnings(&loaded.warnings, verbose, err)?;
    let session = loaded.session.map_err(|error| match error {
        Error::SessionNotFound { query, .. } => Error::SessionNotFound {
            query,
            unsearched: source.map(Source::other).filter(|other| paths.has(*other)),
        },
        error => error,
    })?;
    Ok(session)
}

/// Warns that `--source` names a store that was not found while the other
/// one was, so nothing is read.
fn warn_unread_source(
    paths: &StoragePaths,
    source: Option<Source>,
    err: &mut dyn Write,
) -> Result<()> {
    if let Some(source) = source
        && !paths.has(source)
        && paths.has(source.other())
    {
        let warning = format!(
            "no {} storage was found, and `--source {}` leaves the {} sessions unread",
            source.name(),
            source.as_str(),
            source.other().name()
        );
        print_warnings(&[warning], true, err)?;
    }
    Ok(())
}

/// Prints each warning on one line, so that stored names in it cannot start
/// lines of their own.
fn print_warnings(warnings: &[String], verbose: bool, err: &mut dyn Write) -> Result<()> {
    if verbose {
        for warning in warnings {
            writeln!(err, "warning: {}", ui::one_line(warning))?;
        }
    }
    Ok(())
}

fn write_json(out: &mut dyn Write, value: &impl Serialize) -> Result<()> {
    let json = serde_json::to_string_pretty(value)?;
    writeln!(out, "{}", escape_controls(&json))?;
    Ok(())
}

/// JSON allows DEL and the C1 controls (U+0080 to U+009F) unescaped, but a
/// terminal acts on them, and its filter would drop them and what follows.
/// They only occur inside strings, so escaping them keeps the JSON exact.
fn escape_controls(json: &str) -> Cow<'_, str> {
    let control = |c: char| matches!(c, '\u{7f}'..='\u{9f}');
    if !json.contains(control) {
        return Cow::Borrowed(json);
    }
    let mut escaped = String::with_capacity(json.len() + 16);
    for c in json.chars() {
        if control(c) {
            escaped.push_str(&format!("\\u{:04x}", u32::from(c)));
        } else {
            escaped.push(c);
        }
    }
    Cow::Owned(escaped)
}

fn cmd_list(
    paths: &StoragePaths,
    opts: &OutputOpts,
    out: &mut dyn Write,
    err: &mut dyn Write,
    args: &ListArgs,
) -> Result<()> {
    let load_opts = LoadOptions {
        source: args.source,
        limit: args.limit,
    };
    let loaded = load(paths, &load_opts, args.verbose, err)?;
    if args.json {
        let summaries: Vec<SummaryJson> =
            loaded.sessions.iter().map(SessionSummary::json).collect();
        return write_json(out, &summaries);
    }
    // The IDs shown must tell apart those left out too, as `show` sees them.
    let ids: Vec<&str> = loaded.ids.iter().map(String::as_str).collect();
    write!(
        out,
        "{}",
        ui::render_list_among(&loaded.sessions, &ids, opts.color, opts.width)
    )?;
    Ok(())
}

fn cmd_show(
    paths: &StoragePaths,
    opts: &OutputOpts,
    out: &mut dyn Write,
    err: &mut dyn Write,
    args: &ShowArgs,
) -> Result<()> {
    if paths.is_empty() {
        return Err(Error::NoStorage.into());
    }
    let view = View {
        only: args.only.clone(),
        short: args.short,
    };
    let mut session = load_one(
        paths,
        args.source,
        &args.session_id,
        view.read_options(),
        args.verbose,
        err,
    )?;
    // The summary, with its counts, stays that of the whole session; --limit
    // and the terminal's default count only the messages printed.
    let messages = std::mem::take(&mut session.messages);
    let printed = view.apply(messages);
    if args.json {
        let (messages, _) = ui::select_messages(&printed, false, args.limit, args.all);
        return write_json(out, &session.detail(messages));
    }
    let (messages, hidden) = ui::select_messages(&printed, opts.tty, args.limit, args.all);
    write!(
        out,
        "{}",
        ui::render_show(&session, messages, hidden, opts.color)
    )?;
    // Say why --only printed nothing, rather than leave a bare header.
    if printed.is_empty() && !view.only.is_empty() {
        write!(
            out,
            "{}",
            ui::render_no_match(&view.only_list(), opts.color)
        )?;
    }
    Ok(())
}

fn cmd_export(
    paths: &StoragePaths,
    out: &mut dyn Write,
    err: &mut dyn Write,
    args: &ExportArgs,
) -> Result<()> {
    if paths.is_empty() {
        return Err(Error::NoStorage.into());
    }
    // Each session's messages are read only to write its file.
    let (one, loaded) = match &args.session_id {
        Some(id) => (
            Some(load_one(
                paths,
                args.source,
                id,
                ReadOptions::default(),
                args.verbose,
                err,
            )?),
            None,
        ),
        None => {
            let load_opts = LoadOptions {
                source: args.source,
                ..Default::default()
            };
            (None, Some(load(paths, &load_opts, args.verbose, err)?))
        }
    };
    let workspace = args.workspace.as_deref().map(resolve_workspace);
    let mut selected: Vec<&SessionSummary> = match (&one, &loaded) {
        (Some(session), _) => vec![&session.summary],
        (None, Some(loaded)) => match &workspace {
            Some(workspace) => filter_workspace(&loaded.sessions, workspace),
            None => loaded.sessions.iter().collect(),
        },
        (None, None) => Vec::new(),
    };
    if selected.is_empty() {
        if let Some(workspace) = workspace {
            return Err(Error::NoWorkspaceMatch {
                workspace: workspace.into_owned(),
            }
            .into());
        }
        bail!("no sessions to export");
    }
    if let Some(limit) = args.limit {
        selected.truncate(limit);
    }
    let out_dir = detect::expand_home(&args.out)?;
    fs::create_dir_all(&out_dir)
        .with_context(|| format!("could not create {}", out_dir.display()))?;
    // The files are the result and the `wrote` lines only report progress, so
    // a reader that goes away (`| head`) stops the lines, not the export.
    let mut progress = true;
    let mut files = export::ExportPaths::default();
    for summary in selected {
        let session = match &one {
            Some(session) => Cow::Borrowed(session),
            None => Cow::Owned(load_messages(summary)?),
        };
        let path = files.next(&out_dir, &session, args.format);
        write_export(&session, args.format, &path)
            .with_context(|| format!("could not write {}", path.display()))?;
        if progress && let Err(error) = writeln!(out, "wrote {}", path.display()) {
            if error.kind() != ErrorKind::BrokenPipe {
                return Err(error.into());
            }
            progress = false;
        }
    }
    Ok(())
}

/// `workspace` as an absolute path when it is written relative to the
/// current directory (`.`, `..`, `./x`, `../x`) or the home directory (`~`,
/// `~/x`); anything else is matched as given.
fn resolve_workspace(workspace: &str) -> Cow<'_, str> {
    let relative = [".", "..", "~"].contains(&workspace)
        || ["./", "../", "~/", ".\\", "..\\", "~\\"]
            .iter()
            .any(|prefix| workspace.starts_with(prefix));
    if !relative {
        return Cow::Borrowed(workspace);
    }
    let path = match workspace.strip_prefix('~') {
        Some(rest) => match Env::current().home {
            Some(home) => home.join(rest.trim_start_matches(['/', '\\'])),
            None => return Cow::Borrowed(workspace),
        },
        None => PathBuf::from(workspace),
    };
    // The path Cursor recorded is the one the process ran in, symlinks resolved.
    let resolved = fs::canonicalize(&path)
        .ok()
        .filter(|_| !cfg!(windows))
        .or_else(|| std::path::absolute(&path).ok().map(|path| normalize(&path)));
    match resolved {
        Some(path) => Cow::Owned(path.to_string_lossy().into_owned()),
        None => Cow::Borrowed(workspace),
    }
}

/// `path` with its `.` and `..` components resolved without the filesystem.
fn normalize(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// Writes one export file. Its I/O failures are `Error::Write`, which the
/// binary never mistakes for a closed stdout.
fn write_export(session: &Session, format: Format, path: &Path) -> cursor_session::Result<()> {
    let mut file = BufWriter::new(fs::File::create(path).map_err(Error::Write)?);
    export::export_session(session, format, &mut file)?;
    file.flush().map_err(Error::Write)
}

fn cmd_healthcheck(
    paths: &StoragePaths,
    out: &mut dyn Write,
    err: &mut dyn Write,
    args: &HealthcheckArgs,
) -> Result<()> {
    // Check each location and store on its own so one failure does not hide
    // another. The agent loader skips a location it cannot read and loads the
    // other.
    let load = |source| {
        load_sessions(
            paths,
            &LoadOptions {
                source: Some(source),
                ..Default::default()
            },
        )
    };
    let chats = check_readable(paths.chats_dir.as_deref());
    let transcripts = check_readable(paths.projects_dir.as_deref());
    let agent_store = load(Source::Agent);
    let agent_loaded = agent_store.is_ok();
    let ide_store =
        check_readable(paths.global_storage_db.as_deref()).and_then(|()| load(Source::Ide));
    // A store that loaded with notices is missing something, or shows
    // something that may be wrong.
    let incomplete = |store: &cursor_session::Result<Loaded>| {
        store
            .as_ref()
            .is_ok_and(|loaded| !loaded.notices.is_empty())
    };
    let status = |ok: bool, incomplete: bool| match (ok, incomplete) {
        (false, _) => "failed",
        (true, true) => "incomplete",
        (true, false) => "ok",
    };
    // With --storage, the default locations were never looked at.
    let searched = match args.storage {
        Some(_) => None,
        None => Env::current().candidates().ok(),
    };
    let not_found = |candidates: Option<&Vec<PathBuf>>| match candidates {
        Some(candidates) if !candidates.is_empty() => format!(
            "not found (looked in {})",
            candidates
                .iter()
                .map(|path| shown(path))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        _ => "not found".to_string(),
    };

    writeln!(out, "Cursor session healthcheck\n")?;
    match &paths.chats_dir {
        Some(dir) => writeln!(
            out,
            "agent chats: {} ({})",
            shown(dir),
            // Besides a location it cannot read, which fails that location,
            // the agent store notices only what is missing from the chats.
            status(
                chats.is_ok() && agent_store.is_ok(),
                incomplete(&agent_store) && transcripts.is_ok()
            )
        )?,
        None => writeln!(
            out,
            "agent chats: {}",
            not_found(searched.as_ref().map(|found| &found.chats))
        )?,
    }
    match &paths.projects_dir {
        Some(dir) => writeln!(
            out,
            "transcripts: {} ({})",
            shown(&dir.join("{project}").join("agent-transcripts")),
            status(transcripts.is_ok() && agent_store.is_ok(), false)
        )?,
        None => writeln!(
            out,
            "transcripts: {}",
            not_found(searched.as_ref().map(|found| &found.projects))
        )?,
    }
    match &paths.global_storage_db {
        Some(db) => writeln!(
            out,
            "ide db: {} ({})",
            shown(db),
            status(ide_store.is_ok(), incomplete(&ide_store))
        )?,
        None => writeln!(
            out,
            "ide db: {}",
            not_found(searched.as_ref().map(|found| &found.ide_db))
        )?,
    }

    let mut sessions = Vec::new();
    let mut warnings = Vec::new();
    let mut notices = Vec::new();
    let mut failed: Vec<(Source, anyhow::Error)> = [chats, transcripts]
        .into_iter()
        .filter_map(|checked| checked.err())
        .map(|error| (Source::Agent, error.into()))
        .collect();
    for (source, result) in [(Source::Agent, agent_store), (Source::Ide, ide_store)] {
        match result {
            Ok(loaded) => {
                sessions.extend(loaded.sessions);
                warnings.extend(loaded.warnings);
                notices.extend(loaded.notices);
            }
            // The agent store fails with the location check's own error when
            // none of its locations can be read; its own says how to skip it.
            Err(error) => {
                let error = anyhow::Error::from(error);
                let text = format!("{error:#}");
                match failed
                    .iter_mut()
                    .find(|(_, seen)| format!("{seen:#}") == text)
                {
                    Some((_, seen)) => *seen = error,
                    None => failed.push((source, error)),
                }
            }
        }
    }
    print_warnings(&notices, true, err)?;
    print_warnings(&warnings, args.verbose, err)?;

    let sessions = model::merge_sessions(sessions);
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
    if !args.verbose && !warnings.is_empty() {
        writeln!(
            out,
            "load warnings: {} (rerun with -v to see them)",
            warnings.len()
        )?;
    }
    for (source, error) in &failed {
        writeln!(
            out,
            "{} store failed: {}",
            source.name(),
            messages(error).join(": ")
        )?;
        for hint in hints(error, paths, true) {
            writeln!(out, "  {hint}")?;
        }
    }

    if paths.is_empty() {
        return Err(Error::NoStorage.into());
    }
    let failed_store = |store| failed.iter().any(|(source, _)| *source == store);
    // The agent store loads while one of its locations can be read.
    let agent = if agent_loaded {
        "an Agent CLI location could not be read"
    } else {
        "the Agent CLI store could not be loaded"
    };
    match (failed_store(Source::Agent), failed_store(Source::Ide)) {
        (false, false) => Ok(()),
        (true, false) => bail!("healthcheck failed: {agent}"),
        (false, true) => bail!("healthcheck failed: the IDE store could not be loaded"),
        (true, true) if agent_loaded => {
            bail!("healthcheck failed: {agent}, and the IDE store could not be loaded")
        }
        (true, true) => {
            bail!("healthcheck failed: the Agent CLI and IDE stores could not be loaded")
        }
    }
}

/// The guidance of the library error behind `error`, if any, one line each.
/// The `--source` that skips the failed store is only offered when the other
/// store was found; `healthcheck`, which takes no `--source`, names the
/// commands that do.
pub fn hints(error: &anyhow::Error, paths: &StoragePaths, healthcheck: bool) -> Vec<String> {
    let Some(error) = error
        .chain()
        .find_map(|cause| cause.downcast_ref::<Error>())
    else {
        return Vec::new();
    };
    let mut hints = error.hints();
    if let Some(store) = error.skippable() {
        if !paths.has(store.other()) {
            hints.remove(0);
        } else if healthcheck {
            hints[0] = format!("`list`, `show` and `export` accept {}", store.skip_option());
        }
    }
    hints
        .iter()
        .map(|hint| ui::one_line(hint).into_owned())
        .collect()
}

/// `error` and each of its causes, one line each. SQLite's result code, which
/// rusqlite gives as the cause of its own message, is left out.
pub fn messages(error: &anyhow::Error) -> Vec<String> {
    let mut lines = Vec::new();
    let mut after_sqlite = false;
    for cause in error.chain() {
        let repeats = after_sqlite && cause.downcast_ref::<rusqlite::ffi::Error>().is_some();
        after_sqlite = matches!(
            cause.downcast_ref::<rusqlite::Error>(),
            Some(rusqlite::Error::SqliteFailure(..))
        );
        if !repeats {
            lines.push(ui::one_line(&cause.to_string()).into_owned());
        }
    }
    lines
}

/// A path on one line.
fn shown(path: &Path) -> String {
    ui::one_line(&path.display().to_string()).into_owned()
}

/// Checks that a found location can be opened, since the loaders skip what
/// they cannot read.
fn check_readable(path: Option<&Path>) -> cursor_session::Result<()> {
    let Some(path) = path else {
        return Ok(());
    };
    let readable = if path.is_dir() {
        fs::read_dir(path).map(drop)
    } else {
        fs::File::open(path).map(drop)
    };
    readable.map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use clap::Parser;
    use serde_json::Value;

    use super::*;
    use crate::cli::Cli;

    const AGENT_ID: &str = "f4eea6d2-d2d3-41ad-b290-824445295a15";
    /// Stdout as in CI, whether or not `cargo test` runs in a terminal.
    const PIPED: OutputOpts = OutputOpts {
        tty: false,
        color: false,
        width: None,
    };

    fn fixture_projects() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/home/.cursor/projects")
    }

    fn write_ide_db(path: &Path) {
        let conn = rusqlite::Connection::open(path).unwrap();
        conn.execute(
            "CREATE TABLE cursorDiskKV (key TEXT PRIMARY KEY, value TEXT)",
            [],
        )
        .unwrap();
        let rows = [
            (
                "composerData:c0ffee00-0000-4000-8000-000000000001",
                r#"{"composerId":"c0ffee00-0000-4000-8000-000000000001","name":"Older IDE chat","createdAt":1700000000000,"lastUpdatedAt":1700000500000,"fullConversationHeadersOnly":[{"bubbleId":"b1","type":1}]}"#,
            ),
            (
                "bubbleId:c0ffee00-0000-4000-8000-000000000001:b1",
                r#"{"bubbleId":"b1","type":1,"text":"hello ide"}"#,
            ),
            (
                "composerData:c0ffee00-0000-4000-8000-000000000002",
                r#"{"composerId":"c0ffee00-0000-4000-8000-000000000002","name":"Newer IDE chat","createdAt":1800000000000}"#,
            ),
        ];
        for (key, value) in rows {
            conn.execute(
                "INSERT INTO cursorDiskKV (key, value) VALUES (?1, ?2)",
                [key, value],
            )
            .unwrap();
        }
    }

    fn paths_with_ide(dir: &Path) -> StoragePaths {
        let db = dir.join("state.vscdb");
        write_ide_db(&db);
        StoragePaths {
            projects_dir: Some(fixture_projects()),
            global_storage_db: Some(db),
            ..Default::default()
        }
    }

    fn run_args(paths: &StoragePaths, argv: &[&str]) -> (Result<()>, String) {
        let cli =
            Cli::try_parse_from(std::iter::once("cursor-session").chain(argv.iter().copied()))
                .unwrap();
        let mut out = Vec::new();
        let mut err = Vec::new();
        let result = run(cli.command, paths, &PIPED, &mut out, &mut err);
        (result, String::from_utf8(out).unwrap())
    }

    fn json(text: &str) -> Value {
        assert!(text.ends_with("}\n") || text.ends_with("]\n"), "{text:?}");
        serde_json::from_str(text).unwrap()
    }

    fn ids(list: &Value) -> Vec<&str> {
        list.as_array()
            .unwrap()
            .iter()
            .map(|s| s["id"].as_str().unwrap())
            .collect()
    }

    #[test]
    fn list_json_keeps_table_order_and_filters() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths_with_ide(dir.path());

        let (result, out) = run_args(&paths, &["list", "--json"]);
        result.unwrap();
        let all = json(&out);
        assert_eq!(
            ids(&all),
            [
                "c0ffee00-0000-4000-8000-000000000002",
                "c0ffee00-0000-4000-8000-000000000001",
                AGENT_ID
            ]
        );
        // As for agent sessions, a chat never updated has no updated_at; it is
        // ordered by its created_at, as the table's UPDATED column shows it.
        assert_eq!(all[0]["updated_at"], Value::Null);
        assert_eq!(all[0]["created_at"], "2027-01-15T08:00:00Z");
        assert_eq!(all[1]["updated_at"], "2023-11-14T22:21:40Z");
        // Transcript-only agent sessions have no timestamps and sort last.
        assert_eq!(all[2]["updated_at"], Value::Null);
        assert!(out.starts_with("[\n  {\n    \"id\": "));

        let (_, out) = run_args(&paths, &["list", "--json", "--source", "agent"]);
        assert_eq!(ids(&json(&out)), [AGENT_ID]);

        let (_, out) = run_args(
            &paths,
            &["list", "--json", "--source", "ide", "--limit", "1"],
        );
        assert_eq!(ids(&json(&out)), ["c0ffee00-0000-4000-8000-000000000002"]);

        let (_, out) = run_args(&paths, &["list", "--limit", "2"]);
        assert!(out.starts_with("Found 2 session(s)\n"));
    }

    #[test]
    fn listed_ids_tell_apart_the_sessions_limit_leaves_out() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths_with_ide(dir.path());
        let terminal = OutputOpts {
            tty: true,
            color: false,
            // Wide enough for the full IDs beside TOKENS and a title.
            width: Some(106),
        };
        let table = |argv: &[&str]| {
            let cli =
                Cli::try_parse_from(std::iter::once("cursor-session").chain(argv.iter().copied()))
                    .unwrap();
            let mut out = Vec::new();
            run(cli.command, &paths, &terminal, &mut out, &mut Vec::new()).unwrap();
            String::from_utf8(out).unwrap()
        };
        // The two IDE chats differ only in the last character of their IDs.
        for argv in [&["list"][..], &["list", "--limit", "1"]] {
            let out = table(argv);
            assert!(
                out.contains("│ c0ffee00-0000-4000-8000-000000000002 ┆"),
                "{argv:?}: {out}"
            );
            assert!(!out.contains("IDs shortened"), "{argv:?}: {out}");
        }
        // Alone, the agent session's ID is shortened.
        let out = table(&["list", "--source", "agent"]);
        assert!(out.contains("IDs shortened to "), "{out}");
    }

    #[test]
    fn source_skips_a_broken_store() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("state.vscdb");
        fs::write(
            &db,
            "not a sqlite database, only some text to fill the header",
        )
        .unwrap();
        let paths = StoragePaths {
            projects_dir: Some(fixture_projects()),
            global_storage_db: Some(db),
            ..Default::default()
        };

        let (result, out) = run_args(&paths, &["list", "--json", "--source", "agent"]);
        result.unwrap();
        assert_eq!(ids(&json(&out)), [AGENT_ID]);
        assert!(run_args(&paths, &["list"]).0.is_err());

        let (result, out) = run_args(&paths, &["show", "f4eea6d2", "--source", "agent"]);
        result.unwrap();
        assert!(out.contains(AGENT_ID));
        assert!(run_args(&paths, &["show", "f4eea6d2"]).0.is_err());

        let out_dir = dir.path().join("exports");
        let out_arg = out_dir.to_str().unwrap();
        let (result, _) = run_args(
            &paths,
            &[
                "export", "--source", "agent", "--format", "json", "--out", out_arg,
            ],
        );
        result.unwrap();
        assert!(out_dir.join(format!("{AGENT_ID}.json")).is_file());
    }

    struct Failing(ErrorKind);

    impl Write for Failing {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(self.0.into())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Err(self.0.into())
        }
    }

    #[test]
    fn export_writes_every_file_after_stdout_closes() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths_with_ide(dir.path());
        let out_dir = dir.path().join("exports");
        let export = |stdout: ErrorKind| {
            let argv = [
                "cursor-session",
                "export",
                "--out",
                out_dir.to_str().unwrap(),
            ];
            let cli = Cli::try_parse_from(argv).unwrap();
            let _ = fs::remove_dir_all(&out_dir);
            let result = run(
                cli.command,
                &paths,
                &PIPED,
                &mut Failing(stdout),
                &mut Vec::new(),
            );
            let written = fs::read_dir(&out_dir).map_or(0, Iterator::count);
            (result, written)
        };

        let (result, written) = export(ErrorKind::BrokenPipe);
        result.unwrap();
        assert_eq!(written, 3);

        let (result, written) = export(ErrorKind::PermissionDenied);
        assert!(result.is_err());
        assert_eq!(written, 1);
    }

    #[test]
    fn export_errors_name_the_path() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths_with_ide(dir.path());
        let file = dir.path().join("a-file");
        fs::write(&file, "").unwrap();
        let out = file.to_str().unwrap();

        let (result, _) = run_args(&paths, &["export", "--out", out]);
        let error = result.unwrap_err();
        assert_eq!(error.to_string(), format!("could not create {out}"));
        assert!(error.chain().nth(1).is_some());

        // A directory where the export file should go.
        let out_dir = dir.path().join("exports");
        let blocked = out_dir.join(format!("{AGENT_ID}.md"));
        fs::create_dir_all(&blocked).unwrap();
        let (result, _) = run_args(
            &paths,
            &[
                "export",
                "--source",
                "agent",
                "--out",
                out_dir.to_str().unwrap(),
            ],
        );
        let error = result.unwrap_err();
        assert_eq!(
            error.to_string(),
            format!("could not write {}", blocked.display())
        );
    }

    #[test]
    fn relative_workspaces_resolve_against_the_current_directory() {
        let cwd = std::env::current_dir().unwrap();
        let resolved = if cfg!(windows) {
            cwd.clone()
        } else {
            fs::canonicalize(&cwd).unwrap()
        };
        assert_eq!(resolve_workspace("."), resolved.to_string_lossy());
        assert_eq!(
            resolve_workspace("./missing/../elsewhere"),
            cwd.join("elsewhere").to_string_lossy()
        );
        if let Some(home) = Env::current().home {
            assert_eq!(
                resolve_workspace("~/no-such-workspace/api"),
                home.join("no-such-workspace").join("api").to_string_lossy()
            );
        }
        // Directory names and absolute paths are matched as given.
        for given in ["api", "src/api", "/Users/dana/src/api", ".hidden"] {
            assert_eq!(resolve_workspace(given), given);
        }
    }

    #[test]
    fn export_says_whether_a_filter_matched_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let paths = StoragePaths {
            projects_dir: Some(dir.path().to_path_buf()),
            ..Default::default()
        };
        let (result, _) = run_args(&paths, &["export"]);
        assert_eq!(result.unwrap_err().to_string(), "no sessions to export");
        let (result, _) = run_args(&paths, &["export", "--workspace", "/nowhere"]);
        assert_eq!(
            result.unwrap_err().to_string(),
            "no sessions matched workspace `/nowhere`"
        );
    }

    #[test]
    fn show_and_export_say_when_no_storage_was_found() {
        for argv in [&["show", "abc"][..], &["export"]] {
            let (result, out) = run_args(&StoragePaths::default(), argv);
            assert!(
                matches!(
                    result.unwrap_err().downcast_ref::<Error>(),
                    Some(Error::NoStorage)
                ),
                "{argv:?}"
            );
            assert!(out.is_empty());
        }
    }

    #[test]
    fn json_escapes_controls_a_terminal_would_act_on() {
        let title = "OSC \u{9d}0;x\u{7} DEL \u{7f} NEL \u{85} ok \u{a0}é";
        let json = serde_json::to_string_pretty(&serde_json::json!({ "title": title })).unwrap();
        let escaped = escape_controls(&json);
        // U+00A0 and later are printable and stay as they are.
        assert!(escaped.contains("OSC \\u009d0;x\\u0007 DEL \\u007f NEL \\u0085 ok \u{a0}é"));
        let parsed: Value = serde_json::from_str(&escaped).unwrap();
        assert_eq!(parsed["title"], title);
        assert!(matches!(escape_controls("[]"), Cow::Borrowed(_)));
    }

    #[test]
    fn empty_list_json_is_an_empty_array() {
        let (result, out) = run_args(&StoragePaths::default(), &["list", "--json"]);
        result.unwrap();
        assert_eq!(out, "[]\n");
    }

    #[test]
    fn show_json_includes_messages_and_honours_limit() {
        let paths = StoragePaths {
            projects_dir: Some(fixture_projects()),
            ..Default::default()
        };
        let (result, out) = run_args(&paths, &["show", "F4EEA6D2", "--json"]);
        result.unwrap();
        let detail = json(&out);
        assert_eq!(detail["id"], AGENT_ID);
        assert_eq!(detail["message_count"], 3);
        assert_eq!(detail["messages"].as_array().unwrap().len(), 3);
        assert_eq!(detail["messages"][0]["role"], "user");
        assert_eq!(detail["messages"][1]["timestamp"], Value::Null);

        let (_, out) = run_args(&paths, &["show", AGENT_ID, "--json", "--limit", "1"]);
        let detail = json(&out);
        assert_eq!(detail["message_count"], 3);
        assert_eq!(
            detail["messages"][0]["content"],
            "Here is the semantic layer plan."
        );
        assert_eq!(detail["messages"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn show_reports_typed_lookup_errors() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths_with_ide(dir.path());

        let (result, out) = run_args(&paths, &["show", "c0ffee00"]);
        let error = result.unwrap_err();
        assert!(matches!(
            error.downcast_ref::<Error>(),
            Some(Error::AmbiguousId { matches, .. }) if matches.len() == 2
        ));
        assert!(out.is_empty());

        let (result, _) = run_args(&paths, &["export", "--session-id", "nope"]);
        assert!(matches!(
            result.unwrap_err().downcast_ref::<Error>(),
            Some(Error::SessionNotFound { .. })
        ));
    }

    #[test]
    fn healthcheck_names_the_failed_store() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("state.vscdb");
        fs::write(
            &db,
            "not a sqlite database, only some text to fill the header",
        )
        .unwrap();
        let paths = StoragePaths {
            projects_dir: Some(fixture_projects()),
            global_storage_db: Some(db),
            ..Default::default()
        };

        let (result, out) = run_args(&paths, &["healthcheck"]);
        let error = result.unwrap_err().to_string();
        assert_eq!(
            error,
            "healthcheck failed: the IDE store could not be loaded"
        );
        assert!(out.contains("agent-transcripts (ok)\n"));
        assert!(out.contains("state.vscdb (failed)\n"));
        assert!(out.contains("sessions loaded: 1 (agent: 1, ide: 0)\n"));
        assert!(out.contains("IDE store failed: could not read SQLite database "));
    }

    #[test]
    fn healthcheck_fails_when_a_found_store_cannot_be_opened() {
        let dir = tempfile::tempdir().unwrap();
        // The IDE loader skips a database it cannot open, so this alone would pass.
        let paths = StoragePaths {
            projects_dir: Some(fixture_projects()),
            global_storage_db: Some(dir.path().join("state.vscdb")),
            ..Default::default()
        };
        let (result, out) = run_args(&paths, &["healthcheck"]);
        assert_eq!(
            result.unwrap_err().to_string(),
            "healthcheck failed: the IDE store could not be loaded"
        );
        assert!(out.contains("state.vscdb (failed)\n"));
        assert!(out.contains("agent-transcripts (ok)\n"));
        assert!(out.contains("IDE store failed: could not access "));
    }

    #[cfg(unix)]
    #[test]
    fn healthcheck_reports_each_agent_location_on_its_own() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let chats = dir.path().join("chats");
        let projects = dir.path().join("projects");
        for locked in [&chats, &projects] {
            fs::create_dir(locked).unwrap();
            fs::set_permissions(locked, fs::Permissions::from_mode(0o000)).unwrap();
        }
        let readable = fs::read_dir(&chats).is_ok(); // root ignores permissions
        let paths = StoragePaths {
            chats_dir: Some(chats.clone()),
            projects_dir: Some(fixture_projects()),
            ..Default::default()
        };
        let (result, out) = run_args(&paths, &["healthcheck"]);
        // Neither location readable: each is reported once.
        let both = StoragePaths {
            projects_dir: Some(projects.clone()),
            ..paths.clone()
        };
        let (both_result, both_out) = run_args(&both, &["healthcheck"]);
        let with_ide = paths_with_ide(dir.path());
        let with_ide = StoragePaths {
            chats_dir: Some(chats.clone()),
            projects_dir: Some(projects.clone()),
            ..with_ide
        };
        let (ide_result, ide_out) = run_args(&with_ide, &["healthcheck"]);
        for locked in [&chats, &projects] {
            fs::set_permissions(locked, fs::Permissions::from_mode(0o755)).unwrap();
        }
        if readable {
            eprintln!("skipped: permissions are not enforced for this user (root)");
            return;
        }
        // Two failed locations are still one failed store.
        assert_eq!(
            both_result.unwrap_err().to_string(),
            "healthcheck failed: the Agent CLI store could not be loaded"
        );
        let failures: Vec<&str> = both_out
            .lines()
            .filter(|l| l.contains(" failed: "))
            .collect();
        assert_eq!(failures.len(), 2, "{both_out}");
        assert!(failures[1].contains(&projects.display().to_string()));
        // With the IDE store found, the report says how to skip this one.
        assert_eq!(
            ide_result.unwrap_err().to_string(),
            "healthcheck failed: the Agent CLI store could not be loaded"
        );
        assert!(
            ide_out.contains(&format!(
                "Agent CLI store failed: could not access {}: Permission denied (os error 13)\n  \
                 `list`, `show` and `export` accept `--source ide` to skip Agent CLI sessions\n",
                chats.display()
            )),
            "{ide_out}"
        );

        // The transcripts still load, so only a location failed.
        assert_eq!(
            result.unwrap_err().to_string(),
            "healthcheck failed: an Agent CLI location could not be read"
        );
        assert!(out.contains(&format!("agent chats: {} (failed)\n", chats.display())));
        assert!(out.contains("agent-transcripts (ok)\n"), "{out}");
        assert!(
            out.contains("sessions loaded: 1 (agent: 1, ide: 0)\n"),
            "{out}"
        );
        let failures: Vec<&str> = out.lines().filter(|l| l.contains(" failed: ")).collect();
        assert_eq!(failures.len(), 1, "{out}");
        assert!(failures[0].starts_with(&format!(
            "Agent CLI store failed: could not access {}: ",
            chats.display()
        )));
    }

    #[test]
    fn healthcheck_fails_without_any_storage() {
        let (result, out) = run_args(&StoragePaths::default(), &["healthcheck"]);
        assert!(matches!(
            result.unwrap_err().downcast_ref::<Error>(),
            Some(Error::NoStorage)
        ));
        assert!(out.contains("sessions loaded: 0 (agent: 0, ide: 0)\n"));
    }

    #[test]
    fn warnings_print_only_when_verbose() {
        let warnings = ["skipped composerData:x".to_string()];
        let mut err = Vec::new();
        print_warnings(&warnings, false, &mut err).unwrap();
        assert!(err.is_empty());
        print_warnings(&warnings, true, &mut err).unwrap();
        assert_eq!(err, b"warning: skipped composerData:x\n");
    }

    #[test]
    fn healthcheck_passes_with_both_stores() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths_with_ide(dir.path());
        let (result, out) = run_args(&paths, &["healthcheck"]);
        result.unwrap();
        assert!(out.ends_with("\nsessions loaded: 3 (agent: 1, ide: 2)\n"));
    }
}
