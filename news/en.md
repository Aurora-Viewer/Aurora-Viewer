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
