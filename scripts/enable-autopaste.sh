#!/bin/sh
# Lets Clipnest press the paste shortcut for you after you pick an item.
#
# Wayland doesn't allow one app to type into another, so Clipnest uses a
# virtual keyboard (/dev/uinput), which is root-only by default. This adds a
# udev rule granting access to whoever is logged in at the local seat (the
# same rule Steam ships for controllers). Undo with:
#   sudo rm /etc/udev/rules.d/70-clipnest-uinput.rules
set -e

RULE=/etc/udev/rules.d/70-clipnest-uinput.rules
echo "Installing $RULE (needs sudo)…"
echo 'KERNEL=="uinput", SUBSYSTEM=="misc", TAG+="uaccess", OPTIONS+="static_node=uinput"' \
    | sudo tee "$RULE" >/dev/null
sudo modprobe uinput
sudo udevadm control --reload-rules
sudo udevadm trigger --sysname-match=uinput

sleep 1
if [ -w /dev/uinput ]; then
    echo "Done: auto-paste is ready (a running Clipnest picks it up automatically)."
else
    echo "Rule installed. Log out and back in, then auto-paste will be available."
fi
