<!--
News of the Aurora Viewer login screen, in English.

The viewer reads this file from the main branch at every start: once the
change is merged, the news item shows up for everybody at the next launch
(GitHub takes a few minutes to serve the new version of the file), without
releasing a new viewer.

One news item = one "##" heading, newest first:

## 2026-10-08 — Title of the news item
One or more lines of plain text (no bold, no list, no image).
An empty line starts a new paragraph.
[Read more](https://example.org/optional-link)

- The date is written YYYY-MM-DD, followed by "—", "-" or ":". It is
  optional, and so is the link.
- The link must start with https://; a click on the news item opens it.
  One link per news item.
- At most 20 news items are shown; the title is cut at 120 characters, the
  text at 600.
- Each language has its own file (en.md, fr.md, ja.md): add the news item
  to each. A language without a file shows en.md.
- Full instructions (in French): HUMANS.md, "Publier une actualité".
-->

# Aurora Viewer news

## 2026-11-13 — Aurora 0.9.9: Aurora Weather, Avatar Studio and Lua scripts
Aurora Weather brings rain, snow, wind, thunder and northern lights, with puddles, lying snow and footprints: choose yours in the Environment window, or let regions set it (off by default). Avatar Studio is a calm space to dress, pose and photograph your avatar, with its own animation editor: keyframes, a timeline, ANIM and BVH import. SLua scripts get a real editor, with Lua colours, completion and help. The cache is cleared once at the first start and fills again by itself; your settings are kept.
[Read more](https://github.com/Aurora-Viewer/Aurora-Viewer/releases/tag/v0.9.9)

## 2026-11-13 — Gamepad, VR, building and avatars
With a gamepad, on the desktop or in VR, switch between First Person, Avatar Mode and Flycam. The camera can follow an animated body part. When others edit, grab or give something, their beam shows, and yours is seen. Drag an item out of Contents to rez it, save an object back into the contents it came from, and tick Animated Mesh on a whole linkset. Mesh heads no longer get black eyebrows on AMD cards or after a graphics card reset, and AO walks and runs loop again.

## 2026-11-13 — More like the Legacy Viewer
Opacity sliders for windows, bars and menus (Preferences › Appearance), the nearby media list under the media button, every don't-ask-again answer in Preferences › Confirmations, Maximum nearby lights, moderator options in group chats, a ban from all your groups, parcel music that fades between stations, and the sun and moon drawn like the Legacy Viewer. Thanks to AiAiCaptainn, circuskid, Gaalrauch, Izusl, KanikaBreed, lambskin, Memekiller, Nowalski, Odessa-Draekavik, RheaMercury and voidel.

## 2026-11-12 — Aurora 0.9.8: fixes for building, chat and AMD cards
Select face picks the faces of what you wear again, and Derender with Edit linked parts hides only the selected part. The channel selector shows by every nearby chat input and keeps the channel you chose. On AMD cards, tree leaves are no longer drawn as squares and mesh hair has no more streaks, and blended eyelashes stay visible behind mesh hair. The cache is cleared once at the first start and fills again by itself; your settings are kept.
[Read more](https://github.com/Aurora-Viewer/Aurora-Viewer/releases/tag/v0.9.8)

## 2026-11-12 — Startup, voice and the poser
A small Aurora window now shows while the viewer starts and after an update's restart. Nearby voice connects on slow, high-latency links, invisible HUD buttons take a left click again, and the avatar complexity notice no longer repeats (off by default, Preferences › Graphics › Avatars › Complexity notices). In the poser, holding Ctrl highlights the part of the avatar's meshes that Ctrl + click selects. Thanks to Bruica, iGao, KanikaBreed, Nowalski and Odessa-Draekavik.

## 2026-11-12 — Aurora 0.9.7: a rethought Build window, script recovery and Destinations
The Build window is reorganised: the selection sits in a header that folds away, the options form columns above the tabs that you arrange as you like with Organize options…, position, size and rotation keep the full width, and Contents gets its own display menu. After a crash or a lost connection, Script recovery brings back the unsaved changes of your scripts at the next login, in Lost And Found. Destinations opens Aurora's own window on the Destination Guide: search, categories, ratings and a one-click teleport. Your cache and settings are kept.
[Read more](https://github.com/Aurora-Viewer/Aurora-Viewer/releases/tag/v0.9.7)

## 2026-11-12 — Poser, appearance and avatars
The poser gets a compact window that leaves the scene free, Shift + click to move several joints together, and a ring that turns a joint around the camera's view. Export XML, next to Import XML in the shape, physics and clothing editors, saves an appearance file, and Appearance's gear menu exports your whole look. Your avatar plays the talk, shout and whisper animations in nearby chat, Reload avatar in the right-click menu repairs an avatar that looks broken, and the wet and shine layers of mesh bodies now show.

## 2026-11-12 — More like the Legacy Viewer, and fixes
Script dialogs per object keeps one menu per object instead of a pile. Advanced › Release keys frees the controls a broken vehicle or HUD keeps, and a teleport in progress can be cancelled. Group notices also appear in the group's chat, Preferred browser chooses where web links open, and the Milky Way is fainter, with its own slider. On OpenSim, Request Admin Status turns on admin powers (god mode). Thanks to anne-skydancer, Bruica, circuskid, Gaalrauch, iGao, KanikaBreed, Marakiinfinity, Nowalski and Odessa-Draekavik.

## 2026-11-11 — Aurora 0.9.6: outfit showcase, auto portraits and 65 photo looks
Appearance's Outfit gallery becomes a showcase: large pictures of your outfits, a rail for favourites, outfits without a picture and folders, and wear in one click (the small tiles stay in Preferences › Aurora). Auto portrait frames you in a studio for the outfit's free picture, Take the portraits does every outfit in one go, and an optional HD picture costs the grid's upload price. The photo LUT tab now holds 20 looks by Czar Fenris and 45 by Nanner Pudding, thank you both! Your cache and settings are kept.
[Read more](https://github.com/Aurora-Viewer/Aurora-Viewer/releases/tag/v0.9.6)

## 2026-11-11 — The Milky Way from real stars, and Rendering types
The Milky Way is now drawn from the 2.5 million stars of the Tycho-2 Catalogue, with its star clouds and dark lanes, and Star hour can show the sky by the real clock. Advanced › Rendering types hides terrain, water, objects, avatars or particles to see behind them, like the Legacy Viewer. A Mac starts at the display's logical resolution with Mid-High quality, AMD RX 9000 cards keep their contact shadows, and HUD media answer first in busy regions.

## 2026-11-11 — Poser, building, inventory and fixes
Hold Space to brake your avatar, like the Legacy Viewer. The poser gets Save as… and shows where a pose went. Move handles are easier to grab, land selections snap to the parcel grid, and glTF materials keep 2K maps. Favorites lists starred items without their folders, and the delete confirmation can be turned off. Region / Estate asks before a change that can send people home. Thanks to anne-skydancer, circuskid, emmelinedevere, Gaalrauch, KanikaBreed, LisaScheers, Marakiinfinity, Memekiller, Nowalski and Odessa-Draekavik.

## 2026-11-10 — Aurora 0.9.5: Region Tracker, contact set markers and appearance XML
World › Region tracker watches a list of regions, like the Legacy Viewer: whether each one is up and how many avatars are there, without opening the map; the World map's Track region adds one. A contact set can now mark its members in the world, through walls and at the screen's edge: set its World marker in People › Contact sets (off by default). Import XML, in the shape, physics and clothing editors, loads an appearance file from a shape website or another viewer. Your cache and settings are kept.
[Read more](https://github.com/Aurora-Viewer/Aurora-Viewer/releases/tag/v0.9.5)

## 2026-11-10 — Emojis, chat and L$
Your recent emojis sit in a row by the chat field, and the GIF picker shows the GIFs you sent. Turn on Favorite emojis in Preferences › Chat to keep yours first, with a star tab in the picker. Alt+Enter sends ((OOC)) chat, and Request teleport sits next to Offer teleport in the conversation. A button collapses or expands the conversation list. The L$ you receive or spend show under your balance before they join it, and a resident paying you shows a notification.

## 2026-11-10 — Your view, the world and fixes
Preferences › Graphics can hide the face lights other avatars wear and keep yours, and World › Hide name tags hides every tag in one click. Avatar tips can stay with world tooltips off, and the media filter's question is a banner that lets you keep flying. The ground stays at region crossings, a full video memory no longer freezes the world, and parcel radios hold through teleports. Thanks to circuskid, ClifHofmann, dash-sl, Gaalrauch, iGao, KanikaBreed, Nowalski, Odessa-Draekavik and RheaMercury.

## 2026-11-08 — Aurora 0.9.4: Debug settings, and more of the Legacy Viewer's options
The toolbar's Debug settings button, or Ctrl+Alt+Shift+S, opens the Debug settings window: every setting by name, with a search, a star on the ones you changed and Reset to default. Preferences › Build sets what a new prim starts as: colour, transparency, glow, fullbright, physical, temporary or phantom. Preferences › Graphics gets a progressive draw distance (off by default), the Trees detail and Particles from worn objects, and Preferences › Protections can hide objects with a huge surface. Your cache and settings are kept.
[Read more](https://github.com/Aurora-Viewer/Aurora-Viewer/releases/tag/v0.9.4)

## 2026-11-08 — Neighbouring regions, objects that stay in view, calls that hold
Aurora now asks the region for your own draw distance, so the neighbouring regions appear at 256 m and beyond, as in the Legacy Viewer. Objects in plain view no longer vanish at some camera angles, and the world comes back by itself when the video memory was full at login. Media screens in busy regions load and follow their remotes, and textures left white come back without a relog. Private calls stay connected across teleports, and parcel music no longer cuts out while you cam around.

## 2026-11-08 — Chat, camera, avatars and fixes
Camera smoothing goes up to 300 without jumps, and Preferences › Chat sets how many lines show above the chat bar. Ctrl+Tab cycles through the conversations, the Japanese IME shows the active conversion segment, and script text boxes take several lines. About Land edits stick, the AO no longer leaves a sit playing after a teleport, and the RLVa tools get an Exceptions tab. Thanks to AiAiCaptainn, anne-skydancer, circuskid, ClifHofmann, emmelinedevere, Gaalrauch, iGao, KanikaBreed, Marakiinfinity, MilkIsPrettyGood, Nowalski, Odessa-Draekavik and RheaMercury.

## 2026-11-06 — Aurora 0.9.3: AMD FSR 3.1, and steadier on every graphics card
Choose FSR 3.1 under Anti-aliasing in Preferences › Graphics: smooth edges at the window's size, or a world drawn smaller and rebuilt sharp, for more frames per second. After a graphics card reset Aurora reopens its window and keeps you logged in (a switch in Preferences turns it off). Intel 6th to 9th generation integrated GPUs start again, and so do AMD + NVIDIA laptops; integrated GPUs count their shared memory in the texture budget. The cache is cleared once at the first start of 0.9.3; your settings are kept.
[Read more](https://github.com/Aurora-Viewer/Aurora-Viewer/releases/tag/v0.9.3)

## 2026-11-06 — Updates, your computer, and clicks that land
Aurora now tells you when a new version is out, while it runs. Preferences › Language and updates › Remove Aurora from this computer takes away what Aurora registered outside its own folders, then lists the folders left. The web and media pack downloads only once you agree. Clicks go through alpha textures and invisible prims like the Legacy Viewer, and Shift or Ctrl + right click extends the build selection.

## 2026-11-06 — Camera, inventory and fixes
The camera limits are back on by default, like the Legacy Viewer: a card at login says where to turn them off. The teleport history survives a crash, and neither writing the log nor turning voice off can freeze the viewer. Terrain edges meet the next region, profile pictures stay sharp, Favorites lists starred items and the hover popups of the inventory have a switch. Thanks to ClifHofmann, chrzang, circuskid, emmelinedevere, Gaalrauch, iGao, KanikaBreed, Marakiinfinity, Odessa-Draekavik and xWynt3r.

## 2026-11-05 — Aurora 0.9.2: your chat history comes along, and a fullscreen mode
Preferences › Chat › History imports your conversations from the Legacy Viewer and the other approved viewers that write the same files. The messages join your Aurora history in time order, each login adds only what is new, and the other viewer's files are only read. Preferences › Graphics gets a borderless fullscreen mode, and Alt+Enter turns it on or off at once, without a restart: a clean frame for videos, and the window comes back exactly as it was.
[Read more](https://github.com/Aurora-Viewer/Aurora-Viewer/releases/tag/v0.9.2)

## 2026-11-05 — Smoother motion, lighter clicks
With TAA, moving avatars, animesh and objects no longer smear: the anti-aliasing follows each pixel's own motion. Ambient occlusion now runs at half resolution in a 1440p window too, for about 14 % more frames per second in a crowd with the same image. In busy regions a click on the world costs a tenth of what it did, and the hover cursor skips the worn rigged meshes it can never land on. Escalators and other looping keyframed objects loop without sliding back.

## 2026-11-05 — Building, graphics card and fixes
The texture eyedropper and local textures are back in the build and appearance editors, face drops flash and reach transparent faces, and the Legacy build window shows the link and face number. OBS Studio's Game Capture stays out unless you allow overlays, a window behind pauses its 3D while another program keeps the card busy, and Photo quality turns off after a crash. Stretched triangles on hair and plants are gone. Thanks to circuskid, Gaalrauch, KanikaBreed, Marakiinfinity and Odessa-Draekavik.

## 2026-11-04 — Aurora 0.9.1: steadier logins, teleports and outfits
A region slow to confirm your arrival no longer ends the session, and a refused two-factor code no longer locks the login out of the code field. Outfit changes send one appearance request at a time, for the newest version, so products and RLVa that change the outfit quickly no longer drown your appearance in failed updates. A key the viewer never saw pressed no longer makes your avatar jump or fly up after you switch windows. Imported Windlight skies save to the inventory.
[Read more](https://github.com/Aurora-Viewer/Aurora-Viewer/releases/tag/v0.9.1)

## 2026-11-04 — Photo roll, projector shadows, smoother busy places
Photo mode keeps the session's last 10 photos in a small stack in the viewfinder: look through them, then upload, e-mail or delete one. Choose how many projectors cast shadows, from 1 to 16, in Preferences › Graphics › Light (2 by default). In busy regions, objects that spin or move by themselves are placed by the graphics card, flexible prims out of view hold their mesh, and the frame's buffer writes go out together: more frames per second, the same image.

## 2026-11-04 — Building, avatars and crash reports
Paper lanterns and other tortured spheres keep their shape, Legacy construction gets Tilt, Radius and Revolutions back, and objects rezzed from the inventory can be edited and focused right away. Worn objects follow animations that move their attachment point, an avatar hidden by an animation leaves the screen as on the Legacy Viewer, and the Poser works on every shape. Crash reports after a lost graphics card say what was in play. Thanks to anne-skydancer, Gaalrauch, Marakiinfinity and Odessa-Draekavik.

## 2026-11-04 — Aurora 0.9.0: OpenSim and Hypergrid, for real
On OpenSim grids your avatar now bakes and is seen as you dress it, the land of large regions goes past 256 m, and the inventory, outfits and profiles save on grids without AIS. Buy and prices use the grid's own currency and money symbol. After a Hypergrid teleport Aurora follows the grid you are on, asks before a trip that would leave worn items behind, and offers to add an unknown grid from a link.
[Read more](https://github.com/Aurora-Viewer/Aurora-Viewer/releases/tag/v0.9.0)

## 2026-11-04 — Snap to objects, place on surface, holograms
Building gets three placement aids. Set Snap to Objects and the selection goes flush against nearby objects, purple guides on the faces that meet, never pushing through them. Press P to place it on any surface or on the ground, previewed as a hologram. A hologram can also show the object you move, or where it started (Preferences › Aurora › Behavior › Building). Stretch Textures holds while resizing, and the camera controls roll the view.

## 2026-11-04 — The Legacy toolbar, complete
The toolbar's Quick preferences, Web browser and Voice buttons open their Legacy Viewer windows: the settings changed most often, a built-in browser on a home page, the voice controls. Statistics, Picks, Beacons, Snapshot to disk and Show Friends Only join them, a button lights while its window is open, and the Legacy Viewer's missing menu shortcuts are in. Me › Fly override (Ctrl+Alt+V) lets you fly where flying is off, with a word of caution first.

## 2026-11-04 — Inventory, chat and groups
Give an inventory folder a colour of your own, seen by you only, and back an outfit up to a JSON file to restore it later. Me › Experiences lists your experiences and what their scripts did. Wear a group title chosen by region, and log in straight at a favourite landmark. The chat gets its own text size, a long IM goes out as several messages instead of being cut, and chat times follow the clock's 12 or 24 hours.

## 2026-11-04 — Steadier on your graphics card
Intel Iris Xe starts again, and Aurora shares the graphics card fairly with other viewers and programs. Photo quality climbs to what video memory holds instead of losing the card, and world textures can be capped at 512, 1024 or 2048 pixels. Links, logs and simulator connections are hardened after a security review by Butter Roux, thank you! Thanks also to Aznlilly, anne-skydancer, circuskid, ddagetracker, emmelinedevere, Gaalrauch, iGao, jumpinmimi, KanikaBreed, Marakiinfinity, Memekiller, Odessa-Draekavik, vexxbyte, voidel, wolfosbournesl, xWynt3r and zdub0is.

## 2026-11-02 — Aurora 0.8.5: immersive sound and VR, both experimental
World sounds now go dull behind a wall or inside a closed box, pass behind your head with headphones (HRTF), and ring like the place you stand in: dry in the open air, a long soft tail in a large hall. Three switches in Preferences › Sound; HRTF and reverb are off by default. With an OpenXR headset, the headset button of the bottom toolbar shows the world in stereo, from your avatar's eyes or the camera. Viewing only for now: Esc brings the desktop back.
[Read more](https://github.com/Aurora-Viewer/Aurora-Viewer/releases/tag/v0.8.5)

## 2026-11-02 — Quicker starts, smoother busy places
Aurora keeps its own cache of compiled graphics pipelines, so a start no longer waits twenty seconds on a black window when the driver emptied its own. On 4K screens ambient occlusion costs about a third of what it did, textures stay sharp after a teleport into a busy region, a viewer window behind gives its video memory back to the one in front, and fast cards get back the frame rates of 0.8.3 in busy places.

## 2026-11-02 — Stars, portals and building
Choose where on Earth the real night sky is seen from, and how bright its stars shine. Drag a place from Favorites, Places, the teleport history or search into the world to open a portal. A selection rectangle takes only what it surrounds, objects rez with your active group, BOM textures go in every PBR map, and grayscale PNG textures upload. An experience you allow can now change your sky and water.

## 2026-11-02 — Outfits, shopping and Linux
Pick several items in an outfit or in Worn and wear or take them off at once; Remove from current outfit works on any folder. Vendors sell on one click, and a short balance opens Buy L$. Clear the notification history, see a friend go on or offline in their open conversation, and assign gamepad buttons by pressing them. On Linux, a second viewer keeps the first one's web pages, and the safe graphics are offered after a GPU problem.

## 2026-11-02 — Steadier
Your logs and settings survive a hard reset of the PC, About Land saves its Allowed and Banned lists, the water no longer turns white under some region skies, and photos wait for a busy graphics card instead of failing. Thanks to anne-skydancer, circuskid, Gaalrauch, KanikaBreed, lambskin, Marakiinfinity, Odessa-Draekavik, vexxbyte, voidel, wolfosbournesl and zdub0is.

## 2026-10-31 — Aurora 0.8.4: copy anything, build in a crowd
Click a name in a profile to copy it, and select the text of a greyed-out field to copy it too: a no-modify object's name, a parcel's media address. Outfits copy their item names and creator credits. In a busy sandbox, the build tools can select only your own objects, or only the ones you may move, and the editor has a Legacy layout. Ctrl+Shift+H takes you home.
[Read more](https://github.com/Aurora-Viewer/Aurora-Viewer/releases/tag/v0.8.4)

## 2026-10-31 — Light for photos, LiveMesh for Blender 3
A light button above the viewfinder's shutter opens personal lighting, and the sky, water and day cycle can change for the photo only, or stay after it. LiveMesh syncs textures in Blender 3.x colour spaces, and accessories rigged to an Avastar body dev kit sit where an upload puts them.

## 2026-10-31 — Inventory, Mac and Linux
Items you receive can be accepted automatically, with a receipt. Inventory folders get their own controls and a separate search row, and Legacy Viewer contact sets can be imported. On the Mac, Command works as Control and shortcuts read ⌘ ⌥ ⇧. Ctrl+H opens nearby chat and Ctrl+Shift+W closes every window. On Linux, secondlife:// links open in the running viewer.

## 2026-10-31 — Steadier
A graphics card reset closes the viewer cleanly, photos no longer freeze it when video memory runs short, and texture memory stays in check in busy places. No more takeoff in a no-fly parcel, and the parcel you left no longer lends its rules after a teleport. Worn animesh stand up straight. Thanks to Gaalrauch, Marakiinfinity, circuskid, Aznlilly, DarlCat, voidel and seilimited-ship-it.

## 2026-10-29 — Aurora 0.8.3 runs natively on the Mac
Aurora now runs natively on Apple Silicon Macs, drawn through Metal: every release carries a macOS application to drag into Applications. Web pages and video in the world work there too: the first time they are needed, the viewer downloads its media plugins by itself. The install guide is in docs/MACOS.md.
[Read more](https://github.com/Aurora-Viewer/Aurora-Viewer/releases/tag/v0.8.3)

## 2026-10-29 — Messages that matter, in front of you
A message an estate manager sends to the region now shows as a card in the middle of the screen, like a friendship offer, and is kept under the bell and in nearby chat. A group notice shows its group, its insignia and its attachment, ready to save. The L$ you pay a resident now appear in your conversation with them, like the L$ they pay you, and stay there when you open it again.

## 2026-10-29 — For photos and creators
In the photo viewfinder, Shift + click keeps every avatar of a group shot sharp, your HUDs answer clicks, and the Poser button poses the avatar over the shot. The poser now works with every shape, asks before closing loses a pose, and undoes with Ctrl+Z. LiveMesh previews an animation on a saved local model, Bento tails and wings included.

## 2026-10-29 — Inventory, graphics and Linux
Hover an inventory item to preview its picture, and build an advanced search from visual rules, saved for later. Save your graphics settings as named profiles. On Linux, sound goes through PulseAudio or PipeWire, notifications come from the desktop, and the dock shows Aurora's new howling wolf. Thanks to circuskid for Linux, logue for the native Japanese proofreading, and voidel, our newest contributor.

## 2026-10-29 — Your ideas, on the feedback board
Feature requests now live on our feedback board: suggest an idea, vote for the ones you want most, and follow their progress. The requests made on GitHub have been moved there.
[Open the feedback board](https://feedback.aurora-viewer.org)

## 2026-10-29 — Aurora 0.8.2 now speaks Japanese, 日本語
Aurora is now available in Japanese: choose 日本語 in Preferences › Interface › Language, or let the system language pick it on a Japanese Windows. Every message is translated with Second Life's official Japanese terms, the interface draws with Japanese fonts, and dates read the Japanese way.
[Read more](https://github.com/Aurora-Viewer/Aurora-Viewer/releases/tag/v0.8.2)

## 2026-10-29 — PBR terrain, and the HDR switch
Regions whose ground uses PBR materials now render with them instead of a flat grey, as in the Legacy Viewer, and estate managers see a ground-cover change on the terrain before they apply it. Preferences › Graphics › Image also gets the Legacy Viewer's HDR and Emissive switch, on by default, for those who prefer the older look.

## 2026-10-29 — Chat and name tags
Ctrl+Shift+Enter starts a new line in a message, and pasted paragraphs keep their shape. Your own objects' script errors show in red in the nearby chat instead of popping a window. Three dots bounce on the picture of a resident typing to you, your friends' name plates take the friends' colour, and a right click on your L$ balance hides it while you stream.

## 2026-10-29 — For creators and outfits
Default permissions, at the bottom of the inventory's New menu, apply to what you create and import. Drag your tattoo, alpha and universal layers into the order you want, drop inventory textures onto the swatches of the appearance and material editors, and make the edit handles bigger. 2K texture uploads work again, and Appearance › Worn lists what a script attached to you for a while.

## 2026-10-29 — Also in 0.8.2
Text can now grow to twice its size, in Preferences › Appearance. Hold the back arrow of the top bar to list every place of the session. The window layout is saved shortly after each change, even if the viewer crashes; saved outfits can be deleted again, ridden vehicles swing smoothly through collisions, clickers inside a mesh body answer every click, and the wind no longer crackles. Thanks to circuskid, our newest contributor, Linux no longer jumps to Aurora at each message.

## 2026-10-27 — Aurora 0.8.1: avatars out of the cloud sooner
Downloads now keep pace with your connection: in a busy region, avatars come out of the cloud about three times sooner, already textured, and the region loads about twice as fast with an empty cache. Your own outfit arrives whole: every attachment is worn at login and at each outfit change.
[Read more](https://github.com/Aurora-Viewer/Aurora-Viewer/releases/tag/v0.8.1)

## 2026-10-27 — Freeze the world, save your shots
In the photo mode, freeze the world: every avatar, animation and moving thing holds still while your camera stays free. Save a whole shot as a camera preset (where the camera stands, the lens, the filters and the light) and bring it back in one click. The shutter is now heard around you, as in the Legacy Viewer, unless you choose quiet snapshots.

## 2026-10-27 — For builders
Build › Edit particles shapes a particle effect on your prim, live, and puts the matching script into it. World › Animation explorer shows who plays an animation on you, previews it, and stops or blocks it. And the right-click menus of objects, land, avatars, attachments and HUDs can now be arranged as you like.

## 2026-10-27 — Also in 0.8.1
Hold Alt in mouselook to free the cursor, and pick your crosshair. Arabic and Hebrew now read right to left, and an L$ payment shows as a card in the payer's conversation. Group your outfits in folders in Appearance. Immersive mode can hide your HUDs, and Alt+Shift+H shows or hides them anywhere. With RLVa on, the frame rate no longer drops, gaming mice stay connected, and Linux closes cleanly under Wayland.

## 2026-10-26 — Aurora 0.8.0: filters and effects for your photos
The photo mode gets 66 filters and effects that show live in the viewfinder: the Legacy Viewer's 30 filters and 36 of Aurora's own, from film grain to VHS, plus your own .cube LUTs. Stack up to six, each with its own sliders, and compare before and after. Isolate your avatar, or the avatars and objects you click, on a transparent background: a PNG ready to use.
[Read more](https://github.com/Aurora-Viewer/Aurora-Viewer/releases/tag/v0.8.0)

## 2026-10-26 — Portals, and only your parcel
Open a portal to a place, or drop a landmark into the world, and walk through it to teleport. Other Aurora users see it too: your friends by default. You can also hide what stands outside the current parcel, and immersive mode hides the whole interface but the chat, for photos and events.

## 2026-10-26 — Chat that writes with you
A spell checker with suggestions, auto-replace if you like, @ to mention someone, and / in nearby chat to find a gesture. A thumbnail per conversation waits beside the chat bar, and nearby chat can rise in bubbles above heads. Bluesky and X posts, short videos and animated WebP now preview in the conversation.

## 2026-10-26 — Inventory and outfits
Your whole inventory loads in the background from login on. Open a second inventory window to sort between folders, resize its columns, and give items by dropping them on a profile or an avatar. Copy a box's contents into your inventory or wear them straight away, and edit your skin, hair, eyes, tattoos, alphas and clothes.

## 2026-10-26 — Also in 0.8.0
Sound Explorer finds which object plays a sound, Quickjump skips the jump and landing delay, and Help › Grid status shows the grid's incidents. Name tags mark the Aurora team and Linden Lab staff, and the viewer stays within the video memory your graphics card grants it.

## 2026-10-25 — Aurora 0.7.5: tools for creators
LiveMesh shows your Blender mesh in the viewer while you edit it, with nothing uploaded: rigged meshes on your avatar, static meshes and linksets as local objects you can place, move or attach. The new Poser poses you, your animesh or nearby avatars who agree to it, bone by bone, for photos, with a pose library and BVH export.
[Read more](https://github.com/Aurora-Viewer/Aurora-Viewer/blob/main/docs/BLENDER_LIVE.md)

## 2026-10-25 — Find any object around you
Area search, in the World menu or as a toolbar button, lists the objects rezzed around you and filters them as you type: name, description, owner, creator, group or UUID, the current parcel or the neighbouring regions, for sale, physical, phantom, temporary or by distance. Sort the results, then inspect an object, zoom to it or copy its UUID.

## 2026-10-25 — Conversations your way
Pop a conversation out into its own window, or just drag it out of the list. Links to Gyazo, Flickr, Imgur, Lightshot and Reddit pages show their picture, the YouTube player keeps each video's shape, and a GIF picker sits next to the emoji button once you add your own free key. Messages from objects can take one small, quiet line.

## 2026-10-25 — Easier on the eyes
A new Accessibility page gathers the settings that help, with a button on the login screen. Try the high-contrast palette, the OpenDyslexic font, volume sliders that follow your ear, or pick your own accent, background and text colours.

## 2026-10-25 — Fixed thanks to your reports
Laptops with two graphics cards now use the dedicated one, Overwolf no longer stops the viewer at startup, layered hair is no longer see-through, and gestures reach your HUDs on negative channels. Your avatar stands up when you teleport from a seat, and the Appearance window stays fast with thousands of outfits. Logging in again right after a logout or a crash now counts down a few seconds instead of hanging.

## 2026-10-22 — RLVa, when you want it
Since version 0.7.4, Aurora understands RLVa: the restrictions, exceptions and commands that scripted outfits, furniture and experiences rely on. It stays off until you turn it on in Preferences › Protections, and turning it off releases everything at once.
[Read more](https://github.com/Aurora-Viewer/Aurora-Viewer/blob/main/docs/RLVA.md)

## 2026-10-22 — Aurora on Linux
Every release now comes with a Linux build too, with its own install guide.
[Install guide](https://github.com/Aurora-Viewer/Aurora-Viewer/blob/main/docs/LINUX.md)

## 2026-10-22 — Your settings, safe
Back up your settings to a file and restore them in one click. After a crash, the crash window can send its report as a GitHub issue. And your Alchemy AO sets can now be imported into the built-in AO.

## 2026-10-21 — Livelier conversations
Pictures and YouTube videos preview under their links, messages made only of emoji show large, and a button brings you back to the latest messages. Keyword alerts tell you when someone mentions what matters to you. Alt + arrows go round the conversations, Ctrl+W closes them.

## 2026-10-21 — Preferences you can search
The preferences are now arranged in sections and tabs, with a search across every setting. Also new: a tracker of the L$ you receive and pay, tooltips over objects, avatars and land, a media filter that asks before a new site plays, and optional anti-spam protections.

## 2026-10-20 — A photo mode like a real camera
The photo mode opens full screen under a viewfinder: autofocus that finds faces, lens rings for focal length, aperture and exposure, focus peaking, a live histogram, a camera pad and a self-timer. Photos are 4K by default, up to 8K.

## 2026-10-19 — Places, inventory and search, redrawn
Places shows your favourites, landmarks and history in one list with a preview. The inventory keeps its familiar tree and widens into an explorer. Search finds residents, places, events, groups and classifieds from one field. Each window keeps its Legacy layout in Preferences › Aurora.

## 2026-10-19 — Avatars that feel alive
Body physics, facial expressions, hand poses and breathing from animations, and optional foot placement on the terrain. Also new: a resource monitor for power users, and experimental water effects with waves, foam and caustics.

## 2026-10-17 — Light that grounds every scene
GTAO ambient occlusion, sun contact shadows and specular anti-aliasing arrive, with graphics levels that match them to your machine. Region owners get the complete Region / Estate window, in the Aurora layout or the classic eight tabs.

## 2026-10-08 — Welcome to Aurora Viewer
A modern Second Life client written in Rust on Vulkan: PBR rendering, shadows, water and reflections, and a fully customizable Aurora interface.

## 2026-10-08 — WebRTC voice and audio
Region voice chat, push-to-talk, animated voice dot in name tags, parcel music and fine-grained volume controls.

## 2026-10-08 — An interface that fits you
Rebindable keyboard and mouse shortcuts, remembered window layout, notifications, SLURLs in the navigation bar.
