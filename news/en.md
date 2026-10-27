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
- Each language has its own file (fr.md, en.md): add the news item to both.
  A language without a file shows en.md.
- Full instructions (in French): HUMANS.md, "Publier une actualité".
-->

# Aurora Viewer news

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
