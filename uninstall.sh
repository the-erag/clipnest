#!/bin/sh
# Removes Clipnest (both a per-user and a system-wide install). The history
# stays unless you pass --purge. Asks for sudo only if a system-wide copy exists.
set -e
USER_PREFIX="$HOME/.local"
SYSTEM_PREFIX=/usr/local
EXT_UUID=clipnest@erag.in

for bin in "$USER_PREFIX/bin/clipnest" "$SYSTEM_PREFIX/bin/clipnest"; do
    if [ -x "$bin" ]; then
        "$bin" quit >/dev/null 2>&1 || true
        "$bin" remove-shortcuts || true
        "$bin" autostart off || true
        break
    fi
done

files() {
    echo "$1/bin/clipnest"
    echo "$1/share/applications/in.erag.Clipnest.desktop"
    echo "$1/share/icons/hicolor/scalable/apps/clipnest.svg"
    echo "$1/share/icons/hicolor/symbolic/apps/clipnest-symbolic.svg"
    echo "$1/share/icons/hicolor/scalable/apps/clipnest-panel.svg"
    echo "$1/share/gnome-shell/extensions/$EXT_UUID/metadata.json"
    echo "$1/share/gnome-shell/extensions/$EXT_UUID/extension.js"
}
files "$USER_PREFIX" | xargs rm -f
rmdir "$USER_PREFIX/share/gnome-shell/extensions/$EXT_UUID" 2>/dev/null || true
rm -rf "$USER_PREFIX/lib/clipora"   # leftovers of the first, Python version
if gsettings writable org.gnome.shell enabled-extensions >/dev/null 2>&1; then
    current="$(gsettings get org.gnome.shell enabled-extensions)"
    updated="$(printf '%s' "$current" | sed "s/, '$EXT_UUID'//; s/'$EXT_UUID', //; s/'$EXT_UUID'//")"
    [ "$updated" = "[]" ] && updated="@as []"
    [ "$updated" != "$current" ] && gsettings set org.gnome.shell enabled-extensions "$updated"
fi
if [ -e "$USER_PREFIX/share/icons/hicolor/icon-theme.cache" ]; then
    gtk-update-icon-cache -q -f -t "$USER_PREFIX/share/icons/hicolor" 2>/dev/null || true
fi
if [ -e "$SYSTEM_PREFIX/bin/clipnest" ]; then
    echo "Removing the system-wide copy from $SYSTEM_PREFIX (needs sudo)…"
    files "$SYSTEM_PREFIX" | sudo xargs rm -f
    sudo rmdir "$SYSTEM_PREFIX/share/gnome-shell/extensions/$EXT_UUID" 2>/dev/null || true
fi

if [ "$1" = "--purge" ]; then
    rm -rf "${XDG_DATA_HOME:-$HOME/.local/share}/clipnest" "${XDG_CONFIG_HOME:-$HOME/.config}/clipnest"
    echo "Clipnest and its data were removed."
else
    echo "Clipnest was removed. The history is kept in ~/.local/share/clipnest (use --purge to delete)."
fi
for rule in /etc/udev/rules.d/70-clipnest-uinput.rules /etc/udev/rules.d/70-clipora-uinput.rules; do
    if [ -e "$rule" ]; then
        echo "To also drop the auto-paste permission:  sudo rm $rule"
    fi
done
