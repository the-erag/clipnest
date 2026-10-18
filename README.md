# Clipnest

**Your clipboard, remembered.** A fast clipboard history manager for Linux, made
by [erag](https://erag.in).

Clipnest keeps everything you copy (text, links, images, files and formatted
text) and gives it back with one keystroke: press `Super+Shift+V` and a menu
opens right at your text cursor, ready to paste. Built for GNOME/Ubuntu, works
on Wayland and X11, and is written in Rust with GTK 3.

## Features

- **Full history.** Text, links, images and copied files. Every format of a
  copy is kept (plain text, HTML, RTF, PDF, file lists, images), so pastes keep
  their formatting; hold `Shift` to paste plain text instead. Copying the same
  thing again moves it back to the top, and pinned items (★) survive clearing.
- **A menu where you type.** `Super+Shift+V` opens the menu just below your text
  cursor (or at the mouse pointer). Recent items are numbered, so `1`…`9` and
  `0` paste them straight away; older ones sit in folders that open on hover.
  Images show as thumbnails. The bottom row holds search, clear, preferences and
  quit.
- **Top-bar icon.** Click it for the same menu, right below the icon.
- **Search.** `Super+Shift+F` opens a popup that filters the whole history as you
  type.
- **Auto-paste.** Picking an item pastes it into the app you were using.
- **Private.** History never leaves your computer. Copies that password managers
  mark as secret are skipped, and you can exclude anything matching your own
  patterns (for example `^\d{6}$` for one-time codes).
- **Light.** About 22 MB of its own memory with every window opened once.

## Install

### From a release (recommended)

Download the latest `.deb` from the [Releases page](../../releases/latest) and
install it:

```bash
sudo apt install ./clipnest_*_amd64.deb
```

Then log out and back in once. Clipnest starts by itself, sets up
`Super+Shift+V` / `Super+Shift+F` and switches on its small GNOME helper
extension.

On other distributions, download the `clipnest-*-linux-x86_64.tar.gz` file,
unpack it and run:

```bash
./install.sh            # for your user, in ~/.local
./install.sh --system   # or for everyone, in /usr/local (asks for sudo)
```

Release builds work on Ubuntu 22.04 and newer.

### From source

```bash
sudo apt install build-essential pkg-config libgtk-3-dev
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
git clone <this repository> && cd clipnest
./install.sh            # or ./install.sh --system
```

### Turning on auto-paste (Wayland, once)

Wayland doesn't let one app type into another, so Clipnest pastes through a
small virtual keyboard (`/dev/uinput`), which only root may use by default.
This script gives the logged-in user access with a udev rule, like the one
Steam installs for game controllers:

```bash
scripts/enable-autopaste.sh                     # from the source or tarball
/usr/share/clipnest/enable-autopaste.sh         # after installing the .deb
```

The trade-off: any program you run can then create virtual input devices. If you
skip this, picking an item copies it and you press `Ctrl+V` yourself. On X11,
Clipnest uses `xdotool` if it's installed.

## Using it

| Where | Action | Result |
|---|---|---|
| anywhere | `Super+Shift+V` | the menu, at your text cursor |
| anywhere | `Super+Shift+F` | search popup |
| menu | `1` … `9`, `0` / arrows + `Enter` | paste that item |
| menu | `Shift` + digit / `Shift`-click | paste as plain text |
| menu | `Esc`, or click elsewhere | close |
| popup | type | filter (every word must match) |
| popup | `Enter` / click | paste |
| popup | `Shift+Enter` | paste as plain text |
| popup | `Ctrl+Enter` | copy only, no paste |
| popup | `Alt+1` … `Alt+0` | pick item 1…10 |
| popup | `Ctrl+P` / `Shift+Delete` | pin / delete |
| top-bar icon | left or right click | the menu, below the icon |
| top-bar icon | middle click | search popup |

Change the hotkeys in **Preferences → Shortcuts** or in GNOME Settings →
Keyboard → Custom Shortcuts. Everything else is in **Preferences**: history size,
menu layout and width, which formats to keep, exclusions, launch at login.

### Commands

```
clipnest                   start in the background
clipnest history           show the menu
clipnest search            toggle the search popup
clipnest preferences       open Preferences
clipnest clear             clear history (asks first)
clipnest quit              stop Clipnest
clipnest setup-shortcuts   (re)register the GNOME hotkeys
clipnest remove-shortcuts  remove them
clipnest autostart on|off  launch at login, or not
clipnest version           print the version
```

## How it works

Wayland keeps apps apart, which a clipboard manager has to work around:

- **Reading the clipboard.** Only the focused window may read the Wayland
  clipboard, so Clipnest runs on XWayland, where GNOME mirrors the clipboard and
  changes can be watched from the background.
- **Hotkeys.** Apps can't grab global keys, so Clipnest registers GNOME custom
  shortcuts. They run `clipnest history`, which passes the command to the running
  Clipnest over D-Bus.
- **Finding the text cursor.** Apps can't see other apps' cursors, but GNOME
  Shell can, because input methods report the cursor to it. The "Clipnest helper"
  extension (`extension/`) passes that position on. Without it, Clipnest falls
  back to the mouse pointer.
- **Top-bar icon.** On Ubuntu, Clipnest uses a legacy tray icon so that clicks
  reach it and it can show its own menu. Where that isn't available, it uses an
  AppIndicator instead, and the desktop draws the menu. On other GNOME
  distributions, install the
  [AppIndicator extension](https://extensions.gnome.org/extension/615/appindicator-support/).
- **Other desktops.** Bind `clipnest history` and `clipnest search` to keys in
  your desktop's keyboard settings.

## Files

| Path | Contents |
|---|---|
| `~/.config/clipnest/config.json` | preferences |
| `~/.local/share/clipnest/clipnest.db` | history (SQLite) |
| `~/.local/share/clipnest/images/` | copied images (PNG) |

Clipnest was called Clipora before 2.0. Its settings and history move over
automatically on first start.

## Uninstall

```bash
sudo apt remove clipnest    # if you installed the .deb
./uninstall.sh              # if you used install.sh (keeps your history)
./uninstall.sh --purge      # …and deletes the history too
```

## Contributing

Bug reports, ideas and pull requests are welcome. See
[CONTRIBUTING.md](CONTRIBUTING.md) for the development setup and checks, and
[SECURITY.md](SECURITY.md) for reporting security problems privately. Please
follow the [code of conduct](CODE_OF_CONDUCT.md).

## License

[MIT](LICENSE) © 2026 [erag](https://erag.in)
