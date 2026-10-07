use std::fs;
use std::io::{BufWriter, ErrorKind, Write};
use std::path::Path;

use anyhow::{Context, Result, bail};
use cursor_session::detect::StoragePaths;
use cursor_session::export::{self, Format};
use cursor_session::model::{self, Session, SessionSummary, Source};
use cursor_session::ui;
use cursor_session::{Error, LoadOptions, filter_workspace, find_session, load_sessions};
use serde::Serialize;

use crate::cli::{Commands, ExportArgs, ListArgs, ShowArgs};
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
        Commands::Healthcheck(args) => cmd_healthcheck(paths, out, err, args.verbose),
    }
}

fn load(
    paths: &StoragePaths,
    opts: &LoadOptions,
    verbose: bool,
    err: &mut dyn Write,
) -> Result<Vec<Session>> {
    let loaded = load_sessions(paths, opts)?;
    print_warnings(&loaded.warnings, verbose, err)?;
    Ok(loaded.sessions)
}

fn print_warnings(warnings: &[String], verbose: bool, err: &mut dyn Write) -> Result<()> {
    if verbose {
        for warning in warnings {
            writeln!(err, "warning: {warning}")?;
        }
    }
    Ok(())
}

fn write_json(out: &mut dyn Write, value: &impl Serialize) -> Result<()> {
    let json = serde_json::to_string_pretty(value)?;
    writeln!(out, "{json}")?;
    Ok(())
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
    };
    let mut sessions = load(paths, &load_opts, args.verbose, err)?;
    if let Some(limit) = args.limit {
        sessions.truncate(limit);
    }
    if args.json {
        let summaries: Vec<SessionSummary> = sessions.iter().map(Session::summary).collect();
        return write_json(out, &summaries);
    }
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
    args: &ShowArgs,
) -> Result<()> {
    let load_opts = LoadOptions {
        source: args.source,
    };
    let sessions = load(paths, &load_opts, args.verbose, err)?;
    let session = find_session(&sessions, &args.session_id)?;
    if args.json {
        let (messages, _) = ui::select_messages(&session.messages, false, args.limit, args.all);
        return write_json(out, &session.detail(messages));
    }
    let (messages, hidden) = ui::select_messages(&session.messages, opts.tty, args.limit, args.all);
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
    args: &ExportArgs,
) -> Result<()> {
    let load_opts = LoadOptions {
        source: args.source,
    };
    let sessions = load(paths, &load_opts, args.verbose, err)?;
    let selected: Vec<&Session> = if let Some(id) = &args.session_id {
        vec![find_session(&sessions, id)?]
    } else if let Some(workspace) = &args.workspace {
        filter_workspace(&sessions, workspace)
    } else {
        sessions.iter().collect()
    };
    if selected.is_empty() {
        if args.workspace.is_some() {
            bail!("no sessions matched");
        }
        bail!("no sessions to export");
    }
    fs::create_dir_all(&args.out)
        .with_context(|| format!("could not create {}", args.out.display()))?;
    // The files are the result and the `wrote` lines only report progress, so
    // a reader that goes away (`| head`) stops the lines, not the export.
    let mut progress = true;
    for session in selected {
        let path = export::export_path(&args.out, session, args.format);
        write_export(session, args.format, &path)
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
    verbose: bool,
) -> Result<()> {
    // Check each location and store on its own so one failure does not hide
    // another. The agent loader skips a location it cannot read and loads the
    // other.
    let load = |source| {
        load_sessions(
            paths,
            &LoadOptions {
                source: Some(source),
            },
        )
    };
    let chats = check_readable(paths.chats_dir.as_deref());
    let transcripts = check_readable(paths.projects_dir.as_deref());
    let agent_store = load(Source::Agent);
    let ide_store =
        check_readable(paths.global_storage_db.as_deref()).and_then(|()| load(Source::Ide));
    let status = |ok: bool| if ok { "ok" } else { "failed" };

    writeln!(out, "Cursor session healthcheck\n")?;
    match &paths.chats_dir {
        Some(dir) => writeln!(
            out,
            "agent chats: {} ({})",
            dir.display(),
            status(chats.is_ok() && agent_store.is_ok())
        )?,
        None => writeln!(out, "agent chats: not found (~/.cursor/chats)")?,
    }
    match &paths.projects_dir {
        Some(dir) => writeln!(
            out,
            "transcripts: {}/{{project}}/agent-transcripts ({})",
            dir.display(),
            status(transcripts.is_ok() && agent_store.is_ok())
        )?,
        None => writeln!(out, "transcripts: not found (~/.cursor/projects)")?,
    }
    match &paths.global_storage_db {
        Some(db) => writeln!(
            out,
            "ide db: {} ({})",
            db.display(),
            status(ide_store.is_ok())
        )?,
        None => writeln!(out, "ide db: not found (state.vscdb)")?,
    }

    let mut sessions = Vec::new();
    let mut warnings = Vec::new();
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
            }
            Err(error) => failed.push((source, anyhow::Error::from(error))),
        }
    }
    print_warnings(&warnings, verbose, err)?;

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
    for (source, error) in &failed {
        writeln!(out, "{} store failed: {error:#}", source.as_str())?;
    }

    if paths.chats_dir.is_none()
        && paths.projects_dir.is_none()
        && paths.global_storage_db.is_none()
    {
        return Err(Error::NoStorage.into());
    }
    let mut failed_stores: Vec<Source> = failed.iter().map(|(source, _)| *source).collect();
    failed_stores.dedup();
    match failed_stores.as_slice() {
        [] => Ok(()),
        [source] => bail!(
            "healthcheck failed: the {} store could not be loaded",
            source.as_str()
        ),
        _ => bail!("healthcheck failed: the agent and ide stores could not be loaded"),
    }
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
    use crate::cli::{Cli, ColorChoice};

    const AGENT_ID: &str = "f4eea6d2-d2d3-41ad-b290-824445295a15";

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
        let opts = OutputOpts::detect(ColorChoice::Never);
        let mut out = Vec::new();
        let mut err = Vec::new();
        let result = run(cli.command, paths, &opts, &mut out, &mut err);
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
        assert_eq!(all[0]["updated_at"], "2027-01-15T08:00:00Z");
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
            let opts = OutputOpts::detect(ColorChoice::Never);
            let _ = fs::remove_dir_all(&out_dir);
            let result = run(
                cli.command,
                &paths,
                &opts,
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
    fn export_says_whether_a_filter_matched_nothing() {
        let paths = StoragePaths::default();
        let (result, _) = run_args(&paths, &["export"]);
        assert_eq!(result.unwrap_err().to_string(), "no sessions to export");
        let (result, _) = run_args(&paths, &["export", "--workspace", "/nowhere"]);
        assert_eq!(result.unwrap_err().to_string(), "no sessions matched");
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
            "healthcheck failed: the ide store could not be loaded"
        );
        assert!(out.contains("agent-transcripts (ok)\n"));
        assert!(out.contains("state.vscdb (failed)\n"));
        assert!(out.contains("sessions loaded: 1 (agent: 1, ide: 0)\n"));
        assert!(out.contains("ide store failed: failed to read sqlite database: "));
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
            "healthcheck failed: the ide store could not be loaded"
        );
        assert!(out.contains("state.vscdb (failed)\n"));
        assert!(out.contains("agent-transcripts (ok)\n"));
        assert!(out.contains("ide store failed: could not access "));
    }

    #[cfg(unix)]
    #[test]
    fn healthcheck_reports_each_agent_location_on_its_own() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let chats = dir.path().join("chats");
        fs::create_dir(&chats).unwrap();
        fs::set_permissions(&chats, fs::Permissions::from_mode(0o000)).unwrap();
        let readable = fs::read_dir(&chats).is_ok(); // root ignores permissions
        let paths = StoragePaths {
            chats_dir: Some(chats.clone()),
            projects_dir: Some(fixture_projects()),
            ..Default::default()
        };
        let (result, out) = run_args(&paths, &["healthcheck"]);
        fs::set_permissions(&chats, fs::Permissions::from_mode(0o755)).unwrap();
        if readable {
            return;
        }

        assert_eq!(
            result.unwrap_err().to_string(),
            "healthcheck failed: the agent store could not be loaded"
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
            "agent store failed: could not access {}: ",
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
