# Aurora Viewer branding and interface

The Aurora interface is **clean, modern and slightly glassy**: dark, calm
surfaces, a single strong accent colour (violet), readable text, crisp and
consistent icons. Every screen must look polished and finished.

## Palette

The colours come from `crates/aurora-viewer/src/theme.rs` (`Theme::default`).
In the code, always use the resolved `Palette` (`p.violet`, `p.ink`…),
**never a hard-coded colour**: a skin can change them, and so can the user,
from Preferences › Appearance › Skin › Interface colours
(`interface_colors.rs`): they pick the accent (`violet`), the windows'
background (`panel`) and the text (`ink`), and the other accent, surface
and text roles follow from those three. So never pick a role for its
violet: a teal or pink accent, or a light background, must still read.
On a chosen background the states and `teal` move in lightness to read
4.5:1, their hue kept; a configured chat colour goes through
`ui::colors::text` for the same reason.

### Brand colours

| Name | Hex | Use |
|---|---|---|
| violet | `#8B5CF6` | main accent: selection, active button, focus, links |
| indigo | `#4F46E5` | secondary accent, brand gradients (logo, login background) |
| teal | `#5EEAD4` | small touch: "active / online" states, positive attention points; clickable links in texts (web, places, avatars, groups in the chat, the bubbles and the profiles) |
| navy | `#070B1F` | deep background (login screen, auroras) |
| violet_light | `#A78BFA` | violet on a dark background (accent text, hover) |
| violet_pale | `#C4B5FD` | very light violet (discreet details) |
| indigo_light | `#818CF8` | light variant of indigo |

### Interface neutrals

| Name | Hex | Use |
|---|---|---|
| panel | `#252830` | window background (opacity `panel_alpha` = 230) |
| bar | `#1B1E26` | top and bottom bars |
| field | `#181B23` | input fields, sunken areas |
| raised | `#363B47` | buttons, hovered surfaces |
| ink | `#E8EDFB` | main text |
| muted | `#9AA6C6` | secondary text, labels |
| muted_dim | `#6F7BA0` | tertiary text, disabled |
| on_accent | `#FFFFFF` | text and icons placed on a violet face: main button, active tool, tick of a checkbox, dot of a radio button, knob of a switch that is on |

### States

| Name | Hex | Use |
|---|---|---|
| success | `#4ADE80` | success, validated |
| warn | `#FB923C` | warning |
| danger | `#F87171` | error, destructive action |
| amber | `#FCD34D` | soft highlight (favourites, ratings) |
| rose | `#F472B6` | rare: categories, playful accents |

The build axes take three of these colours, wherever an axis is shown (edit
gizmo, X / Y / Z boxes of "Transform", value readout): X `danger`,
Y `success`, Z `indigo_light` (`build::draw::axis_color`). Hovering and
dragging a handle switch to `violet_light`, like any selection.

## Design rules

- **The look comes from the theme.** A window writes no colour, no radius
  and no stroke: it takes the roles of the `Palette`, the radii of the theme
  and the shared widgets of `ui/widgets.rs`. What a window draws with its
  own values is content, not dressing: colours of a map and of its legend,
  maturity rating colours, inside of a colour swatch, image shown as it is
  (`widgets::PICTURE_TINT`). A test checks it (`ui/theme_guard.rs`, run by
  `check.ps1`); a content line is declared there with a comment
  `// theme-guard: content — the reason`.
- **Discreet glass.** A shared GPU blur appears in the windows, the
  dialogs, the top and bottom bars and everything laid over the world
  (notifications, volume, emoji, media controls, local chat bubbles, pie
  menu), under a dark tint. A soft shadow and a thin border separate the
  surface from the world; the fields stay calm and readable. A skin with
  `panel_alpha = 255` keeps opaque surfaces without this blur. The high
  contrast palette of Preferences › Accessibility (`high_contrast.rs`) is
  opaque too, and gives buttons, fields, windows, islands and dividers a
  light border: a new border goes through `widgets::border_stroke` so that
  it follows. Outside a
  window, a surface is drawn with `widgets::overlay`. An already positioned
  scene label uses `widgets::paint_world_label`, with a lighter tint and
  shaded scene blur to keep its text readable. With no world behind, as on
  the login screen, use `widgets::overlay_frame`. Never write a background,
  border or radius by hand. The bars
  keep the `bar` colour as their tint (`widgets::finish_bar_glass`); the
  minimap keeps the Legacy Viewer's opacity with the border and the shadow of the
  windows. Menus, submenus, right-click menus and drop-down lists get this
  glass from a single place (`menu::glaze`, at the end of the frame, in the
  world): a new menu has nothing to do to get it.
  Gradients are reserved for brand elements (login background, auroras,
  fill of the progress bar).
- **Preferences › Aurora.** The page of what Aurora does differently from
  the Legacy Viewer carries the brand in two places only: its entry in the
  category list (edge mark and name in the aurora gradient, from `success`
  through `teal`, `indigo_light` and `violet_light` to `rose`) and the head
  of the page (the aurora over the `navy` sky, a hairline of the same
  gradient along its bottom edge). The settings below keep the cards and
  controls of the other pages; the choices between Aurora's look and
  the Legacy Viewer's are a `widgets::segmented` track, Aurora's segment lit in the
  accent and the Legacy Viewer's in `raised`. It is not a precedent for the other
  pages (human's choice, 2026-10-14).
- **Login screen.** It is the viewer's showcase. The window blur only
  knows the 3D scene, which is absent here: each card (`login::card`)
  therefore redraws under its tint the sky it covers, out of focus
  (`aurora_bg::frosted`: no stars, aurora rays melted together, ridges
  softened), with two glows of the aurora (violet, turquoise) that spill
  around it. Its top edge catches the light; the login card carries the
  hairline with the brand gradient. The news sits next to the login card,
  on its row and at its height (the list scrolls), the version above it.
  The bottom buttons and the version number share the same line.
- **Progress bars.** `widgets::progress`: a flat track (that of the
  sliders) filled with the brand gradient, which slides to its value
  instead of jumping; the percentage written below slides with it. The
  loading and teleport banners slide in the same way. An instantaneous
  level (microphone) goes through `widgets::level`, which only smooths the
  jitter between two measurements. Never `egui::ProgressBar`.
- **Microphone button and panel.** The microphone stays in the centre of
  its button, like the neighbouring icons; the arrow that follows it points
  to the panel it opens above the bar. The panel (window glass) gives the
  state of the voice, the two ways of talking as cards (the current one lit
  like an active button of the bar and ticked, the key to hold drawn as a
  key), then the microphone and the audio output one under the other, and
  the access to the preferences.
- **Loading and teleport.** A "cinema banner" without a panel
  (`ui/loading.rs`): the last view, blurred, fills the whole screen, the
  name of the region is written large at the bottom left with what is known
  about it (maturity rating, arrival point, residents present) and the
  current step; at the bottom right, the tile of the region on the world map
  with the arrival point, and the progress in figures; the progress itself
  is a luminous hairline with the brand gradient along the whole bottom
  edge. The brand (wolf and name) stays discreet at the top left.
- **Discreet rounding.** Flat design, almost square corners. Buttons:
  radius 5; windows, cards and pills of the toolbar: radius 3, with corners
  close to those of the build tools, including when collapsed.
  The radii come from the theme, never from a number in the code:
  `ui.visuals().window_corner_radius` for cards, pills, list highlights and
  image or colour frames,
  `ui.visuals().widgets.inactive.corner_radius` for buttons,
  `widgets::inner_radius` (one pixel less than the windows) for a small
  mark inside a control or a row: knob of a switch, key, status square,
  maturity rating letter. Left as they are: the deliberate right angles
  (radius 0), the circles (status dots, notification dot) and the thin bars
  (track of a progress bar or of a level, marker on a rail), whose 2 px
  ends come from a single constant, `widgets::TRACK_RADIUS`.
- **Name plate.** The tag above an avatar (`ui/nameplate.rs`) is the
  **only exception** to the almost square corners: a glass plate with fully
  rounded ends. It is not a precedent: nothing else in the interface is
  rounder, but for the residents' pictures (below).
- **Residents' pictures.** The profile pictures of the residents are round:
  on the name plates, and in the Conversations window (bubbles, head of a
  conversation, list), drawn by `widgets::avatar_disc`, with their status
  dot on the edge (`ui/presence.rs`). Only these pictures and the name
  plates are round; groups, places and objects keep square tiles. The
  bubbles of the conversations themselves keep the buttons' radius, and
  the user's own bubbles carry the accent gradient (violet to indigo), the
  one gradient outside the brand elements, chosen by the human
  (2026-10-14).
  - *Glass*: the tint of the plate (halfway between `navy` and `bar`, at
    52%) over the window blur, a light one-pixel border (`ink` at 14%), a
    soft shadow below. The blur goes to the 16 nearest plates; the others,
    and the distant pills, only have the tint, denser. An opaque skin
    (`panel_alpha = 255`) has no blur.
  - *Three levels depending on the distance to the camera*: up to 15 m, the
    full plate (profile picture in a circle, status dot, display name in
    semi-bold, user name in `muted`, group title in `teal`, round badges
    floating on the top right edge); up to 40 m, a compact pill (name and
    badges on one line); beyond, the name alone. A plate only changes level
    1.5 m beyond a threshold and slides from one level to the other in
    0.18 s. In a crowd, only the eight nearest plates are full, the next
    ones are compact pills. A line hidden by the preferences is absent:
    never an empty line.
  - *Status*: dot at the bottom right of the picture, `success` (present),
    `amber` (away), `danger` (do not disturb); none for a blocked
    resident.
  - *Badges*: `success` microphone (in the voice channel), three animated
    dots (typing), `success` heart (friend), `amber` triangle (avatar too
    complex, shown as a silhouette), `danger` no-entry sign (blocked).
  - *Voice*: in the voice channel, the outline of the plate takes the
    `success` colour, at every level; while the person is speaking, a tight
    halo around the plate follows the level of their voice (fast rise,
    slower fall, no halo in silence).
  - *Name colour.* Every display name in `ink`, ours too; a blocked
    resident's in italics on a plate at 62%. Preferences › Colors ›
    "Color the names by relation" (off by default) colours the names of
    the plates and of the classic name tags alike, with the colours of that
    page (me, friends, blocked, Linden Lab, from the palette by default).
  - *Size*: the display name keeps the size of the classic name tags
    (13 points) on the full plate; everything else follows from it through
    the proportions of the mock-up.
- **Solid buttons.** All buttons and drop-down lists are solid faces
  without a visible border: the theme (`theme.rs`) gives their border the
  colour of the face, only input fields and checkboxes keep an outline. A
  button therefore sets neither a background nor a radius itself, except for
  the main action (violet) or a destructive one (`danger`). An action with
  an icon uses `widgets::action_button`. The bottom toolbar has no
  separator lines: the spacing and the pill of the hovered or active button
  are enough.
- **Switches and checkboxes.** The switch (`widgets::switch`) is almost
  square: track with the theme radius, square knob that slides. It is used
  for a standalone setting (alone with its label:
  `widgets::switch_labeled`); lists of flags (rights, data of a report,
  cells of a table) keep checkboxes, drawn by `widgets::checkbox` (flat
  square, violet and ticked when it is active), never by `ui.checkbox`.
- **Radio buttons.** A choice among several goes through `widgets::radio`
  (`radio_value` for a value), never through `ui.radio`: the row of the
  checkbox (same size, same label, same hover), but round so that it is not
  taken for a checkbox: thin ring at rest, violet around a dot when the
  choice is made. It is the only round control of the interface; the other
  circles are marks (status dots, notification dot).
- **Collapsible sections.** `widgets::collapsing`: a full-width row with a
  Tabler chevron and the title of the section, lit as a pill on hover,
  without a line; never `egui::CollapsingHeader`.
- **Colours.** A colour is chosen with `widgets::color_button`: the swatch
  opens a small glass panel (saturation / value field, hue bar, new colour
  above the old one, R G B and hexadecimal fields, eyedropper), never
  egui's `color_edit_button_*`. The colour palette of the build tools stays
  a separate window.
- **Scroll bars.** Thin and solid, in a strip of their own at the right of
  what scrolls, faint at rest: the theme sets them for every scroll area
  (`widgets::reserved_scroll_style`). Never egui's floating bar, which
  covers the switches, buttons and values of the right edge; a window
  does not set its own scroll style.
- **Shared hierarchy.** Window title and controls at the top, stable
  navigation, content grouped by function and accessible actions.
  The inventory keeps a large list and a single status line at the bottom,
  without a details sheet for the selection.
  Lists scroll within their area and do not make the window grow.
  The build tools use a compact inspector with a rail of sections.
- **Light navigation.** The tabs and navigation controls of the inventory
  are underlined in grey at rest, in violet when activated, without an
  outline. Hover stays discreet. The secondary actions of dialogs keep
  their dark surface. The command bar directly follows the title;
  the clear cross is built into the filter, without moving the controls.
- **A single accent colour per area.** Violet signals what is active or
  important; the rest is neutral.
- **Contrast and readability.** `ink` text on `panel`; secondary text in
  `muted`. Never dark violet text on a dark background (use
  `violet_light`). On a solid violet face, text and icons in `on_accent`
  (neither hard-coded white, nor `ink`, which a skin can darken); on a
  `danger` or `amber` face, which are light, the text stays in `navy`.
  Signed height sliders use `widgets::height_slider`: a visible neutral
  track, zero marker, violet fill from zero and a solid rectangular handle
  in the theme's radius. Secondary controls can be grey while retaining
  their interaction; the hover-height manual controls switch automatic
  mode off when edited. The surrounding shared floater keeps its glass.
- **Very subtle but visible separators.** A thin line of a neutral barely
  lighter than the background, so that one can see where each element starts
  and ends without weighing things down. Between two areas of a window:
  `widgets::divider` (horizontal in a column, vertical in a row); between
  two groups of entries of a menu: `menu::separator`; a line placed by hand
  (edge of a rail, boundary between two panes) takes
  `widgets::divider_stroke`. Never `ui.separator()`, and no line between
  buttons or the elements of a toolbar.
- **Typography.** Inter (regular, semi-bold) by default; Noto Sans,
  Roboto and OpenDyslexic (an accessibility choice for readers with
  dyslexia), each with its own semi-bold or bold face, as the other
  embedded choices of Preferences › Fonts. No other font without a reason. Every text
  follows the chosen font: semi-bold through `fonts::semibold`, monospace
  only for code-like text (script editor, UUIDs, keys, colour codes, URLs,
  paths, versions), and an arrow or a caret is a Tabler icon, never a
  character (`widgets::disclosure_caret`, `sort_caret`). The characters they lack
  (display names and messages in Georgian, Japanese, Arabic, mathematical
  letters…) are drawn with the system fonts, which are never chosen for a
  text of the interface itself.
- **Text size.** Preferences › Fonts and the skin's font scale apply to all
  font faces, including explicit rich text, painter fonts and monospace.
  Keep font sizes in unscaled points; shaping supplies the scaled advances
  and row heights. Size custom text containers from their laid-out text.
- **Regular spacing.** Reuse the existing widgets (`ui/widgets.rs`: flat
  buttons, switches, help badges, warning triangle…) rather than recreating
  them.
- **Interface texts are short and clear**, in English and French through
  `aurora_i18n::tr!` (see [TRANSLATING.md](TRANSLATING.md)); English is the
  fallback. A tooltip explains every option whose effect is not obvious.
- **Snapping windows.** A window moved near another one, a bar or an edge
  follows the pointer without a jump. Moving without Alt creates no snap
  or attachment. With Alt held, what will happen on release is shown
  by a guide: a 2 px line in `violet_light` along the edges that are going
  to touch (over the length they share) or to align, and the very light
  outline of the place where the window will land, with the window radius.
  No guide, no snapping. Snapped windows touch, edge to edge, with each
  other as with the bars and the edges, with no gap, like those of the
  first-launch layout (`layout::GAP`, at 0). During the gesture, a small
  pill on the window glass recalls Alt's role: the keycap (`widgets::keycap`)
  followed by "snap and attach" for a free move, "detach this window" for
  a grouped move, or "do not snap" for a snapped resize. Alt detaches a
  moved window from its group and enables attachment to a new target;
  resizing keeps automatic snapping, disabled by Alt. The pill uses `muted`
  text, placed near the window, away from the guide and
  the pointer, after a short delay. No button and no permanent decoration
  for snapped windows.

## Logos

Files in [`assets/branding/`](../assets/branding/):

| File | Use |
|---|---|
| `aurora-logo.svg` / `.png` (720 px) | full logo in colour, transparent background: README, window icon, login and loading screens, About, system notifications (`LOGO_PNG` of `app.rs`) |
| `aurora-logo-white.svg` / `.png` (4096 px) | white silhouette: the Aurora menu button of the toolbars (tinted with the palette), monochrome uses on a dark background |
| `aurora-logo-black.svg` / `.png` (4096 px) | black silhouette: monochrome uses on a light background |
| `aurora-logo-purple.png` / `aurora-logo-teal.png` (4096 px) | silhouette in violet or in turquoise |
| `aurora-logo-bg.png` (4096 px) | full logo on a navy background (the only file with an opaque background): square icons that need one (website's Apple touch icon) |
| `aurora.ico` | full logo as a Windows icon (16 to 256 px, taken from `aurora-logo.png`): icon of `aurora-viewer.exe` (embedded by its `build.rs`), "Aurora Tools" and "Aurora PR" shortcuts |

The logo is a howling wolf's head in profile, facing up and to the right,
its mane made of aurora ribbons, in a circle. Its colours: violet `#8B5CF6`
and indigo `#4F46E5` for the head, turquoise `#5EEAD4` and a deeper
turquoise `#56CAA9` for the ribbons. Do not distort it, recolour it or add
an effect to it; the one-colour versions above are the only recolourings.

## README buttons

The buttons of the README header and its "Soon" pill are small SVG files in
[`docs/readme/`](readme/), drawn like the viewer's own: flat face, radius of
4, a Tabler icon and Inter SemiBold.

| File | Use |
|---|---|
| `button-download.svg` | main action (violet face, `on_accent` text): latest release |
| `button-discord.svg` / `button-website.svg` | flat buttons (`raised` face, `ink` text): the Discord and website links of the login screen (`links.rs`) |
| `pill-soon.svg` | marks a planned feature in a list (teal face, navy text), 18 px high |

The text is live text, not outlines. Its `textLength` is the width of the
label in Inter SemiBold at the file's font size, so that the layout holds
with the fallback fonts of a machine without Inter. For another button, copy
a file, change the icon paths, the label and that width, then the width of
the `svg` and of its `rect` (18 + 20 + 9 + text + 18).

## Icons: Tabler

All the icons of the interface are [Tabler Icons](https://tabler.io/icons)
(MIT), drawn in white then tinted with the palette.
Do not mix them with another icon set.

- The full library (version 3.49.0) is in
  [`assets/tabler-icons/`](../assets/tabler-icons/): `outline/` (5,184
  outline icons) and `filled/` (1,054 filled icons). This is where an icon
  is chosen; the Tabler site lets you search by keyword.
- Only the icons in use are embedded in the viewer:
  `crates/aurora-viewer/assets/icons/tabler/outline/<name>.svg` and
  `…/tabler/filled/<name>.svg`.
- In the code, an icon bears its Tabler name: `eye-off` for the outline
  version, `eye-filled` for the filled version.

**The filled version first.** Tabler only offers a filled version for one
icon in five: we take it whenever it exists and expresses the function
well. The outline version is used in three cases:

- **linear glyphs**, which have no surface to fill: cross, plus, minus,
  tick, arrows, undo / redo, refresh, rotate, `</>`;
- **struck-through icons** (`eye-off`, `microphone-off`, `volume-off`,
  `message-off`, `ban` for blocking): Tabler only draws them as outlines;
- **functions with no suitable filled icon**: object (`box`), people in the
  plural (`users`, `users-group`, `user-plus`), loudspeaker (`volume`), map
  (`map`), hands (`hand-grab`, `hand-stop`), sitting (`armchair`), walking /
  running (`walk`, `run`)…

Do not force a filled icon that does not express the function. Two icons
that go as a pair (link / unlink, plus / minus) stay in the same style.
A function keeps the same icon everywhere (script = `code`, object = `box`,
outfit = `hanger-2-filled`, profile = `user-filled`, block = `ban`, a
friend's right to modify our objects = `pencil-filled`…): look for the icon
already in use before adding one.

**Adding an icon:**

1. Choose the name in `assets/tabler-icons/filled/`, otherwise in
   `assets/tabler-icons/outline/` (e.g. `microphone.svg`).
2. Copy it into `crates/aurora-viewer/assets/icons/tabler/filled/` (or
   `outline/`).
3. Add its name to the `FILLED` (or `OUTLINE`) list of
   `crates/aurora-viewer/src/ui/icons.rs`.
4. Use it with `icons.get("microphone-filled")` (or `icons.get("volume")`
   for an outline icon), tinted by the palette.

The icons of the parcel permissions (right end of the location field) are
coloured green / red; a bar drawn by the code strikes through those that
are forbidden. The icon in front of the web links of the chat and of the
profiles is `circle-check-filled` in `success` (trusted site),
`alert-triangle-filled` in `amber` (external link) or `circle-x-filled` in
`danger` (dangerous link), also in their warnings.

## Cursors

The world cursors are drawn from the same Tabler outline icons, in the
`ink` colour outlined with `bar` to stay readable over any scenery
(`crates/aurora-viewer/src/cursors.rs`). The rules are those of the Legacy Viewer,
only the drawings change:

| Situation | Cursor |
|---|---|
| Object to touch | `hand-finger` alone, the fingertip is the clicked point |
| Sit, buy, pay, open | arrow + badge `armchair`, `shopping-cart`, `coin`, `package` |
| Media: play, pause, open | arrow + badge `player-play`, `player-pause`, `device-tv` |
| Zoom on the object | arrow + badge `zoom-in` |
| Grab a physical object, move a HUD | `hand-grab` alone |
| Camera: Alt, Ctrl + Alt, Ctrl + Alt + Shift | `focus-centered`, `view-360`, `arrows-move`, centred on the targeted point |
| Eyedropper of the colour picker | `color-picker`, the tip is the sampled point |

With the arrow + badge, the tip of the arrow is the clicked point.

## Shortcut keys

In Preferences › Shortcuts, each key of a shortcut is a light capsule on
the dark button, without a "+" between them. Letters, digits, F1 to F9 and
the arrows use Tabler's key icons (`square-letter-w-filled`,
`square-number-1-filled`, `square-f5-filled`, `square-arrow-up-filled`).
The other keys (Ctrl, Alt, Shift, Esc, F10 to F12, numeric keypad, accented
letters…) are a capsule of the same shape around their name; a mouse button
adds the `mouse-filled` icon before its name, as Tabler has no icon per
button.
