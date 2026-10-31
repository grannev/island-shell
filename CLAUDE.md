# island-shell — project brief

Reply to the user in Russian. Code, comments and commit messages in English.
For menu and sheet controls, follow the role-based, permissive visual guide
in `docs/interface-style.md`; homogeneous controls share a style, while new
control types remain allowed when their interaction calls for one.

## Design reference
Since 2026-10 new ideas and the look follow Vorssaint's Dynamic Island
(open-source macOS, github.com/vorssaint/vorssaint-utils, GPL-3.0, Swift;
the island is `Sources/Vorssaint/{UI,Services}/Notch`). Rebuild its design in
Rust/cairo from observation and its measurements; do not copy its code. The
island keeps the hanging silhouette (Vorssaint's "notch" mode), not the
floating capsule.

## Goal
A single lightweight program (Rust preferred: wayland-client +
smithay-client-toolkit, layer-shell; cairo/tiny-skia + pango for drawing;
zbus for D-Bus) that draws the "Dynamic Island" at the top centre of the
screen and replicates every capability of the current waybar-based island,
but with real per-frame animation (springs / minimum-jerk interpolation at
60 fps), no scripts, no FIFOs, no CSS frame classes, no patched waybar or
walker. Everything drawn inside the island is written from scratch here.
Background helpers that only talk to the environment may stay: sway IPC,
fprintd (D-Bus), MPRIS players (D-Bus), cava (raw output) or an own FFT on
a PipeWire capture, dunst (or an own org.freedesktop.Notifications).

## Environment (fixed facts)
- Debian 13, SwayFX 0.5.1 (built from ~/.local/src/swayfx, patched), output
  eDP-1 2256x1504 @ scale 1.5 → 1504x1002 logical. Fractional scale: the
  island must render crisply at 1.5 (glyphs on whole output pixels).
- The main Waybar is retired (2026-10-31): the island draws its row
  itself (`src/bar.rs`, config `bar = on`; the sway `bar {}` block is
  commented out with a note on going back). Inventory and plan:
  `docs/bar.md`. The ⋯ drawer (mod+o → details-toggle.sh → `island-shell
  dodge-toggle`) opens to the left of the right-hand items; the island
  dodges it.
- Fonts: SF Pro Text (Regular…Black) in ~/.local/share/fonts/SF-Pro; SF
  Symbols glyphs are PUA code points of the same fonts (used: speaker
  U+1002A5/A7/A9, mute U+1002A3 (right-facing, not the RTL U+10063A), keyboard.fill U+100E91, battery
  U+1006EA/E9/100EB6/EB8/1006E8, bolt U+10088B, ellipsis U+100360, wifi
  U+100647, magnifyingglass U+1002AB). Nerd-font glyphs used for Touch ID:
  fingerprint U+F0237, check U+F05E0, cross U+F0159 (JetBrainsMono Nerd).
- The user cannot run sudo from the agent: root steps go into numbered
  scripts the user runs. Runtime scripts: no Python (sh/bash/gawk only);
  Python is fine for one-off build tools. Prefer existing tools over
  hand-rolled ones for anything outside the island's own drawing.
- Commits end with
  `Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>` and a
  `Claude-Session: <url>` trailer.

## Current island to match (reference: ~/.config/waybar/island-*.sh, sway.css)
(Since 2026-10-31 every corner radius — the box, fillets, pills, sheets,
cards, the bar's drops, `screen_corners` — is drawn at 0.7 of the values
below: `menu_style::CORNER_SCALE`; capsules and discs stay round.)
Geometry (logical px): black box 32 tall at y 0..32, bottom corners r=14,
14 px concave fillets where it meets the top edge, text SF Pro Text
Semibold 16 px white, padding 16, content-sized width; clock text
`%a %b %-d  %-I:%M %p`.
- Show/hide (mod+i, and auto while a fullscreen window is focused, sway
  IPC): slides up into the edge with a "drop meeting water" spread on the
  way, ~0.25 s minimum-jerk (10t³−15t⁴+6t⁵); no overshoot.
- Hover (pointer over it): padding 16 → 34, bg #000 → #0A0A0A, 420 ms.
- Now playing (MPRIS): "eq  Artist — Title   |   clock"; the title slides
  out from behind the divider (19-step min-jerk) when playback appears,
  retracts on stop; track change = out then in. Equaliser: four 3 px cyan
  (#66D9FF) bars, heights from cava (4 bands, 15 fps) with ~90 ms easing;
  paused → bars sink flat over 700 ms. Paused or sink muted/0 % for 15 s →
  title slides away; back when sound resumes. Click play/pause, scroll /
  right / middle = next / prev. Title cut to ~42 chars with "…".
- Dodge: when the drawer opens the island moves 240 px left (40-frame
  min-jerk, ~0.25 s, ahead of the 350 ms drawer) and the song folds away;
  back the same way. Never shows while hidden.
- Touch ID (fprintd VerifyFingerSelected / VerifyStatus on the system
  bus, from hyprlock, sudo, polkit): the box swells into a 92 px pill
  (r=44) in 30 frames — old line rides up and dissolves while the box
  grows (p1..p11), then a big fingerprint glyph + "Touch ID" climb in from
  below (p12..p30); pulse cycles 12 colours at 80 ms; match → green tick
  (1 s) then close; miss → red cross "Try again", pill stays for the next
  attempt (PAM max-tries 3), closes 1.4 s after the last miss.
- Notifications (dunst rule → script; app, summary, body): a wider pill
  (420 px min, 92 tall) with an app icon + name line and summary — body
  line, ellipsised; a new one while open rolls the old one out through the
  top and the new one in from below (24 frames); 3.5 s timer restarted per
  notification; waits behind an active Touch ID; appears independently while
  the compact island is hidden, including over fullscreen windows.
  Icons: Telegram (blue paper plane), notify-send → gear "System", Firefox,
  Discord, mail, generic bell; critical = red.
- Launcher (mod+d): the island grows into a 660x512 panel (r=32, clock
  stays at the top, 24 min-jerk steps ~150 ms) with a search field and an
  app list with icons inside (currently walker, transparent, service mode,
  reveal 340 ms); Escape / launch / mod+d closes and the panel folds. A
  hidden island shows the panel temporarily and an open drawer closes first.
- Lock screen (hyprlock) is separate and stays as is.

The island's Control Center (shortcut modules, "Now playing" while a player
is active, Display/Sound sliders, audio
Sound page (outputs, per-application levels, microphone), keyboard layout, Lock/Sleep/Log out with a confirming second
press) opens from a normal left click on the ordinary compact body;
`src/control.rs` is its environment worker (never block the UI on it). mod+d still opens the
launcher directly. A right click on the clock opens the calendar sheet
(`src/calendar.rs`), with open-meteo weather under the month (`src/weather.rs`,
place from the time zone, fetched by a background `curl`). The config key
`compact` (`island-shell compact <item> on|off`, persisted) chooses what the
closed island shows; without the clock it rests as a short empty capsule and
activities part into Vorssaint's wings (cover and timer ring left, bars and
orange reading right, the song's title in the middle). Open
pages carry Vorssaint's floating drop buttons (`src/floating.rs`) on both sides;
the grid button opens the sections gallery (`quick::View::Sections`,
`island-shell sections`). Music controls are scoped to the equaliser (and the wings' cover) on the compact
island or the visible player panel: left play/pause, right next, middle
previous; scrolling changes tracks only there. Explicit controls, timer,
authentication and modal panels retain their interactions.

## Improvements wanted
True 60 fps interpolated motion (springs / min-jerk), pixel-stable text,
morphing shapes without GTK limits, native input (hover/click) instead of a
passthrough window + touch target, one process, the launcher and
notifications living inside the island.

## Plan
1. Core: layer-shell window, cairo/pango rendering at scale 1.5, clock,
   fillets, show/hide, hover, fullscreen auto-hide, dodge via sway IPC.
   Ship with a toggle so the old island can be disabled and compared.
2. Player + equaliser + Touch ID.
3. Notifications (own org.freedesktop.Notifications or via dunst).
4. Launcher (search, .desktop entries, icons, keyboard focus).
5. Polish: multi-output, config file, idle CPU ~0.

Status: steps 1–5 done (multi-output: `output = auto|focused|<name>`, the
surface is recreated on hotplug / `closed`, never exits; config keys in
island::Config, README "Configuration"); the old waybar island and its scripts are removed
(backup: ~/.config/waybar/old-island-backup.tar.gz), island-shell runs as
a systemd user service. Neovim integration lives in nvim/ and the iVim
repository (~/it/coding/ivim). iswm, a soft fork of SwayFX with island-styled
title bars and animations, lives in ~/it/coding/iswm (plan: `docs/iswm.md`);
the user dropped it and its `startiswm` (2026-10-31).
niri is supported too (2026-10-31): `startniri` (`scripts/startniri`,
installed to ~/.local/bin) starts `niri --session` from a TTY without
niri.service, so neither DankMaterialShell nor waybar.service (via
graphical-session.target) starts — an empty niri with the island as its whole
shell; `~/.config/niri/config.kdl` restarts island-shell.service at startup
(no dunst / nm-applet there; Mod+D launcher, Mod+I toggle, Mod+L island lock).
`src/compositor.rs` picks sway (`src/sway.rs`) or niri (`src/niri.rs`, when
NIRI_SOCKET answers). niri's questions are answered from a model kept from its
event stream (a fullscreen window sends a layout change every frame; asking
niri each time overflowed its socket); niri has no fullscreen flag, so a tiled
window the size of the island's output counts as one; `Outputs` can take a
second to answer and is never asked. Never edit ~/.local/bin/startway.

Live activities are native too: volume and brightness sliders, charging
and low-battery states, Bluetooth and Wi-Fi connection changes, screenshot
thumbnails, and a compact clickable countdown / Pomodoro beside the clock.
Jobs from `island-shell task|progress|run` (`src/tasks.rs`) are a ring
beside the clock and an unfolding pill; a job ends as interrupted when its
owner process exits (checked once a second, outside painting).
An orange dot shows while an application records from a microphone (sink
monitors such as cava excluded), and mic mute changes get a short pill.
Screen recording (`src/recorder.rs`, wf-recorder) counts down 3-2-1, shows a
red dot with its length beside the clock and flies the first frame in when done;
without wf-recorder a press shows a pill with the install command (each press
re-checks PATH). The Control Center's Screenshot tile folds the menu, then runs
`~/.config/sway/scripts/screenshot.sh` (else `island-screenshot`) in a scope.
Environment readers live in `src/activity.rs`; a grim wrapper is installed
as `~/.local/bin/island-screenshot`.
The timer opens from the island's shortcut menu as Vorssaint's timer page
(`src/timer_page.rs`: Timer / Pomodoro / Stopwatch, a minute ruler that drags,
scrolls and clicks, iOS round Cancel/Pause while running, Pomodoro phases that
wait for their start); a running session appears beside the clock; left click
pauses/resumes, right click opens the page, and Escape closes it.
`island-shell timer` opens the same page. The ruler's ticks and the timer's end
are sounds synthesized in `src/sound.rs` (the end: one soft low chime, G4 then
D5, played once; `pw-play` on a worker; config `timer_sound`): the
X1 Titanium's Sensel touchpad has no HID haptics page, so Vorssaint's haptic
ticks cannot be reproduced on it. The main Waybar has no timer, Wi-Fi or Bluetooth buttons, and its
⋯ drawer contains only disk, CPU and memory. `src/connections.rs` owns native connection sheets and their
motion; `src/network.rs` owns asynchronous NetworkManager and BlueZ operations.
Wi-Fi supports scanning, saved/open/personal secured connections, password
entry, disconnect and forgetting saved networks (out-of-range ones too).
Bluetooth has paired/discovery tabs, connection, disconnect, unpairing and
pairing with a scoped BlueZ agent for PIN/code confirmation. Forgetting takes
a confirming second press on the row's red bin. Connection panels in tests
and the gallery are `offline` and never reach NetworkManager or BlueZ.
Search is cancellable; replies carry session/request IDs and cannot reopen
a dismissed sheet. Escape or an outside click closes modal sheets and releases
keyboard focus. A temporary transparent layer surface catches outside clicks,
with an input-region hole that follows the island's animated geometry; the
closing click is consumed. It has exclusive zone 0, so it covers only the free
area: wvkbd (on-screen keyboard, `~/.config/sway/scripts/osk.sh`) and the bar
stay usable while a sheet is open; its offset comes from sway IPC
(`sway::Query::usable`).
The island's surface covers the whole output (input only over the box): the
island is one drop of a black frame beyond the screen's edges, of which only
the corners show (`src/bezel.rs`, config `screen_corners`, 16 px measured off
the lid). The four corner squares are wiped and the corners painted over
everything every frame (never `Clear` inside them: what lights the screen
inside the arc — edge light, the curtain — must stay), and edge light follows
the arc in the corners (a radial falloff from the arc meeting the straight
stretches). Future
whole-screen effects belong on this surface; their staged plan is
`docs/frame-effects.md`. Light along the edges (`src/edge.rs`, config
`edge_light`; light runs along `outline::Outline`, the frame's inner edge as
one loop from the middle of the island's lower edge (the box as last drawn)
round the island, along the top, round the bar's drops (down their sides,
under them, onto the side edges) and the screen; distances along that loop,
straight stretches as masked gradients and corners as radial ones, convex or
concave. The membrane's ripples ride every edge hanging from the top
(the island's, the drops') and the top edge between them, flowing
round each turn as one edge (`edge::Shore`, offset along its normal), and end
by slamming into the screen's sides: the crest piles up,
the corner there (a drop's fillet or the screen's) swells and shudders; pill waves wait
`PILL_LIGHT_DELAY` for the pill to open): a notification's colour runs along the top, a
critical one goes round, Touch ID breathes blue / flashes green / shudders
red; a timer draws nothing round the frame while it runs, and the frame flashes when it runs out. Recording's 3-2-1 flashes the frame red
(scheduled waves, cancelled with the countdown; nothing stays lit while
wf-recorder films the overlay); the microphone in use is an orange LED in
the frame's top-right corner (`edge::mic_led`, drawn after the bezel).
Volume / brightness changes no longer open the island: `src/side_hud.rs`
draws the old level pill turned on its side, a black drop of the frame
hanging from the left edge (concave fillets) half way up the upper half,
symbol / vertical slider / percentage, held 1.45 s after the last change
(with the Control Center open its own sliders show it); plugging in rises green from the bottom corners; a low battery (until
the next Charging event) leaves the bottom corners glowing red. The top edge
is a membrane (`edge::Membrane`, drawn with the frame marks, over the
island): a tug pulls it down round the island, letting go or hiding the
island sends ripples to the corners, whose radius quivers as they arrive;
lighter ones come from a new notification's pill, a workspace switch (from
the focus pill, `Bar::take_splash`) and an opening tray menu.
A screenshot blinks a rounded shutter in from every side
(`Curtain::shutter`); Do Not Disturb lights the island's own rim purple and
unread notifications light it blue (`Island::draw_rim`: drawn with the island
round its very outline — leaning and drawn out while tugged — from an exact
distance field of its shape through `outline::falloff`, kept as an A8 mask
while the shape stands still; compact keys `dnd` / `unread` switch them),
instead of a moon and a dot on its line; only on the closed line (tugged
too): anything opening puts it out in ~0.2 s, its mask not rebuilt for a
growing sheet. Frame light at corners is a cairo
mesh (slices along the arc × rings into the picture) so its strength changes
smoothly round a turn; while the island is tugged the frame light runs along
the top under it instead of round its box. Skipping a
track streaks the cover's colour along the top edge; `ambilight` (off by
default) lights the bottom edge with the cover's colour and the bass.
Locking or sleeping from the island (Control
Center, `island-shell lock|sleep`, mod+l) first switches the screen off into
the island like an old TV (`src/curtain.rs`), then sends the request; the
picture comes back out of the island when the locker exits
(`dbus::watch_unlock`, 6 s fallback; `island-shell wake`). Each edge is one composite (a gradient along it through a precomputed
falloff mask, only the lit stretch): ~2 ms a frame, never full outline strokes
(those cost 40–80 ms). Damage is a few rectangles (`app::Damage`), so edge
strips never turn into a whole-screen copy.
Touchscreen: the island binds wl_touch (`src/touch.rs`), so sway no longer
emulates clicks; one finger becomes pointer events. Sliders, the playhead,
the timer ruler and notifications are grabbed at once; elsewhere a tap clicks,
a 450 ms hold is the right button, a drag scrolls (axis-locked) with an iOS-like
glide stepped from the frame callback, and a sideways swipe over the music
changes tracks. A tap on the bars / editor chip reveals their panel; it stays
(with the outside catcher) until a touch lands outside or the mouse returns.
The player's × (also on the Control Center's music card, `island-shell
player-clear`) pauses the player and hides it; the MPRIS watcher keeps it
hidden until it plays again (`dbus::clear_player`). Timer and Bluetooth icons are matched SVG assets
in `assets/waybar`, installed under `~/.local/share/island-shell/icons`.
The compact equaliser on the right (and the wings' cover) is the only hover trigger for music;
Neovim's chip on the left is the hover trigger for editor details. An already
open panel stays open while the pointer moves across it.
The closed line can be tugged (`Island::tug_press/tug_to/tug_release`): a
tight rubber band (Apple's 0.55 constant), a lean, a drop-like stretch and a
jelly spring back; open pages cannot be tugged. Left presses on the closed
line therefore act on release (`App::tug_click` replays the press when it was
not a tug); a finger's drag there tugs, a tap stays a tap.
Hovering the ordinary compact body still widens its side padding on a spring,
even when music and Neovim are active. A panel target suppresses that padding
while the corresponding sheet unfolds.
Bluetooth discovery runs up to 30 seconds and updates names live; unnamed
advertisements remain explicitly unnamed with an address in the subtitle. OSD
entrance timestamps are separate from last-update timestamps: key repeat only
extends the hold. Cairo icon helpers start fresh paths so text positions never
become stray stroked lines.

The bar (`src/bar.rs`): two drops of the frame pressed into the top
corners, 32 px like the island (an empty top-layer surface reserves the room,
`BarReserve`): workspaces (sway events, a sliding focus pill, click / scroll,
up to four icons of the applications on each from the sway tree, resolved
through the desktop entries and loaded on the launcher's icon worker)
left; the drawer (disk / CPU / memory sampled once a second only while open,
`bar_sources.rs`; CPU and memory carry small graphs of the readings since it
opened, sliding left on a spring as each arrives), volume (click mute, scroll 5 %), layout (click: wvkbd),
battery (tooltip with the time left) and the tray right; the speaker, keyboard
and battery are vectors after SF Symbols (`src/bar_icons.rs`: one smooth
speaker body with even waves lit by the level, a mute
slash, keyboard.fill's three rows, a dotted ⋯, readings in Semibold, iOS's
battery with the percentage cut out of its fill, red ≤ 20 %,
green and a bolt while charging). A drop can be pulled down like the island
(`Bar::tug_press/tug_to/tug_release`, left presses on it act on release; a
pull jumps back on jelly and ripples the edge). The bar holds state
and returns `bar::Action`s; it does no I/O. The tray (`src/tray.rs`): the
island owns `org.kde.StatusNotifierWatcher` (same properties/signals as
Waybar's, lock.sh counts items) and hosts the items on threads; menus
(dbusmenu, variant-wrapped a{sv} values unwrapped) open as sheets hanging from
the right drop (`drop::notch` under its edge), driven by keys too (arrows /
Tab, Enter / Space / →, ← / Backspace back, Escape; exclusive keyboard focus
while open), closed by an outside click (the
dismiss catcher has holes for the bar).
Performance: the surface is the whole screen, so a full repaint and copy is
a 10–17 ms frame: only a new size (or the first configure) resets the canvas
(a configure that merely acknowledges a keyboard-focus change must not), and
`WARM_BUFFERS` spare shm buffers are filled while idle so a buffer first used
mid-animation never needs a whole-screen copy. Vector symbols (`quick::icon`, `quick::glyph`) and discs
(`menu_style::well`/`disc`) are stamped from cached A8 masks
(`menu_style::stamp`) at whole device pixels — an unfiltered mask composite;
a fractional offset makes cairo filter and costs ~10x. Rotated or freely
scaled ones (bounces, shakes) draw directly. Measure such changes with an
interleaved A/B in one process: the laptop's clock and load make separate runs
noisy. Desktop entries, icons and art are loaded on bounded background
workers (`background.rs`); launcher caches survive closing. Worker replies wake
the event loop; never add file decoding, process waits or directory scans to a
paint/input handler. Art generations reject stale results. Pango's cache is a
bounded LRU with pixel-quantized widths. Audio startup/mute polling are also
background jobs. Keep a single timed redraw source for a steady countdown.
The ignored `animation_benchmark` release test measures offscreen paint cost
at 1.5× against the real desktop catalog (not compositor latency).
Picture tests (`src/golden.rs`): scenes such as the bar's symbols, its drawer,
a pulled drop and a ripple hitting the wall are compared with references in
`tests/golden/` (colour channels, small slack); a mismatch writes the picture
and a difference map to `target/golden/`; `UPDATE_GOLDEN=1 cargo test`
accepts a deliberate change of look.

The launcher also searches filenames/folders under HOME using fdfind/fd:
two-character minimum, 180 ms debounce, 80 results, two worker threads,
three-second search limit. `file:`/`файл:` restricts results to files and
folders; ordinary queries combine apps and files. `files.rs` owns cancellable
search and shell-free, asynchronous opening. Directories use Alacritty;
text uses Neovim inside Alacritty; PDF uses Evince; office formats use
LibreOffice; other formats or missing openers fall back to xdg-open.
Calculations and unit/currency conversions answer first (`src/calc.rs`,
rates cached by a background `curl`); `clip:` lists the `cliphist` history
(`src/clipboard.rs`, read on a worker). Copying goes through `wl-copy` in a
systemd scope.
MIME probing with `file` runs only on opening the selected file, off the UI
thread; search itself never probes contents. Query generations and Launcher::drop cancel
stale work; results arrive through the existing asset wake channel. Never
read file contents or index hidden/cache/build directories as part of search.

All user applications and file openers use `launch.rs` to run in independent
systemd user scopes. Never spawn them directly in island-shell.service or use
setsid alone: service stop/restart kills the entire cgroup, including user
editors. Scope startup stays off the UI thread, preserves argv/environment,
and disables systemd argument expansion. Keep the service's default KillMode
for its own helpers. The ignored scope lifetime test stops only its private
test service and verifies that its spawned process survives.
