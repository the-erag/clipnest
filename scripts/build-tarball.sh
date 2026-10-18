#!/bin/sh
# Packages target/release/clipnest with its installer as
# dist/clipnest-<version>-linux-<arch>.tar.gz. Unpack it and run ./install.sh
# (no Rust needed: the installer uses the prebuilt binary).
set -e
umask 022   # packaged files: 755 / 644

SRC="$(cd "$(dirname "$0")/.." && pwd)"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$SRC/Cargo.toml" | head -n 1)"
NAME="clipnest-${VERSION}-linux-$(uname -m)"
BIN="$SRC/target/release/clipnest"
[ -x "$BIN" ] || { echo "Build it first: cargo build --release" >&2; exit 1; }

STAGE="$SRC/dist/$NAME"
rm -rf "$STAGE"
mkdir -p "$STAGE/scripts" "$STAGE/data"
install -m755 "$BIN" "$STAGE/clipnest"
install -m755 "$SRC/install.sh" "$SRC/uninstall.sh" "$STAGE/"
install -m755 "$SRC/scripts/enable-autopaste.sh" "$STAGE/scripts/"
cp -r "$SRC/data/icons" "$STAGE/data/"
cp -r "$SRC/extension" "$STAGE/"
cp "$SRC/LICENSE" "$SRC/README.md" "$STAGE/"
tar -C "$SRC/dist" -czf "$SRC/dist/$NAME.tar.gz" "$NAME"
echo "dist/$NAME.tar.gz"
