#!/bin/sh
# island-switch.sh new|old|status|autostart — choose between the new
# island-shell daemon and the old waybar island, so the two can be compared.
#   new        remember the choice, start the daemon, park the old island
#   old        remember the choice, stop the daemon, bring the old island back
#   autostart  (sway exec_always) apply the remembered choice after a reload
#   status     which one is live
# The choice lives in ~/.config/island-shell/enabled; while the daemon is
# meant to be live a marker $XDG_RUNTIME_DIR/island-shell.active tells the
# old scripts (waybar.sh, island-launcher.sh) to leave their island parked.
rt="${XDG_RUNTIME_DIR:-/tmp}"
cfg="${XDG_CONFIG_HOME:-$HOME/.config}/island-shell"
marker="$rt/island-shell.active"
bin="${ISLAND_SHELL:-$HOME/.local/bin/island-shell}"
[ -x "$bin" ] || bin=island-shell
old_toggle="$HOME/.config/waybar/island-toggle.sh"

running() { "$bin" status >/dev/null 2>&1; }
old_hidden() { [ -e "$rt/waybar-island-step" ]; }

start_new() {
	: > "$marker"
	# park the old island first so the two do not overlap on screen
	old_hidden || "$old_toggle"
	if ! running; then
		# quit a stale daemon that is not answering, then start afresh
		pkill -x island-shell 2>/dev/null
		setsid "$bin" >"$rt/island-shell.log" 2>&1 </dev/null &
	fi
}
stop_new() {
	rm -f "$marker"
	running && "$bin" quit >/dev/null 2>&1
	sleep 0.1; pkill -x island-shell 2>/dev/null
	# the old island comes back unless something else keeps it hidden
	if old_hidden && [ ! -e "$rt/waybar-island-autohidden" ]; then "$old_toggle"; fi
}

case "$1" in
	new) mkdir -p "$cfg"; : > "$cfg/enabled"; start_new ;;
	old) rm -f "$cfg/enabled"; stop_new ;;
	autostart) if [ -e "$cfg/enabled" ]; then start_new; else rm -f "$marker"; fi ;;
	status)
		if running; then echo "new ($("$bin" status))"; else echo "old"; fi ;;
	*) echo "usage: island-switch.sh new|old|status|autostart" >&2; exit 2 ;;
esac
