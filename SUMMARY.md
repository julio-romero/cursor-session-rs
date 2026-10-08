# cursor-session 0.4.0, "the handoff release": summary

Everything is on `release/0.4.0`, built from local `master` (1bb5c81, "added benchmark tool", which is not on `origin/master` yet). Each
track was built on its own branch and merged with `--no-ff`, so every
track's history is kept. Nothing was pushed to `master`.

The work ran in four stages:

1. **Five tracks**, each built in its own worktree and checked by three
   independent verifiers (gates, spec conformance, hard-rules/code review).
   Fix rounds ran until no verifier found anything blocking.
2. **Merge and Finish**: the tracks were merged in the order 1, 5, 2, 3, 4,
   then audited line by line to confirm no track lost code or tests in
   conflict resolution. Then came the Finish step and a docs check that ran
   every documented example.
3. **Blind verification** by two agents that had seen no diffs: a release
   engineer and an adversarial user. A skeptic tried to refute each finding
   before anything was fixed.
4. **Final verification** of the fixes that blind verification produced, then
   the benchmarks.

## What shipped

### Track 1: housekeeping
- `serde_yaml` is replaced with `serde_norway`. YAML exports are
  byte-identical: the snapshots are unchanged, and about 60 tricky strings
  were compared between the old and new builds.
- Ran `cargo update`. rusqlite went from 0.32 to 0.40.2 (bundled SQLite
  3.46.0 to 3.53.2) and comfy-table from 7 to 8.0.1. `src/sqlite.rs` needed
  no change. The minimum Rust version stays 1.88.
- The release steps moved to `RELEASING.md`, without the "make this
  repository public" step. They came from `docs/development.md`, not the
  README: the README only linked there.
- The README has a "Similar tools" section crediting iksnae/cursor-session,
  S2thend/cursor-history and SpecStory. The claims were checked against each
  project's own pages and worded conservatively.
- `[profile.dist]` now sets `strip = true` and `codegen-units = 1`. Sizes are
  under Bench below.
- `bench/` was already committed (1bb5c81). The README now references it. The
  0.3.0 and 0.4.0 tables are committed under `bench/results/`, and only the
  Markdown tables are tracked.

### Track 2: search
- `search <QUERY>...`: every term must appear in the session, in any order and
  any case. Matching is Unicode-aware with simple case folding. A quoted
  phrase is one term; terms are literals. Each term is passed through
  `regex::escape`, and one `RegexSet` with `case_insensitive(true)` makes one
  pass per message.
- Ranking: sessions with all terms in one message come first, then more
  matching messages, then the newest, then ID.
- Output: ripgrep-style blocks with a highlighted snippet of the best message.
  Flags: `-n/--limit`, `--context`, `--source`, `--since`, `--json`. No match
  exits 1 with `no sessions match`.
- Pure functions `parse_query`, `matches -> TermMask`, `score -> Option<Score>`,
  `rank` and `render`, with proptest properties for score and rank (all terms
  in one message always outranks spread terms; the order is independent of
  input order).
- Search streams: Agent CLI transcripts are read line by line, IDE chats one at
  a time inside a single read transaction, and only each session's score and
  snippet are kept.
- `--since <30d|12h|90m|1w|45s>` also works on `list` and `export`.

### Track 3: show filters and token estimates
- `show --only user,assistant,tool` filters by role. `show --short` cuts each
  message to 300 characters and tool calls/results to one-line previews. Both
  work with `--json` and `--limit`.
- Tool messages did not exist in 0.3.0: the readers dropped them. They are now
  read only when `--only` includes `tool`, from Agent CLI
  `tool_use`/`tool_result` parts and IDE `toolFormerData`. Default output is
  byte-identical, and the existing show snapshots prove it.
- Token estimate, ceil(chars / 4):
  - a `TOKENS` column in `list` (in a terminal table from 79 columns wide;
    always in piped output)
  - a `tokens:` line in the `show` header
  - `token_estimate` in `--json`
  - It is documented as an estimate, and tests prove `list` and `show` agree
    for both stores.

### Track 4: handoff
- `handoff <ID>` builds a transcript with the preamble from the spec, the
  messages as `show --short --only user,assistant` gives them, and a trailer
  with the transcript's token estimate.
- By default it copies the transcript and prints
  `copied N messages (~T tokens) to clipboard`. `--stdout`, `--limit N`,
  `--preamble <text>` and `--no-preamble` are supported.
- If no clipboard can be reached, or copying fails, it prints the transcript
  with a warning and exits 0. It never fails because of the clipboard.
- The `--stdout` form is snapshot-tested for both stores.
- Hardening beyond the spec:
  - Escape sequences are stripped.
  - Lines that would read as a role header or the trailer, including ones
    disguised with zero-width characters or Unicode line separators, get a
    leading backslash.
  - A session with nothing to hand off never overwrites the clipboard.
- Tests use a fake clipboard, so `cargo test` never touches the real one.

### Track 5: completions and man pages
- `cursor-session completions <bash|zsh|fish>` and a hidden
  `cursor-session man [COMMAND]` generate completions with clap_complete and
  man pages with clap_mangen.
- Generated files are committed under `completions/` and `man/man1/`. A test
  fails when they go stale, and it says how to regenerate them.
- The files are added to dist's `include`, so every release archive carries
  them.
- `release.yml` was regenerated with dist 0.32.0.
- Also fixed a clap_complete bug: with a hyphenated binary name, bash
  completion matched nothing after a subcommand.

### Finish
- Version 0.4.0.
- `--help` examples for every new command; the long help mentions search and
  handoff.
- README Features/Usage, `docs/usage.md`, `docs/install.md` and
  `docs/development.md`. Every example was run.
- `CHANGELOG.md` with 0.4.0 and 0.3.0 entries; 0.3.0 cites its bench numbers.
- `bench/run.sh` gained search and handoff rows.
- **Your request:** the VHS demo now shows `list` with TOKENS, `search` with
  highlighting, `show --only assistant,tool --short`, `handoff --stdout | head`
  and `export`. `demo/demo.gif` was re-recorded, about 33 s.

## Bench: before and after

Paired: hyperfine ran 0.3.0 and 0.4.0 in turn on the same 1000-session store
(about 300 KB per session; both builds use the release profile, on Apple
silicon). Nothing else was running.

| Command | 0.3.0 time | 0.4.0 time | Δ time | 0.3.0 peak RSS | 0.4.0 peak RSS | Δ RSS |
|---|---:|---:|---:|---:|---:|---:|
| `agent list --limit 5` | 37.0 ms | 37.0 ms | +0.1% | 10.4 MB | 11.2 MB | +8.0% |
| `agent list --json` | 306.5 ms | 317.1 ms | +3.5% | 11.0 MB | 11.8 MB | +7.7% |
| `agent show (1 session)` | 36.1 ms | 36.5 ms | +0.9% | 10.7 MB | 11.6 MB | +8.1% |
| `ide list --limit 5` | 15.0 ms | 15.1 ms | +1.0% | 13.0 MB | 13.8 MB | +6.1% |
| `ide list --json` | 200.1 ms | 206.3 ms | +3.1% | 14.0 MB | 14.9 MB | +6.4% |
| `ide show (1 session)` | 14.6 ms | 14.8 ms | +1.4% | 13.0 MB | 13.8 MB | +6.1% |

New commands at 1000 sessions: `search` over every session 807 ms (Agent CLI) and 535 ms (IDE), peak 13.5 / 16.3 MB; `handoff --stdout` of one session 36.5 ms / 15.1 ms (the same as `show`).

Every command is within the 10% budget, on both time and peak memory. Memory
is the closer of the two: about +0.8 MB on every command, which follows the
larger binary rather than the data read.

Full tables from `bench/run.sh` (50/300/1000 sessions) are in
`bench/results/0.3.0/` and `bench/results/0.4.0/`.

Binary size (aarch64-apple-darwin, `cargo build --profile dist`):

| Build | Bytes |
|---|---:|
| 0.3.0 | 4,022,224 |
| 0.3.0's code with `strip` + `codegen-units = 1` | 3,271,120 (-18.7%) |
| 0.4.0 | 4,772,768 (+18.7% over 0.3.0) |

The new commands more than use up what the profile change saved. Most of the
growth is search: the regex engine itself, not its Unicode tables. I built
regex with only the Unicode tables search needs (case folding, `\s`), which
saved 231 KB and costs nothing at runtime.

## Deviations from the brief, and why

1. **macOS copies with `/usr/bin/pbcopy`, not arboard (your decision).**
   Linking arboard makes every command load AppKit: +2.5 MB max RSS, +18-24%
   in the bench table. arboard is still used on Linux (X11) and Windows. The
   0.4.0 binary links the same three libraries as 0.3.0.
2. **Tool messages are opt-in (`--only ...,tool`).** The 0.3.0 readers had no
   tool role, and building tool messages unconditionally would have changed
   default output.
3. **`handoff` follows "same as show --short --only user,assistant"
   literally**, so it includes no tool messages. The spec also mentioned tool
   output previews, which would only apply if `tool` were included.
4. **"Releasing" was in `docs/development.md`, not the README.** It moved from
   there, and both places now link `RELEASING.md`.
5. **Homebrew does not put completions where shells find them.** dist 0.32's
   formula template has no hook for completions. Files from `include` land in
   `$(brew --prefix)/share/cursor-session/`, so `docs/install.md` gives the
   one-line setup per shell. The alternative, a custom job that patches the
   tap's formula, would be fragile.
6. **A hidden `serve-clipboard` subcommand was added (Linux/X11 only).** On
   X11 the program that copied text must keep serving it, so `handoff` starts
   this helper in the background. It is bounded to 12 hours, and it is left
   out of `--help`, the man pages and the completion scripts.
7. **Some existing test assertions changed** wherever a track was allowed to
   change output (details below).

## Not done, or left as is

- The first line of the top-level about ("List, show, and export ...") and
  the healthcheck hint ("`list`, `show` and `export` accept `--source`") are
  unchanged, because existing tests assert that exact text. Both are still
  true. The long help mentions search and handoff.
- **`search` reads each session twice.** The first read is the same counting
  pass `list` does, which chooses among duplicate transcripts and catches IDE
  format changes. The second read matches. A single-pass search needs a new
  loader. It isn't a regression of an existing command, so it's left for
  0.4.x.
- **The `list` table on narrow terminals is unchanged.** Dates wrap at 60-64
  columns. That layout is locked in by the existing snapshots, and comfy-table
  8 changes it slightly at 63-65 columns (two new snapshots pin it).
- **The `TOKENS` column takes room from the ID column.** Table IDs are
  shortened sooner: 8 characters at 80 columns (13 before) and 23 at 100 (full
  before). `show` accepts the prefix, and this is documented.
- **The Linux X11 and Windows clipboard paths could not be run here (macOS
  only).** They compile with clippy `-D warnings` for both targets, the Linux
  parent-side protocol is unit-tested, and CI runs the Linux-only CLI test.
- **The man pages trigger mandoc lint warnings.** These are cosmetic, from
  clap_mangen. The empty `.TH` date is deliberate so the pages are
  reproducible.

## Edits to existing tests and snapshots

The "every existing test and snapshot must pass unchanged" rule was broken
only where a track was allowed to change output, or where the track's own
spec required it:

- **Snapshots (Track 3, tokens):**
  - `list_table_080/100/100_color/120/160/200`, `list_plain`,
    `list_plain_color`: the TOKENS column
  - all six `show_*`: one `tokens:` line
  - `cli_list_json`, `cli_show_agent_json`, `cli_show_ide_json`:
    `token_estimate`
  - Every `export_*` snapshot is byte-identical to 0.3.0.
- **Unit tests (Track 3):**
  - `id_prefix_width(80)` went from 13 to 8, and two table tests moved to
    wider terminals, because of the TOKENS column.
  - The show JSON key count went from 10 to 11.
  - `SUMMARY_KEYS` gained `token_estimate`.
- **`tests/cli.rs`:** the `SUBCOMMANDS` list gained the new commands.
- **Track 1:** `serde_yaml::from_str` became `serde_norway::from_str` in two
  tests.
- **`tests/generated.rs`:** after the merge, bash completion of `list --s`
  also offers `--since`.
- **`agent.rs`:** a test of transcript counting expects 6 messages instead of
  4. Two input lines were added to it, and they are what it now also counts.
- **`tests/memory.rs`:** grew from 6 to 21 commands. The list-vs-show count
  check now covers both stores and the token estimates.

## Verification

- **Gates on the final head:**
  - `cargo fmt --check`
  - `cargo clippy --all-targets --locked -D warnings`
  - `cargo test --locked`: 364 tests, including `tests/memory.rs` (21
    commands under 32 MB on about 100 MB of history)
  - `cargo +1.88 check --locked`
  - `cargo package --locked`, plus the packaged crate's unit tests
- **Blind verification:** every documented example reproduced byte for byte,
  and all subcommands and flags were exercised against the fixture, demo and
  300-session stores. The read-only check (hashes before and after) found no
  writes to any store. The real macOS clipboard was used once, with your
  clipboard saved and restored.
- **Fixes from blind verification**, each reproduced by a skeptic, then fixed
  and verified over two more rounds of three verifiers:
  - **IDE chat deleted mid-read** (while Cursor runs): it used to show 0
    messages, and handoff gave an empty transcript. Now `list` and `search`
    leave it out, and `show`/`handoff`/`export --session-id` fail with
    "session <id> was deleted while it was being read". A bulk `export`
    skips it with a warning and exports the rest.
  - **Unreadable Agent CLI transcript lines** were dropped silently. They now
    give one `-v` warning and count in `healthcheck`. Lines of the
    never-shown `system`/`tool` roles and a cut-off last line stay quiet.
  - **`--since -1d`** gave clap's confusing "unexpected argument" error. It
    now reports an invalid `--since` value, and `--since --json` still
    reports the missing value.
  - **The `-shm` sidecar.** Reading a WAL database whose `-shm` file is
    orphaned rewrites the `-shm`. The skeptics judged this not a bug: SQLite
    does it for every reader, it's documented in `docs/storage.md`, and
    `src/sqlite.rs` was off-limits. The database file and its `-wal` are
    never written.
- **Not run here:** Linux and Windows test runs (CI covers them), fish
  completions (fish is not installed here), and a real Homebrew install.
