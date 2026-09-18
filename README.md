# cursor-session

List, show, and export chat sessions from **Cursor Agent CLI** and the **Cursor IDE** composer.

This is a Rust rewrite of [iksnae/cursor-session](https://github.com/iksnae/cursor-session). The Go tool skipped Agent storage on macOS, preferred `state.vscdb` over `~/.cursor/chats`, and tried to scrape encrypted `store.db` blobs. This crate does not.

## Storage

Agent sessions (macOS and Linux):

```text
~/.cursor/chats/<md5(cwd)>/<session-id>/meta.json
~/.cursor/projects/<slug>/agent-transcripts/<session-id>/<session-id>.jsonl
```

`store.db` is only used for extra metadata (`name`, `lastUsedModel`). Blobs are not decrypted.

IDE composer sessions:

```text
macOS: ~/Library/Application Support/Cursor/User/globalStorage/state.vscdb
Linux: ~/.config/Cursor/User/globalStorage/state.vscdb
```

Both sources are loaded and merged by session id.

## Install

```bash
cargo install --path .
```

## Usage

```bash
cursor-session list
cursor-session show f4eea6d2-d2d3-41ad-b290-824445295a15
cursor-session export --format md --session-id f4eea6d2-d2d3-41ad-b290-824445295a15
cursor-session healthcheck
```

Global flags: `--storage <path>`, `--verbose`.

Export formats: `jsonl`, `md`, `json`, `yaml`.
