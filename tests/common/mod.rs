//! Fixture homes for the integration tests, generated at test time from JSON
//! and SQL. Each lives in its own temporary directory, removed on drop.
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

use cursor_session::detect::{Env, Os, StoragePaths, workspace_md5};
use cursor_session::model::{Session, SessionSummary, Source};
use cursor_session::{LoadOptions, load_messages, load_sessions};
use rusqlite::Connection;
use rusqlite::types::Value as SqlValue;
use serde_json::{Value, json};
use tempfile::TempDir;

pub const BIN: &str = env!("CARGO_BIN_EXE_cursor-session");

/// Variables that would let the host's terminal or Cursor setup reach the binary.
const HOST_ENV: [&str; 8] = [
    "CURSOR_CONFIG_DIR",
    "NO_COLOR",
    "TERM",
    "COLUMNS",
    "LINES",
    "CLICOLOR",
    "CLICOLOR_FORCE",
    "FORCE_COLOR",
];

/// Agent session with meta.json, a hex-encoded store.db and a nested transcript.
pub const AGENT_ID: &str = "f4eea6d2-d2d3-41ad-b290-824445295a15";
/// Agent session with only a store.db (BLOB value) and a flat transcript.
pub const STORE_ONLY_ID: &str = "7b1e0c55-9d2a-4c4e-8f61-2a3b4c5d6e7f";
/// Agent session known only from its transcript: no title, times or workspace.
pub const TRANSCRIPT_ONLY_ID: &str = "0a1b2c3d-4e5f-4061-8728-394a5b6c7d8e";
/// IDE composer with TEXT values, a richText-only bubble and a code block.
pub const IDE_TEXT_ID: &str = "c0ffee00-0000-4000-8000-000000000001";
/// IDE composer with BLOB values and a long multi-byte title.
pub const IDE_BLOB_ID: &str = "d00dfeed-0000-4000-8000-000000000002";
/// IDE composer without a name, an update time or messages.
pub const IDE_UNTITLED_ID: &str = "e1e10000-5555-4666-8777-888899990000";
/// In both stores; the agent copy wins.
pub const SHARED_ID: &str = "5ba7ed00-1111-4222-8333-444455556666";

pub const PROJECT_X: &str = "/Users/demo/project-x";
pub const PROJECT_Y: &str = "/Users/demo/project-y";
pub const LONG_TITLE: &str = "データパイプラインの再設計レビュー 🚀 — überprüfe den Migrationsplan für Q4 \
     und die Rollback-Strategie (ünïcödé ✓)";

/// Every session in [`standard`], newest first, as `list` orders them.
pub const STANDARD_IDS: [&str; 7] = [
    SHARED_ID,
    IDE_BLOB_ID,
    IDE_UNTITLED_ID,
    IDE_TEXT_ID,
    STORE_ONLY_ID,
    AGENT_ID,
    TRANSCRIPT_ONLY_ID,
];

/// How a JSON value is stored in a SQLite column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stored {
    Text,
    Blob,
    /// Hex-encoded JSON as TEXT, as cursor-agent writes store.db.
    HexText,
    /// Hex-encoded JSON as a BLOB.
    HexBlob,
}

impl Stored {
    pub fn value(self, json: &Value) -> SqlValue {
        let text = json.to_string();
        match self {
            Stored::Text => SqlValue::Text(text),
            Stored::Blob => SqlValue::Blob(text.into_bytes()),
            Stored::HexText => SqlValue::Text(hex(text.as_bytes())),
            Stored::HexBlob => SqlValue::Blob(hex(text.as_bytes()).into_bytes()),
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Journal {
    Delete,
    Wal,
}

/// Where a transcript sits under `projects/<project>/agent-transcripts`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    /// `<id>/<id>.jsonl`
    Nested,
    /// `<id>.jsonl`
    Flat,
}

/// A fake home directory. The binary sees it through HOME, USERPROFILE,
/// XDG_CONFIG_HOME and APPDATA, and the library through [`Fixture::paths`].
pub struct Fixture {
    dir: TempDir,
    config_dir: PathBuf,
}

impl Default for Fixture {
    fn default() -> Self {
        Self::new()
    }
}

impl Fixture {
    /// An empty home whose config directories are the platform defaults:
    /// `~/.config` (XDG_CONFIG_HOME) and `~/AppData/Roaming` (APPDATA).
    pub fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("tmp")).unwrap();
        let config_dir = if Os::current() == Os::Windows {
            dir.path().join("AppData").join("Roaming")
        } else {
            dir.path().join(".config")
        };
        Self { dir, config_dir }
    }

    /// An empty home whose XDG_CONFIG_HOME and APPDATA point at `<home>/<name>`,
    /// away from the defaults.
    pub fn with_config_dir(name: &str) -> Self {
        let mut fixture = Self::new();
        fixture.config_dir = fixture.home().join(name);
        fixture
    }

    pub fn home(&self) -> &Path {
        self.dir.path()
    }

    /// The binary's temporary directory, where it copies a database a crash
    /// left with a `-wal`.
    pub fn tmp(&self) -> PathBuf {
        self.home().join("tmp")
    }

    pub fn env(&self) -> Env {
        Env {
            os: Os::current(),
            home: Some(self.home().to_path_buf()),
            xdg_config_home: Some(self.config_dir.clone()),
            appdata: Some(self.config_dir.clone()),
            cursor_config_dir: None,
        }
    }

    /// The storage the binary detects with [`Fixture::command`]'s environment.
    pub fn paths(&self) -> StoragePaths {
        StoragePaths::from_env(&self.env()).unwrap()
    }

    pub fn load(&self) -> Full {
        self.load_source(None).unwrap()
    }

    pub fn load_source(&self, source: Option<Source>) -> cursor_session::Result<Full> {
        load_full(
            &self.paths(),
            &LoadOptions {
                source,
                ..Default::default()
            },
        )
    }

    pub fn chats_dir(&self) -> PathBuf {
        self.home().join(".cursor").join("chats")
    }

    pub fn projects_dir(&self) -> PathBuf {
        self.home().join(".cursor").join("projects")
    }

    /// `state.vscdb` where Cursor keeps it on this OS.
    pub fn ide_db_path(&self) -> PathBuf {
        let app_data = match Os::current() {
            Os::MacOs => self.home().join("Library").join("Application Support"),
            Os::Linux | Os::Windows => self.config_dir.clone(),
        };
        app_data
            .join("Cursor")
            .join("User")
            .join("globalStorage")
            .join("state.vscdb")
    }

    pub fn session_dir(&self, cwd: &str, id: &str) -> PathBuf {
        self.chats_dir().join(workspace_md5(cwd)).join(id)
    }

    pub fn write_meta_json(&self, cwd: &str, id: &str, meta: &Value) -> PathBuf {
        let path = self.session_dir(cwd, id).join("meta.json");
        write(&path, &meta.to_string());
        path
    }

    /// A cursor-agent `store.db` whose `meta` row `'0'` holds `meta`.
    pub fn write_store_db(&self, cwd: &str, id: &str, meta: &Value, stored: Stored) -> PathBuf {
        let path = self.session_dir(cwd, id).join("store.db");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE meta (key TEXT PRIMARY KEY, value BLOB);
             CREATE TABLE blobs (id TEXT PRIMARY KEY, data BLOB);",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO meta (key, value) VALUES ('0', ?1)",
            [stored.value(meta)],
        )
        .unwrap();
        path
    }

    pub fn write_transcript(
        &self,
        project: &str,
        id: &str,
        layout: Layout,
        lines: &[Value],
    ) -> PathBuf {
        let dir = self.projects_dir().join(project).join("agent-transcripts");
        let path = match layout {
            Layout::Nested => dir.join(id).join(format!("{id}.jsonl")),
            Layout::Flat => dir.join(format!("{id}.jsonl")),
        };
        let text: String = lines.iter().map(|line| format!("{line}\n")).collect();
        write(&path, &text);
        path
    }

    /// Writes `state.vscdb` at the platform location; see [`write_ide_db`].
    pub fn write_ide_db(&self, journal: Journal, rows: &[(String, SqlValue)]) -> PathBuf {
        let path = self.ide_db_path();
        write_ide_db(&path, journal, rows);
        path
    }

    /// The binary, run in this home with the host's terminal settings removed.
    pub fn command(&self) -> Command {
        self.isolated(BIN)
    }

    pub fn cmd(&self) -> assert_cmd::Command {
        assert_cmd::Command::from_std(self.command())
    }

    /// `program` with this home's environment (see [`Fixture::command`]).
    pub fn isolated(&self, program: impl AsRef<OsStr>) -> Command {
        let mut cmd = Command::new(program);
        for name in HOST_ENV {
            cmd.env_remove(name);
        }
        let tmp = self.tmp();
        cmd.current_dir(self.home())
            .env("HOME", self.home())
            .env("USERPROFILE", self.home())
            .env("XDG_CONFIG_HOME", &self.config_dir)
            .env("APPDATA", &self.config_dir)
            .env("TMPDIR", &tmp)
            .env("TMP", &tmp)
            .env("TEMP", &tmp);
        cmd
    }
}

/// Every session listed, as a summary and with its messages.
#[derive(Debug)]
pub struct Full {
    pub summaries: Vec<SessionSummary>,
    pub sessions: Vec<Session>,
    pub warnings: Vec<String>,
    pub notices: Vec<String>,
}

/// Lists the sessions and loads each one's messages, which must number what
/// the list counted.
pub fn load_full(paths: &StoragePaths, opts: &LoadOptions) -> cursor_session::Result<Full> {
    let loaded = load_sessions(paths, opts)?;
    let sessions = with_messages(&loaded.sessions)?;
    Ok(Full {
        summaries: loaded.sessions,
        sessions,
        warnings: loaded.warnings,
        notices: loaded.notices,
    })
}

/// Loads the messages of each of `summaries`, checking that they number what
/// the summary counted, with as many characters.
pub fn with_messages(summaries: &[SessionSummary]) -> cursor_session::Result<Vec<Session>> {
    summaries
        .iter()
        .map(|summary| {
            let session = load_messages(summary)?;
            assert_eq!(
                session.messages.len(),
                summary.message_count,
                "messages of {} differ from the count",
                summary.id
            );
            assert_eq!(
                session.content_chars, summary.content_chars,
                "characters of {} differ from the count",
                summary.id
            );
            Ok(session)
        })
        .collect()
}

/// The summaries of `sessions`, as `list` renders them.
pub fn summaries(sessions: &[Session]) -> Vec<SessionSummary> {
    sessions
        .iter()
        .map(|session| session.summary.clone())
        .collect()
}

pub fn write(path: &Path, contents: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

/// Creates `state.vscdb` with VS Code's `ItemTable` and Cursor's `cursorDiskKV`.
pub fn create_kv_db(path: &Path, journal: Journal) -> Connection {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let conn = Connection::open(path).unwrap();
    let mode = match journal {
        Journal::Delete => "delete",
        Journal::Wal => "wal",
    };
    conn.pragma_update(None, "journal_mode", mode).unwrap();
    conn.execute_batch(
        "CREATE TABLE ItemTable (key TEXT UNIQUE ON CONFLICT REPLACE, value BLOB);
         CREATE TABLE cursorDiskKV (key TEXT UNIQUE ON CONFLICT REPLACE, value BLOB);",
    )
    .unwrap();
    conn
}

pub fn insert_rows(conn: &Connection, rows: &[(String, SqlValue)]) {
    conn.execute_batch("BEGIN").unwrap();
    for (key, value) in rows {
        conn.execute(
            "INSERT INTO cursorDiskKV (key, value) VALUES (?1, ?2)",
            rusqlite::params![key, value],
        )
        .unwrap();
    }
    conn.execute_batch("COMMIT").unwrap();
}

/// Writes a closed `state.vscdb` holding `rows`. A WAL database loses its
/// `-wal` and `-shm` on close, as when Cursor quits.
pub fn write_ide_db(path: &Path, journal: Journal, rows: &[(String, SqlValue)]) {
    let conn = create_kv_db(path, journal);
    insert_rows(&conn, rows);
}

/// Creates a SQLite database at `path` from `sql`.
pub fn write_sql_db(path: &Path, sql: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    Connection::open(path).unwrap().execute_batch(sql).unwrap();
}

pub fn composer(id: &str, json: &Value, stored: Stored) -> (String, SqlValue) {
    (format!("composerData:{id}"), stored.value(json))
}

pub fn bubble(
    composer_id: &str,
    bubble_id: &str,
    json: &Value,
    stored: Stored,
) -> (String, SqlValue) {
    (
        format!("bubbleId:{composer_id}:{bubble_id}"),
        stored.value(json),
    )
}

/// A composer whose conversation lists `bubbles` as (id, type) headers.
pub fn composer_json(
    id: &str,
    name: &str,
    created: i64,
    updated: i64,
    bubbles: &[(&str, i64)],
) -> Value {
    json!({
        "composerId": id,
        "name": name,
        "createdAt": created,
        "lastUpdatedAt": updated,
        "fullConversationHeadersOnly": headers(bubbles),
    })
}

pub fn headers(bubbles: &[(&str, i64)]) -> Value {
    bubbles
        .iter()
        .map(|(id, kind)| json!({"bubbleId": id, "type": kind}))
        .collect()
}

pub fn text_bubble(id: &str, kind: i64, text: &str) -> Value {
    json!({"bubbleId": id, "type": kind, "text": text})
}

/// A user message as cursor-agent writes it, wrapped in its tags.
pub fn user_query(timestamp: &str, query: &str) -> Value {
    json!({
        "role": "user",
        "message": {"content": [{
            "type": "text",
            "text": format!("<timestamp>{timestamp}</timestamp>\n<user_query>\n{query}\n</user_query>"),
        }]},
    })
}

/// A message whose content is a plain string rather than parts.
pub fn plain_message(role: &str, text: &str) -> Value {
    json!({"role": role, "message": {"content": text}})
}

pub fn assistant(parts: &[Value]) -> Value {
    json!({"role": "assistant", "message": {"content": parts}})
}

pub fn text_part(text: &str) -> Value {
    json!({"type": "text", "text": text})
}

pub fn tool_use(name: &str, input: &Value) -> Value {
    json!({"type": "tool_use", "name": name, "input": input})
}

/// What a tool returned, as a `tool_result` content part.
pub fn tool_result(content: &Value) -> Value {
    json!({"type": "tool_result", "tool_use_id": "call-1", "content": content})
}

/// A transcript line of the role `tool`.
pub fn tool_line(parts: &[Value]) -> Value {
    json!({"role": "tool", "message": {"content": parts}})
}

/// An IDE message that makes a tool call, with what it returned, as Cursor
/// keeps them in `toolFormerData`: the arguments and the result as JSON in
/// strings.
pub fn tool_bubble(id: &str, text: &str, name: &str, args: &Value, result: &Value) -> Value {
    json!({
        "bubbleId": id,
        "type": 2,
        "text": text,
        "toolFormerData": {
            "tool": 5,
            "toolCallId": format!("call-{id}"),
            "name": name,
            "rawArgs": args.to_string(),
            "params": args.to_string(),
            "result": result.to_string(),
            "status": "completed",
        },
    })
}

/// Agent CLI session with tool calls and results, from [`tools`].
pub const AGENT_TOOLS_ID: &str = "70015000-aaaa-4bbb-8ccc-000000000001";
/// IDE chat with tool calls, from [`tools`].
pub const IDE_TOOLS_ID: &str = "70015000-aaaa-4bbb-8ccc-000000000002";

/// One session in each store whose messages make tool calls.
pub fn tools() -> Fixture {
    let fixture = Fixture::new();
    fixture.write_meta_json(
        PROJECT_X,
        AGENT_TOOLS_ID,
        &json!({
            "title": "Agent with tools",
            "createdAtMs": 1_757_500_000_000_i64,
            "updatedAtMs": 1_757_500_060_000_i64,
            "cwd": PROJECT_X,
        }),
    );
    fixture.write_transcript(
        "Users-demo-project-x",
        AGENT_TOOLS_ID,
        Layout::Nested,
        &[
            user_query("Mon", "Where is langfuse configured?"),
            assistant(&[
                text_part("Let me search."),
                tool_use("Grep", &json!({"pattern": "langfuse"})),
            ]),
            tool_line(&[tool_result(&json!("src/config.rs:12: langfuse_host"))]),
            assistant(&[tool_use("Read", &json!({"path": "src/config.rs"}))]),
            tool_line(&[tool_result(&json!([text_part(&"long line ".repeat(40))]))]),
            assistant(&[text_part(&format!(
                "It is configured in src/config.rs.{}",
                " Details follow.".repeat(20)
            ))]),
        ],
    );
    fixture.write_ide_db(
        Journal::Delete,
        &[
            composer(
                IDE_TOOLS_ID,
                &composer_json(
                    IDE_TOOLS_ID,
                    "IDE with tools",
                    1_757_400_000_000,
                    1_757_400_060_000,
                    &[("t1", 1), ("t2", 2), ("t3", 2)],
                ),
                Stored::Text,
            ),
            bubble(
                IDE_TOOLS_ID,
                "t1",
                &text_bubble("t1", 1, "List the files."),
                Stored::Text,
            ),
            bubble(
                IDE_TOOLS_ID,
                "t2",
                &tool_bubble(
                    "t2",
                    "Listing them.",
                    "list_dir",
                    &json!({"relative_workspace_path": "."}),
                    &json!({"files": ["Cargo.toml", "src"]}),
                ),
                Stored::Text,
            ),
            bubble(
                IDE_TOOLS_ID,
                "t3",
                &text_bubble("t3", 2, "Two entries."),
                Stored::Text,
            ),
        ],
    );
    fixture
}

/// Seven sessions covering both stores, every storage variant and missing
/// fields, with fixed times (see the `*_ID` constants).
pub fn standard() -> Fixture {
    let fixture = Fixture::new();
    write_standard_agent(&fixture);
    fixture.write_ide_db(Journal::Delete, &standard_ide_rows());
    fixture
}

pub fn write_standard_agent(fixture: &Fixture) {
    fixture.write_meta_json(
        PROJECT_X,
        AGENT_ID,
        &json!({
            "schemaVersion": 1,
            "createdAtMs": 1_757_000_000_000_i64,
            "hasConversation": true,
            "title": "Langfuse Semantic Layer",
            "updatedAtMs": 1_757_003_600_000_i64,
            "cwd": PROJECT_X,
        }),
    );
    fixture.write_store_db(
        PROJECT_X,
        AGENT_ID,
        &json!({
            "name": "Store name loses to meta.json",
            "lastUsedModel": "claude-4.5-sonnet",
            "createdAt": 1_756_999_000_000_i64,
        }),
        Stored::HexText,
    );
    fixture.write_transcript(
        "Users-demo-project-x",
        AGENT_ID,
        Layout::Nested,
        &[
            user_query(
                "Thursday, Sep 4, 2025, 9:33 AM (UTC-6)",
                "Design a semantic layer for Langfuse traces.",
            ),
            assistant(&[
                text_part("Let me look at the trace schema first."),
                tool_use("Grep", &json!({"pattern": "trace"})),
            ]),
            assistant(&[tool_use("Read", &json!({"path": "schema.sql"}))]),
            plain_message("user", "Keep it read-only."),
            assistant(&[text_part(
                "Here is the plan:\n1. Model traces as facts.\n2. Expose metrics.",
            )]),
        ],
    );

    fixture.write_store_db(
        PROJECT_Y,
        STORE_ONLY_ID,
        &json!({
            "name": "Fix flaky CI on Windows",
            "lastUsedModel": "gpt-5",
            "createdAt": 1_757_100_000_000_i64,
        }),
        Stored::HexBlob,
    );
    let flat = fixture.write_transcript(
        "Users-demo-project-y",
        STORE_ONLY_ID,
        Layout::Flat,
        &[
            plain_message("user", "Why does CI fail on Windows only?"),
            json!({"role": "system", "message": {"content": "ignored role"}}),
            assistant(&[
                text_part("Path separators: build paths with Path::join."),
                tool_use("Bash", &json!({"command": "cargo test"})),
            ]),
        ],
    );
    // Lines that are not JSON are skipped.
    let mut text = fs::read_to_string(&flat).unwrap();
    text.push_str("{not json\n\n");
    fs::write(&flat, text).unwrap();

    fixture.write_transcript(
        "Users-demo-project-x",
        TRANSCRIPT_ONLY_ID,
        Layout::Nested,
        &[
            plain_message("user", "Summarize yesterday's changes."),
            plain_message("assistant", "Nothing changed yesterday."),
        ],
    );

    fixture.write_meta_json(
        PROJECT_Y,
        SHARED_ID,
        &json!({
            "createdAtMs": 1_757_400_000_000_i64,
            "updatedAtMs": 1_757_400_060_000_i64,
            "title": "Shared session from the agent",
            "cwd": PROJECT_Y,
            "hasConversation": true,
        }),
    );
    fixture.write_transcript(
        "Users-demo-project-y",
        SHARED_ID,
        Layout::Nested,
        &[
            user_query("Saturday, Sep 9, 2025, 1:20 AM (UTC-6)", "Which copy wins?"),
            assistant(&[text_part("The agent transcript wins.")]),
        ],
    );
}

pub fn standard_ide_rows() -> Vec<(String, SqlValue)> {
    let rich_text = json!({"root": {"children": [{"children": [
        {"text": "Here is "},
        {"text": "a plan."},
    ]}]}});
    vec![
        composer(
            IDE_TEXT_ID,
            &composer_json(
                IDE_TEXT_ID,
                "Dynamic table proposal",
                1_757_200_000_000,
                1_757_203_600_000,
                &[("b1", 1), ("b2", 2), ("b-missing", 2), ("b3", 2), ("b4", 2)],
            ),
            Stored::Text,
        ),
        bubble(
            IDE_TEXT_ID,
            "b1",
            &json!({
                "bubbleId": "b1",
                "type": 1,
                "text": "Replace this view with a dynamic table.",
                "timestamp": 1_757_200_000_000_i64,
            }),
            Stored::Text,
        ),
        bubble(
            IDE_TEXT_ID,
            "b2",
            &json!({
                "bubbleId": "b2",
                "type": 2,
                "text": "",
                "richText": rich_text.to_string(),
                "timestamp": 1_757_200_060_000_i64,
            }),
            Stored::Text,
        ),
        bubble(
            IDE_TEXT_ID,
            "b3",
            &json!({
                "bubbleId": "b3",
                "type": 2,
                "text": "Use this component:",
                "codeBlocks": [
                    {"language": "tsx", "content": "export function Table() {\n  return null;\n}"},
                    {"language": "sh", "content": ""},
                ],
            }),
            Stored::Text,
        ),
        // Whitespace only: no message.
        bubble(
            IDE_TEXT_ID,
            "b4",
            &text_bubble("b4", 2, "  \n "),
            Stored::Text,
        ),
        composer(
            IDE_BLOB_ID,
            &composer_json(
                IDE_BLOB_ID,
                LONG_TITLE,
                1_757_300_000_000,
                1_757_386_400_000,
                &[("e1", 1), ("e2", 2)],
            ),
            Stored::Blob,
        ),
        bubble(
            IDE_BLOB_ID,
            "e1",
            &text_bubble(
                "e1",
                1,
                "Kannst du die Migration prüfen? 日本語も大丈夫です。",
            ),
            Stored::Blob,
        ),
        bubble(
            IDE_BLOB_ID,
            "e2",
            &text_bubble("e2", 2, "Ja — the rollback plan looks fine ✓"),
            Stored::Blob,
        ),
        composer(
            IDE_UNTITLED_ID,
            &json!({"composerId": IDE_UNTITLED_ID, "createdAt": 1_757_250_000_000_i64}),
            Stored::Text,
        ),
        composer(
            SHARED_ID,
            &composer_json(
                SHARED_ID,
                "Shared session from the IDE",
                1_757_399_990_000,
                1_757_400_120_000,
                &[("s1", 1), ("s2", 2), ("s3", 1)],
            ),
            Stored::Blob,
        ),
        bubble(
            SHARED_ID,
            "s1",
            &text_bubble("s1", 1, "IDE question"),
            Stored::Text,
        ),
        bubble(
            SHARED_ID,
            "s2",
            &text_bubble("s2", 2, "IDE answer"),
            Stored::Text,
        ),
        bubble(
            SHARED_ID,
            "s3",
            &text_bubble("s3", 1, "IDE follow-up"),
            Stored::Text,
        ),
    ]
}

/// File names, contents and modification times in `dir`, recursively.
pub fn listing(dir: &Path) -> Vec<(PathBuf, Vec<u8>, Option<SystemTime>)> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let meta = fs::metadata(&path).unwrap();
        if meta.is_dir() {
            entries.extend(listing(&path));
        } else {
            let bytes = fs::read(&path).unwrap();
            entries.push((path, bytes, meta.modified().ok()));
        }
    }
    entries.sort();
    entries
}

pub fn stdout(output: &std::process::Output) -> String {
    String::from_utf8(output.stdout.clone()).unwrap()
}

pub fn stderr(output: &std::process::Output) -> String {
    String::from_utf8(output.stderr.clone()).unwrap()
}
