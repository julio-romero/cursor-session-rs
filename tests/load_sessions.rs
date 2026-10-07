use std::path::PathBuf;

use cursor_session::detect::{StoragePaths, workspace_md5};
use cursor_session::export::{self, Format};
use cursor_session::{LoadOptions, find_session, load_sessions};
use rusqlite::Connection;

fn fixture_home() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/home")
}

#[test]
fn workspace_hash_matches_live_path() {
    assert_eq!(
        workspace_md5("/Users/manuel.romero"),
        "a08c4602b56d190715ccec0647aa6db9"
    );
}

#[test]
fn loads_agent_session_from_meta_json_and_transcript() {
    let paths = StoragePaths::from_home(&fixture_home());
    let sessions = load_sessions(&paths, &LoadOptions::default())
        .expect("load")
        .sessions;
    let session = find_session(&sessions, "f4eea6d2-d2d3-41ad-b290-824445295a15")
        .expect("session should be found");

    assert_eq!(session.title, "Langfuse Semantic Layer");
    assert_eq!(session.source.as_str(), "agent");
    assert_eq!(session.workspace.as_deref(), Some("/Users/manuel.romero"));
    assert_eq!(session.messages.len(), 3);
    assert_eq!(session.messages[0].role, "user");
    assert_eq!(session.messages[0].content, "hello from a fixture");
    assert!(
        session.messages[0]
            .timestamp
            .as_deref()
            .unwrap()
            .contains("Sep 8, 2026")
    );
    assert_eq!(session.messages[1].content, "You want a design, not code.");
    assert_eq!(
        session.messages[2].content,
        "Here is the semantic layer plan."
    );
}

#[test]
fn prefix_lookup_is_unique() {
    let paths = StoragePaths::from_home(&fixture_home());
    let sessions = load_sessions(&paths, &LoadOptions::default())
        .unwrap()
        .sessions;
    assert!(find_session(&sessions, "f4eea6d2").is_ok());
}

#[test]
fn loads_ide_composer_from_sqlite() {
    let dir = std::env::temp_dir().join(format!("cursor-session-ide-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let db_path = dir.join("state.vscdb");
    let _ = std::fs::remove_file(&db_path);

    let conn = Connection::open(&db_path).unwrap();
    conn.execute(
        "CREATE TABLE cursorDiskKV (key TEXT PRIMARY KEY, value TEXT)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO cursorDiskKV (key, value) VALUES (?1, ?2)",
        [
            "composerData:composer-1",
            r#"{"composerId":"composer-1","name":"Dynamic table proposal","createdAt":1700000000000,"lastUpdatedAt":1700000500000,"fullConversationHeadersOnly":[{"bubbleId":"bubble-1","type":1},{"bubbleId":"bubble-2","type":2}]}"#,
        ],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO cursorDiskKV (key, value) VALUES (?1, ?2)",
        [
            "bubbleId:composer-1:bubble-1",
            r#"{"bubbleId":"bubble-1","type":1,"text":"replace this view"}"#,
        ],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO cursorDiskKV (key, value) VALUES (?1, ?2)",
        [
            "bubbleId:composer-1:bubble-2",
            r#"{"bubbleId":"bubble-2","type":2,"text":"","richText":"{\"root\":{\"children\":[{\"text\":\"here is a plan\"}]}}"}"#,
        ],
    )
    .unwrap();
    drop(conn);

    let sessions = cursor_session::ide::load_from_db(&db_path, &mut Vec::new()).unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].id, "composer-1");
    assert_eq!(sessions[0].title, "Dynamic table proposal");
    assert_eq!(sessions[0].source.as_str(), "ide");
    assert_eq!(sessions[0].messages.len(), 2);
    assert_eq!(sessions[0].messages[0].content, "replace this view");
    assert_eq!(sessions[0].messages[1].content, "here is a plan");
}

#[test]
fn exports_markdown_and_jsonl() {
    let paths = StoragePaths::from_home(&fixture_home());
    let sessions = load_sessions(&paths, &LoadOptions::default())
        .unwrap()
        .sessions;
    let session = find_session(&sessions, "f4eea6d2-d2d3-41ad-b290-824445295a15").unwrap();

    let mut md = Vec::new();
    export::export_session(session, Format::Md, &mut md).unwrap();
    let md = String::from_utf8(md).unwrap();
    assert!(md.contains("# Langfuse Semantic Layer"));
    assert!(md.contains("hello from a fixture"));

    let mut jsonl = Vec::new();
    export::export_session(session, Format::Jsonl, &mut jsonl).unwrap();
    let jsonl = String::from_utf8(jsonl).unwrap();
    assert!(jsonl.contains("\"role\":\"user\""));
    assert!(jsonl.lines().count() >= 3);
}

#[test]
fn duplicate_project_transcripts_keep_the_complete_conversation() {
    let root = std::env::temp_dir().join(format!(
        "cursor-session-duplicate-transcripts-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let nested = root.join("project-a/agent-transcripts/session-1");
    let flat = root.join("project-b/agent-transcripts");
    std::fs::create_dir_all(&nested).unwrap();
    std::fs::create_dir_all(&flat).unwrap();
    let short = concat!(
        "{\"role\":\"user\",\"message\":{\"content\":\"hello\"}}\n",
        "{\"role\":\"assistant\",\"message\":{\"content\":\"first reply\"}}\n",
    );
    let long = format!(
        "{short}{}\n",
        r#"{"role":"assistant","message":{"content":"later reply"}}"#
    );
    let paths = StoragePaths {
        projects_dir: Some(root.clone()),
        ..Default::default()
    };

    // Either storage layout may hold the complete copy.
    for (nested_text, flat_text) in [(short, long.as_str()), (long.as_str(), short)] {
        std::fs::write(nested.join("session-1.jsonl"), nested_text).unwrap();
        std::fs::write(flat.join("session-1.jsonl"), flat_text).unwrap();
        let sessions = load_sessions(&paths, &LoadOptions::default())
            .unwrap()
            .sessions;
        assert_eq!(sessions.len(), 1);
        let session = find_session(&sessions, "session-1").unwrap();
        assert_eq!(session.messages.len(), 3);
        assert_eq!(session.messages[0].content, "hello");
        assert_eq!(session.messages[1].content, "first reply");
        assert_eq!(session.messages[2].content, "later reply");

        let mut exported = Vec::new();
        export::export_session(session, Format::Jsonl, &mut exported).unwrap();
        let exported = String::from_utf8(exported).unwrap();
        assert_eq!(exported.lines().count(), 3);
        assert!(exported.contains("later reply"));
    }
    std::fs::remove_dir_all(root).unwrap();
}
