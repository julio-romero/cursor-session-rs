//! The binary end to end. Every run is confined to a fixture home (see
//! `Fixture::command`), so no test can reach real Cursor data.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::fs;
use std::io::Read;
use std::path::Path;
use std::process::{Output, Stdio};

use common::*;
use cursor_session::model::{Message, Session};
use cursor_session::ui;
use predicates::prelude::*;
use serde_json::Value;

const SUBCOMMANDS: [&str; 4] = ["list", "show", "export", "healthcheck"];

/// The start of a usage line. clap names the program after the file it ran
/// from, which ends in `.exe` on Windows.
fn usage(rest: &str) -> String {
    format!(
        "Usage: cursor-session{} {rest}",
        std::env::consts::EXE_SUFFIX
    )
}

fn run(fixture: &Fixture, args: &[&str]) -> Output {
    fixture.cmd().args(args).output().unwrap()
}

/// Runs a command that must succeed quietly and returns its stdout.
fn ok(fixture: &Fixture, args: &[&str]) -> String {
    let output = run(fixture, args);
    assert!(output.status.success(), "{args:?}: {}", stderr(&output));
    assert_eq!(stderr(&output), "", "{args:?}");
    stdout(&output)
}

/// Runs a command that must fail with exit code 1 and returns its stderr.
fn fails(fixture: &Fixture, args: &[&str]) -> String {
    let output = run(fixture, args);
    let err = stderr(&output);
    assert_eq!(output.status.code(), Some(1), "{args:?}: {err}");
    assert_eq!(stdout(&output), "", "{args:?}");
    assert!(!err.contains("panicked"), "{err}");
    err
}

fn json(text: &str) -> Value {
    assert!(text.ends_with("]\n") || text.ends_with("}\n"), "{text:?}");
    serde_json::from_str(text).unwrap()
}

fn ids(list: &Value) -> Vec<&str> {
    list.as_array()
        .unwrap()
        .iter()
        .map(|s| s["id"].as_str().unwrap())
        .collect()
}

fn keys(value: &Value) -> Vec<&str> {
    value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect()
}

const SUMMARY_KEYS: [&str; 9] = [
    "id",
    "title",
    "source",
    "workspace",
    "workspace_hash",
    "model",
    "created_at",
    "updated_at",
    "message_count",
];

#[test]
fn version_is_the_package_version() {
    let fixture = Fixture::new();
    for flag in ["--version", "-V"] {
        assert_eq!(
            ok(&fixture, &[flag]),
            format!("cursor-session {}\n", env!("CARGO_PKG_VERSION"))
        );
    }
}

#[test]
fn help_describes_every_subcommand() {
    let fixture = Fixture::new();
    for flag in ["--help", "-h"] {
        let help = ok(&fixture, &[flag]);
        assert!(help.starts_with("List, show, and export Cursor IDE and Agent CLI chat sessions"));
        for command in SUBCOMMANDS {
            assert!(
                help.contains(&format!("\n  {command} ")),
                "{flag}: {command}\n{help}"
            );
        }
        assert!(help.contains("--storage <PATH>"));
        assert!(help.contains("Examples:"));
    }
    let long = ok(&fixture, &["--help"]);
    assert!(long.contains("Exit codes:"));
    assert!(long.contains("~/.cursor/chats"));
    assert!(long.contains("state.vscdb"));

    for command in SUBCOMMANDS {
        let help = ok(&fixture, &[command, "--help"]);
        assert!(help.contains(&usage(command)), "{help}");
    }
    for flag in ["--json", "--source <SOURCE>", "--limit <N>"] {
        assert!(ok(&fixture, &["list", "--help"]).contains(flag), "{flag}");
        assert!(ok(&fixture, &["show", "--help"]).contains(flag), "{flag}");
    }
}

#[test]
fn usage_errors_exit_2() {
    let fixture = standard();
    for args in [
        &[][..],
        &["frobnicate"],
        &["list", "--bogus"],
        &["list", "--limit", "0"],
        &["list", "--limit", "-1"],
        &["list", "--limit", "many"],
        &["list", "--source", "web"],
        &["show"],
        &["show", AGENT_ID, "--limit", "0"],
        &["show", AGENT_ID, "--limit", "1", "--all"],
        &["export", "--format", "pdf"],
        &["export", "--session-id", "f4ee", "--workspace", PROJECT_X],
        &["--color", "sometimes", "list"],
    ] {
        let output = run(&fixture, args);
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert_eq!(stdout(&output), "", "{args:?}");
        let err = stderr(&output);
        if args.is_empty() {
            // Without a subcommand the help itself is the message.
            assert!(err.contains(&format!("\n{}", usage("[OPTIONS]"))), "{err}");
        } else {
            assert!(err.starts_with("error: "), "{args:?}: {err}");
            assert!(err.contains("try '--help'"), "{args:?}: {err}");
        }
    }
}

#[test]
fn unknown_id_exits_1_with_a_hint() {
    let fixture = standard();
    for args in [&["show", "ffff"][..], &["export", "--session-id", "ffff"]] {
        assert_eq!(
            fails(&fixture, args),
            "error: session not found: ffff\nrun `cursor-session list` to see session IDs\n"
        );
    }
}

#[test]
fn empty_id_exits_1_with_a_hint() {
    let fixture = standard();
    for id in ["", "   "] {
        assert_eq!(
            fails(&fixture, &["show", id]),
            "error: session id is empty\nrun `cursor-session list` to see session IDs\n"
        );
    }
}

fn ambiguous_fixture() -> Fixture {
    let fixture = Fixture::new();
    write_standard_agent(&fixture);
    let mut rows = standard_ide_rows();
    for (id, name, at) in [
        (
            "abcd1111-0000-4000-8000-000000000001",
            "Older abcd",
            1_757_500_000_000,
        ),
        (
            "abcd2222-0000-4000-8000-000000000002",
            "Newer abcd",
            1_757_600_000_000,
        ),
    ] {
        rows.push(composer(
            id,
            &composer_json(id, name, at, at, &[]),
            Stored::Text,
        ));
    }
    fixture.write_ide_db(Journal::Delete, &rows);
    fixture
}

#[test]
fn ambiguous_prefix_exits_1_and_lists_the_candidates() {
    let fixture = ambiguous_fixture();
    let expected = "error: session id prefix \"ABCD\" is ambiguous (2 matches)\n  \
                    abcd2222-0000-4000-8000-000000000002  ide    Newer abcd\n  \
                    abcd1111-0000-4000-8000-000000000001  ide    Older abcd\n\
                    use more characters of the ID\n";
    assert_eq!(fails(&fixture, &["show", "ABCD"]), expected);
    assert_eq!(fails(&fixture, &["show", "ABCD", "--json"]), expected);
    assert_eq!(
        fails(
            &fixture,
            &["export", "--session-id", "ABCD", "--out", "out"]
        ),
        expected
    );
    assert!(!fixture.home().join("out").exists());

    let shown = json(&ok(&fixture, &["show", "AbCd2", "--json"]));
    assert_eq!(shown["title"], "Newer abcd");
}

#[test]
fn show_accepts_case_insensitive_prefixes() {
    let fixture = standard();
    for query in ["f4eea6d2", "F4EEA6D2-D2D3", AGENT_ID, " f4ee "] {
        let detail = json(&ok(&fixture, &["show", query, "--json"]));
        assert_eq!(detail["id"], AGENT_ID, "{query:?}");
    }
}

#[test]
fn list_json_has_the_documented_keys() {
    let fixture = standard();
    let list = json(&ok(&fixture, &["list", "--json"]));
    assert_eq!(ids(&list), STANDARD_IDS);
    for summary in list.as_array().unwrap() {
        assert_eq!(keys(summary), SUMMARY_KEYS);
        assert!(["agent", "ide"].contains(&summary["source"].as_str().unwrap()));
        assert!(summary["message_count"].is_u64());
        for time in ["created_at", "updated_at"] {
            let value = &summary[time];
            assert!(
                value.is_null() || value.as_str().unwrap().ends_with('Z'),
                "{value}"
            );
        }
    }
    assert_eq!(
        list[5],
        serde_json::json!({
            "id": AGENT_ID,
            "title": "Langfuse Semantic Layer",
            "source": "agent",
            "workspace": PROJECT_X,
            "workspace_hash": cursor_session::detect::workspace_md5(PROJECT_X),
            "model": "claude-4.5-sonnet",
            "created_at": "2025-09-04T15:33:20Z",
            "updated_at": "2025-09-04T16:33:20Z",
            "message_count": 4,
        })
    );
    // No update time is shown as null, not as the creation time.
    assert_eq!(list[4]["updated_at"], Value::Null);
    assert_eq!(list[4]["created_at"], "2025-09-05T19:20:00Z");
}

#[test]
fn list_filters_by_source_and_limit() {
    let fixture = standard();
    let listed = |args: &[&str]| -> Vec<String> {
        let mut argv = vec!["list", "--json"];
        argv.extend_from_slice(args);
        ids(&json(&ok(&fixture, &argv)))
            .into_iter()
            .map(str::to_string)
            .collect()
    };
    assert_eq!(
        listed(&["--source", "agent"]),
        [SHARED_ID, STORE_ONLY_ID, AGENT_ID, TRANSCRIPT_ONLY_ID]
    );
    assert_eq!(
        listed(&["--source", "ide"]),
        [SHARED_ID, IDE_BLOB_ID, IDE_UNTITLED_ID, IDE_TEXT_ID]
    );
    assert_eq!(listed(&["--limit", "2"]), [SHARED_ID, IDE_BLOB_ID]);
    assert_eq!(listed(&["--limit", "100"]).len(), STANDARD_IDS.len());
    assert_eq!(
        listed(&["--source", "agent", "--limit", "3"]),
        [SHARED_ID, STORE_ONLY_ID, AGENT_ID]
    );
    let ide: Vec<Value> = json(&ok(&fixture, &["list", "--json", "--source", "ide"]))
        .as_array()
        .unwrap()
        .clone();
    assert!(ide.iter().all(|s| s["source"] == "ide"));

    let table = ok(&fixture, &["list", "--limit", "2", "--source", "ide"]);
    assert!(table.starts_with("Found 2 session(s)\n"));
    assert!(table.contains(SHARED_ID) && table.contains(IDE_BLOB_ID));
}

#[test]
fn show_json_has_the_documented_keys() {
    let fixture = standard();
    let detail = json(&ok(&fixture, &["show", AGENT_ID, "--json"]));
    let mut expected = SUMMARY_KEYS.to_vec();
    expected.push("messages");
    assert_eq!(keys(&detail), expected);
    assert_eq!(detail["message_count"], 4);
    let messages = detail["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 4);
    for message in messages {
        assert_eq!(keys(message), ["role", "content", "timestamp"]);
    }
    assert_eq!(
        messages[0]["timestamp"],
        "Thursday, Sep 4, 2025, 9:33 AM (UTC-6)"
    );
    assert_eq!(messages[1]["timestamp"], Value::Null);

    // --limit keeps the last N messages; message_count stays the total.
    let detail = json(&ok(&fixture, &["show", AGENT_ID, "--json", "--limit", "1"]));
    assert_eq!(detail["message_count"], 4);
    assert_eq!(
        detail["messages"],
        serde_json::json!([{
            "role": "assistant",
            "content": "Here is the plan:\n1. Model traces as facts.\n2. Expose metrics.",
            "timestamp": null,
        }])
    );
    let all = json(&ok(&fixture, &["show", AGENT_ID, "--json", "--all"]));
    assert_eq!(all["messages"].as_array().unwrap().len(), 4);

    let ide = json(&ok(
        &fixture,
        &["show", "c0ffee00", "--json", "--source", "ide"],
    ));
    assert_eq!(ide["source"], "ide");
    assert_eq!(ide["messages"][0]["timestamp"], "1757200000000");
    assert_eq!(
        fails(&fixture, &["show", "c0ffee00", "--source", "agent"]),
        "error: session not found: c0ffee00\nrun `cursor-session list` to see session IDs\n"
    );
}

#[test]
fn empty_home_lists_nothing() {
    let fixture = Fixture::new();
    assert_eq!(ok(&fixture, &["list", "--json"]), "[]\n");
    assert_eq!(ok(&fixture, &["list"]), "No sessions found\n");
    assert_eq!(ok(&fixture, &["list", "--source", "ide", "--json"]), "[]\n");
    assert_eq!(
        fails(&fixture, &["show", "abc"]),
        "error: session not found: abc\nrun `cursor-session list` to see session IDs\n"
    );
}

#[test]
fn piped_output_is_the_plain_layout_without_escape_sequences() {
    let fixture = standard();
    let sessions = fixture.load().sessions;
    let agent = sessions.iter().find(|s| s.id == AGENT_ID).unwrap();
    // A capable terminal type alone does not color a pipe.
    let plain = |args: &[&str]| {
        let output = fixture
            .cmd()
            .args(args)
            .env("TERM", "xterm-256color")
            .output()
            .unwrap();
        assert!(output.status.success(), "{args:?}");
        let out = stdout(&output);
        assert!(!out.contains('\u{1b}'), "{args:?}: {out:?}");
        out
    };
    assert_eq!(plain(&["list"]), ui::render_list(&sessions, false, None));
    assert_eq!(
        plain(&["show", AGENT_ID]),
        ui::render_show(agent, &agent.messages, None, false)
    );
    for args in [
        &["list", "--json"][..],
        &["show", AGENT_ID, "--json"],
        &["healthcheck"],
        &["list", "--color", "never"],
    ] {
        plain(args);
    }
    // A pipe gets every message, not the terminal's last 20.
    let long = long_session_fixture(25);
    let out = ok(&long, &["show", "long"]);
    assert_eq!(out.matches("\n[user]\n").count(), 25);
    assert!(!out.contains("omitted"));
}

#[test]
fn color_always_colors_piped_output() {
    let fixture = standard();
    let sessions = fixture.load().sessions;
    let agent = sessions.iter().find(|s| s.id == AGENT_ID).unwrap();
    for no_color in [None, Some("1")] {
        let colored = |args: &[&str]| {
            let mut cmd = fixture.cmd();
            if let Some(value) = no_color {
                cmd.env("NO_COLOR", value);
            }
            let output = cmd.args(args).output().unwrap();
            assert!(output.status.success(), "{args:?}");
            stdout(&output)
        };
        assert_eq!(
            colored(&["list", "--color", "always"]),
            ui::render_list(&sessions, true, None)
        );
        assert_eq!(
            colored(&["--color=always", "show", AGENT_ID]),
            ui::render_show(agent, &agent.messages, None, true)
        );
        // JSON is never colored.
        assert!(!colored(&["list", "--json", "--color", "always"]).contains('\u{1b}'));
    }
}

/// One agent session, `long`, with `exchanges` questions and answers.
fn long_session_fixture(exchanges: usize) -> Fixture {
    let fixture = Fixture::new();
    let lines: Vec<Value> = (1..=exchanges)
        .flat_map(|n| {
            [
                plain_message("user", &format!("question {n}")),
                plain_message("assistant", &format!("answer {n}")),
            ]
        })
        .collect();
    fixture.write_transcript("p", "long", Layout::Nested, &lines);
    fixture
}

fn big_fixture() -> Fixture {
    let fixture = Fixture::new();
    let mut rows = Vec::new();
    let mut conversation = Vec::new();
    for n in 0..2000_i64 {
        let bubble_id = format!("b{n}");
        rows.push(bubble(
            "00000000-0000-4000-8000-000000000000",
            &bubble_id,
            &text_bubble(
                &bubble_id,
                1 + n % 2,
                &format!("message {n} {}", "lorem ipsum ".repeat(30)),
            ),
            Stored::Blob,
        ));
        conversation.push((bubble_id, 1 + n % 2));
    }
    let headers: Vec<(&str, i64)> = conversation
        .iter()
        .map(|(id, kind)| (id.as_str(), *kind))
        .collect();
    for n in 0..4000_i64 {
        let id = format!("{n:08x}-0000-4000-8000-{n:012x}");
        let bubbles = if n == 0 { &headers[..] } else { &[] };
        let name = format!("Session {n} {}", "padding ".repeat(16));
        rows.push(composer(
            &id,
            &composer_json(
                &id,
                &name,
                1_757_000_000_000 + n,
                1_757_000_000_000 + n,
                bubbles,
            ),
            Stored::Blob,
        ));
    }
    fixture.write_ide_db(Journal::Delete, &rows);
    fixture
}

/// Reads the start of the output, then closes the pipe while the binary is
/// still writing, as `| head` does.
fn hang_up_early(fixture: &Fixture, args: &[&str]) -> Output {
    let mut child = fixture
        .command()
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdout = child.stdout.take().unwrap();
    let mut start = [0; 512];
    stdout.read_exact(&mut start).unwrap();
    drop(stdout);
    child.wait_with_output().unwrap()
}

#[test]
fn a_closed_pipe_ends_the_output_quietly() {
    let fixture = big_fixture();
    for args in [
        &["list"][..],
        &["list", "--json"],
        &["list", "--color", "always"],
        &["show", "00000000", "--all"],
        &["show", "00000000", "--json"],
    ] {
        // Far more than any pipe buffer, so the binary is mid-write when the
        // reader goes away.
        let full = run(&fixture, args);
        assert!(full.status.success(), "{args:?}");
        assert!(
            full.stdout.len() > 512 * 1024,
            "{args:?}: {}",
            full.stdout.len()
        );

        let output = hang_up_early(&fixture, args);
        let err = stderr(&output);
        assert!(
            output.status.success(),
            "{args:?}: {:?} {err}",
            output.status
        );
        assert!(!err.contains("panicked"), "{args:?}: {err}");
        assert!(
            !err.to_lowercase().contains("broken pipe"),
            "{args:?}: {err}"
        );
        assert_eq!(err, "", "{args:?}");
    }
}

#[test]
fn a_closed_pipe_does_not_stop_an_export() {
    // 4000 `wrote` lines, far more than a pipe holds.
    let fixture = big_fixture();
    let output = hang_up_early(&fixture, &["export", "--out", "out"]);
    assert!(output.status.success(), "{:?}", output.status);
    assert_eq!(stderr(&output), "");
    assert_eq!(exported_files(&fixture.home().join("out")).len(), 4000);
}

#[test]
fn stored_ids_cannot_move_export_files_out_of_the_directory() {
    let fixture = Fixture::new();
    let outside = fixture.home().join("outside");
    fs::create_dir(&outside).unwrap();
    let absolute = outside.join("absolute").to_str().unwrap().to_string();
    let ids = [
        "../outside/relative",
        r"..\outside\backslash",
        absolute.as_str(),
        "nested/dir/id",
        "nul",
        ".hidden",
        "ok-id",
    ];
    let mut rows: Vec<_> = ids
        .iter()
        .enumerate()
        .map(|(n, id)| {
            composer(
                &format!("row{n}"),
                &composer_json(id, "Crafted", 1_757_000_000_000, 1_757_000_000_000, &[]),
                Stored::Text,
            )
        })
        .collect();
    // A blank composerId falls back to the key, so `show` can find it.
    rows.push(composer(
        "from-key",
        &serde_json::json!({"composerId": "", "name": "Blank"}),
        Stored::Text,
    ));
    fixture.write_ide_db(Journal::Delete, &rows);

    let out_dir = fixture.home().join("a").join("out");
    let out = ok(
        &fixture,
        &[
            "export",
            "--format",
            "json",
            "--out",
            out_dir.to_str().unwrap(),
        ],
    );
    assert_eq!(out.lines().count(), ids.len() + 1);
    assert_eq!(fs::read_dir(&outside).unwrap().count(), 0);
    assert_eq!(fs::read_dir(fixture.home().join("a")).unwrap().count(), 1);
    let names = exported_files(&out_dir);
    assert_eq!(names.len(), ids.len() + 1);
    assert!(names.contains(&"ok-id.json".to_string()));
    assert!(names.contains(&"from-key.json".to_string()));
    for name in &names {
        assert!(!name.starts_with('.'), "{name}");
        let exported: Session =
            serde_json::from_str(&fs::read_to_string(out_dir.join(name)).unwrap()).unwrap();
        assert!(
            ids.contains(&exported.id.as_str()) || exported.id == "from-key",
            "{name}"
        );
    }
    assert_eq!(
        json(&ok(&fixture, &["show", "from-key", "--json"]))["title"],
        "Blank"
    );
}

#[test]
fn healthcheck_passes_when_the_stores_load() {
    let fixture = standard();
    let out = ok(&fixture, &["healthcheck"]);
    let expected = format!(
        "Cursor session healthcheck\n\n\
         agent chats: {} (ok)\n\
         transcripts: {}/{{project}}/agent-transcripts (ok)\n\
         ide db: {} (ok)\n\n\
         sessions loaded: 7 (agent: 4, ide: 3)\n",
        fixture.chats_dir().display(),
        fixture.projects_dir().display(),
        fixture.ide_db_path().display()
    );
    assert_eq!(out, expected);

    // One store is enough.
    let agent_only = Fixture::new();
    write_standard_agent(&agent_only);
    let out = ok(&agent_only, &["healthcheck"]);
    assert!(out.contains("ide db: not found (state.vscdb)\n"));
    assert!(out.ends_with("sessions loaded: 4 (agent: 4, ide: 0)\n"));
}

#[test]
fn healthcheck_fails_without_any_store() {
    let fixture = Fixture::new();
    let output = run(&fixture, &["healthcheck"]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        stdout(&output),
        "Cursor session healthcheck\n\n\
         agent chats: not found (~/.cursor/chats)\n\
         transcripts: not found (~/.cursor/projects)\n\
         ide db: not found (state.vscdb)\n\n\
         sessions loaded: 0 (agent: 0, ide: 0)\n\
         warning: no sessions were parsed\n"
    );
    assert_eq!(
        stderr(&output),
        "error: no Cursor session storage found\n\
         pass --storage <path> if your Cursor data lives elsewhere\n"
    );
}

#[test]
fn healthcheck_fails_when_a_store_is_broken() {
    let fixture = Fixture::new();
    write_standard_agent(&fixture);
    write_sql_db(
        &fixture.ide_db_path(),
        "CREATE TABLE ItemTable (key TEXT, value BLOB);",
    );
    let output = run(&fixture, &["healthcheck"]);
    assert_eq!(output.status.code(), Some(1));
    let out = stdout(&output);
    assert!(out.contains(&format!(
        "ide db: {} (failed)\n",
        fixture.ide_db_path().display()
    )));
    assert!(out.contains("agent-transcripts (ok)\n"));
    assert!(out.contains("sessions loaded: 4 (agent: 4, ide: 0)\n"));
    assert!(out.contains("ide store failed: unrecognized Cursor IDE storage format in "));
    assert_eq!(
        stderr(&output),
        "error: healthcheck failed: the ide store could not be loaded\n"
    );
}

fn exported_files(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

#[test]
fn export_writes_one_file_per_session_in_every_format() {
    let fixture = standard();
    let sessions = fixture.load().sessions;
    for format in ["md", "json", "jsonl", "yaml"] {
        let dir = fixture.home().join("exports").join(format);
        let out = ok(
            &fixture,
            &["export", "--format", format, "--out", dir.to_str().unwrap()],
        );
        let mut expected: Vec<String> = STANDARD_IDS
            .iter()
            .map(|id| format!("{id}.{format}"))
            .collect();
        expected.sort();
        assert_eq!(exported_files(&dir), expected);
        assert_eq!(out.lines().count(), STANDARD_IDS.len());
        for line in out.lines() {
            assert!(
                line.starts_with("wrote ") && line.ends_with(&format!(".{format}")),
                "{line}"
            );
        }

        for session in &sessions {
            let text = fs::read_to_string(dir.join(format!("{}.{format}", session.id))).unwrap();
            let expected = serde_json::to_value(session).unwrap();
            match format {
                "md" => assert!(text.starts_with(&format!("# {}\n", session.title))),
                "json" => {
                    let parsed: Session = serde_json::from_str(&text).unwrap();
                    assert_eq!(serde_json::to_value(parsed).unwrap(), expected);
                }
                "jsonl" => {
                    let messages: Vec<Message> = text
                        .lines()
                        .map(|line| serde_json::from_str(line).unwrap())
                        .collect();
                    assert_eq!(
                        serde_json::to_value(messages).unwrap(),
                        expected["messages"]
                    );
                }
                _ => {
                    let parsed: Session = serde_yaml::from_str(&text).unwrap();
                    assert_eq!(serde_json::to_value(parsed).unwrap(), expected);
                }
            }
        }
    }
}

#[test]
fn export_selects_by_id_workspace_and_source() {
    let fixture = standard();
    let out = ok(
        &fixture,
        &[
            "export",
            "--session-id",
            "F4EE",
            "--format",
            "json",
            "--out",
            "one",
        ],
    );
    assert_eq!(out.lines().count(), 1);
    assert_eq!(
        exported_files(&fixture.home().join("one")),
        [format!("{AGENT_ID}.json")]
    );

    // The default directory is ./exports, here the fixture home.
    ok(&fixture, &["export", "--workspace", PROJECT_X]);
    assert_eq!(
        exported_files(&fixture.home().join("exports")),
        [format!("{AGENT_ID}.md")]
    );

    ok(
        &fixture,
        &[
            "export", "--source", "ide", "--format", "yaml", "--out", "ide",
        ],
    );
    assert_eq!(exported_files(&fixture.home().join("ide")).len(), 4);

    assert_eq!(
        fails(
            &fixture,
            &["export", "--workspace", "/nowhere", "--out", "none"]
        ),
        "error: no sessions matched\n"
    );
    assert!(!fixture.home().join("none").exists());
}

#[test]
fn verbose_shows_only_paths_inside_the_fixture() {
    let fixture = standard();
    let output = run(&fixture, &["-v", "list", "--json"]);
    assert!(output.status.success());
    assert_eq!(
        stderr(&output),
        format!(
            "chats: {}\nprojects: {}\nide db: {}\n",
            fixture.chats_dir().display(),
            fixture.projects_dir().display(),
            fixture.ide_db_path().display()
        )
    );

    let empty = Fixture::new();
    let output = run(&empty, &["list", "--verbose"]);
    assert_eq!(
        stderr(&output),
        "chats: not found\nprojects: not found\nide db: not found\n"
    );
}

#[test]
fn ide_db_is_found_through_xdg_config_home_or_appdata() {
    // Linux and Windows look only where the variable points; macOS ignores it.
    let fixture = Fixture::with_config_dir("custom-config");
    fixture.write_ide_db(Journal::Delete, &standard_ide_rows());
    let list = json(&ok(&fixture, &["list", "--json"]));
    assert_eq!(
        ids(&list),
        [SHARED_ID, IDE_BLOB_ID, IDE_UNTITLED_ID, IDE_TEXT_ID]
    );
}

#[test]
fn storage_reads_only_the_given_location() {
    let fixture = standard();
    // HOME points at an empty home; --storage alone decides what is read.
    let empty = Fixture::new();
    let listed = |storage: &Path| {
        let output = empty
            .cmd()
            .args(["list", "--json", "--storage"])
            .arg(storage)
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", stderr(&output));
        json(&stdout(&output))
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["id"].as_str().unwrap().to_string())
            .collect::<Vec<_>>()
    };
    assert_eq!(listed(fixture.home()), STANDARD_IDS);
    assert_eq!(
        listed(&fixture.ide_db_path()),
        [SHARED_ID, IDE_BLOB_ID, IDE_UNTITLED_ID, IDE_TEXT_ID]
    );
    assert_eq!(
        listed(&fixture.home().join(".cursor")),
        [SHARED_ID, STORE_ONLY_ID, AGENT_ID, TRANSCRIPT_ONLY_ID]
    );

    let missing = fixture.home().join("missing");
    empty
        .cmd()
        .args(["list", "--storage"])
        .arg(&missing)
        .assert()
        .code(1)
        .stdout("")
        .stderr(predicate::str::starts_with(format!(
            "error: storage path does not exist: {}\n  caused by: ",
            missing.display()
        )));
}

/// The binary in a pseudo-terminal, through script(1). Windows has no
/// script(1), and ConPTY would need a new dependency, so these run on Unix.
#[cfg(unix)]
mod tty {
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    use super::*;

    const WIDTH: usize = 100;
    const TTY_TIMEOUT: Duration = Duration::from_secs(60);

    fn quote(arg: &str) -> String {
        format!("'{}'", arg.replace('\'', r"'\''"))
    }

    /// Runs the binary in a terminal `WIDTH` columns wide and returns what it
    /// printed, with the terminal's CRLF line ends turned back into LF.
    fn run_tty(fixture: &Fixture, args: &[&str], env: &[(&str, &str)]) -> String {
        let inner = fixture.home().join("tty.sh");
        let command: Vec<String> = std::iter::once(BIN)
            .chain(args.iter().copied())
            .map(quote)
            .collect();
        fs::write(
            &inner,
            format!(
                "stty cols {WIDTH} 2>/dev/null\nexec {}\n",
                command.join(" ")
            ),
        )
        .unwrap();
        let mut cmd = fixture.isolated("script");
        if cfg!(target_os = "linux") {
            let inner = format!("/bin/sh {}", quote(inner.to_str().unwrap()));
            cmd.args(["-qec", &inner, "/dev/null"]);
        } else {
            cmd.args(["-q", "/dev/null", "/bin/sh"]).arg(&inner);
        }
        let errors = fixture.home().join("tty.err");
        let mut child = cmd
            .env("SHELL", "/bin/sh")
            .env("COLUMNS", WIDTH.to_string())
            .envs(env.iter().copied())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(fs::File::create(&errors).unwrap())
            .spawn()
            .expect("script(1) runs commands in a pseudo-terminal");
        // BSD script echoes ^D into the output when its input ends, so the
        // input stays open until the command is done.
        let input = child.stdin.take();
        let mut pipe = child.stdout.take().unwrap();
        let (done, finished) = mpsc::channel();
        let reader = thread::spawn(move || {
            let mut out = Vec::new();
            pipe.read_to_end(&mut out).unwrap();
            done.send(()).unwrap();
            out
        });
        // A script(1) that never exits fails the test instead of hanging CI.
        let timed_out = finished.recv_timeout(TTY_TIMEOUT).is_err();
        if timed_out {
            child.kill().unwrap();
        }
        let status = child.wait().unwrap();
        drop(input);
        let out = String::from_utf8(reader.join().unwrap()).unwrap();
        assert!(
            !timed_out,
            "{args:?} still running after {TTY_TIMEOUT:?}: {out:?}"
        );
        assert!(status.success(), "{args:?}: {status}: {out:?}");
        assert_eq!(fs::read_to_string(&errors).unwrap(), "");
        out.replace("\r\n", "\n")
    }

    #[test]
    fn terminal_gets_the_fitted_table_colored_unless_told_otherwise() {
        crossterm::style::force_color_output(true);
        let fixture = standard();
        let sessions = fixture.load().sessions;
        let colored = ui::render_list(&sessions, true, Some(WIDTH));
        let plain = ui::render_list(&sessions, false, Some(WIDTH));
        assert!(colored.contains('\u{1b}') && plain.contains('│'));

        assert_eq!(run_tty(&fixture, &["list"], &[]), colored);
        assert_eq!(run_tty(&fixture, &["list"], &[("NO_COLOR", "")]), colored);
        assert_eq!(run_tty(&fixture, &["list"], &[("NO_COLOR", "1")]), plain);
        assert_eq!(run_tty(&fixture, &["list"], &[("TERM", "dumb")]), plain);
        assert_eq!(run_tty(&fixture, &["list", "--color", "never"], &[]), plain);
        // --color always beats NO_COLOR, also for the table's own styling.
        assert_eq!(
            run_tty(
                &fixture,
                &["list", "--color", "always"],
                &[("NO_COLOR", "1")]
            ),
            colored
        );
        // JSON is never colored or fitted.
        assert_eq!(
            run_tty(&fixture, &["list", "--json"], &[]),
            ok(&fixture, &["list", "--json"])
        );
    }

    #[test]
    fn terminal_show_defaults_to_the_last_twenty_messages() {
        let fixture = long_session_fixture(15);
        let session = fixture.load().sessions.remove(0);
        assert_eq!(session.messages.len(), 30);
        let show =
            |messages: &[Message], hidden| ui::render_show(&session, messages, hidden, false);

        let env = [("NO_COLOR", "1")];
        assert_eq!(
            run_tty(&fixture, &["show", "long"], &env),
            show(&session.messages[10..], Some(10))
        );
        assert_eq!(
            run_tty(&fixture, &["show", "long", "--all"], &env),
            show(&session.messages, None)
        );
        assert_eq!(
            run_tty(&fixture, &["show", "long", "--limit", "3"], &env),
            show(&session.messages[27..], Some(27))
        );
        // JSON holds every message unless --limit asks for fewer.
        let shown = json(&run_tty(&fixture, &["show", "long", "--json"], &env));
        assert_eq!(shown["messages"].as_array().unwrap().len(), 30);
        assert_eq!(shown, json(&ok(&fixture, &["show", "long", "--json"])));
        let shown = json(&run_tty(
            &fixture,
            &["show", "long", "--json", "--limit", "3"],
            &env,
        ));
        assert_eq!(shown["messages"].as_array().unwrap().len(), 3);
    }
}
