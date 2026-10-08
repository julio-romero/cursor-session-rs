# Releasing

Back to the [README](README.md). Building and testing are covered in
[docs/development.md](docs/development.md).

Releases are built by [cargo-dist](https://github.com/axodotdev/cargo-dist)
when a version tag is pushed. Before the first release, once:

- Create the public repository `julio-romero/homebrew-tap` with at least one
  commit (a README is enough).
- Add the secret `HOMEBREW_TAP_TOKEN` to this repository: a fine-grained
  personal access token with Contents read and write access to the tap only.
- Add the secret `CARGO_REGISTRY_TOKEN`: a crates.io API token with the
  publish-new and publish-update scopes, from an account with a verified email.

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
