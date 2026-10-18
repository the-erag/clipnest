# Changelog

All notable changes to Clipnest are listed here. Versions follow
[semantic versioning](https://semver.org/).

## 0.0.1

The first release of Clipnest.

- Clipboard history for text, links, images and copied files. Every format of a
  copy is kept (plain text, HTML, RTF, PDF, file lists, images), so pastes keep
  their formatting; hold Shift to paste plain text.
- `Super+Shift+V` opens the history menu just below the text cursor (with the
  "Clipnest helper" GNOME Shell extension) or at the mouse pointer. Items are
  numbered, so `1`…`9` and `0` paste them, and older items sit in folders that
  open on hover.
- Top-bar icon that opens the same menu right below it, sized to fit the screen.
- `Super+Shift+F` search popup that filters the whole history as you type.
- Auto-paste into the app you were using, through a virtual keyboard on Wayland
  or `xdotool` on X11.
- Pinned items, exclusion patterns, and skipping of copies that password
  managers mark as secret.
- Preferences for history size, menu layout, formats to keep, exclusions,
  hotkeys and launch at login.
- First-run setup: Clipnest registers its hotkeys, enables its GNOME extension
  and starts at login on its own.
- `.deb` package and release tarball, built automatically for every release.
