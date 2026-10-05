#!/bin/sh
# Run manually: sudo sh scripts/03-install-screen-recorder.sh
# wf-recorder records the screen for the island's "Запись" button
# (`island-shell record`); ffmpeg (already installed) makes the thumbnail.
set -eu
if [ "$(id -u)" -ne 0 ]; then
    echo 'Run with sudo: sudo sh scripts/03-install-screen-recorder.sh' >&2
    exit 1
fi
apt-get update
apt-get install -y wf-recorder
echo
echo 'Installed. Restart the island so it finds wf-recorder:'
echo '    systemctl --user restart island-shell'
