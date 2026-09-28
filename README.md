# island-shell

A Dynamic-Island-style notch for SwayFX: clock, now playing with a live
equaliser, Touch ID, notifications, a launcher — one Wayland layer-shell
client rendering everything itself at the display's refresh rate.

Replaced the waybar-based island (island-*.sh, 500 lines of CSS frames)
and the walker launcher; see `CLAUDE.md` for the brief and the plan.

## Status

Live activities: volume changes from PipeWire/PulseAudio and backlight
changes replace the desktop OSD with a native island slider. Plugging in
power shows the battery percentage and bolt; crossing 20, 10 or 5 percent
while discharging produces a red low-battery pill. Wi-Fi network changes
and newly connected Bluetooth devices (including their reported battery)
appear briefly as well. The readers run outside the Wayland event loop and
only push state changes into it, so drawing and animation remain
single-threaded.

Screenshots can fly into the island as a thumbnail. Use
`island-screenshot [output.png] [grim options...]`, installed by
`scripts/install.sh`, in the sway binding instead of calling `grim`
directly. Existing screenshot scripts can keep their capture logic and run
`island-shell screenshot "$file"` after `grim` succeeds.

The watchers use the desktop's existing tools: `pactl subscribe` plus
`wpctl` for audio, the kernel backlight and power-supply interfaces,
`nmcli monitor`, and BlueZ through `dbus-monitor` / `bluetoothctl`. Missing
optional tools only disable that source. The commands below can be used to
preview the activities without changing hardware state:

    island-shell volume 0.65
    island-shell brightness 0.8
    island-shell battery 72 charging
    island-shell battery 10 low
    island-shell bluetooth 84 Headphones
    island-shell wifi My network

Click the ordinary compact island to open a four-button menu for Wi-Fi, Bluetooth,
Timer and Launcher. Click a button or select it with the arrow keys and Enter;
Escape, × or a click outside closes the menu. `island-shell quick` opens the
same menu from a script; `mod+m` opens it directly. The Waybar's ⋯ drawer
now contains only disk, CPU and memory information, with no separate network
or timer buttons.
`mod+d` still opens the launcher directly.
Inside a shortcut sheet, × returns to the quick menu; Escape and a click
outside dismiss it entirely.

The panel has animated 5, 15, 25 and 50 minute presets (25 minutes is the
Pomodoro shortcut); click a preset to start. While running, a compact ring
and remaining time sit beside the clock. Click that indicator to pause or
resume, or right-click to reopen the controls. Escape closes the panel and ×
returns to the quick menu, neither stopping the countdown. Completion expands into a short “Time is up”
activity. The timer is kept in memory and resets when the service restarts.

`island-shell wifi` / `island-shell bluetooth` open the same
native sheets; ×, Escape or a click outside closes them. The closing click is
consumed, not forwarded to an application underneath. This also works for
the timer and launcher; `island-shell dismiss` provides the same action.
The transparent input surface exists only while a modal menu is open.
Waybar module highlights have a 3 px vertical inset and ease in/out over 180 ms.

Wi-Fi has All networks / Saved tabs, a radio switch, search, connection and
disconnection. Click a protected network to enter its password; Enter submits.
Saved profiles connect directly, and a failed attempt can retry with a new
password. WPA/WPA2/WPA3 personal and open networks are supported; enterprise
networks requiring an identity or certificate need an existing NetworkManager
profile. Secrets go directly to NetworkManager over D-Bus, never in argv or
temporary files.

Bluetooth has My devices / Discover tabs. Click a paired device to connect
or disconnect; discovery finds new devices and offers pairing. PIN entry,
passkey display and confirmation are handled in the island by an app-scoped
[BlueZ agent](https://bluez.readthedocs.io/en/latest/agent-api/). Discovery runs
for 30 seconds and can be stopped; closing the panel releases its scan session
and cancels pending pairing. Battery levels appear when supported.
Friendly aliases and remote names are preferred over address placeholders;
named devices sort first. Names update during discovery. Devices that do not
advertise a name are explicitly labelled as unnamed with their address below,
not assigned a guessed product name.

Both sheets show operation progress and errors, scroll through the complete
list, and refresh state while open. Worker replies are scoped to a panel and
request so late results cannot reopen a closed sheet. Buttons, tabs, list
positions, scroll and panel size animate; interactive sheets keep focus and
queue transient OSDs until they close. Timer hover fades both in and out, and
Pause / Resume / Cancel leave the controls open.

`island-shell timer` opens the same panel. CLI controls also remain:
`island-shell timer 25m` (or `1h30m`), `timer pause`, `timer resume`,
`timer toggle`, `timer status`, `timer cancel`, and `timer pomodoro`.

Volume and brightness key repeats extend the OSD's hold without restarting
its entrance. The slider follows on a spring while the percentage shows the
latest value. The volume symbol uses SF Pro's SF Symbols when available,
with a vector fallback; brightness uses a rounded vector sun. Backlight
changes are sampled every 80 ms independently of the slower battery reader.

Neovim: with the companion plugin (`nvim/island.lua`) the island is the
editor's status line. The mode rides beside the clock as a coloured chip
with the unsaved dot, the diagnostics counts, the search count and a
breathing red dot while a macro records. Saving flashes the chip green;
`:IslandTask` shows progress and its outcome as a pill. LSP progress and git
event pills are optional, while diagnostics stay in the chip and panel.
Hovering the chip opens a panel with the
file, the position, git, the diagnostics and the buffer list — click a
buffer to switch to it, or Save / Format / Close. The island writes
`~/.config/island-shell/nvim-token` (0600) on first run and ignores any
message that does not quote it, so only an editor running the plugin ever
appears. See `nvim/README.md`; the Neovim configuration built around it
lives in its own repository, iVim (`~/it/coding/ivim`).

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
The list holds every match and scrolls — the wheel, or the selection
walking past the edge — with no scrollbar: the rows simply fade into the
top and bottom of the viewport.

A left click on the ordinary compact body opens the four-button menu immediately.
Left, right and middle clicks over the compact equaliser or the open music panel
pause/resume, skip forward and skip back respectively. The wheel changes tracks
only over that music area. Timer controls, seek dragging, authentication,
notifications and other modal sheets keep their own interactions.
The music panel opens only from the equaliser on the right of the compact
island; Neovim's details open only from its chip on the left. Once a panel
opens, moving the pointer across it keeps it open until the pointer leaves.
Hovering the clock or another ordinary part gently widens the compact island,
including when music or Neovim is active; the transition into either detailed
panel continues from the current animated size.

Keyboard: text, Backspace, Ctrl+U, Up/Down/Tab, Enter to launch, Escape to fold; the pointer picks rows and
the wheel scrolls. The launcher takes the keyboard
(layer-shell exclusive) only while it is open, gives way to Touch ID, and
notifications wait behind it.

The same search field also finds files and folders by name in your home
directory, starting at two characters. For example, type `report`, `README`,
or `file:отчёт` (`файл:` also works) to show only filesystem results. Paths
inside home, such as `~/Documents/report`, can narrow the search. Results show
their parent folder; Enter or a click opens the selected item without passing
filenames to a shell. Folders open in Alacritty with that working directory;
text files open in Neovim inside Alacritty, with their parent as the working
directory. PDFs open in Evince; Word/OpenDocument/RTF documents, spreadsheets
and presentations open in LibreOffice. Other formats use the system association
(`xdg-open`). Extensions are case-insensitive; text is detected with `file`,
including extensionless files. Detection and startup run off the UI thread.
If a preferred opener is not installed, the system association is used instead.
Install LibreOffice if needed with `sudo sh scripts/01-install-office-opener.sh`.
This is filename/path search, not content search; no persistent filename
database is written. Only opening a selected file may probe its contents.

Apps and file openers run in independent systemd user scopes (`launch.rs`),
so restarting or crashing the island does not close terminals, editors or
document windows. Merely using `setsid` would not escape the service's cgroup.
The scopes inherit the session environment and preserve literal filename
arguments. If scope creation fails, the error is logged; there is no unsafe
direct-spawn fallback. The service retains its default `KillMode=control-group`
so its own background readers are still cleaned up correctly.

File search requires `fdfind` (Debian package `fd-find`) or `fd`. It runs on a
worker after a 180 ms debounce, uses at most two search threads and returns up
to 80 results with a three-second search limit. A new query or closing the
launcher cancels the old process; late replies are discarded. Hidden files,
ignore-file exclusions, `node_modules`, `target` and `__pycache__` are skipped;
symlinked directories are not traversed. Partial results are labelled so the
query can be narrowed. The best five app matches precede file results, with
remaining app matches still accessible below them.
The launcher selection stops at the first and last result, with a small edge
nudge instead of wrapping. `Ctrl+Backspace` deletes the previous word in its
search field.

Step 3: notifications. The island itself owns `org.freedesktop.Notifications`:
no dunst process, monitor permission or rule script is needed. It implements
the standard notification methods, replacement IDs, actions, expiry and close
signals. A bounded history (50 entries) is saved privately under
`$XDG_STATE_HOME/island-shell/notifications.json`; open its animated sheet
from the island's shortcut menu or `island-shell notifications`. Active
notifications offer their application actions, and the sheet supports reading,
deleting, clearing and scrolling. Incoming rows ease into place while the
sheet is open; deleted rows fade and slide away as the list closes the gap.
Notifications from the same application are folded into a single row with a
count; click the row to expand individual messages, or its × to clear that
application's group. Single messages open their details directly.
The center fits its height to the visible rows. Grouped messages and their
emoji fade together, and switching between the list and a notification's
details uses a short crossfade and vertical glide. Shortcut sheets also
crossfade when opened from the quick menu.
App icons and supplied notification images appear in the pill and history;
the pill exposes up to two application actions directly, while clicking its
body invokes the default action when one exists. The shortcut menu has a
**Do Not Disturb** tile, also controlled by `island-shell dnd on|off|toggle|status`.
It suppresses ordinary popups and sounds while still saving them to history;
critical alerts continue to appear. The choice survives restarts, and its
shortcut tile switches in place without closing the menu; the label stays
"Тишина" while the whole tile eases into a purple active appearance.
Optional local sounds use `paplay` and the
existing TriTone/Critical Stop files, respecting `suppress-sound` and avoiding
duplicates for Telegram and Thunderbird. The box morphs into a wide pill with the app's icon
on the left, its name in the app's colour and the message beside it; the
message wraps to two lines and the pill grows to fit it, with the sender
kept semibold ahead of a lighter body. The icon is a vector tile that pops
in and plays its own entrance (the paper
plane flies in, the gear turns into place, the bell rings, the envelope's
flap falls shut, Clyde bounces, the alert shakes), and a countdown ring
around it empties over the 3.5 s the notification is up. A newer one rolls
the old content out through the top and itself in from below, and the pill
widens to fit. Critical notifications get a breathing red rim. A click
dismisses; notifications wait behind an active Touch ID. A notification
rises on its own even while the compact island is hidden, then retreats
without leaving the island visible.
The same temporary reveal applies to a now-playing pill when playback starts
in fullscreen; it is fully placed on-screen without exposing the compact bar.

Step 2: now playing (MPRIS over D-Bus, zbus). A track that starts
announces itself with a "now playing" toast — album art, the app, the
title, the bars — which then collapses to four cava-fed equaliser bars
beating beside the clock; hovering those bars opens it into
a player with the art, the title and artist, a live equaliser, a progress
line you can click or drag to scrub — it swells under the pointer and
again in the hand, on springs — and vector transport controls that light
up as the pointer reaches them. Silent playback
(paused / muted) folds the bars away after 15 s. Touch ID (fprintd on the system bus) — the box morphs into the
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

## Performance

Desktop entries are scanned on a worker and retained between launcher
openings (refreshed on opening, at most once per five seconds). Theme lookup,
SVG rendering, icon decoding, cover art and screenshot thumbnails also run on
bounded background queues. Only pixel bytes cross threads; Cairo surfaces
remain thread-local. Visible launcher icons are prefetched, and their scaled
cache survives closing the panel. Stale cover-art replies cannot replace a
new track's image. Failed/missing resources use the existing placeholders.

Text shaping uses a bounded 512-entry LRU cache with device-pixel width keys,
not floating-point animation values or whole-cache resets. Audio setup and
mute checks no longer wait for subprocesses on the Wayland thread. A steady
countdown uses timed updates (30 Hz in its final minute) rather than both a
60 Hz frame callback and a second timer. Unchanged outside-click regions are
not recommitted; fully opaque connection sheets avoid intermediate groups.

Reproduce the read-only, offscreen 1.5× launcher/OSD benchmark with:

```sh
CARGO_INCREMENTAL=0 cargo test --release --locked --bin island-shell animation_benchmark -- --ignored --nocapture
```

It reports constructor time and median/p95/max paint time, not end-to-end
compositor latency. Live `status` includes launcher result count and frame CPU
timings; `ISLAND_DEBUG=1` additionally logs presentation prediction errors.

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
    island-shell quick                          # mod+m: quick-access menu
    island-shell launcher                       # mod+d: unfold / fold the panel
    island-shell notifications                  # open the notification history
    island-shell dnd on|off|toggle|status       # quiet ordinary notifications
    island-shell type <text> | key <name>       # drive the launcher from a script
    island-shell scrub <0..1> | scrub end       # drag the playhead from a script
    island-shell timer 25m | timer pause        # compact countdown
    island-shell screenshot <file.png>          # animate a finished grim image
    island-shell status | quit

### Configuration

`~/.config/island-shell/config`, `key = value` lines (`#` comments), read at
start; `island-shell set key value` changes a value in the running island:

    smoothing = 0.6   # corner smoothing 0..1 (0 = circular arcs)
    liquid = 1        # velocity stretch, 0 = rigid
    hover = 1         # widen the box under the pointer

### Running as a service

`scripts/install.sh` also installs `systemd/island-shell.service` as a
user unit; the sway config starts it with
`exec_always systemctl --user start island-shell.service` (a no-op on
reload while it is up), systemd restarts it if it ever falls over, and its
log is `journalctl --user -u island-shell`. A second instance finding the
socket answered exits quietly instead of failing. The `exec dunst` autostart
should be removed from the sway config: the island now owns the notification
bus name itself. Restart the island service after installation; other
notification daemons must not be running. The old waybar island is gone:
`scripts/island-cmd.sh launcher` is what mod+d runs and
`scripts/island-cmd.sh quick` is bound to mod+m (both close the ⋯ drawer
first). `island-shell toggle` is mod+i, and
`~/.config/sway/scripts/details-toggle.sh` (mod+o) signals waybar's
drawer and tells the island to dodge. In a fullscreen window, mod+i can
temporarily reveal the compact island; mod+d opens the launcher directly.
Notifications and live activities appear over the fullscreen window even
while the compact island remains hidden.

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
    src/notify.rs   notification model and text sanitization
    src/notification_server.rs  native desktop notification D-Bus service
    src/notice_center.rs        animated, persistent history sheet
    src/icons.rs    the vector app icons and the countdown ring
    src/launcher.rs the launcher panel (search field, list, re-ranking)
    src/apps.rs     desktop entries: scanning, matching, launching
    src/iconload.rs icon-theme lookup and loading
    src/nvim.rs     the Neovim protocol, its state and the calls back into it
    src/editor.rs   the mode chip and the editor panel
    nvim/           the companion plugin (lua/island.lua), a lazy.nvim `dir`
    scripts/        install.sh, island-cmd.sh (the sway bindings' entry)
    systemd/        the user service
    examples/icons.rs  renders the icon set to a PNG for eyeballing
    examples/apps.rs   lists what the launcher would show, with icon sizes
