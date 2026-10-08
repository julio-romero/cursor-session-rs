//! Library tests: loading both stores, merging, lookup and export.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::fs;

use common::*;
use cursor_session::detect::{StoragePaths, workspace_md5};
use cursor_session::export::{self, Format};
use cursor_session::model::{Message, MessagesAt, Session, SessionSummary, Source, merge_sessions};
use cursor_session::{Error, LoadOptions, filter_workspace, find_session, load_sessions};
use serde_json::json;

fn ids(sessions: &[Session]) -> Vec<&str> {
    sessions.iter().map(|s| s.id.as_str()).collect()
}

fn contents(session: &Session) -> Vec<&str> {
    session
        .messages
        .iter()
        .map(|m| m.content.as_str())
        .collect()
}

fn get<'a>(sessions: &'a [Session], id: &str) -> &'a Session {
    sessions
        .iter()
        .find(|s| s.id == id)
        .unwrap_or_else(|| panic!("{id} not loaded: {:?}", ids(sessions)))
}

#[test]
fn stores_are_detected_at_the_platform_locations() {
    let fixture = standard();
    let paths = fixture.paths();
    assert_eq!(paths.chats_dir, Some(fixture.chats_dir()));
    assert_eq!(paths.projects_dir, Some(fixture.projects_dir()));
    assert_eq!(paths.global_storage_db, Some(fixture.ide_db_path()));
    assert_eq!(
        StoragePaths::from_home(fixture.home()).global_storage_db,
        Some(fixture.ide_db_path())
    );
}

#[test]
fn ide_db_follows_xdg_config_home_and_appdata() {
    let fixture = Fixture::with_config_dir("custom-config");
    let db = fixture.write_ide_db(Journal::Delete, &standard_ide_rows());
    // macOS keeps it under ~/Library whatever the environment says.
    if !cfg!(target_os = "macos") {
        assert!(db.starts_with(fixture.home().join("custom-config")));
    }
    assert_eq!(fixture.paths().global_storage_db, Some(db));
    assert_eq!(fixture.load().sessions.len(), 4);
}

#[test]
fn both_sources_load_newest_first_without_warnings() {
    let loaded = standard().load();
    assert_eq!(ids(&loaded.sessions), STANDARD_IDS);
    assert_eq!(loaded.warnings, Vec::<String>::new());
    let sources: Vec<Source> = loaded.sessions.iter().map(|s| s.source).collect();
    use Source::{Agent, Ide};
    assert_eq!(sources, [Agent, Ide, Ide, Ide, Agent, Agent, Agent]);
}

#[test]
fn agent_session_combines_meta_json_store_db_and_transcript() {
    let sessions = standard().load().sessions;
    let session = get(&sessions, AGENT_ID);
    assert_eq!(session.title, "Langfuse Semantic Layer");
    assert_eq!(session.source, Source::Agent);
    assert_eq!(session.workspace.as_deref(), Some(PROJECT_X));
    assert_eq!(
        session.workspace_hash.as_deref(),
        Some(workspace_md5(PROJECT_X).as_str())
    );
    assert_eq!(session.model.as_deref(), Some("claude-4.5-sonnet"));
    assert_eq!(session.created_at_ms, Some(1_757_000_000_000));
    assert_eq!(session.updated_at_ms, Some(1_757_003_600_000));

    // The wrappers are stripped and tool calls are not messages.
    let roles: Vec<&str> = session.messages.iter().map(|m| m.role.as_str()).collect();
    assert_eq!(roles, ["user", "assistant", "user", "assistant"]);
    assert_eq!(
        contents(session),
        [
            "Design a semantic layer for Langfuse traces.",
            "Let me look at the trace schema first.",
            "Keep it read-only.",
            "Here is the plan:\n1. Model traces as facts.\n2. Expose metrics.",
        ]
    );
    assert_eq!(
        session.messages[0].timestamp.as_deref(),
        Some("Thursday, Sep 4, 2025, 9:33 AM (UTC-6)")
    );
    assert_eq!(session.messages[2].timestamp, None);
}

#[test]
fn store_db_alone_names_a_session_and_flat_transcripts_load() {
    let sessions = standard().load().sessions;
    let session = get(&sessions, STORE_ONLY_ID);
    assert_eq!(session.title, "Fix flaky CI on Windows");
    assert_eq!(session.model.as_deref(), Some("gpt-5"));
    assert_eq!(session.created_at_ms, Some(1_757_100_000_000));
    assert_eq!(session.updated_at_ms, None);
    assert_eq!(session.workspace, None);
    assert_eq!(
        session.workspace_hash.as_deref(),
        Some(workspace_md5(PROJECT_Y).as_str())
    );
    // The system line, the broken line and the blank line are skipped.
    assert_eq!(
        contents(session),
        [
            "Why does CI fail on Windows only?",
            "Path separators: build paths with Path::join."
        ]
    );
}

#[test]
fn transcript_only_session_is_titled_by_its_id() {
    let sessions = standard().load().sessions;
    let session = get(&sessions, TRANSCRIPT_ONLY_ID);
    assert_eq!(session.title, TRANSCRIPT_ONLY_ID);
    assert_eq!(session.source, Source::Agent);
    assert_eq!(
        (
            &session.workspace,
            &session.workspace_hash,
            session.created_at_ms,
            session.updated_at_ms,
            &session.model
        ),
        (&None, &None, None, None, &None)
    );
    assert_eq!(session.messages.len(), 2);
}

#[test]
fn ide_composer_reads_text_rich_text_and_code_blocks() {
    let sessions = standard().load().sessions;
    let session = get(&sessions, IDE_TEXT_ID);
    assert_eq!(session.title, "Dynamic table proposal");
    assert_eq!(session.source, Source::Ide);
    assert_eq!(session.created_at_ms, Some(1_757_200_000_000));
    assert_eq!(session.updated_at_ms, Some(1_757_203_600_000));
    // The missing and the whitespace-only bubbles are left out.
    assert_eq!(
        contents(session),
        [
            "Replace this view with a dynamic table.",
            "Here is a plan.",
            "Use this component:\n\n```tsx\nexport function Table() {\n  return null;\n}\n```",
        ]
    );
    let roles: Vec<&str> = session.messages.iter().map(|m| m.role.as_str()).collect();
    assert_eq!(roles, ["user", "assistant", "assistant"]);
    let stamps: Vec<Option<&str>> = session
        .messages
        .iter()
        .map(|m| m.timestamp.as_deref())
        .collect();
    assert_eq!(stamps, [Some("1757200000000"), Some("1757200060000"), None]);
}

#[test]
fn blob_and_text_values_load_the_same_sessions() {
    let as_json = |stored: Stored| {
        let fixture = Fixture::new();
        let rows: Vec<_> = standard_ide_rows()
            .into_iter()
            .map(|(key, value)| {
                let text = match value {
                    rusqlite::types::Value::Text(text) => text,
                    rusqlite::types::Value::Blob(bytes) => String::from_utf8(bytes).unwrap(),
                    other => panic!("{other:?}"),
                };
                let json: serde_json::Value = serde_json::from_str(&text).unwrap();
                (key, stored.value(&json))
            })
            .collect();
        fixture.write_ide_db(Journal::Delete, &rows);
        let loaded = fixture.load();
        assert_eq!(loaded.warnings, Vec::<String>::new());
        serde_json::to_string(&loaded.sessions).unwrap()
    };
    let text = as_json(Stored::Text);
    assert_eq!(text, as_json(Stored::Blob));
    let sessions: Vec<Session> = serde_json::from_str(&text).unwrap();
    assert_eq!(sessions.len(), 4);
    assert_eq!(get(&sessions, IDE_BLOB_ID).title, LONG_TITLE);
}

#[test]
fn ide_composer_without_a_name_is_untitled() {
    let sessions = standard().load().sessions;
    let session = get(&sessions, IDE_UNTITLED_ID);
    assert_eq!(session.title, "Untitled");
    assert_eq!(session.created_at_ms, Some(1_757_250_000_000));
    // Like an agent session, a chat never updated has no update time; the
    // table shows, and the list orders, its creation time instead.
    assert_eq!(session.updated_at_ms, None);
    assert_eq!(session.updated_display(), "2025-09-07 13:00");
    assert!(session.messages.is_empty());
}

#[test]
fn a_session_in_both_stores_merges_into_the_agent_copy() {
    let fixture = standard();
    let sessions = fixture.load().sessions;
    let shared = get(&sessions, SHARED_ID);
    assert_eq!(sessions.iter().filter(|s| s.id == SHARED_ID).count(), 1);
    assert_eq!(shared.source, Source::Agent);
    assert_eq!(shared.title, "Shared session from the agent");
    assert_eq!(shared.workspace.as_deref(), Some(PROJECT_Y));
    assert_eq!(shared.created_at_ms, Some(1_757_400_000_000));
    // The newer update time of the two.
    assert_eq!(shared.updated_at_ms, Some(1_757_400_120_000));
    assert_eq!(
        contents(shared),
        ["Which copy wins?", "The agent transcript wins."]
    );

    let ide = fixture.load_source(Some(Source::Ide)).unwrap().sessions;
    let shared = get(&ide, SHARED_ID);
    assert_eq!(shared.source, Source::Ide);
    assert_eq!(shared.title, "Shared session from the IDE");
    assert_eq!(
        contents(shared),
        ["IDE question", "IDE answer", "IDE follow-up"]
    );
}

#[test]
fn merge_fills_gaps_from_the_other_store() {
    let at = |source| match source {
        Source::Agent => MessagesAt::Transcript("agent.jsonl".into()),
        Source::Ide => MessagesAt::IdeChat {
            db: "state.vscdb".into(),
            key: "composerData:same".into(),
            blob_key: false,
        },
    };
    let session = |source, title: &str, messages: usize| SessionSummary {
        updated_at_ms: Some(1),
        message_count: messages,
        messages_at: if messages > 0 {
            at(source)
        } else {
            MessagesAt::Nowhere
        },
        ..SessionSummary::new("same", title, source)
    };
    // A transcript-only agent session is titled by its ID; the IDE names it.
    let merged = merge_sessions(vec![
        session(Source::Agent, "same", 1),
        session(Source::Ide, "Named in the IDE", 2),
    ]);
    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0].title, "Named in the IDE");
    assert_eq!(merged[0].source, Source::Agent);
    assert_eq!(
        (merged[0].message_count, &merged[0].messages_at),
        (1, &at(Source::Agent))
    );

    // An agent session without a transcript takes the IDE's messages.
    let merged = merge_sessions(vec![
        session(Source::Agent, "agent", 0),
        session(Source::Ide, "ide", 2),
    ]);
    assert_eq!(merged[0].source, Source::Agent);
    assert_eq!(merged[0].title, "agent");
    assert_eq!(
        (merged[0].message_count, &merged[0].messages_at),
        (2, &at(Source::Ide))
    );
}

#[test]
fn source_filter_loads_only_that_store() {
    let fixture = standard();
    let agent = fixture.load_source(Some(Source::Agent)).unwrap();
    assert_eq!(
        ids(&agent.sessions),
        [SHARED_ID, STORE_ONLY_ID, AGENT_ID, TRANSCRIPT_ONLY_ID]
    );
    let ide = fixture.load_source(Some(Source::Ide)).unwrap();
    assert_eq!(
        ids(&ide.sessions),
        [SHARED_ID, IDE_BLOB_ID, IDE_UNTITLED_ID, IDE_TEXT_ID]
    );
    assert!(ide.sessions.iter().all(|s| s.source == Source::Ide));

    // The agent store loads without ever opening a broken IDE database.
    fs::write(
        fixture.ide_db_path(),
        "not a database, but long enough for a header",
    )
    .unwrap();
    assert_eq!(
        fixture
            .load_source(Some(Source::Agent))
            .unwrap()
            .sessions
            .len(),
        4
    );
    assert!(matches!(
        fixture.load_source(None),
        Err(Error::Database { .. })
    ));
    assert!(matches!(
        fixture.load_source(Some(Source::Ide)),
        Err(Error::Database { .. })
    ));
}

#[test]
fn missing_stores_load_nothing() {
    let fixture = Fixture::new();
    let paths = fixture.paths();
    assert_eq!(
        (paths.chats_dir, paths.projects_dir, paths.global_storage_db),
        (None, None, None)
    );
    let loaded = fixture.load();
    assert!(loaded.sessions.is_empty());
    assert!(loaded.warnings.is_empty());
}

#[test]
fn find_session_matches_exact_ids_and_unique_prefixes() {
    let sessions = standard().load().summaries;
    let found = |query: &str| find_session(&sessions, query).map(|s| s.id.as_str());
    assert_eq!(found(AGENT_ID).unwrap(), AGENT_ID);
    assert_eq!(found("f4eea6d2").unwrap(), AGENT_ID);
    assert_eq!(found("C0FFEE00-0000").unwrap(), IDE_TEXT_ID);
    assert_eq!(found(&IDE_BLOB_ID.to_uppercase()).unwrap(), IDE_BLOB_ID);
    assert_eq!(found("  e1e1\n").unwrap(), IDE_UNTITLED_ID);
    assert_eq!(found("5").unwrap(), SHARED_ID);
}

#[test]
fn find_session_reports_unknown_empty_and_ambiguous_ids() {
    let sessions = standard().load().summaries;

    let err = find_session(&sessions, "ffff").unwrap_err();
    assert!(matches!(
        &err,
        Error::SessionNotFound { query, unsearched: None } if query == "ffff"
    ));
    assert_eq!(err.to_string(), "session not found: ffff");
    assert_eq!(
        err.hints(),
        ["run `cursor-session list` to see session IDs"]
    );

    for empty in ["", "   ", "\t\n"] {
        let err = find_session(&sessions, empty).unwrap_err();
        assert!(matches!(err, Error::EmptyId), "{empty:?}");
    }
    // A query longer than every ID is not a prefix of any.
    let longer = format!("{AGENT_ID}0");
    assert!(matches!(
        find_session(&sessions, &longer),
        Err(Error::SessionNotFound { .. })
    ));
    assert!(matches!(
        find_session(&[], "f4"),
        Err(Error::SessionNotFound { .. })
    ));
}

fn ambiguous_fixture() -> Fixture {
    let fixture = Fixture::new();
    let rows: Vec<_> = [
        ("abcd1111-0000-4000-8000-000000000001", "First abcd", 1),
        ("abcd2222-0000-4000-8000-000000000002", "Second abcd", 2),
        ("ABCE3333-0000-4000-8000-000000000003", "Upper-case ID", 3),
    ]
    .iter()
    .map(|(id, name, n)| {
        composer(
            id,
            &composer_json(id, name, 1_757_000_000_000 + n, 1_757_000_000_000 + n, &[]),
            Stored::Text,
        )
    })
    .collect();
    fixture.write_ide_db(Journal::Delete, &rows);
    fixture
}

#[test]
fn ambiguous_prefix_lists_every_candidate_newest_first() {
    let sessions = ambiguous_fixture().load().summaries;
    let err = find_session(&sessions, "ABCD").unwrap_err();
    let Error::AmbiguousId { query, matches } = &err else {
        panic!("expected an ambiguous ID, got {err:?}");
    };
    assert_eq!(query, "ABCD");
    assert_eq!(
        matches,
        &[
            "abcd2222-0000-4000-8000-000000000002  ide    Second abcd",
            "abcd1111-0000-4000-8000-000000000001  ide    First abcd",
        ]
    );
    assert_eq!(
        err.to_string(),
        r#"session ID prefix "ABCD" is ambiguous (2 matches)"#
    );
    assert_eq!(err.hints().last().unwrap(), "use more characters of the ID");

    // Case-insensitive prefixes reach upper-case IDs too.
    assert!(matches!(
        find_session(&sessions, "abc"),
        Err(Error::AmbiguousId { matches, .. }) if matches.len() == 3
    ));
    assert_eq!(
        find_session(&sessions, "abce").unwrap().title,
        "Upper-case ID"
    );
    assert_eq!(
        find_session(&sessions, "abcd1").unwrap().title,
        "First abcd"
    );
}

#[test]
fn filter_workspace_matches_paths_and_hashes() {
    let sessions = standard().load().summaries;
    let found = |workspace: &str| ids_of(&filter_workspace(&sessions, workspace));
    assert_eq!(found(PROJECT_X), [AGENT_ID]);
    assert_eq!(found("project-y"), [SHARED_ID]);
    assert_eq!(found(&workspace_md5(PROJECT_Y)), [SHARED_ID, STORE_ONLY_ID]);
    assert!(found("/nowhere").is_empty());
}

fn ids_of<'a>(sessions: &[&'a SessionSummary]) -> Vec<&'a str> {
    sessions.iter().map(|s| s.id.as_str()).collect()
}

fn exported(session: &Session, format: Format) -> String {
    let mut out = Vec::new();
    export::export_session(session, format, &mut out).unwrap();
    String::from_utf8(out).unwrap()
}

#[test]
fn every_export_format_round_trips_the_session() {
    let sessions = standard().load().sessions;
    for id in [AGENT_ID, IDE_TEXT_ID, IDE_UNTITLED_ID] {
        let session = get(&sessions, id);
        let expected = serde_json::to_value(session).unwrap();

        let json = exported(session, Format::Json);
        assert!(json.ends_with("}\n"));
        let parsed: Session = serde_json::from_str(&json).unwrap();
        assert_eq!(serde_json::to_value(&parsed).unwrap(), expected);

        let yaml = exported(session, Format::Yaml);
        let parsed: Session = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(serde_json::to_value(&parsed).unwrap(), expected);

        let jsonl = exported(session, Format::Jsonl);
        let messages: Vec<Message> = jsonl
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(
            serde_json::to_value(&messages).unwrap(),
            expected["messages"]
        );
        assert_eq!(jsonl.lines().count(), session.messages.len());

        let md = exported(session, Format::Md);
        assert!(md.starts_with(&format!("# {}\n\n- **ID:** `{id}`\n", session.title)));
        assert!(md.contains(&format!("- **Messages:** {}\n", session.messages.len())));
        for message in &session.messages {
            assert!(md.contains(&message.content), "{id}: {}", message.content);
        }
    }
}

#[test]
fn export_json_keeps_raw_times_and_omits_missing_fields() {
    let sessions = standard().load().sessions;
    let value: serde_json::Value =
        serde_json::from_str(&exported(get(&sessions, TRANSCRIPT_ONLY_ID), Format::Json)).unwrap();
    let keys: Vec<&str> = value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(keys, ["id", "title", "source", "messages"]);

    let value: serde_json::Value =
        serde_json::from_str(&exported(get(&sessions, AGENT_ID), Format::Json)).unwrap();
    assert_eq!(value["created_at_ms"], json!(1_757_000_000_000_i64));
    assert_eq!(
        value["messages"][2],
        json!({"role": "user", "content": "Keep it read-only."})
    );
}

#[test]
fn export_paths_use_the_session_id_and_format_extension() {
    let sessions = standard().load().sessions;
    let dir = std::path::Path::new("out");
    for (format, ext) in [
        (Format::Md, "md"),
        (Format::Json, "json"),
        (Format::Jsonl, "jsonl"),
        (Format::Yaml, "yaml"),
    ] {
        assert_eq!(
            export::export_path(dir, get(&sessions, AGENT_ID), format),
            dir.join(format!("{AGENT_ID}.{ext}"))
        );
    }
}

#[test]
fn the_most_complete_duplicate_transcript_wins() {
    let short = [
        plain_message("user", "hello"),
        plain_message("assistant", "first reply"),
    ];
    let mut long = short.to_vec();
    long.push(plain_message("assistant", "later reply"));

    // Either storage layout may hold the complete copy.
    for (nested, flat) in [(&short[..], &long[..]), (&long[..], &short[..])] {
        let fixture = Fixture::new();
        fixture.write_transcript("project-a", "session-1", Layout::Nested, nested);
        fixture.write_transcript("project-b", "session-1", Layout::Flat, flat);
        let sessions = fixture.load().sessions;
        assert_eq!(ids(&sessions), ["session-1"]);
        assert_eq!(
            contents(&sessions[0]),
            ["hello", "first reply", "later reply"]
        );
    }
}

#[test]
fn agent_problems_are_warnings_and_the_rest_loads() {
    let fixture = Fixture::new();
    write_standard_agent(&fixture);
    fixture.write_meta_json(PROJECT_X, "broken-meta", &json!({"title": "kept"}));
    fs::write(
        fixture
            .session_dir(PROJECT_X, "broken-meta")
            .join("meta.json"),
        "{not json",
    )
    .unwrap();
    write(
        &fixture
            .session_dir(PROJECT_X, "broken-store")
            .join("store.db"),
        "not a sqlite database, just some text",
    );

    let loaded = fixture.load();
    assert_eq!(loaded.sessions.len(), 6);
    assert_eq!(get(&loaded.sessions, "broken-meta").title, "broken-meta");
    assert_eq!(loaded.warnings.len(), 2, "{:?}", loaded.warnings);
    assert!(loaded.warnings[0].starts_with("ignored unreadable meta.json "));
    assert!(loaded.warnings[0].contains("broken-meta"));
    assert!(loaded.warnings[1].starts_with("ignored unreadable store.db "));
    assert!(loaded.warnings[1].contains("broken-store"));
}

#[test]
fn transcripts_whose_roles_were_renamed_are_an_unrecognized_format() {
    let fixture = Fixture::new();
    for id in ["a", "b"] {
        fixture.write_transcript(
            "Users-demo-project-x",
            id,
            Layout::Nested,
            &[plain_message("human", "hello"), plain_message("ai", "hi")],
        );
    }
    let err = fixture.load_source(Some(Source::Agent)).unwrap_err();
    assert!(
        matches!(
            err,
            Error::SchemaMismatch {
                store: Source::Agent,
                ..
            }
        ),
        "{err}"
    );
    assert!(
        err.to_string()
            .contains(": none of its 2 transcripts has a readable message.")
    );
}

#[test]
fn text_cut_inside_an_emoji_costs_only_that_character() {
    // JavaScript writes a string cut inside an emoji with a lone surrogate.
    let cut = format!("{}ud83d", '\\');
    let fixture = Fixture::new();
    let row = |key: &str, json: String| (key.to_string(), rusqlite::types::Value::Text(json));
    fixture.write_ide_db(
        Journal::Delete,
        &[
            row(
                "composerData:cut-ide",
                format!(
                    r#"{{"composerId":"cut-ide","name":"Fix the build {cut}",
                        "fullConversationHeadersOnly":[{{"bubbleId":"b1","type":1}},{{"bubbleId":"b2","type":2}}]}}"#
                ),
            ),
            row("bubbleId:cut-ide:b1", format!(r#"{{"type":1,"text":"Done {cut}"}}"#)),
            // Rich text is JSON inside a JSON string, with the cut inside.
            row(
                "bubbleId:cut-ide:b2",
                format!(
                    r#"{{"type":2,"text":"","richText":"{{\"root\":{{\"children\":[{{\"text\":\"Rich {}\"}}]}}}}"}}"#,
                    cut.replace('\\', "\\\\")
                ),
            ),
        ],
    );
    write(
        &fixture.session_dir(PROJECT_X, "cut-meta").join("meta.json"),
        &format!(r#"{{"title":"Plan {cut}"}}"#),
    );
    // A store.db `meta` value as plain JSON, and hex-encoded.
    let store_meta = |id: &str, value: &str| {
        write_sql_db(
            &fixture.session_dir(PROJECT_X, id).join("store.db"),
            &format!(
                "CREATE TABLE meta (key TEXT PRIMARY KEY, value BLOB);
                 INSERT INTO meta VALUES ('0', '{value}');"
            ),
        );
    };
    store_meta("cut-store", &format!(r#"{{"name":"Named {cut}"}}"#));
    let hex: String = format!(r#"{{"name":"Hex {cut}"}}"#)
        .bytes()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    store_meta("cut-hex", &hex);
    write(
        &fixture
            .projects_dir()
            .join("Users-demo-project-x")
            .join("agent-transcripts")
            .join("cut-agent.jsonl"),
        &format!(
            "{}\n{{\"role\":\"assistant\",\"message\":{{\"content\":\"Sure {cut}\"}}}}\n",
            plain_message("user", "hi")
        ),
    );

    let loaded = fixture.load();
    assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
    assert!(loaded.notices.is_empty(), "{:?}", loaded.notices);
    let ide = get(&loaded.sessions, "cut-ide");
    assert_eq!(ide.title, "Fix the build \u{fffd}");
    assert_eq!(contents(ide), ["Done \u{fffd}", "Rich \u{fffd}"]);
    for (id, title) in [
        ("cut-meta", "Plan \u{fffd}"),
        ("cut-store", "Named \u{fffd}"),
        ("cut-hex", "Hex \u{fffd}"),
    ] {
        assert_eq!(get(&loaded.sessions, id).title, title);
    }
    let agent = get(&loaded.sessions, "cut-agent");
    assert_eq!(contents(agent), ["hi", "Sure \u{fffd}"]);
}

#[test]
fn empty_and_plain_json_store_dbs_load_without_warnings() {
    let fixture = Fixture::new();
    let meta = json!({"name": "Plain JSON", "lastUsedModel": "gpt-5", "createdAt": 1});
    let text = fixture.write_store_db(PROJECT_X, "plain-text", &meta, Stored::Text);
    fixture.write_store_db(PROJECT_X, "plain-blob", &meta, Stored::Blob);
    // A session that was never used: an empty file, no tables, or no meta row.
    write(
        &fixture
            .session_dir(PROJECT_X, "zero-bytes")
            .join("store.db"),
        "",
    );
    let no_tables = fixture.session_dir(PROJECT_X, "no-tables").join("store.db");
    write_sql_db(
        &no_tables,
        "PRAGMA journal_mode = wal; PRAGMA user_version = 1;",
    );
    write_sql_db(
        &fixture.session_dir(PROJECT_X, "no-row").join("store.db"),
        "CREATE TABLE meta (key TEXT PRIMARY KEY, value BLOB);",
    );
    assert!(fs::metadata(&no_tables).unwrap().len() > 0);
    assert!(fs::metadata(&text).unwrap().len() > 0);

    let loaded = fixture.load();
    assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
    assert!(loaded.notices.is_empty(), "{:?}", loaded.notices);
    for id in ["plain-text", "plain-blob"] {
        let session = get(&loaded.sessions, id);
        assert_eq!(session.title, "Plain JSON");
        assert_eq!(session.model.as_deref(), Some("gpt-5"));
    }
    for id in ["zero-bytes", "no-tables", "no-row"] {
        let session = get(&loaded.sessions, id);
        assert_eq!(
            (session.title.as_str(), session.model.as_deref()),
            (id, None)
        );
    }
}

#[test]
fn load_options_default_to_both_stores() {
    let fixture = standard();
    let both = load_sessions(&fixture.paths(), &LoadOptions::default()).unwrap();
    assert_eq!(both.sessions.len(), STANDARD_IDS.len());
}

#[test]
fn updated_since_lists_only_recent_sessions_and_keeps_every_id() {
    let fixture = standard();
    let load = |updated_since, limit| {
        load_sessions(
            &fixture.paths(),
            &LoadOptions {
                updated_since,
                limit,
                ..Default::default()
            },
        )
        .unwrap()
    };
    let listed = |loaded: &cursor_session::Loaded| -> Vec<String> {
        loaded.sessions.iter().map(|s| s.id.clone()).collect()
    };
    // By the time the UPDATED column shows: IDE_UNTITLED_ID was only created.
    let recent = load(Some(1_757_250_000_000), None);
    assert_eq!(listed(&recent), [SHARED_ID, IDE_BLOB_ID, IDE_UNTITLED_ID]);
    assert_eq!(recent.ids, STANDARD_IDS);
    let counts: Vec<usize> = recent.sessions.iter().map(|s| s.message_count).collect();
    assert_eq!(counts, [2, 2, 0]);
    // The limit applies to the sessions kept.
    let limited = load(Some(1_757_250_000_000), Some(2));
    assert_eq!(listed(&limited), [SHARED_ID, IDE_BLOB_ID]);
    assert_eq!(limited.ids, STANDARD_IDS);
    // A session without any time is never recent.
    let all = load(Some(i64::MIN), None);
    assert_eq!(listed(&all), &STANDARD_IDS[..6]);
    assert!(load(Some(i64::MAX), None).sessions.is_empty());
}

#[test]
fn visiting_messages_gives_the_messages_loaded() {
    let fixture = standard();
    for session in fixture.load().sessions {
        let mut visited = Vec::new();
        cursor_session::visit_messages(&session.summary, &mut |message| visited.push(message))
            .unwrap();
        let contents = |messages: &[Message]| -> Vec<(String, String)> {
            messages
                .iter()
                .map(|m| (m.role.clone(), m.content.clone()))
                .collect()
        };
        assert_eq!(
            contents(&visited),
            contents(&session.messages),
            "{}",
            session.id
        );
    }
}

#[test]
fn search_session_scores_the_messages_show_lists() {
    use cursor_session::search::parse_query;
    let fixture = standard();
    let query = parse_query("PLAN here").unwrap();
    let mut found = Vec::new();
    for session in fixture.load().sessions {
        let Some(hit) = cursor_session::search_session(&session.summary, &query, 60).unwrap()
        else {
            continue;
        };
        // The snippet is of the message `show --json` lists at its index.
        let best = &session.messages[hit.score.best_message];
        assert_eq!(hit.snippet.role, best.role);
        found.push((
            session.id.clone(),
            hit.score.all_in_one,
            hit.score.matching_messages,
            hit.snippet.text,
        ));
    }
    assert_eq!(
        found,
        [
            (
                IDE_TEXT_ID.to_string(),
                true,
                1,
                "Here is a plan.".to_string()
            ),
            (
                AGENT_ID.to_string(),
                true,
                1,
                "Here is the plan: 1. Model traces as facts. 2. Expose metrics.".to_string()
            ),
        ]
    );
    // A term in no message: no hit, also where the other terms are.
    let missing = parse_query("plan nowhere-to-be-found").unwrap();
    for session in fixture.load().sessions {
        assert!(
            cursor_session::search_session(&session.summary, &missing, 60)
                .unwrap()
                .is_none()
        );
    }
}

#[test]
fn searching_all_sessions_finds_what_searching_each_finds() {
    use cursor_session::search::parse_query;
    let fixture = standard();
    let loaded = load_sessions(&fixture.paths(), &LoadOptions::default()).unwrap();
    for text in ["PLAN here", "question", "the", "nowhere-to-be-found"] {
        let query = parse_query(text).unwrap();
        let mut each: Vec<(String, usize, String)> = loaded
            .sessions
            .iter()
            .filter_map(|summary| cursor_session::search_session(summary, &query, 60).unwrap())
            .map(|hit| (hit.session.id, hit.score.best_message, hit.snippet.text))
            .collect();
        let mut all: Vec<(String, usize, String)> =
            cursor_session::search_sessions(&loaded.sessions, &query, 60)
                .unwrap()
                .into_iter()
                .map(|hit| (hit.session.id, hit.score.best_message, hit.snippet.text))
                .collect();
        each.sort();
        all.sort();
        assert_eq!(all, each, "{text}");
    }
}
