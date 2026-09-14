#!/bin/sh
# island-cmd.sh <command> — route an island command to whichever island is
# live: the new island-shell daemon when it answers, otherwise the old
# waybar scripts (so the sway bindings work in both modes while the two are
# compared; see island-switch.sh).
#   toggle        mod+i: show / hide
#   dodge-toggle  the ⋯ drawer opening / closing (details-toggle.sh)
bin="${ISLAND_SHELL:-$HOME/.local/bin/island-shell}"
[ -x "$bin" ] || bin=island-shell
if "$bin" status >/dev/null 2>&1; then
	exec "$bin" "$1"
fi
case "$1" in
	toggle) exec "$HOME/.config/waybar/island-toggle.sh" ;;
	dodge-toggle) exec "$HOME/.config/waybar/island-shift.sh" ;;
esac
