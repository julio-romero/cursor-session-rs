//! Listing, searching or exporting a large history holds no more than one
//! session's messages: peak memory stays far below the size of the history.
//!
//! On Linux a spawned child starts out with the peak memory of the process
//! that spawned it, so this test writes the history a row at a time and checks
//! that it stays small itself.
#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::fs;
use std::io::{BufWriter, Write};
use std::mem::MaybeUninit;
use std::process::Stdio;

use common::*;

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

/// About 2 KB of what a command printed.
fn command_output(n: usize) -> String {
    format!("test retry::backoff_{n} ... ok\n").repeat(64)
}

fn agent_id(n: usize) -> String {
    format!("{n:08x}-0000-4000-8000-{n:012x}")
}

fn chat_id(n: usize) -> String {
    format!("c{n:07x}-1111-4222-8333-{n:012x}")
}

/// Writes the history a line and a row at a time, so that this process
/// stays small.
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
        let path = fixture
            .projects_dir()
            .join("Users-demo-big")
            .join("agent-transcripts")
            .join(&id)
            .join(format!("{id}.jsonl"));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut file = BufWriter::new(fs::File::create(path).unwrap());
        let (mut written, mut line) = (0, 0);
        while written < AGENT_BYTES / AGENT_SESSIONS {
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
            let entry = entry.to_string();
            writeln!(file, "{entry}").unwrap();
            written += entry.len() + 1;
            line += 1;
        }
        file.flush().unwrap();
    }

    let conn = create_kv_db(&fixture.ide_db_path(), Journal::Wal);
    let tx = conn.unchecked_transaction().unwrap();
    {
        let mut insert = tx
            .prepare("INSERT INTO cursorDiskKV (key, value) VALUES (?1, ?2)")
            .unwrap();
        let mut put = |(key, value): (String, rusqlite::types::Value)| {
            insert.execute(rusqlite::params![key, value]).unwrap();
        };
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
                // Every other reply runs a command, whose output is kept with it.
                let json = if headers.len() % 4 == 1 {
                    tool_bubble(
                        &bubble_id,
                        &text,
                        "run_terminal_cmd",
                        &serde_json::json!({"command": "cargo test"}),
                        &serde_json::json!({"output": command_output(headers.len()), "exitCodeV2": 0}),
                    )
                } else {
                    text_bubble(&bubble_id, kind, &text)
                };
                bytes += json.to_string().len();
                put(bubble(&id, &bubble_id, &json, Stored::Text));
                headers.push((bubble_id, kind));
            }
            let created = 1_740_000_000_000_i64 + n as i64 * 60_000;
            let listed: Vec<(&str, i64)> = headers.iter().map(|(b, k)| (b.as_str(), *k)).collect();
            let json = composer_json(&id, &format!("Chat {n}"), created, created + 5_000, &listed);
            put(composer(&id, &json, Stored::Text));
        }
    }
    tx.commit().unwrap();
}

/// Bytes from a `ru_maxrss`: kilobytes on Linux, bytes on macOS.
fn rss_bytes(max_rss: libc::c_long) -> usize {
    let max_rss = usize::try_from(max_rss).unwrap();
    if cfg!(target_os = "macos") {
        max_rss
    } else {
        max_rss * 1024
    }
}

/// The peak resident memory of this process's own address space, in bytes,
/// which is what a child it spawns starts out with on Linux. There
/// `ru_maxrss` also holds the peak this process started out with itself, from
/// cargo, so the kernel's own record of the address space is read instead.
fn own_peak_rss() -> usize {
    if let Ok(status) = fs::read_to_string("/proc/self/status") {
        let kb = status
            .lines()
            .find_map(|line| line.strip_prefix("VmHWM:"))
            .unwrap();
        return kb
            .trim()
            .trim_end_matches("kB")
            .trim()
            .parse::<usize>()
            .unwrap()
            * 1024;
    }
    let mut usage = MaybeUninit::<libc::rusage>::zeroed();
    // SAFETY: `getrusage` fills the struct it is given and reads nothing else.
    let usage = unsafe {
        assert_eq!(libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()), 0);
        usage.assume_init()
    };
    rss_bytes(usage.ru_maxrss)
}

/// Runs the binary with `args` and returns its peak resident memory, in
/// bytes. It must succeed.
// `wait4` reaps the child, to read its own resource usage.
#[allow(clippy::zombie_processes)]
fn command_peak_rss(fixture: &Fixture, args: &[&str]) -> usize {
    let child = fixture
        .command()
        .args(args)
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    let pid = libc::pid_t::try_from(child.id()).unwrap();
    let mut status = 0;
    let mut usage = MaybeUninit::<libc::rusage>::zeroed();
    // SAFETY: `wait4` waits for the child spawned above, which nothing else
    // waits for, and fills the status and struct it is given.
    let usage = unsafe {
        assert_eq!(libc::wait4(pid, &mut status, 0, usage.as_mut_ptr()), pid);
        usage.assume_init()
    };
    assert!(
        libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0,
        "{args:?} failed: {status}"
    );
    rss_bytes(usage.ru_maxrss)
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
    let commands: [&[&str]; 19] = [
        &["list"],
        &["list", "--json"],
        &["list", "--limit", "5"],
        &["list", "--since", "10000d"],
        &["list", "--since", "10000d", "--json", "--limit", "5"],
        &["show", &newest_agent, "--json"],
        &["show", &newest_chat],
        // Tool calls are read only for these, and only for this session.
        &[
            "show",
            &newest_agent,
            "--only",
            "user,assistant,tool",
            "--json",
        ],
        &["show", &newest_agent, "--only", "tool", "--short"],
        &[
            "show",
            &newest_chat,
            "--only",
            "assistant,tool",
            "--short",
            "--json",
        ],
        &["show", &newest_chat, "--only", "tool"],
        &["healthcheck"],
        &["completions", "bash"],
        &["man"],
        // Every message of every session is searched, one at a time.
        &["search", "retry", "JITTER"],
        &[
            "search",
            "\"backs off exponentially\"",
            "question",
            "--json",
        ],
        &[
            "search",
            "question",
            "--since",
            "10000d",
            "--context",
            "500",
        ],
        &["search", "reply", "-n", "3", "--source", "ide"],
        &[
            "export", "--since", "10000d", "--limit", "2", "--out", "exports",
        ],
    ];
    // A child starts out with this peak on Linux, so it must leave room.
    let own = own_peak_rss();
    assert!(
        own < PEAK_LIMIT / 2,
        "the test itself peaked at {} MB",
        own / MB
    );
    for args in commands {
        let peak = command_peak_rss(&fixture, args);
        assert!(
            peak < PEAK_LIMIT,
            "`{}` peaked at {} MB, over the {} MB limit, for {} MB of history",
            args.join(" "),
            peak / MB,
            PEAK_LIMIT / MB,
            history as usize / MB
        );
    }

    // Every session matched, each with its best message.
    let found = json(&ok(&fixture, &["search", "retry", "jitter", "--json"]));
    assert_eq!(found.as_array().unwrap().len(), AGENT_SESSIONS + IDE_CHATS);
    assert!(
        found
            .as_array()
            .unwrap()
            .iter()
            .all(|hit| hit["all_terms_in_one_message"] == true)
    );

    // Counted without holding the messages, the counts are the messages,
    // and the token estimates theirs.
    let listed = json(&ok(&fixture, &["list", "--json"]));
    for id in [&newest_chat, &newest_agent] {
        let show = json(&ok(&fixture, &["show", id, "--json"]));
        let summary = listed
            .as_array()
            .unwrap()
            .iter()
            .find(|session| session["id"] == id.as_str())
            .unwrap();
        let count = summary["message_count"].clone();
        assert_eq!(count, show["message_count"]);
        assert_eq!(
            show["messages"].as_array().unwrap().len() as u64,
            count.as_u64().unwrap()
        );
        let chars: usize = show["messages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["content"].as_str().unwrap().chars().count())
            .sum();
        assert_eq!(summary["token_estimate"], show["token_estimate"]);
        assert_eq!(
            summary["token_estimate"].as_u64().unwrap(),
            chars.div_ceil(4) as u64
        );
    }
    // Each assistant message of the transcript makes a tool call.
    let tools = json(&ok(
        &fixture,
        &["show", &newest_agent, "--only", "tool", "--json"],
    ));
    let tools = tools["messages"].as_array().unwrap();
    assert!(!tools.is_empty());
    assert!(
        tools
            .iter()
            .all(|m| m["content"] == r#"Read {"path":"src/lib.rs"}"#)
    );
    // Every other reply of the chat runs a command: its call, then what it
    // printed.
    let tools = json(&ok(
        &fixture,
        &["show", &newest_chat, "--only", "tool", "--all", "--json"],
    ));
    let tools = tools["messages"].as_array().unwrap();
    assert!(
        !tools.is_empty() && tools.len().is_multiple_of(2),
        "{}",
        tools.len()
    );
    for pair in tools.chunks(2) {
        assert_eq!(
            pair[0]["content"],
            r#"run_terminal_cmd {"command":"cargo test"}"#
        );
        let output = pair[1]["content"].as_str().unwrap();
        assert!(output.starts_with("test retry::backoff_"), "{output}");
        assert!(output.ends_with(" ... ok"), "{output}");
    }
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
