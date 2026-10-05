# island-shell — project brief

Reply to the user in Russian. Code, comments and commit messages in English.
For menu and sheet controls, follow the role-based, permissive visual guide
in `docs/interface-style.md`; homogeneous controls share a style, while new
control types remain allowed when their interaction calls for one.

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
- The main waybar (~/.config/waybar/sway.jsonc, bar "main") stays: 26 px
  tall, transparent, SF Pro Text Medium 16 px; its "⋯" drawer (group
  signal SIGRTMIN+9, mod+o = ~/.config/sway/scripts/details-toggle.sh)
  opens to the left of the right-hand modules — the island must dodge it.
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

The island's Control Center (shortcut modules, Display/Sound sliders, audio
Sound page (outputs, per-application levels, microphone), keyboard layout, Lock/Sleep/Log out with a confirming second
press) opens from a normal left click on the ordinary compact body;
`src/control.rs` is its environment worker (never block the UI on it). mod+d still opens the
launcher directly. A right click on the clock opens the calendar sheet
(`src/calendar.rs`). Music controls are scoped to the equaliser on the compact
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
repository (~/it/coding/ivim).

Live activities are native too: volume and brightness sliders, charging
and low-battery states, Bluetooth and Wi-Fi connection changes, screenshot
thumbnails, and a compact clickable countdown / Pomodoro beside the clock.
Jobs from `island-shell task|progress|run` (`src/tasks.rs`) are a ring
beside the clock and an unfolding pill; a job ends as interrupted when its
owner process exits (checked once a second, outside painting).
Environment readers live in `src/activity.rs`; a grim wrapper is installed
as `~/.local/bin/island-screenshot`.
The timer opens from the island's shortcut menu; a running
countdown appears beside the clock; left click pauses/resumes, right click
opens controls, and Escape closes the panel. `island-shell timer` opens the
same panel. The main Waybar has no timer, Wi-Fi or Bluetooth buttons, and its
⋯ drawer contains only disk, CPU and memory. `src/connections.rs` owns native connection sheets and their
motion; `src/network.rs` owns asynchronous NetworkManager and BlueZ operations.
Wi-Fi supports scanning, saved/open/personal secured connections, password
entry and disconnect. Bluetooth has paired/discovery tabs, connection,
disconnect and pairing with a scoped BlueZ agent for PIN/code confirmation.
Search is cancellable; replies carry session/request IDs and cannot reopen
a dismissed sheet. Escape or an outside click closes modal sheets and releases
keyboard focus. A temporary transparent layer surface catches outside clicks,
with an input-region hole that follows the island's animated geometry; the
closing click is consumed. Timer and Bluetooth icons are matched SVG assets
in `assets/waybar`, installed under `~/.local/share/island-shell/icons`.
The compact equaliser on the right is the only hover trigger for music;
Neovim's chip on the left is the hover trigger for editor details. An already
open panel stays open while the pointer moves across it.
Hovering the ordinary compact body still widens its side padding on a spring,
even when music and Neovim are active. A panel target suppresses that padding
while the corresponding sheet unfolds.
Bluetooth discovery runs up to 30 seconds and updates names live; unnamed
advertisements remain explicitly unnamed with an address in the subtitle. OSD
entrance timestamps are separate from last-update timestamps: key repeat only
extends the hold. Cairo icon helpers start fresh paths so text positions never
become stray stroked lines.

Performance: desktop entries, icons and art are loaded on bounded background
workers (`background.rs`); launcher caches survive closing. Worker replies wake
the event loop; never add file decoding, process waits or directory scans to a
paint/input handler. Art generations reject stale results. Pango's cache is a
bounded LRU with pixel-quantized widths. Audio startup/mute polling are also
background jobs. Keep a single timed redraw source for a steady countdown.
The ignored `animation_benchmark` release test measures offscreen paint cost
at 1.5× against the real desktop catalog (not compositor latency).

The launcher also searches filenames/folders under HOME using fdfind/fd:
two-character minimum, 180 ms debounce, 80 results, two worker threads,
three-second search limit. `file:`/`файл:` restricts results to files and
folders; ordinary queries combine apps and files. `files.rs` owns cancellable
search and shell-free, asynchronous opening. Directories use Alacritty;
text uses Neovim inside Alacritty; PDF uses Evince; office formats use
LibreOffice; other formats or missing openers fall back to xdg-open.
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
