//! Listing a large history holds no session's messages: peak memory stays
//! far below the size of the history. Its own test binary, so that the peak of
//! its child processes is that of the commands run here.
#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::fmt::Write as _;
use std::fs;
use std::process::Stdio;

use common::*;
use rusqlite::types::Value as SqlValue;

const MB: usize = 1024 * 1024;
/// Synthetic history: transcripts and IDE messages, 96 MB in all.
const AGENT_BYTES: usize = 64 * MB;
const IDE_BYTES: usize = 32 * MB;
/// Peak resident memory allowed for any one command, a third of the history.
const PEAK_LIMIT: usize = 32 * MB;

const AGENT_SESSIONS: usize = 64;
const IDE_CHATS: usize = 32;

/// About 2 KB of assistant text.
fn reply(n: usize) -> String {
    format!("Reply {n}: the retry policy backs off exponentially, starting at 500 ms with jitter. ")
        .repeat(24)
}

fn agent_id(n: usize) -> String {
    format!("{n:08x}-0000-4000-8000-{n:012x}")
}

fn chat_id(n: usize) -> String {
    format!("c{n:07x}-1111-4222-8333-{n:012x}")
}

fn write_history(fixture: &Fixture) {
    let cwd = "/Users/demo/big";
    for n in 0..AGENT_SESSIONS {
        let id = agent_id(n);
        let created = 1_750_000_000_000_i64 + n as i64 * 60_000;
        fixture.write_meta_json(
            cwd,
            &id,
            &serde_json::json!({
                "title": format!("Session {n}"),
                "createdAtMs": created,
                "updatedAtMs": created + 5_000,
                "cwd": cwd,
            }),
        );
        let mut text = String::new();
        let mut line = 0;
        while text.len() < AGENT_BYTES / AGENT_SESSIONS {
            let entry = if line % 2 == 0 {
                serde_json::json!({"role": "user", "message": {"content": [
                    {"type": "text", "text": format!("<timestamp>Mon</timestamp>\n<user_query>\nquestion {line}\n</user_query>")}
                ]}})
            } else {
                serde_json::json!({"role": "assistant", "message": {"content": [
                    {"type": "text", "text": reply(line)},
                    {"type": "tool_use", "name": "Read", "input": {"path": "src/lib.rs"}}
                ]}})
            };
            writeln!(text, "{entry}").unwrap();
            line += 1;
        }
        let path = fixture
            .projects_dir()
            .join("Users-demo-big")
            .join("agent-transcripts")
            .join(&id)
            .join(format!("{id}.jsonl"));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    let mut rows = Vec::new();
    for n in 0..IDE_CHATS {
        let id = chat_id(n);
        let (mut headers, mut bytes) = (Vec::new(), 0);
        while bytes < IDE_BYTES / IDE_CHATS {
            let bubble_id = format!("b{:06}", headers.len());
            let kind = 1 + headers.len() as i64 % 2;
            let text = if kind == 1 {
                format!("question {}", headers.len())
            } else {
                reply(headers.len())
            };
            let json = text_bubble(&bubble_id, kind, &text);
            bytes += json.to_string().len();
            rows.push(bubble(&id, &bubble_id, &json, Stored::Text));
            headers.push((bubble_id, kind));
        }
        let created = 1_740_000_000_000_i64 + n as i64 * 60_000;
        let listed: Vec<(&str, i64)> = headers.iter().map(|(b, k)| (b.as_str(), *k)).collect();
        let json = composer_json(&id, &format!("Chat {n}"), created, created + 5_000, &listed);
        rows.push(composer(&id, &json, Stored::Text));
    }
    let rows: Vec<(String, SqlValue)> = rows;
    fixture.write_ide_db(Journal::Wal, &rows);
}

/// The largest peak resident memory, in bytes, of the child processes this
/// process has waited for.
fn children_peak_rss() -> usize {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::zeroed();
    // SAFETY: `getrusage` fills the struct it is given and reads nothing else.
    let usage = unsafe {
        assert_eq!(
            libc::getrusage(libc::RUSAGE_CHILDREN, usage.as_mut_ptr()),
            0
        );
        usage.assume_init()
    };
    let max_rss = usize::try_from(usage.ru_maxrss).unwrap();
    // Linux reports kilobytes, macOS bytes.
    if cfg!(target_os = "macos") {
        max_rss
    } else {
        max_rss * 1024
    }
}

#[test]
fn listing_a_large_history_stays_under_a_fixed_peak_memory() {
    let fixture = Fixture::new();
    write_history(&fixture);
    let history: u64 = [fixture.projects_dir(), fixture.ide_db_path()]
        .iter()
        .map(|path| dir_size(path))
        .sum();
    assert!(history as usize >= AGENT_BYTES + IDE_BYTES, "{history}");

    let newest_agent = agent_id(AGENT_SESSIONS - 1);
    let newest_chat = chat_id(IDE_CHATS - 1);
    let commands: [&[&str]; 6] = [
        &["list"],
        &["list", "--json"],
        &["list", "--limit", "5"],
        &["show", &newest_agent, "--json"],
        &["show", &newest_chat],
        &["healthcheck"],
    ];
    for args in commands {
        let status = fixture
            .command()
            .args(args)
            .stdout(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "{args:?}");
        let peak = children_peak_rss();
        assert!(
            peak < PEAK_LIMIT,
            "`{}` peaked at {} MB, over the {} MB limit, for {} MB of history",
            args.join(" "),
            peak / MB,
            PEAK_LIMIT / MB,
            history as usize / MB
        );
    }

    // Counted without holding the messages, the counts are the messages.
    let listed = json(&ok(&fixture, &["list", "--json"]));
    let show = json(&ok(&fixture, &["show", &newest_chat, "--json"]));
    let count = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|session| session["id"] == newest_chat.as_str())
        .unwrap()["message_count"]
        .clone();
    assert_eq!(count, show["message_count"]);
    assert_eq!(
        show["messages"].as_array().unwrap().len() as u64,
        count.as_u64().unwrap()
    );
}

fn ok(fixture: &Fixture, args: &[&str]) -> String {
    let output = fixture.command().args(args).output().unwrap();
    assert!(output.status.success(), "{args:?}");
    stdout(&output)
}

fn json(text: &str) -> serde_json::Value {
    serde_json::from_str(text).unwrap()
}

fn dir_size(path: &std::path::Path) -> u64 {
    let meta = fs::metadata(path).unwrap();
    if meta.is_file() {
        return meta.len();
    }
    fs::read_dir(path)
        .unwrap()
        .map(|entry| dir_size(&entry.unwrap().path()))
        .sum()
}
