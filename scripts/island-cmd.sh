#!/bin/sh
# island-cmd.sh <command> — the sway bindings' way into the island:
#   toggle        mod+i: show / hide
#   launcher      mod+d: the island unfolds into the launcher; the ⋯ drawer
#                 (mod+o, details-toggle.sh) is closed first if it is open,
#                 since the island cannot be a panel and dodge it at once
#   quick         mod+m: unfold the island's shortcut menu
#   notes         mod+n: the notes page
#   shelf         mod+s: the shelf
bin="${ISLAND_SHELL:-$HOME/.local/bin/island-shell}"
[ -x "$bin" ] || bin=island-shell
case "$1" in launcher|quick|notes|shelf) panel=1 ;; *) panel= ;; esac
if [ -n "$panel" ]; then
	case "$("$bin" status 2>/dev/null)" in
		*" dodged "*) "$HOME/.config/sway/scripts/details-toggle.sh" ;;
	esac
fi
exec "$bin" "$1"
