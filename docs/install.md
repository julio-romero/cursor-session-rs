# Installing cursor-session

Back to the [README](../README.md).

## Homebrew (macOS and Linux)

```sh
brew install julio-romero/tap/cursor-session
```

On Linux the binary needs glibc 2.35 or newer (Ubuntu 22.04, Debian 12,
Fedora 36 or later). On older systems, such as RHEL 9 or Amazon Linux 2023,
or on musl-based ones such as Alpine, use `cargo install` instead.

## Shell installer (macOS and Linux)

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/julio-romero/cursor-session-rs/releases/latest/download/cursor-session-installer.sh | sh
```

The script downloads the binary for your platform (macOS on Apple silicon or
Intel, Linux on x86_64 or arm64 with glibc 2.35 or newer) from the latest GitHub
Release and installs it to `$CARGO_HOME/bin`, which defaults to `~/.cargo/bin`.
If that directory is not on your `PATH`, it adds it in your shell profile. On
an older glibc the script stops with `no compatible downloads were found for
your platform`; use `cargo install` there.

To install somewhere else, set `CURSOR_SESSION_INSTALL_DIR`. The binary goes in
its `bin` subdirectory, so this installs `~/.local/bin/cursor-session`:

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/julio-romero/cursor-session-rs/releases/latest/download/cursor-session-installer.sh | CURSOR_SESSION_INSTALL_DIR="$HOME/.local" sh
```

Set `CURSOR_SESSION_NO_MODIFY_PATH=1` as well to leave your shell profiles alone.

## Cargo

```sh
cargo install cursor-session --locked
```

This builds from source and needs Rust 1.88 or newer.

**Windows:** download `cursor-session-x86_64-pc-windows-msvc.zip` from the
[latest release](https://github.com/julio-romero/cursor-session-rs/releases/latest)
and put `cursor-session.exe` on your `PATH`, or use `cargo install`.

## Upgrade and uninstall

- **Homebrew:** `brew upgrade cursor-session`, and `brew uninstall
  cursor-session` to remove it.
- **Shell installer:** run the install command again to upgrade. To remove it,
  delete `cursor-session` from `$CARGO_HOME/bin` (`~/.cargo/bin` by default, or
  the `bin` directory under `CURSOR_SESSION_INSTALL_DIR`) and the
  `~/.config/cursor-session` directory (under `XDG_CONFIG_HOME` if that is set),
  where the installer keeps a record of the install. The installer may also have added a line to your shell profile
  that loads `~/.cargo/env`; keep it if you use Rust.
- **Cargo:** `cargo install cursor-session --locked` again to upgrade, and
  `cargo uninstall cursor-session` to remove it. The shell installer and Cargo
  both install to `~/.cargo/bin`, so to move from the installer to Cargo, add
  `--force`; without it, Cargo stops with `binary cursor-session already exists
  in destination`.
