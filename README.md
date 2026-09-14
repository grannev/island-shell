# island-shell

A Dynamic-Island-style notch for SwayFX: clock, now playing with a live
equaliser, Touch ID, notifications, a launcher — one Wayland layer-shell
client rendering everything itself at the display's refresh rate.

Replaces the waybar-based island in `~/.config/waybar` (island-*.sh,
sway.css) and the walker/fuzzel launcher patches. See `CLAUDE.md` for the
brief, the current island's behaviour to match, and the plan.

## Status

Step 1 of the plan: layer-shell window on the overlay layer, cairo/pango
rendering through `wp_fractional_scale` + `wp_viewporter` (crisp at 1.5),
clock, concave fillets, show/hide (minimum-jerk slide with the spread),
hover, fullscreen auto-hide and the ⋯ drawer dodge over sway IPC. Idle
CPU is zero: frames are only drawn while something moves, the clock wakes
once a minute.

## Build and install

    scripts/install.sh        # cargo build --release → ~/.local/bin/island-shell

Needs the cairo, pango and wayland-client development packages (Debian:
`libcairo2-dev libpango1.0-dev libwayland-dev`).

## Running

`island-shell` with no arguments runs the island; with a command it talks
to the running one over `$XDG_RUNTIME_DIR/island-shell-<display>.sock`:

    island-shell toggle | show | hide          # mod+i
    island-shell dodge | undodge | dodge-toggle # the ⋯ drawer
    island-shell status | quit

### Switching between the old and the new island

    scripts/island-switch.sh new     # start island-shell, park the waybar island
    scripts/island-switch.sh old     # stop it, bring the waybar island back
    scripts/island-switch.sh status

The choice is remembered in `~/.config/island-shell/enabled` and applied on
every sway reload (`exec_always island-switch.sh autostart` in the sway
config). While the new island is live a marker
`$XDG_RUNTIME_DIR/island-shell.active` tells the old scripts
(`waybar.sh`, `island-launcher.sh`, `details-toggle.sh`) to leave their
island parked; `scripts/island-cmd.sh` routes mod+i to whichever is live.

## Layout

    src/main.rs     CLI: daemon or a command for it
    src/app.rs      Wayland: layer surface, shm buffers, scale, input, event loop
    src/island.rs   the island's state, animated geometry and drawing
    src/anim.rs     min-jerk / cubic-bezier tweens
    src/ctl.rs      control socket
    src/sway.rs     sway IPC (fullscreen watch)
    scripts/        install, switch, routing
