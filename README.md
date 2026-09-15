# island-shell

A Dynamic-Island-style notch for SwayFX: clock, now playing with a live
equaliser, Touch ID, notifications, a launcher — one Wayland layer-shell
client rendering everything itself at the display's refresh rate.

Replaces the waybar-based island in `~/.config/waybar` (island-*.sh,
sway.css) and the walker/fuzzel launcher patches. See `CLAUDE.md` for the
brief, the current island's behaviour to match, and the plan.

## Status

Step 4: the launcher (mod+d). The panel stands on its own: it opens even
with the island parked above the edge (mod+i) and folds back out of sight,
and the ⋯ drawer (mod+o) folds it away before the island dodges. The island unfolds into a 660×512 sheet —
the clock stays at the top — with a search field and the matching
applications: icon, name and subtitle per row. Desktop entries are read
from every `applications` directory (localised names, `TryExec`, hidden
entries skipped, namesakes merged) and icons resolved through the user's
icon theme (PNG straight, SVG rendered with resvg, cached). Matching is
literal-first with acronyms ("vsc" finds Visual Studio Code) and nothing
looser, ranked by how often each entry has been launched from here.
Typing re-ranks the list and the rows *glide* to their new places on
springs while the selection follows; rows entering and leaving fade.
Keyboard: text, Backspace, Ctrl+U, Up/Down/Tab, Enter to launch, Escape
to fold; the pointer picks rows too. The launcher takes the keyboard
(layer-shell exclusive) only while it is open, gives way to Touch ID, and
notifications wait behind it.

Step 3: notifications. The island watches the `Notify` calls going to
`org.freedesktop.Notifications` as a D-Bus monitor — dunst keeps the
history, the sounds and its own pop-ups, and the island needs no rule
script or state file. The box morphs into a wide pill with the app's icon
on the left, its name in the app's colour and the message beside it; the
message wraps to two lines and the pill grows to fit it, with the sender
kept semibold ahead of a lighter body. The icon is a vector tile that pops
in and plays its own entrance (the paper
plane flies in, the gear turns into place, the bell rings, the envelope's
flap falls shut, Clyde bounces, the alert shakes), and a countdown ring
around it empties over the 3.5 s the notification is up. A newer one rolls
the old content out through the top and itself in from below, and the pill
widens to fit. Critical notifications get a breathing red rim. A click
dismisses; notifications wait behind an active Touch ID and are skipped
while the island is hidden.

Step 2: now playing (MPRIS over D-Bus, zbus) — "eq  Artist — Title  |
clock", the title sliding out from behind the divider and back, track
changes as out-then-in, a four-band equaliser fed by cava (30 fps, eased
per frame, sinking flat on pause), silent playback (paused / muted) folding
away after 15 s, click / right / middle / scroll = play-pause / next /
previous; Touch ID (fprintd on the system bus) — the box morphs into the
92 px pill with a vector fingerprint whose ridges breathe in a ripple; on
a result the ridges converge into a ring that fills green or red and the
checkmark / cross draws itself on it (checkmark.circle.fill /
xmark.circle.fill), with a bounce on success and a head-shake on a miss;
a new attempt undraws the mark and splits the ring back into the
fingerprint; the song folds away for the drawer and the pill.

Step 1 of the plan: layer-shell window on the overlay layer, cairo/pango
rendering through `wp_fractional_scale` + `wp_viewporter` (crisp at 1.5),
clock, concave fillets, show/hide (minimum-jerk slide with the spread),
hover, fullscreen auto-hide and the ⋯ drawer dodge over sway IPC. Idle
CPU is zero: frames are only drawn while something moves, the clock wakes
once a minute.

Motion quality:
- every frame is sampled at the instant it will be on screen: the
  presentation clock (`wp_presentation`) gives the vblank cadence and the
  feedback teaches how many frames ahead the compositor runs
  (`ISLAND_DEBUG=1` prints the residual error per frame — ±0.01 ms);
- moves are minimum-jerk quintics through the current position, velocity
  and acceleration, so a toggle mid-flight turns the island around without
  a jolt; hover is a damped spring (response 0.4 s, damping 0.6);
- glyph rendering is identical in motion and at rest (hinted metrics,
  origin on whole output pixels); the box edges snap to output pixels at
  rest and glide sub-pixel while moving;
- corners with continuous curvature (`smoothing`, Figma-style; limited by
  the room on the shorter edge, so subtle on the 32 px box, full on the
  pill and the panel) and a velocity-dependent stretch (`liquid`);
- damage and opaque regions are reported per frame.

## Build and install

    scripts/install.sh        # cargo build --release → ~/.local/bin/island-shell

Needs the cairo, pango and wayland-client development packages (Debian:
`libcairo2-dev libpango1.0-dev libwayland-dev`).

## Running

`island-shell` with no arguments runs the island; with a command it talks
to the running one over `$XDG_RUNTIME_DIR/island-shell-<display>.sock`:

    island-shell toggle | show | hide          # mod+i
    island-shell dodge | undodge | dodge-toggle # the ⋯ drawer
    island-shell set <key> <value>              # tune live (see below)
    island-shell dump <file.png>                # render the current frame off screen
    island-shell demo <app> <summary>|<body>    # fabricate a notification
    island-shell launcher                       # mod+d: unfold / fold the panel
    island-shell type <text> | key <name>       # drive the launcher from a script
    island-shell status | quit

### Configuration

`~/.config/island-shell/config`, `key = value` lines (`#` comments), read at
start; `island-shell set key value` changes a value in the running island:

    smoothing = 0.6   # corner smoothing 0..1 (0 = circular arcs)
    liquid = 1        # velocity stretch, 0 = rigid
    hover = 1         # widen the box under the pointer

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
    src/dbus.rs     MPRIS players and fprintd (zbus, threads)
    src/touchid.rs  the vector Touch ID icon and its transformations
    src/eq.rs       cava feed
    src/notify.rs   notifications (a D-Bus monitor on Notify)
    src/icons.rs    the vector app icons and the countdown ring
    src/launcher.rs the launcher panel (search field, list, re-ranking)
    src/apps.rs     desktop entries: scanning, matching, launching
    src/iconload.rs icon-theme lookup and loading
    scripts/        install, switch, routing
    examples/icons.rs  renders the icon set to a PNG for eyeballing
    examples/apps.rs   lists what the launcher would show, with icon sizes
