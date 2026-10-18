# Changelog

All notable changes to Clipnest are listed here. Versions follow
[semantic versioning](https://semver.org/).

## 2.0.0

The first release under the name Clipnest (it was called Clipora before).

### Added

- Rewritten in Rust; uses about half the memory of the previous version.
- The hotkey menu (`Super+Shift+V`) opens just below the text cursor, with the
  new "Clipnest helper" GNOME Shell extension, or at the mouse pointer.
- Clicking the top-bar icon opens the same menu right below it, with folders
  that open on hover.
- Every format of a copy is kept (plain text, HTML, RTF, PDF, files, images),
  so pastes keep their formatting; hold Shift to paste plain text.
- `.deb` package and release tarball, built automatically for every release.
- First-run setup: a freshly installed Clipnest registers its hotkeys, enables
  its GNOME extension and starts at login on its own.

### Changed

- The top-bar menu shrinks to fit the screen and puts older items in folders.
- Settings and history move over automatically from Clipora.

### Fixed

- Formatted copies no longer lose their HTML when the app they came from quits.
- Copied files no longer also appear as a separate text entry.
