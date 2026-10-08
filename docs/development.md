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
without keeping them, and `show` and `export` read one session at a time.

Output rendering is covered by [insta](https://insta.rs) snapshot tests. When
output changes, `cargo test` fails and writes the new output next to the old
snapshot as a `.snap.new` file. To review and accept the changes:

```sh
cargo insta review                        # needs cargo-insta: cargo install cargo-insta
INSTA_UPDATE=always cargo test --locked   # without cargo-insta: accept all, then check git diff
```

`INSTA_UPDATE=always` leaves the `.snap.new` files of earlier runs behind.
Delete them with `find tests/snapshots -name '*.snap.new' -delete`.

## Demo GIF

`demo/demo.gif` is recorded with [VHS](https://github.com/charmbracelet/vhs)
from invented sessions, never real data. `demo/make-fixture.py` writes them to
`demo/home` (ignored by git), and the samples in these docs come from the same
sessions:

```sh
cargo build --release
python3 demo/make-fixture.py
vhs demo/demo.tape
```

To run the CLI against them yourself: `HOME="$PWD/demo/home" target/release/cursor-session list`.

## Releasing

How a release is cut, and the one-time setup it needs, is in
[RELEASING.md](../RELEASING.md).
