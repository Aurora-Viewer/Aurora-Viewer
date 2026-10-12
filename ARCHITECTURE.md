# Architecture d'Aurora Viewer

## Périmètre

Aurora Viewer est un viewer Second Life complet écrit en Rust : connexion,
réseau UDP et HTTP, monde (objets, terrain, avatars, environnement), rendu
moderne (wgpu sur Vulkan), son, voix, médias et interface (egui).

**Référence de comportement : Firestorm.** Chaque fonctionnalité reproduit
ce que fait Firestorm (et le viewer de Linden Lab dont il dérive) : mêmes
messages réseau, mêmes règles, mêmes valeurs par défaut, sauf choix explicite
noté dans `TASKS.md`. L'interface, elle, est propre à Aurora (voir
[docs/BRANDING.md](docs/BRANDING.md)). Firestorm est la référence du
résultat, pas de l'implémentation : le rendu doit être le même ou meilleur,
mais le moteur est conçu pour Vulkan et le GPU moderne, sans reprendre les
procédés hérités d'OpenGL (voir [AGENTS.md](AGENTS.md#2-comportement-de-référence--firestorm)).

**Hors périmètre pour l'instant :** les grilles OpenSim, le support de
plateformes autres que Windows pour les releases (le code reste portable).

## Arborescence

Le fond des dialogues de mise à jour et de fermeture est flouté par `aurora-render/src/ui_blur.rs`
et `shaders/ui_blur.wgsl` : un marqueur egui sépare les couches de fond du
dialogue. La scène et ces couches sont rendues dans une texture, floutées
horizontalement puis verticalement sur le GPU ; le dialogue reste net.
Ces cibles existent seulement tant que le dialogue est affiché.

```
aurora-viewer/                  dépôt git (GitHub : Aurora-Viewer/Aurora-Viewer)
├─ Cargo.toml                   workspace : version, édition, licence, lints communs
├─ rust-toolchain.toml          Rust stable (toujours la dernière version)
├─ rustfmt.toml                 formatage (lignes de 140 caractères)
├─ README.md                    présentation, différences avec Firestorm, compilation
├─ AGENTS.md                    règles des agents IA (Claude, Codex…)
├─ CLAUDE.md                    renvoie vers AGENTS.md
├─ HUMANS.md                    guide des humains : PR, fusion, compilation
├─ ARCHITECTURE.md              ce fichier
├─ TASKS.md                     suivi des tâches : fait / en cours / à faire
├─ LICENSE, NOTICE.md           GPL-3.0-or-later, licences tierces
├─ aurora-tools.cmd             outils de développement (double-clic) : scripts/tools
├─ assets/
│  ├─ branding/                 logos (couleur, silhouette), SVG + PNG 4096, aurora.ico
│  ├─ tabler-icons/             bibliothèque Tabler complète (SVG : outline, filled)
│  └─ emoji/                    police emoji (téléchargée, hors git)
├─ crates/                      le code (voir ci-dessous)
├─ docs/                        BRANDING.md, TESTING.md (options de test `AURORA_*`, profilage), captures du README
├─ scripts/                     outils des agents et des humains (PowerShell)
│  ├─ automation-window-border.ps1 contour jaune en pointillés des fenêtres de test automatisé (Windows)
│  └─ tools/                    aurora-tools : menu, interface (ui), vérifications et réparation (checks), actions, PR en direct (prs), releases, réinitialisation (reset), permissions de Claude Code (claude)
└─ .github/                     CI (workflows/, actions/), release, modèle de PR, Dependabot
```

À côté du dépôt (hors git), dans le dossier du projet installé par la
commande en une ligne (`scripts/setup.ps1`, puis les outils) :

```
<dossier du projet>\            par exemple C:\Aurora-Viewer
├─ aurora-viewer\               le dépôt
├─ phoenix-firestorm\           sources officielles de Firestorm (lecture seule)
├─ RELEASE\                     build --release du dernier main (scripts/build-release.ps1)
└─ work\                        un worktree git par tâche d'agent
```

## Les crates

| Crate | Rôle |
|---|---|
| `aurora-viewer` | L'application : boucle de fenêtre (winit), état du monde, scène, interface egui, agent, caméra, démo hors ligne |
| `aurora-updater` | Releases GitHub stables et seuils de maintenance ; téléchargement annulable, SHA-256 et extraction ZIP bornée ; auxiliaire Windows, remplacement transactionnel et restauration tant que la nouvelle version n’a pas présenté une image. `user_data.rs` sauvegarde/restaure le cache et `settings.json` si la politique l’exige. Tests HTTP et fichiers isolés. |
| `aurora-version` | Source commune de la version compilée : tag Git, provenance des builds entre tags, User-Agent HTTP et version de connexion SL à quatre composantes numériques. Son `build.rs` suit aussi les références Git des worktrees ; `build_support.rs` contient la résolution pure testée. |
| `aurora-i18n` | Langue commune au viewer et aux messages réseau : catalogues Fluent `locales/*.ftl`, détection de l'OS, choix enregistré, repli anglais, variables, pluriels et décimales. `build.rs` découvre automatiquement les langues ; les libellés simples et tableaux de libellés sont mis en cache par langue. Voir `docs/TRANSLATING.md`. |
| `aurora-net` | Connexion (XML-RPC), circuits UDP, capabilities, file d'événements, téléchargement HTTP des assets ; tourne sur son propre runtime tokio. `session/object_actions.rs` : toucher / saisir / relâcher et inventaire d'objet (capability ou transfert UDP borné en mémoire) ; `task_inventory.rs` : parsing LLSD et fichier historique ; `land.rs`, `session_land.rs` : messages des parcelles (« À propos du terrain », RemoteParcelRequest d'un lieu quelconque) |
| `aurora-msg` | Système de messages UDP de SL : trames, zerocoding, messages typés générés depuis `message_template.msg` (build.rs) |
| `aurora-llsd` | Type LLSD et ses formats XML / binaire / notation |
| `aurora-prim` | Modèle des prims : paramètres de volume, faces, paramètres étendus, génération de la géométrie (port de `llvolume`) |
| `aurora-assets` | Décodeurs d'assets : JPEG2000, mesh, animations, matériaux, maillages d'avatar `.llm`, squelette |
| `aurora-render` | Moteur de rendu wgpu / Vulkan : textures bindless regroupées en pages (texture arrays), géométrie sous-allouée, multi-draw-indirect, ombres, reflets, post-traitement ; animations de texture (`tex_anim.rs` : référence CPU et paramètres des enregistrements, évaluées par les vertex shaders) ; listes de dessin pilotées par le GPU (`gpu_cull.rs`, `shaders/cull.wgsl` : tables des faces et des objets tenues par la scène, culling et compactage en compute pour chaque vue, dessin par `multi_draw_indexed_indirect_count`) ; lumières locales triées par case d'écran et par profondeur (`light_tiles.rs` : pour chaque colonne, rangée et tranche de profondeur de la vue principale, l'ensemble des lumières qui la couvrent, calculé à chaque image sur le thread de rendu ; un fragment ne parcourt que les lumières communes aux trois, `light_walk` dans `shaders/common.wgsl`, au lieu des 64 de la liste) ; occlusion Hi-Z en deux phases (`occlusion.rs`, `shaders/occlusion.wgsl`, test commun `shaders/hiz_test.wgsl`) ; ombres dessinées, dans chaque cascade, avec les seuls objets dont l'ombre peut tomber sur ce que la vue y lit (`shadow_cull.rs` : volume des pixels qui lisent la cascade, étiré vers la lumière, que testent le culling GPU et les listes du CPU ; faces opaques sans fragment shader) ; envois du streaming sans copie sur le thread principal (`upload.rs` : mémoire de transit mappée où écrivent les tâches de fond, copies GPU enregistrées dans un encodeur soumis en tête de l'image) ; textures compressées en BC7 quand elles le supportent sans dommage (`bc7.rs` : encodeur Intel ISPC et contrôle de qualité de chaque niveau ; les pages sont en RGBA8 ou en BC7 selon le format de la chaîne, `TexelFormat`) ; rien qui grandisse ou se libère d'un coup dans une image (`arena.rs` : plages de géométrie allouées au meilleur ajustement par un index des tailles, enregistrements de dessin gardés par blocs et tampon GPU agrandi par une copie GPU ; `textures.rs` : réduction d'une texture par copie de ses niveaux vers une couche plus petite, substitut gris partagé par les textures en attente, pages vidées détruites quelques-unes par image par un thread à part) ; mode de présentation de la swapchain (`present.rs` : une seule règle à la création de la surface et à chaque bascule de la synchronisation verticale, mode forcé par `AURORA_PRESENT_MODE`) ; deux moitiés, voir « Threads et propriété des données » : `main_thread.rs` (le `Renderer` vu par la scène : ses stores, la clôture de l'image), `writes.rs` (journal des écritures GPU de l'image), `packet.rs` (paquet d'image et résultat), `ui_frame.rs` (formes d'egui remises au thread de rendu, qui les tesselle comme `egui::Context::tessellate`), `thread_cpu.rs` (temps processeur d'un thread, pour le profil), `render_thread.rs` (thread de rendu, remise des paquets en rendez-vous), `renderer.rs` (`Backend` : surface, pipelines, cibles, passes), `helpers.rs` (threads auxiliaires de `finish`) |
| `aurora-audio` | Sortie audio : mixeur avec les canaux de volume SL, streams de musique, sons du monde |
| `aurora-voice` | Voix SL en WebRTC (réception et émission) |
| `aurora-media` | Hôte des plugins médias SLPlugin (CEF pour le web, LibVLC pour la vidéo) |

Dépendances entre crates (simplifié) :

```
aurora-viewer ──► aurora-net ──► aurora-msg, aurora-llsd
      │      └──► aurora-render ──► (wgpu)
      ├──► aurora-prim, aurora-assets
      ├──► aurora-audio, aurora-voice, aurora-media
```

## `aurora-viewer` en détail

`ui/scripts/indent.rs` gère les paliers de quatre colonnes pour Tab et Retour
arrière et conserve le retrait à Entrée dans les deux éditeurs,
avec le curseur et l'historique d'annulation natifs d'egui. Le collage reste
exact. `ui/scripts/experiences.rs` suit l'association et les choix disponibles
dans les scripts d'objets ; `aurora-net/src/scripts/experiences.rs` charge
GetMetadata / GetCreatorExperiences / GetExperienceInfo en arrière-plan.
L'association choisie fait partie du document et de l'instantané enregistré :
la sauvegarde n'a plus besoin de relire les métadonnées.

La saisie de `}` aligne son retrait sur le `{` correspondant, hors chaînes,
commentaires et directives. `ui/scripts/colors.rs` affiche des pastilles
près des vecteurs numériques entre 0 et 1 inclus, détectés et mis en cache
par `syntax.rs`. Le nuancier commun garde un identifiant de fenêtre propre
à chaque document et partage les préférences de palette avec la construction.
L'aperçu est local, Annuler restaure les octets d'origine et OK forme une seule
action dans l'historique natif. La saisie de la source et l'enregistrement
attendent la fermeture du nuancier ; une modification externe ferme la session
sans écraser le nouveau texte.

`ui/scripts/editing.rs` regroupe les transformations sur les lignes sélectionnées
et les bascules de commentaires, avec une seule étape Annuler / Rétablir et
conservation du sens de la sélection. Les commandes clavier arrivées après une
saisie ou un déplacement natif attendent le curseur réellement calculé par egui.
`ui/scripts/folding.rs` suit les paires d'accolades hors commentaires / chaînes,
remappe les replis après les modifications et réduit à zéro la hauteur des lignes
cachées dans le layout. Le texte et ses indices restent complets ; les actions
de copie, sauvegarde, compilation et annulation n'utilisent aucune projection.
La navigation / recherche vers un caractère caché rouvre son bloc.

`ui/scripts/syntax.rs` colore la source LSL des deux éditeurs, aussi en lecture
seule. Le catalogue des noms natifs de Firestorm est embarqué dans
`assets/lsl/keywords.json`. L'analyse lexicale et les formats restent en cache
par document tant que la source, le thème et la police ne changent pas ; la
source envoyée au simulateur conserve exactement ses octets lorsque le
préprocesseur est désactivé.

`ui/scripts/view.rs` compose les menus, la barre d'outils, la gouttière des
lignes et les diagnostics cliquables. `editing.rs` partage le curseur et
l'historique natifs entre les commandes et la saisie ; `dialogs.rs` porte la
recherche/remplacement, les préférences et l'aide par mot-clé.
`settings.rs` conserve les couleurs et options de l'éditeur dans les réglages
communs. `reference.rs` expose les signatures et descriptions du catalogue
Firestorm embarqué dans `assets/lsl/reference.json`.
`files.rs` réalise les dialogues, imports/exports et la surveillance d'un
éditeur externe en arrière-plan, sans shell ni écrasement silencieux d'une
modification concurrente. `world/script_debug.rs` filtre les sorties de scripts
possédés après les blocages et le bridge ; `ui/scripts/debug.rs` les affiche
ensemble ou par objet.

`ui/scripts/preprocessor/` prépare l'enregistrement sur un thread distinct :
lexique, macros C et expressions conditionnelles, inclusions de scripts de
l'inventaire, inclusions locales facultatives, transformations LSL des
listes étendues et `switch`, suppression des fonctions/variables inutilisées.
Les demandes d'inclusions et leurs réponses traversent les actions de
l'éditeur ; aucune attente réseau ou fichier ne bloque une image. La source
originale est encapsulée avec les marqueurs de Firestorm et décodée à
l'ouverture, tandis que le simulateur compile le résultat prétraité.
Les deux vues utilisent des curseurs distincts : consulter le résultat ne
remplace pas la source ni son historique d'annulation.
`preprocessor/clock.rs` produit les macros de date et d'heure : horloge locale
de Windows, avec appel système limité à la structure SYSTEMTIME ; UTC pour
les autres plateformes.

Les scripts utilisent un éditeur commun (`ui/scripts.rs`, actions dans `app/scripts.rs`) pour les éléments de l'inventaire et le contenu d'une prim. Chaque document garde sa source, sa version enregistrée et ses requêtes en cours. `aurora-net/src/session/scripts.rs` charge les sources avec `TransferRequest` et le contexte de permissions de l'inventaire, y compris lorsque l'UUID d'asset est masqué : le simulateur résout la source à partir de l'élément et de son propriétaire / objet. Un UUID d'asset nul ne signifie pas un nouveau script et ne déclenche pas le modèle local. `aurora-net/src/scripts.rs` enregistre et compile par `UpdateScriptAgent` / `UpdateScriptTask`, avec retour distinct de l'asset enregistré et des diagnostics de compilation. Aucun transfert ne bloque la boucle d'image.

| Dossier / fichier | Contenu |
|---|---|
| `imports/`, `app/imports.rs` | Brouillons d’import par fichier, aperçu, sélection des cartes de matériaux, paramètres de texture / animation / LOD / physique ; file d’envoi séquentielle avec dépendances de matériaux dédupliquées, devis mesh figé avant confirmation et traitement des fichiers sur les workers. `local.rs` surveille les textures, matériaux et meshes locaux et prépare leurs ressources GPU ; meshes, textures et matériaux ciblent la sélection ou un prim support par des surcouches visuelles conservées dans `world/objects.rs`, sans modifier les données serveur. Les textures utilisent le pool de staging de la scène ; la boucle de rendu installe et libère explicitement leurs slots GPU. Le sélecteur local reconnaît les trois types, dont les documents glTF/GLB selon leur contenu. `persistence.rs` sauvegarde les références/réglages et les applications par compte/grille dans le profil privé ; relecture sur worker, écritures atomiques regroupées et vidées à la fermeture, conservation des sources manquantes. `ui/destination.rs` filtre les dossiers autorisés, avec cache et liste virtualisée ; `ui/mesh_panel.rs` sépare aperçu, LOD, collisions et données d’import ; `animation.rs` garde une horloge indépendante ; `preview.rs` prépare aussi les poses de mains, expressions et positions d’articulations importées |
| `aurora-assets/src/import_*` | Validation et conversion pures : dimensions / alpha / JPEG2000, WAV vers Vorbis, BVH vers animation, cartes PBR RGB / ORM, ressources glTF / GLB partagées, géométrie Collada / glTF, LOD par meshoptimizer, transfert spatial des poids, enveloppes convexes et sérialisation du mesh en sections LLSD comprimées. `import_mesh/vhacd.rs` et `native/vhacd/` encapsulent V-HACD 4.1 : allocations bornées, exceptions contenues, annulation / progression atomiques ; chaque modèle tourne sur un worker Rust |
| `aurora-net/src/inventory/{upload,mesh_upload}.rs`, `session/asset_imports.rs` | Protocole d’import à deux étapes, coût attendu, relecture de l’élément créé ; matériaux créés puis enregistrés par leur capability dédiée. Les URL de transfert mesh restent privées à la session, associées à un jeton local consommé une seule fois et expirant après cinq minutes |
| `aurora-render/src/preview.rs`, `shaders/preview.wgsl` | Aperçus 3D dans les fenêtres : rendu PBR et skinning sur le thread GPU, cibles hors écran reliées directement à egui, déduplication des cartes partagées, aucune lecture GPU ni attente du GPU dans la boucle d’interface |
| `build/replace.rs`, `build/replace/`, `build/ui/replace.rs` | Remplacement guidé d'une partie liée : cible verrouillée, sélection du remplaçant, options et avancement ; étapes confirmées par le serveur, copie du contenu via l'inventaire du résident, conservation des états des scripts, récupération de la configuration initiale et simulateur hors ligne |
| `ui/inventory/document.rs`, `aurora-net/src/inventory/notecard.rs` | Cycle d’édition des documents : brouillon et sauvegarde corrélés à la fenêtre, fermeture après confirmation, refus conservant le texte ; import des notecards v1 / v2, export v2 avec longueur UTF-8 bornée et conservation des permissions des pièces jointes encore référencées |
| `ui/inventory/embedded.rs` | Éléments intégrés aux notes : icône et nom dans le texte, aperçu / ouverture et copie dans l’inventaire, dépôt à la position du pointeur ; un caractère par élément pour conserver la sélection, la suppression et l’annulation |
| `aurora-net/src/session/inventory_preview.rs` | Lecture des notes et scripts protégés par transfert d’inventaire du simulateur (identifiants de l’agent, de session, du propriétaire, de l’objet, de l’élément et de l’asset) ; réassemblage borné des paquets désordonnés, corrélation au simulateur et expiration |
| `app/task_notecards.rs`, `aurora-net/src/session/task_notecard.rs` | Notes du contenu des objets : double-clic / Ouvrir, éditeur commun avec état propre à l’objet et à l’élément ; droits de l’objet et de la note, transfert d’asset autorisé par l’inventaire, sauvegarde via UpdateNotecardTaskInventory, relecture et brouillon conservé en cas de refus |
| `main.rs`, `app.rs`, `app/` | Démarrage, boucle d'événements, une image du viewer, actions de l'interface ; commandes de la barre de chat (`app/chat_commands.rs`), menus clic droit (`app/context_menu.rs` : ouverture, droits sur l'objet, actions choisies), clics sur le monde sans attente du GPU (`app/click_pick.rs` : la profondeur sous le clic est demandée au rendu, le clic est traité à l'arrivée de la réponse avec le curseur et les touches qu'il avait, et ce qui le suit — relâchement, autres boutons, mouvement de souris qui déplace la caméra — attend derrière lui dans l'ordre ; contrôle `AURORA_CLICK_PICK`, clics de démo `AURORA_DEMO_CLICKS`), geste de déplacement des HUDs avec touche configurable (ALT par défaut) et clic gauche et scénario hors ligne (`app/hud_drag.rs`) |
| `cli.rs` | Arguments de ligne de commande (`--title`, `--fps-limit`, `--language`, `--version`) ; titre avec la version commune et le profil de compilation |
| `cmdline/` | Barre de chat comme ligne de commande (FSCmdLine) : analyse des commandes, calculatrice (LLCalc), dés ; exécutées par `app/chat_commands.rs` |
| `agent.rs` | Notre avatar (AgentUpdate, extrapolation depuis vitesse / accélération serveur, lissage comme LLDrawable) |
| `ao/`, `app/ao.rs`, `ui/ao.rs` | Animation Overrider intégré : catalogue des ensembles `#Firestorm/#AO`, liens d’inventaire et import ZHAO (`inventory.rs`), états / cycles / groupes / pistes et remappage de la seule pose de notre avatar (`runtime.rs`), assets et transitions synthétiques (`demo.rs`) ; opérations AIS et réponses asynchrones dans `app/ao.rs`, fenêtre de gestion ouverte par le bouton AO de la barre du bas. Les signaux bruts du simulateur restent utilisés pour le mouvement et les transitions ; les remplacements sont annoncés par AgentAnimation. Activation et dernier ensemble enregistrés par UUID de compte ; pause et poses debout propres à la session. |
| `camera/` | La caméra comme LLAgentCamera : vue derrière l'avatar, caméra Alt sur un point ou un objet (`focus.rs`), caméra de siège, vue subjective, transitions, lissage, recul en vol, collision envoyée par le simulateur ; réglages Firestorm (`settings.rs`), scénarios de démo (`demo.rs`) |
| `world/` | État du monde reçu du réseau : objets, régions, terrain, environnement (EEP : cycles et assets de réglages dans `eep.rs`, choix local > parcelle > région > jour par défaut, environnement local et fondus dans `eep_env.rs`, listes du sélecteur d'environnement et chargement des dossiers dans `env_select.rs`), parcelle sélectionnée d'« À propos du terrain » et ses droits (`land.rs`), social, profils des avatars, inventaire, groupes (actif, épinglés), cercles de contacts et surnoms (`contact_sets.rs`), blocages, messages du bridge LSL de Firestorm (cachés), orientation des corps, regard (LookAt), carte, profils de lieux de « Lieux » et des fenêtres séparées (`place_details.rs` : région par son nom, repère, RemoteParcelRequest, ParcelInfoRequest), assets des repères et handles de leurs régions (`landmarks.rs`), historique de téléportation daté et enregistré (`tphistory.rs`)… |
| `scene/` | Ce qu'on envoie au GPU : géométrie et LOD, textures (streaming), avatars (squelette, animations, silhouette), particules, eau, sondes de reflets, sons du monde, imposteurs, assets de réglages d'environnement (`settings.rs` : ciel, eau, cycle du jour) |
| `ui/` | Interface egui : barres, fenêtres, options, chat, contacts (amis, groupes, cercles : `contacts.rs`), profils des avatars, « Lieux » (`places.rs` : favoris, repères, historique de téléportation) et profils de lieux (`place_details.rs`, aussi en fenêtres séparées), « À propos du terrain » (`land/`, un fichier par onglet, dialogues), choix d'un résident (`avatar_picker.rs`), page web dans une fenêtre (`web_view.rs`), choix d'une texture, inventaire, sélecteur d'environnement et « Éclairage personnel » (`environment.rs`), cartes, overlays de débogage ; sons causés par les widgets (`sound_cues.rs` : clics, touches refusées, fenêtres) ; menus clic droit : apparence commune (`menu.rs` : icône Tabler, sous-menus, cases cochées) et entrées comme Firestorm (`context.rs` : monde, noms d'avatars, groupes) |
| `build/` | Outils de construction comme LLFloaterTools : sélection et manipulateurs (`manip.rs`), régressions de sélection des parties liées et des objets entiers (`selection_tests.rs`), parcours de la sélection sous le pointeur avec CTRL + molette (`cycle.rs` : unités par linkset ou prims d’un même objet, molette de haute précision), déplacement direct des HUDs (`hud_drag.rs` : racine du linkset, conversion écran / point d’attachement, écho local et MultipleObjectUpdate de position au relâchement), outils Déplacer (`grab.rs`) et Aligner (`align.rs`), terrain (`land.rs`), modifications de la sélection (`edits.rs`), matériaux et médias (`materials.rs`), contenu des objets (`contents.rs`), impact et poids (`costs.rs`), simulateur de démo (`demo_sim.rs`) ; la fenêtre et ses onglets dans `build/ui/` |
| `interaction.rs`, `cursors.rs`, `ui/object_actions.rs` | Règles des actions de clic 0–9, héritage, permissions, curseurs dessinés à partir des icônes Tabler (règles de Firestorm), fenêtres d'achat / paiement et liste du contenu ; transaction après confirmation |
| `app/object_actions.rs` | Déclenchement des actions, toucher maintenu, déplacement physique, lecture de parcelle, ouverture de média et cadrage de caméra |
| `scene/picking.rs` | Rayons contre les triangles partagés avec la géométrie affichée (prims, sculpts, meshes) ; prim réellement visée au survol et au clic gauche malgré des boîtes recouvrantes, IGNORE traverse la géométrie hors construction, informations de surface pour les scripts de toucher ; les mêmes tests sur les seuls candidats de l'index de survol |
| `scene/drawn.rs` | Ce qu'un clic désigne d'après les faces que le GPU a dessinées sous son pixel (`aurora-render/src/drawn_pick.rs`) : les objets que le CPU ne sait pas trouver là où ils sont affichés — animesh, mesh riggés portés, mesh portés — sont désignés tels que le GPU les pose, pour le clic gauche, le clic droit et la construction. Règle isolée et testée (`reaches` : geste × statut de l'objet), informations de surface d'un toucher (face, position, ST / UV, normale et binormale de la surface posée). Écart voulu avec Firestorm, qui n'atteint un mesh riggé qu'au clic droit et avec l'outil Déplacer |
| `scene/hover.rs` | Objet sous le curseur sans recherche à chaque image : index compact des sphères englobantes tenu par la synchro (candidats proches du rayon du curseur ou du point de profondeur), réponse gardée tant que le rayon, la profondeur lue sur le GPU et les objets alentour ne changent pas, refaite au moins toutes les 100 ms ; jamais pour les clics. Les objets testés quel que soit le rayon (avatars, mesh riggés : des milliers dans une foule) ont une classe, établie depuis l'objet à la première recherche qui suit son arrivée ou son changement, qui les écarte des étapes où ils ne peuvent rien changer. Contrôle : `AURORA_HOVER_CHECK` ; comparaison : `AURORA_HOVER_LEGACY` ; ligne `hover summary` du profil |
| `scene/pick_accel.rs` | Triangles d'une face de picking rangés en hiérarchie de volumes englobants, pour la recherche de survol : même distance que le test de chaque triangle (même calcul sur ceux qu'elle n'écarte pas), construite par une tâche de fond la première fois que le survol teste une grande face de mesh ou de sculpt ; les clics testent toujours chaque triangle |
| `media/` | Médias des prims et des parcelles (objets à médias tenus à jour par le flux de changements de l'`ObjectStore`, sans passe sur tous les objets), cookie OpenID des pages web de la grille (`openid.rs`) |
| `demo.rs`, `demo_land.rs`, `demo_place.rs`, `demo_eep.rs`, `demo_env.rs`, `demo_stream.rs`, `demo_glyphs.rs`, `demo_tags.rs` | Le mode démo : une scène locale qui simule un serveur (et ses réponses à « À propos du terrain », aux profils de lieux, repères et historique de « Lieux » et à ExtEnvironment, une bibliothèque d'environnements pour le sélecteur) ; `demo_stream.rs` : objets et textures qui arrivent par vagues (test des à-coups du streaming, AURORA_DEMO_STREAM) ; `demo_glyphs.rs` : résidents dont le titre, le nom et le message utilisent les caractères étendus des noms d'affichage (AURORA_DEMO_GLYPHS) |
| `settings.rs`, `keybinds.rs`, `keybinds/layout.rs`, `theme.rs` | Réglages enregistrés, raccourcis, disposition Windows et touches de déplacement par défaut, palette |
| `ui_sound.rs` | Catalogue des sons de l'interface (UISnd* de Firestorm), réglages par son |
| `ui/appearance.rs`, `ui/appearance/items.rs`, `ui/appearance/gallery.rs`, `world/appearance.rs` | Fenêtre Apparence, galerie / tenues / portés, édition et dialogue Enregistrer sous ; menus des éléments et de la galerie, confirmation de sauvegarde / suppression, choix d’image, renommage ; points d’attachement / HUD, profil et original ; règles de changement du COF, sauvegarde par liens, protection des parties du corps, scénario hors ligne |
| `aurora-net/src/outfits.rs`, `aurora-net/src/outfits/categories.rs` | Écriture des liens de tenue, favoris, noms et images par AIS InventoryAPIv3, déplacement dans la corbeille ; confirmation et relecture des dossiers / éléments |
| `slurl.rs`, `link_trust.rs` | Liens des textes : SLURL (barre de navigation, libellés « Région (x,y,z) » des liens de lieu) ; confiance des liens web (site de confiance, inconnu, dangereux : raccourcisseurs, faux noms officiels, adresses IP…), jugée localement sur l'URL |
| `ui/widgets.rs`, `ui/widgets/resize.rs`, `ui/widgets/demo.rs` | Widgets communs et fenêtres flottantes ; repli par double-clic du titre ou bouton, taille développée conservée et barre repliée déplaçable ; gestes hors ligne sur le titre réel ; poignées de redimensionnement bornées avant le déplacement, bord opposé fixe, limites du contenu et de la zone visible ; position et taille conservées par egui ; noms plus longs que leur place coupés par « … » avec le nom entier au survol (`elided_label`, `paint_elided`) pour les listes de résidents, de groupes et de conversations |
| `ui/layout.rs`, `ui/layout/gesture.rs` | Disposition de l'interface : mémoire d'egui (positions, tailles, fenêtres repliées) écrite dans `ui_layout.ron` à la sortie et relue au démarrage ; zone de travail laissée par les barres (`work_area`) et écart commun `GAP` de la disposition par défaut ; `Slot`, par lequel passent `widgets::Floater` et la mini-carte (`widgets::bare_window`), enregistre où chaque fenêtre se tenait et dans quel écran, et la fait suivre ses bords les plus proches quand la fenêtre du viewer change de taille, pendant la session ou d'une session à l'autre (`follow_edges` ; un groupe de fenêtres collées suit en bloc). `gesture.rs` : les gestes de l'image (`layout::frame`, `layout::resizing`) : fenêtre tirée par son titre avec celles qui lui sont collées, bloc gardé dans l'écran, aimantation proposée puis appliquée au relâchement (sous le pointeur pour un redimensionnement), liens de collage créés, enregistrés avec la disposition et retirés quand ils ne tiennent plus (fenêtre fermée, repliée, écartée, Alt) |
| `ui/theme_guard.rs` | Test seulement : lit les sources de l'interface (`ui/`, `build/ui/`, `imports/ui*`, `crash/window.rs`) et échoue si une fenêtre utilise directement un widget d'egui remplacé par un widget partagé de `ui/widgets.rs`, ou écrit elle-même un rayon ou une couleur au lieu de ceux du thème (AGENTS.md, section 7) ; liste des règles, de leur remplacement et des exceptions (fichiers d'`ALLOWED`, lignes marquées `// theme-guard:`), elles-mêmes vérifiées |
| `ui/snap.rs`, `ui/snap/overlay.rs` | Aimantation des fenêtres, géométrie seule : quels bords mobiles se collent à quoi (bord en face d'une autre fenêtre, bord de même côté d'une voisine, zone de travail ; marge de 10 px, écart `layout::GAP`, limites de taille d'un redimensionnement, maintien du collage affiché), guide à dessiner, fenêtres côte à côte (`touching`), groupes (`group`, `block_limits`). `overlay.rs` : le guide et la pastille « Alt » dessinés au-dessus des fenêtres |
| `ui/numeric_drag.rs` | Glissement continu des champs numériques, y compris ceux des curseurs : pointeur virtuel alimenté par le mouvement relatif, curseur Windows maintenu près du champ ; relâchement, perte de focus et disparition du widget libèrent le pointeur. Saisie au clavier et barres des curseurs inchangées |
| `ui/hud.rs`, `ui/nameplate.rs`, `ui/nameplate/mesh.rs`, `ui/nameplate/paint.rs`, `ui/nameplate/tests.rs` | Étiquettes au-dessus des avatars et textes flottants. `hud.rs` choisit les avatars étiquetés, leur point d'ancrage et ce qu'on sait d'eux (noms, statut, voix, complexité), et dessine l'étiquette classique. `nameplate.rs` est la plaque de nom (style par défaut, `Settings::classic_name_tags` pour l'ancien) : fonctions pures pour le niveau selon la distance avec son hystérésis (`level_for`), les lignes, badges et pastille de statut de chaque niveau, les mesures tirées de la maquette (`Metrics`) et la mise en page (`layout`), l'enveloppe de la voix et son halo, et `Plates`, ce que chaque plaque garde d'une image à l'autre (niveau et transition, enveloppe, photos demandées au plus quatre par seconde, géométrie). `mesh.rs` construit la géométrie en mesh simples — pastille, bordure, ombre et halo en anneaux autour d'un contour, disques et anneaux antialiasés — au lieu des formes arrondies d'egui, redécoupées à chaque image ; `paint.rs` dessine une plaque, garde sa géométrie tant que son état ne change pas et la redessine sans copie quand elle n'a pas bougé |
| `ui/fonts.rs`, `ui/fonts/fallback.rs` | Polices de l'interface : police principale embarquée ou choisie, celles d'egui, puis les polices de repli du système pour les caractères que les autres n'ont pas (noms d'affichage, titres, chat : géorgien, CJK, arabe, lettres mathématiques…). `fallback.rs` : liste par écriture, première police installée qui dessine le caractère témoin, fichiers mappés en mémoire et vérifiés sur le thread `aurora-fonts`, alignement de leur ligne de base sur la police principale ; installées à l'image suivante si la recherche n'était pas finie. `AURORA_FONT_FALLBACK=0` les retire |
| `ui/exit.rs` | Modale de fermeture traduite : quitter, déconnecter vers le login, annuler ; fond flouté sur le GPU et interactions bloquées ; attente non bloquante de la fin de session, avec le délai de secours de six secondes de Firestorm |
| `updater.rs`, `ui/update.rs` | Contrôleur asynchrone et fenêtre de mise à jour : notes, progression, relance et erreurs ; recherche manuelle dans Aide / connexion, tag ignoré et choix Demander / Automatique / Obligatoires uniquement. Démo synthétique isolée via `AURORA_DEMO_UPDATE`. |
| `ui/inventory.rs`, `ui/inventory/view.rs`, `ui/inventory/controls.rs`, `ui/inventory/context.rs`, `ui/inventory/clipboard.rs`, `ui/inventory/delete.rs`, `ui/inventory/dialogs.rs`, `ui/inventory/properties.rs`, `ui/inventory/thumbnail.rs`, `app/inventory.rs`, `app/inventory_thumbnail.rs` | Inventaire : tri / filtres purs et cache de vues par génération dans `view.rs` (index, tri et filtrage parallèles avec rayon), commandes et préférences persistantes dans `controls.rs`, arborescence filtrée incluant les dossiers parents, états de dépliage par onglet, résultats dessinés selon la zone visible sans limite de 500 et zone de statut de hauteur fixe ; défilement sur les deux axes, largeur des noms mesurée dans la zone visible puis conservée pour les résultats virtuels, commandes adaptées aux petites largeurs ; menus par type, sélection multiple avec ajout / détachement groupés, renommage dans la ligne après création ou sur demande, fenêtres de dossiers limitées à leurs descendants avec un filtre propre, presse-papiers partagé entre fenêtres et raccourcis natifs Ctrl+C / Ctrl+X / Ctrl+V selon le focus de la liste dans `clipboard.rs`, Suppr et corbeille de bas de fenêtre via la suppression partagée dans `delete.rs`, propriétés et confirmations, aperçus / édition des notes et scripts, lecture des statistiques d’animation ; éditeur d’image à six actions, choix en arbre et capture de la scène sans interface ; décodage / redimensionnement / encodage hors du thread de rendu et exécution des actions après dessin de l’interface |
| `world/inventory/actions.rs`, `world/inventory/wearable.rs`, `world/inventory/demo.rs`, `world/inventory/preview.rs`, `world/inventory/tests.rs` | Règles de permissions, déplacements / copies / liens et offres, préparation des annonces Place du marché, assets de vêtements par défaut et scénario synthétique ; cache d’inventaire v2 avec permissions complètes |
| `aurora-net/src/inventory/operations.rs`, `aurora-net/src/inventory/thumbnail.rs`, `aurora-net/src/session/inventory_upload.rs` | Mutations AIS avec relecture, remappage des UUID attribués par le serveur, créations / copies UDP et accusés avec expiration, sauvegarde des documents par capabilities ; chargement AssetUpload / Xfer des vêtements avant création de l’élément ; vignettes gratuites par InventoryThumbnailUpload, POST JPEG2000 puis AIS et relecture |
| `aurora-assets/src/j2k/encode.rs` | Encodeur OpenJPEG borné en mémoire, vignettes RGB / RGBA carrées de 64 à 256 pixels et conservation de l’alpha |
| `logging.rs`, `cache.rs`, `credentials.rs` | Logs, cache disque, mot de passe retenu (coffre de l'OS) |
| `bug_report/`, `ui/bug_report.rs`, `ui/bug_report/tests.rs`, `app/bug_report.rs` | Rapport de bug : instantané explicite des données autorisées, formulaire avec onglets Description / Image / Données, valeurs localisées et décochables, export aux titres anglais ; diagnostics publics à vocabulaire fermé et images réencodées hors du thread principal. Export local avec dossiers public / privé séparés ; lien GitHub prérempli avec repli presse-papiers, ou création d’issue après Device Flow d’une GitHub App. `uploads.rs` déclare les empreintes et envoie l’image publique et le ZIP privé facultatif vers l’API PHP / Cellar, puis ajoute les liens ; échec partiel repris sur la même issue. L’autorisation GitHub reste uniquement en mémoire dans `github.rs` et n’est pas transmise à l’API Aurora. Aucun secret de connexion SL, chat ou IM n’entre dans le modèle public. |
| `screen_rate.rs` | Fréquence de l'écran sous la fenêtre, relue une fois par seconde (déplacement de la fenêtre, changement de mode de l'écran) : plafond de l'écran de connexion et images déjà cadencées par la synchronisation verticale |
| `frame_profile.rs` | Profil des images (AURORA_PROFILE) : temps de chaque étape de l'image, ligne de synthèse par seconde dans le log (moyenne et maximum de chaque étape, nombre d'images lentes ; détail du streaming sur le thread principal, `s_*`, des paramètres de l'image, `p_*`, et de l'interface par groupe de fenêtres, `u_*` ; temps passé hors du processeur par étape, `off_cpu:` ; rencontre du thread principal et du thread de rendu, segment `thread`) |
| `scene/animesh.rs` | Squelettes autonomes des objets animés, animations du linkset, limites des poses pour le culling et les ombres, scénario de démo |
| `scene/skin_binds.rs` | Liaisons de skin envoyées au GPU (matrice inverse et os de la palette de chaque os d'un mesh riggé, squelette par défaut en tête) : une plage par mesh, rendue quand sa dernière géométrie est libérée et reprise par les mesh suivants ; seules les plages modifiées sont envoyées |
| `scene/sync_sets.rs` | Objets que la synchro de la scène visite à chaque image, tenus à jour par événements (objets modifiés notés par `ObjectStore`, ensemble des objets qui bougent d'eux-mêmes et de ce qui les suit, géométries en attente, contrôles du niveau de détail : objets immobiles dus quand la caméra a assez bougé, objets placés par le GPU à tour de rôle (toutes les 24 images si la caméra bouge, toutes les 0,4 s sinon), tout objet toutes les 10 s ; HUDs portés listés à leurs changements et non avec leur porteur) au lieu d'un parcours de tous les objets ; l'ancienne tranche de LOD de tous les objets reste derrière `AURORA_SYNC_SLICE` ; file d'attente des synchros complètes qui n'ont pas tenu dans le budget de temps de l'image, servie du plus proche au plus lointain (`Backlog`, `FullSyncBudget`) ; file des nouveaux objets, qui attendent leur première synchro dans l'ordre d'arrivée sans passer par la liste de l'image (`Arrivals`, marqués par l'`ObjectStore` jusqu'à cette première synchro) |
| `scene/lod_watch.rs` | Quand revoir le niveau de détail d'un objet immobile : chaque contrôle lui laisse la distance que la caméra peut encore parcourir avant que `lod_for` puisse changer de niveau (`lod_slack`, arrondie par défaut), et l'objet redevient dû quand la caméra l'a parcourue (file triée par échéance) ; un changement de réglage (facteur de LOD, champ de vision) rend tout dû. Le contrôle lui-même reste celui de la synchro : mêmes niveaux aux mêmes distances |
| `scene/sync_check.rs` | Diagnostic `AURORA_SYNC_CHECK` : par seconde, les objets que la synchro visite par image, selon ce qui les a listés et leur nature, et ce qu'il en est sorti (vérifié, replacé, synchro complète) |
| `scene/still_check.rs` | Diagnostic `AURORA_STILL_CHECK` : par seconde, ce qui a changé alors que l'image devrait être immobile (caméra, notre avatar, faces reconstruites et leur raison, objets posés replacés) |
| `scene/sync_plan.rs` | Placement en parallèle (`frame_par.rs`) des objets de l'image, niveau par niveau des chaînes de parents : transformation, LOD, limites ; mise à jour de la seule matrice ou synchro complète |
| `frame_par.rs` | Boucles parallèles de l'image auxquelles le thread appelant participe (placement de la synchro, listes de dessin, poses, prims flexibles) : il commence la boucle aussitôt, quelques workers du pool rayon la rejoignent à mesure qu'ils se réveillent, et ceux qui arrivent trop tard ne sont pas attendus. Le nombre d'aides suit la quantité de travail (`Grain`) ; une boucle courte reste sur le thread appelant |
| `scene/anchors.rs` | Ce que portent les avatars, placé par le GPU : chaque palette d'os se termine par des ancres, matrices monde rigides écrites à chaque image — la première place le squelette (espace du squelette vers le monde), les suivantes les points d'attache du corps tels qu'ils sont posés. L'enregistrement d'un prim porté contient son placement par rapport à l'ancre de son point (`flags::ANCHORED`), celui d'un mesh riggé (mesh porté, bento, animesh porté ou au sol) et du corps d'un avatar lit l'ancre de son squelette : les vertex shaders composent. Le culling GPU compose de même la sphère des prims portés (`CullObject::anchor`) et calcule à chaque image celle des mesh riggés à partir des boîtes par os de leur géométrie (`JointBoxes`, `object_flags::POSED`). La synchro écrit les ancres utilisées, les envoie avec les palettes posées, tient à jour le centre des objets ancrés pour le CPU (tri des faces transparentes, survol, lumières, imposteurs ; les listes que le CPU construit encore testent un mesh riggé avec l'étendue de son squelette) et ne visite plus ces objets tant que seul leur porteur bouge. Restent à la synchro : HUDs, prims flexibles, objets qui bougent d'eux-mêmes et ce qui leur est lié |
| `scene/texture_cache.rs` | Cache disque des textures transcodées (`cache/tex/<id>.bc7`) : chaîne de mips BC7 produite par une tâche de décodage, classe alpha, masque de picking et en-tête de l'image JPEG2000 ; relue au niveau voulu sans décodage au retour dans une région, écrite par le thread `aurora-texcache` ; un fichier sans niveaux retient qu'une texture doit rester en RGBA8 |
| `scene/texture_load.rs` | Mesure des chargements de textures : temps des tâches de décodage par étape (codec, conversion, RGBA, mips, mémoire de transit) et ligne `texture load:` du journal à la fin de chaque chargement (durée, décodages par niveau, octets téléchargés, cache disque) |
| `scene/texture_budget.rs` | Budget mémoire des textures : cible du moment (le réglage, moins quand le reste de ce que le viewer tient sur la carte laisse moins de place sous le budget que le pilote lui accorde, `aurora-render/src/vram.rs`), et palier de pression quand les pages la dépassent. Sous pression, `scene/textures.rs` réduit les textures chargées plus finement que leur taille à l'écran, hors champ d'abord, par copie de leurs niveaux sur le GPU (`TextureTable::shrink`), sans les redécoder ; les pages qu'elles quittent sont compactées sans attendre. Forçage : `AURORA_TEXTURE_BUDGET_MB` |

Les indicateurs vocaux des étiquettes classiques sont dessinés par `ui/voice_tag.rs`
(les plaques de nom ont leur contour et leur halo, `ui/nameplate.rs`) :
contour lissé selon le niveau de parole, couleurs calibrées sur l'échelle
WebRTC (rouge dès 0,58, le micro local plafonnant vers 0,63), micro barré
Tabler (`microphone-off`) pour les avatars hors vocal. `voice.rs` garantit l'entrée de notre avatar à partir du
micro local, même sans écho de sa présence par le serveur vocal.

## Déroulement d'une image

`build/task_inventory.rs` centralise les permissions et les destinations du
transfert du contenu d’un objet vers l’inventaire personnel ;
`ui/inventory/task_drop.rs` fournit la source commune aux deux fenêtres de
contenu, les cibles de dépôt et la confirmation des retraits. `app/inventory.rs`
revérifie les permissions avant le message `MoveTaskInventory`, avec deux
préférences d’avertissement persistantes (objet ordinaire ou contenant des scripts).

La sélection de faces garde un masque d’indices TextureEntry par prim dans
`scene/mod.rs`. Les listes de dessin monde et HUD portent une passe de surlignage
des faces visibles, colorées par la palette, sans modifier les matériaux. Chaque dessin
conserve son véritable indice de face, même quand des faces transparentes sont
omises. Sélectionner une face opaque ne reconstruit pas ses enregistrements ;
une face totalement transparente sélectionnée reçoit temporairement un
enregistrement destiné au seul surlignage.

`aurora-render/src/selection.rs` et `shaders/selection.wgsl` calculent un halo
extérieur à partir de la couverture des triangles sélectionnés, avec la profondeur
du monde ou du HUD et la géométrie posée habituelle. Deux passes séparables sur
le GPU suivent les silhouettes, les trous et les morceaux discontinus, y compris
une face fermée sans arête de bord. Le halo est composé après le post-traitement,
sans toucher à l’intérieur des faces ; une brève teinte au clic s’efface en 0,7 s.
« Revoir la sélection » relance cette teinte sans modifier les objets.
`scene/selection_demo.rs` fournit le mesh hors ligne à îlots courbes séparés
du scénario `faces-complex`, à côté d’une primitive torique fermée.

`ui/texture_picker/tree.rs` partage le cache de recherche et l’arborescence virtuelle
de `ui/inventory/view.rs`. Les dossiers se chargent par lots bornés ; les états
dépliés de recherche et de navigation restent séparés. `ui/texture_picker.rs`
présente les lignes visibles, les noms longs tronqués avec infobulle, un aperçu
borné, une séparation ajustable et les commandes en bas de la fenêtre.
Ses choix intentionnels sont distincts de la confirmation :
`build/ui/texture_preview.rs` applique l’aperçu immédiat à la sélection capturée,
conserve les textures et matériaux d’origine de chaque face, confirme sans
renvoi avec « OK » ou restaure avec « Annuler ». Les autres utilisateurs du
sélecteur gardent leur confirmation habituelle ; les textures d’inventaire
non copiables ne produisent pas d’aperçu, comme dans Firestorm.

Le sélecteur `ui/inventory/move_to.rs` propose les destinations de
« Déplacer vers… » pour la sélection d’origine, avec filtre par nom et
chemins. La liste est calculée en parallèle, gardée par génération et dessinée
uniquement pour les lignes visibles ; le déplacement réutilise les mutations
d’inventaire existantes, avec vérification des droits à la validation.

Les changements de parent passent par `aurora-net/src/session/inventory_moves.rs`
(MoveInventoryItem / MoveInventoryFolder UDP fiables, comme Firestorm).
Les déplacements seuls publient `InventoryMoved` dès l'envoi : l'arborescence
change immédiatement, sans attendre une relecture HTTP. Le statut indique
« Déplacement demandé » ; les mises à jour serveur réconcilient le modèle.
Les mutations mêlant déplacements et métadonnées gardent leur vérification AIS.

`inventory/operations.rs` remappe les UUID des dossiers confirmés par AIS et
retourne leurs métadonnées, sans relire des dossiers encore vides. Le modèle
peut ainsi recevoir leurs items immédiatement. `session/inventory_copies.rs`
envoie les copies fiables et suit leurs confirmations ; le nom vide est encodé
avec une longueur nulle, comme `LLTemplateMessageBuilder::addString(std::string)`.
La réception des items complète la copie, sans imposer un second snapshot HTTP.
Les créations sans copie des appelants de `mutate` / `mutate_with_moves`,
notamment le remplacement de prims, gardent leur confirmation par relecture
avant de poursuivre leur travail.
Les dossiers publiés directement dans l'inventaire utilisent le nom normalisé
envoyé au serveur, avec les mêmes règles de caractères et de longueur.

`session/inventory_updates.rs` reçoit `UpdateCreateInventoryItem` et
`BulkUpdateInventory` directement par HTTP, comme `LLEventPollImpl` de Firestorm,
et les variantes LLSD de `LargeGenericMessage` / `GenericMessage`, en plus du
transport UDP. `InventoryUpdated` applique items et dossiers dès leur arrivée.
`world/inventory.rs` avance les versions des parents chargés, ignore les
réponses plus anciennes et évite de recompter les enregistrements répétés.

`ui/inventory/progress.rs` distingue la soumission d'une mutation du suivi des
copies en arrière-plan : plusieurs copies peuvent être suivies sans désactiver
les autres commandes. `InventoryProgress` transmet étapes et compteurs ; la
ligne de statut, partagée entre fenêtres, indique l'activité et le temps écoulé,
puis la confirmation de fin. Les erreurs gardent la possibilité de recharger
les dossiers concernés, sans rejouer les copies ni les déplacements.

### Surfaces des fenêtres

`world/radar.rs` tient le radar de la fenêtre Personnes (balayage des avatars une fois par seconde, première
apparition, franchissements signalés), `ui/people/radar.rs` son tableau, ses réglages et son menu d'options.
`ui/widgets/keycap.rs` dessine les touches d'un raccourci (liste des
raccourcis, panneau du micro). `ui/widgets/color.rs` est le sélecteur de couleur compact (`color_button`,
`color_panel`, conversions RVB / TSV / hexadécimal) et la pipette qu'il partage
avec l'application (`app/color_picker.rs`).
`ui/widgets/workspace.rs` partage la navigation, les panneaux de détails, les
dialogues et les lignes de l'inventaire utilisateur et des contenus d'objets
(densité, survol, sélection et style du nom). `build/ui/compact.rs` agence la
construction en six sections dans un inspecteur étroit.
`ui/color_picker.rs` compose le sélecteur commun (RVB / LSL / Hexa, TSL,
aperçus, palette de 32 cases et pipette) ; `ui/color_picker/model.rs` garde
les conversions et préférences sérialisées, `demo.rs` les gestes hors ligne.
`build/ui/color.rs` capture les couleurs par objet et face, applique les
aperçus et restaure seulement le RGB à l'annulation. Les envois intermédiaires
sont regroupés dans `build/edits.rs` jusqu'au relâchement. `app/color_picker.rs`
route la pipette vers `Scene::color_at_ray` : candidats de l'index spatial,
triangles réels en parallèle et teinte de la face, sans lecture GPU ni effet
de l'éclairage. Elle ne change pas la sélection.
Le sélecteur conserve l'identité de sa fenêtre appelante : placement contre son
bord droit, aligné en haut, ou à gauche si l'espace manque, recalculé à chaque
déplacement de la fenêtre ou changement de taille de l'écran. Sa taille mesurée
garde les commandes visibles et le déplacement conserve son état réduit.
Déplacer la palette la détache ; au relâchement, une proximité de 10 points
avec une position accolée à gauche ou à droite rétablit le suivi. Le côté
choisi reste stable tant qu'il tient à l'écran. Le lien vers la couleur d'origine
reste présent quand les fenêtres sont indépendantes. « Appliquer » est coché
par défaut, puis le dernier choix est sérialisé avec la palette enregistrée.
Les pastilles transmettent leur rectangle visible à chaque passage : un cône
dégradé les relie à la palette pendant qu'elle est déplacée séparément, sans
surface interactive supplémentaire. Un second clic sur la même pastille ferme
la transaction ; une autre pastille restaure la précédente avant de s'ouvrir.
`cursors.rs` garde le curseur pipette Tabler en mémoire, avec la pointe comme
point de prélèvement et le contour contrasté de la skin, comme les curseurs
des HUDs, de la caméra Alt et des actions de clic.
`aurora-render/src/ui_glass.rs` produit un fond dépoli partagé
par les fenêtres visibles : copie et deux passes de flou sur le GPU, puis
échantillonnage dans leurs seuls rectangles arrondis. Les ressources sont
réutilisées tant que la taille et l'échelle restent stables, puis libérées quand
aucune surface n'en a besoin. Le monde autour des fenêtres reste net. Une skin
avec `panel_alpha = 255` conserve des fenêtres opaques sans ces passes. Une
surface GPU qui ne permet pas la copie garde le cadre teinté ; les marqueurs
restent sans dessin pour préserver les indices du flou des modales.

L'affichage des permissions d'éléments est partagé par l'inventaire, la fenêtre
de contenu d'un objet et l'onglet Contenu de construction dans
`ui/inventory/permissions.rs` : indicateurs compacts C·M·T, droits accordés verts
et droits refusés rouges et barrés, couleurs issues de la palette du thème,
détail du propriétaire actuel et du prochain propriétaire au survol ; les
liens d'inventaire reprennent les droits de leur original.

Les prims flexibles ont leur simulation propre (`aurora-prim/src/flexible.rs`,
port de `LLVolumeImplFlexible`) et une géométrie unique par objet. La scène
(`scene/flexible.rs`) calcule en parallèle les forces, le chemin déformé,
les normales, les limites et les triangles de sélection ; elle réécrit
seulement les sommets dans leurs allocations GPU existantes. Le vent de chaque
région est reçu par `LayerData` (deux patches DCT), interpolé comme `LLWind`.
Les prims flexibles sont dans l'ensemble des objets actifs, même sans mouvement
réseau ; les changements d'origine décalent leur simulation avec le monde.
Le contour des flexibles sélectionnés est préparé sur le même worker depuis
les sommets simulés (`build/silhouette.rs`). Les outils de déplacement et de
rotation utilisent leur ancrage négatif en Z ; la boîte et les poignées de
taille gardent les dimensions droites, comme `LLManipScale`. Modifier la taille
ou la souplesse remappe la chaîne existante au lieu de la réinitialiser droite.

Le rez depuis l’inventaire passe par `ui/inventory.rs` (charge utile et relâché avant la fin de la passe egui), `app/inventory_drag.rs` (règles, groupe, préparation des commandes et consommation locale des exemplaires non copiables), `Scene::inventory_surface` (triangles parmi les candidats de l’index, terrain, sans attente GPU) et `aurora-net/src/inventory/rez.rs` (message fiable `RezObject` dans la région visée). Les pixels transparents sont testés avec un masque d’un bit par bloc 2×2, préparé par les tâches de décodage ; les surfaces invisibles et riggées sont écartées. Le curseur garde le résultat pendant 100 ms au plus quand le rayon ne bouge pas ; le relâché recalcule toujours la surface. Ctrl sur un objet envoie `UpdateTaskInventory`. La démo utilise un cube synthétique comme réponse au rez.

1. **Réseau → monde.** `aurora-net` décode les messages sur son runtime et les
   envoie en `NetEvent` ; `World::apply` met à jour l'état (objets, avatars,
   animations, chat…).
2. **Entrées → agent.** Les touches deviennent des drapeaux de contrôle ;
   `AgentState` et la caméra avancent ; l'`AgentUpdate` est envoyé.
3. **Monde → scène.** Les poses des avatars sont calculées (contrôleur de
   mouvements façon `LLMotionController`), puis les glissements des outils
   de construction appliquent leur transformation locale, après les réponses
   du simulateur et avant `Scene::sync`. Ainsi, l'objet affiché et ses poignées
   suivent la même taille à chaque image, sans retour à un écho réseau ancien.
   Lorsque le focus appartient à la sélection en édition, la caméra est
   calculée après ce glissement, avant les matrices de rendu et le culling.
   La bascule « Suivre l’objet en édition » est désactivée par défaut :
   point de focus et cadrage restent fixes au centre de l'écran ; pendant
   un déplacement au bord, `build/edge_pan.rs` fait glisser la caméra
   progressivement sur l'axe ou dans le plan de la poignée. La projection
   inverse tient compte de la perspective ; la caméra avance avant le
   solveur de déplacement, qui applique la même translation à l'objet dans
   l'image courante. Leur distance ne dérive donc pas pendant le maintien
   du curseur au bord (sans effet sur les HUDs).
   Activée, la bascule suit la transformation locale. La projection du
   curseur est alors conservée au début du glissement
   pour que le mouvement de caméra ne modifie pas l'objet sans saisie.
   `Scene::sync` met à jour la
   géométrie, les textures et les enregistrements GPU de ce qui a changé ou
   bouge (ensembles tenus par événements, placement calculé en parallèle).
   Les déplacements et les avatars sont appliqués à chaque image ; les
   synchros complètes (objet nouveau, géométrie liée, faces reconstruites)
   et la libération des objets retirés tiennent dans un budget de temps :
   le reste attend quelques images, le plus proche de la caméra d'abord.
4. **Scène → GPU.** La synchro tient à jour sur le GPU une table des faces
   (une entrée par enregistrement de dessin, avec sa passe) et une table des
   objets (sphère englobante, drapeaux, état des avatars). `build_lists` ne
   prépare sur le CPU que ce que le GPU ne fait pas : terrain, eau, faces
   transparentes triées de l'arrière vers l'avant, glow, imposteurs,
   sélection et listes de débogage (seuls les objets qui en ont sont
   parcourus). `aurora-render` lance le culling en compute (vue principale,
   cascades d'ombres, reflets, sonde ; occlusion Hi-Z en deux phases), puis
   encode les passes (ombres, prépasse, scène, eau, reflets,
   post-traitement) avec des draws indirects dont le nombre est écrit par le
   GPU. Sans `MULTI_DRAW_INDIRECT_COUNT` (ou avec `AURORA_CPU_CULL=1`),
   `build_lists` prépare toutes les listes sur le CPU comme avant. La passe
   de scène part toujours d'une profondeur déjà écrite, pour ne calculer la
   couleur que de ce qui se voit : celle de la prépasse à un échantillon
   sans MSAA ; avec le MSAA, celle d'une seconde prépasse dans sa propre
   profondeur multi-échantillonnée (terrain et faces opaques, les dessins
   mêmes de la passe de scène ; les faces masquées par alpha, dont le bord
   vient alors de l'`alpha to coverage`, n'y sont pas). Les
   sommets des objets et du terrain sont calculés relativement à la caméra
   (les objets et le terrain comme les prépasses de profondeur, opération
   pour opération : la passe de scène les teste contre cette profondeur, et
   un écart d'un arrondi montrerait le ciel à travers le sol) : le vertex
   shader retire sa position de la translation du modèle (les
   enregistrements restent en coordonnées de région), et la matrice de vue
   de la caméra principale se réduit à sa rotation (`eye_view_proj` dans
   `renderer.rs`) ; en altitude, la simple précision ne suffit pas en
   coordonnées de région. Le thread
   principal s'arrête à la clôture de l'image (`Renderer::render`) : le
   reste de cette étape est fait par le thread de rendu à partir du paquet
   d'image, pendant que l'image suivante commence (voir « Threads et
   propriété des données »).
5. **Interface.** egui compose les fenêtres sur le thread principal ; ses
   formes partent telles quelles dans le paquet d'image, et le thread de
   rendu les découpe en triangles (`aurora-render/src/ui_frame.rs`) puis les
   dessine par-dessus la scène. Les étiquettes des avatars, les textes
   flottants et les points des cartes lisent les listes d'avatars et d'objets
   à texte de l'`ObjectStore` (`world/objects.rs`), tenues à jour par les
   changements : aucune fenêtre ne parcourt tous les objets à chaque image.

Les autres passes de l'image suivent la même règle. Les systèmes de
particules et les sons attachés qu'apportent les mises à jour d'objets sont
relevés par deux flux de l'`ObjectStore` (`take_particle_changes`,
`take_sound_changes`) ; les miroirs sont listés par la synchro comme les
lumières ; l'orientation des corps et le chargement des avatars lisent la
liste des avatars.

Les traitements lourds (décodage d'images, maillage, sons) passent par des
tâches en arrière-plan (`scene/jobs.rs`, rayon, en priorité basse) pour
garder l'image fluide. Ce qu'elles produisent pour le GPU (chaînes de mips,
sommets et indices), elles l'écrivent elles-mêmes dans la mémoire de transit
du rendu (`aurora-render/src/upload.rs`) : le thread principal n'enregistre
que les copies, les plus visibles d'abord, dans un budget d'environ 1 ms par
image (`scene/textures.rs`, `Scene::process_results`). L'entretien se fait par
files d'attente, sans passe périodique sur toutes les textures (éviction de
quelques textures inutilisées par image, données téléchargées à écrire
notées à leur arrivée), et la mémoire que le thread principal rend (niveaux
décodés, données J2C, triangles de picking) est libérée par un thread à
part (`Jobs::discard`) : pendant que les tâches allouent sur tous les
cœurs, la libérer sur place coûtait plusieurs millisecondes.

## Threads et propriété des données

Une image coûte le plus long des deux threads, pas leur somme : le thread
principal simule et prépare l'image N+1 pendant que le thread de rendu
dessine l'image N. `AURORA_RENDER_THREAD=0` exécute le même paquet sur le
thread principal (comparaison, débogage).

| Thread | Ce qu'il possède |
|---|---|
| **Principal** (boucle winit, `App::frame`) | Le monde, la scène, l'interface egui (jusqu'aux formes à dessiner, non tessellées), la fenêtre. Dans `aurora-render`, le `Renderer` (`main_thread.rs`) : *quoi* dessiner — enregistrements de dessin et table des faces (`RecordStore`), table des objets du culling (`CullTables`), allocateurs de l'arène de géométrie, pages, emplacements et positions des textures (`TextureTable`), palettes d'os et liaisons de skin, mémoire de transit et copies en transit (`UploadQueue`), réglages et taille de la fenêtre. Il crée des ressources GPU (la création n'a pas d'ordre) mais **n'écrit jamais dans la file GPU et ne soumet rien** |
| **Rendu** (`aurora-render`, `render_thread.rs`) | Le `Backend` (`renderer.rs`) : *comment* dessiner — surface et swapchain, pipelines, cibles de rendu, uniformes de l'image, tessellation des formes d'egui (`ui_frame.rs`) et renderer egui, culling compute et ses cases, occlusion, sondes, atlas des imposteurs, relectures (profondeur sous le curseur, compteurs du culling, chronos GPU, captures). **Seul utilisateur de la file** (`write_*`, `submit`, `present`) ; deux threads auxiliaires finissent les encodeurs en parallèle (`helpers.rs`) |
| Aides de l'image (pool rayon global, `aurora-worker-<i>`) | Rien : elles prêtent main-forte au thread principal dans ses boucles parallèles (`frame_par.rs`), sur les données qu'il leur prête le temps de la boucle. Le thread principal fait sa part sans les attendre : une aide réveillée trop tard ne retarde pas l'image |
| Tâches de fond (`scene/jobs.rs`, priorité basse) | Décodages, maillages ; écrivent dans la mémoire de transit mappée, jamais dans la file |
| Cache des textures transcodées (`aurora-texcache`, `scene/texture_cache.rs`) | Écrit les fichiers `.bc7` que les tâches de décodage lui remettent : créer un fichier prend des millisecondes, que les tâches ne passent pas à attendre le disque. File bornée à 256 Mo ; au-delà, le fichier n'est pas écrit et la texture sera décodée de nouveau |
| Libérations (`aurora-trash` : `Jobs::discard`) et pages de textures (`aurora-pages`, `textures.rs`) | Libèrent hors du thread principal la mémoire et les pages de textures que plus rien n'utilise. Une page n'est remise à `aurora-pages` qu'après la reconstruction du groupe de textures qui la contenait ; les poignées étant comptées, une image encore en cours sur le thread de rendu (son paquet tient l'ancien groupe) la garde en vie jusqu'à sa fin, et ces threads ne touchent jamais la file. `aurora-pages` crée aussi les pages des textures en streaming (une page qui demande un nouveau bloc de mémoire vidéo attend le pilote 1 ms ou plus) : le thread principal demande la page d'une taille en avance sur les envois en file pour elle (`TextureTable::prepare_page`) et la reçoit prête, l'envoi qui arrive avant elle attend une image |
| Réseau (`aurora-net`, runtime tokio) | Circuits, capabilities, téléchargements |

Ce qui passe d'un thread à l'autre :

- **Journal d'écritures** (`writes.rs`). `Queue::write_*` prend effet au
  prochain `submit`, quel que soit le thread qui l'appelle : une écriture
  faite directement par le thread principal pour l'image N+1 partirait avec
  l'image N encore en cours. Tout ce que les stores de la scène veulent
  écrire (plages modifiées des miroirs CPU, géométrie, texels, copies à
  soumettre en cours d'image comme l'agrandissement d'une arène) est donc
  enregistré, dans l'ordre, dans le journal de l'image en construction ; le
  thread de rendu rejoue le journal de l'image N juste avant d'encoder
  l'image N. Une image est toujours dessinée avec les données d'une seule
  image, entières, et les envois gardent leur ordre par rapport aux dessins.
- **Paquet d'image** (`packet.rs`), un par image, autonome : le journal,
  l'encodeur des copies en transit et les morceaux de mémoire de transit à
  remapper après la soumission, les poignées des tampons de la scène tels
  qu'ils sont pour cette image (le thread principal peut les remplacer
  ensuite), caméra et environnement (`FrameParams`), listes de dessin CPU
  (`DrawLists`, copiées) et vue du culling GPU, formes d'egui encore à
  tesseller et changements de ses textures (`ui_frame.rs`), taille de la
  fenêtre / vsync / réglages à appliquer, demandes de relecture (pixel
  survolé, pixels cliqués, capture).
- **Remise en rendez-vous** (`render_thread::handoff`) : le thread de rendu tient au plus un
  paquet, aucun n'attend derrière. Si le rendu est en retard, le thread
  principal attend (contre-pression) au lieu d'empiler des images ; chaque
  côté mesure son attente (`wait_render`, `rt_idle` du profil).
- **Résultat d'image**, rendu à la remise suivante : statistiques du rendu,
  réponse au survol (gardée avec la caméra de l'image qui l'a demandée),
  réponses aux clics, compteurs du culling, capture, tampons du paquet à
  réutiliser.

Une seule chose attend le thread de rendu : l'image qui porte une capture
(même image capturée qu'avant, lue juste après). La profondeur sous un clic
ne l'attend plus (`Renderer::request_pick`) : la demande part dans le paquet
de l'image suivante, le thread de rendu copie le texel avant d'encoder cette
image — la profondeur et la caméra sont encore celles de l'image sur
laquelle le clic a été fait — dans des commandes soumises à part, et la
réponse revient avec un résultat d'image, une à trois images plus tard (le
résultat d'une image que le thread de rendu a déjà finie est pris sans
attendre la remise suivante)
(`aurora-render/src/pick.rs`, `aurora-viewer/src/app/click_pick.rs`).
Le même clic demande aussi ce qui est dessiné sous son pixel
(`Renderer::request_drawn`, `aurora-render/src/drawn_pick.rs`) : l'image
qui porte la demande redessine les faces de la vue par une caméra resserrée
sur ce pixel et ses voisins, avec le vertex shader de la scène, dans une
cible de 2 × 2 texels (enregistrement de dessin, profondeur, coordonnées de
surface) : faces opaques et masquées, puis faces transparentes devant
elles. Rien n'est dessiné ni alloué par image hors d'un clic.
L'ancienne lecture, qui attendait l'image en cours puis tout ce que le GPU
avait en file (`Renderer::pick_world`), ne sert plus qu'à la comparaison
(`AURORA_CLICK_PICK=wait` ou `check`). Si le thread de rendu disparaît
(panique), toute attente se termine, les clics en attente reçoivent une
réponse vide et le viewer se ferme proprement.

Les HUDs portés passent par `scene/hud.rs` (listes de notre avatar, intérêt des textures, picking de triangles) et la projection partagée `aurora-render/src/hud.rs`. Les points d’attachement dans `Scene::object_transform` sont relatifs à l’écran, avec l’aspect de `mScreen`, sans position ni orientation de l’avatar. Le renderer dessine les faces opaques / masquées puis transparentes après le post-traitement, dans une profondeur indépendante, avant egui ; il réutilise la géométrie, les textures bindless, les matériaux et les animations UV du monde. `demo_hud.rs` fournit les mises à jour et réponses de toucher hors ligne.

L’édition des HUDs réutilise les outils de construction : `build/geom.rs` adapte leur caméra à la projection orthographique HUD (rayons parallèles, poignées de taille constante en pixels), `build/mod.rs::edit_parent` fournit la transformation du point d’attachement pour convertir les déplacements en coordonnées locales. Sélections HUD et monde restent séparées. `build/manip.rs` envoie les positions locales au relâché avec le drapeau de linkset ; l’onglet Objet utilise lui aussi les valeurs locales pour les HUDs. Le déplacement direct avec la touche choisie (ALT par défaut) partage ce même edit_parent et le picking des faces visibles, avec les exceptions double face. Préférences › Raccourcis › HUDs utilise `ui/options.rs` et l’action `keybinds::Action::HudDrag` : les deux touches partagent la capture, l’effacement, les conflits et la réinitialisation des autres raccourcis. Les modificateurs seuls sont capturés au relâchement et leurs côtés gauche / droite sont équivalents. `Settings::sanitized` migre l’ancien `hud_drag_key` vers la touche principale sans écraser les deux choix déjà enregistrés ; `app/hud_drag.rs` utilise ces raccourcis pour le geste et le curseur.

Les faces HUD tournées dos à l’écran sont rejetées dans `fs_hud`, avant toute couleur ou écriture de profondeur. Le drapeau de matériau `DOUBLE_SIDED` conserve l’exception glTF explicite sans casser l’ordre des faces translucides. Le picking HUD suit le même sens des triangles et les matériaux / overrides double face ; les faces arrière invisibles n’interceptent pas les boutons visibles.

La projection et les listes HUD font partie de la copie du paquet d’image (`DrawLists::copy_from`) : le thread de rendu possède sa propre copie pendant que le thread principal garde la projection pour le picking.

Les menus du monde partagent un modèle d'entrées dans `ui/context/entries.rs`
(actions, droits, sous-menus, coches). `ui/menu.rs` dessine la liste existante ;
`ui/context/pie.rs` dispose ces mêmes entrées en huit secteurs, avec les directions
de Firestorm et des sous-menus « Plus » pour les actions supplémentaires. Les
secteurs non survolés reprennent la transparence des fenêtres du thème. Le clic
gauche sélectionne ; le clic droit remonte d'un niveau (ferme à la racine). Le
chemin de navigation reste dans le menu ouvert et se réinitialise à sa fermeture.
`Settings::use_pie_menu` conserve le choix de Préférences › Interface ; la liste
reste le choix initial d'Aurora.

## Données sur la machine

Le zoom local propre à chaque HUD est tenu par `world/hud_scale.rs` : profils `hud_scales.json` dans le dossier du compte, identité par nom d’inventaire et créateur, écriture atomique sur un thread auxiliaire et copies portées indépendantes. `app/hud_drag.rs` réserve la molette avec la touche de déplacement, dessine le cadre / pourcentage et garde une partie du HUD accessible. En édition avec une sélection HUD, la molette sans cette touche change uniquement `World::hud_zoom`, projection commune à tous les HUDs, comme LLAgentCamera ; elle précède les médias et la caméra même dans une zone vide. Ce zoom de vue ne modifie pas les facteurs ni les décalages locaux. La racine conserve son origine à l’écran ; les offsets liés et les matrices de placement / picking suivent le facteur uniforme. Les outils de construction reconvertissent les positions liées, et les informations de toucher envoyées au serveur retrouvent les coordonnées originales. Dimensions et positions serveur ne sont pas modifiées par le zoom ; l’éventuel décalage pour garder le HUD accessible reste local.

### Récupération après plantage

`aurora-viewer/src/crash/` conserve un dossier par session (UUID), avant
l'initialisation du réseau et du rendu. `mod.rs` gère les instantanés
(version, commit, langue, thème), la trace Rust et la récupération au lancement
suivant ; `native.rs` installe le filtre d'exception Windows et surveille
la fin du processus par une poignée système, avec contrôle de sa date de
création pour éviter la réutilisation d'un PID. Le surveillant est le même
exécutable, démarré en mode interne avant toute initialisation du viewer.
Son dossier de travail est celui des rapports, pour laisser l'updater
remplacer le dossier d'installation après l'arrêt du viewer. Il ne relance
jamais le viewer.

`window.rs` réutilise Fluent, la Palette, les polices, les icônes Tabler
et les boutons du viewer. `software.rs` dessine ses maillages egui sur le
CPU et `softbuffer` présente les pixels, sans initialiser Vulkan ni aucun
adaptateur graphique. La boucle attend les événements entre les images.

`report.rs` compose le brouillon depuis cet instantané et les marqueurs de
la session arrêtée. « Envoyer un rapport de plantage » ouvre le même
`ui/bug_report.rs` que le viewer, avec ses cases, son aperçu et ses modes
d'envoi GitHub. Le formulaire reçoit le dossier de la session ; il ne
relève pas la configuration ou le journal du processus de récupération.
Les workers lisent l'extrait filtré et préparent les fichiers pendant que
la fenêtre attend les événements. Le ZIP privé facultatif contient aussi
les traces Rust / native, jamais dans le texte public de l'issue.

Le journal est écrit également dans le dossier propre à la session : ni
la rotation du journal ordinaire ni une autre instance ne peuvent le
remplacer. Les traces et journaux complets restent locaux. Un verrou de
fichier empêche deux fenêtres de présenter le même rapport ; une seule
tentative de récupération automatique est faite au lancement suivant si
la première fenêtre n'a pas pu s'ouvrir. Les rapports confirmés restent
sur disque ; les dossiers des sessions terminées normalement sont retirés.

La surveillance immédiate et la capture native concernent Windows
(plateforme des releases). Une trace native contient le code d'exception,
l'adresse fautive et les adresses de la pile ; une trace Rust contient
les symboles disponibles. Une pile endommagée ou un arrêt forcé peut ne
laisser aucune trace, ce que la fenêtre explique explicitement.

Les rapports sont sous `logs/crashes/sessions/<uuid>/`, et les essais
hors ligne sous `logs/crashes/demo/<uuid>/` ; `session.json`, `session.log`,
`panic.txt`, `native.txt` et les marqueurs appartiennent à une seule
exécution. Les arguments internes `--crash-watch` et `--crash-show`
n'acceptent que les dossiers UUID immédiatement sous cette racine.

| Quoi | Où (Windows) |
|---|---|
| Réglages, skins, disposition des fenêtres | `%APPDATA%\Aurora\AuroraViewer\config\` |
| Cache (textures, mesh, animations, inventaire…) | `%LOCALAPPDATA%\Aurora\AuroraViewer\cache\` |
| Logs (`aurora.log`, `aurora-demo.log`, `aurora-demo-<titre>.log`) | `%LOCALAPPDATA%\Aurora\AuroraViewer\data\logs\` |
| Mot de passe retenu (empreinte, jamais le mot de passe) | Gestionnaire d'identification Windows, service « Aurora Viewer » |
