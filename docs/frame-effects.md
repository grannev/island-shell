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

## Stage 3 — screen off into the island
- [ ] Lock / sleep from the island: the frame closes the screen into a
      rounded rectangle that is drawn into the island (an old TV switching
      off); waking, the island lets the screen out again.

## Stage 4 — indicators
- [ ] Screen recording: a thin red rim around the screen, tied to the red dot.
- [ ] Camera / microphone in use: an orange dot in a frame corner (iOS).

## Stage 5 — levels and power
- [ ] Volume / brightness: a thin level slides out of the right edge.
- [ ] Charging: green "liquid" rises from the bottom corners and drains.
- [ ] Low battery: the bottom corners smoulder red.

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
