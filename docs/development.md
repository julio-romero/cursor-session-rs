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
3. Check that `dist plan` prints `announcing vX.Y.Z`. Use dist 0.32.0, the
   version `dist-workspace.toml` pins; a newer dist refuses this configuration.
   Install it with `cargo install cargo-dist --version 0.32.0 --locked`, or with
   `curl --proto '=https' --tlsv1.2 -LsSf https://github.com/axodotdev/cargo-dist/releases/download/v0.32.0/cargo-dist-installer.sh | sh`.
4. Push the tag: `git tag vX.Y.Z && git push origin vX.Y.Z`.

After changing `dist-workspace.toml`, run `dist generate` with that same
version to update `.github/workflows/release.yml`. Moving to a newer dist is a
change of its own: run `dist init` with it and review the regenerated workflow.

The Release workflow builds archives for the five targets, creates the GitHub
Release with the archives, checksums and the shell installer, pushes the
formula to [julio-romero/homebrew-tap](https://github.com/julio-romero/homebrew-tap)
and publishes the crate to crates.io. A pre-release tag such as `v1.0.0-rc.1`
creates a GitHub pre-release and skips Homebrew and crates.io.
