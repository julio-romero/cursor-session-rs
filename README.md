# cursor-session

`cursor-session` lists, shows and exports the chat sessions Cursor keeps on your
machine. It reads two local stores: the **Cursor Agent CLI** chats under
`~/.cursor`, and the **Cursor IDE** chat history in its `state.vscdb` database.
Both are merged into one list that you can print as a table, plain text or JSON,
or export to Markdown, JSON, JSONL or YAML. It only reads: databases are opened
read-only, so it is safe to run while Cursor is open.

A Rust rewrite of [iksnae/cursor-session](https://github.com/iksnae/cursor-session).

## Install

### Homebrew (macOS and Linux)

```sh
brew install julio-romero/tap/cursor-session
```

On Linux the binary needs glibc 2.35 or newer (Ubuntu 22.04, Debian 12,
Fedora 36 or later). On older systems, such as RHEL 9 or Amazon Linux 2023,
or on musl-based ones such as Alpine, use `cargo install` instead.

### Shell installer (macOS and Linux)

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/julio-romero/cursor-session-rs/releases/latest/download/cursor-session-installer.sh | sh
```

The script downloads the binary for your platform (macOS on Apple silicon or
Intel, Linux on x86_64 or arm64 with glibc 2.35 or newer) from the latest GitHub
Release and installs it to `$CARGO_HOME/bin`, which defaults to `~/.cargo/bin`.
If that directory is not on your `PATH`, it adds it in your shell profile. On
an older glibc the script stops with `no compatible downloads were found for
your platform`; use `cargo install` there.

To install somewhere else, set `CURSOR_SESSION_INSTALL_DIR`. The binary goes in
its `bin` subdirectory, so this installs `~/.local/bin/cursor-session`:

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/julio-romero/cursor-session-rs/releases/latest/download/cursor-session-installer.sh | CURSOR_SESSION_INSTALL_DIR="$HOME/.local" sh
```

Set `CURSOR_SESSION_NO_MODIFY_PATH=1` as well to leave your shell profiles alone.

### Cargo

```sh
cargo install cursor-session --locked
```

This builds from source and needs Rust 1.88 or newer.

**Windows:** download `cursor-session-x86_64-pc-windows-msvc.zip` from the
[latest release](https://github.com/julio-romero/cursor-session-rs/releases/latest)
and put `cursor-session.exe` on your `PATH`, or use `cargo install`.

## Usage

### List sessions

```sh
cursor-session list
```

Sessions from both stores, most recently updated first. In a terminal you get a
table fitted to the window:

```text
Found 4 session(s)

┌──────────────────────────────────────┬────────┬──────┬──────────────────┬───────────────────┐
│ ID                                   ┆ SOURCE ┆ MSGS ┆ UPDATED          ┆ TITLE             │
╞══════════════════════════════════════╪════════╪══════╪══════════════════╪═══════════════════╡
│ a71d0e58-2c39-4f7b-b6a4-19e8c3d5f027 ┆ agent  ┆ 4    ┆ 2026-10-05 14:12 ┆ Add retry with b… │
│ e5b8c4d2-6a1f-4e93-8d27-5f0a9b3c1e46 ┆ ide    ┆ 3    ┆ 2026-10-04 16:48 ┆ Speed up the mon… │
│ 3f9c2a71-8b4e-4d6a-9e15-7c0b2d4f8a63 ┆ agent  ┆ 3    ┆ 2026-10-02 08:31 ┆ Fix flaky timezo… │
│ 9d2f6b13-47ce-4a85-a0b9-e3c51d7f2864 ┆ ide    ┆ 2    ┆ 2026-09-28 11:09 ┆ Explain the migr… │
└──────────────────────────────────────┴────────┴──────┴──────────────────┴───────────────────┘
```

Long titles are cut to fit. On narrower terminals the IDs are shortened to whole
UUID groups (8, 13, 18 or 23 characters) and a footer says so:

```text
IDs shortened to 8 chars; `show` accepts a prefix.
```

When the output is piped or redirected, `list` prints plain columns with full
IDs and titles and no color. This layout is stable for `grep`, `awk` and
scripts:

```text
Found 4 session(s)

ID                                    SOURCE   MSGS  UPDATED           TITLE
----------------------------------------------------------------------------------------------------
a71d0e58-2c39-4f7b-b6a4-19e8c3d5f027  agent       4  2026-10-05 14:12  Add retry with backoff to the webhook sender
e5b8c4d2-6a1f-4e93-8d27-5f0a9b3c1e46  ide         3  2026-10-04 16:48  Speed up the monthly revenue dashboard query
3f9c2a71-8b4e-4d6a-9e15-7c0b2d4f8a63  agent       3  2026-10-02 08:31  Fix flaky timezone test in invoice due dates
9d2f6b13-47ce-4a85-a0b9-e3c51d7f2864  ide         2  2026-09-28 11:09  Explain the migration lock error
```

- `UPDATED` is in UTC. A session with no stored update time shows its creation time.
- With no sessions, `list` prints `No sessions found` and exits 0.
- In a terminal, control characters and escape sequences in stored titles and
  messages are removed before printing. The exception is `show` with color on:
  it keeps color and style codes, except blinking and hidden text, and resets
  them at the end of each line.
  Piped output is written as stored, so a title that contains a line break
  spans two lines there; `list --json` is exact for scripts.

### Filter by source and count

```sh
cursor-session list --source agent --limit 1
```

```text
Found 1 session(s)

ID                                    SOURCE   MSGS  UPDATED           TITLE
----------------------------------------------------------------------------------------------------
a71d0e58-2c39-4f7b-b6a4-19e8c3d5f027  agent       4  2026-10-05 14:12  Add retry with backoff to the webhook sender
```

- `--source agent` or `--source ide` reads only that store. The other one is
  never opened, so a broken store cannot get in the way.
- `--limit N` (N ≥ 1) keeps the N most recently updated sessions.

### Show a session

```sh
cursor-session show a71d --limit 2
```

```text
Add retry with backoff to the webhook sender
id:        a71d0e58-2c39-4f7b-b6a4-19e8c3d5f027
source:    agent
workspace: /Users/dana/src/billing-api
model:     claude-4.5-sonnet
created:   2026-10-05 13:40 UTC
updated:   2026-10-05 14:12 UTC
messages:  4

2 earlier message(s) omitted. Use --limit N or --all to see more.

[user (Monday, Oct 5, 2026, 9:05 AM (UTC-5))]
Make the attempt count configurable.

[assistant]
Done. `WEBHOOK_MAX_ATTEMPTS` (default 5) is read in `Config::from_env`, and the tests cover 1, 3 and 5 attempts.
```

- The ID can be the full session ID or any unique prefix of it, in any case, such
  as the 8-character prefix from the table. If several sessions share the
  prefix, the error lists them and you can type more characters.
- In a terminal, `show` prints the last 20 messages. Piped, it prints all of them.
  `--limit N` prints the last N and `--all` prints everything. The two flags
  cannot be combined.
- `--source agent|ide` works here too.
- The time next to a message is local-time text as the Agent CLI stored it,
  or for IDE messages a UTC time such as `2026-10-04 16:21 UTC`.

### Export

```sh
cursor-session export                                          # every session, Markdown, into ./exports
cursor-session export --format json --session-id 3f9c --out sessions
cursor-session export --format jsonl --workspace /Users/dana/src/billing-api
cursor-session export --format yaml --source ide
```

Each session is written to `<out>/<session-id>.<ext>`, one `wrote <path>` line
per file. `--out` defaults to `exports` and is created if missing. Existing
files are overwritten. A session ID that is not a plain file name, which only a
damaged or crafted database holds, is written under a name made from it plus a
short hash, so every file stays inside `<out>`. If the `wrote` lines go to a
reader that stops early (`| head`), every file is still written.

| `--format`     | Contents                                                                                                |
| -------------- | ------------------------------------------------------------------------------------------------------- |
| `md` (default) | Title, a metadata list, then every message                                                              |
| `json`         | The session object: `id`, `title`, `source`, `workspace`, `workspace_hash`, `created_at_ms`, `updated_at_ms`, `model`, `messages`. Missing values are left out. |
| `jsonl`        | One message per line: `role`, `content`, `timestamp`. A missing `timestamp` is left out.                |
| `yaml`         | The same fields as `json`                                                                               |

- `--session-id` takes an ID or a unique prefix. It cannot be combined with
  `--workspace`.
- `--workspace` takes a workspace path, which also selects the workspaces
  below it; whole directory names from a path, such as `billing-api` or
  `src/billing-api`; or the MD5 hash that names its directory under
  `~/.cursor/chats`. A trailing `/` makes no difference, and `.`, `..` and `~`
  are resolved first, so `--workspace .` is the current directory. Only Agent
  CLI sessions record a workspace.
- An unknown `--session-id` gives `session not found`, a `--workspace` that
  matches nothing gives `no sessions matched`, and no sessions at all gives
  `no sessions to export`. A file or directory that cannot be written gives
  `could not write <path>` or `could not create <dir>` with the reason. All exit 1.
- Exports contain the stored text unchanged. Times in `json` and `yaml` exports
  are epoch milliseconds; this is not the `--json` format described below.
  Markdown exports show times like `show` does.

```console
$ cursor-session export --session-id 3f9c
wrote exports/3f9c2a71-8b4e-4d6a-9e15-7c0b2d4f8a63.md
$ head -n 15 exports/3f9c2a71-8b4e-4d6a-9e15-7c0b2d4f8a63.md
# Fix flaky timezone test in invoice due dates

- **ID:** `3f9c2a71-8b4e-4d6a-9e15-7c0b2d4f8a63`
- **Source:** agent
- **Workspace:** /Users/dana/src/invoice-service
- **Model:** gpt-5
- **Created:** 2026-10-02 08:03 UTC
- **Updated:** 2026-10-02 08:31 UTC
- **Messages:** 3

---

**user:** (Friday, Oct 2, 2026, 10:03 AM (UTC+2))

test_due_date_end_of_month fails only on CI. Why?
```

### Healthcheck

```sh
cursor-session healthcheck
```

```text
Cursor session healthcheck

agent chats: /Users/dana/.cursor/chats (ok)
transcripts: /Users/dana/.cursor/projects/{project}/agent-transcripts (ok)
ide db: /Users/dana/Library/Application Support/Cursor/User/globalStorage/state.vscdb (ok)

sessions loaded: 4 (agent: 2, ide: 2)
```

It shows each location it found, with `(ok)` or `(failed)`, or `not found` and
the paths it looked in. It exits 1 when no storage is found at all, or when a
store that was found fails to load; the report then includes the reason and what
to do about it. When rows or files were skipped, a `load warnings: N` line says
so; `-v` lists them.

### Global flags

`--storage`, `-v`/`--verbose` and `--color` work before or after the subcommand.
`-V`/`--version` works only before it, and `-h`/`--help` after a subcommand
prints that subcommand's options.

| Flag                          | Effect                                                                                               |
| ----------------------------- | ---------------------------------------------------------------------------------------------------- |
| `--storage <PATH>`            | Read only this location instead of detecting Cursor's data. See `--storage` under "Where it reads data from". |
| `-v`, `--verbose`             | Print the storage paths in use and the rows and files that were skipped to stderr.                  |
| `--color <auto\|always\|never>` | `auto` (default) colors only a terminal, and only when `NO_COLOR` is unset or empty and `TERM` is not `dumb`. `always` colors even when piped and overrides `NO_COLOR`. `never` prints no color codes. Help and usage errors follow the same rules. |
| `-h`, `--help`                | `-h` prints a summary with examples. `--help` adds the data sources and exit codes.                  |
| `-V`, `--version`             | Print the version.                                                                                   |

Color never changes the layout: a terminal with `NO_COLOR=1` still gets the
table, and `--color always` on a pipe colors the plain columns.

```sh
cursor-session --storage ~/.cursor list
cursor-session list --storage "$HOME/Library/Application Support/Cursor/User/globalStorage/state.vscdb"
cursor-session -v list --source ide
NO_COLOR=1 cursor-session list
```

## JSON output

`list --json` prints an array of sessions and `show --json` prints one session
with its messages.

- Written to stdout, pretty-printed with two-space indentation and a trailing
  newline. No color is added and nothing is fitted to the terminal.
- Control characters in strings are escaped (as `\n`, `\t` or `\u00XX`), and
  so are DEL and U+0080 to U+009F (as `\u00XX`), so the JSON is the same in a
  terminal and in a pipe.
- `list --json` keeps the list order (most recently updated first) and applies
  `--source` and `--limit`. No sessions gives `[]`.
- `show --json` includes every message, also in a terminal, unless `--limit N`
  is given.
- Every key is always present, in this order. Unknown values are `null`.

| Key              | Type                 | Notes                                                                                            |
| ---------------- | -------------------- | ------------------------------------------------------------------------------------------------ |
| `id`             | string               |                                                                                                  |
| `title`          | string               | Agent CLI: the stored title, else the session ID. IDE: the chat name, else `Untitled`.           |
| `source`         | string               | `"agent"` or `"ide"`                                                                             |
| `workspace`      | string or null       | Agent CLI working directory. `null` for IDE sessions.                                            |
| `workspace_hash` | string or null       | MD5 of the workspace path, the directory name under `~/.cursor/chats`. `null` for IDE sessions. |
| `model`          | string or null       | Last model used, from the Agent CLI `store.db`. `null` for IDE sessions.                         |
| `created_at`     | string or null       | RFC 3339 in UTC with whole seconds, such as `"2026-10-05T13:40:12Z"`. Works with jq's `fromdate`. |
| `updated_at`     | string or null       | Same format as `created_at`. `null` when Cursor stored no update time; the list then uses `created_at`. |
| `message_count`  | integer              | All messages in the session, even when `show --limit` returns fewer.                             |
| `messages`       | array (`show` only)  | Objects with `role` (`"user"` or `"assistant"`), `content` (string) and `timestamp`.             |

A message `timestamp` is a string as Cursor stored it, or `null`: free text such
as `"Friday, Oct 2, 2026, 10:29 AM (UTC+2)"` for Agent CLI messages, and epoch
milliseconds such as `"1790593334000"` for IDE messages.

```console
$ cursor-session list --json --limit 2
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
  },
  {
    "id": "e5b8c4d2-6a1f-4e93-8d27-5f0a9b3c1e46",
    "title": "Speed up the monthly revenue dashboard query",
    "source": "ide",
    "workspace": null,
    "workspace_hash": null,
    "model": null,
    "created_at": "2026-10-04T16:20:00Z",
    "updated_at": "2026-10-04T16:48:31Z",
    "message_count": 3
  }
]
```

```console
$ cursor-session show 3f9c --json --limit 2
{
  "id": "3f9c2a71-8b4e-4d6a-9e15-7c0b2d4f8a63",
  "title": "Fix flaky timezone test in invoice due dates",
  "source": "agent",
  "workspace": "/Users/dana/src/invoice-service",
  "workspace_hash": "8998393e31d4405c107afef419529440",
  "model": "gpt-5",
  "created_at": "2026-10-02T08:03:55Z",
  "updated_at": "2026-10-02T08:31:09Z",
  "message_count": 3,
  "messages": [
    {
      "role": "assistant",
      "content": "CI runs in UTC and your laptop in Europe/Berlin. The test builds the due date with `Local::now()`, so the month boundary moves. Use a fixed `Utc` timestamp.",
      "timestamp": null
    },
    {
      "role": "user",
      "content": "Thanks, that fixed it.",
      "timestamp": "Friday, Oct 2, 2026, 10:29 AM (UTC+2)"
    }
  ]
}
```

With [jq](https://jqlang.org):

```console
$ cursor-session list --json | jq -r '.[] | select(.source == "ide") | "\(.id)  \(.title)"'
e5b8c4d2-6a1f-4e93-8d27-5f0a9b3c1e46  Speed up the monthly revenue dashboard query
9d2f6b13-47ce-4a85-a0b9-e3c51d7f2864  Explain the migration lock error

$ cursor-session show a71d --json | jq -r '.messages[] | select(.role == "user") | .content'
Webhook deliveries fail for good on the first 503. Add retries with exponential backoff.
Make the attempt count configurable.

$ cursor-session list --json | jq '[.[] | select(.updated_at != null and (.updated_at | fromdate) > now - 7 * 86400)] | length'
3
```

## Where it reads data from

| OS      | Agent CLI                                                                                                   | Cursor IDE                                                                                               |
| ------- | ----------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------- |
| macOS   | `~/.cursor/chats` and `~/.cursor/projects` (fallback: `~/.config/cursor/`)                                   | `~/Library/Application Support/Cursor/User/globalStorage/state.vscdb`                                    |
| Linux   | `~/.cursor/chats` and `~/.cursor/projects` (fallbacks: `$XDG_CONFIG_HOME/cursor/`, `~/.config/cursor/`)       | `$XDG_CONFIG_HOME/Cursor/User/globalStorage/state.vscdb` (default `~/.config/Cursor/User/globalStorage/state.vscdb`) |
| Windows | `%USERPROFILE%\.cursor\chats` and `%USERPROFILE%\.cursor\projects`                                          | `%APPDATA%\Cursor\User\globalStorage\state.vscdb`                                                        |

- The first location that exists is used, separately for chats, transcripts and
  the IDE database.
- If `CURSOR_CONFIG_DIR` is set, `$CURSOR_CONFIG_DIR/chats` and
  `$CURSOR_CONFIG_DIR/projects` are checked first.
- The home directory is `HOME` on macOS and Linux and `USERPROFILE` on Windows.
- Under WSL, the Linux home inside WSL is used. The IDE runs on Windows and is
  not detected; pass `--storage /mnt/c/Users/<you>` to read the Windows side.
  SQLite cannot share its locks across the WSL boundary, so the database is
  then read without them: as immutable, or from a copy while Cursor has commits
  in its `-wal` (see "Read-only access").

The Agent CLI keeps one directory per session:

```text
~/.cursor/chats/<md5 of workspace path>/<session-id>/meta.json     title, workspace, times
~/.cursor/chats/<md5 of workspace path>/<session-id>/store.db      name, last model used
~/.cursor/projects/<project>/agent-transcripts/<session-id>/<session-id>.jsonl   messages
```

The IDE keeps its chats in the `cursorDiskKV` table of `state.vscdb`: one
`composerData:<id>` row per chat and one `bubbleId:<chat id>:<message id>` row
per message.

### `--storage`

With `--storage`, only that location is read and nothing is detected from your
home directory. A leading `~` is expanded. It accepts:

| Path                                                                   | Reads                                                         |
| ---------------------------------------------------------------------- | ------------------------------------------------------------- |
| A home directory, such as `/mnt/c/Users/<you>` from WSL                | Its `.cursor` data and its macOS, Linux or Windows IDE database |
| A `.cursor` directory                                                  | Its `chats` and `projects`                                    |
| A `chats` directory                                                    | Those chats, plus transcripts from the `projects` directory next to it |
| A workspace directory (`chats/<md5>`), a session directory, or a `store.db` | Only those sessions, with their transcripts                 |
| A `projects` directory                                                 | Transcripts only; the session ID is the title                 |
| `state.vscdb` (any `*.vscdb` or `*.vscdb.backup` file)                 | IDE sessions only                                             |
| A directory containing `state.vscdb`: `globalStorage`, `Cursor/User` or `Cursor` | IDE sessions only                                   |

A path that does not exist gives `storage path does not exist`, a path that
cannot be read gives `could not access`, and any other path gives
`unrecognized storage location`. All three exit 1.

### Read-only access

cursor-session never changes Cursor's data. Every SQLite database it reads
(`state.vscdb` and each session's `store.db`) is opened with SQLite's read-only
flag and `PRAGMA query_only`.

A database in rollback-journal mode, SQLite's default, is read in place and
creates no files. A read can make Cursor wait briefly to commit, and if the
database is locked the read waits up to 5 seconds before giving up. If a crash
left a journal that must be rolled back, which takes write access, the database
and its journal are copied to a private temporary directory and rolled back
there. A database in WAL mode is read depending on the files next to it:

- **While Cursor is running**, the database has `-wal` and `-shm` files next to
  it, even when the `-wal` is empty. cursor-session reads it in place like any
  other SQLite reader and sees everything Cursor has committed, each chat with
  its messages from the same commit. Readers do not block Cursor's writes, and
  if the database is locked it waits up to 5 seconds before giving up.
- **When Cursor is closed**, the database is opened as immutable. SQLite then
  creates no `-wal` or `-shm` files next to it, and reading works even in a
  read-only directory. If Cursor starts and changes the file during the read,
  the read is retried; if the file changes during the retry too, the command
  stops with `changed while it was being read` and you can run it again.
- **After a crash**, Cursor can leave a `-wal` file without its `-shm`. Reading
  that in place would create files next to your data, so the database and its
  `-wal` are copied to a private temporary directory, read there and deleted.
  This needs free space for the copy in `TMPDIR` (`TMP` on Windows). Starting
  and quitting Cursor once avoids the copy. If cursor-session is killed during
  such a read, its copy stays behind until a later run removes it, an hour on.

A database where SQLite's locks cannot do their job is never read in place. That
is one on a filesystem that another machine or VM serves, such as a network
share or the Windows drives WSL mounts under `/mnt`, because the locks and the
`-shm` of a Cursor on the other side do not reach across it. It is also one on a
read-only volume, such as a Time Machine backup or a disk image mounted
read-only, where no Cursor can be writing. Such a database is read as immutable
while its journal is empty, and from a copy of it and its journal otherwise. If
Cursor writes to it during every attempt, the command stops with `changed while
it was being read`.

One case can still leave files next to the database: when Cursor quits in the
moment between cursor-session finding it open and starting to read, the read
creates an empty `-wal` and a `-shm`, which a read-only connection cannot
remove. Cursor deletes them the next time it opens and closes the database.

### What is included

- Both stores are merged by session ID. A session found in both appears once,
  as `agent`, with the Agent CLI transcript.
- When a session has transcripts in several projects, the copy with the most
  parsed text messages is used. Ties go to the most recently modified file, then
  to the lexicographically greatest path, so directory order does not affect the
  result. Copies are not concatenated. Flat `<session-id>.jsonl` files directly
  in `agent-transcripts` are also read.
- Agent CLI messages: only user and assistant text. Tool calls, tool results,
  and plans stored inside tool calls are left out. The `<timestamp>` and
  `<user_query>` wrappers around user messages are removed, and the timestamp
  text becomes the message time. `store.db` is only used for metadata (name,
  last model used, creation time); its other blobs are not decoded.
- IDE messages: the text of each message, or its rich text when the plain text
  is empty, followed by its code blocks as fenced code. Messages with neither
  text nor code blocks are skipped. Chats from older Cursor versions, which keep
  their messages inside the chat row, are read too.
- `show --all` shows all loaded text messages, not the excluded records or
  content that exists only in the databases.

## Exit codes and troubleshooting

| Code | Meaning                                                                                                     |
| ---- | ----------------------------------------------------------------------------------------------------------- |
| 0    | Success, including `--help`, `--version`, an empty list, and output cut short by a closed pipe (`cursor-session list \| head`), which prints nothing on stderr |
| 1    | Runtime error: session not found or ambiguous, unreadable storage, changed storage format, failed healthcheck, nothing to export |
| 2    | Usage error: unknown command or flag, invalid value (`list --limit 0`, `--source web`, an empty session ID or `--workspace`), `--limit` together with `--all`, missing subcommand |

Errors go to stderr as `error: <message>`, then one `caused by:` line per
underlying cause, then hints:

```text
error: session not found: nope
run `cursor-session list` to see session IDs
```

**Cursor changed its storage format.** If a Cursor update changes the IDE
database layout, `list`, `show` and `export` stop with an error like this one.
The same error, with another reason, appears when none of the chat rows or none
of the message rows can be read, when messages are stored but no chat row is
found or no chat lists them, when chats list messages but none of them can be
read, or when no message has a known type (user or assistant):

```text
error: unrecognized Cursor IDE storage format in /Users/dana/Library/Application Support/Cursor/User/globalStorage/state.vscdb: table `cursorDiskKV` not found (tables present: ItemTable). Cursor may have changed its storage format.
rerun with `--source agent` to skip IDE sessions
report it at https://github.com/julio-romero/cursor-session-rs/issues and include your Cursor version
```

Agent CLI sessions still work with `--source agent`, which never opens the IDE
database:

```sh
cursor-session list --source agent
```

The Agent CLI gets the same treatment. When transcripts hold lines but none of
them yields a message, for example because their roles are no longer `user` and
`assistant`, the error reads `unrecognized Cursor Agent CLI storage
format` and `--source ide` skips those sessions. When no `store.db` can be read
because its tables or values changed, the sessions still list, without a model
and under their `meta.json` title or else their ID, and a `warning:` line says
so. An empty `store.db`, as a session that was never used leaves, is not a
change of format.

Please [open an issue](https://github.com/julio-romero/cursor-session-rs/issues)
with your Cursor version.

**No sessions, or not the ones you expect.** Run `cursor-session healthcheck` to
see which locations were found and whether they load. If your data lives
elsewhere, point `--storage` at it. When no location is found at all, `show`
and `export` stop with `no Cursor session storage found`.

**Sessions or messages missing.** Unreadable rows and files are skipped so the
rest still loads. A location that cannot be read, such as `~/.cursor/chats`
without permission while the transcripts load, is reported with a `warning:`
line on every run. `-v` prints the paths in use and a `warning:` line for each
kind of skipped data:

```console
$ cursor-session -v list --source ide >/dev/null
chats: /Users/dana/.cursor/chats
projects: /Users/dana/.cursor/projects
ide db: /Users/dana/Library/Application Support/Cursor/User/globalStorage/state.vscdb
warning: skipped 1 unreadable composer row in /Users/dana/Library/Application Support/Cursor/User/globalStorage/state.vscdb
```

**`could not read SQLite database`** with the hint `try again in a moment`:
Cursor held a lock for more than 5 seconds. Run the command again.

**`could not copy … to a temporary directory for reading`**: the temporary
directory is not writable or is full. Point `TMPDIR` (`TMP` on Windows)
elsewhere, or start and quit Cursor once so the database can be read in place.

## Development

```sh
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
```

CI runs clippy and the tests on Linux, macOS and Windows, rustfmt and
`cargo package` (the crate as crates.io receives it) on Linux, and `cargo check`
with the minimum supported Rust version, 1.88.

Output rendering is covered by [insta](https://insta.rs) snapshot tests. When
output changes, `cargo test` fails and writes the new output next to the old
snapshot as a `.snap.new` file. To review and accept the changes:

```sh
cargo insta review                        # needs cargo-insta: cargo install cargo-insta
INSTA_UPDATE=always cargo test --locked   # without cargo-insta: accept all, then check git diff
```

`INSTA_UPDATE=always` leaves the `.snap.new` files of earlier runs behind.
Delete them with `find tests/snapshots -name '*.snap.new' -delete`.

### Releasing

Releases are built by [cargo-dist](https://github.com/axodotdev/cargo-dist)
when a version tag is pushed. Before the first release, once:

- Create the public repository `julio-romero/homebrew-tap` with at least one
  commit (a README is enough).
- Add the secret `HOMEBREW_TAP_TOKEN` to this repository: a fine-grained
  personal access token with Contents read and write access to the tap only.
- Add the secret `CARGO_REGISTRY_TOKEN`: a crates.io API token with the
  publish-new and publish-update scopes, from an account with a verified email.
- Make this repository public. Until then the Homebrew formula and the shell
  installer download from private releases and fail with 404.

`gh secret list --repo julio-romero/cursor-session-rs` should then show both
secrets. Without them a tag push still creates the GitHub Release, and then
the Homebrew and crates.io jobs fail. For each release:

1. Set `version` in `Cargo.toml`, run `cargo check` to update `Cargo.lock`, then
   commit and push to `master`.
2. Wait for CI to pass on that commit. The Release workflow does not run the
   tests itself.
3. Check that `dist plan` prints `announcing vX.Y.Z`.
4. Push the tag: `git tag vX.Y.Z && git push origin vX.Y.Z`.

The Release workflow builds archives for the five targets, creates the GitHub
Release with the archives, checksums and the shell installer, pushes the
formula to [julio-romero/homebrew-tap](https://github.com/julio-romero/homebrew-tap)
and publishes the crate to crates.io. A pre-release tag such as `v1.0.0-rc.1`
creates a GitHub pre-release and skips Homebrew and crates.io.

## License

MIT. See the `LICENSE` file.
