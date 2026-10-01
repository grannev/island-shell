# Island interface style

The island has a small visual vocabulary, not a ban on new controls. Match the
role of an element before matching its appearance. Within one role, use the
same geometry, surface, border, type treatment and motion in the quick menu
and every sheet opened from it.

## Shared roles

- **Action buttons:** use `menu_style::button`. Neutral is the default;
  primary marks the main action, warm is reserved for the timer's chosen
  duration/resume action, and destructive marks removal. The role determines
  colour, not the window that happens to contain the button. Use a 1 px soft
  border, 10 px rounded-rectangle corners, centred ellipsized label, and eased
  hover. Disabled controls keep their shape and reduce opacity.
- **Close buttons:** use `menu_style::close` in every menu and sheet. Keep a
  rounded-square surface with the same 10 px corners as action buttons,
  identical cross strokes and hover response. Place the
  drawing inside the existing hit rectangle. The meaning of close (dismiss
  or return to the quick menu) comes from navigation context, not a new icon.
- **Navigation tiles:** the quick menu's large icon-and-label cards form a
  separate family. All tiles share size, layout, radius and hover motion;
  semantic icon colours and the active Quiet state may differ.
- **Segmented controls, switches and list-row actions:** these are not action
  buttons. Keep each family internally consistent across Wi-Fi, Bluetooth
  and future sheets. State changes should ease rather than snap.

## Alignment

Wi-Fi, Bluetooth and notification sheets use a 22 px outer content rail:
headers, tabs, rows, footers and full-width actions start there and end
22 px before the opposite edge. Header controls share a centreline; adjacent
controls keep a visible gap. Inset content inside a card uses its own
consistent padding (18 px in connection prompts). Drawing and hit testing
must be updated together when a control moves.

New kinds of controls are welcome when their interaction differs (for
example, a slider, countdown ring or swipeable row). Give the new family a
clear purpose and apply its style consistently wherever that same purpose
recurs. Avoid one-off geometry or colour changes between otherwise identical
controls. Prefer editing a shared drawing helper over patching a single
window; preserve hit targets and keyboard behaviour while restyling.
