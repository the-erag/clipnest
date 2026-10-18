#!/bin/sh
# Packages target/release/clipnest as dist/clipnest_<version>_<arch>.deb.
#   cargo build --release && scripts/build-deb.sh
# Set MAINTAINER="Name <email>" to put a contact address in the package.
set -e
umask 022   # packaged files: 755 / 644

SRC="$(cd "$(dirname "$0")/.." && pwd)"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$SRC/Cargo.toml" | head -n 1)"
ARCH="$(dpkg --print-architecture)"
MAINTAINER="${MAINTAINER:-erag}"
BIN="$SRC/target/release/clipnest"
EXT=clipnest@erag.in
[ -x "$BIN" ] || { echo "Build it first: cargo build --release" >&2; exit 1; }

ROOT="$SRC/dist/deb/clipnest_${VERSION}_${ARCH}"
rm -rf "$ROOT"
install -Dm755 "$BIN" "$ROOT/usr/bin/clipnest"
install -Dm644 "$SRC/data/icons/clipnest.svg" "$ROOT/usr/share/icons/hicolor/scalable/apps/clipnest.svg"
install -Dm644 "$SRC/data/icons/clipnest-panel.svg" "$ROOT/usr/share/icons/hicolor/scalable/apps/clipnest-panel.svg"
install -Dm644 "$SRC/data/icons/clipnest-symbolic.svg" \
    "$ROOT/usr/share/icons/hicolor/symbolic/apps/clipnest-symbolic.svg"
install -Dm644 "$SRC/extension/$EXT/metadata.json" "$ROOT/usr/share/gnome-shell/extensions/$EXT/metadata.json"
install -Dm644 "$SRC/extension/$EXT/extension.js" "$ROOT/usr/share/gnome-shell/extensions/$EXT/extension.js"
install -Dm755 "$SRC/scripts/enable-autopaste.sh" "$ROOT/usr/share/clipnest/enable-autopaste.sh"
install -Dm644 "$SRC/LICENSE" "$ROOT/usr/share/doc/clipnest/copyright"

mkdir -p "$ROOT/usr/share/applications" "$ROOT/etc/xdg/autostart" "$ROOT/DEBIAN"
cat > "$ROOT/usr/share/applications/in.erag.Clipnest.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=Clipnest
GenericName=Clipboard Manager
Comment=Clipboard history manager
Exec=/usr/bin/clipnest history
Icon=clipnest
Terminal=false
Categories=Utility;GTK;
Keywords=clipboard;history;paste;copy;
StartupNotify=false
StartupWMClass=Clipnest
Actions=search;preferences;

[Desktop Action search]
Name=Search History
Exec=/usr/bin/clipnest search

[Desktop Action preferences]
Name=Preferences
Exec=/usr/bin/clipnest preferences
EOF

# Starts at login for every user; each user can switch it off in Preferences.
cat > "$ROOT/etc/xdg/autostart/in.erag.Clipnest.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=Clipnest
Comment=Clipboard history manager
Exec=/usr/bin/clipnest
Icon=clipnest
Terminal=false
NoDisplay=true
X-GNOME-Autostart-enabled=true
X-GNOME-Autostart-Delay=2
EOF

cat > "$ROOT/DEBIAN/control" <<EOF
Package: clipnest
Version: $VERSION
Section: utils
Priority: optional
Architecture: $ARCH
Depends: libgtk-3-0t64 | libgtk-3-0
Recommends: gnome-shell-extension-appindicator
Maintainer: $MAINTAINER
Homepage: https://erag.in
Installed-Size: $(du -sk "$ROOT/usr" "$ROOT/etc" | awk '{s += $1} END {print s}')
Description: clipboard history manager for Linux
 Clipnest keeps everything you copy (text, links, images, files and
 formatted text) and gives it back with one keystroke: Super+Shift+V opens
 a menu right at your text cursor, ready to paste. Made by erag.
EOF

cat > "$ROOT/DEBIAN/postinst" <<'EOF'
#!/bin/sh
set -e
if [ "$1" = configure ]; then
    echo "Clipnest is installed. Log out and back in once: it starts by itself, sets up"
    echo "Super+Shift+V and switches on its GNOME helper extension."
    echo "For automatic pasting on Wayland, run once: /usr/share/clipnest/enable-autopaste.sh"
fi
EOF
chmod 755 "$ROOT/DEBIAN/postinst"

mkdir -p "$SRC/dist"
dpkg-deb --root-owner-group --build "$ROOT" "$SRC/dist/clipnest_${VERSION}_${ARCH}.deb"
