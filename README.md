# cursor-session

[![CI](https://github.com/julio-romero/cursor-session-rs/actions/workflows/ci.yml/badge.svg?branch=master)](https://github.com/julio-romero/cursor-session-rs/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/cursor-session.svg)](https://crates.io/crates/cursor-session)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

**Find, search, read, export and hand off your Cursor chat history from the terminal.**

![cursor-session listing, searching, showing, handing off and exporting Cursor chats](demo/demo.gif)

Cursor keeps every chat on your machine, split between a SQLite database for
the IDE and a folder per session for the Agent CLI. `cursor-session` reads both
and merges them into one list. Search them, open any conversation in your
terminal, export it to Markdown, JSON, JSONL or YAML, copy it to the clipboard
for another agent to continue, or pipe `--json` into your own scripts.

It only reads and never writes to Cursor's data, so it is safe to run while
Cursor is open.

## Features

- **Both sources, one list:** Cursor IDE chats and Cursor Agent CLI sessions,
  newest first, with a message count and a token estimate for each.
- **Search:** find the sessions whose messages hold every word of a query,
  best matches first, with the matching words highlighted.
- **Short IDs:** `show a71d` works with any unique prefix of a session ID.
- **Focused reading:** `show --only user,assistant,tool` picks the roles,
  including tool calls and their results, and `--short` cuts long messages.
- **Handoff:** copy a session's transcript to the clipboard, ready to paste
  into another agent when a Cursor session runs out of credits.
- **Export:** Markdown, JSON, JSONL or YAML, one file per session, selected by
  ID, workspace, source, count or age (`--since 7d`).
- **Scriptable:** `--json` with a stable schema for `jq`, plain columns when
  piped, and a clean exit when `| head` closes the pipe.
- **Safe:** read-only, works while Cursor is running, and stops with a clear
  error when a Cursor update changes the storage format, rather than showing
  wrong data.
- **Cross-platform:** macOS, Linux and Windows, with shell completions for
  bash, zsh and fish, and man pages.

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
cursor-session search retry timeout       # sessions that mention both words
cursor-session show a71d                  # one session, by ID or ID prefix
cursor-session handoff a71d               # copy it for another agent
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

ID                                    SOURCE   MSGS    TOKENS  UPDATED           TITLE
----------------------------------------------------------------------------------------------------
a71d0e58-2c39-4f7b-b6a4-19e8c3d5f027  agent       4       175  2026-10-05 14:12  Add retry with backoff to the webhook sender
e5b8c4d2-6a1f-4e93-8d27-5f0a9b3c1e46  ide         3        80  2026-10-04 16:48  Speed up the monthly revenue dashboard query
3f9c2a71-8b4e-4d6a-9e15-7c0b2d4f8a63  agent       3        57  2026-10-02 08:31  Fix flaky timezone test in invoice due dates
```

`TOKENS` estimates how many tokens the messages `show` prints take:
characters / 4, rounded up. It is an estimate, not any model's tokenizer count.
A terminal table shows it from 79 columns wide; piped output always has it.

Narrow it down with `--source agent` or `--source ide`, `--limit N`, and
`--since` (a number and `s`, `m`, `h`, `d` or `w`) for the sessions updated
recently:

```sh
cursor-session list --source agent --limit 5
cursor-session list --since 7d
```

### Search

```console
$ cursor-session search retry timeout
Found 2 matching session(s)

a71d0e58-2c39-4f7b-b6a4-19e8c3d5f027  agent  2026-10-05 14:12  Add retry with backoff to the webhook sender
  [assistant] I added `RetryPolicy` to `webhooks/sender.rs`: 5 attempts, backoff from 50…

9d2f6b13-47ce-4a85-a0b9-e3c51d7f2864  ide    2026-09-28 11:09  Explain the migration lock error
  [assistant] …stomers` while the migration ran `ALTER TABLE`. Set a `lock_timeout` and retry, or run it outside peak hours.
```

Every word must appear in the session's messages, in any order and any case;
`"connection pool"` in quotes is one phrase. Sessions with every word in one
message come first. In a terminal the matches are highlighted. `-n N` keeps the
best N, `--context N` sets how much of the message to show, and `--source`,
`--since` and `--json` work as for `list`.

### Show

````console
$ cursor-session show e5b8 --limit 2
Speed up the monthly revenue dashboard query
id:        e5b8c4d2-6a1f-4e93-8d27-5f0a9b3c1e46
source:    ide
created:   2026-10-04 16:20 UTC
updated:   2026-10-04 16:48 UTC
messages:  3
tokens:    ~80 (estimate)

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
`--limit N` prints the last N and `--all` prints everything. The `tokens:` line
is the same estimate as `list`'s `TOKENS`.

Tool calls and their results are left out unless you ask for them, and
`--short` cuts each message to 300 characters and each tool call to one line:

```console
$ cursor-session show a71d --only tool --short | tail -n 11
[tool]
Grep {"pattern":"fn deliver","path":"src/webhooks"}

[tool]
src/webhooks/sender.rs:58: pub async fn deliver(&self, event: &Event) -> Result<(), SendError> {

[tool]
Shell {"command":"cargo test webhooks"}

[tool]
running 9 tests ......... test result: ok. 9 passed; 0 failed; 0 ignored; finished in 0.84s
```

`--only` takes `user`, `assistant` and `tool`, comma-separated.

### Handoff

When a Cursor session runs out of credits, `handoff` copies its transcript to
the clipboard so that you can paste it into another agent and carry on:

```console
$ cursor-session handoff 9d2f --stdout
The following is a transcript from a Cursor session that ran out of credits. Continue from where it ended; do not summarize it back.

[user]
What does `could not obtain lock on relation "customers"` mean during deploy?

[assistant]
Another transaction held a lock on `customers` while the migration ran `ALTER TABLE`. Set a `lock_timeout` and retry, or run it outside peak hours.

[end of transcript: 2 messages, ~110 tokens (estimate)]
```

The transcript holds the user and assistant messages as `show --short --only
user,assistant` prints them. Without `--stdout` it goes to the clipboard and
`handoff` prints one `copied N messages (~T tokens) to clipboard` line. It uses
`pbcopy` on macOS, the system clipboard on Windows, and X11 on Linux. Where no
clipboard can be reached, such as on Linux without a display, it prints the
transcript with a warning instead. On Wayland without XWayland, pipe it yourself:
`cursor-session handoff 9d2f --stdout | wl-copy`. `--limit N` keeps the last N
messages, and `--preamble TEXT` or `--no-preamble` replaces or drops the first
paragraph.

### Export

```sh
cursor-session export                                      # every session as Markdown, into ./exports
cursor-session export --format json --session-id 3f9c --out sessions
cursor-session export --format jsonl --workspace ~/src/billing-api
```

Each session is written to `<out>/<session-id>.<ext>`. The formats are `md`
(the default), `json`, `jsonl` and `yaml`. `--since 2w` exports only the
sessions updated in the last two weeks.

### JSON output

`list --json` prints an array of sessions in list order, `show --json` prints
one session with its `messages`, and `search --json` prints the matching
sessions with their best snippet. Every key is always present, unknown values
are `null`, and times are RFC 3339 in UTC.

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
    "message_count": 4,
    "token_estimate": 175
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

### Shell completions and man pages

```sh
cursor-session completions zsh > ~/.zfunc/_cursor-session    # also bash and fish
```

Release archives and the Homebrew package also carry the completion scripts
and man pages; [docs/install.md](docs/install.md#shell-completions-and-man-pages)
says where they land and how to enable them.

## Where it reads data from

| OS      | Agent CLI                                        | Cursor IDE                                                             |
| ------- | ------------------------------------------------ | ---------------------------------------------------------------------- |
| macOS   | `~/.cursor/chats`, `~/.cursor/projects`          | `~/Library/Application Support/Cursor/User/globalStorage/state.vscdb`  |
| Linux   | `~/.cursor/chats`, `~/.cursor/projects`          | `~/.config/Cursor/User/globalStorage/state.vscdb` (or `$XDG_CONFIG_HOME`) |
| Windows | `%USERPROFILE%\.cursor\chats`, `...\projects`    | `%APPDATA%\Cursor\User\globalStorage\state.vscdb`                      |

If your data lives elsewhere, point `--storage` at a home directory, a `.cursor`
directory or a `state.vscdb` file. Under WSL, `--storage /mnt/c/Users/<you>`
reads the Windows side.

Cursor's databases are opened with SQLite's read-only flag and
`PRAGMA query_only`. While Cursor runs, `cursor-session` reads alongside it like
any other SQLite reader. When Cursor is closed, it opens the database as
immutable and creates no files next to it. In a few cases, such as after a
crash or on a network share, it reads a private temporary copy instead and
deletes it afterwards; Cursor's own files are never written. The details, the
fallbacks and what is included are in [docs/storage.md](docs/storage.md).

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
`1` for errors, including a search that matches nothing; `2` for usage errors. More in
[docs/storage.md](docs/storage.md#troubleshooting).

## Development

```sh
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
```

Snapshot tests, the generated completions and man pages, and the demo GIF are
covered in [docs/development.md](docs/development.md), cutting a release in
[RELEASING.md](RELEASING.md). Changes per release are in
[CHANGELOG.md](CHANGELOG.md).

[`bench/`](bench) times `list`, `show`, `search` and `handoff` with
[hyperfine](https://github.com/sharkdp/hyperfine) on generated stores of 50,
300 and 1000 sessions and records their peak memory. Each release's numbers
are kept in [`bench/results`](bench/results).

## Similar tools

- [iksnae/cursor-session](https://github.com/iksnae/cursor-session) (Go)
  inspired this one and shares its core commands (list, show, export,
  healthcheck); it also has `snoop`, `upgrade` and `reconstruct`. Its README
  lists reading Agent CLI sessions as Linux-only; this one reads both the Agent
  CLI and the IDE store on macOS, Linux and Windows, and never writes to
  Cursor's data.
- [S2thend/cursor-history](https://github.com/S2thend/cursor-history) (Node.js)
  also lists, shows and searches sessions, and adds backup and restore and
  migrating sessions between workspaces, which changes Cursor's data; this one
  is a single binary with no runtime to install, and never writes to Cursor's
  data, so it is safe to run while Cursor is open.
- [SpecStory](https://specstory.com) is an editor extension and CLI that saves
  sessions from Cursor and other coding agents as Markdown in your project;
  this one installs nothing into Cursor and reads, read-only, the history
  Cursor already keeps.

## License

MIT. A Rust rewrite of [iksnae/cursor-session](https://github.com/iksnae/cursor-session).
