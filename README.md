<div align="center">

<img src="assets/branding/logo-loup-couleur.svg" alt="Aurora Viewer" width="180">

# Aurora Viewer

**A modern Second Life viewer written in Rust.**

[![CI](https://github.com/Aurora-Viewer/Aurora-Viewer/actions/workflows/ci.yml/badge.svg)](https://github.com/Aurora-Viewer/Aurora-Viewer/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/Aurora-Viewer/Aurora-Viewer?include_prereleases&color=8B5CF6)](https://github.com/Aurora-Viewer/Aurora-Viewer/releases)
[![License: GPL v3+](https://img.shields.io/badge/license-GPL--3.0--or--later-4F46E5)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-stable-5EEAD4?logo=rust&logoColor=white)](https://www.rust-lang.org)
[![Platform](https://img.shields.io/badge/platform-Windows-070B1F)](#build)

[Français](#en-français) · [Build](#build) · [Test switches](#test-switches) · [Contributing](#contributing)

</div>

![Aurora Viewer in Second Life](docs/screenshots/scene.jpg)

Aurora Viewer is a Second Life client built from scratch in Rust, with a
modern GPU renderer (wgpu on Vulkan) and a clean, flat interface. It follows
the behaviour of [Firestorm](https://www.firestormviewer.org) — same protocol,
same rules, same defaults — while rethinking the user interface.

> **Status:** early development (0.x). Usable for exploring, chatting and
> testing; many features are still in progress — see [TASKS.md](TASKS.md).

## Highlights

- **Rendering** — wgpu / Vulkan, bindless textures, multi-draw-indirect,
  PBR and legacy materials, shadows, reflection probes, water, glow,
  particles, GPU occlusion (experimental).
- **Avatars** — skeleton and Bento animations blended like Firestorm,
  server bakes, shape, rigged mesh, complexity limits, impostors, look-at.
- **World** — prims, sculpts, mesh with LOD, terrain, EEP environment,
  world sounds, parcel music, media on prims.
- **Social** — local chat, IMs and groups, people list, blocking,
  notifications, voice (WebRTC).
- **Interface** — flat Aurora theme, Phosphor icons, mini-map and world map,
  inventory, preferences, debug overlays.

![Login screen](docs/screenshots/login.jpg)

## Build

Requirements: Windows 10/11, [Rust](https://rustup.rs) (stable, selected
automatically by `rust-toolchain.toml`), a GPU with Vulkan support.

**Development environment:** create a folder for the project, clone this
repository into it, then double-click **`aurora-tools.cmd`**:

```bat
git clone https://github.com/Aurora-Viewer/Aurora-Viewer aurora-viewer
```

The tools check the environment and offer to **repair** it: they install
whatever is missing (GitHub CLI, the Visual Studio C++ tools, Rust), clone
the Firestorm sources next to the repository, and download the emoji font,
the Rust toolchain and the crates. Then, driven by the keyboard: release /
dev / debug viewers, demo, agents' tasks, disk, logs, live pull requests with
Windows notifications, releases (see
[HUMANS.md](HUMANS.md)).

Without Git yet, this command (in a command prompt, in the project folder)
installs everything, Git included:

```bat
powershell -NoProfile -ExecutionPolicy Bypass -Command "$f = Join-Path $env:TEMP 'aurora-setup.ps1'; irm https://raw.githubusercontent.com/Aurora-Viewer/Aurora-Viewer/main/scripts/setup.ps1 -OutFile $f; & $f"
```

Or by hand, to build the viewer only:

```powershell
git clone https://github.com/Aurora-Viewer/Aurora-Viewer aurora-viewer
cd aurora-viewer
./scripts/fetch-assets.ps1          # emoji font (too large for git)
cargo build --release -p aurora-viewer
./target/release/aurora-viewer.exe
```

Run the same checks as the CI with `./scripts/check.ps1` (rustfmt, clippy
with warnings as errors, tests). Day-to-day development uses the default
profile (`cargo run -p aurora-viewer`): it behaves like the release build
without its slow link-time optimization. `./scripts/build-release.ps1`
builds GitHub's latest `main` with `--release` into `..\RELEASE\`.

### Command line

| Argument | Effect |
|---|---|
| `--title <text>` | Window title "Aurora Viewer - <text> (Dev)" (the build profile ends the title: Dev, Release, Debug); log file `aurora-<text>.log` (`aurora-demo-<text>.log` in demo mode) |
| `--fps-limit <fps>` | Abaisse le plafond de cette instance (1–60 images/s), sans enregistrer les préférences. Les anciennes valeurs jusqu'à 500 sont ramenées à 60 ; les captures et les sessions en ligne respectent le même plafond. |
| `-h`, `--help` | Show the help |

Le viewer est plafonné à **60 images/s** dans tous les profils, en démo,
en capture et en ligne. Préférences › Graphismes permet de choisir un plafond
inférieur ; les anciens réglages illimités sont migrés automatiquement.

Dans l'inventaire, **Filtres** sélectionne les types, permissions, liens,
créateur et ancienneté ; **Préférences** règle le tri, les onglets, les dossiers
inclus dans la recherche et le double-clic. Les dossiers système viennent
en premier ; les éléments sont triés du plus récent au plus ancien et les
dossiers par nom. Ces options sont aussi dans Préférences › Interface.
La recherche peut porter sur le nom, la description, le créateur ou l'UUID ;
`+` combine des termes, `"mot"` cherche un mot exact. Les éléments restent dans
leurs dossiers, y compris dans Récent, Porté, les recherches et les vues filtrées.
Réduire / Développer agit sur ces dossiers ; le sélecteur des champs est à droite
de la saisie. « Afficher aussi les dossiers sans résultat » ajoute les dossiers
vides à la vue, les dossiers parents des résultats étant toujours présents.
La liste défile aussi horizontalement quand un nom dépasse la largeur disponible ;
les noms longs ne bloquent pas la réduction de la fenêtre. Les commandes et onglets
s'adaptent aux petites largeurs. À la taille minimale, le bord déplacé s'arrête
sans déplacer le bord opposé de la fenêtre.
Le chemin et la date restent disponibles en info-bulle. Récent part de la dernière déconnexion
(24 h au premier lancement). Les filtres restent propres à chaque fenêtre ;
« Garder par défaut » les mémorise pour les prochaines ouvertures.

### Test switches

The offline **demo mode** simulates a server locally (a plaza, avatars,
objects, chat, notifications…): no connection, no account. It keeps its own
settings and cache (`…\config\demo`, `…\cache\demo`).

| Variable | Effect |
|---|---|
| `AURORA_DEMO=1` | Offline demo mode |
| `AURORA_DEMO_INVENTORY=sort\|filters\|preferences\|recent\|worn\|filtered\|large\|long\|long-filtered\|resize-left\|resize-right` | Tri système / date, filtres, préférences, arborescence Récent / Porté, filtre excluant les objets ou recherche de 1 500 éléments. `long` et `long-filtered` montrent les noms longs et le défilement horizontal dans l'arbre complet ou filtré. Avec une capture, `resize-left` / `resize-right` tirent le bord correspondant au-delà de la taille minimale puis relâchent la souris. Données synthétiques uniquement ; aucun benchmark de rendu. |
| `AURORA_CAPTURE=<file.png>` | Save a capture of the frame (`AURORA_CAPTURE_FRAMES`, default 240; ~620 to pass the loading fade). Several frames separated by commas (`2500,2600`) save one file each, `<file>-<frame>.png`. The background frame cap ("Limiter hors focus") is ignored |
| `AURORA_CAPTURE_EXIT=1` | Quit after the capture |
| `AURORA_DEMO_CAM="yaw,pitch,dist"` | Camera heading offset and pitch around the avatar (radians, positive pitch looks down) and distance (meters) |
| `AURORA_DEMO_PARTICLES=wind\|smoke\|both\|legacy` | Motorcycle wind streaks and smoke script parameters through compressed updates, in front of a dark panel; extended glow format (`legacy`: wind without glow), default soft particle texture offline |
| `AURORA_DEMO_CAMERA=alt,x,y\|pan,x,y\|zoom,x,y\|tag\|ml\|wheel\|fly\|sit` | Camera scenario logged as `demo camera`: Alt+click at (x, y) then a drag and a walk back, a press on our own name tag then a drag (steering), mouselook in / out, wheel, flight lag, sitting on a turning seat with a sit camera and an animation started by a linked prim (frame 300), then standing and requesting its stop (frame 700; `camera/demo.rs`) |
| `AURORA_DEMO_ACTIONS=1\|sit\|buy\|pay\|none\|touch\|disabled\|ignore\|ignore-build\|open\|play\|pause\|open-media\|open-media-playing\|zoom\|grab\|buy-confirm\|pay-confirm\|linked-touch\|linked-sit` | Curseurs et clics d'objet comme Firestorm. `1` : tous les objets pour un test manuel ; modes nommés : cible isolée, survol et clic par le chemin habituel. NONE / TOUCH envoie appui, déplacement et relâché ; Grab simule le déplacement physique. IGNORE traverse un écran visible et ouvre Buy derrière ; DISABLED arrête le clic sans toucher ; `ignore-build` sélectionne cet écran en construction. Open affiche trois éléments de contenu simulés. Play lance une page HTML locale ; Pause et `open-media-playing` injectent un état de lecture simulé avant le clic. Open Media simule l'ouverture du navigateur externe. Zoom cadre le cube. `*-confirm` confirme la transaction simulée. `linked-touch` / `linked-sit` : racine creuse avec Touch et siège enfant Sit, boîtes englobantes recouvrantes ; vérifie la prim effectivement visée. Aucun L$ réel ni connexion à une grille. |
| `AURORA_DEMO_POS="x,y"` | Start position |
| `AURORA_DEMO_ACTIONS=pay-layout\|pay-hidden\|pay-large\|buy-original\|buy-contents\|buy-empty` | Fenêtres de transaction : quatre montants à quatre chiffres, champ libre et boutons masqués, montant maximal ; achat d'un original, d'une copie (`buy`) ou du contenu, liste des éléments inclus et permissions du prochain propriétaire ; `buy-empty` propose un contenu non vendable et désactive l'achat. |
| `AURORA_DEMO_DIALOG=12\|4\|long` | Menu llDialog de douze réponses sur trois colonnes, rangées du bas vers le haut comme Firestorm ; quatre réponses pour vérifier la dernière rangée incomplète, ou libellé long tronqué avec texte complet au survol. Boutons Bloquer et Ignorer sous le menu. |
| `AURORA_DEMO_TP=1` ou `"x,y,z"` ou `remote` | Téléportation après le fondu du chargement initial (image 240 au plus tôt) : arrivée locale immédiate sans écran de chargement ; `remote` simule la progression entre régions |
| `AURORA_DEMO_CHATCMD="calc 2+2;rolld 2 20"` | Lines typed in the chat bar at frame 240, separated by `;` (chat bar commands: `calc`, `rolld`, `gtp`…) |
| `AURORA_DEMO_SIT=n` | Click the toolbar sit button n times (frames 240, 300, 360…) |
| `AURORA_DEMO_KEY=down\|up\|left\|right` | Hold an arrow key from frame 235 (movement, body orientation) |
| `AURORA_DEMO_ANIM_LOOP=1` | Loop with a missing first interval, changing sequence every 120 frames and stopping/restarting at frames 960/1020 every 1200 frames |
| `AURORA_DEMO_ANIMESH=1` | Animated root mesh, linked mesh signaled by a child, and worn animesh with independent custom skeletons and alpha shadows |
| `AURORA_DEMO_MMO="x,y[,1\|2]"` | Left press on the avatar then right button held (mouse steering); `1`: right arrow instead, `2`: double right click (run) |
| `AURORA_DEMO_RCLICK="x,y"\|tag` | Right click (context menu) at (x, y) at frame 225, or on the name tag of the nearest other avatar at frame 600 |
| `AURORA_DEMO_POINTER="x,y[,r][;x,y…]"` | Move the interface pointer to these window pixels from frame 300, one point every 60 frames (hover states, sub-menus); `,r` right-clicks the interface there (menus of lists and names) |
| `AURORA_DEMO_HUDS=1\|touch\|zoom\|hidden\|media\|menu\|edit\|edit-zoom\|drag\|drag-zoom\|drag-ignore\|drag-ctrl\|drag-key` | Huit HUDs texturés sur les points 31–38, avec texte flottant et bouton enfant translucide ; HUD d’un autre avatar caché. `touch` simule appui, déplacement et relâché sur l’enfant (images 720–900), puis change la couleur du HUD ; `zoom` réduit à 50 %, `hidden` masque les HUDs, `media` affiche la page locale de test sur le HUD central. `drag` saisit l’enfant avec ALT (image 720), déplace tout le HUD, relâche ALT avant le clic (image 800), puis enregistre la position au relâchement gauche (image 840) sans toucher de script ; `drag-zoom` fait de même à 50 %, `drag-ignore` sur un enfant avec action IGNORE ; `drag-ctrl` utilise Ctrl et `drag-key` la touche B à la place de ALT. Les scénarios menu, edit et edit-zoom vérifient le clic droit sur l’enfant, Modifier et le déplacement du linkset par la poignée Y, aussi à 50 %. ALT + clic gauche maintenu sur la géométrie visible déplace le HUD sans ouvrir les outils ; main de saisie au survol et pendant le déplacement. Préférences › Pratique › HUDs : touche configurable (ALT par défaut), clic sur la touche puis saisie au clavier, Échap pour annuler et bouton Rétablir ALT ; choix enregistré et appliqué au survol comme au déplacement. Le droit Déplacer suffit, y compris sans droit Modifier. Inventaire de démo lié à la tenue actuelle pour tester le détachement et l’affichage dans l’inventaire. Menu Monde › HUDs : afficher / masquer, réduire / agrandir et taille normale. Aucun accès à une grille. |
| `AURORA_FPS_LIMIT=<n>` | Plafond utilisateur pour les essais (10–60 images/s ; valeurs positives ramenées à cette plage, valeur invalide ignorée). Le plafond commun de 60 s'applique toujours ; le plafond inférieur en arrière-plan reste applicable hors captures. |
| `AURORA_DEMO_HUDS=faces\|faces-turned` | Neuf panneaux HUD à une seule face visible : colonnes avant / arrière / matériau PBR explicitement double face, lignes opaque / translucide / masqué. La colonne arrière doit rester vide ; `faces-turned` retourne tous les panneaux de 180° : la colonne avant devient vide, l’arrière apparaît, le double face reste visible. Les libellés flottants restent visibles dans les deux sens. |
| `AURORA_DEMO_LOOKAT=1` | Eye tracking on (in memory only) and a remote look-at |
| `AURORA_DEMO_SOUND=1` | Audible world sounds (looped chime) and interface sounds (from the real sound cache when present, else a short tick per sound) |
| `AURORA_DEMO_CLOUD=1` | Loading clouds for avatars |
| `AURORA_DEMO_OCCLUSION=1` | Wall with hidden objects (occlusion test) |
| `AURORA_DEMO_TEXTURES=<n>` | Texture stress test: n small cubes (9000 for a non-number), each with its own texture of several sizes (one not a power of two), streamed low resolution first then full, as in a busy region |
| `AURORA_DEMO_CROWD=<n>` | Scene sync stress test: n extra avatars (40 for a non-number) playing the idle animation, each wearing eight attachment linksets of seven prims on bones all over the body (with `AURORA_DEMO_ANIMESH`, also the rigged demo mesh), as in a busy shop |
| `AURORA_DEMO_TEXTURES_CHURN=1` | With `AURORA_DEMO_TEXTURES`: a third of the cubes removed at frame 300 (their textures evicted 2 s later: freed layers, page compaction), then back at frame 700 with new textures (freed slots reused, streamed again); capture after frame ~1000 |
| `AURORA_DEMO_STREAM=<n>[,<wave>][,leave]` | Streaming hitch test: n textured objects (2000 for a non-number) arriving in waves after the loading fade, `wave` every 250 ms (100 by default, ~400 a second as on the grid; `wave` = n sends everything at once, as a teleport arrival). Each object has its own shape and its own texture, of the sizes of a grid region (mostly 512 and 1024), decoded on the background jobs at a quarter of its size, then at full size 1.5 s later. The log gives the end of the loading (`demo stream: fully loaded in … s`, with the uploads staged by the jobs / written by the main thread); with `AURORA_PROFILE=1`, read `stream`, `results` and the `s_*` parts in `max:`. With `,leave`, the region is left a second after the loading, as by a teleport: every object is removed at once and its texture, by then holding downloaded data of the size of its J2C file, is unused and due for eviction 2 s later (`demo stream: region left`, then `demo stream: textures evicted`); read `s_maintain`, `sync`, `s_sync_list` and `r_submit` in `max:` |
| `AURORA_DEMO_HOVER=1` | Hover cursor test: from frame 300 the cursor is swept over the window for 120 frames, then left still for 120, and so on (search of the object under the cursor and answers kept between frames, `scene/hover.rs`); read `hover` in the profile, and combine with `AURORA_HOVER_CHECK=1`, `AURORA_DEMO_ACTIONS=1` or a stress scene |
| `AURORA_DEMO_PLANAR=1` | Floor slabs with planar texture mapping (tiles must line up across slabs) |
| `AURORA_DEMO_PBR_OVERRIDE=1` | Two PBR slabs, one with a GLTF material override (4 × 4 repeats, tint) |
| `AURORA_DEMO_TEXANIM=1` | Texture animations (llSetTextureAnim) on two rows of panels in place of the alpha panels: smooth scrolling, 4 × 4 frame grid, ping-pong, rotation, scale (alpha masked: prepass and shadows), a cube animated on one face only, a legacy material (normal map follows) and a PBR face |
| `AURORA_DEMO_SKY=<gamma>` | Classic EEP sky (no reflection probe ambiance) with this sky gamma and a sunlight color above 1: legacy gamma and normalized object light as Firestorm |
| `AURORA_DEMO_DEBUG="bounds,culling,lights,probes,skeletons,alpha,wire,complexity,glow,glow_view,freeze"` | Debug overlays |
| `AURORA_DEMO_OPTIONS=<tab>` | Ouvre les préférences sur une section (`12` : Pratique, touche de déplacement des HUDs) |
| `AURORA_DEMO_KEYBOARD=system\|wasd\|zqsd\|fallback` | Use fresh movement defaults with the Windows layout, simulated QWERTY / AZERTY, or failed detection; combine with `AURORA_DEMO_OPTIONS=8` to inspect secondary bindings |
| `AURORA_DEMO_PERF=compact\|full` | Open the performance window in its compact or full view |
| `AURORA_DEMO_UI=<tab>` | Open people (tab), inventory and chat |
| `AURORA_DEMO_APPEARANCE=gallery\|outfits\|worn\|save\|edit\|menu\|duplicates` | Open Appearance with four synthetic outfits: gallery (right-click includes rename, favorites, image selection, save into the chosen outfit and move to Trash), outfit list, worn items, Save As dialog, outfit editor or an expanded outfit with an unworn object for right-click menus; `duplicates` replays folder responses with repeated links and originals from other parents; these operations stay offline; creation of new clothing / body parts is still unavailable |
| `AURORA_DEMO_INVENTORY=1\|folder\|object\|animation\|script\|note\|properties\|nocopy\|notransfer\|link` | Ouvre un inventaire synthétique et, sauf `1`, le menu de l’élément choisi ou ses propriétés. Dossier personnel, sous-dossier, tenues, objets, documents, texture, son, geste, ciel, matériau, lien et permissions variées ; créations et mutations simulées hors ligne. `clothes`, `body`, `settings`, `uploads` sont des alias de `folder`, à combiner avec `AURORA_DEMO_POINTER` pour survoler les sous-menus ; `animation-open`, `animation-properties`, `image`, `image-photo`, `image-picker` ouvrent les cinq panneaux de référence. Bibliothèque dans l’arbre, racine ouverte et libellés alignés à gauche. `image-photo-save` capture puis sauvegarde une vignette à la frame 2100 et ferme la fenêtre Photo en gardant l’éditeur Image ouvert. `folder-window` ouvre une fenêtre limitée au contenu du dossier personnel, avec son nom en titre et un filtre propre ; `folder-window-search` montre cette recherche limitée aux descendants. Les scénarios `rename`, `new-script`, `new-note`, `new-folder` montrent le renommage dans la ligne sans fenêtre (leurs captures simulent le focus du champ sans activer la fenêtre Windows) ; `multi-add`, `multi-detach`, `delete` montrent la sélection multiple et la confirmation de suppression. Les fichiers et photos chargés en démo restent hors ligne |
| `AURORA_DEMO_LOGIN=remembered\|empty` | Offline login screen with a synthetic saved-password marker or an empty password field (no grid or credential-store access) |
| `AURORA_DEMO_MAP=1` or `mini` | World map and mini-map |
| `AURORA_DEMO_NOTIF=1`, `AURORA_DEMO_STATUSMENU=1`, `AURORA_DEMO_NAVEDIT=1` | Notification list, status menu, location field |
| `AURORA_DEMO_CONV=1`, `AURORA_DEMO_TALK=1` | Group conversation, microphone on |
| `AURORA_DEMO_LAND=<onglet>[:owner]` | About Land on a tab (`general`, `reglement`, `objets`, `options`, `medias`, `son`, `acces`, `experiences`, `environnement` or its index); `:owner` makes the avatar own the parcel (controls enabled), else it belongs to a group without powers |
| `AURORA_DEMO_CONTACTS=amis\|groupes\|cercles\|detache\|ajout` | Contacts tab of Conversations on Friends, Groups or Contact sets (demo sets and aliases), the torn-off Contacts window, or the resident picker of « Ajouter... » |
| `AURORA_DEMO_PROFILE=loup\|nova\|friend\|self[:tab]` | A profile window (tab 0 2nd life, 1 feed, 2 picks, 3 classifieds, 4 1st life, 5 notes) |
| `AURORA_DEMO_PLACE=1\|lagune\|nordheim\|pinede\|faille\|inconnue\|repere\|historique` | « Lieux » on a place profile, as if a place link were clicked: `1` Loup Violet's home on Aurora Démo (the About Land parcel), `lagune` a Moderate parcel, `nordheim` an Adult group-owned one, `pinede` a parcel without name, description nor snapshot, `faille` RemoteParcelRequest answering HTTP 404, `inconnue` a region nobody answers (error after 10 s); `repere` a landmark's profile, `historique` a teleport history entry's |
| `AURORA_DEMO_PLACES=favoris\|reperes\|historique` | « Lieux » on a tab: the demo's Favorites and Landmarks folders (with a sub-folder) and a teleport history over several months |
| `AURORA_DEMO_PLACE_WINDOW=1` | With `AURORA_DEMO_PLACE`: the standalone place windows instead of « Lieux » (option « Repères et profils de lieux », not saved) |
| `AURORA_DEMO_FEED=<url>` | Profile "Flux" tab on this page (the username and `/?feed_only=true` are appended, a `data:` URL can comment them out) |
| `AURORA_DEMO_DISPLAYNAME="name"`, `AURORA_DEMO_DISPLAYNAME_ERROR=…` | Display name change (simulated) |
| `AURORA_DEMO_MEDIA=<url>\|1`, `AURORA_DEMO_PARCEL_MEDIA=<url>\|1`, `AURORA_DEMO_MEDIA_CLICK="x,y[,x2,y2]"`, `AURORA_DEMO_MEDIA_CLICK_FRAME` | Média sur prim ou parcelle ; clic de focus à l’image 700 par défaut, deuxième clic 60 images après, clic facultatif sur le champ 120 images après, puis saisie « aurora ». `CLICK_FRAME` change l’image de départ pour laisser charger le plugin ; fonctionne aussi avec `AURORA_DEMO_HUDS=media`. |
| `AURORA_DEMO_MUSIC=<url>\|1` | Parcel music on the demo parcel (`1`: an unreachable placeholder); it waits for the radio button |
| `AURORA_DEMO_BUILD="mode,x,y[,part,dx,dy]"`, `AURORA_DEMO_BUILD_FRAME` | Build tools script (mode: move, rotate, stretch, face, align, grab, focus, create, land, select) |
| `AURORA_DEMO_BUILD_TAB=general\|object\|features\|texture[:pbr\|bp\|media]\|contents` | Build floater tab shown by the build script; `+weights`, `+grid`, `+media` also open those floaters |
| `AURORA_DEMO_BAN=1` | Ban lines |
| `AURORA_DEMO_RESTRICTED=1` | Parcel that forbids everything (red parcel icons, damage on, 72 % health) |
| `AURORA_DEMO_LSL_BRIDGE=1` | Firestorm LSL bridge messages (hidden) between two owner-say lines |
| `AURORA_DEMO_TOD=0..4` | Time of day |
| `AURORA_DEMO_EEP_PARCEL=1\|parcel\|default\|stale` | EEP answers replayed like the grid (`demo_eep.rs`): a dusk region day then, 0.4 s later, the agent's parcel without a day (`1`: the dusk stays), with its own noon day (`parcel`: 5 s crossfade), a region without a day (`default`: Firestorm's default day asset, the built-in default sky offline) or answers for another parcel and another region (`stale`: ignored) |
| `AURORA_DEMO_ENV_SELECT=1\|lighting\|list` | Environment selector (`demo_env.rs`): a library with an "Environments" folder and a Settings folder in the inventory, their settings assets known locally. `1`: the window opens, then a sky is picked (frame 2600), a water (2900), the next sky with › (3200), a day cycle (3500) and « Environnement partagé » (3800, 5 s crossfade); `lighting`: « Éclairage personnel » opens too, a sky is picked (2600) then edited (2900); `list`: the sky list dropped down (2500) |
| `AURORA_MAX_AVATARS`, `AURORA_MAX_COMPLEXITY`, `AURORA_AA`, `AURORA_SHADOWS` | Override these settings |
| `AURORA_OCCLUSION=0\|1` | Override the « Occlusion » setting (Graphismes › Qualité) |
| `AURORA_NO_OCCLUSION=1`, `AURORA_NOVSYNC=1` | Turn GPU occlusion / vsync off |
| `AURORA_CPU_CULL=1` | Build every draw list on the CPU (the fallback when the GPU lacks `MULTI_DRAW_INDIRECT_COUNT`) instead of culling them on the GPU, for comparison; `cull=cpu\|gpu` in the `perf summary` line |
| `AURORA_RENDER_THREAD=0` | Draw the frames on the main thread, as before the render thread, for comparison and debugging. By default the main thread hands each frame over to the render thread as a self-contained packet and goes on with the next frame; with `0` the same packet is executed in place. `thread=on\|off` in the `perf summary` line, and a log line at start (`renderer: frames drawn by…`) |
| `AURORA_PROFILE=1` | Profiling in the log: once a second a `perf summary` line, plus a `perf settings` line when the settings change (see [Profiling](#profiling)). The background frame cap ("Limiter hors focus") is ignored, so a window without the focus is still measured at full speed |
| `AURORA_PROFILE_FRAMES=1` | Also one `render profile` and one `gpu profile` line per frame (renderer steps and GPU time by element of every frame) |
| `AURORA_GPU_VALIDATION=1` | wgpu validation layers |
| `AURORA_HOVER_CHECK=1` | Hover cursor diagnostics: every answer about the object under the cursor (kept from an earlier frame, or searched among the candidates of the pick index) is compared with the full search of the scene; differences are logged as `hover check`, with the searches done and the answers reused every ten seconds. Costs the full search every frame |
| `AURORA_DEBUG_GLOW=1`, `AURORA_GLOW_SKIP=<mask>`, `AURORA_MEDIA_DEBUG=1` | Renderer and media diagnostics |
| `AURORA_EMOJI_FONT=<path>` | Use another emoji font |
| `AURORA_LOG_NAME=<name>` | Log file name |

Logs are in `%LOCALAPPDATA%\Aurora\AuroraViewer\data\logs\`.

### Profiling

With `AURORA_PROFILE=1`, each `perf summary` line covers one second. Two
threads share a frame: the **main thread** runs the simulation and the
interface and closes the frame as a packet; the **render thread** turns the
packet into GPU work while the main thread is already on the next frame.
With `AURORA_RENDER_THREAD=0` everything below runs on the main thread.

- `fps`, `frame` (average frame time, ms), `p95`, `max`, and `slow`: the
  number of frames over 1.5 × the median of that second (the hitches the
  frame graph shows). The frame is the main thread's: the time between two
  turns of its loop, waits included;
- **main thread**: the average CPU time of each step of the frame (`events`,
  `social`, `sync`, `media`, `lists`, `stream`, `params`, `ui`, `render`…),
  which add up to `frame`. `render` is what rendering costs the main
  thread: the frame packet and the wait for the render thread to take it
  (the whole encoding, swapchain wait included, with
  `AURORA_RENDER_THREAD=0`);
- `max:` the steps whose longest single-frame time reached 1 ms in that
  second, longest first, renderer steps (`r_*`), streaming and sync parts
  (`s_*`) and the `thread` fields included, or `-` when none did. A
  periodic slow frame shows up here with the step that caused it, e.g.
  `max: media=6.10 render=2.31`, while its average stays tiny;
- `thread=on|off`, then how the two threads meet:
  - `packet` (main thread, inside `render`): closing the frame — the
    changed ranges of the scene's tables into the frame's write journal, a
    copy of the CPU draw lists — and handing it over;
  - `wait_render` (main thread, inside `render`): time waited for the
    render thread to finish the frame before (back-pressure: at most one
    packet is in the render thread's hands, none is queued). A frame that
    carries a capture is waited for entirely;
  - `rt_frame` (render thread): its whole time on a frame, from the packet
    to the end of the presentation, swapchain wait included (0 with
    `thread=off`);
  - `rt_idle` (render thread): time waited for the main thread's packet.

  A frame costs about the longer of the main thread's work (`frame` minus
  `wait_render` and `limiter`) and `rt_frame`. `rt_idle` above 0 with
  `wait_render` near 0: the main thread is the limit; the opposite: the
  render thread is, or the GPU / vsync behind it when `r_acquire` or
  `r_present` hold most of `rt_frame`;
- **render thread** (main thread with `thread=off`): the renderer steps
  (`r_*`: `resources` — the replay of the write journal —, passes, egui,
  `finish`, `submit`, `present`, `acquire`, the wait for the swapchain
  image). With the render thread they are those of the frame before: a
  frame's result comes back with the next hand-over;
- main thread again: the parts of the streaming
  work (`s_*`, inside `results` and `stream`: `fetched`
  downloads handed to the streamers, `geometry` built geometry put in the
  arena, `decoded` other finished jobs, `tex_update` fetches and decodes
  started, `assets` meshes / animations / sounds / materials, `skin` skin
  bindings, `upload` textures sent to the GPU, of which `pages` new texture
  pages, `maintain` eviction and cache writes, `diag`) and of the scene sync
  (inside `sync`: `sync_list` removed and changed objects and the frame's
  list, `sync_plan` their placement, `sync_apply` moves and the full syncs
  that fit the frame's time budget, `sync_alpha` faces classified again
  after a texture changed alpha class), the GPU time by element
  (`g_*`), draws and draw commands, synced / rebuilt objects, posed
  avatars, bytes of records and palettes sent to the GPU and of the whole
  write journal of the frame (`journal_kb`), texture memory (live textures,
  texture pages and their allocated memory), geometry memory, and the
  objects whose full sync waits for a frame with time left
  (`sync_backlog`).

## Contributing

This repository is worked on by humans together with AI agents, each agent on
its own task in its own git worktree:

- [AGENTS.md](AGENTS.md) — rules for AI agents (testing, safety, PRs, reviews)
- [HUMANS.md](HUMANS.md) — how the repository works for humans
- [ARCHITECTURE.md](ARCHITECTURE.md) — scope, crates and file layout
- [TASKS.md](TASKS.md) — what is done, in progress and to do
- [docs/BRANDING.md](docs/BRANDING.md) — palette, logos, icons and UI rules

Behaviour follows Firestorm: its sources are expected next to the repository
(`../phoenix-firestorm`, cloned from
[FirestormViewer/phoenix-firestorm](https://github.com/FirestormViewer/phoenix-firestorm)).

## License

Aurora Viewer is free software under the
[GNU General Public License v3.0 or later](LICENSE). Parts of it are ported
from the Second Life viewer and Firestorm (originally LGPL 2.1); see
[NOTICE.md](NOTICE.md) for details and third-party licenses.

Aurora Viewer is not affiliated with Linden Lab or the Firestorm project.
Second Life® is a trademark of Linden Research, Inc.

---

## En français

**Aurora Viewer** est un viewer Second Life moderne écrit en Rust : rendu GPU
récent (wgpu / Vulkan), interface plate et soignée, et un comportement fidèle
à Firestorm (mêmes messages, mêmes règles, mêmes valeurs par défaut).

- **Compiler :** voir [Build](#build) ci-dessus (Windows, Rust stable).
- **Tester sans compte :** `AURORA_DEMO=1` lance un mode démo hors ligne ;
  la liste des options est dans [Test switches](#test-switches).
- **Contribuer :** le dépôt est pensé pour des humains travaillant avec des
  agents IA ; lis [HUMANS.md](HUMANS.md) (humains) et
  [AGENTS.md](AGENTS.md) (agents). Le suivi des tâches est dans
  [TASKS.md](TASKS.md).
- **Licence :** GPL-3.0-or-later, voir [NOTICE.md](NOTICE.md).
