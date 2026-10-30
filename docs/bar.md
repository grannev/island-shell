# The bar in the island — inventory and plan

Goal: replace the main Waybar (`~/.config/waybar/sway.jsonc` + `sway.css`,
started by sway's `bar { swaybar_command scripts/waybar.sh }`) with a module of
island-shell. Every element stays where it is now; the left group becomes a
drop of the island's black frame pressed into the top-left corner, the right
group a drop pressed into the top-right corner.

## 1. What Waybar has today (2026-10-31)

Bar: layer `top`, top edge, 26 px tall (logical), exclusive zone 26,
transparent, SF Pro Text Medium 16 px white, no separators. Every module is as
wide as its text; width changes slide the neighbours (`width-anim.sh` +
`hN`/`gN` padding classes). Hover: white 0.10 wash, radius 5, 180 ms.

### Left (`modules-left`)
| element | content | behaviour |
|---|---|---|
| `sway/workspaces` | names (`1 2 3 10`), padding 0 11, margin 3/1 | text white 0.60; focused: white, Black weight, white 0.18 pill r=3; urgent: #FF8C42 on orange 0.18; hover: white 0.10; click switches, scroll cycles |

### Right (`modules-right`, left to right)
| element | content | behaviour |
|---|---|---|
| `group/details` handle `custom/details` | `􀍠` (ellipsis), padding 0 12 | click (or mod+o, SIGRTMIN+9 via `details-toggle.sh`) toggles the drawer; the island dodges 240 px left (`island-shell dodge-toggle`) |
| drawer (`disk`, `cpu`, `memory`) | `􀧘 {free}` /, `􀧓 {usage}%` every 3 s, `􀧖 {used:0.1f}GB` every 5 s | revealed to the left of the handle, 350 ms; patched Waybar (signal toggle) |
| `custom/volume` (`bar-volume.sh`) | `speaker.wave.1/2/3` (U+1002A5/A7/A9 by ≤33/≤66/>66 %) + `N%`; muted: U+1002A3 + `N%` in #8FCDEF | click mute, scroll ±5 % (capped at 100 %); pactl subscribe; writes `$XDG_RUNTIME_DIR/waybar-volume` |
| `custom/lang` (`sway-lang.sh`) | `keyboard.fill` U+100E91 + layout name without "(US)" (first non-virtual keyboard) | click toggles the on-screen keyboard (`osk.sh toggle`) |
| `custom/battery` (`bar-battery.sh`) | battery.0/25/50/75/100 (U+1006EA/E9/100EB6/EB8/1006E8) or bolt U+10088B when charging + `N%` | warning ≤30 % #FFD166, critical ≤15 % #FF8C42 (not when charging); tooltip "status, Xh YYmin" |
| `tray` | StatusNotifierItems, 18 px icons, spacing 6 (now: 2 items) | Waybar is the `org.kde.StatusNotifierWatcher` and the host; click / right-click menus (dbusmenu) |

### Outside Waybar that depends on it
- `sway/config`: the `bar { … }` block starts it; its exclusive zone (26 px)
  keeps windows below the bar.
- `scripts/lock.sh` reads `RegisteredStatusNotifierItems` from the watcher to
  place hyprlock's status line.
- `scripts/details-toggle.sh` (mod+o) signals the patched Waybar.
- `island-player.sh` used `waybar-volume` (the old island; already retired).

## Decisions (2026-10-31)
- The drops are as tall as the island, 32 px (Waybar was 26): the bar's
  reserved room is 32 px.
- Everything — the island and the bar drops — on every output (each output's
  drops show its own workspaces).
- Tray menus are island sheets out of the right drop.

## 2. Architecture

### 2.1 One shape language: `src/drop.rs`
Today three pieces trace "a drop of the frame" separately: the island's notch
(`Island::trace_notch_with`), the side level (`side_hud::drop_shape`) and the
membrane. Extract one geometry: a `Drop` is a rectangle attached to one or two
screen edges, with concave fillets where it leaves an edge and convex corners
where it is free (continuous-curvature `Corner`s, the island's smoothing).
- top-centre: the island (attached to the top, fillets both sides);
- left edge: the side level (attached to the left);
- top-left corner: the left bar drop (attached to top and left: a fillet
  where it leaves the top edge, one where it leaves the left edge, one convex
  corner bottom-right);
- top-right corner: the mirror.
The island, the side level and the bar drops all trace through it; edge light
(`edge::Way`) learns to run round any drop, so a notification's light can
pass round the bar drops on its way along the frame.

### 2.2 The bar as a component: `src/bar/`
```
bar/mod.rs        Bar: state, layout, draw, hit testing, input; no I/O
bar/layout.rs     a row of items with animated widths (springs), left / right
bar/workspaces.rs the left drop's buttons
bar/status.rs     volume, layout, battery, details handle + drawer items
bar/tray/         watcher.rs (org.kde.StatusNotifierWatcher), host.rs, item.rs
                  (icons, Activate / ContextMenu / Scroll), menu.rs (dbusmenu)
bar/sources.rs    environment workers feeding `BarEvent`s into the loop
```
- `Bar` is a sibling of `Island`, owned by `App`; both paint on the island's
  full-screen surface. `Bar::draw(cr, renderer, w, now) -> BarFrame { bounds,
  input: Vec<Rect> }`: bounds go into `app::Damage`, the input rects join the
  island's input region (a `Region` takes several rectangles).
- Pointer routing in `App::pointer_events`: the bar first (its drops never
  overlap the island's box), then the island. Touch comes through the same
  path for free.
- No I/O in paint or input: everything the bar shows arrives as `BarEvent`s
  on a calloop channel (like `dbus::Event`): `Workspaces(Vec<Ws>)`,
  `Volume{value, muted}`, `Layout(String)`, `Battery{percent, charging,
  remaining}`, `Details{disk, cpu, memory}`, `Tray(TrayEvent)`.
- Commands go out through existing workers: workspace switch through sway IPC
  (`sway::Query::command`), volume / mute through `control.rs`, the on-screen
  keyboard and tray activations off the UI thread (`launch.rs` / zbus threads).

### 2.3 Sources (reuse what already runs)
- workspaces: subscribe to sway `workspace` events in the existing
  `sway::Events`; on change query `get_workspaces` (name, focused, visible,
  urgent, output).
- volume: `activity.rs` already follows `pactl subscribe`; publish the state
  alongside the OSD event.
- battery: `activity.rs::spawn_power` already reads it; add a state event
  (percent, status, energy/power for the remaining time).
- layout: `control.rs` reads it from sway; follow `input` events instead of
  polling.
- disk / cpu / memory: `/proc/stat`, `/proc/meminfo`, `statvfs("/")` on a
  worker **only while the drawer is open** (Waybar polls them always).
- tray: own `org.kde.StatusNotifierWatcher` (same properties and signals, so
  `lock.sh` keeps working) and register a host; per item read `IconName` /
  `IconPixmap` / `IconThemePath` / `Status` / `ToolTip` / `Menu`, follow
  `NewIcon` / `NewStatus`; decode icons on `background.rs` workers.

### 2.4 Look and motion (Apple-like, from the island's toolkit)
- Drops are the island's black, 26 px tall like Waybar, content at the same
  x positions; each drop's width follows its content on the island's Morph
  springs (grow with a swing, shrink without), replacing `width-anim.sh`.
- Workspaces: the focused highlight is one pill that slides between buttons
  on a spring (not a fade per button); urgent ones breathe orange; new
  workspaces grow in, closed ones fold out.
- Values roll their digits (`motion::text`), symbols are stamped vectors /
  SF Symbols; the speaker's waves follow the level.
- The ⋯ drawer: the right drop grows leftwards, disk / cpu / memory come in
  with a stagger; the island dodges ahead of it as it does now, driven
  internally (no signal, no script); mod+o → `island-shell details`.
- Tray menus open as island sheets out of the right drop (menu_style rows),
  with Escape / outside-click like the other modal sheets.
- Fullscreen: the drops draw up into the edge with the island; hover widens a
  drop's item wash like Waybar's; battery's tooltip is a small pill under the
  right drop.

### 2.5 Taking over from Waybar
- Exclusive zone: a tiny transparent `top` layer surface (no input) anchored
  top/left/right with exclusive zone 26 reserves the bar's room; the overlay
  surface keeps `exclusive_zone -1`.
- Config `bar = on|off` (off by default until it is complete); while off
  nothing of it runs. Switching: remove the sway `bar { }` block (or point it
  at a no-op), rebind mod+o, keep `waybar.sh` for a quick way back.
- Only one process may own the watcher name: the tray part starts only when
  Waybar is gone (or takes the name over if it is free).

## 3. Stages (1–7 done 2026-10-31; Waybar is off, `bar = on`)
1. `drop.rs`: one drop geometry; the island and the side level use it;
   gallery shots of the corner drops.
2. Bar skeleton: `Bar` with two empty corner drops, layout rows with animated
   widths, input / damage integration, the exclusive-zone reserver,
   fullscreen hiding, config `bar`.
3. Workspaces: sway events, buttons, sliding focus pill, urgent, click /
   scroll.
4. Volume, layout, battery: state events, symbols, rolling digits, click /
   scroll / on-screen keyboard, battery colours and remaining time.
5. Details drawer: ⋯, disk / cpu / memory sampled only while open, island
   dodge driven by the bar, `island-shell details`.
6. Tray: watcher + host, icons, activate / secondary / scroll; then dbusmenu
   menus as island sheets.
7. Switch-over: sway config, mod+o, lock.sh check, Waybar kept as fallback;
   README and CLAUDE.md.
8. Every output: one surface (canvas, buffers, damage) per output; the
   island's state drawn on each, hit rects per surface; each output's bar
   shows its own workspaces.
