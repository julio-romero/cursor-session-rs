# Changelog

All notable changes to cursor-session are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.4.0] - 2026-10-08

The handoff release: search your sessions, read them by role, and hand one
off to another agent when a Cursor session runs out of credits.

### Added

- `search <QUERY>...` finds the sessions whose messages hold every term of a
  query, in any order and any case (Unicode-aware, simple case folding, so `ß`
  does not match `SS`). Separate words are separate terms; a quoted phrase is
  one term whose words match across any whitespace. Terms are literal text,
  never patterns. Sessions with every term in one message rank first, then
  those with more matching messages, then the most recently updated. Each
  result shows its best message cut around the first match (`--context`, 0 to
  1000 characters, default 60), with the matches highlighted in a terminal.
  Flags: `-n/--limit`, `--source`, `--since`, `--json` (items with
  `matching_messages`, `all_terms_in_one_message` and a `snippet` with its
  `message_index`). No match exits 1 with `no sessions match`; a query with no
  terms or more than 64 exits 2. Titles, tool calls and results are not
  searched.
- `--since <DURATION>` on `list`, `export` and `search` (`45s`, `90m`, `12h`,
  `30d`, `2w`): only the sessions updated within that span, applied before
  `--limit`. It cannot be combined with `export --session-id`, and an export
  that selects nothing exits 1 with `no sessions updated in the last <since>
  to export`.
- A token estimate, ceil(characters / 4) of the messages `show` prints by
  default (tool calls and results not counted): a `TOKENS` column in `list`
  (in a terminal table from 79 columns wide, always in piped output), a
  `tokens:` line in `show`, and `token_estimate` in `list --json` and
  `show --json`. It is an estimate, not any model's tokenizer count.
- `show --only user,assistant,tool` prints only those roles. Tool calls
  (`Name {"arg":"value"}`) and their results are read only when `tool` is
  named, from Agent CLI `tool_use`/`tool_result` parts and IDE tool data.
  Failed or stopped calls are marked `(error)` or `(cancelled)`; images and
  binary data print as placeholders such as `[image]`. `--limit` counts only
  the messages selected, and `messages:`/`tokens:` stay those of the whole
  session.
- `show --short` cuts each message to 300 characters plus `…`, and each tool
  call or result to a one-line preview of about 120 characters. It works with
  `--only` and `--json`.
- `handoff <ID>` copies a transcript of a session to the clipboard, for
  another agent to continue: a preamble, the user and assistant messages as
  `show --short --only user,assistant` gives them, and a trailer with the
  message count and the transcript's token estimate. `--stdout` prints it
  instead; `--limit N`, `--preamble TEXT` and `--no-preamble` shape it. Escape
  sequences are removed, and lines that would read as a role header or the
  trailer get a leading backslash. Copying uses `/usr/bin/pbcopy` on macOS,
  the system clipboard on Windows, and X11 on Linux, where a hidden
  `serve-clipboard` helper keeps the text on the clipboard after `handoff`
  exits, until something else is copied or for at most 12 hours. Where no
  clipboard can be reached, or copying fails, it prints the transcript with a
  warning and still exits 0. A session without user or assistant messages is
  never copied.
- `completions <bash|zsh|fish>` prints a completion script, and the hidden
  `man [COMMAND]` prints man pages. The scripts and pages are committed under
  `completions/` and `man/man1/`, and ship in the release archives and the
  Homebrew package (in its share directory; see
  [docs/install.md](docs/install.md#shell-completions-and-man-pages)).
- Library: `cursor_session::search`, `since`, `view` and `handoff` modules,
  `search_sessions`, `load_session_with` returning a `LoadedSession`, and
  `ReadOptions { tools }`.
- `CHANGELOG.md`, `RELEASING.md` (release steps moved out of
  docs/development.md), the 0.3.0 benchmark baselines under
  `bench/results/0.3.0/`, `search` and `handoff` rows in `bench/run.sh`, and a
  "Similar tools" section in the README.
- The demo GIF now shows `search`, `show --only assistant,tool --short` and
  `handoff --stdout`.

### Changed

- `list` tables are one column wider for `TOKENS`, so IDs are shortened
  sooner: to 8 characters at 80 columns (13 before) and 23 at 100 (full IDs
  before). `show` accepts the shortened prefix as before. Below 79 columns the
  table has no `TOKENS` column, and is laid out as in 0.3.0 apart from the
  comfy-table 8 change below.
- `list --json` and `show --json` objects gain the `token_estimate` key after
  `message_count`, and the plain `list` layout a `TOKENS` column.
- The long help mentions `search` and `handoff`, and exit code 1 includes a
  search that matches nothing.
- Replaced the deprecated `serde_yaml` with `serde_norway`; YAML exports are
  unchanged. Updated dependencies, including rusqlite 0.40 (bundled SQLite
  3.53.2) and comfy-table 8. New dependencies: `regex`, `clap_complete`,
  `clap_mangen`, and `arboard` 3.6 without default features on every
  platform but macOS. The minimum Rust version stays 1.88.
- With comfy-table 8, a `list` table with a truncated title can be up to a few
  columns narrower than the terminal (seen at 63 to 65 columns), as the title
  column now fits its content.

### Fixed

- A Cursor IDE chat deleted while it is being read is left out of `list` and
  `search`, and `show`, `handoff` and `export` stop with `changed while it was
  being read`. Before, it listed with 0 messages, and `handoff` copied an empty
  transcript.
- Agent CLI transcript lines that cannot be read, or that are of an unknown
  role, now give a `-v` warning (`skipped N unreadable lines in …`), so
  `healthcheck` counts them under `load warnings`. A last line that is not
  JSON, as a session still being written leaves, is still skipped quietly.
- The `list` table no longer widens the `TITLE` column beyond its content at
  some narrow widths, a comfy-table 7 bug that measured the `…` of a cut title
  in bytes.

### Performance

- Release binaries are stripped and built with one codegen unit. On 0.3.0's
  code that takes the aarch64-apple-darwin binary from 4,022,224 to 3,271,120
  bytes (-18.7%); the new commands then more than use that up (`search` with
  its regex dependency, built with only the Unicode case-folding and Perl-class
  tables it needs, and the completion and man page generators), so the 0.4.0
  binary is 4,756,240 bytes, +18% over 0.3.0.
- `handoff` copies through `pbcopy` on macOS rather than a clipboard crate, so
  no command links AppKit; linking it would have added about 2.5 MB of peak
  memory to every command on macOS.
- The whole release against 0.3.0 at 1000 sessions, release builds run in
  turns by hyperfine on the same store (Apple silicon, macOS): wall time from
  -2.3% to +3.8%, and peak memory about 0.8 MB higher on every command, +5.5%
  to +8.1%, which tracks the larger binary rather than the data read.

  | Command                  | 0.3.0              | 0.4.0              |
  | ------------------------ | -----------------: | -----------------: |
  | `agent list --limit 5`   | 37.5 ms, 10.4 MB   | 37.6 ms, 11.2 MB   |
  | `agent list --json`      | 311.0 ms, 10.9 MB  | 320.0 ms, 11.8 MB  |
  | `agent show (1 session)` | 36.7 ms, 10.8 MB   | 37.2 ms, 11.5 MB   |
  | `ide list --limit 5`     | 15.0 ms, 13.0 MB   | 15.2 ms, 13.7 MB   |
  | `ide list --json`        | 200.0 ms, 14.0 MB  | 207.6 ms, 14.8 MB  |
  | `ide show (1 session)`   | 15.3 ms, 13.0 MB   | 15.0 ms, 13.7 MB   |

  New commands on the same store: `search` over every session 807.9 ms
  (Agent CLI) and 527.1 ms (IDE), at most 16.1 MB; `handoff --stdout` of one
  session as fast as `show`. Full tables in `bench/results/0.4.0/`.
- `search` reads every IDE chat of a database in one read transaction, and
  keeps one session's messages in memory at a time; `tests/memory.rs` checks
  `search`, `list --since`, `export --since`, `show --only tool` and
  `handoff --stdout` stay under 32 MB on about 100 MB of history.

### Known limitations

- `search` reads each session twice: once to count it, as `list` does, and
  once to match it.
- The shapes of tool calls and results are read leniently, as Cursor does not
  document them; they have not been checked against every real format. An
  Agent CLI transcript of only tool activity shows under `--only tool` only
  for a session that is listed.
- With `tool` among the `--only` roles, text the model wrote around a tool
  call in one Agent CLI line prints as separate messages; `messages:` counts
  it as one.
- `handoff` on Linux supports X11 (including XWayland) only; on Wayland
  without XWayland it prints the transcript with a warning, and
  `handoff --stdout | wl-copy` copies it. Interrupting `handoff` during its
  wait of up to 5 seconds can leave the helper holding the transcript, and
  over `ssh -X` the helper keeps the X connection open until something else
  is copied. The X11 helper is covered by unit tests and CI, not yet by a run
  against a real X server.
- The completion scripts also complete the hidden `man` command, and Homebrew
  does not link the completions or man pages into the shells' and `man`'s
  paths.
- The `healthcheck` hint still names only `list`, `show` and `export` as
  taking `--source`.

## [0.3.0] - 2026-10-07

### Changed

- `list` no longer holds every session's messages in memory to count them
  (2f35bf6). Agent transcripts are counted line by line and IDE chats one at a
  time from their key range; `show` reads only the session asked for, and
  `list --limit N` counts only the N it lists. On a 300 MB transcript plus
  159 MB IDE history, peak memory dropped from 574 MB to 15 MB for `list` and
  13 MB for `list --limit 5`, with identical output.
- Library API: `load_sessions` returns summaries without messages,
  `LoadOptions` gains `limit`, and `load_session` and `load_messages` are new.
- The README leads with a demo GIF and getting started; the detailed
  reference moved into `docs/` (407ecb6).

### Added

- `tests/memory.rs` fails when a command peaks above 32 MB on about 100 MB of
  history, measuring each command's own peak (d894074, c43fc43).

### Performance

Measured after the release with `bench/run.sh` (1bb5c81) on 1000 generated
sessions of 200 messages each (`bench/results/0.3.0/`), mean wall time:

| Command                  | 0.3.0    |
| ------------------------ | -------: |
| `agent list --limit 5`   | 41.7 ms  |
| `agent list --json`      | 314.2 ms |
| `agent show (1 session)` | 40.0 ms  |
| `ide list --limit 5`     | 16.2 ms  |
| `ide list --json`        | 197.1 ms |
| `ide show (1 session)`   | 16.1 ms  |

Peak memory at 1000 sessions was 10.4 to 14.0 MB for every command.

## Earlier versions

0.2.0 and earlier are described on the
[GitHub releases](https://github.com/julio-romero/cursor-session-rs/releases)
page.

[0.4.0]: https://github.com/julio-romero/cursor-session-rs/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/julio-romero/cursor-session-rs/compare/v0.2.0...v0.3.0
