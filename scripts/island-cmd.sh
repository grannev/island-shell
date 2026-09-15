#!/bin/sh
# island-cmd.sh <command> — route an island command to whichever island is
# live: the new island-shell daemon when it answers, otherwise the old
# waybar scripts (so the sway bindings work in both modes while the two are
# compared; see island-switch.sh).
#   toggle        mod+i: show / hide
#   dodge-toggle  the ⋯ drawer opening / closing (details-toggle.sh)
#   launcher      mod+d: the island unfolds into the launcher panel
bin="${ISLAND_SHELL:-$HOME/.local/bin/island-shell}"
[ -x "$bin" ] || bin=island-shell
if status=$("$bin" status 2>/dev/null); then
	# the launcher wants the island to itself: close the ⋯ drawer first
	# (that also brings the island back to the middle)
	if [ "$1" = launcher ]; then
		case "$status" in *" dodged "*) "$HOME/.config/sway/scripts/details-toggle.sh" ;; esac
	fi
	exec "$bin" "$1"
fi
case "$1" in
	toggle) exec "$HOME/.config/waybar/island-toggle.sh" ;;
	dodge-toggle) exec "$HOME/.config/waybar/island-shift.sh" ;;
	launcher) exec "$HOME/.config/waybar/island-launcher.sh" ;;
esac
