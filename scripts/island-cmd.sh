#!/bin/sh
# island-cmd.sh <command> — the sway bindings' way into the island:
#   toggle        mod+i: show / hide
#   launcher      mod+d: the island unfolds into the launcher; the ⋯ drawer
#                 (mod+o, details-toggle.sh) is closed first if it is open,
#                 since the island cannot be a panel and dodge it at once
bin="${ISLAND_SHELL:-$HOME/.local/bin/island-shell}"
[ -x "$bin" ] || bin=island-shell
if [ "$1" = launcher ]; then
	case "$("$bin" status 2>/dev/null)" in
		*" dodged "*) "$HOME/.config/sway/scripts/details-toggle.sh" ;;
	esac
fi
exec "$bin" "$1"
