# Where cursor-session reads data from

Where each store is found, how it is opened without changing anything, and
what to do when loading fails. Back to the [README](../README.md).

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
  in its `-wal` (see [Read-only access](#read-only-access)).

The Agent CLI keeps one directory per session:

```text
~/.cursor/chats/<md5 of workspace path>/<session-id>/meta.json     title, workspace, times
~/.cursor/chats/<md5 of workspace path>/<session-id>/store.db      name, last model used
~/.cursor/projects/<project>/agent-transcripts/<session-id>/<session-id>.jsonl   messages
```

The IDE keeps its chats in the `cursorDiskKV` table of `state.vscdb`: one
`composerData:<id>` row per chat and one `bubbleId:<chat id>:<message id>` row
per message.

## `--storage`

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

## Read-only access

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
- **After a crash**, Cursor leaves its `-wal` and `-shm` behind, which looks the
  same as Cursor running, so the database is read in place. SQLite then rebuilds
  the `-shm`, an index of the `-wal`, as Cursor does the next time it opens the
  database; the database and its `-wal` are not changed. When a `-wal` is left
  without its `-shm`, reading in place would create files next to your data, so
  the database and its `-wal` are copied to a private temporary directory, read
  there and deleted. This needs free space for the copy in `TMPDIR` (`TMP` on
  Windows). Starting and quitting Cursor once avoids the copy. If cursor-session
  is killed during such a read, its copy stays behind until a later run removes
  it, an hour on.

A database where SQLite's locks cannot do their job is never read in place. That
is one on a filesystem that another machine or VM serves, such as a network
share or the Windows drives WSL mounts under `/mnt`, because the locks and the
`-shm` of a Cursor on the other side do not reach across it. On macOS it is also
one on a read-only volume, such as a Time Machine backup or a disk image mounted
read-only, as SQLite there opens such a database without locks and so without
its `-shm`. Such a database is read as immutable while its journal is empty, and
from a copy of it and its journal otherwise. On Linux, a database on a
read-only mount is read like any other: while it has its `-wal` and `-shm`, in
place with the `-shm` opened read-only, which also keeps up with a Cursor that
writes it through another mount, such as the host behind a container's
read-only bind mount. On
Windows, one on a network path (`\\server\share\...`) is always read from a copy,
as SQLite cannot open such a path as immutable. If Cursor writes to it during
every attempt, the command stops with `changed while it was being read`.

One case can still leave files next to the database: when Cursor quits in the
moment between cursor-session finding it open and starting to read, the read
creates an empty `-wal` and a `-shm`, which a read-only connection cannot
remove. Cursor deletes them the next time it opens and closes the database.

## What is included

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
  text nor code blocks, such as tool calls and images, are skipped. Chats from
  older Cursor versions, which keep their messages inside the chat row, are read
  too. A message whose type is neither user nor assistant is shown with the role
  `unknown` rather than a guess.
- `show --all` shows all loaded text messages, not the excluded records or
  content that exists only in the databases.

## Troubleshooting

**Cursor changed its storage format.** If a Cursor update changes the IDE
database layout, `list`, `show` and `export` stop with an error like this one.
The same error, with another reason, appears when none of the chat rows or none
of the message rows can be read, when messages are stored only for chats whose
rows are under another key or when no chat lists them, when chats list messages
but none of them can be read, or when no message has a known type (user or
assistant):

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

When only part of the data changed, as when Cursor writes new chats in a new
format and leaves older ones as they are, what can be read still lists and a
`warning:` line says what is left out, also without `-v`, and `healthcheck`
marks the store `(incomplete)`. That is the case for chats whose rows are under
another key (`left out 2 chat(s) with 10 message row(s) in …`), for messages of
a type this version does not know, which are shown as `unknown`, and for a
database with no chat or message rows but rows under keys named like them, such
as `composerV2:`. Messages of a chat that has no row at all and that no other row
names, as a deleted chat leaves them, are only a `-v` warning, and so are rows
of other kinds in a database without chats.

The Agent CLI gets the same treatment. When transcripts hold lines but none of
them yields a message, for example because their roles are no longer `user` and
`assistant`, the error reads `unrecognized Cursor Agent CLI storage
format` and `--source ide` skips those sessions. When no `store.db` can be read
because its tables or values changed, the sessions still list, without a model
and under their `meta.json` title or else their ID, and a `warning:` line says
so. When no `meta.json` can be read, because it holds none of the keys this
version reads, the sessions list under their `store.db` name or else their ID,
without workspace or times, and a `warning:` line says so too. A value of an
unexpected type costs only that value. An empty `store.db` or `meta.json`, as a
session that was never used leaves, is not a change of format.

`list --limit N` and `show` read the messages of only the sessions they print.
When what they read looks like a change of format, they check every chat
before deciding, so they stop with the same error `list` would; warnings about
skipped rows cover only what they read.

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
