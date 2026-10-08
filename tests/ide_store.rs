//! IDE database resilience: odd files, changed schemas, malformed rows and the
//! states a WAL database can be in while Cursor runs, quits or crashes.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::*;
use cursor_session::ide::load_from_db;
use cursor_session::model::{Session, Source};
use cursor_session::{Error, LoadOptions, load_messages, load_sessions};
use rusqlite::Connection;
use rusqlite::types::Value as SqlValue;
use serde_json::json;

/// Loads `path`, which must give no notices, with its warnings.
fn load_db(path: &Path) -> (cursor_session::Result<Vec<Session>>, Vec<String>) {
    let (sessions, warnings, notices) = load_db_noticed(path);
    assert!(notices.is_empty(), "{notices:?}");
    (sessions, warnings)
}

/// Loads `path`, with its warnings and notices.
fn load_db_noticed(
    path: &Path,
) -> (
    cursor_session::Result<Vec<Session>>,
    Vec<String>,
    Vec<String>,
) {
    let (mut warnings, mut notices) = (Vec::new(), Vec::new());
    let sessions = load_from_db(path, &mut warnings, &mut notices)
        .and_then(|summaries| with_messages(&summaries));
    (sessions, warnings, notices)
}

fn sidecar(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

fn listed_ids(fixture: &Fixture, args: &[&str]) -> Vec<String> {
    let output = fixture.cmd().args(args).arg("--json").output().unwrap();
    assert!(output.status.success(), "{}", stderr(&output));
    let list: serde_json::Value = serde_json::from_str(&stdout(&output)).unwrap();
    list.as_array()
        .unwrap()
        .iter()
        .map(|s| s["id"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn empty_kv_table_holds_no_sessions() {
    let fixture = Fixture::new();
    let db = fixture.write_ide_db(Journal::Delete, &[]);
    let (sessions, warnings) = load_db(&db);
    assert!(sessions.unwrap().is_empty());
    assert!(warnings.is_empty());

    fixture
        .cmd()
        .args(["list", "--json"])
        .assert()
        .success()
        .stdout("[]\n");
    fixture
        .cmd()
        .arg("list")
        .assert()
        .success()
        .stdout("No sessions found\n");
}

#[test]
fn zero_byte_database_is_reported_and_skipped() {
    let fixture = Fixture::new();
    write_standard_agent(&fixture);
    let db = fixture.ide_db_path();
    write(&db, "");

    let (sessions, warnings) = load_db(&db);
    assert!(sessions.unwrap().is_empty());
    assert_eq!(
        warnings,
        [format!(
            "{} is empty; no Cursor IDE sessions loaded",
            db.display()
        )]
    );

    let output = fixture.cmd().args(["-v", "list"]).output().unwrap();
    assert!(output.status.success());
    assert!(stdout(&output).starts_with("Found 4 session(s)\n"));
    assert!(
        stderr(&output).ends_with(&format!(
            "warning: {} is empty; no Cursor IDE sessions loaded\n",
            db.display()
        )),
        "{}",
        stderr(&output)
    );
    // Warnings are for --verbose only.
    let output = fixture.cmd().arg("list").output().unwrap();
    assert_eq!(stderr(&output), "");
}

#[test]
fn database_without_tables_is_reported_and_skipped() {
    let fixture = Fixture::new();
    let db = fixture.ide_db_path();
    write_sql_db(&db, "PRAGMA user_version = 7;");
    let (sessions, warnings) = load_db(&db);
    assert!(sessions.unwrap().is_empty());
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].ends_with(" has no tables; no Cursor IDE sessions loaded"));
}

#[test]
fn non_database_file_fails_with_a_database_error() {
    let fixture = Fixture::new();
    write_standard_agent(&fixture);
    let db = fixture.ide_db_path();
    write(
        &db,
        "this is plain text, long enough to fill a SQLite header and then some",
    );

    let err = load_db(&db).0.unwrap_err();
    assert!(matches!(&err, Error::Database { path, .. } if *path == db));

    let output = fixture.cmd().arg("list").output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(stdout(&output), "");
    let err = stderr(&output);
    assert!(
        err.starts_with(&format!(
            "error: could not read SQLite database {}\n  caused by: file is not a database\n",
            db.display()
        )),
        "{err}"
    );
    assert!(!err.contains("panicked"));
    fixture
        .cmd()
        .args(["list", "--source", "agent"])
        .assert()
        .success();
}

/// State databases whose layout is not the one Cursor uses today.
const CHANGED_SCHEMAS: [(&str, &str); 3] = [
    (
        "CREATE TABLE ItemTable (key TEXT UNIQUE ON CONFLICT REPLACE, value BLOB);",
        "table `cursorDiskKV` not found (tables present: ItemTable)",
    ),
    (
        "CREATE TABLE ItemTable (key TEXT, value BLOB);
         CREATE TABLE cursorDiskKV2 (key TEXT, value BLOB);
         INSERT INTO cursorDiskKV2 VALUES ('composerData:x', '{}');",
        "table `cursorDiskKV` not found (tables present: ItemTable, cursorDiskKV2)",
    ),
    (
        "CREATE TABLE cursorDiskKV (key TEXT UNIQUE ON CONFLICT REPLACE, data BLOB);
         INSERT INTO cursorDiskKV VALUES ('composerData:x', '{}');",
        "table `cursorDiskKV` has no `value` column (columns: key, data)",
    ),
];

#[test]
fn changed_schema_is_a_clear_error() {
    for (sql, detail) in CHANGED_SCHEMAS {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("state.vscdb");
        write_sql_db(&db, sql);
        let err = load_db(&db).0.unwrap_err();
        assert!(
            matches!(err, Error::SchemaMismatch { .. }),
            "{detail}: {err:?}"
        );
        assert_eq!(
            err.to_string(),
            format!(
                "unrecognized Cursor IDE storage format in {}: {detail}. Cursor may have \
                 changed its storage format.",
                db.display()
            )
        );
        assert_eq!(
            err.hints()[0],
            "rerun with `--source agent` to skip IDE sessions"
        );
    }
}

#[test]
fn changed_schema_fails_the_cli_cleanly_unless_only_agent_sessions_are_asked_for() {
    for (sql, detail) in CHANGED_SCHEMAS {
        let fixture = Fixture::new();
        write_standard_agent(&fixture);
        let db = fixture.ide_db_path();
        write_sql_db(&db, sql);

        for args in [
            &["list"][..],
            &["list", "--json"],
            &["show", AGENT_ID],
            &["export", "--out", "exports"],
        ] {
            let output = fixture.cmd().args(args).output().unwrap();
            let err = stderr(&output);
            assert_eq!(output.status.code(), Some(1), "{args:?}: {err}");
            assert_eq!(stdout(&output), "", "{args:?}");
            assert_eq!(
                err,
                format!(
                    "error: unrecognized Cursor IDE storage format in {}: {detail}. Cursor may \
                     have changed its storage format.\n\
                     rerun with `--source agent` to skip IDE sessions\n\
                     report it at https://github.com/julio-romero/cursor-session-rs/issues and \
                     include your Cursor version\n",
                    db.display()
                ),
                "{args:?}"
            );
            assert!(!err.contains("panicked"));
        }
        assert!(!fixture.home().join("exports").exists());

        assert_eq!(
            listed_ids(&fixture, &["list", "--source", "agent"]).len(),
            4
        );
        fixture
            .cmd()
            .args(["show", AGENT_ID, "--source", "agent"])
            .assert()
            .success();
    }
}

/// Rows a Cursor update could change the format of, with how loading them
/// reports it. The table and its columns are as expected.
fn changed_row_formats() -> Vec<(Vec<(String, SqlValue)>, &'static str)> {
    let encoded = |key: &str| {
        (
            key.to_string(),
            SqlValue::Blob(vec![0x78, 0x9c, 0xcb, 0x48]),
        )
    };
    vec![
        (
            vec![encoded("composerData:z1")],
            "its composerData row could not be read",
        ),
        (
            vec![
                encoded("composerData:z1"),
                encoded("composerData:z2"),
                bubble("z1", "b1", &text_bubble("b1", 1, "hello"), Stored::Text),
            ],
            "none of its 2 composerData rows could be read",
        ),
        // The conversation is listed under a key this version does not know.
        (
            vec![
                composer(
                    "r1",
                    &json!({"composerId": "r1", "conversationHeaders": [{"bubbleId": "b1"}]}),
                    Stored::Text,
                ),
                bubble("r1", "b1", &text_bubble("b1", 1, "hello"), Stored::Text),
                composer(
                    "r2",
                    &json!({"composerId": "r2", "name": "New and empty"}),
                    Stored::Text,
                ),
            ],
            "no chat lists the messages stored for it",
        ),
        (
            vec![
                composer(
                    "m1",
                    &composer_json("m1", "Encoded", 1, 1, &[("b1", 1), ("b2", 2)]),
                    Stored::Text,
                ),
                encoded("bubbleId:m1:b1"),
                encoded("bubbleId:m1:b2"),
            ],
            "none of its 2 bubbleId rows could be read",
        ),
        // The messages are kept under a key this version does not know.
        (
            vec![
                composer(
                    "k1",
                    &composer_json("k1", "Moved", 1, 1, &[("b1", 1)]),
                    Stored::Text,
                ),
                (
                    "messageV2:k1:b1".to_string(),
                    Stored::Text.value(&text_bubble("b1", 1, "hello")),
                ),
            ],
            "its one chat that lists messages has no readable message",
        ),
        // The text is kept in a field this version does not know.
        (
            ["t1", "t2"]
                .into_iter()
                .flat_map(|id| {
                    [
                        composer(
                            id,
                            &composer_json(id, "Renamed", 1, 1, &[("b1", 1), ("b2", 2)]),
                            Stored::Text,
                        ),
                        bubble(id, "b1", &json!({"type": 1, "content": "hi"}), Stored::Text),
                        bubble(
                            id,
                            "b2",
                            &json!({"type": 2, "content": "hello"}),
                            Stored::Text,
                        ),
                    ]
                })
                .collect(),
            "none of its 2 chats that list messages has a readable message",
        ),
        // The text is kept in a field this version does not know, next to
        // tool calls that are never shown.
        (
            vec![
                composer(
                    "u1",
                    &composer_json("u1", "Renamed", 1, 1, &[("b1", 1), ("b2", 2)]),
                    Stored::Text,
                ),
                bubble(
                    "u1",
                    "b1",
                    &json!({"type": 1, "content": "hi"}),
                    Stored::Text,
                ),
                bubble(
                    "u1",
                    "b2",
                    &json!({"type": 2, "toolFormerData": {"name": "read_file"}}),
                    Stored::Text,
                ),
            ],
            "its one chat that lists messages has no readable message",
        ),
        // The chats are kept under a key this version does not know.
        (
            ["p1", "p2"]
                .into_iter()
                .flat_map(|id| {
                    [
                        (
                            format!("composer:{id}"),
                            Stored::Text.value(&composer_json(id, "Moved", 1, 1, &[("b1", 1)])),
                        ),
                        bubble(id, "b1", &text_bubble("b1", 1, "hello"), Stored::Text),
                    ]
                })
                .collect(),
            "none of its 2 bubbleId rows belongs to a composerData row, and `composer:` rows \
             name their chats",
        ),
        // Who wrote a message is kept in a field this version does not know.
        (
            vec![
                composer(
                    "y1",
                    &json!({"composerId": "y1", "fullConversationHeadersOnly": [
                        {"bubbleId": "b1", "role": 1},
                        {"bubbleId": "b2", "role": 2},
                    ]}),
                    Stored::Text,
                ),
                bubble("y1", "b1", &json!({"role": 1, "text": "hi"}), Stored::Text),
                bubble(
                    "y1",
                    "b2",
                    &json!({"role": 2, "text": "hello"}),
                    Stored::Text,
                ),
            ],
            "none of its 2 messages has a known type",
        ),
    ]
}

#[test]
fn changed_row_formats_are_a_clear_error() {
    for (rows, detail) in changed_row_formats() {
        let fixture = Fixture::new();
        write_standard_agent(&fixture);
        let db = fixture.write_ide_db(Journal::Delete, &rows);
        let err = load_db(&db).0.unwrap_err();
        assert_eq!(
            err.to_string(),
            format!(
                "unrecognized Cursor IDE storage format in {}: {detail}. Cursor may have \
                 changed its storage format.",
                db.display()
            )
        );
        let output = fixture.cmd().arg("list").output().unwrap();
        assert_eq!(output.status.code(), Some(1), "{detail}");
        assert!(stderr(&output).contains("\nrerun with `--source agent` to skip IDE sessions\n"));
        assert_eq!(
            listed_ids(&fixture, &["list", "--source", "agent"]).len(),
            4
        );
    }
}

#[test]
fn odd_field_types_cost_only_those_fields() {
    let fixture = Fixture::new();
    let chat = |id: &str, fields: serde_json::Value| {
        let mut json = composer_json(id, id, 1_757_000_000_000, 1_757_000_000_000, &[("b1", 1)]);
        for (key, value) in fields.as_object().unwrap() {
            json[key] = value.clone();
        }
        [
            composer(id, &json, Stored::Text),
            bubble(id, "b1", &text_bubble("b1", 1, "question"), Stored::Text),
        ]
    };
    let mut rows: Vec<_> = [
        chat("null-headers", json!({"fullConversationHeadersOnly": null})),
        chat("fractional", json!({"createdAt": 1_757_000_000_000.5})),
        chat("iso-time", json!({"lastUpdatedAt": "2025-09-04T15:33:20Z"})),
        chat("numeric-name", json!({"name": 42})),
    ]
    .into_iter()
    .flatten()
    .collect();
    rows.extend([
        composer(
            "odd-bubbles",
            &composer_json("odd-bubbles", "Odd bubbles", 1, 1, &[("b1", 1), ("b2", 2)]),
            Stored::Text,
        ),
        bubble(
            "odd-bubbles",
            "b1",
            &json!({"type": 1, "text": "when?", "timestamp": "2023-11-14T22:13:20.000Z"}),
            Stored::Text,
        ),
        bubble(
            "odd-bubbles",
            "b2",
            &json!({"type": 2, "text": "now", "codeBlocks": null, "bubbleId": 7}),
            Stored::Text,
        ),
    ]);
    let db = fixture.write_ide_db(Journal::Delete, &rows);
    let (sessions, warnings) = load_db(&db);
    // Without its headers, that chat no longer lists its stored message.
    assert_eq!(
        warnings,
        [format!(
            "1 chat(s) in {} list none of their stored messages",
            db.display()
        )]
    );
    let mut sessions = sessions.unwrap();
    sessions.sort_by(|a, b| a.id.cmp(&b.id));
    let summary: Vec<(&str, &str, usize)> = sessions
        .iter()
        .map(|s| (s.id.as_str(), s.title.as_str(), s.messages.len()))
        .collect();
    assert_eq!(
        summary,
        [
            ("fractional", "fractional", 1),
            ("iso-time", "iso-time", 1),
            ("null-headers", "null-headers", 0),
            ("numeric-name", "Untitled", 1),
            ("odd-bubbles", "Odd bubbles", 2),
        ]
    );
    assert_eq!(sessions[0].created_at_ms, Some(1_757_000_000_000));
    assert_eq!(sessions[1].updated_at_ms, Some(1_757_000_000_000));
    assert_eq!(
        sessions[4].messages[0].timestamp.as_deref(),
        Some("2023-11-14T22:13:20.000Z")
    );
}

#[test]
fn message_types_may_be_named_and_unknown_ones_are_shown_as_unknown() {
    let fixture = Fixture::new();
    let chat = |id: &str, kinds: [serde_json::Value; 2]| {
        let headers: Vec<_> = ["b1", "b2"]
            .into_iter()
            .map(|bubble_id| json!({"bubbleId": bubble_id}))
            .collect();
        let json = json!({"composerId": id, "fullConversationHeadersOnly": headers});
        let [first, second] = kinds;
        [
            composer(id, &json, Stored::Text),
            bubble(
                id,
                "b1",
                &json!({"type": first, "text": "question"}),
                Stored::Text,
            ),
            bubble(
                id,
                "b2",
                &json!({"type": second, "text": "answer"}),
                Stored::Text,
            ),
        ]
    };
    // Older chats keep their types; a newer one has types this version does
    // not know, as a change of format for new chats would leave them.
    let rows: Vec<_> = [
        chat("named", [json!("user"), json!("assistant")]),
        chat("numbered", [json!(1), json!(2)]),
        chat("unknown", [json!("human"), json!(3)]),
    ]
    .into_iter()
    .flatten()
    .collect();
    let db = fixture.write_ide_db(Journal::Delete, &rows);
    let (sessions, warnings, notices) = load_db_noticed(&db);
    assert!(warnings.is_empty(), "{warnings:?}");
    let notice = format!(
        "2 message(s) in {} have a type this version does not know and are shown as \
         `unknown`. Cursor may have changed its storage format.",
        db.display()
    );
    assert_eq!(notices, std::slice::from_ref(&notice));
    let mut sessions = sessions.unwrap();
    sessions.sort_by(|a, b| a.id.cmp(&b.id));
    let roles: Vec<Vec<&str>> = sessions
        .iter()
        .map(|s| s.messages.iter().map(|m| m.role.as_str()).collect())
        .collect();
    assert_eq!(
        roles,
        [
            ["user", "assistant"],
            ["user", "assistant"],
            ["unknown", "unknown"]
        ]
    );

    // No guess at who wrote them, and a notice even without -v.
    let output = fixture.cmd().args(["show", "unknown"]).output().unwrap();
    assert!(output.status.success());
    assert_eq!(stderr(&output), format!("warning: {notice}\n"));
    let shown = stdout(&output);
    assert!(
        shown.contains("[unknown]\nquestion\n") && shown.contains("[unknown]\nanswer\n"),
        "{shown}"
    );
    let healthcheck = fixture.cmd().arg("healthcheck").output().unwrap();
    assert!(healthcheck.status.success());
    assert!(
        stdout(&healthcheck).contains(&format!("ide db: {} (incomplete)\n", db.display())),
        "{}",
        stdout(&healthcheck)
    );
    assert_eq!(stderr(&healthcheck), format!("warning: {notice}\n"));
}

/// A chat of `n` messages whose row is kept under `prefix`.
fn chat_under(prefix: &str, id: &str, n: usize) -> Vec<(String, SqlValue)> {
    let ids: Vec<String> = (0..n).map(|i| format!("b{i}")).collect();
    let headers: Vec<(&str, i64)> = ids
        .iter()
        .zip([1, 2].into_iter().cycle())
        .map(|(id, kind)| (id.as_str(), kind))
        .collect();
    let mut rows = vec![(
        format!("{prefix}{id}"),
        Stored::Text.value(&composer_json(id, id, 1, 1, &headers)),
    )];
    for (bubble_id, kind) in headers {
        rows.push(bubble(
            id,
            bubble_id,
            &text_bubble(bubble_id, kind, &format!("{id} {bubble_id}")),
            Stored::Text,
        ));
    }
    rows
}

#[test]
fn chats_whose_rows_moved_are_a_notice_and_deleted_ones_a_warning() {
    let fixture = Fixture::new();
    // The older chat keeps its row; two newer ones have theirs under a new key.
    let mut rows = chat_under("composerData:", "old", 2);
    rows.extend(chat_under("composerDataV2:", "new1", 4));
    rows.extend(chat_under("composerDataV2:", "new2", 6));
    // A deleted chat leaves its messages, and maybe rows that name it within
    // their key, behind.
    let deleted = chat_under("deleted:", "gone", 2);
    rows.extend(deleted[1..].iter().cloned());
    rows.push((
        "checkpointId:gone:k1".to_string(),
        SqlValue::Text("{}".into()),
    ));
    // A chat whose row cannot be read is reported as that alone.
    rows.push((
        "composerData:broken".to_string(),
        SqlValue::Text("{".into()),
    ));
    rows.extend(chat_under("unused:", "broken", 1)[1..].iter().cloned());
    let db = fixture.write_ide_db(Journal::Delete, &rows);

    let (sessions, warnings, notices) = load_db_noticed(&db);
    let ids: Vec<String> = sessions
        .unwrap()
        .into_iter()
        .map(|s| s.summary.id)
        .collect();
    assert_eq!(ids, ["old"]);
    assert_eq!(
        warnings,
        [
            format!("skipped 1 unreadable composer row in {}", db.display()),
            format!(
                "skipped 2 message row(s) of 1 chat(s) in {} that have no composerData row",
                db.display()
            ),
        ]
    );
    let notice = format!(
        "left out 2 chat(s) with 10 message row(s) in {}: they have no composerData row, but \
         `composerDataV2:` rows name them. Cursor may have changed its storage format.",
        db.display()
    );
    assert_eq!(notices, std::slice::from_ref(&notice));

    // The notice prints without -v; the chats that load still list.
    let output = fixture.cmd().arg("list").output().unwrap();
    assert!(output.status.success());
    assert_eq!(stderr(&output), format!("warning: {notice}\n"));
    assert!(stdout(&output).starts_with("Found 1 session(s)\n"));
    let healthcheck = fixture.cmd().arg("healthcheck").output().unwrap();
    assert!(healthcheck.status.success());
    assert!(
        stdout(&healthcheck).contains(&format!("ide db: {} (incomplete)\n", db.display())),
        "{}",
        stdout(&healthcheck)
    );

    // With every chat deleted, what is left is no sign of a new format.
    let fixture = Fixture::new();
    let db = fixture.write_ide_db(Journal::Delete, &deleted[1..]);
    let (sessions, warnings) = load_db(&db);
    assert!(sessions.unwrap().is_empty());
    assert_eq!(
        warnings,
        [format!(
            "skipped 2 message row(s) of 1 chat(s) in {} that have no composerData row",
            db.display()
        )]
    );
    fixture
        .cmd()
        .arg("list")
        .assert()
        .success()
        .stdout("No sessions found\n")
        .stderr("");
}

#[test]
fn chat_and_message_rows_under_new_keys_are_a_notice() {
    let fixture = Fixture::new();
    let other = |key: &str| (key.to_string(), SqlValue::Text("{}".into()));
    let db = fixture.write_ide_db(
        Journal::Delete,
        &[
            other("composerV2:x"),
            other("bubbleV2:x:b0"),
            other("checkpointId:x:1"),
        ],
    );
    let (sessions, warnings, notices) = load_db_noticed(&db);
    assert!(sessions.unwrap().is_empty());
    assert!(warnings.is_empty(), "{warnings:?}");
    let notice = format!(
        "{} has no composerData or bubbleId rows, but rows under `bubbleV2:`, `composerV2:`, \
         which are left out. Cursor may have changed its storage format.",
        db.display()
    );
    assert_eq!(notices, std::slice::from_ref(&notice));
    let output = fixture.cmd().arg("list").output().unwrap();
    assert!(output.status.success());
    assert_eq!(stdout(&output), "No sessions found\n");
    assert_eq!(stderr(&output), format!("warning: {notice}\n"));
    let healthcheck = fixture.cmd().arg("healthcheck").output().unwrap();
    assert!(
        stdout(&healthcheck).contains(&format!("ide db: {} (incomplete)\n", db.display())),
        "{}",
        stdout(&healthcheck)
    );

    // Rows of other kinds alone may well be a store without chats.
    let fixture = Fixture::new();
    let db = fixture.write_ide_db(Journal::Delete, &[other("checkpointId:x:1")]);
    let (sessions, warnings) = load_db(&db);
    assert!(sessions.unwrap().is_empty());
    assert_eq!(
        warnings,
        [format!(
            "{} has no composerData or bubbleId rows, only rows under `checkpointId:`",
            db.display()
        )]
    );
}

#[test]
fn chats_of_only_tool_calls_and_images_load_without_messages() {
    let fixture = Fixture::new();
    let tool = |name: &str| json!({"type": 2, "text": "", "toolFormerData": {"name": name}});
    let rows = vec![
        composer(
            "tools",
            &composer_json("tools", "Tools", 1, 1, &[("b1", 1), ("b2", 2), ("b3", 2)]),
            Stored::Text,
        ),
        bubble(
            "tools",
            "b1",
            &json!({"type": 1, "text": "", "images": [{"path": "screenshot.png"}]}),
            Stored::Text,
        ),
        bubble("tools", "b2", &tool("run_terminal_cmd"), Stored::Text),
        bubble("tools", "b3", &tool("read_file"), Stored::Text),
    ];
    let db = fixture.write_ide_db(Journal::Delete, &rows);
    let (sessions, warnings) = load_db(&db);
    assert!(warnings.is_empty(), "{warnings:?}");
    let sessions = sessions.unwrap();
    assert_eq!(sessions.len(), 1);
    assert!(sessions[0].messages.is_empty());
}

#[test]
fn keys_stored_as_blobs_are_read() {
    let fixture = Fixture::new();
    let db = fixture.ide_db_path();
    let conn = create_kv_db(&db, Journal::Delete);
    for (key, value) in chat_under("composerData:", "blob-keys", 2) {
        conn.execute(
            "INSERT INTO cursorDiskKV (key, value) VALUES (?1, ?2)",
            rusqlite::params![SqlValue::Blob(key.into_bytes()), value],
        )
        .unwrap();
    }
    drop(conn);
    let (sessions, warnings) = load_db(&db);
    assert!(warnings.is_empty(), "{warnings:?}");
    let sessions = sessions.unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].messages.len(), 2);
}

#[test]
fn a_chat_that_lists_none_of_its_messages_is_reported() {
    let fixture = Fixture::new();
    let mut rows = standard_ide_rows();
    rows.extend([
        composer(
            "r1",
            &json!({"composerId": "r1", "conversationHeaders": [{"bubbleId": "b1"}]}),
            Stored::Text,
        ),
        bubble("r1", "b1", &text_bubble("b1", 1, "hello"), Stored::Text),
    ]);
    let db = fixture.write_ide_db(Journal::Delete, &rows);
    let (sessions, warnings) = load_db(&db);
    assert_eq!(sessions.unwrap().len(), 5);
    assert_eq!(
        warnings,
        [format!(
            "1 chat(s) in {} list none of their stored messages",
            db.display()
        )]
    );
}

#[test]
fn messages_kept_inside_older_composers_are_read() {
    let fixture = Fixture::new();
    let legacy = json!({
        "_v": 2,
        "composerId": "legacy",
        "name": "From an older Cursor",
        "createdAt": 1_700_000_000_000_i64,
        "conversation": [
            {"bubbleId": "a", "type": 1, "text": "old question", "timestamp": 1_700_000_000_000_i64},
            {"bubbleId": "b", "type": 2, "text": "old answer", "codeBlocks": [{"language": "sh", "content": "ls"}]},
            {"bubbleId": "c", "type": "unknown"},
            {"bubbleId": "d", "type": 2, "text": ""},
        ],
    });
    let db = fixture.write_ide_db(
        Journal::Delete,
        &[composer("legacy", &legacy, Stored::Text)],
    );
    let (sessions, warnings) = load_db(&db);
    let sessions = sessions.unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    let messages: Vec<(&str, &str)> = sessions[0]
        .messages
        .iter()
        .map(|m| (m.role.as_str(), m.content.as_str()))
        .collect();
    assert_eq!(
        messages,
        [
            ("user", "old question"),
            ("assistant", "old answer\n\n```sh\nls\n```")
        ]
    );
    assert_eq!(
        sessions[0].messages[0].timestamp.as_deref(),
        Some("1700000000000")
    );
}

#[test]
fn chats_that_reuse_a_message_id_keep_their_own_text() {
    let fixture = Fixture::new();
    let rows: Vec<_> = [("aaaa", "first chat"), ("bbbb", "second chat")]
        .into_iter()
        .enumerate()
        .flat_map(|(n, (id, text))| {
            let at = 1_757_000_000_000 + i64::try_from(n).unwrap();
            [
                composer(
                    id,
                    &composer_json(id, text, at, at, &[("b1", 1)]),
                    Stored::Text,
                ),
                bubble(id, "b1", &text_bubble("b1", 1, text), Stored::Text),
            ]
        })
        .collect();
    let db = fixture.write_ide_db(Journal::Delete, &rows);
    for session in load_db(&db).0.unwrap() {
        assert_eq!(session.messages.len(), 1);
        assert_eq!(session.messages[0].content, session.title);
    }
}

fn malformed_rows() -> Vec<(String, SqlValue)> {
    let mut rows = vec![
        composer(
            "ok",
            &composer_json(
                "ok",
                "Still loads",
                1_757_000_000_000,
                1_757_000_000_000,
                &[
                    ("good", 1),
                    ("bad", 2),
                    ("wrong-types", 2),
                    ("empty", 2),
                    ("null", 2),
                    ("gone", 2),
                    ("reply", 2),
                ],
            ),
            Stored::Blob,
        ),
        bubble(
            "ok",
            "good",
            &text_bubble("good", 1, "question"),
            Stored::Text,
        ),
        bubble(
            "ok",
            "reply",
            &text_bubble("reply", 2, "answer"),
            Stored::Blob,
        ),
        // Loads, but has nothing to show.
        bubble("ok", "empty", &text_bubble("empty", 2, ""), Stored::Text),
        composer(
            "dangling",
            &composer_json("dangling", "Bubbles are gone", 1, 1, &[("x", 1), ("y", 2)]),
            Stored::Text,
        ),
        // The key names the composer when the value does not.
        composer("keyed", &json!({"name": "Named by its key"}), Stored::Text),
    ];
    let raw = |key: &str, value: SqlValue| (key.to_string(), value);
    rows.extend([
        raw("composerData:bad-json", SqlValue::Text("{not json".into())),
        raw("composerData:array", SqlValue::Text("[]".into())),
        raw(
            "composerData:positional",
            SqlValue::Text(r#"["positional","Array fields by position"]"#.into()),
        ),
        raw(
            "composerData:string",
            SqlValue::Text(r#""composer""#.into()),
        ),
        raw(
            "composerData:wrong-types",
            SqlValue::Text(r#"{"composerId":"wrong-types","name":42}"#.into()),
        ),
        raw(
            "composerData:bad-headers",
            SqlValue::Text(r#"{"fullConversationHeadersOnly":"b1,b2"}"#.into()),
        ),
        raw(
            "composerData:bad-utf8",
            SqlValue::Blob(vec![b'{', 0xff, 0xfe, b'}']),
        ),
        raw("composerData:number", SqlValue::Integer(7)),
        raw("composerData:null", SqlValue::Null),
        raw("bubbleId:ok:bad", SqlValue::Text("{".into())),
        raw(
            "bubbleId:ok:wrong-types",
            SqlValue::Text(r#"{"bubbleId":"wrong-types","text":["a"]}"#.into()),
        ),
        raw(
            "bubbleId:ok:array",
            SqlValue::Text(r#"["array","text"]"#.into()),
        ),
        raw("bubbleId:ok:null", SqlValue::Null),
    ]);
    rows
}

#[test]
fn malformed_rows_are_skipped_and_reported() {
    let fixture = Fixture::new();
    let db = fixture.write_ide_db(Journal::Delete, &malformed_rows());

    let (sessions, warnings) = load_db(&db);
    let mut sessions = sessions.unwrap();
    sessions.sort_by(|a, b| a.id.cmp(&b.id));
    let summary: Vec<(&str, &str, usize)> = sessions
        .iter()
        .map(|s| (s.id.as_str(), s.title.as_str(), s.messages.len()))
        .collect();
    // Fields of an unexpected type are left out, not their chat.
    assert_eq!(
        summary,
        [
            ("bad-headers", "Untitled", 0),
            ("dangling", "Bubbles are gone", 0),
            ("keyed", "Named by its key", 0),
            ("ok", "Still loads", 2),
            ("wrong-types", "Untitled", 0),
        ]
    );
    let ok = sessions.iter().find(|s| s.id == "ok").unwrap();
    let contents: Vec<&str> = ok.messages.iter().map(|m| m.content.as_str()).collect();
    assert_eq!(contents, ["question", "answer"]);
    // NULL values are absent rows, not malformed ones: the loader filters them
    // out by design, as it does for a NULL store.db meta value, so they never warn.
    assert_eq!(
        warnings,
        [
            format!("skipped 2 unreadable message rows in {}", db.display()),
            format!("skipped 6 unreadable composer rows in {}", db.display()),
        ]
    );
    assert_eq!(fixture.load().warnings, warnings);
}

#[test]
fn malformed_rows_are_warnings_on_the_command_line() {
    let fixture = Fixture::new();
    let db = fixture.write_ide_db(Journal::Delete, &malformed_rows());

    let output = fixture.cmd().args(["-v", "list"]).output().unwrap();
    assert!(output.status.success());
    assert!(stdout(&output).starts_with("Found 5 session(s)\n"));
    let err = stderr(&output);
    assert!(
        err.ends_with(&format!(
            "warning: skipped 2 unreadable message rows in {path}\n\
             warning: skipped 6 unreadable composer rows in {path}\n",
            path = db.display()
        )),
        "{err}"
    );
    assert!(!err.contains("panicked"));

    // Without --verbose the skips are silent.
    let output = fixture.cmd().args(["show", "ok"]).output().unwrap();
    assert!(output.status.success());
    assert_eq!(stderr(&output), "");
    assert!(stdout(&output).ends_with("\n[user]\nquestion\n\n[assistant]\nanswer\n"));
}

fn wal_db(fixture: &Fixture) -> PathBuf {
    let db = fixture.ide_db_path();
    write_ide_db(&db, Journal::Wal, &standard_ide_rows());
    db
}

/// Opens `db` as Cursor does while running and commits a composer that stays
/// in the `-wal`, out of the main file.
fn live_writer(db: &Path) -> Connection {
    let writer = Connection::open(db).unwrap();
    writer.pragma_update(None, "wal_autocheckpoint", 0).unwrap();
    insert_rows(
        &writer,
        &[composer(
            "fresh",
            &composer_json(
                "fresh",
                "Only in the WAL",
                1_800_000_000_000,
                1_800_000_000_000,
                &[],
            ),
            Stored::Blob,
        )],
    );
    assert!(fs::metadata(sidecar(db, "-wal")).unwrap().len() > 0);
    assert!(sidecar(db, "-shm").is_file());
    writer
}

#[test]
fn commits_still_in_the_wal_of_a_running_cursor_are_read() {
    let fixture = Fixture::new();
    let db = wal_db(&fixture);
    let writer = live_writer(&db);

    let sessions = fixture.load().sessions;
    assert_eq!(sessions.len(), 5);
    assert_eq!(sessions[0].title, "Only in the WAL");
    let ids = listed_ids(&fixture, &["list"]);
    assert_eq!(ids[0], "fresh");
    assert_eq!(ids.len(), 5);

    // Cursor can keep writing while we read.
    writer
        .execute(
            "DELETE FROM cursorDiskKV WHERE key = 'composerData:fresh'",
            [],
        )
        .unwrap();
    assert_eq!(fixture.load().sessions.len(), 4);
    drop(writer);
}

#[test]
fn closed_wal_database_is_read_without_touching_its_directory() {
    let fixture = Fixture::new();
    let db = wal_db(&fixture);
    let dir = db.parent().unwrap();
    // Cursor removes the sidecars when it quits.
    assert_eq!(fs::read_dir(dir).unwrap().count(), 1);
    let before = listing(dir);

    assert_eq!(fixture.load().sessions.len(), 4);
    assert_eq!(listed_ids(&fixture, &["list"]).len(), 4);
    fixture.cmd().args(["show", "c0ffee00"]).assert().success();
    assert_eq!(listing(dir), before);
    assert_eq!(fs::read_dir(fixture.tmp()).unwrap().count(), 0);
}

#[test]
fn rollback_journal_database_is_read_without_touching_its_directory() {
    let fixture = standard();
    let dir = fixture.ide_db_path().parent().unwrap().to_path_buf();
    let before = listing(&dir);
    assert_eq!(fixture.load().sessions.len(), STANDARD_IDS.len());
    assert_eq!(listed_ids(&fixture, &["list"]), STANDARD_IDS);
    assert_eq!(listing(&dir), before);
}

#[test]
fn wal_left_by_a_crash_is_read_from_a_private_copy() {
    let fixture = Fixture::new();
    // Copy a running database and its -wal, but not its -shm: what a crash
    // leaves behind. (Windows cannot fs::copy a file SQLite has open.)
    let live = fixture.home().join("live").join("state.vscdb");
    write_ide_db(&live, Journal::Wal, &standard_ide_rows());
    let writer = live_writer(&live);
    let db = fixture.ide_db_path();
    for suffix in ["", "-wal"] {
        write_bytes(
            &sidecar(&db, suffix),
            &fs::read(sidecar(&live, suffix)).unwrap(),
        );
    }
    drop(writer);
    let dir = db.parent().unwrap();
    let before = listing(dir);
    assert_eq!(before.len(), 2);

    let sessions = fixture.load().sessions;
    assert_eq!(sessions.len(), 5);
    assert_eq!(sessions[0].id, "fresh");
    let ids = listed_ids(&fixture, &["list"]);
    assert_eq!(ids.len(), 5);
    assert_eq!(listing(dir), before);
    // The copy in the binary's temporary directory is gone.
    assert_eq!(fs::read_dir(fixture.tmp()).unwrap().count(), 0);
}

#[test]
fn a_read_waits_for_cursor_to_finish_writing() {
    let fixture = standard();
    let db = fixture.ide_db_path();
    let (locked, wait) = std::sync::mpsc::channel();
    // Cursor holds the write lock of a rollback-journal database for a moment.
    let writer = std::thread::spawn(move || {
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch("BEGIN EXCLUSIVE").unwrap();
        let (key, value) = composer(
            "late",
            &composer_json(
                "late",
                "Committed late",
                1_800_000_000_000,
                1_800_000_000_000,
                &[],
            ),
            Stored::Text,
        );
        conn.execute(
            "INSERT INTO cursorDiskKV (key, value) VALUES (?1, ?2)",
            rusqlite::params![key, value],
        )
        .unwrap();
        locked.send(()).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1500));
        conn.execute_batch("COMMIT").unwrap();
    });
    wait.recv().unwrap();
    let ids = listed_ids(&fixture, &["list", "--source", "ide"]);
    writer.join().unwrap();
    assert_eq!(ids[0], "late");
}

#[test]
fn snapshots_left_by_a_killed_process_are_removed_on_the_next_run() {
    use std::time::{Duration, SystemTime};

    let fixture = standard();
    let stale = fixture.tmp().join("cursor-session-1-0");
    let fresh = fixture.tmp().join("cursor-session-2-0");
    for dir in [&stale, &fresh] {
        fs::create_dir(dir).unwrap();
        fs::write(dir.join("state.vscdb"), "copy").unwrap();
    }
    let two_hours_ago = SystemTime::now() - Duration::from_secs(2 * 60 * 60);
    open_dir_for_times(&stale)
        .set_modified(two_hours_ago)
        .unwrap();

    // A run that copies no database still sweeps the temporary directory.
    assert_eq!(listed_ids(&fixture, &["list"]), STANDARD_IDS);
    assert!(!stale.exists());
    assert!(fresh.exists());
}

/// A directory opened so that its times can be set: Windows opens one only
/// with backup semantics, and setting times needs the right to write them.
fn open_dir_for_times(dir: &Path) -> fs::File {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_WRITE_ATTRIBUTES: u32 = 0x0100;
        const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
        fs::OpenOptions::new()
            .access_mode(FILE_WRITE_ATTRIBUTES)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(dir)
            .unwrap()
    }
    #[cfg(not(windows))]
    fs::File::open(dir).unwrap()
}

fn write_bytes(path: &Path, bytes: &[u8]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

#[test]
fn store_db_in_wal_mode_is_read_without_touching_its_directory() {
    let fixture = Fixture::new();
    let store = fixture.write_store_db(
        PROJECT_X,
        AGENT_ID,
        &json!({"name": "From a WAL store", "lastUsedModel": "gpt-5"}),
        Stored::HexText,
    );
    let conn = Connection::open(&store).unwrap();
    conn.pragma_update(None, "journal_mode", "wal").unwrap();
    drop(conn);
    let dir = store.parent().unwrap();
    let before = listing(dir);

    let sessions = fixture.load_source(Some(Source::Agent)).unwrap().sessions;
    assert_eq!(sessions[0].title, "From a WAL store");
    assert_eq!(listing(dir), before);
}

#[cfg(unix)]
#[test]
fn unreadable_database_is_an_access_error() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = standard();
    let db = fixture.ide_db_path();
    fs::set_permissions(&db, fs::Permissions::from_mode(0o000)).unwrap();
    // Root ignores permissions.
    let readable = fs::File::open(&db).is_ok();
    let loaded = fixture.load_source(None);
    let output = fixture.cmd().arg("list").output().unwrap();
    let agent_only = fixture
        .cmd()
        .args(["list", "--source", "agent"])
        .output()
        .unwrap();
    let healthcheck = fixture.cmd().arg("healthcheck").output().unwrap();
    fs::set_permissions(&db, fs::Permissions::from_mode(0o644)).unwrap();
    if readable {
        eprintln!("skipped: permissions are not enforced for this user (root)");
        return;
    }

    assert!(matches!(
        loaded,
        Err(Error::Io { path, source }) if path == db
            && source.kind() == std::io::ErrorKind::PermissionDenied
    ));
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(stdout(&output), "");
    let err = stderr(&output);
    assert!(
        err.starts_with(&format!(
            "error: could not access {}\n  caused by: Permission denied",
            db.display()
        )),
        "{err}"
    );
    assert!(agent_only.status.success());
    assert_eq!(healthcheck.status.code(), Some(1));
    assert!(stdout(&healthcheck).contains("state.vscdb (failed)\n"));
}

/// Two chats of one message each: `keep`, and `gone`.
fn two_chats(fixture: &Fixture) -> PathBuf {
    let mut rows = Vec::new();
    for (id, updated) in [("keep", 2_000), ("gone", 1_000)] {
        rows.push(composer(
            id,
            &composer_json(id, id, 1_000, updated, &[("b1", 1)]),
            Stored::Text,
        ));
        rows.push(bubble(
            id,
            "b1",
            &text_bubble("b1", 1, "hello"),
            Stored::Text,
        ));
    }
    fixture.write_ide_db(Journal::Delete, &rows)
}

#[test]
fn a_chat_deleted_while_it_is_read_is_a_distinct_error() {
    let fixture = Fixture::new();
    let db = two_chats(&fixture);
    let loaded = load_sessions(&fixture.paths(), &LoadOptions::default()).unwrap();
    let gone = loaded.sessions.iter().find(|s| s.id == "gone").unwrap();
    assert_eq!(gone.message_count, 1);
    Connection::open(&db)
        .unwrap()
        .execute_batch("DELETE FROM cursorDiskKV WHERE key LIKE '%gone%'")
        .unwrap();

    let error = load_messages(gone).unwrap_err();
    assert!(
        matches!(&error, Error::SessionGone { id } if id == "gone"),
        "{error:?}"
    );
    assert_eq!(
        error.to_string(),
        "session gone was deleted while it was being read"
    );
    assert_eq!(
        error.hints(),
        ["run `cursor-session list` to see the sessions there now"]
    );
    let keep = loaded.sessions.iter().find(|s| s.id == "keep").unwrap();
    assert_eq!(load_messages(keep).unwrap().messages.len(), 1);
}
