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
│  ├─ phosphor-icons/           bibliothèque Phosphor complète (SVG, 6 graisses)
│  └─ emoji/                    police emoji (téléchargée, hors git)
├─ crates/                      le code (voir ci-dessous)
├─ docs/                        BRANDING.md, TESTING.md (options de test `AURORA_*`, profilage), captures du README
├─ scripts/                     outils des agents et des humains (PowerShell)
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
| `aurora-render` | Moteur de rendu wgpu / Vulkan : textures bindless regroupées en pages (texture arrays), géométrie sous-allouée, multi-draw-indirect, ombres, reflets, post-traitement ; animations de texture (`tex_anim.rs` : référence CPU et paramètres des enregistrements, évaluées par les vertex shaders) ; listes de dessin pilotées par le GPU (`gpu_cull.rs`, `shaders/cull.wgsl` : tables des faces et des objets tenues par la scène, culling et compactage en compute pour chaque vue, dessin par `multi_draw_indexed_indirect_count`) ; occlusion Hi-Z en deux phases (`occlusion.rs`, `shaders/occlusion.wgsl`, test commun `shaders/hiz_test.wgsl`) ; envois du streaming sans copie sur le thread principal (`upload.rs` : mémoire de transit mappée où écrivent les tâches de fond, copies GPU enregistrées dans un encodeur soumis en tête de l'image) ; rien qui grandisse ou se libère d'un coup dans une image (`arena.rs` : plages de géométrie allouées au meilleur ajustement par un index des tailles, enregistrements de dessin gardés par blocs et tampon GPU agrandi par une copie GPU ; `textures.rs` : substitut gris partagé par les textures en attente, pages vidées détruites quelques-unes par image par un thread à part) ; deux moitiés, voir « Threads et propriété des données » : `main_thread.rs` (le `Renderer` vu par la scène : ses stores, la clôture de l'image), `writes.rs` (journal des écritures GPU de l'image), `packet.rs` (paquet d'image et résultat), `render_thread.rs` (thread de rendu, remise des paquets en rendez-vous), `renderer.rs` (`Backend` : surface, pipelines, cibles, passes), `helpers.rs` (threads auxiliaires de `finish`) |
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

| Dossier / fichier | Contenu |
|---|---|
| `main.rs`, `app.rs`, `app/` | Démarrage, boucle d'événements, une image du viewer, actions de l'interface ; commandes de la barre de chat (`app/chat_commands.rs`), menus clic droit (`app/context_menu.rs` : ouverture, droits sur l'objet, actions choisies), geste de déplacement des HUDs avec touche configurable (ALT par défaut) et clic gauche et scénario hors ligne (`app/hud_drag.rs`) |
| `cli.rs` | Arguments de ligne de commande (`--title`, `--fps-limit`, `--language`, `--version`) ; titre avec la version commune et le profil de compilation |
| `cmdline/` | Barre de chat comme ligne de commande (FSCmdLine) : analyse des commandes, calculatrice (LLCalc), dés ; exécutées par `app/chat_commands.rs` |
| `agent.rs` | Notre avatar (AgentUpdate, extrapolation depuis vitesse / accélération serveur, lissage comme LLDrawable) |
| `ao/`, `app/ao.rs`, `ui/ao.rs` | Animation Overrider intégré : catalogue des ensembles `#Firestorm/#AO`, liens d’inventaire et import ZHAO (`inventory.rs`), états / cycles / groupes / pistes et remappage de la seule pose de notre avatar (`runtime.rs`), assets et transitions synthétiques (`demo.rs`) ; opérations AIS et réponses asynchrones dans `app/ao.rs`, fenêtre de gestion ouverte par le bouton AO de la barre du bas. Les signaux bruts du simulateur restent utilisés pour le mouvement et les transitions ; les remplacements sont annoncés par AgentAnimation. Activation et dernier ensemble enregistrés par UUID de compte ; pause et poses debout propres à la session. |
| `camera/` | La caméra comme LLAgentCamera : vue derrière l'avatar, caméra Alt sur un point ou un objet (`focus.rs`), caméra de siège, vue subjective, transitions, lissage, recul en vol, collision envoyée par le simulateur ; réglages Firestorm (`settings.rs`), scénarios de démo (`demo.rs`) |
| `world/` | État du monde reçu du réseau : objets, régions, terrain, environnement (EEP : cycles et assets de réglages dans `eep.rs`, choix local > parcelle > région > jour par défaut, environnement local et fondus dans `eep_env.rs`, listes du sélecteur d'environnement et chargement des dossiers dans `env_select.rs`), parcelle sélectionnée d'« À propos du terrain » et ses droits (`land.rs`), social, profils des avatars, inventaire, groupes (actif, épinglés), cercles de contacts et surnoms (`contact_sets.rs`), blocages, messages du bridge LSL de Firestorm (cachés), orientation des corps, regard (LookAt), carte, profils de lieux de « Lieux » et des fenêtres séparées (`place_details.rs` : région par son nom, repère, RemoteParcelRequest, ParcelInfoRequest), assets des repères et handles de leurs régions (`landmarks.rs`), historique de téléportation daté et enregistré (`tphistory.rs`)… |
| `scene/` | Ce qu'on envoie au GPU : géométrie et LOD, textures (streaming), avatars (squelette, animations, silhouette), particules, eau, sondes de reflets, sons du monde, imposteurs, assets de réglages d'environnement (`settings.rs` : ciel, eau, cycle du jour) |
| `ui/` | Interface egui : barres, fenêtres, options, chat, contacts (amis, groupes, cercles : `contacts.rs`), profils des avatars, « Lieux » (`places.rs` : favoris, repères, historique de téléportation) et profils de lieux (`place_details.rs`, aussi en fenêtres séparées), « À propos du terrain » (`land/`, un fichier par onglet, dialogues), choix d'un résident (`avatar_picker.rs`), page web dans une fenêtre (`web_view.rs`), choix d'une texture, inventaire, sélecteur d'environnement et « Éclairage personnel » (`environment.rs`), cartes, overlays de débogage ; sons causés par les widgets (`sound_cues.rs` : clics, touches refusées, fenêtres) ; menus clic droit : apparence commune (`menu.rs` : icône Phosphor, sous-menus, cases cochées) et entrées comme Firestorm (`context.rs` : monde, noms d'avatars, groupes) |
| `build/` | Outils de construction comme LLFloaterTools : sélection et manipulateurs (`manip.rs`), déplacement direct des HUDs (`hud_drag.rs` : racine du linkset, conversion écran / point d’attachement, écho local et MultipleObjectUpdate de position au relâchement), outils Déplacer (`grab.rs`) et Aligner (`align.rs`), terrain (`land.rs`), modifications de la sélection (`edits.rs`), matériaux et médias (`materials.rs`), contenu des objets (`contents.rs`), impact et poids (`costs.rs`), simulateur de démo (`demo_sim.rs`) ; la fenêtre et ses onglets dans `build/ui/` |
| `interaction.rs`, `cursors.rs`, `ui/object_actions.rs` | Règles des actions de clic 0–9, héritage, permissions, curseurs natifs Firestorm, fenêtres d'achat / paiement et liste du contenu ; transaction après confirmation |
| `app/object_actions.rs` | Déclenchement des actions, toucher maintenu, déplacement physique, lecture de parcelle, ouverture de média et cadrage de caméra |
| `scene/picking.rs` | Rayons contre les triangles partagés avec la géométrie affichée (prims, sculpts, meshes) ; prim réellement visée au survol et au clic gauche malgré des boîtes recouvrantes, IGNORE traverse la géométrie hors construction, informations de surface pour les scripts de toucher ; les mêmes tests sur les seuls candidats de l'index de survol |
| `scene/hover.rs` | Objet sous le curseur sans recherche à chaque image : index compact des sphères englobantes tenu par la synchro (candidats proches du rayon du curseur ou du point de profondeur), réponse gardée tant que le rayon, la profondeur lue sur le GPU et les objets alentour ne changent pas, refaite au moins toutes les 100 ms ; jamais pour les clics. Contrôle : `AURORA_HOVER_CHECK` |
| `media/` | Médias des prims et des parcelles (objets à médias tenus à jour par le flux de changements de l'`ObjectStore`, sans passe sur tous les objets), cookie OpenID des pages web de la grille (`openid.rs`) |
| `demo.rs`, `demo_land.rs`, `demo_place.rs`, `demo_eep.rs`, `demo_env.rs`, `demo_stream.rs` | Le mode démo : une scène locale qui simule un serveur (et ses réponses à « À propos du terrain », aux profils de lieux, repères et historique de « Lieux » et à ExtEnvironment, une bibliothèque d'environnements pour le sélecteur) ; `demo_stream.rs` : objets et textures qui arrivent par vagues (test des à-coups du streaming, AURORA_DEMO_STREAM) |
| `settings.rs`, `keybinds.rs`, `keybinds/layout.rs`, `theme.rs` | Réglages enregistrés, raccourcis, disposition Windows et touches de déplacement par défaut, palette |
| `ui_sound.rs` | Catalogue des sons de l'interface (UISnd* de Firestorm), réglages par son |
| `ui/appearance.rs`, `ui/appearance/items.rs`, `ui/appearance/gallery.rs`, `world/appearance.rs` | Fenêtre Apparence, galerie / tenues / portés, édition et dialogue Enregistrer sous ; menus des éléments et de la galerie, confirmation de sauvegarde / suppression, choix d’image, renommage ; points d’attachement / HUD, profil et original ; règles de changement du COF, sauvegarde par liens, protection des parties du corps, scénario hors ligne |
| `aurora-net/src/outfits.rs`, `aurora-net/src/outfits/categories.rs` | Écriture des liens de tenue, favoris, noms et images par AIS InventoryAPIv3, déplacement dans la corbeille ; confirmation et relecture des dossiers / éléments |
| `slurl.rs`, `link_trust.rs` | Liens des textes : SLURL (barre de navigation, libellés « Région (x,y,z) » des liens de lieu) ; confiance des liens web (site de confiance, inconnu, dangereux : raccourcisseurs, faux noms officiels, adresses IP…), jugée localement sur l'URL |
| `ui/widgets.rs`, `ui/widgets/resize.rs`, `ui/widgets/demo.rs` | Widgets communs et fenêtres flottantes ; repli par double-clic du titre ou bouton, taille développée conservée et barre repliée déplaçable ; gestes hors ligne sur le titre réel ; poignées de redimensionnement bornées avant le déplacement, bord opposé fixe, limites du contenu et de la zone visible ; position et taille conservées par egui |
| `ui/exit.rs` | Modale de fermeture traduite : quitter, déconnecter vers le login, annuler ; fond flouté sur le GPU et interactions bloquées ; attente non bloquante de la fin de session, avec le délai de secours de six secondes de Firestorm |
| `updater.rs`, `ui/update.rs` | Contrôleur asynchrone et fenêtre de mise à jour : notes, progression, relance et erreurs ; recherche manuelle dans Aide / connexion, tag ignoré et choix Demander / Automatique / Obligatoires uniquement. Démo synthétique isolée via `AURORA_DEMO_UPDATE`. |
| `ui/inventory.rs`, `ui/inventory/view.rs`, `ui/inventory/controls.rs`, `ui/inventory/context.rs`, `ui/inventory/dialogs.rs`, `ui/inventory/properties.rs`, `ui/inventory/thumbnail.rs`, `app/inventory.rs`, `app/inventory_thumbnail.rs` | Inventaire : tri / filtres purs et cache de vues par génération dans `view.rs` (index, tri et filtrage parallèles avec rayon), commandes et préférences persistantes dans `controls.rs`, arborescence filtrée incluant les dossiers parents, états de dépliage par onglet, résultats dessinés selon la zone visible sans limite de 500 et zone de statut de hauteur fixe ; défilement sur les deux axes, largeur des noms mesurée dans la zone visible puis conservée pour les résultats virtuels, commandes adaptées aux petites largeurs ; menus par type, sélection multiple avec ajout / détachement groupés, renommage dans la ligne après création ou sur demande, fenêtres de dossiers limitées à leurs descendants avec un filtre propre, presse-papiers partagé entre fenêtres, propriétés et confirmations, aperçus / édition des notes et scripts, lecture des statistiques d’animation ; éditeur d’image à six actions, choix en arbre et capture de la scène sans interface ; décodage / redimensionnement / encodage hors du thread de rendu et exécution des actions après dessin de l’interface |
| `world/inventory/actions.rs`, `world/inventory/wearable.rs`, `world/inventory/demo.rs`, `world/inventory/preview.rs`, `world/inventory/tests.rs` | Règles de permissions, déplacements / copies / liens et offres, préparation des annonces Place du marché, assets de vêtements par défaut et scénario synthétique ; cache d’inventaire v2 avec permissions complètes |
| `aurora-net/src/inventory/operations.rs`, `aurora-net/src/inventory/thumbnail.rs`, `aurora-net/src/session/inventory_upload.rs` | Mutations AIS avec relecture, remappage des UUID attribués par le serveur, créations / copies UDP et accusés avec expiration, sauvegarde des documents par capabilities ; chargement AssetUpload / Xfer des vêtements avant création de l’élément ; vignettes gratuites par InventoryThumbnailUpload, POST JPEG2000 puis AIS et relecture |
| `aurora-assets/src/j2k/encode.rs` | Encodeur OpenJPEG borné en mémoire, vignettes RGB / RGBA carrées de 64 à 256 pixels et conservation de l’alpha |
| `logging.rs`, `cache.rs`, `credentials.rs` | Logs, cache disque, mot de passe retenu (coffre de l'OS) |
| `bug_report/`, `ui/bug_report.rs`, `ui/bug_report/tests.rs`, `app/bug_report.rs` | Rapport de bug : instantané explicite des données autorisées, formulaire avec onglets Description / Image / Données, valeurs localisées et décochables, export aux titres anglais ; diagnostics publics à vocabulaire fermé et images réencodées hors du thread principal. Export local avec dossiers public / privé séparés ; lien GitHub prérempli avec repli presse-papiers, ou création d’issue après Device Flow d’une GitHub App. `uploads.rs` déclare les empreintes et envoie l’image publique et le ZIP privé facultatif vers l’API PHP / Cellar, puis ajoute les liens ; échec partiel repris sur la même issue. L’autorisation GitHub reste uniquement en mémoire dans `github.rs` et n’est pas transmise à l’API Aurora. Aucun secret de connexion SL, chat ou IM n’entre dans le modèle public. |
| `frame_profile.rs` | Profil des images (AURORA_PROFILE) : temps de chaque étape de l'image, ligne de synthèse par seconde dans le log (moyenne et maximum de chaque étape, nombre d'images lentes ; détail du streaming sur le thread principal, `s_*` ; rencontre du thread principal et du thread de rendu, segment `thread`) |
| `scene/animesh.rs` | Squelettes autonomes des objets animés, animations du linkset, limites des poses pour le culling et les ombres, scénario de démo |
| `scene/sync_sets.rs` | Objets que la synchro de la scène visite à chaque image, tenus à jour par événements (objets modifiés notés par `ObjectStore`, ensemble des objets qui bougent d'eux-mêmes et de ce qui les suit, géométries en attente, tranche de LOD) au lieu d'un parcours de tous les objets ; file d'attente des synchros complètes qui n'ont pas tenu dans le budget de temps de l'image, servie du plus proche au plus lointain (`Backlog`, `FullSyncBudget`) |
| `scene/sync_plan.rs` | Placement en parallèle (rayon) des objets de l'image, niveau par niveau des chaînes de parents : transformation, LOD, limites ; mise à jour de la seule matrice ou synchro complète |

## Déroulement d'une image

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
   mouvements façon `LLMotionController`), puis `Scene::sync` met à jour la
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
   `build_lists` prépare toutes les listes sur le CPU comme avant. Le thread
   principal s'arrête à la clôture de l'image (`Renderer::render`) : le
   reste de cette étape est fait par le thread de rendu à partir du paquet
   d'image, pendant que l'image suivante commence (voir « Threads et
   propriété des données »).
5. **Interface.** egui dessine les fenêtres par-dessus.

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
| **Principal** (boucle winit, `App::frame`) | Le monde, la scène, l'interface egui (jusqu'à la tessellation), la fenêtre. Dans `aurora-render`, le `Renderer` (`main_thread.rs`) : *quoi* dessiner — enregistrements de dessin et table des faces (`RecordStore`), table des objets du culling (`CullTables`), allocateurs de l'arène de géométrie, pages, emplacements et positions des textures (`TextureTable`), palettes d'os et liaisons de skin, mémoire de transit et copies en transit (`UploadQueue`), réglages et taille de la fenêtre. Il crée des ressources GPU (la création n'a pas d'ordre) mais **n'écrit jamais dans la file GPU et ne soumet rien** |
| **Rendu** (`aurora-render`, `render_thread.rs`) | Le `Backend` (`renderer.rs`) : *comment* dessiner — surface et swapchain, pipelines, cibles de rendu, uniformes de l'image, renderer egui, culling compute et ses cases, occlusion, sondes, atlas des imposteurs, relectures (profondeur sous le curseur, compteurs du culling, chronos GPU, captures). **Seul utilisateur de la file** (`write_*`, `submit`, `present`) ; deux threads auxiliaires finissent les encodeurs en parallèle (`helpers.rs`) |
| Tâches de fond (`scene/jobs.rs`, priorité basse) | Décodages, maillages ; écrivent dans la mémoire de transit mappée, jamais dans la file |
| Libérations (`aurora-trash` : `Jobs::discard` ; `aurora-pages` : pages de textures vidées, `textures.rs`) | Libèrent hors du thread principal la mémoire et les pages de textures que plus rien n'utilise. Une page n'est remise à `aurora-pages` qu'après la reconstruction du groupe de textures qui la contenait ; les poignées étant comptées, une image encore en cours sur le thread de rendu (son paquet tient l'ancien groupe) la garde en vie jusqu'à sa fin, et ces threads ne touchent jamais la file |
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
  (`DrawLists`, copiées) et vue du culling GPU, primitives et textures
  egui, taille de la fenêtre / vsync / réglages à appliquer, demandes de
  relecture (pixel survolé, capture).
- **Remise en rendez-vous** (`render_thread::handoff`) : le thread de rendu tient au plus un
  paquet, aucun n'attend derrière. Si le rendu est en retard, le thread
  principal attend (contre-pression) au lieu d'empiler des images ; chaque
  côté mesure son attente (`wait_render`, `rt_idle` du profil).
- **Résultat d'image**, rendu à la remise suivante : statistiques du rendu,
  réponse au survol (gardée avec la caméra de l'image qui l'a demandée),
  compteurs du culling, capture, tampons du paquet à réutiliser.

Deux choses attendent le thread de rendu : l'image qui porte une capture
(même image capturée qu'avant, lue juste après) et la profondeur sous un
clic (`Renderer::pick_world`, exécutée après l'image en cours). Si le thread
de rendu disparaît (panique), toute attente se termine et le viewer se
ferme proprement.

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

`window.rs` réutilise Fluent, la Palette, les polices, les icônes Phosphor
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
