#!/bin/sh
# Run manually: sudo sh scripts/04-install-tablet-tools.sh
# Tablet mode under niri (~/.config/niri/scripts/tablet-mode.sh):
#   lisgd           touchscreen edge swipes (columns, the overview, workspaces)
#   libinput-tools  `libinput debug-events` to see what the touchscreen, the
#                   pen and the tablet-mode switch report
# lisgd reads the touchscreen's evdev device. Rather than the `input` group
# (which would let every program read every keyboard), a udev rule lets the
# logged-in user read the touchscreen's finger device only (uaccess).
set -eu
if [ "$(id -u)" -ne 0 ]; then
    echo 'Run with sudo: sudo sh scripts/04-install-tablet-tools.sh' >&2
    exit 1
fi
apt-get update
apt-get install -y lisgd libinput-tools
cat > /etc/udev/rules.d/70-island-touchscreen.rules <<'RULE'
# island-shell: the seat's user may read the touchscreen's finger device, for
# lisgd's gestures in tablet mode (scripts/04-install-tablet-tools.sh).
SUBSYSTEM=="input", KERNEL=="event*", ATTRS{name}=="*Finger", TAG+="uaccess"
SUBSYSTEM=="input", KERNEL=="event*", ATTRS{name}=="*[Tt]ouchscreen*", TAG+="uaccess"
RULE
udevadm control --reload
udevadm trigger --subsystem-match=input --action=change
echo
echo 'Installed. The touchscreen is readable now (no new login needed):'
ls -l /dev/input/event* | grep '+' || true
echo 'Fold the screen back to tablet mode to start the gestures.'
