//! The completion scripts and man pages committed in `completions/` and
//! `man/`, which release archives carry, are what the binary prints now.
//!
//! After changing the command line, regenerate them with
//!
//!     CURSOR_SESSION_REGENERATE=1 cargo test --locked --test generated
//!
//! which rewrites every file from the binary and removes the man pages of
//! commands that are gone.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::*;

const REGENERATE: &str = "CURSOR_SESSION_REGENERATE";

/// Each shell and the file its script is committed as, named as the shell
/// looks for it.
const SCRIPTS: [(&str, &str); 3] = [
    ("bash", "cursor-session.bash"),
    ("zsh", "_cursor-session"),
    ("fish", "cursor-session.fish"),
];

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn generate(fixture: &Fixture, args: &[&str]) -> Vec<u8> {
    let output = fixture.command().args(args).output().unwrap();
    assert!(
        output.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(stderr(&output), "", "{args:?}");
    output.stdout
}

/// The commands with a page of their own, as the overview page lists them,
/// e.g. `cursor\-session\-list(1)`.
fn documented_commands(overview: &str) -> Vec<String> {
    let commands: Vec<String> = overview
        .lines()
        .filter_map(|line| {
            line.strip_prefix("cursor\\-session\\-")?
                .strip_suffix("(1)")
        })
        .map(|name| name.replace("\\-", "-"))
        .collect();
    assert!(commands.contains(&"list".to_string()), "{overview}");
    commands
}

/// Each committed file and what the binary prints in its place.
fn expected_files(fixture: &Fixture) -> Vec<(PathBuf, Vec<u8>)> {
    let mut files = Vec::new();
    for (shell, name) in SCRIPTS {
        let script = generate(fixture, &["completions", shell]);
        files.push((root().join("completions").join(name), script));
    }
    let overview = generate(fixture, &["man"]);
    let commands = documented_commands(&String::from_utf8(overview.clone()).unwrap());
    files.push((root().join("man/cursor-session.1"), overview));
    for command in commands {
        let page = generate(fixture, &["man", &command]);
        files.push((root().join(format!("man/cursor-session-{command}.1")), page));
    }
    files
}

/// `path` relative to the repository, as the failure messages name it.
fn shown(path: &Path) -> String {
    path.strip_prefix(root())
        .unwrap_or(path)
        .display()
        .to_string()
}

/// The files in `dir`, but not hidden ones such as `.DS_Store`.
fn files_in(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap())
        .filter(|entry| !entry.file_name().to_string_lossy().starts_with('.'))
        .map(|entry| entry.path())
        .collect();
    files.sort();
    files
}

#[test]
fn committed_completions_and_man_pages_are_current() {
    let fixture = Fixture::new();
    let expected = expected_files(&fixture);
    let regenerate = std::env::var_os(REGENERATE).is_some_and(|value| value == "1");
    if regenerate {
        for dir in ["completions", "man"] {
            let dir = root().join(dir);
            fs::create_dir_all(&dir).unwrap();
            for stale in files_in(&dir) {
                fs::remove_file(stale).unwrap();
            }
        }
        for (path, contents) in &expected {
            fs::write(path, contents).unwrap();
        }
    }

    let how = format!("regenerate with: {REGENERATE}=1 cargo test --locked --test generated");
    for (path, contents) in &expected {
        let committed = fs::read(path)
            .unwrap_or_else(|error| panic!("{} is missing ({error}); {how}", shown(path)));
        assert!(
            committed == *contents,
            "{} differs from what the binary prints; {how}",
            shown(path)
        );
    }
    for dir in ["completions", "man"] {
        for path in files_in(&root().join(dir)) {
            assert!(
                expected.iter().any(|(expected, _)| *expected == path),
                "{} belongs to no command; {how}",
                shown(&path)
            );
        }
    }
}

#[test]
fn pages_are_the_same_on_every_run() {
    let fixture = Fixture::new();
    let page = generate(&fixture, &["man"]);
    assert_eq!(page, generate(&fixture, &["man"]));
    let text = String::from_utf8(page).unwrap();
    let title = text.lines().find(|line| line.starts_with(".TH ")).unwrap();
    assert_eq!(
        title,
        format!(
            ".TH cursor-session 1  \"cursor-session {}\" \"User Commands\"",
            env!("CARGO_PKG_VERSION")
        ),
        "no date"
    );
}
