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

┌─────────────────────────┬────────┬──────┬────────┬──────────────────┬────────────────────────┐
│ ID                      ┆ SOURCE ┆ MSGS ┆ TOKENS ┆ UPDATED          ┆ TITLE                  │
╞═════════════════════════╪════════╪══════╪════════╪══════════════════╪════════════════════════╡
│ a71d0e58-2c39-4f7b-b6a4 ┆ agent  ┆ 4    ┆ 175    ┆ 2026-10-05 14:12 ┆ Add retry with backof… │
│ e5b8c4d2-6a1f-4e93-8d27 ┆ ide    ┆ 3    ┆ 80     ┆ 2026-10-04 16:48 ┆ Speed up the monthly … │
│ 3f9c2a71-8b4e-4d6a-9e15 ┆ agent  ┆ 3    ┆ 57     ┆ 2026-10-02 08:31 ┆ Fix flaky timezone te… │
│ 9d2f6b13-47ce-4a85-a0b9 ┆ ide    ┆ 2    ┆ 56     ┆ 2026-09-28 11:09 ┆ Explain the migration… │
│ 6c0e4b9a-3d21-4f87-9a5c ┆ agent  ┆ 2    ┆ 51     ┆ 2026-09-24 15:37 ┆ Backfill customer reg… │
│ 4a6d1c88-0f3e-4b52-96d7 ┆ ide    ┆ 2    ┆ 45     ┆ 2026-09-22 14:05 ┆ Add dark mode to the … │
│ b8e27f40-91c6-4e3d-8f0a ┆ agent  ┆ 2    ┆ 37     ┆ 2026-09-19 09:40 ┆ Cache the Rust worksp… │
│ d3c9f7e1-5b84-4a06-b2e9 ┆ ide    ┆ 2    ┆ 45     ┆ 2026-09-16 09:02 ┆ Review the OAuth call… │
└─────────────────────────┴────────┴──────┴────────┴──────────────────┴────────────────────────┘
IDs shortened to 23 chars; `show` accepts a prefix.
```

That is a 100-column terminal. Long titles are cut to fit. On narrower
terminals the IDs are shortened to whole UUID groups (8, 13, 18 or 23
characters), with as many groups as it takes to tell every ID apart, and the
footer says so: 23 characters at 100 columns, 8 at 80.

`TOKENS` estimates how many tokens the messages `show` prints by default take:
the characters of the user and assistant messages divided by 4, rounded up.
Tool calls and results and titles are not counted. It is an estimate, not any
model's tokenizer count, and it is always the number `show` prints for the same
session. A terminal table has the column only from 79 columns wide; below that
it is left out, so narrow tables look as they did before 0.4.0. In a
78-column terminal:

```text
Found 8 session(s)

┌───────────────┬────────┬──────┬──────────────────┬────────────────────┐
│ ID            ┆ SOURCE ┆ MSGS ┆ UPDATED          ┆ TITLE              │
╞═══════════════╪════════╪══════╪══════════════════╪════════════════════╡
│ a71d0e58-2c39 ┆ agent  ┆ 4    ┆ 2026-10-05 14:12 ┆ Add retry with ba… │
│ e5b8c4d2-6a1f ┆ ide    ┆ 3    ┆ 2026-10-04 16:48 ┆ Speed up the mont… │
│ 3f9c2a71-8b4e ┆ agent  ┆ 3    ┆ 2026-10-02 08:31 ┆ Fix flaky timezon… │
│ 9d2f6b13-47ce ┆ ide    ┆ 2    ┆ 2026-09-28 11:09 ┆ Explain the migra… │
│ 6c0e4b9a-3d21 ┆ agent  ┆ 2    ┆ 2026-09-24 15:37 ┆ Backfill customer… │
│ 4a6d1c88-0f3e ┆ ide    ┆ 2    ┆ 2026-09-22 14:05 ┆ Add dark mode to … │
│ b8e27f40-91c6 ┆ agent  ┆ 2    ┆ 2026-09-19 09:40 ┆ Cache the Rust wo… │
│ d3c9f7e1-5b84 ┆ ide    ┆ 2    ┆ 2026-09-16 09:02 ┆ Review the OAuth … │
└───────────────┴────────┴──────┴──────────────────┴────────────────────┘
IDs shortened to 13 chars; `show` accepts a prefix.
```

When the output is piped or redirected, `list` prints plain columns with full
IDs and titles and no color, `TOKENS` included. This layout is stable for
`grep`, `awk` and scripts:

```text
Found 8 session(s)

ID                                    SOURCE   MSGS    TOKENS  UPDATED           TITLE
----------------------------------------------------------------------------------------------------
a71d0e58-2c39-4f7b-b6a4-19e8c3d5f027  agent       4       175  2026-10-05 14:12  Add retry with backoff to the webhook sender
e5b8c4d2-6a1f-4e93-8d27-5f0a9b3c1e46  ide         3        80  2026-10-04 16:48  Speed up the monthly revenue dashboard query
3f9c2a71-8b4e-4d6a-9e15-7c0b2d4f8a63  agent       3        57  2026-10-02 08:31  Fix flaky timezone test in invoice due dates
9d2f6b13-47ce-4a85-a0b9-e3c51d7f2864  ide         2        56  2026-09-28 11:09  Explain the migration lock error
6c0e4b9a-3d21-4f87-9a5c-e1b7d2f04a38  agent       2        51  2026-09-24 15:37  Backfill customer regions in a migration
4a6d1c88-0f3e-4b52-96d7-c2e8a1b5f309  ide         2        45  2026-09-22 14:05  Add dark mode to the settings page
b8e27f40-91c6-4e3d-8f0a-5d6c3b2a1e97  agent       2        37  2026-09-19 09:40  Cache the Rust workspace in GitHub Actions
d3c9f7e1-5b84-4a06-b2e9-7f1a6c4d8b20  ide         2        45  2026-09-16 09:02  Review the OAuth callback handler for CSRF
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

## Filter by source, count and age

```sh
cursor-session list --source agent --limit 1
```

```text
Found 1 session(s)

ID                                    SOURCE   MSGS    TOKENS  UPDATED           TITLE
----------------------------------------------------------------------------------------------------
a71d0e58-2c39-4f7b-b6a4-19e8c3d5f027  agent       4       175  2026-10-05 14:12  Add retry with backoff to the webhook sender
```

- `--source agent` or `--source ide` reads only that store. The other one is
  never opened, so a broken store cannot get in the way. When the store named
  was not found but the other one was, a `warning:` line says so.
- `--limit N` (N ≥ 1) keeps the N most recently updated sessions. Only their
  messages are read to count them, so it stays quick on a large history.
- `--since DURATION` keeps the sessions updated within that span of now: a
  whole number of at least 1 and a unit, `s`, `m`, `h`, `d` or `w` (`45s`,
  `90m`, `12h`, `30d`, `2w`). The time compared is the one `UPDATED` shows (the
  update time, else the creation time); sessions with neither are left out. It
  is applied before `--limit`, and the sessions it leaves out are not read. The
  same flag works for `search` and `export`. An invalid value, such as `30D` or
  `1.5h`, exits 2.

Run on 2026-10-08, `list --since 7d` keeps the sessions updated since
2026-10-01:

```text
$ cursor-session list --since 7d
Found 3 session(s)

ID                                    SOURCE   MSGS    TOKENS  UPDATED           TITLE
----------------------------------------------------------------------------------------------------
a71d0e58-2c39-4f7b-b6a4-19e8c3d5f027  agent       4       175  2026-10-05 14:12  Add retry with backoff to the webhook sender
e5b8c4d2-6a1f-4e93-8d27-5f0a9b3c1e46  ide         3        80  2026-10-04 16:48  Speed up the monthly revenue dashboard query
3f9c2a71-8b4e-4d6a-9e15-7c0b2d4f8a63  agent       3        57  2026-10-02 08:31  Fix flaky timezone test in invoice due dates
```

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
tokens:    ~175 (estimate)

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
- `tokens:` is the estimate `list` shows as `TOKENS`. It and `messages:` are
  always those of the whole session, whatever `--only`, `--short` or `--limit`
  print.

## Show only some messages, or shorter

`--only ROLES` prints only the messages of those roles, a comma-separated list
of `user`, `assistant` and `tool` (the flag can also be repeated). Tool calls
and their results are read only when `tool` is named; without it they are left
out, as they always were. `--short` cuts each message to its first 300
characters followed by `…` (a message of exactly 300 is not cut), and each tool
call or result to a one-line preview of about 120 characters.

```sh
cursor-session show a71d --only tool --short
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
tokens:    ~175 (estimate)

[tool]
Grep {"pattern":"fn deliver","path":"src/webhooks"}

[tool]
src/webhooks/sender.rs:58: pub async fn deliver(&self, event: &Event) -> Result<(), SendError> {

[tool]
Shell {"command":"cargo test webhooks"}

[tool]
running 9 tests ......... test result: ok. 9 passed; 0 failed; 0 ignored; finished in 0.84s
```

Without `--short`, a result keeps its lines:

```text
$ cursor-session show a71d --only tool --limit 2 | tail -n 7
[tool]
Shell {"command":"cargo test webhooks"}

[tool]
running 9 tests
.........
test result: ok. 9 passed; 0 failed; 0 ignored; finished in 0.84s
```

- A tool call prints as its name and its arguments as compact JSON; its result
  prints as its text. They come from the Agent CLI's `tool_use` and
  `tool_result` parts and from the IDE's tool data. A result stored as a JSON
  object shows its output field (such as `output`, `stdout` or `contents`), or
  the object indented. Images print as `[image]`, other binary parts as
  `[audio]`, `[document]`, `[file]` or `[blob]`, and base64 data as
  `[binary data]`. A call that failed or was stopped ends in `(error)` or
  `(cancelled)`.
- With `tool` among the roles, text the model wrote around a tool call in one
  Agent CLI line prints as separate messages around it; `messages:` still
  counts it as one.
- Messages of a type this version does not know (role `unknown`) print only
  without `--only`.
- `--limit N` and the terminal's default of 20 count only the messages
  selected, and so does the "earlier message(s) omitted" note. When `--only`
  selects no message, a line says `No messages match --only <roles>.`
- The tool output shown is a best reading of formats Cursor does not document,
  and may need adjusting as they change.
- `--short` and `--only` work with `--json` too: the messages are filtered and
  cut in the same JSON shape, and tool messages have the role `"tool"`.

## Search

```sh
cursor-session search retry timeout
```

```text
Found 2 matching session(s)

a71d0e58-2c39-4f7b-b6a4-19e8c3d5f027  agent  2026-10-05 14:12  Add retry with backoff to the webhook sender
  [assistant] I added `RetryPolicy` to `webhooks/sender.rs`: 5 attempts, backoff from 50…

9d2f6b13-47ce-4a85-a0b9-e3c51d7f2864  ide    2026-09-28 11:09  Explain the migration lock error
  [assistant] …stomers` while the migration ran `ALTER TABLE`. Set a `lock_timeout` and retry, or run it outside peak hours.
```

`search` finds the sessions whose messages contain every term of the query.

- Words given as separate arguments are separate terms: `search retry timeout`
  is two. One argument with spaces in it, as the shell passes `"connection
  pool"`, is one phrase, and so is a phrase in double quotes inside an argument:
  `search '"connection pool" timeout'`. A phrase's words match with any
  whitespace between them. A quote mark always starts or ends a phrase, and an
  unclosed one runs to the end.
- Every term is literal text, never a pattern: `a.b`, `c++` and `(x)` match
  themselves. Put `--` before a query that starts with `-`:
  `search -- --force-with-lease`.
- Case is ignored, Unicode-aware (`ÉCOLE` matches `école`, `Σ` matches `σ`),
  with simple case folding, so `ß` does not match `SS`.
- Terms can appear in any order and in different messages of the session.
- The text searched is each message as `show` prints it. Titles, tool calls
  and results, and images are not searched.

Results are ranked: first the sessions with every term in a single message,
then those with more matching messages (messages holding at least one term),
then the most recently updated, then by ID. Each result shows the session's
ID, source, update time and title, then its best message: the one with the
most distinct terms, the earliest of any that tie. That message is put on one
line and cut to `--context` characters (default 60, from 0 to 1000) on each side
of its first match, with `…` where it was cut. In a terminal the matches are
shown in bold red, and IDs are shortened as in the `list` table; piped output is
plain with full IDs.

| Flag                | Effect                                                         |
| ------------------- | -------------------------------------------------------------- |
| `-n`, `--limit N`   | Keep the N best matches                                        |
| `--context N`       | Characters of the best message to show on each side of the match |
| `--source agent\|ide` | Search one store only                                       |
| `--since DURATION`  | Search only the sessions updated within that span, as for `list` |
| `--json`            | Print the matches as JSON; see [JSON output](#json-output)     |

```text
$ cursor-session search the -n 2 --context 30
Found 2 matching session(s)

a71d0e58-2c39-4f7b-b6a4-19e8c3d5f027  agent  2026-10-05 14:12  Add retry with backoff to the webhook sender
  [user] …k deliveries fail for good on the first 503. Add retries with e…

6c0e4b9a-3d21-4f87-9a5c-e1b7d2f04a38  agent  2026-09-24 15:37  Backfill customer regions in a migration
  [user] …t fills customers.region from the billing address country.
```

When nothing matches, `search` prints `error: no sessions match` and exits 1,
with nothing on stdout, also with `--json`. A query with no terms (`search
""`) or more than 64 terms is a usage error and exits 2. A session whose
transcript disappears between listing and searching is left out, with a
warning under `-v`.

## Hand a session off to another agent

When a Cursor session runs out of credits, `handoff` builds a transcript of it
to paste into another agent and copies it to the clipboard.

```sh
cursor-session handoff 9d2f --stdout
```

```text
The following is a transcript from a Cursor session that ran out of credits. Continue from where it ended; do not summarize it back.

[user]
What does `could not obtain lock on relation "customers"` mean during deploy?

[assistant]
Another transaction held a lock on `customers` while the migration ran `ALTER TABLE`. Set a `lock_timeout` and retry, or run it outside peak hours.

[end of transcript: 2 messages, ~110 tokens (estimate)]
```

The transcript is:

1. A preamble paragraph, the one above unless `--preamble TEXT` replaces it or
   `--no-preamble` leaves it out.
2. The session's user and assistant messages, each under a `[user]` or
   `[assistant]` line, as `show --short --only user,assistant` gives them: cut
   to 300 characters plus `…`, without tool calls and results.
3. A trailer with the number of messages and the transcript's token estimate:
   its characters, trailer included, divided by 4 and rounded up.

| Flag               | Effect                                                         |
| ------------------ | -------------------------------------------------------------- |
| `--stdout`         | Print the transcript instead of copying it                     |
| `--limit N`        | Keep only the last N messages                                  |
| `--preamble TEXT`  | Start with TEXT instead of the default preamble                |
| `--no-preamble`    | Start with the first message                                   |
| `--source agent\|ide` | Look for the session in one store only                     |

```text
$ cursor-session handoff 9d2f --stdout --limit 1 --no-preamble
[assistant]
Another transaction held a lock on `customers` while the migration ran `ALTER TABLE`. Set a `lock_timeout` and retry, or run it outside peak hours.

[end of transcript: 1 message, ~54 tokens (estimate)]
```

- The ID can be a unique, case-insensitive prefix, as for `show`.
- Escape sequences and control characters in stored text are removed, so the
  clipboard and a pipe get the same plain text. Each message is cut after they
  are removed, so the 300 characters are visible ones; for such text the
  transcript can differ from what `show --short` prints.
- A line of a message or the preamble that reads as the transcript's own
  structure, a `[user]` or `[assistant]` line or one starting
  `[end of transcript`, gets a leading backslash (`\[assistant]`), so that
  quoted text cannot fake where a message starts or the transcript ends.
  Invisible characters are ignored when deciding that, and carriage returns and
  the Unicode line and paragraph separators count as line breaks.
- `--preamble` takes text that starts with `-`, such as `'- Continue the
  refactor'`. A value that is one of handoff's own options, as in `--preamble
  --stdout`, is taken for a missing text and is a usage error; write
  `--preamble=--stdout` to use it as text. A preamble is trimmed, and a blank
  one is a usage error.

### Where the transcript goes

Without `--stdout`, the transcript is copied to the system clipboard and
`handoff` prints one line, such as `copied 2 messages (~110 tokens) to
clipboard`.

- **macOS:** through `/usr/bin/pbcopy`, with UTF-8 text. If `pbcopy` does not
  finish within 5 seconds, the transcript is printed instead.
- **Windows:** through the system clipboard, which keeps the text after the
  command ends.
- **Linux and other Unix systems with X11:** on X11 the program that copied
  text must keep serving it, so `handoff` starts a copy of itself in the
  background (the hidden `cursor-session serve-clipboard`), in its own process
  group. It takes the clipboard once and serves it until something else is
  copied, the X connection is lost (checked every 10 minutes), or 12 hours
  pass; a clipboard manager then keeps the text. `handoff` says `copied` only
  after the background copy confirms it owns the clipboard, and waits at most
  5 seconds for that. If `handoff` is interrupted in those seconds, it prints
  nothing, but the background copy may still take the clipboard. Over
  `ssh -X`, the background copy keeps the X connection open until something
  else is copied, which can keep the session from closing: use `--stdout`
  there, or copy something else before logging out.
- **Wayland without XWayland:** not supported, as only X11 is built in. Pipe the
  transcript into `wl-copy`: `cursor-session handoff 9d2f --stdout | wl-copy`.

When there is no clipboard (on Linux, `DISPLAY` is not set) or copying fails
for any reason, the transcript is printed to stdout with one `warning: could
not copy to the clipboard (<reason>); printing the transcript` line on stderr,
and `handoff` still exits 0. In a headless or SSH session, use `--stdout`.

A session with no user or assistant messages, only tool calls for example, is
never copied: `handoff` warns `session <id> has no user or assistant messages;
nothing to hand off, so the clipboard was left alone`. With `--stdout` it prints
the transcript of no messages and warns `...; the transcript is empty`.

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
  N most recently updated of the selected sessions. `--since DURATION` exports
  only the sessions updated within that span (see
  [`--since`](#filter-by-source-count-and-age)); it is applied before `--limit`
  and cannot be combined with `--session-id`.
- An unknown `--session-id` gives `session not found`, a `--workspace` that
  matches nothing gives `no sessions matched workspace` with a way to list the
  recorded ones, and no sessions at all gives `no sessions to export`. A
  `--since` that selects nothing gives `no sessions updated in the last 1h to
  export`, or with `--workspace`, ``no sessions of workspace `<w>` were updated
  in the last 1h``. A file or directory that cannot be written gives
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

## Shell completions and man pages

`completions` prints a completion script for bash, zsh or fish:

```sh
# bash
cursor-session completions bash > ~/.local/share/bash-completion/completions/cursor-session
# zsh: then add `fpath+=~/.zfunc; autoload -Uz compinit; compinit` to ~/.zshrc
cursor-session completions zsh > ~/.zfunc/_cursor-session
# fish
cursor-session completions fish > ~/.config/fish/completions/cursor-session.fish
```

Other shells are a usage error. The scripts complete subcommands, flags and
their values, and directories for `export --out`. They also complete the hidden
`man` subcommand.

The hidden `man` command prints a man page in roff, for packagers: the overview
page, or with a command name that command's page.

```sh
cursor-session man > cursor-session.1 && man ./cursor-session.1
cursor-session man search > cursor-session-search.1
```

The same scripts and pages ship in the release archives and the Homebrew
package; see [install.md](install.md#shell-completions-and-man-pages).

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

`list --json` prints an array of sessions, `show --json` prints one session
with its messages, and `search --json` prints the matching sessions.

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
| `message_count`  | integer              | All messages in the session, even when `show --limit` or `--only` returns fewer.                 |
| `token_estimate` | integer              | The estimate `list` shows as `TOKENS`: ceil(characters / 4) of the messages `show` prints by default. Whole session, whatever `show` filters. |
| `messages`       | array (`show` only)  | Objects with `role` (`"user"` or `"assistant"`, `"tool"` with `--only tool`, or `"unknown"` for an IDE message whose stored type this version does not know), `content` (string) and `timestamp`. |

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
    "message_count": 4,
    "token_estimate": 175
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
    "message_count": 3,
    "token_estimate": 80
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
  "token_estimate": 57,
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

`search --json` prints an array of the matching sessions, best first, `[]`
never: no match exits 1 with nothing on stdout. Each item has these keys, in
this order:

| Key                        | Type           | Notes                                                                 |
| -------------------------- | -------------- | --------------------------------------------------------------------- |
| `id`, `title`, `source`, `workspace` | as above |                                                                  |
| `created_at`, `updated_at` | string or null | As in `list --json`. Ranking uses `updated_at`, or `created_at` when it is `null`. |
| `matching_messages`        | integer        | Messages holding at least one term                                    |
| `all_terms_in_one_message` | boolean        | Whether one message holds every term                                  |
| `snippet`                  | object         | The best message: `role`, `text` (plain, cut with `…`) and `message_index`, its 0-based index among the messages `show --json` lists |

```console
$ cursor-session search lock_timeout --json
[
  {
    "id": "9d2f6b13-47ce-4a85-a0b9-e3c51d7f2864",
    "title": "Explain the migration lock error",
    "source": "ide",
    "workspace": null,
    "created_at": "2026-09-28T10:55:12Z",
    "updated_at": "2026-09-28T11:09:44Z",
    "matching_messages": 1,
    "all_terms_in_one_message": true,
    "snippet": {
      "role": "assistant",
      "text": "…n `customers` while the migration ran `ALTER TABLE`. Set a `lock_timeout` and retry, or run it outside peak hours.",
      "message_index": 1
    }
  }
]
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
| 1    | Runtime error: session not found or ambiguous, no sessions match (`search`), unreadable storage, changed storage format, failed healthcheck, nothing to export |
| 2    | Usage error: unknown command or flag, invalid value (`list --limit 0`, `--source web`, `--since 30D`, an empty session ID or `--workspace`, a search query without terms, `--context 1001`), `--limit` together with `--all`, `--since` together with `export --session-id`, one of handoff's options as `--preamble`, missing subcommand |

Errors go to stderr as `error: <message>`, then one `caused by:` line per
underlying cause, then hints:

```text
error: session not found: nope
run `cursor-session list` to see session IDs
```
