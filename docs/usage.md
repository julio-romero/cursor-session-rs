# Using cursor-session

Every command and flag, with output from the invented sessions in
[`demo/`](../demo). Back to the [README](../README.md).

## List sessions

```sh
cursor-session list
```

Sessions from both stores, most recently updated first. In a terminal you get a
table fitted to the window:

```text
Found 8 session(s)

┌──────────────────────────────────────┬────────┬──────┬──────────────────┬───────────────────┐
│ ID                                   ┆ SOURCE ┆ MSGS ┆ UPDATED          ┆ TITLE             │
╞══════════════════════════════════════╪════════╪══════╪══════════════════╪═══════════════════╡
│ a71d0e58-2c39-4f7b-b6a4-19e8c3d5f027 ┆ agent  ┆ 4    ┆ 2026-10-05 14:12 ┆ Add retry with b… │
│ e5b8c4d2-6a1f-4e93-8d27-5f0a9b3c1e46 ┆ ide    ┆ 3    ┆ 2026-10-04 16:48 ┆ Speed up the mon… │
│ 3f9c2a71-8b4e-4d6a-9e15-7c0b2d4f8a63 ┆ agent  ┆ 3    ┆ 2026-10-02 08:31 ┆ Fix flaky timezo… │
│ 9d2f6b13-47ce-4a85-a0b9-e3c51d7f2864 ┆ ide    ┆ 2    ┆ 2026-09-28 11:09 ┆ Explain the migr… │
│ 6c0e4b9a-3d21-4f87-9a5c-e1b7d2f04a38 ┆ agent  ┆ 2    ┆ 2026-09-24 15:37 ┆ Backfill custome… │
│ 4a6d1c88-0f3e-4b52-96d7-c2e8a1b5f309 ┆ ide    ┆ 2    ┆ 2026-09-22 14:05 ┆ Add dark mode to… │
│ b8e27f40-91c6-4e3d-8f0a-5d6c3b2a1e97 ┆ agent  ┆ 2    ┆ 2026-09-19 09:40 ┆ Cache the Rust w… │
│ d3c9f7e1-5b84-4a06-b2e9-7f1a6c4d8b20 ┆ ide    ┆ 2    ┆ 2026-09-16 09:02 ┆ Review the OAuth… │
└──────────────────────────────────────┴────────┴──────┴──────────────────┴───────────────────┘
```

Long titles are cut to fit. On narrower terminals the IDs are shortened to whole
UUID groups (8, 13, 18 or 23 characters), with as many groups as it takes to
tell every ID apart, and a footer says so:

```text
IDs shortened to 8 chars; `show` accepts a prefix.
```

When the output is piped or redirected, `list` prints plain columns with full
IDs and titles and no color. This layout is stable for `grep`, `awk` and
scripts:

```text
Found 8 session(s)

ID                                    SOURCE   MSGS  UPDATED           TITLE
----------------------------------------------------------------------------------------------------
a71d0e58-2c39-4f7b-b6a4-19e8c3d5f027  agent       4  2026-10-05 14:12  Add retry with backoff to the webhook sender
e5b8c4d2-6a1f-4e93-8d27-5f0a9b3c1e46  ide         3  2026-10-04 16:48  Speed up the monthly revenue dashboard query
3f9c2a71-8b4e-4d6a-9e15-7c0b2d4f8a63  agent       3  2026-10-02 08:31  Fix flaky timezone test in invoice due dates
9d2f6b13-47ce-4a85-a0b9-e3c51d7f2864  ide         2  2026-09-28 11:09  Explain the migration lock error
6c0e4b9a-3d21-4f87-9a5c-e1b7d2f04a38  agent       2  2026-09-24 15:37  Backfill customer regions in a migration
4a6d1c88-0f3e-4b52-96d7-c2e8a1b5f309  ide         2  2026-09-22 14:05  Add dark mode to the settings page
b8e27f40-91c6-4e3d-8f0a-5d6c3b2a1e97  agent       2  2026-09-19 09:40  Cache the Rust workspace in GitHub Actions
d3c9f7e1-5b84-4a06-b2e9-7f1a6c4d8b20  ide         2  2026-09-16 09:02  Review the OAuth callback handler for CSRF
```

- `UPDATED` is in UTC. A session with no stored update time shows its creation time.
- With no sessions, `list` prints `No sessions found` and exits 0.
- In a terminal, control characters and escape sequences in stored titles and
  messages are removed before printing. The exception is `show` with color on:
  it keeps color and style codes, except blinking and hidden text, and resets
  them at the end of each line. Stored colors can still make text hard to see,
  such as black on black; `--color never` prints messages without them.
  Piped output is written as stored, so a title that contains a line break
  spans two lines there; `list --json` is exact for scripts.

## Filter by source and count

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
  never opened, so a broken store cannot get in the way. When the store named
  was not found but the other one was, a `warning:` line says so.
- `--limit N` (N ≥ 1) keeps the N most recently updated sessions. Only their
  messages are read to count them, so it stays quick on a large history.

## Show a session

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
- `show` reads the messages of that one session only.
- In a terminal, `show` prints the last 20 messages. Piped, it prints all of them.
  `--limit N` prints the last N and `--all` prints everything. The two flags
  cannot be combined.
- `--source agent|ide` works here too. When it leaves the session unfound, the
  hint says which store was not searched.
- The time next to a message is local-time text as the Agent CLI stored it,
  or for IDE messages a UTC time such as `2026-10-04 16:21 UTC`.

## Export

```sh
cursor-session export                                          # every session, Markdown, into ./exports
cursor-session export --format json --session-id 3f9c --out sessions
cursor-session export --format jsonl --workspace /Users/dana/src/billing-api
cursor-session export --format yaml --source ide --limit 10
```

Each session is written to `<out>/<session-id>.<ext>`, one `wrote <path>` line
per file. `--out` defaults to `exports` and is created if missing; a leading `~`
is expanded. Existing files are overwritten. A session ID that is not a plain
file name, which only a damaged or crafted database holds, is written under a
name made from it plus a short hash, so every file stays inside `<out>`. So is
an ID that differs only in case from one exported before it, since macOS and
Windows would take both names for one file. If the `wrote` lines go to a reader
that stops early (`| head`), every file is still written.

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
  CLI sessions record a workspace. A session whose `meta.json` holds no path is
  still found by its exact path, through that hash.
- `--source agent|ide` reads only that store, and `--limit N` exports only the
  N most recently updated of the selected sessions.
- An unknown `--session-id` gives `session not found`, a `--workspace` that
  matches nothing gives `no sessions matched workspace` with a way to list the
  recorded ones, and no sessions at all gives `no sessions to export`. A file or directory that cannot be written gives
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

## Healthcheck

```sh
cursor-session healthcheck
```

```text
Cursor session healthcheck

agent chats: /Users/dana/.cursor/chats (ok)
transcripts: /Users/dana/.cursor/projects/{project}/agent-transcripts (ok)
ide db: /Users/dana/Library/Application Support/Cursor/User/globalStorage/state.vscdb (ok)

sessions loaded: 8 (agent: 4, ide: 4)
```

It shows each location it found, with `(ok)` or `(failed)`, or `not found` and
the paths it looked in. A location reads `(incomplete)` when its store loaded but
a `warning:` line on stderr says what is missing or may be wrong, such as chats
or titles in a format this version does not know. It exits 1 when no storage is
found at all, or when a store that was found fails to load; the report then
includes the reason and what to do about it. When rows or files were skipped, a
`load warnings: N` line says so; `-v` lists them.

## Global flags

`--storage`, `-v`/`--verbose` and `--color` work before or after the subcommand.
`-V`/`--version` works only before it, and `-h`/`--help` after a subcommand
prints that subcommand's options.

| Flag                          | Effect                                                                                               |
| ----------------------------- | ---------------------------------------------------------------------------------------------------- |
| `--storage <PATH>`            | Read only this location instead of detecting Cursor's data. See [`--storage`](storage.md#--storage). |
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
- Control characters in strings are escaped, with JSON's short escapes (`\n`,
  `\r`, `\t`, `\b`, `\f`) or as `\u00XX`, and so are DEL and U+0080 to U+009F
  (as `\u00XX`), so the JSON is the same in a terminal and in a pipe.
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
| `messages`       | array (`show` only)  | Objects with `role` (`"user"` or `"assistant"`, or `"unknown"` for an IDE message whose stored type this version does not know), `content` (string) and `timestamp`. |

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
4a6d1c88-0f3e-4b52-96d7-c2e8a1b5f309  Add dark mode to the settings page
d3c9f7e1-5b84-4a06-b2e9-7f1a6c4d8b20  Review the OAuth callback handler for CSRF

$ cursor-session show a71d --json | jq -r '.messages[] | select(.role == "user") | .content'
Webhook deliveries fail for good on the first 503. Add retries with exponential backoff.
Make the attempt count configurable.

$ cursor-session list --json | jq '[.[] | (.updated_at // .created_at) | select(. != null and fromdate > now - 7 * 86400)] | length'
3
```

## Exit codes and errors

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
