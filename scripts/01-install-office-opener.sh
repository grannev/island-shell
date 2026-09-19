#!/bin/sh
# Run manually: sudo sh scripts/01-install-office-opener.sh
set -eu
if [ "$(id -u)" -ne 0 ]; then
    echo 'Run with sudo: sudo sh scripts/01-install-office-opener.sh' >&2
    exit 1
fi
apt-get update
apt-get install libreoffice
