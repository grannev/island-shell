# Frame effects — roadmap

The island's surface covers the whole output and the island is one drop of
a black frame beyond the screen's edges (`src/bezel.rs`). These effects use
that frame: events flow out of the island along the screen's edges and back.
Constraints: a Wayland client cannot read what is under it, so everything is
drawn over the screen (light, colour, dimming, more black); edge effects
damage only thin strips, whole-screen effects are short.

Status: [ ] to do, [x] done.

## Stage 1 — infrastructure and light along the edge (done: `src/edge.rs`, config `edge_light`)
- [x] Edge layer: the island draws effects along the screen's rounded outline;
      damage as several thin rectangles instead of one bounding box.
- [x] Notification glow: a thin light in the app's colour leaves the island
      and runs along the top edge towards the corners, fading.
- [x] Critical notification: a red pulse runs around the whole screen.
- [x] Touch ID: a slow breathing blue wave around the perimeter while
      waiting; a green flash on success, a red shudder from the island on a
      miss.

## Stage 2 — the frame as a gauge (done)
- [x] Timer / Pomodoro along the perimeter: a thin line goes round the screen
      as time runs (phases in different colours); at the end it closes and
      the frame flashes. (A stopwatch is a comet going round once a minute;
      a paused timer's line dims; the line is still between the 1 Hz ticks.)

## Stage 3 — screen off into the island (done: `src/curtain.rs`)
- [x] Lock / sleep from the island: the frame closes the screen into a
      rounded rectangle that is drawn into the island (an old TV switching
      off); waking, the island lets the screen out again. Triggers: the
      Control Center's Lock / Sleep, `island-shell lock|sleep` (mod+l in
      sway). The lock or suspend goes out once the screen is off; a thread
      waits for hyprlock/swaylock to come and go (`dbus::watch_unlock`; a
      locker that never shows within 6 s counts as gone). `island-shell
      curtain` is a demo without locking, `island-shell wake` a way out.
      Not yet: locks the island did not start (swayidle idle / before-sleep).

## Stage 4 — indicators (done)
- [x] Screen recording: the frame flashes red with each count of 3-2-1 (the
      flashes end before capture starts) and a red wave leaves the island
      when it stops. Not a steady rim: wf-recorder films the overlay too.
- [x] Microphone in use: an orange LED in the frame's black top-right
      corner with a glow on the screen (`edge::mic_led`), shown even while
      the island is hidden.
- [ ] (skipped) Camera in use: the laptop has a camera LED; detecting an
      open /dev/video* means scanning /proc and costs idle CPU.

## Stage 5 — levels and power (done)
- [x] Volume / brightness: a thin level stands up the screen's side while
      the level pill shows (volume right, brightness left, warm).
- [x] Charging: green "liquid" rises from the bottom corners and drains.
- [x] Low battery: the bottom corners smoulder red until the charger is
      plugged in (a still glow: no frames of its own).

## Stage 6 — the frame as a material
- [ ] Rubber edge: a hard tug bends the top edge after the island like a
      membrane; let go, a wave runs to the corners and they quiver.
- [ ] Drop into water: hiding the island (mod+i) sends a ripple along the
      top edge to the corners.

## Stage 7 — moments
- [ ] Screenshot shutter: the frame closes in from all sides as a rounded
      iris and opens again.
- [ ] Focus / Do Not Disturb: the frame thickens by 6–8 px while it is on.
- [ ] Hot corners: a pointer pressed into a corner bulges it like rubber
      (the frame never takes input).

## Stage 8 — music (optional, config)
- [ ] Ambilight: a faint glow along the bottom edge in the cover's colours,
      breathing with the bass.
- [ ] Track change: a streak of light along the top edge in the swipe's
      direction.
