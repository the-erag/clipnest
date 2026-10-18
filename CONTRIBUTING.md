# Contributing to Clipnest

Thanks for helping! Bug reports, ideas and pull requests are all welcome.

## Reporting bugs and asking for features

Open an issue using one of the templates. For bugs, the most useful details are:

- your distribution and version (`cat /etc/os-release`), and Wayland or X11
  (`echo $XDG_SESSION_TYPE`)
- how you installed Clipnest (.deb, tarball or from source) and `clipnest version`
- what you did, what you expected and what happened instead
- the output of Clipnest started from a terminal with `CLIPNEST_DEBUG=1`

## Development setup

You need a C toolchain, the GTK 3 development files and Rust:

```bash
sudo apt install build-essential pkg-config libgtk-3-dev
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
```

Then:

```bash
clipnest quit                                  # only one Clipnest can run at a time
cargo run                                      # run from the source tree
cargo run -- history                           # send it a command
CLIPNEST_HOME=/tmp/clipnest-dev cargo run      # throwaway settings and history
CLIPNEST_DEBUG=1 cargo run                     # trace clipboard reads and tray clicks
```

`CLIPNEST_HOME` also stops the development build from registering hotkeys or
autostart entries for your account.

## Before you open a pull request

CI runs the same checks, so run them first:

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test
```

- Keep pull requests focused: one fix or feature each.
- Add or update tests for logic that can be tested without a display
  (storage, config, menu layout, clipboard capture rules).
- Try clipboard and menu changes in a real GNOME session, on Wayland if you can:
  focus, pointer and clipboard behave differently from X11 there.
- Update `README.md` and `CHANGELOG.md` when behaviour changes.

## How the code is organised

| File | What it does |
|---|---|
| `src/main.rs` | Command line; forwards commands to the running instance |
| `src/app.rs` | The main process; connects everything |
| `src/clipboard.rs` | Watches and sets the clipboard, with all formats |
| `src/storage.rs` | SQLite history, with each entry's extra formats |
| `src/menu.rs` | The menu model, shown as a GTK menu |
| `src/popup.rs` | Search popup |
| `src/tray.rs`, `src/status_icon.rs` | Top-bar icon |
| `src/preferences.rs` | Preferences window |
| `src/paste.rs` | Auto-paste (virtual keyboard via uinput, or xdotool) |
| `src/system.rs` | GNOME shortcuts, autostart, first-run setup |
| `extension/` | "Clipnest helper" GNOME Shell extension |
| `scripts/` | Packaging and the auto-paste setup script |

## Releasing (maintainers)

1. Update the version in `Cargo.toml`, run `cargo build` so `Cargo.lock`
   follows, and add a section to `CHANGELOG.md`.
2. Commit, then tag and push:

   ```bash
   git tag v0.0.2
   git push origin main v0.0.2
   ```

3. The **Release** workflow checks that the tag matches `Cargo.toml`, runs the
   tests, builds the `.deb` and the tarball, and publishes them with
   `SHA256SUMS` as a GitHub release.

## Code of conduct

By taking part you agree to follow the [code of conduct](CODE_OF_CONDUCT.md).
