#!/bin/sh
# Builds Clipnest and installs it, then sets it up for the current user
# (GNOME hotkeys Super+Shift+V / Super+Shift+F and launch-at-login).
#
#   ./install.sh            for your user only, in ~/.local (no root needed)
#   ./install.sh --system   for every user, in /usr/local (asks for sudo)
#
# Run it as yourself, not with sudo: building uses your Rust toolchain, and
# the hotkeys and autostart belong to your account.
set -e

SRC="$(dirname "$(readlink -f "$0")")"
USER_PREFIX="$HOME/.local"
if [ "$1" = "--system" ]; then
    PREFIX=/usr/local
    SUDO=sudo
else
    PREFIX="${PREFIX:-$USER_PREFIX}"
    SUDO=
fi
BIN="$PREFIX/bin/clipnest"
APPS="$PREFIX/share/applications"
ICONS="$PREFIX/share/icons/hicolor"
EXT_UUID=clipnest@erag.in
EXTENSIONS="$PREFIX/share/gnome-shell/extensions"

if [ "$(id -u)" = 0 ]; then
    echo "Run ./install.sh as your normal user; it asks for sudo itself when needed." >&2
    exit 1
fi
if [ -x "$SRC/clipnest" ] && [ ! -f "$SRC/Cargo.toml" ]; then
    BUILT="$SRC/clipnest"   # unpacked release tarball: already built
else
    CARGO="$(command -v cargo || echo "$HOME/.cargo/bin/cargo")"
    if [ ! -x "$CARGO" ]; then
        echo "Rust is not installed. Get it with:" >&2
        echo "  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y" >&2
        exit 1
    fi
    if ! pkg-config --exists gtk+-3.0 2>/dev/null; then
        echo "GTK 3 development files are missing:  sudo apt install build-essential pkg-config libgtk-3-dev" >&2
        exit 1
    fi
    echo "Building Clipnest (release)…"
    (cd "$SRC" && "$CARGO" build --release --quiet)
    BUILT="$SRC/target/release/clipnest"
fi

# Stop whichever copy is running so the new one takes over.
for running in "$USER_PREFIX/bin/clipnest" /usr/local/bin/clipnest; do
    if [ -x "$running" ]; then "$running" quit >/dev/null 2>&1 || true; fi
done
sleep 0.5
rm -rf "$USER_PREFIX/lib/clipora"   # the first, Python version

# Clipnest used to be called Clipora: stop it and remove what it installed.
# (Its settings and history move over when Clipnest first starts.)
for old in "$USER_PREFIX/bin/clipora" /usr/local/bin/clipora; do
    if [ -x "$old" ]; then "$old" quit >/dev/null 2>&1 || true; fi
done
OLD_FILES="bin/clipora share/applications/io.clipora.Clipora.desktop
    share/icons/hicolor/scalable/apps/clipora.svg share/icons/hicolor/scalable/apps/clipora-panel.svg
    share/icons/hicolor/symbolic/apps/clipora-symbolic.svg"
for f in $OLD_FILES; do rm -f "$USER_PREFIX/$f"; done
rm -rf "$USER_PREFIX/share/gnome-shell/extensions/clipora@clipora.io"
if [ -e /usr/local/bin/clipora ] || [ -d /usr/local/share/gnome-shell/extensions/clipora@clipora.io ]; then
    echo "Removing the old Clipora files from /usr/local (needs sudo)…"
    for f in $OLD_FILES; do sudo rm -f "/usr/local/$f"; done
    sudo rm -rf /usr/local/share/gnome-shell/extensions/clipora@clipora.io
fi
rm -f "${XDG_CONFIG_HOME:-$HOME/.config}/autostart/io.clipora.Clipora.desktop"
if gsettings writable org.gnome.shell enabled-extensions >/dev/null 2>&1; then
    current="$(gsettings get org.gnome.shell enabled-extensions)"
    updated="$(printf '%s' "$current" | sed "s/, 'clipora@clipora.io'//; s/'clipora@clipora.io', //; s/'clipora@clipora.io'//")"
    [ "$updated" = "[]" ] && updated="@as []"
    [ "$updated" != "$current" ] && gsettings set org.gnome.shell enabled-extensions "$updated"
fi

echo "Installing to $PREFIX"
$SUDO mkdir -p "$PREFIX/bin" "$APPS" "$ICONS/scalable/apps" "$ICONS/symbolic/apps"
$SUDO install -m 755 "$BUILT" "$BIN"
$SUDO install -m 644 "$SRC/data/icons/clipnest.svg" "$ICONS/scalable/apps/clipnest.svg"
$SUDO install -m 644 "$SRC/data/icons/clipnest-panel.svg" "$ICONS/scalable/apps/clipnest-panel.svg"
$SUDO install -m 644 "$SRC/data/icons/clipnest-symbolic.svg" "$ICONS/symbolic/apps/clipnest-symbolic.svg"

$SUDO tee "$APPS/in.erag.Clipnest.desktop" >/dev/null <<EOF
[Desktop Entry]
Type=Application
Name=Clipnest
GenericName=Clipboard Manager
Comment=Clipboard history manager
Exec=$BIN history
Icon=clipnest
Terminal=false
Categories=Utility;GTK;
Keywords=clipboard;history;paste;copy;
StartupNotify=false
StartupWMClass=Clipnest
Actions=search;preferences;

[Desktop Action search]
Name=Search History
Exec=$BIN search

[Desktop Action preferences]
Name=Preferences
Exec=$BIN preferences
EOF
$SUDO update-desktop-database -q "$APPS" 2>/dev/null || true
$SUDO gtk-update-icon-cache -q -t "$ICONS" 2>/dev/null || true

# The "Clipnest helper" GNOME Shell extension tells Clipnest where the text
# cursor is, so the menu can open where you paste (Wayland hides it from apps).
$SUDO mkdir -p "$EXTENSIONS/$EXT_UUID"
$SUDO install -m 644 "$SRC/extension/$EXT_UUID/metadata.json" "$SRC/extension/$EXT_UUID/extension.js" \
    "$EXTENSIONS/$EXT_UUID/"

if [ -n "$SUDO" ]; then
    # A per-user copy would come first on PATH and hide the system one.
    rm -f "$USER_PREFIX/bin/clipnest" \
          "$USER_PREFIX/share/applications/in.erag.Clipnest.desktop" \
          "$USER_PREFIX/share/icons/hicolor/scalable/apps/clipnest.svg" \
          "$USER_PREFIX/share/icons/hicolor/symbolic/apps/clipnest-symbolic.svg" \
          "$USER_PREFIX/share/icons/hicolor/scalable/apps/clipnest-panel.svg"
    rm -rf "$USER_PREFIX/share/gnome-shell/extensions/$EXT_UUID"
    # Its icon cache still lists those files; a stale entry hides the system icon.
    if [ -e "$USER_PREFIX/share/icons/hicolor/icon-theme.cache" ]; then
        gtk-update-icon-cache -q -f -t "$USER_PREFIX/share/icons/hicolor" 2>/dev/null || true
    fi
fi

# Turn the extension on for this user. GNOME only picks up new extensions at
# login; until then Clipnest falls back to the mouse pointer.
HELPER_LOADED=no
if gdbus call --session --dest in.erag.Clipnest.Shell --object-path /in/erag/Clipnest/Shell \
        --method in.erag.Clipnest.Shell.GetPointer >/dev/null 2>&1; then
    HELPER_LOADED=yes
fi
if gsettings writable org.gnome.shell enabled-extensions >/dev/null 2>&1; then
    current="$(gsettings get org.gnome.shell enabled-extensions)"
    case "$current" in
        *"'$EXT_UUID'"*) ;;
        "@as []") gsettings set org.gnome.shell enabled-extensions "['$EXT_UUID']" ;;
        *) gsettings set org.gnome.shell enabled-extensions "${current%]}, '$EXT_UUID']" ;;
    esac
fi

echo "Registering global shortcuts"
"$BIN" setup-shortcuts || true
"$BIN" autostart on
echo "Enabled launch at login"

nohup "$BIN" >/dev/null 2>&1 &
echo
echo "Clipnest is running. Press Super+Shift+V for the history menu, Super+Shift+F to search it."
if [ "$HELPER_LOADED" = no ]; then
    echo "Log out and back in once so GNOME loads the Clipnest helper extension; then the menu opens at your text cursor."
else
    echo "If you changed the helper extension, log out and back in to reload it."
fi
if [ -n "$SUDO" ]; then
    echo "Other users on this computer: run 'clipnest setup-shortcuts' and 'clipnest autostart on' once,"
    echo "and enable the extension:  gnome-extensions enable $EXT_UUID"
fi
if [ ! -w /dev/uinput ] && [ "$XDG_SESSION_TYPE" != "x11" ]; then
    echo "Auto-paste is off until you run (once):  $SRC/scripts/enable-autopaste.sh"
fi
case ":$PATH:" in
    *":$PREFIX/bin:"*) ;;
    *) echo "Note: $PREFIX/bin is not on your PATH yet (log out and back in, or use $BIN)." ;;
esac
