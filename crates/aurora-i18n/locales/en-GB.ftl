### British English overrides of en.ftl.
### Shared with Australian English through catalogue inheritance.

language-name = English (UK)

anim-couple-face-tip = Stand Person B half a metre in front of you, facing you

joystick-enable-gamepad-hint = Use a recognised gamepad for avatar movement and camera control.

joystick-center-hint = Centre the controls and release all buttons after changing settings, switching modes or returning to this window.

joystick-zoom = Zoom (analogue axis)

joystick-zoom-hold-hint = Assign a button or trigger, then hold it while moving the zoom axis. Other camera actions on that axis pause; centre it after release to resume them. In relative mode, release keeps the zoom and a quick double-press restores the reference field of view.

joystick-axis-hint = Click to assign, then move an axis. The dominant movement is selected; release and retry if the gesture stays diagonal. None disables it; Escape cancels. Invert reverses direction for both Avatar and Camera. Each keeps its own gain (0 disables it) and dead zone around the centre (0–1).

toolbar-seg-middle = Centre

toolbar-align-center = Centre aligned

toolbar-align-middle = Centre aligned

area-search-neighbors = Include neighbouring regions

director-key-time-tip = When the camera is on this keyframe. Only this keyframe moves, between its two neighbours.

director-lock-on-tip = The target stays in the centre of the view wherever you move the camera, with no keyframe needed. A keyframe added while the view is locked is locked on this target, even if you choose another one afterwards; each keyframe keeps its own lock, set in its row of the Track tab.

director-key-color = Background colour

director-key-color-tip = Fills the empty background with one flat colour, green at first: a green screen, to key out when editing the film. Choose a colour the subject does not wear.

director-shot-orbit-tip = The camera goes round the centre, looking at it, from where it is now.

director-shot-dolly-zoom-tip = The camera comes closer to the centre (or backs away) while the lens widens (or narrows): the subject keeps its size, the background stretches or closes in. The vertigo effect.

director-shot-crane-tip = The camera rises straight up, still looking at the centre.

director-shot-push-in-tip = The camera moves forward along the view, never past the centre.

director-shot-fly-over-tip = The camera passes over the centre and on to its other side, looking ahead and down.

director-shot-arc-tip = How far round the centre the camera goes: 360° is one turn.

director-shot-distance-tip = Metres the camera travels.

director-shot-rise-tip = Metres the camera rises.

director-shot-over-tip = Metres above the centre the camera passes (above the camera's own height when it is higher).

director-shot-around = Centre

director-shot-no-room = No room for this shot from here: the camera is on its centre, or the lens is already at its limit.

director-shot-radius-tip = The orbit's distance from the centre, metres. Live: the camera's own, so that the orbit starts where it is.

director-shot-height-tip = The orbit's height over the centre, metres. Live: the camera's own.

director-shot-climb-tip = Metres the camera climbs (or comes down) from the orbit's start to its end: a spiral.

director-output-format-tip = What the film is written as. A format this ffmpeg cannot write is greyed out.

director-title-centre = Centre

director-title-color = Colour

director-title-color-tip = The title's colour.

login-accounts-confirm-mfa = Forget the 30-day 2FA authorisation for { $name }? A new code will be requested at the next login.

login-accounts-confirm-remove = Remove { $name } from this viewer and forget its saved password and 2FA authorisation?

import-decompose = Analyse convex hulls

import-local-meters = Assume units are metres

import-physics-resolution-help = Precision of the 3D grid used to analyse collisions ({ $voxels } voxels). Higher settings can preserve finer details, but take longer and use more memory.

import-cancelled = Cancelled

import-preview-colors = Preview colours

import-base-color = Base colour

inventory-syncing = Synchronising…

inventory-synced = Synchronised

toolbar-command-wearable-favorites = Favourite wearables

inventory-tab-favorites = Favourites

ao-invalid-note = This notecard contains no recognised animations. Put it in the same folder as the animations it names.

notecard-markdown-help-code = Use a ```lsl block for the script editor’s colours. In Edit: Tab indents, Shift+Tab outdents, Enter keeps LSL indentation and closing braces align with their opener. Ctrl+K comments selected LSL lines; Ctrl+Shift+K toggles a block comment. Pasted text stays unchanged.

bug-account-needed = A GitHub account is required. Authorisation takes place in your browser.

bug-authorize-browser = Open authorisation page

bug-cancel-auth = Cancel authorisation

bug-github-denied = GitHub refused authorisation or access. Use the browser option or check the app’s installation.

bug-github-expired = The authorisation code expired. Please connect again.

bug-github-unauthorized = The GitHub authorisation expired or was revoked. Please connect again.

ui-options-favorites-access = Favourites access

ui-options-favorites-access-help = Where the islands show your favourites: a button of the place island and its menu, a row under the islands, or a button of the dock and its grid.

ui-options-traffic-indicator-help = A small curve of the network traffic in the system island, coloured by the losses, like the Legacy Viewer's indicator. A click opens Performance › Network.

favorites-filter = Filter favourites

favorites-empty-title = No favourites yet

favorites-empty-text = Favourites are the landmarks of your inventory's Favorites folder, as in the Legacy Viewer. The star at the end of the place island adds where you are.

favorites-add-here = Add this place to favourites

favorites-already-there = already in your favourites

favorites-row-empty = No favourites: drag a landmark here, or click the star to add this place.

favorites-row-fold = Fold the favourites row

favorites-row-unfold = Show the favourites row

favorites-no-match = No favourite matches the filter.

bug-demo-title = Example: a texture stays grey

bug-demo-actual = A panel stays grey. This is a sample report, not a real bug.

language-file-help = Use a Fluent translation file (.ftl) instead of the list, in the format of the viewer's own catalogues. It is read when the viewer starts; the texts it lacks are shown in English.

common-add-to-favorites = Add to favourites

ui-contacts-add-or-remove-a-group-from-favorites = Add or remove a group from favourites

ui-place-details-add-to-favorites-bar = Add to favourites bar

ui-options-synchronizes-frames-with-the-monitor-refresh-rate-no-tearing = Synchronises frames with the monitor refresh rate (no tearing, slightly higher latency). Disabled by default

ui-options-blend-order-help = Standard sorts the transparent surfaces object by object, as the Legacy Viewer does: surfaces that cross or sit inside one another can show in the wrong order. Exact composes them in their true order at each pixel, whatever the object, for more GPU time and video memory. Its levels set how faint a layer must be to be mixed with its neighbours instead of placed one by one: Full places every layer, Fast costs the least. Stochastic estimates that order from 8 random samples per pixel averaged over the frames: close to Exact when still, with a faint grain and a softer image in motion; the only one of the two that also works with MSAA. Standard is used in place of Exact with MSAA

ui-options-colored-sky-ambient = Coloured sky ambient

ui-options-colored-sky-ambient-help = Lights with the sky's ambient colour at full strength, as the first PBR viewers did, instead of a softened ambient that linear and HDR skies turn grey: warm or tinted skies colour the interiors again. Classic skies keep their colour either way and only get a little brighter. Differs from the Legacy Viewer

ui-options-planet-curvature-help = Draws the world on a round planet: open water falls away to a real horizon instead of running out to eye level, with the sky below it. Only the view changes: regions, physics and what others see stay flat. On the Earth the drop within the draw distance is a few centimetres; it shows on the far sea. Differs from the Legacy Viewer

ui-options-planet-radius-help = The radius of the planet drawn, in kilometres. The Earth's is 6,371 km: from 10 m up, the sea ends about 11 km away. A smaller planet brings the horizon closer

ui-options-login-favorites = Show my favourite landmarks at login

ui-context-attach-cancelled = Attach cancelled.

ui-options-above-this-limit-the-avatar-appears-as-a-gray = Above this limit, the avatar appears as a grey silhouette without attachments (as in the Legacy Viewer; far right: unlimited)

app-aurora-viewer-unable-to-initialize-vulkan = { "Aurora Viewer: unable to initialise Vulkan: " }{ $e }

app-aurora-viewer-unable-to-initialize-metal = { "Aurora Viewer: unable to initialise Metal: " }{ $e }

demo-region-notecard-rules-text = Rules of the Aurora demo estate: build tidily, keep your scripts light, be kind to your neighbours.

demo-land-welcome-to-the-aurora-demo-estate-by-buying-land = { "Welcome to the Aurora Demo estate.\u000A\u000ABy buying land on this estate, you agree to the following rules:\u000A\u000A1. Builds must fit the estate's Nordic theme.\u000A2. No advertising boards visible from neighbouring parcels.\u000A3. Background sounds (radios, sound scripts) must stay within your parcel.\u000A4. Parcels left unused for 90 days may be reclaimed.\u000A\u000AThank you for keeping Aurora's plaza welcoming to everyone." }

color-picker-title = Colour picker

color-picker-help = Choose a colour using RGB, LSL, hexadecimal or hue / saturation / lightness. The eyedropper samples an object face’s tint. Drag the current colour onto a slot to save it. Cancel restores the original colours.

color-picker-original = Original colour

color-picker-mixed = Mixed colours

color-picker-current = Current colour

color-picker-original-mixed = The faces have different colours; Cancel restores each of them.

color-picker-save-hint = Drag the current colour onto a slot to save it.

color-picker-save-slot = Save the current colour here

color-picker-saved = Colour saved in the palette.

color-picker-apply-hint = Preview the colour on the object while choosing.

color-picker-open = Open the colour picker

ui-minimap-auto-centering = Auto-centring

build-ui-tools-center = Centre

ui-worldmap-center-map-on-my-position = Centre map on my position

contacts-import-hint = Choose settings_friends_groups.xml from your Legacy Viewer account folder, or an exported contact set. Members are merged; existing colours and nicknames are kept.

contacts-import-success = Contact sets imported ({ $count }). Existing members, colours and nicknames were kept.

demo-choose-a-color-for-the-water = Choose a colour for the water:

ui-options-distance-in-name-tags-help = After the name, in the colour of its range: within chat, within a shout, farther

ui-options-names-by-distance = Names coloured by distance

ui-options-names-by-distance-help = Within chat, within a shout, farther, as in the radar; a name coloured by relation or by a contact set keeps its colour

common-behavior = Behaviour

app-login-canceled = Login cancelled.

ui-options-favorite-emojis = Favourite emojis

ui-options-favorite-emojis-hint = Right-click an emoji in the picker or the recent row to keep it: favourites come first in that row and have their own tab in the picker.

ui-options-blocked-chat-greyed = Greyed

ui-options-flash-conversations-button-hint = When a message arrives with the Conversations window closed, its button flashes softly in violet, then stays tinted until the window is opened. Minimised, the window's bar flashes instead, then stays tinted while messages wait unread.

common-color = Colour

ui-contacts-friend-list-color-based-on-contact-set = Friend list colour based on contact set

ui-options-name-tag-color = Name colours

ui-options-names-by-relation = Colour the names by relation

ui-contacts-default-color = Default colour

ui-options-colors = Colours

ui-options-default-colors = Default colours

ui-options-customize-colors-icons-and-the-toolbar-in = Customise colours, icons and the toolbar in:

common-create-favorite = Create favourite

landmark-create-favorites-bar = Favourites bar

landmark-star-in-favorites = In Favourites: { $name }
    Click to edit it

landmark-star-add-favorites = Add to Favourites

ui-login-favorites = Favourites

app-destination-not-recognized = Destination not recognised.

ui-options-backup-save-help = Saves the preferences, the colours, the shortcuts, the bars, the windows' layout, the skin and each account's preference files (contact sets, voice volumes, blocked assets…) to one file, to put them back after a reset or on another PC. Nothing of the login goes in it, nor your GIF service keys.

ui-options-backup-part-settings = Preferences, with the colours, shortcuts and bars

uninstall-second-factor = Second-factor authorisation of { $user } ({ $grid }), in the credential store

uninstall-crash-reports = GitHub authorisation of crash reports, in the credential store

ui-options-toast-friends-seconds-aurora = For the Legacy Viewer's toasts (Preferences › Aurora › Notifications): Aurora's notification centre keeps its own timing

ui-options-ims-in-notification-center-hint = The toasts of messages also stay in the notification centre until their conversation is read. Off, as in the Legacy Viewer: the Conversations window keeps them.

ui-options-keywords-on-hint = Colours the messages that hold one of your words and plays a sound when they arrive

ui-options-keywords-color = Change the message colour

ui-options-camera-offset-from-the-avatar-in-meters-x-forward-y = Offset from the avatar, in metres: X forward, Y left, Z up (Ctrl + wheel raises the camera)

dev-toolbox-help = Debug tools in one place. Each button has an identifier (J2, L1…) that does not depend on the language: an agent or a developer can ask you to press one and to paste what it copies, instead of your whole log. Customise to move the buttons; right-click a button to pin it to Favourites.

dev-toolbox-customize = Customise

dev-toolbox-pin = Pin to Favourites

dev-toolbox-unpin = Unpin from Favourites

dev-toolbox-group-favorites = Favourites

ui-places-move-to-favorites = Move to Favourites

ui-chat-gif-favorites = Favourites

ui-chat-gif-no-favorites = No favourite yet: hover a GIF and click its star.

ui-land-options-prevents-scripts-from-pushing-people-useful-for-preventing-abusive = Prevents scripts from pushing people. Useful for preventing abusive behaviour on your land.

ui-profile-let-other-residents-know-your-favorite-places-in-second = Let other residents know your favourite places in Second Life.

ui-places-drag-a-landmark-here-to-add-it-to-your = Drag a landmark here to add it to your Favourites.

common-favorites = Favourites

ui-options-always-show-teleport-cancel-help = Shows Cancel on the teleport screen even when the region says the teleport cannot be cancelled. The region still refuses to cancel those, and RLVa restrictions can still hide the button

ui-options-high-contrast-help = For poor vision: white text on opaque black surfaces, one light accent and a visible border around buttons, fields and windows, instead of the skin's colours. Every text reaches at least 7:1 against its background.

ui-options-high-contrast-skin = While high contrast is on (Accessibility), it replaces the skin's colours.

ui-options-interface-colors = Interface colours

ui-options-interface-colors-high-contrast = High contrast is on: it keeps its own colours. These come back when it is turned off.

ui-options-accent-color = Accent colour

ui-options-all-interface-colors = Every interface colour

ui-options-color-blindness = Colour blindness

ui-options-color-blindness-help = The interface's colours adapt to your colour vision: what is told apart by colour (online, away, busy, success, warnings, errors…) stays easy to tell apart. The status dots also differ by their shape.

ui-options-color-blindness-strength-help = 100 % for a full deficiency (-anopia), less for a milder one (-anomaly) that still sees some of the colour.

colorblind-off-help = The interface's usual colours.

colorblind-mono = Achromatopsia (no colours)

colorblind-mono-help = Everything in shades of grey, the meaningful colours at clearly different lightnesses.

ui-appearance-portrait-grey = Grey

ui-options-night-sky-brightness-help = How bright the real stars are, from half to four times the catalogue's. Twice by default, so they show over a night sky that is not quite black.

ui-options-probe-influence-turquoise-for-automatic-purple-for-manual-gray = Probe influence: turquoise for automatic, purple for manual, grey for not yet captured

build-ui-floaters-intersections-up-to-meters = Intersections up to (metres)

build-mesh-behavior = Object LOD behaviour

ui-land-environment-the-parcel-must-be-at-least-128-square-meters = The parcel must be at least 128 square metres to support an environment.

ui-bars-customize-bars = Customise the bars…

ui-options-mute-when-minimized = Mute when minimised

ui-options-mute-when-minimized-help = No sound at all while the viewer is minimised to the taskbar (losing the focus is not enough). Not on Wayland, which does not tell a window that it is minimised

ui-minimap-custom-marker = Custom colour…

ui-avatar-mark-title = Marker colour

world-inventory-preview-pbr-material-color-metallic-roughness-double-sided-base-color = { "PBR material\u000A\u000AColour: " }{ $value0 }{ "\u000AMetallic: " }{ $value1 }{ "\u000ARoughness: " }{ $value2 }{ "\u000ADouble-sided: " }{ $value3 }{ "\u000ABase colour texture: " }{ $value4 }{ "\u000ANormal texture: " }{ $value5 }{ "\u000AMetallic / roughness texture: " }{ $value6 }{ "\u000AEmissive texture: " }{ $value7 }

ui-appearance-favorites-to-top = Favourites to top

ui-options-screen-indirect-intensity-help = How much light the surfaces around bounce: 1 is a physical bounce, higher shows the colours more

ui-options-screen-indirect-light-help = Light bounced by the surfaces around (a lit wall colours the floor beside it), added on top of the GTAO occlusion, which stays the same; about a tenth of a millisecond more on the graphics card

ui-inventory-controls-favorites-tab = Favourites tab

ui-chat-blocked-show = Lines from residents you blocked: Show puts them here, greyed.

common-customize = Customise

build-ui-features-center-point = Centre point

build-ui-object-position-meters = Position (metres)

ui-options-light-positions-and-radii-in-their-colors = Light positions and radii (in their colours)

build-ui-tools-drag-the-colored-bands-to-rotate-the-object = Drag the coloured bands to rotate the object.

ui-minimap-center-map = Centre map

region-restart-cancelled-title = Restart cancelled

region-restart-cancelled-text = A manager cancelled the restart of { $elide ->
       *[other] { $region }
    }.

region-restart-minimize = Minimise

region-restart-neighbour = Neighbour region: { $region }

region-restart-chat-cancelled = The region restart was cancelled.

ui-appearance-gallery-remove-from-favorites = Remove from favourites

controls-orbit-tip = Orbit around the focus: press and pull. Click the centre to return.

camera-position-hint = In metres from the avatar: X forward, Y left, Z up. Ctrl + wheel raises the camera, Shift + wheel the focus point.

build-ui-texture-repeats-meter = Repeats / metre

ui-options-no-neighbour-regions-help = Do not connect to the neighbouring regions: lighter on the network and the computer, but they are not shown, crossings on foot or in vehicles are less reliable and a teleport may be needed. Takes full effect after the next teleport.

report-abuse-category-55 = Harassment > Targeted behaviour intended to disrupt

ui-options-below-the-name-from-green-to-red-based-on-2 = Below the name, from green to red based on the complexity limit (like the Legacy Viewer); grey for yourself

ui-options-each-object-s-tested-sphere-green-drawn-red-outside = Each object's tested sphere: green drawn, red outside view, orange too far, grey too small

ui-profile-delete-this-favorite = Delete this favourite?

common-remove-from-favorites = Remove from favourites

ui-emoji-favorites = Favourites

ui-emoji-favorite = Favourite

ui-emoji-no-favorites = No favourite yet: right-click an emoji to keep it here.

ui-options-vertical-synchronization = Vertical synchronisation

ui-perf-vertical-synchronization-frame-rate-limit-presentation-render-thread-wait = Vertical synchronisation, frame rate limit, presentation, render thread wait

build-ui-texture-synchronize-materials = Synchronise materials

build-ui-object-size-meters = Size (metres)

ui-options-tone-mapping-mix-help = Mix between the linear and the tone mapped colours (0.7 as in the Legacy Viewer)

ui-options-all-settings-graphics-interface-fonts-chat-sound-and-voice = All settings (graphics, interface, fonts, chat, sound and voice, controls, shortcuts, colours, cache) return to their defaults. Username, grid and window size are retained.

ui-options-about-camera = Viewpoint, movements, behaviour and mouselook.

ui-options-about-appearance = Font, text size, colours and skin.

ui-fun-cat-warning-body = What comes next flashes: the whole view changes colour and brightness very quickly, in time with music. It can trigger seizures in people with photosensitive epilepsy, even in someone who never had one.

ui-options-color-and-sharpness = Colour and sharpness

ui-options-crash-github-hint = Forgets the GitHub authorisation kept on this computer for crash reports. It can also be revoked on github.com, in Settings › Applications.

ui-options-all-colors = Every colour

ui-options-color-vision = Colour vision

session-object-actions-inventory-transfer-canceled-by-the-simulator = Inventory transfer cancelled by the simulator.

login-camera-limits = Like the Legacy Viewer, the camera keeps within the draw distance again and stops in front of walls and objects. To turn this off again: Preferences › Camera › Behaviour, "Unrestricted camera" and "Disable camera collisions".

build-ui-floaters-unit-meters = Unit (metres)

demo-teleport-access-neighbour = Adult neighbour (demo)

photo-look-colors-1970 = 1970's Colours

photo-look-focus = Centre Focus

photo-look-linearize = Linearise

photo-look-posterize = Posterise

photo-look-color-shift = Colour Shift

photo-looks-catalogue = Effects catalogue

photo-looks-catalogue-fold = Show or hide the catalogue

photo-looks-replace-tip = Replace: a click in the catalogue swaps the chosen card instead of adding a new one

photo-looks-favorites = Favourites: every effect you starred

photo-looks-favorites-empty = No favourite yet. Point at an effect and click its star: it shows first in its list, and here.

photo-looks-favorite = Add to favourites

photo-looks-unfavorite = Remove from favourites

photo-knob-border-color = Border colour

photo-knob-tint = Glow colour

photo-knob-hue = Colour kept

photo-knob-color = Colour

photo-knob-bar-color = Bar colour

photo-looks-color = Colour

world-look-help = Filters and LUTs on the 3D view all the time, apart from the photos' ones. The interface is never coloured.

photo-look-a-bleach-bypass-tip = Washed-out colours and hard contrast, as a film that skipped its bleach

photo-look-a-instant-tip = Soft warm colours, raised blacks, darker corners and a white border

photo-look-a-chrome-tip = Rich saturated colours with a little contrast, like a slide film

photo-look-a-vivid-tip = More saturated colours and a little contrast

photo-look-a-faded-tip = Raised blacks and softened colours

photo-look-a-keep-tip = One colour kept, everything else in black and white

photo-look-a-old-lens-tip = A little distortion, colour fringes, a soft glow and darker corners

photo-look-a-vhs-tip = A videotape: torn lines, colour fringes, noise and scan lines

photo-look-a-glitch-tip = Bands torn sideways and the colours split apart

photo-look-a-oil-paint-tip = Flat strokes of colour that keep the edges

photo-look-a-miniature-3d-tip = A blur by distance around the focus and vivid colours: the scene looks like a model

photo-look-a-subject-tip = The surroundings of the focus distance greyed and darkened

photo-vf-timer-cancelled = Self-timer cancelled

photo-vf-help-guide-text = Guide: thirds, golden ratio, diagonals, centre

photo-guide-center = Centre

ui-places-all-hint = Landmarks, favourites and places visited

ui-places-favorites-hint = Drag to change the order of the favourites bar

ui-places-add-favorite = Add to favourites

ui-places-remove-favorite = Remove from favourites

ui-places-no-favorites = Click the star of a landmark to add it to your favourites.

ui-bars-open-favorites = Open Places › Favourites

ui-options-interface-scale-help = Drag to resize the interface live. The centre is the default size (100%). You can also type a percentage and press Enter.

ui-env-editor-cloud-color = Cloud Colour

ui-env-editor-sun-color = Sun & Moon Colour

ui-env-editor-fog-color = Water Fog Colour

monitor-row-scene = Scene and synchronisation

monitor-row-sync = Object synchronisation

monitor-desc-scene = Culling and draw lists, synchronisation of the objects received from the network, particles, lights and reflection probes.

monitor-desc-world = UDP circuits with the region and its neighbours: object updates, movements, acknowledgements.

monitor-region-neighbour = Neighbour

monitor-huds-help = Detaching the HUDs you are not using saves memory and can make the viewer faster; they are often heavily scripted too. Minimising a HUD does not detach it.

monitor-child-agents = + { $count } in neighbouring regions

crash-github-waiting = Waiting for your authorisation on GitHub…

bug-vsync = Vertical synchronisation

script-external-started = External editor opened. Its saves are synchronised with the script.

script-preferences-colors = Editor colours

script-preferences-reset-colors = Theme colours

script-preferences-optimize = Script optimiser

script-preproc-directive = Invalid or unrecognised preprocessor directive.

script-preproc-cancelled = Preprocessing cancelled.

build-place-tip = Then click a surface or the ground: the selection goes against it, centred on the point, shown there as a hologram (P turns it on or off, Esc off).

build-place-offset-tip = How far off the surface it goes (metres, along the surface’s normal); negative sinks it in.

script-color-hint = Possible colour: click to open the colour picker.

radar-opt-color-names = Colour names by distance

ui-options-reset-windows-text = Every window goes back to its default place and size, and a minimised window opens expanded. Your settings are kept. You can still undo for a few seconds.

ui-options-aurora-profiles-help = A header with picture, badges and actions, the content in cards; or windows with Second Life, Feed, Favourites… tabs like the Legacy Viewer. For avatars and groups.

ui-options-aurora-rez-effect-help = An object or avatar appearing while you watch: a glowing grid while it loads, then a line of light that reveals it. Or grey until it has loaded, like the Legacy Viewer. Off with Reduce animations.

ui-options-aurora-rez-effect-classic = Grey (Legacy)

ui-options-aurora-ground-feet-help = Replace standard Legacy Viewer foot placement with Aurora's adaptation to slopes and individual steps. Foot lifts and high-priority poses keep control. When off, native standing, crouching and recovery animations use the standard behaviour. The two corrections never accumulate.

ui-options-aurora-animation-clock-experimental = Experimental: synchronised playback is still being validated on the grid for avatars and animesh. Legacy remains the default.

ui-options-aurora-animation-clock-help = Avatars and animesh: the synchronised clock follows elapsed time from the first received signal, including after late loading or editing. Legacy starts at loading and resumes from the paused pose after editing.

ui-options-aurora-animation-clock-synchronized = Synchronised clock

ui-options-plates-friend-tint-help = A light tint of the friends' colour (Name colours), to spot them in a crowd. A contact set's background comes first.

ui-options-plate-shift-click-help = Independent action with Shift held on someone's plate. Alt and Ctrl + Shift keep their usual behaviour.

ui-options-plate-ctrl-click-help = Independent action with Ctrl held on someone's plate. Alt and Ctrl + Shift keep their usual behaviour.

ui-options-chat-logs-show-help = When a conversation opens, the end of its file is shown greyed above the new messages.

search-land-sort-per-meter = Price per metre

marketplace-listings-initializing-title = Initialising Marketplace.

search-group-enrollment = Enrolment:

region-int-delay-cap = Other: from 15 s to 1 h. The restart can be cancelled until the end, here or from the header's pill.

region-terrain-bake-desc = The current ground becomes the centre of the limits and what the “Revert” tool brings back.

region-terrain-heights-desc = Low: highest point of texture 1. High: lowest point of texture 4. In metres.

region-ask-bake = Do you really want to bake the current terrain, make it the centre of the raise and lower limits, and what the “Revert” tool brings back?

ui-land-manage-see-avatars = See and talk from the neighbouring parcels

ui-land-manage-media-why = Music (URL, favourites), the parcel's media (type, page, replaced texture, size, loop), gesture and object sounds, avatar sounds (everyone / group), voice (on, limited to the parcel), hide MOAP.

ui-group-create-cost-to-join-tip = Sets whether to require an enrolment fee to join the group

ui-group-create-rating-tip = Maturity ratings designate the type of content and behaviour allowed in a group

ui-group-profile-open-enrollment = Open enrolment

ui-group-notices-drop = Drag an item or landmark from Inventory, Favourites, or Places (copy + transfer)

ui-group-admin-fee = Enrolment fee

group-ability-member-options = Toggle 'Open Enrolment' and change 'Enrolment fee'

group-ability-member-options-help = Toggle 'Open Enrolment' to let new Members join without an invitation, and change the 'Enrolment fee' in the General section.

currency-bought = { "Thank you for your payment!\u000A\u000AYour L$ balance will be updated when processing completes. If processing takes more than 20 minutes, your transaction may be cancelled. In that case, the purchase amount will be credited to your US$ balance.\u000A\u000AThe status of your payment can be checked on the Transaction History page of your account." }

import-mesh-account-denied = This account is not authorised to upload meshes.

shape-meters = Metres

texture-library-manage = Organise in inventory

voice-show-visualizers = Show voice visualisers

voice-show-self = Show my own voice visualiser

movement-pass-cancelled = Passage cancelled.

bridge-disabled = The Aurora bridge is turned off in Preferences › Aurora › Behaviour.

water-effects-absorption = Depth colour and underwater haze

import-collision-analyze = Analyse

import-collision-analyzing = Analysing…

poser-bridge-disabled = Requests go through the Aurora bridge, turned off in Preferences › Aurora › Behaviour.

poser-hint-position = metres, avatar axes · the chain follows

poser-hint-scale = factors · centre cube = uniform

poser-ball-scale = Cubes in world: one axis · centre: uniform

poser-bone-forehead-center = Forehead centre

poser-bone-brow-center = Eyebrow centre

poser-bone-nose-center = Nose centre

poser-bone-lip-lower-center = Lower lip centre

poser-bone-lip-upper-center = Upper lip centre

poser-position = Local position (metres)

poser-close-confirm = Closing stops the pose: its { $count ->
    [one] { $count } edited joint is
   *[other] { $count } edited joints are
    } lost. Minimise the window to keep them.

poser-close-minimize = Minimise

anim-edit-pelvis-bound = Native animation positions must be finite and within -5 to 5 metres per axis.

rlva-im-queries-hint = Recognise @version, @stopim, @list and @except from online residents. Listing restrictions asks for permission.

rlva-legacy-naming = Recognise legacy attachment folder names

rlva-legacy-naming-hint = Also recognise point folders such as Chest, .chest and Outfit (chest).

set-editor-colour = Colour

set-editor-tab-behaviour = Behaviour

set-editor-shown-on = Show the set's colour on

set-editor-colour-custom = Custom colour

build-ui-floaters-limit-select-distance = Limit select distance (metres)

build-ui-floaters-limit-drag-distance = Limit drag distance (metres)

build-layout-organize = Organise options…

build-layout-finish = Finish organising

blender-shape-changed = The worn shape or its Modify rights changed. Transfer cancelled.

blender-live-upload-materials = Includes base colours and textures. Upload PBR materials separately.

blender-live-invalid-color = Empty surface or invalid material colour.

inventory-menu-favorites = Favourites

inventory-menu-organize = Organise

inventory-menu-finish = Finish organising

scene-menu-customize = Customise this menu

blender-live-warning-textures = Textures skipped, plain colour shown

parcel-visibility-same-owner = Include neighbouring parcels with the same owner

demo-parcel-visibility-neighbour = Same owner's neighbouring parcel

parcel-visibility-same-group = Include neighbouring parcels with the same group

parcel-visibility-same-group-help = Match the current parcel’s assigned group, or its owning group if deeded. Neighbours may be assigned to that group or owned by it. Parcels without a group do not match.

parcel-visibility-always-visible-help = These parcels remain visible while filtering, regardless of their owner or group. Add the current parcel or select a neighbour in About Land and add the selected parcel. Remove an entry to restore the normal filtering rules.

demo-parcel-visibility-pinned = Always-visible neighbour

demo-parcel-visibility-long = Aurora Observatory and Northern Gardens — Visitor Centre and Exhibition Pavilion

portal-help = Opens four metres ahead for two minutes, for you alone: nobody else sees it. Walk through to travel. If it appears too close, move away first, then return.

wearable-tint = Colour

wearable-group-skin-color = Skin colour

wearable-group-hair-color = Hair colour

wearable-param-99 = Eye Colour

wearable-param-108 = Rainbow Colour

wearable-param-112 = Rainbow Colour

wearable-param-700 = Lipstick Colour

wearable-param-705 = Blush Colour

wearable-param-708 = Outer Shadow Colour

wearable-param-712 = Inner Shadow Colour

wearable-param-714 = Eyeliner Colour

wearable-param-715 = Nail Polish Colour

about-tab-licences = Licences

about-licence-title = Aurora Viewer's licence

particle-editor-start-color = Start colour

particle-editor-end-color = End colour

particle-editor-blend-dest-color = Destination colour

particle-editor-blend-src-color = Source colour

particle-editor-blend-one-minus-dest-color = 1 − destination colour

particle-editor-blend-one-minus-src-color = 1 − source colour

particle-editor-flag-interpolate-color = Interpolate colour

inventory-favorites-show-parent-folders = Show parent folders in Favourites

inventory-favorites-show-parent-folders-help = Show favourites in their original folder tree. Off by default: favourites appear directly in the list; a favourite folder keeps its contents.

inventory-folder-colors = Folder colours

inventory-folder-colors-help = Add "Folder colour…" to a folder's menu, to give its icon a colour of your choice. The colours are kept in this viewer only: nobody else sees them.

inventory-folder-color = Folder colour…

inventory-folder-color-title = Folder colour

inventory-folder-color-custom = Custom colour

inventory-folder-color-private = Only you see these colours.

inventory-folder-color-none = Select a folder to colour it.

area-search-neighbor-regions = In neighbouring regions

import-folders-enable = Organise upload folders

ui-weather-northern-lights-colours = Colours

wearable-favorites-all = All Favourites

wearable-favorites-new-folder-tip = Create new favourites folder

wearable-favorites-rename-folder-tip = Rename selected favourites folder

wearable-favorites-delete-folder-tip = Delete selected favourites folder

wearable-favorites-import = Import favourites from the Legacy Viewer

wearable-favorites-import-tip = Links the wearables favourited in the Legacy Viewer into these favourites, with their folders. Nothing is copied or moved, and the Legacy Viewer's favourites stay as they are.

wearable-favorites-options-tip = Wearable favourites options

wearable-favorites-remove-tip = Remove wearable from favourites list

wearable-favorites-delete-confirm =
    Delete Favorite Wearables folder "{ $name }"?

    This removes the folder and its favourite links only. Original inventory items are not deleted.

debug-settings-type-color = Colour

ui-options-build-size-hint = Width, depth and height in metres (0.01 to 10)

ui-options-build-beam-color = Beam colour

ui-options-build-beam-color-hint = The colour of your beams, as the others see them too.

photo-looks-not-in-360 = Not drawn in a 360° picture: only the colour looks are.

app-mac-minimize = Minimise

vr-vibrancy = VR Colour vibrancy

vr-vibrancy-help = Adjust headset colour intensity. The default is 1.0.

game-control-crosshair-hint = Draw a crosshair at the centre of the screen while flying the flycam.

notecard-lsl-demo =
    # LSL in a notecard
    Use the **LSL** button, then type as in the script editor.

    ```lsl
    default
    { "{" }
        state_entry()
    { "    {" }
            // LSL colours and indentation
            integer count = 2;
            vector tint = <1.0, 0.5, 0.0>;
            llSay(PUBLIC_CHANNEL, "Hello 🐺");
    { "    }" }
    { "}" }
    ```
