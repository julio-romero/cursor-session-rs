# Development

Back to the [README](../README.md).

```sh
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
```

CI runs clippy and the tests on Linux, macOS and Windows, rustfmt and
`cargo package` (the crate as crates.io receives it) on Linux, and `cargo check`
with the minimum supported Rust version, 1.88.

`tests/memory.rs` (Linux and macOS) builds about 100 MB of history and fails
when any command peaks above 32 MB of resident memory: `list` counts messages
without keeping them, `show`, `export` and `handoff` read one session at a
time, and `search` matches one session at a time and keeps only its hits.
Every new command gets a line there.

Output rendering is covered by [insta](https://insta.rs) snapshot tests. When
output changes, `cargo test` fails and writes the new output next to the old
snapshot as a `.snap.new` file. To review and accept the changes:

```sh
cargo insta review                        # needs cargo-insta: cargo install cargo-insta
INSTA_UPDATE=always cargo test --locked   # without cargo-insta: accept all, then check git diff
```

`INSTA_UPDATE=always` leaves the `.snap.new` files of earlier runs behind.
Delete them with `find tests/snapshots -name '*.snap.new' -delete`.

## Completions and man pages

`completions/` and `man/man1/` hold the shell completion scripts and man pages
the binary generates, committed so that release archives and the Homebrew
package can ship them. `tests/generated.rs` fails when they are stale. After
any change to the CLI (a command, a flag or help text) and after a version
bump, since each man page's title carries the version, regenerate them and
commit the result:

```sh
CURSOR_SESSION_REGENERATE=1 cargo test --locked --test generated
git status completions man
```

## Benchmarks

[`bench/`](../bench) times `list`, `show`, `search` and `handoff` with
[hyperfine](https://github.com/sharkdp/hyperfine) on generated stores of 50,
300 and 1000 sessions and records peak memory. Run it on a quiet machine with
a release build:

```sh
cargo build --release --locked
bench/run.sh                                                  # into bench/results
BENCH_SIZES=50 bench/run.sh target/release/cursor-session /tmp/cs-bench   # quick check
bench/run.sh target/release/cursor-session bench/results/0.4.0           # a release's baseline
```

The tables of two versions compare row by row; [bench/README.md](../bench/README.md)
lists the rows and options. Commit only the Markdown tables of a release's
baseline.

## Demo GIF

`demo/demo.gif` is recorded with [VHS](https://github.com/charmbracelet/vhs)
from invented sessions, never real data. `demo/make-fixture.py` writes them to
`demo/home` (ignored by git), and the samples in these docs come from the same
sessions. The tape shows `list`, `search`, `show --only assistant,tool --short`,
`handoff --stdout` (never the real clipboard) and `export`; it avoids `--since`,
whose output depends on the day it is recorded:

```sh
cargo build --release
python3 demo/make-fixture.py
vhs demo/demo.tape
```

To run the CLI against them yourself: `HOME="$PWD/demo/home" target/release/cursor-session list`.
To check a recording, extract a frame per second with
`ffmpeg -i demo/demo.gif -vf fps=1 /tmp/frame_%03d.png` and look at a few.

## Releasing

How a release is cut, and the one-time setup it needs, is in
[RELEASING.md](../RELEASING.md).
