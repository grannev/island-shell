#!/bin/sh
# Run manually: sudo sh scripts/02-install-clipboard-history.sh
# cliphist keeps the clipboard's history for the launcher's "clip:" search;
# the sway config feeds it with `wl-paste --watch cliphist store`.
set -eu
if [ "$(id -u)" -ne 0 ]; then
    echo 'Run with sudo: sudo sh scripts/02-install-clipboard-history.sh' >&2
    exit 1
fi
apt-get update
apt-get install -y cliphist
echo
echo 'Installed. Start the history now (sway starts it on later logins):'
echo "    swaymsg exec 'wl-paste --watch cliphist store'"
