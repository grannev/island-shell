# island-shell

A Dynamic-Island-style notch for SwayFX: clock, now playing with a live
equaliser, Touch ID, notifications, a launcher — one Wayland layer-shell
client rendering everything itself at the display's refresh rate.

Replaced the waybar-based island (island-*.sh, 500 lines of CSS frames)
and the walker launcher; see `CLAUDE.md` for the brief and the plan.
Every sheet is drawn with one Apple-style design system (`src/menu_style.rs`:
system colours, SF Pro text styles, capsule buttons, grouped lists, a
circular close control); its roles are documented in
[`docs/interface-style.md`](docs/interface-style.md). The gallery test renders
every surface at 1.5× for review:
`ISLAND_GALLERY=1 cargo test gallery -- --ignored` → `target/gallery/`.

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

    island-shell mic muted | mic on
    island-shell volume 0.65
    island-shell brightness 0.8
    island-shell battery 72 charging
    island-shell battery 10 low
    island-shell bluetooth 84 Headphones
    island-shell wifi My network

**Night mode.** The Control Center's «Ночь» tile (or `mod+n`,
which runs `island-shell night`) tints the screen warm with `gammastep -O
<night_temp>` (4500 K by default; `set night_temp 3800`). The tile's well warms to
orange and its sunset symbol bounces. From the keyboard a short pill says
«Включён»/«Выключен». gammastep runs in its own systemd scope, so the tint
survives an island restart; switching off kills it. `island-shell night
on|off|toggle|status` works from scripts too.

**Screen recording.** The «Запись» tile in the Control Center (or
`island-shell record`) counts down 3-2-1 in a pill: a red ring empties and the
digit rolls down. `Escape` cancels. Once the pill has folded away,
`wf-recorder` records the island's output. A red dot and the recording's
length sit beside the clock; a click on them (or «Стоп», or `island-shell
record` again) stops it. The recorder gets SIGINT so the file is finished
properly. The video goes to `~/Videos/Screencasts/<date>.mp4`; its first
frame (made with `ffmpeg`) flies into the island with the length and the file
name. `wf-recorder` needs `sudo sh scripts/03-install-screen-recorder.sh`;
until then the button is dimmed.
Without `wf-recorder` the tile says so in a pill with the command that
installs it (`sudo sh scripts/03-install-screen-recorder.sh`); each press looks
again, so a freshly installed recorder works without restarting the island.

**Microphone.** While any application records from a microphone, an orange
privacy dot sits at the right end of the compact island. It swells in and
shrinks away and stays for as long as the recording runs, as in iOS. Speaker
listeners such as cava and visualisers, level meters and paused streams do
not count. Muting or unmuting the default microphone (the mic-mute key)
shows a short pill: the microphone symbol bounces, struck through on red
when muted. With the Control Center open, its own microphone button shows
the change instead.

**Jobs** from anywhere become live activities. A running job is a small
ring beside the clock: it fills with the job's percentage, or an arc turns
beside the time it has been running when the job does not say how far it
is. When other jobs are running too, a «+N» follows the value. A click on the
ring unfolds every job into a pill: a ring well, the title, its status line,
the percentage and a progress bar. Click the pill to fold it again;
it stays open while the pointer rests on it. A job that ends turns its ring
into a green tick («Готово») or a red cross («Ошибка»), which stays for four
seconds. A job that ran for three seconds or more also unfolds the pill
with its outcome and duration, even over a fullscreen window:

    island-shell run -- cargo build --release   # the command, as a job
    island-shell run --title "Бэкап" -- rsync -a ~/docs /mnt/backup
    island-shell progress "Копирование" 40%     # start/update; 100% ends it
    island-shell task start build "Сборка"      # explicit control by ID
    island-shell task set build 60% linking     # percentage and/or detail
    island-shell task done build | task fail build disk full
    island-shell task cancel build | task list

In bash, every command can be a job without `run`: `scripts/install.sh`
installs `shell/island.bash`, and a line in `~/.bashrc` sources it. A
command still running after 10 seconds (`ISLAND_JOB_AFTER`) shows its ring,
titled with the command, and ends with a tick or with «код N»/«прервано».
Quick commands never appear. The hook only tells the island when a command
starts and ends; it starts no background process. Editors, pagers, `ssh`,
`tmux` and other interactive programs (`ISLAND_JOB_IGNORE`) are left out.

`run` passes the command's input, output and exit status through untouched.
A failure shows «код N», or «прервано» when the command was killed by a
signal; without a running island the command simply runs. Every job belongs to
the process that started it: the calling shell or script for `task` and
`progress`, the `run` wrapper itself for `run`. When that process exits without
ending the job, the job ends as «прервано». Pass `--pid=<pid>` right
after `task`/`progress` to name another owner; `--pid=0` detaches the job.

A right click on the clock unfolds the **calendar**. It shows the month,
weeks starting on Monday, with weekends dimmer and today on a red disc.
‹ › or the wheel turn the months; the grid slides the way the months go,
and the sheet eases to five or six weeks. A click selects a day (a white
disc). The footer names the selected day and its distance from today
(«Вторник, 13 октября · через 8 дней»); «Сегодня» brings you back. With
the keyboard: arrows move by a day or a week, crossing into the
neighbouring month; `PageUp`/`PageDown` turn the month; `Home` goes to
today; `Escape` closes. `island-shell calendar` opens it too.
Under the month sits the **weather**, from open-meteo.com (no key). On the
left is now: the sky's symbol, the temperature, the sky and the place,
today's high and low. On the right are the next four days, with the selected
day's column lit. The footer adds the selected day's high/low when it is
forecast. The place is the config's `weather`: `auto` (default) takes the
city of the system time zone (Europe/Samara → Самара) and finds it with
open-meteo's geocoder, so no IP lookup is made. It can also be a city
(`set weather Казань`), `lat,lon`, or `off`. `curl` fetches the forecast on a
background thread at most every 30 minutes and caches it in
`~/.cache/island-shell/weather.json`.

Click the ordinary compact island to open the **Control Center**, laid out
as Vorssaint's open island (560 px): the title «Управление» with a faint ⋯
at the end of its row that turns into the collapse chevron under the
pointer; a row of cards — while something plays, a **Now playing** card
with the art, the title over the artist and previous / play-pause / next
(first in the keyboard order; a new track's title rises in) beside one slim
card holding the Sound and Display levels, otherwise a Sound card and a
Display card of their own. Press anywhere on a slider to jump there, drag to
adjust; moving the volume unmutes. The Sound level's output name opens the
**Sound** page (a chevron back to the main page beside its title): the audio outputs
(unplugged HDMI ports hidden; a click switches the default sink), every
application playing with its own level slider (all of an application's
streams move together; notification chimes are left out), and the default
microphone with its level and a mute button on its symbol (struck through
and red while muted); the page re-reads the applications once a second
while it is open. Beside every open page — the Control Center, the sound
page, the notification center, the calendar and the timer — stand
Vorssaint's **floating buttons**: 44 px black circles that drip out of the
island's sides once the page has most of its size (a small circle tucked
under the edge swells as it moves out, its neck thins and lets go) and fall
back in quickly when the page closes. Left: Sections, Search and Timer;
right: Notifications, Sound and Calendar. The page showing is lit; a click on
another button morphs the island into that page. **Sections** (the grid
button, or `island-shell sections`) is Vorssaint's gallery of destinations:
ten tiles (Control Center, Sound, Calendar, Notifications, Timer, Wi-Fi,
Bluetooth, Search, Files and Clipboard — the last two open the launcher with
`file:` / `clip:` typed), the calendar's symbol red and the timer's orange,
the timer and notifications with their state under the name; the arrow keys
walk them and Enter opens one. Under the cards, a rail of thirteen round shortcut tiles, five to a
row with the short last row centred
(a 40 px well over a one-line label, a light wash and a small lift under
the pointer; a tile that is on takes its colour): Wi-Fi, Bluetooth, Timer
(its remaining time in orange while it runs), Notifications (the unread
count, red), Do Not Disturb (purple), Night mode (orange), Search (the
launcher), Screenshot (the menu folds away first, then
`~/.config/sway/scripts/screenshot.sh` — or the island's `island-screenshot` —
saves the whole screen, copies it and flies the thumbnail in), Record, the
keyboard layout (click for the next one), Lock (`loginctl
lock-session`), Sleep (`systemctl suspend`) and Log out (sway `exit`,
only on a second press within three seconds — the button turns red and
asks «Точно?»). While the menu is open, volume and brightness keys move its
sliders instead of raising an OSD. A worker thread (`src/control.rs`) does
all reading and writing — `wpctl`, `pactl`, the backlight in sysfs (the
video group may write it), sway IPC — and coalesces slider drags to the
latest value. Arrow keys and Enter walk
every control on the page; Escape, the collapse chevron or a click outside closes the menu. Changing text animates the
Apple way (`src/motion.rs`): digits roll — the clock's minutes, the
countdown, percentages, counts — and changed words lift away as the new
one rises in («Вкл.» ↔ «Выкл.», the layout's name). Pressed controls dip
and spring back, toggled symbols bounce, the logout button wiggles while
it waits for confirmation, held sliders swell, and the menu's rows arrive
in a wave.

The timer page is Vorssaint's: three modes in the title row (Timer, Pomodoro,
Stopwatch — the chosen one underlined in orange, the line sliding between
them) beside an orange Start button, and under them a ruler of orange ticks,
one per minute up to three hours, labelled every five. Drag it like a tape
(left brings later minutes under the pointer), turn it with the wheel (a
minute a notch) or two fingers, click along it to jump, or use the arrow
keys; Tab changes the mode and Return starts. Each minute passing under the
pointer gives a soft click (Vorssaint's haptic tick; this ThinkPad's Sensel
touchpad exposes no haptics to the system, so the click is a sound) and the
pointer nods. The Pomodoro adds its plan under the ruler — break, long break,
long break after, sessions — clicked through presets or turned with the
wheel. A running session names its phase in the title row and shows iOS's
round Cancel and Pause buttons beside the large reading (red in a countdown's
last minute). A Pomodoro phase that ends rings, says what follows and waits:
its page offers the next phase's start. While running, a compact ring and the
reading sit beside the clock (the stopwatch's ring sweeps each minute).
Click that indicator to pause or resume, or right-click to reopen the page.
Escape closes the page and × returns to the quick menu, neither stopping the
session. A countdown that ends rings (`alarm-clock-elapsed`) and expands into
a short “Time is up” activity. `timer_sound = off` silences the ticks and the
alarm. The session and the page's choices are kept in memory and reset when
the service restarts.

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
temporary files. The Saved tab also lists saved networks that are out of
range. Under the pointer a saved network shows a red bin beside its action:
the first press arms it (it fills red, shakes its head and the status line
asks again), a second press within four seconds deletes every
NetworkManager profile of that network, password included.

Bluetooth has My devices / Discover tabs. Click a paired device to connect
or disconnect; discovery finds new devices and offers pairing. PIN entry,
passkey display and confirmation are handled in the island by an app-scoped
[BlueZ agent](https://bluez.readthedocs.io/en/latest/agent-api/). Discovery runs
for 30 seconds and can be stopped; closing the panel releases its scan session
and cancels pending pairing. Battery levels appear when supported. A paired
device's red bin unpairs it the same way (BlueZ `RemoveDevice`, which also
drops its keys and disconnects it).
Friendly aliases and remote names are preferred over address placeholders;
named devices sort first. Names update during discovery. Devices that do not
advertise a name are explicitly labelled as unnamed with their address below,
not assigned a guessed product name.

Both sheets show operation progress and errors, scroll through the complete
list, and refresh state while open. Worker replies are scoped to a panel and
request so late results cannot reopen a closed sheet. Buttons, tabs, list
positions, scroll and panel size animate; interactive sheets keep focus and
queue transient OSDs until they close. The timer page's controls fade in and
out under the pointer, and Start / Pause / Cancel leave the page open.

`island-shell timer` opens the same page. CLI controls also remain:
`island-shell timer 25m` (or `1h30m`), `timer pomodoro` (a cycle with the
page's plan), `timer stopwatch`, `timer next` (the waiting Pomodoro phase),
`timer pause`, `timer resume`, `timer toggle`, `timer status` and
`timer cancel`.

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
top and bottom of the viewport. The wheel glides rather than steps: each
notch pushes the list about a row and eases out, quick notches merge into
one motion, and a touchpad moves it 1:1. The selection is a light card,
like a hovered notification.

A left click on the ordinary compact body opens the four-button menu immediately.
Left, right and middle clicks over the compact equaliser (or the cover, in the
wings without the clock) or the open music panel
pause/resume, skip forward and skip back respectively. The wheel changes tracks
only over that music area. Timer controls, seek dragging, authentication,
notifications and other modal sheets keep their own interactions.
The music panel opens only from the equaliser on the right of the compact
island (and from the cover in the wings); Neovim's details open only from its chip on the left. Once a panel
opens, moving the pointer across it keeps it open until the pointer leaves.
Hovering the clock or another ordinary part gently widens the compact island,
including when music or Neovim is active; the transition into either detailed
panel continues from the current animated size.
The closed island can be tugged with the mouse or a finger: it gives against
a tight rubber band (at most ~34 px down, ~26 px sideways, a squash up), leans
with its top lagging, stretches like a drop with its fillets drawn out, and
jumps back like jelly when let go. A press that moves less than 6 px is still
a click, decided on release.
Under niri the bar's left drop (the workspaces) is the overview's handle:
pulled down with a finger or the mouse it gives further than the right one,
the overview's four tiles come up beneath the workspaces, bounce as the pull
passes ~65 px, and letting go there opens niri's overview (the tiles zoom
away); a shorter pull just jumps back.

**Tablet mode under niri** (`~/.config/niri/config.kdl`, `switch-events`):
folding the screen back starts `~/.config/niri/scripts/tablet-mode.sh on` —
the screen follows the accelerometer (`monitor-sensor`, niri's transform)
and lisgd turns touchscreen swipes into niri actions (one finger in from the
left / right edge: that column; up from the bottom edge: the overview; three
fingers sideways: columns, up / down: workspaces); folding it back stops
both and stands the screen upright. The on-screen keyboard is not shown on
its own. The touchscreen and the pen are mapped to eDP-1. Root step:
`sudo sh scripts/04-install-tablet-tools.sh` (lisgd, libinput-tools, and a
udev rule that lets the logged-in user read the touchscreen only).

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
**Answers.** As in Spotlight, a calculation or a conversion becomes the
launcher's first row (an orange calculator tile), and `Enter` copies the
number with `wl-copy`. The number uses a dot and no grouping, so it pastes
into code. Arithmetic supports brackets, `^`/`**`, `!` and `mod`, functions
such as `sqrt 2`, `sin`, `ln`, `log`, `round`, and the constants `pi` and
`e`; `2400 + 15%` gives 2760 and `15% от 2400` gives 360. Units convert with
`в`/`to`/`in`/`->`: length, mass, temperature, data (`GB` decimal, `GiB`
binary), time, volume, speed and area, in Russian and English names
(`5 миль в км`, `100 f to c`, `90 км/ч в м/с`). Currencies (`100 usd в rub`,
`$50 в евро`, any ISO code) use the open.er-api.com rates. They are fetched
with `curl` on a background thread when the launcher opens, at most every 12
hours, and cached in `~/.cache/island-shell/rates.json`; the row says the
date of the rate. A bare number or a word asks nothing; a time such as `12:30`
is not a division.

**Clipboard history.** `clip:` (or `буфер:`) lists the clipboard's history
from `cliphist`, newest first; text after the prefix filters it. Images show
a thumbnail, and `Enter` puts the entry back on the clipboard. `cliphist` needs
`sudo sh scripts/02-install-clipboard-history.sh`. The sway config feeds it
with `wl-paste --watch cliphist store` once it is installed. Until then,
`clip:` says how to install it.

The launcher selection stops at the first and last result. Arrow keys and the
mouse wheel share a resisted rubber-band pull at the list edges instead of
wrapping or snapping. `Ctrl+Backspace` deletes the previous word in its
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
shortcut tile switches in place without closing the menu; the title stays
"Тишина", its status line reads Вкл./Выкл. and the round icon eases into
purple.
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

Notifications can be worked with in place:

- **Progress.** A notification with the standard `value` hint (0–100, as
  sent by downloads and copy jobs) fills the ring around its icon on a
  spring and shows a progress bar under the message; updates with the same
  ID move the bar instead of restarting the pill. Its card in the center
  keeps the bar while the job runs.
- **Swipe to dismiss.** Two fingers sideways on a touchpad, or a held mouse
  drag, move the pill or a card in the center with the pointer. Past a third
  of the width, or thrown quickly, it flies out that way and is closed (the
  app receives the close signal); otherwise it springs back. A press that
  does not move is an ordinary click.
- **Reply.** The server advertises KDE's `inline-reply` capability. For an
  app that offers it (the `inline-reply` action and an optional
  `x-kde-reply-placeholder-text` hint), the pill's «Ответить» opens an
  iMessage-style field under the message: it takes the keyboard (also over a
  fullscreen window), and the pill stays while you type. `Enter` or the
  round send button emits `NotificationReplied` and closes the
  notification, `Escape` cancels. A live notification opened in the center
  shows the same field above its buttons; typing goes straight into it, the
  sent text lifts away and «Ответ отправлен» takes its place, and `Escape`
  clears typed text before it closes the sheet. Apps read the capability
  when they connect, so restart an already running app (Telegram Desktop)
  after first installing this version.
The same temporary reveal applies to a now-playing pill when playback starts
in fullscreen; it is fully placed on-screen without exposing the compact bar.

Step 2: now playing (MPRIS over D-Bus, zbus). A track that starts
announces itself with a "now playing" toast — album art, the app, the
title, the bars — which then collapses to four cava-fed equaliser bars
beating beside the clock; hovering those bars opens it into
a player with the art, the title and artist, a live equaliser, a progress
line you can click or drag to scrub — it swells under the pointer and
again in the hand, on springs — and vector transport controls that light
up as the pointer reaches them. A quiet × beside them (and on the Control
Center's "Now playing" card) clears the player: it pauses and folds off the
island, the app keeps running, and the same player comes back as soon as it
plays again. Silent playback
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

    scripts/install.sh        # cargo build --release → ~/.local/bin/island-shell,
                              # then restarts a running island-shell.service

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
    island-shell sections                       # the sections gallery
    island-shell launcher                       # mod+d: unfold / fold the panel
    island-shell notifications                  # open the notification history
    island-shell dnd on|off|toggle|status       # quiet ordinary notifications
    island-shell type <text> | key <name>       # drive the launcher from a script
    island-shell scrub <0..1> | scrub end       # drag the playhead from a script
    island-shell player-clear                   # pause and hide the player until it plays again
    island-shell lock | sleep                   # the screen switches off into the island, then locks / suspends
    island-shell curtain                        # the same switch-off and back, without locking (a demo)
    island-shell wake                           # let the screen out of the island, should it stay off
    island-shell timer 25m | timer pause        # compact countdown
    island-shell screenshot <file.png>          # animate a finished grim image
    island-shell calendar                       # the month (right click on the clock)
    island-shell notes                          # the notes page
    island-shell shelf | downloads              # the shelf, on either tab
    island-shell record                         # 3-2-1, record the screen; again stops
    island-shell night [on|off|toggle|status]   # night mode (gammastep)
    island-shell compact [<item> on|off|toggle] # what the closed island shows (saved)
    island-shell run [--title T] -- <command>   # a command as a live job
    island-shell progress <title> <n>%          # a job by its percentage
    island-shell task start|set|done|fail|cancel <id> … | task list
    island-shell status | quit

### Configuration

`~/.config/island-shell/config`, `key = value` lines (`#` starts a comment
at the beginning of a line or as a separate word), read at start;
`island-shell set key value` changes a value in the running island and
`island-shell status` shows the current ones:

    smoothing = 0.6       # corner smoothing 0..1 (0 = circular arcs)
    liquid = 1            # velocity stretch, 0 = rigid
    hover = 1             # widen the box under the pointer
    output = auto         # auto | focused | a connector name (eDP-1, HDMI-A-1…)
    clock = %a %b %-d  %-I:%M %p   # strftime(3); seconds tick every second
    accent = #66D9FF      # equaliser, progress line, "now playing"
    notice_hold = 3.5     # seconds a notification stays up
    toast_hold = 4        # seconds the "now playing" toast stays up
    silent_after = 15     # seconds of pause / mute before the bars fold away
    dodge = 240           # px the island moves left for the ⋯ drawer
    night_temp = 4500     # night mode's colour temperature, K
    weather = auto        # the calendar's weather: auto, a city, lat,lon or off
    compact = all         # what the closed island shows: all, none, or a list of
                          # clock editor timer tasks record music unread dnd mic mascot
    timer_sound = on      # the timer ruler's ticks and the alarm
    screen_corners = 8    # radius of the screen's rounded corners, px (0 = square)
    edge_light = on       # light along the screen's edges (notifications, Touch ID)
    ambilight = off       # music lights the bottom edge in the cover's colour
    bar = on              # Waybar's row in the island's corner drops (off: run Waybar again)

Without `clock` the closed island rests as a short black capsule (150 px), and
a right click on any free part of it opens the calendar. Its activities then
part into wings, as on Vorssaint's island: the song's cover and the timer's
ring at the left end, the bars and the orange reading at the right, and the
song's title in the middle where the clock was (the shorter wing evens up a
little so the title stays centred). The cover answers like the equaliser, and
the ring like the timer's reading. `island-shell compact
clock off` (or `on`, `toggle`) changes one item in the running island and writes
the new list to the config file, so it survives a restart.

**Notes** (mod+n, the sections gallery's «Заметки», `island-shell notes`), after
Vorssaint's scratchpad: the top row holds the pads as tabs, titled by
their first line, with «+» for a new one; the card below is the chosen
pad's text, ready for typing. Each pad is a Markdown file in
`~/Documents/notes/` (named by when it was made), saved 0.6 s after the
last change and whenever the page closes (written to a temporary file and
renamed). Keys: arrows (Ctrl by word, Shift selecting), Home / End,
Backspace / Delete (Ctrl by word), Enter, Tab, Ctrl+A / C / X / V (wl-copy
/ wl-paste), Ctrl+T or Ctrl+N a new pad, Ctrl+Tab / Ctrl+PageDown the next
pad, Ctrl+Shift+Tab / Ctrl+PageUp the previous, Escape closes. The mouse
places the caret and drags a selection; the wheel scrolls. A pad emptied
and left goes away; the bin on the hovered chosen tab sends a pad to the
trash (`gio trash`) on a second press.

**The shelf** (after Vorssaint's file tray): drag files from any
application onto the island and it unfolds into the shelf with its drop
zone lit; letting go puts them there (as references — nothing is copied;
the drop is a copy action, never a move). A shelf a drag unfolded folds
again when the drag leaves, or a moment after the drop. Its tiles show
the file's icon (pictures as thumbnails) and name; a click opens the file,
a tile dragged out goes to wherever it is dropped (the shelf folds out of
the way, the file's icon rides under the pointer), and the × on a hovered
tile takes it off. «Сжать» packs everything into `~/Downloads/Полка
<time>.zip` (a job ring while it works; the archive joins the shelf),
«Пути» copies the paths, the bin empties it on a second press. The list
is kept in `~/.local/state/island-shell/shelf`. Open it by hand from the
sections gallery («Полка»), mod+s or `island-shell shelf`.

**Downloads**: the shelf's second tab («Загрузки», also in the gallery and
`island-shell downloads`) shows the newest files in `~/Downloads`, running
downloads first with a turning ring. A download in progress — Firefox's
`.part` or Chromium's `.crdownload` beside the file it becomes, watched
with inotify — is also a job ring beside the clock with its size and
speed («734 МБ · 4,2 МБ/с»; browsers do not tell the total), ending in a
tick («Загружено») when the partial becomes its file, or a cross when it
vanishes without one.

**The companion** (`mascot`, after Vorssaint's): a small coral capsule with
two soft black eyes at the left end of the closed line. It blinks now and
then, glances about, follows the pointer over the island, bobs to the
music's bass, and answers what the island sees — a startled hop for a
notification or the microphone opening, smiling eyes for a Touch ID match,
music starting, power or a Bluetooth device, a head shake for a failed
match, a critical notification or a low battery, two hops when a timer runs
out, a hard blink for a screenshot, a nod when a timer starts. A reaction to
something that opened a pill plays once the line is back. After two minutes
without input (ext-idle-notify) it falls asleep, a "z" drifting up for a
while; it wakes with a stretch. A click pokes it. The Control Center's
«Друг» tile (or `island-shell compact mascot toggle`) puts it away or brings
it back, and the choice is saved.

Monitors: `auto` puts the island on the laptop's own panel (eDP/LVDS/DSI),
else on the first output; `focused` follows sway's focused output; a
connector name pins it there and falls back to `auto` while that output is
unplugged. Hotplug, disabling an output and scale changes move or re-render
the island without restarting it — everything it is showing carries on.
Fullscreen auto-hide looks at the workspace visible on the island's own
output, so a fullscreen video on the other monitor leaves it alone.
Animation timings are not configurable: they are tuned together.

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
    src/tasks.rs    jobs from task/progress/run: the ring, the pill, owners
    src/calendar.rs the month sheet: dates, grid, selection, month motion
    src/calc.rs     the launcher's answers: arithmetic, units, currencies
    src/clipboard.rs  wl-copy and the cliphist history for "clip:"
    src/recorder.rs wf-recorder: start, stop (SIGINT), the first frame
    src/weather.rs  open-meteo forecast for the calendar, sky symbols
    nvim/           the companion plugin (lua/island.lua), a lazy.nvim `dir`
    scripts/        install.sh, island-cmd.sh (the sway bindings' entry)
    shell/          island.bash: long shell commands as live activities
    systemd/        the user service
    examples/icons.rs  renders the icon set to a PNG for eyeballing
    examples/apps.rs   lists what the launcher would show, with icon sizes
