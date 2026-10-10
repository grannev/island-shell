# Island interface style

The island's sheets follow Apple's dark-mode design language: system
colours, SF Pro text styles, capsule buttons, grouped lists and a circular
close control. Everything is drawn by `src/menu_style.rs`; match the role of
an element before matching its appearance, and draw it with that module's
helper rather than local geometry. New kinds of controls are welcome when
their interaction differs (a slider, a countdown ring, a swipeable row) —
give the new family a clear purpose and add its helper to `menu_style.rs`.

## Tokens

- **Colours:** `BLUE` #0A84FF (actions, selection, connected state),
  `GREEN` #30D158, `ORANGE` #FF9F0A (timer), `RED` #FF453A (removal,
  errors, notifications), `PURPLE` #BF5AF2 (Do Not Disturb), `TEAL`
  #64D2FF, `GRAY` #636366. Text is white at `PRIMARY` 1.0, `SECONDARY`
  0.6 or `TERTIARY` 0.3. Cards, tiles and lists use `GROUPED` #1C1C1E on
  the island's black. No outlines on buttons, cards or tiles.
- **Type** (`Font`): `Title` Semibold 17 (sheet titles), `Headline`
  Semibold 15 (row and tile titles), `Body` Regular 15 (list rows, running
  text), `Subhead` Regular 13 (second lines, status), `Label` Medium 14
  (buttons, segments), `Footnote` Regular 12 (timestamps, status lines).
  The compact island keeps its own `Text`/`Caption`/`Small`/`Tiny`.
- **Geometry:** a 22 px outer rail (`RAIL`); the header row is centred on
  y = 40 (`HEADER_MID`); content starts at y = 76 (`CONTENT_TOP`); 22 px
  bottom rail. Grouped lists and cards have 12 px corners, quick-menu tiles
  16 px, app tiles 25 % of their size.

## Roles

- **Header** (`header`): optional 36 px app tile with a white glyph, title,
  optional subtitle stacked on the header centre line. Trailing controls
  (a switch, a small "Очистить" capsule) sit left of the close control and
  share the centre line.
- **Close** (`close`, `close_rect`): a 30 px grey circle with an xmark at
  the header's right end, in every sheet and the launcher. Whether it
  dismisses or returns to the quick menu comes from navigation context.
- **Buttons** (`button`): capsules, 34 px (`BUTTON_H`) or 30 px
  (`SMALL_BUTTON_H`). `Primary` is filled blue for the main action,
  `Neutral` grey, `Warm` orange-tinted for the timer's chosen duration /
  resume, `Destructive` red-tinted for removal and cancelling, `Plain`
  blue text for navigation ("‹ Назад") and inline toggles ("Показать").
  Disabled controls keep their shape at reduced opacity.
- **Grouped lists** (`group`, `separator`, `highlight`): one rounded
  container per list, hairline separators inset to the text column, a
  rounded wash under the pointer. Leading symbols sit in a round `well`
  (blue when connected / active). A row shows its state at rest (signal,
  lock, battery, time) and what a click does only under the pointer.
- **Cards**: notifications are separate `group` platters, as in
  Notification Center; time and an unread dot on the right, × on hover.
- **Open-island pages** (Control Center, after Vorssaint): a 24 px inset,
  a 36 px title row at y 12 (title left; at its end a faint ⋯ that becomes a
  28 px collapse chevron under the pointer, or a back chevron before the
  title on a sub-page), content from y 60, 16 px at the bottom. **Cards**
  are a white 6.5 % wash with 18 px corners and no outline.
- **Shortcut tiles**: a 40 px round well (white 7.5 %, or the tile's colour
  when it is on) over a one-line `Footnote` label that reads the tile's
  state when it has one; under the pointer a white 9 % wash with 14 px
  corners and a 2 % lift; pressed, a dip.
- **Segmented controls, switches, fields** (`segmented`, `toggle`,
  `field`): the iOS shapes; focus is a blue ring, selection a grey thumb.
- **Message field** (`reply_field`): a capsule with a dim placeholder,
  a blinking blue caret and a round send button that turns blue once
  there is text; the notification pill and the center's details share it.
- **Swipeable cards**: a notification pill or center card follows two
  fingers or a held drag sideways; past 30 % or on a flick it flies out
  and closes, otherwise it springs back.
- **Calendar days**: today is a red disc with a semibold number, a
  selected day a white disc with a black number, a hovered day a light
  wash; weekend days and weekday names are dimmer.
- **Selection and hover** are a light card (`group` at full hover), the
  same in the launcher and the notification center; no solid fills.
- **Scrolling** glides (`anim::Glide`): a wheel notch is a push of about
  one row that starts at once and eases out, quick notches extend one
  glide, a touchpad moves content 1:1. Lists scroll continuously (never
  snapping to whole rows), dissolve into soft `edge_fades`, and show an
  overlay indicator only while moving.

## Motion

- **The box's shape** moves on Vorssaint's springs (`anim::Morph`):
  growing swings a little past the size and settles back (width 0.44 s /
  bounce 0.25, height 0.38 s / 0.22, never more than 12 px past), shrinking
  settles without a swing (0.30 s / 0.26 s). A pill's content climbs in
  from 0.86 of its size around the box's top middle and leaves the same way.
- **Changing text** goes through `motion::text` with a key that includes
  the sheet's instance (its opening time, an OSD's entrance) — a label
  seen for the first time just appears. Digits (and a time's `:`/`.`)
  that changed roll: up as a number grows, down as it shrinks (a
  countdown); the rest of the label stays and its tail slides to the new
  width. Changed words lift away as the new word rises in. The compact
  clock's width eases with its digits.
- **Presses**: a pressed tile or button dips about 5 % and springs back
  past rest (`press_scale`).
- **Symbols**: a toggled or changed symbol bounces (`bounce_scale`: Do
  Not Disturb's moon, the keyboard on a layout switch); a control asking
  for confirmation wiggles (`wiggle_angle`).
- **Sliders** swell slightly while held.
- **Paging**: content that pages sideways (the calendar's months)
  slides about 44 px the way it goes while the old page fades out first;
  the sheet's height eases to the new content.
- **Arrival**: the Control Center's rows arrive in a wave, each a beat
  after the one above, rising a few points as they fade in.

## Copy

Interface text is Russian; product names (Wi-Fi, Bluetooth, Touch ID) and
the clock format stay as they are. Sheets carry no instruction footers
("Esc — закрыть"): keyboard behaviour is documented in the README, and a
status line appears only while there is a status or an error.

## Rules

Within one role use the same geometry, colour, type and motion everywhere.
Drawing and hit testing must move together; preserve hit targets and
keyboard behaviour while restyling. Check changes with the gallery:
`ISLAND_GALLERY=1 cargo test gallery -- --ignored` renders every surface at
1.5× into `target/gallery/`.
