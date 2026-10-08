//! Snapshots of the rendered output for the standard fixture. Times are fixed
//! and rendered in UTC, and no snapshot holds a path, so they are the same on
//! every OS.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::*;
use cursor_session::export::{self, Format};
use cursor_session::model::Session;
use cursor_session::ui;
use unicode_width::UnicodeWidthStr;

const WIDTHS: [usize; 7] = [40, 60, 80, 100, 120, 160, 200];

fn sessions() -> Vec<Session> {
    standard().load().sessions
}

fn session(id: &str) -> Session {
    sessions().into_iter().find(|s| s.id == id).unwrap()
}

#[test]
fn list_table_at_each_width() {
    let sessions = sessions();
    for width in WIDTHS {
        let rendered = ui::render_list(&sessions, false, Some(width));
        assert!(!rendered.contains('\u{1b}'));
        let table = rendered
            .lines()
            .skip(2)
            .take_while(|line| line.starts_with(['┌', '│', '╞', '└']));
        assert!(
            table.clone().all(|line| line.width() <= width),
            "{rendered}"
        );
        if width >= 80 {
            // The header and every session take one line each, the wide title too.
            let rows = table.filter(|line| line.starts_with('│'));
            assert_eq!(rows.count(), sessions.len() + 1, "{rendered}");
        }
        insta::assert_snapshot!(format!("list_table_{width:03}"), rendered);
    }
}

#[test]
fn list_table_with_color() {
    // As the binary does, so NO_COLOR in the test environment changes nothing.
    crossterm::style::force_color_output(true);
    let rendered = ui::render_list(&sessions(), true, Some(100));
    assert!(rendered.contains('\u{1b}'));
    insta::assert_snapshot!("list_table_100_color", rendered);
}

#[test]
fn list_plain_layout() {
    let sessions = sessions();
    insta::assert_snapshot!("list_plain", ui::render_list(&sessions, false, None));
    insta::assert_snapshot!("list_plain_color", ui::render_list(&sessions, true, None));
}

#[test]
fn show_every_message() {
    for (name, id) in [
        ("show_agent", AGENT_ID),
        ("show_ide", IDE_TEXT_ID),
        ("show_transcript_only", TRANSCRIPT_ONLY_ID),
        ("show_untitled", IDE_UNTITLED_ID),
    ] {
        let session = session(id);
        let rendered = ui::render_show(&session, &session.messages, None, false);
        assert!(!rendered.contains("omitted"));
        insta::assert_snapshot!(name, rendered);
    }
}

#[test]
fn show_with_hidden_messages() {
    let session = session(AGENT_ID);
    let (messages, hidden) = ui::select_messages(&session.messages, true, Some(2), false);
    assert_eq!(hidden, Some(2));
    insta::assert_snapshot!(
        "show_agent_hidden",
        ui::render_show(&session, messages, hidden, false)
    );
    insta::assert_snapshot!(
        "show_agent_hidden_color",
        ui::render_show(&session, messages, hidden, true)
    );
}

#[test]
fn export_formats() {
    for (name, id) in [("agent", AGENT_ID), ("ide", IDE_TEXT_ID)] {
        let session = session(id);
        for format in [Format::Md, Format::Json, Format::Jsonl, Format::Yaml] {
            let mut out = Vec::new();
            export::export_session(&session, format, &mut out).unwrap();
            insta::assert_snapshot!(
                format!("export_{name}_{}", format.extension()),
                String::from_utf8(out).unwrap()
            );
        }
    }
}

fn cli_stdout(fixture: &Fixture, args: &[&str]) -> String {
    let output = fixture.cmd().args(args).output().unwrap();
    assert!(output.status.success(), "{}", stderr(&output));
    let out = stdout(&output);
    assert!(out.ends_with("\n"));
    out
}

#[test]
fn cli_json_output() {
    let fixture = standard();
    insta::assert_snapshot!("cli_list_json", cli_stdout(&fixture, &["list", "--json"]));
    insta::assert_snapshot!(
        "cli_show_agent_json",
        cli_stdout(&fixture, &["show", "f4eea6d2", "--json"])
    );
    insta::assert_snapshot!(
        "cli_show_ide_json",
        cli_stdout(&fixture, &["show", "c0ffee00", "--json"])
    );
}
