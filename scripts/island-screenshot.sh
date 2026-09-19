#!/bin/sh
# island-screenshot.sh [output.png] [grim options...]
# Capture a frame and send its finished path to the island for the thumbnail.
set -e
if [ "$#" -gt 0 ]; then
	file=$1
	shift
else
	dir=${XDG_PICTURES_DIR:-"$HOME/Pictures"}/Screenshots
	mkdir -p "$dir"
	file="$dir/$(date +%Y-%m-%d_%H-%M-%S).png"
fi
grim "$@" "$file"
bin=${ISLAND_SHELL:-"$HOME/.local/bin/island-shell"}
[ -x "$bin" ] || bin=island-shell
"$bin" screenshot "$file" >/dev/null 2>&1 || true
printf '%s\n' "$file"
