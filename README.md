# cursor-session

[![CI](https://github.com/julio-romero/cursor-session-rs/actions/workflows/ci.yml/badge.svg?branch=master)](https://github.com/julio-romero/cursor-session-rs/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/cursor-session.svg)](https://crates.io/crates/cursor-session)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

**Find, read and export your Cursor chat history from the terminal.**

![cursor-session listing, showing and exporting Cursor chats](demo/demo.gif)

Cursor keeps every chat on your machine, split between a SQLite database for
the IDE and a folder per session for the Agent CLI. `cursor-session` reads both
and merges them into one list. Open any conversation in your terminal, export
it to Markdown, JSON, JSONL or YAML, or pipe `--json` into your own scripts.

It only reads. Every database is opened read-only, so it is safe to run while
Cursor is open.

## Features

- **Both sources, one list:** Cursor IDE chats and Cursor Agent CLI sessions,
  newest first.
- **Short IDs:** `show a71d` works with any unique prefix of a session ID.
- **Export:** Markdown, JSON, JSONL or YAML, one file per session, selected by
  ID, workspace, source or count.
- **Scriptable:** `--json` with a stable schema for `jq`, plain columns when
  piped, and a clean exit when `| head` closes the pipe.
- **Safe:** read-only, works while Cursor is running, and stops with a clear
  error when a Cursor update changes the storage format, rather than showing
  wrong data.
- **Cross-platform:** macOS, Linux and Windows.

## Install

**Homebrew** (macOS and Linux):

```sh
brew install julio-romero/tap/cursor-session
```

**Shell installer** (macOS and Linux):

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/julio-romero/cursor-session-rs/releases/latest/download/cursor-session-installer.sh | sh
```

**Cargo** (any platform, Rust 1.88 or newer):

```sh
cargo install cursor-session --locked
```

**Windows:** download `cursor-session-x86_64-pc-windows-msvc.zip` from the
[latest release](https://github.com/julio-romero/cursor-session-rs/releases/latest)
and put `cursor-session.exe` on your `PATH`, or use Cargo.

The Linux binaries need glibc 2.35 or newer; on older or musl-based systems,
use Cargo. Install locations, upgrading and uninstalling are in
[docs/install.md](docs/install.md).

## Quick start

```sh
cursor-session list                       # every session, newest first
cursor-session show a71d                  # one session, by ID or ID prefix
cursor-session export --session-id a71d   # write it to exports/<id>.md
cursor-session list --json | jq '.[0]'    # JSON for scripts
cursor-session healthcheck                # where it looked and what it found
```

## Usage

### List

In a terminal, `list` prints a table fitted to the window, as in the GIF above.
Piped or redirected, it prints plain columns with full IDs and titles, ready for
`grep`, `awk` and scripts:

```console
$ cursor-session list | head -n 7
Found 8 session(s)

ID                                    SOURCE   MSGS  UPDATED           TITLE
----------------------------------------------------------------------------------------------------
a71d0e58-2c39-4f7b-b6a4-19e8c3d5f027  agent       4  2026-10-05 14:12  Add retry with backoff to the webhook sender
e5b8c4d2-6a1f-4e93-8d27-5f0a9b3c1e46  ide         3  2026-10-04 16:48  Speed up the monthly revenue dashboard query
3f9c2a71-8b4e-4d6a-9e15-7c0b2d4f8a63  agent       3  2026-10-02 08:31  Fix flaky timezone test in invoice due dates
```

Narrow it down with `--source agent` or `--source ide`, and `--limit N`:

```sh
cursor-session list --source agent --limit 5
```

### Show

````console
$ cursor-session show e5b8 --limit 2
Speed up the monthly revenue dashboard query
id:        e5b8c4d2-6a1f-4e93-8d27-5f0a9b3c1e46
source:    ide
created:   2026-10-04 16:20 UTC
updated:   2026-10-04 16:48 UTC
messages:  3

1 earlier message(s) omitted. Use --limit N or --all to see more.

[assistant (2026-10-04 16:22 UTC)]
It scans every invoice and groups by a computed month. Add an index on `(status, issued_at)` and filter on a range so it can use it:

```sql
CREATE INDEX invoices_status_issued_at ON invoices (status, issued_at);
```

[user (2026-10-04 16:48 UTC)]
Down to 120 ms. Thanks!
````

In a terminal, `show` prints the last 20 messages; piped, it prints all of them.
`--limit N` prints the last N and `--all` prints everything.

### Export

```sh
cursor-session export                                      # every session as Markdown, into ./exports
cursor-session export --format json --session-id 3f9c --out sessions
cursor-session export --format jsonl --workspace ~/src/billing-api
```

Each session is written to `<out>/<session-id>.<ext>`. The formats are `md`
(the default), `json`, `jsonl` and `yaml`.

### JSON output

`list --json` prints an array of sessions in list order, and `show --json`
prints one session with its `messages`. Every key is always present, unknown
values are `null`, and times are RFC 3339 in UTC.

```console
$ cursor-session list --json --limit 1
[
  {
    "id": "a71d0e58-2c39-4f7b-b6a4-19e8c3d5f027",
    "title": "Add retry with backoff to the webhook sender",
    "source": "agent",
    "workspace": "/Users/dana/src/billing-api",
    "workspace_hash": "8a7c14142f6bbd963582518deca64fe4",
    "model": "claude-4.5-sonnet",
    "created_at": "2026-10-05T13:40:12Z",
    "updated_at": "2026-10-05T14:12:47Z",
    "message_count": 4
  }
]
```

With [jq](https://jqlang.org):

```console
$ cursor-session list --json | jq -r '.[] | select(.source == "ide") | .title'
Speed up the monthly revenue dashboard query
Explain the migration lock error
Add dark mode to the settings page
Review the OAuth callback handler for CSRF

$ cursor-session show a71d --json | jq -r '.messages[] | select(.role == "user") | .content'
Webhook deliveries fail for good on the first 503. Add retries with exponential backoff.
Make the attempt count configurable.
```

Every key and its type are listed in [docs/usage.md](docs/usage.md#json-output).

### Global flags

| Flag                            | Effect                                                              |
| ------------------------------- | ------------------------------------------------------------------- |
| `--storage <PATH>`              | Read only this location instead of detecting Cursor's data          |
| `-v`, `--verbose`               | Print the paths in use and anything skipped while loading to stderr |
| `--color <auto\|always\|never>` | By default, color only a terminal; `NO_COLOR` is respected          |

Every command and flag in detail: [docs/usage.md](docs/usage.md).

## Where it reads data from

| OS      | Agent CLI                                        | Cursor IDE                                                             |
| ------- | ------------------------------------------------ | ---------------------------------------------------------------------- |
| macOS   | `~/.cursor/chats`, `~/.cursor/projects`          | `~/Library/Application Support/Cursor/User/globalStorage/state.vscdb`  |
| Linux   | `~/.cursor/chats`, `~/.cursor/projects`          | `~/.config/Cursor/User/globalStorage/state.vscdb` (or `$XDG_CONFIG_HOME`) |
| Windows | `%USERPROFILE%\.cursor\chats`, `...\projects`    | `%APPDATA%\Cursor\User\globalStorage\state.vscdb`                      |

If your data lives elsewhere, point `--storage` at a home directory, a `.cursor`
directory or a `state.vscdb` file. Under WSL, `--storage /mnt/c/Users/<you>`
reads the Windows side.

Every database is opened with SQLite's read-only flag and `PRAGMA query_only`.
While Cursor runs, `cursor-session` reads alongside it like any other SQLite
reader. When Cursor is closed, it opens the database as immutable and creates
no files next to it. The details, the fallbacks and what is included are in
[docs/storage.md](docs/storage.md).

## Troubleshooting

- **No sessions, or not the ones you expect:** `cursor-session healthcheck`
  shows where it looked and what loaded.
- **A Cursor update changed the storage format:** commands stop with
  `unrecognized Cursor IDE storage format` instead of showing wrong data.
  `--source agent` still lists your Agent CLI sessions. Please
  [open an issue](https://github.com/julio-romero/cursor-session-rs/issues)
  with your Cursor version.
- **Something is missing:** `-v` prints a `warning:` line for each kind of data
  that was skipped.

Exit codes: `0` for success, also when a closed pipe cuts the output short;
`1` for errors; `2` for usage errors. More in
[docs/storage.md](docs/storage.md#troubleshooting).

## Development

```sh
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
```

Snapshot tests, the demo GIF and releasing are covered in
[docs/development.md](docs/development.md).

## License

MIT. A Rust rewrite of [iksnae/cursor-session](https://github.com/iksnae/cursor-session).
