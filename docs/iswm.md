# iswm — island-sway WM: stages 1–2

iswm is a soft fork of SwayFX whose window decorations and motion follow the
island's look. It is not a rewrite: the tree, layouts, i3/sway IPC, input,
Xwayland and every protocol stay SwayFX's. iswm adds its own decoration
drawing and an animation layer over sway's scene graph. A rewrite on
Smithay comes up again only if stage 3 (animated tiling, see the end) finds
sway's transaction model impossible to work with.

Status: [ ] to do, [x] done.

## Where the work lives

**The repository is `~/it/coding/iswm`, and all iswm work continues there**
(code, its own CLAUDE.md, docs, commits). island-shell only keeps this plan
and whatever the island itself needs (for example IPC that iswm might add).
`~/.local/src/swayfx` stays as it is: it is the build of the current
`/usr/local/bin/sway` and the fallback session.

Base, as installed today: SwayFX 0.5.1 at `1710f7d` (sway 1.10.1),
wlroots 0.18.2, scenefx 0.3.0. On top of it, the uncommitted working tree
of `~/.local/src/swayfx` carries about 790 lines of the user's own changes,
whose sources are `~/.config/sway/patches/*.patch`.

### Stage 0 — the repository
- [ ] `git clone https://github.com/WillPower3309/swayfx ~/it/coding/iswm`,
      check out `1710f7d`, rename the remote to `upstream`, start `main` there.
- [ ] One commit per existing change, in this order: `titlebar-position`
      (`titlebar_position top|bottom`), `titlebar-controls` (`floating_csd`,
      the close button, `close_icon.c`), `touch-resize`, the floating-move
      tear-off / docking / drop preview (`seatop_move_floating.c`), and
      `lock-no-black-frame`. The patch files may lag behind the working
      tree, so the working tree is the source of truth. Check:
      `diff -r` of both trees (outside `build*/`) is empty.
- [ ] `CLAUDE.md` for iswm: reply in Russian, code, comments and commits in
      English, the same commit trailers as island-shell, no sudo from the
      agent (root steps go into numbered scripts), a pointer to this plan,
      and C in sway's own style (tabs, `sway_log`, `wl_list`).
- [ ] The binary is `iswm` (meson `executable`). The IPC stays byte-for-byte
      sway's, and it still exports `SWAYSOCK` / `I3SOCK`, so island-shell,
      `swaymsg` and the scripts keep working unchanged.
- [ ] Config: iswm reads `~/.config/iswm/config` first, which does
      `include ~/.config/sway/config`. iswm-only options go there, so stock
      swayfx never sees commands it does not know.
- [ ] Install to its own prefix (`/usr/local` with the binary renamed is
      enough) with a separate `iswm.desktop` in
      `/usr/share/wayland-sessions`. The user runs this from
      `scripts/01-install.sh`. The swayfx session stays selectable as a
      fallback.
- [ ] Development loop: `meson setup build && ninja -C build`, then run
      nested inside the current session: `WLR_BACKENDS=wayland
      ./build/sway/iswm -c <test config>`. A crash of the nested instance
      cannot take the session down. Before relying on it, check whether a
      second island-shell can run against the nested `WAYLAND_DISPLAY` /
      `SWAYSOCK`: some D-Bus names (Notifications, StatusNotifierWatcher) can
      have only one owner per session.
- [ ] Picture tests in C, like island-shell's `golden.rs`: a `meson test`
      executable renders decoration scenes to PNG and compares them with
      `tests/golden/*.png` (colour channels, small slack). On a mismatch it
      writes the picture and a difference map to `build/golden/`, and
      `UPDATE_GOLDEN=1` accepts a deliberate change.

Line numbers below refer to `~/.local/src/swayfx` (base plus current
patches). They are the same in iswm after stage 0.

## Stage 1 — title bars in the island's style

The look: the title bar is a black drop like the island, hanging off the
window's edge where the bar is (`titlebar_position bottom` today, top as
well). It is content-sized and centred, with corners r=14 away from the
window and 14 px concave fillets where it meets the window's edge. Text is
SF Pro Text Semibold 16 px, white. Unfocused windows dim the text to about
60 %; hover is #000 → #0A0A0A, as on the island. The rest of the title strip
stays empty, so the wallpaper and the blur show through.

- [ ] **A shared cairo buffer.** `sway_text_node.c` (cairo_buffer_*) and
      `close_icon.c` each wrap a cairo surface in a `wlr_buffer`. Factor this
      into `sway/cairo_buffer.c` before adding a third copy.
- [ ] **Drawing** (`sway/island_deco.c`, `include/sway/island_deco.h`): a pure
      function `island_deco_render(cairo_t *, const struct island_deco_style *,
      width, height, scale, state)` draws the drop: a rounded box plus
      fillets, edge side up or down. Buffers are cached by
      (w, h, scale, colour, state), like the `close_icon.c` cache, and drawn
      at the output's scale, with the edge on whole device pixels so that
      1.5× stays crisp.
- [ ] **Scene nodes.** `struct sway_container.title_bar`
      (`include/sway/tree/container.h:75`) gets a
      `struct wlr_scene_buffer *shape`, created in `container_create`
      (`sway/tree/container.c:81`). With `titlebar_style island`,
      `title_bar.background` / `border` (rects) are turned off.
- [ ] **Layout.** `container_arrange_title_bar` (`container.c:381`)
      measures the title (`sway_text_node` width), sets the drop's width to
      text + padding 16 ×2 + close button, capped at the bar's width, centres
      it, and places the text and the × inside it. Long titles are cut with
      "…" by `sway_text_node_set_max_width`. `arrange_title_bar` in
      `sway/desktop/transaction.c:258` stays the entry point.
- [ ] **Colours and state.** `container_update` (`container.c:309`) chooses
      the state (focused / inactive / unfocused / urgent / hover) and the
      cached buffer. The fill comes from the `client.*` background column, so
      the theme stays in the config (`client.focused #000000 …`).
      The text node's background is transparent.
- [ ] **Close button.** Today it is a red rect plus × at the bar's end
      (`close_bg` / `close_icon`). In island style it becomes a 20 px disc
      inside the drop's right end: #2A2A2A with a white ×, red on hover.
      The hit testing in `container_close_button_at` (`container.c:620`)
      follows the disc.
- [ ] **Input.** `container_title_bar_at` (`container.c:606`) keeps the whole
      strip as the title bar, so a press beside the drop still focuses and
      drags (the docking and tear-off in `seatop_move_floating.c` depend on
      it).
- [ ] **Tabbed / stacked.** The tab row is one drop. The active tab is a
      lighter pill (#2A2A2A) under its title, like the bar's workspace focus
      pill. It is static in stage 1 and slides in stage 2 (2.6).
      `arrange_children` (`transaction.c:291`) places the segments.
- [ ] **Config.** `titlebar_style default|island`
      (`sway/commands/titlebar_style.c`) and `titlebar_corner_radius <px>`.
      Setting `font pango:SF Pro Text Semibold 16px` is the user's choice in
      the config.
- [ ] **Application icon (optional, last).** A 16 px icon left of the title,
      from `app_id` → desktop entry `Icon=` → icon theme. This needs PNG and
      SVG loading (librsvg) in the compositor, off the main loop. If it costs
      too much, leave it out.
- [ ] **Golden scenes:** focused, unfocused, hover on ×, a long ellipsised
      title, tabbed with three tabs, top and bottom position, at 1× and 1.5×.

Done when, in the nested session and then the real one, a tiled window, a
floating one and tabbed / stacked groups look like the island, text is crisp
at 1.5, docking and tear-off still work, and stock swayfx still reads
`~/.config/sway/config` without errors.

## Stage 2 — animations that don't resize clients

The rule: an animation only moves, fades or scales **scene nodes**. It never
changes the tree's `pending` / `current` state and never calls
`view_configure`. Transactions stay the truth, and an animation is a visual
offset from `current` that decays to zero. Everything that changes a
client's size (tiling reflow, fullscreen, floating toggle) is stage 3.

### Infrastructure
- [ ] `sway/desktop/animation.c`: the list of running animations. Each one
      has a target (scene node or ghost), a property (x/y offset, opacity,
      scale via `wlr_scene_buffer_set_dest_size`, corner radius), from/to,
      start, duration and a curve: min-jerk 10t³−15t⁴+6t⁵ like the island, or
      a critically damped spring where it must be retargeted.
- [ ] Ticked from `handle_frame` (`sway/desktop/output.c:363`) before the
      scene output commits, sampled at the predicted presentation time
      (`last_presentation + refresh_nsec`, already computed there). While
      anything runs on an output it calls `wlr_output_schedule_frame`. Then
      there are no timers and no wakeups when idle, so idle CPU stays at 0.
      wlr_scene does the damage itself.
- [ ] Retargeting: a new transaction that moves a node mid-flight starts
      from its current visual position (a spring keeps its velocity). There
      are no jumps.
- [ ] Opacity multiplies with sway's `opacity` command (`con->alpha`) and is
      applied per buffer through `wlr_scene_node_for_each_buffer` +
      `wlr_scene_buffer_set_opacity` (scenefx).
- [ ] Session lock: when it starts (`sway/lock.c`) every animation
      finishes instantly. Nothing moves under or over the locker.
- [ ] Config: `animations on|off` and
      `animation <kind> <ms> [min_jerk|spring]`; `debug noanim` for tests.
- [ ] Input during motion: `wlr_scene_node_at` hits visual positions, which
      is what the eye expects. At the end of an animation call
      `cursor_rebase_all()` so hover and focus-follows-mouse catch up.
- [ ] A unit test of the curves and of retargeting (a `meson test`, no
      compositor).

### Animations
- [ ] **2.1 Window opens.** In `view_map` (`sway/tree/view.c:743`) mark the
      container. When the first transaction that gives it geometry is
      applied (`transaction_apply`, `transaction.c:817`, then `arrange_root`),
      start a fade 0 → 1 with a 12 px rise toward its place, ~220 ms min-jerk.
      This works on the live surface tree, popups and subsurfaces included.
      A scale-in would need a snapshot and is left for stage 3.
- [ ] **2.2 Window closes.** In `view_unmap` (`view.c:907`), before
      `container_begin_destroy`, copy the view's buffers like
      `view_save_buffer_iterator` (`view.c:1180`) into a **ghost** tree. The
      ghost is reparented into a per-output layer above `tiling` and below
      `shell_top` (`include/sway/output.h:25`), at the same layout position,
      and the title drop's buffer is copied with it. The ghost holds the
      buffer locks, so the picture survives the client's death. It shrinks to
      0.92 and fades in ~180 ms, then it is destroyed. Neighbours reflow at
      once for now (stage 3 animates that).
- [ ] **2.3 Workspace switch.** `workspace_switch`
      (`sway/tree/workspace.c:584`) → `arrange_output` (`transaction.c:678`)
      turns the old workspace's trees off. Instead, keep them on for the
      length of the animation and slide old and new horizontally. The
      direction comes from comparing workspace numbers or names. It takes
      ~250 ms min-jerk, in step with the island bar's focus pill (which
      already animates on the IPC event). A quick second switch retargets.
      A fullscreen window moves with its workspace (`layers.fullscreen`).
- [ ] **2.4 Floating windows move by command.** In
      `arrange_workspace_floating` (`transaction.c:616`), if a floating
      container's position changed and the size did not, it glides
      (`move position`, `move to center`, scratchpad). Interactive drags in
      `seatop_move_floating.c` stay 1:1 under the finger and the pointer.
- [ ] **2.5 Scratchpad.** `scratchpad show` drops the window out of the
      island's direction (from the top: position plus fade), and hiding
      lifts it back. This is a position-only variant of 2.4.
- [ ] **2.6 Tab pill.** In a tabbed or stacked group the active tab's pill
      slides to the new tab, like the bar's workspace pill.
- [ ] **2.7 Focus and hover.** The title drop crossfades between two cached
      buffers (the old and new state, opacity) rather than redrawing every
      frame. SwayFX's inactive-window dim fades in and out. The × hover fades.
- [ ] Manual checklist in the nested session, then the real one: Firefox,
      Alacritty, an Xwayland window, popups and menus during 2.1–2.3, mod+l
      during an animation, fullscreen video on another workspace,
      `grim` / `wf-recorder` while switching.
- [ ] Measure: frame time with `debug txn-timings` and `WLR_DEBUG`, and idle
      CPU with `top` after animations stop. It must be 0 %.

Done when every animation above runs at 60 fps in the real session,
`animations off` restores sway's exact behaviour, island-shell sees the same
IPC events at the same moments, and nothing is left drawn after a quick
sequence (open, close, switch, switch back).

## After that: stage 3 (not planned here)
Animated tiling reflow, fullscreen and floating toggle change the client's
size. Sway applies a layout only after every client has acknowledged its
new size (`transaction_commit`, `transaction.c:921`; timeout
`txn_timeout_ms`). Animating it means interpolating each container's box
from the old `current` to the new one: the saved snapshot is scaled while
the client catches up, then crossfaded to its new buffer (the way Hyprland
and niri do it). Before starting, measure how slow real clients are to
answer (Firefox, Electron, Xwayland). That decides whether sway's model can
carry it or the Smithay question returns.
