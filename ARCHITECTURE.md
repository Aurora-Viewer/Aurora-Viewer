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

```
aurora-viewer/                  dépôt git (GitHub : Aurora-Viewer/Aurora-Viewer)
├─ Cargo.toml                   workspace : version, édition, licence, lints communs
├─ rust-toolchain.toml          Rust stable (toujours la dernière version)
├─ rustfmt.toml                 formatage (lignes de 140 caractères)
├─ README.md                    présentation, compilation, options de test
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
├─ docs/                        BRANDING.md et autres documents
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
| `aurora-net` | Connexion (XML-RPC), circuits UDP, capabilities, file d'événements, téléchargement HTTP des assets ; tourne sur son propre runtime tokio. `session/object_actions.rs` : toucher / saisir / relâcher et inventaire d'objet (capability ou transfert UDP borné en mémoire) ; `task_inventory.rs` : parsing LLSD et fichier historique ; `land.rs`, `session_land.rs` : messages des parcelles (« À propos du terrain », RemoteParcelRequest d'un lieu quelconque) |
| `aurora-msg` | Système de messages UDP de SL : trames, zerocoding, messages typés générés depuis `message_template.msg` (build.rs) |
| `aurora-llsd` | Type LLSD et ses formats XML / binaire / notation |
| `aurora-prim` | Modèle des prims : paramètres de volume, faces, paramètres étendus, génération de la géométrie (port de `llvolume`) |
| `aurora-assets` | Décodeurs d'assets : JPEG2000, mesh, animations, matériaux, maillages d'avatar `.llm`, squelette |
| `aurora-render` | Moteur de rendu wgpu / Vulkan : textures bindless regroupées en pages (texture arrays), géométrie sous-allouée, multi-draw-indirect, ombres, reflets, post-traitement ; animations de texture (`tex_anim.rs` : référence CPU et paramètres des enregistrements, évaluées par les vertex shaders) |
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
| `main.rs`, `app.rs`, `app/` | Démarrage, boucle d'événements, une image du viewer, actions de l'interface ; commandes de la barre de chat (`app/chat_commands.rs`), menus clic droit (`app/context_menu.rs` : ouverture, droits sur l'objet, actions choisies) |
| `cli.rs` | Arguments de ligne de commande (`--title`) |
| `cmdline/` | Barre de chat comme ligne de commande (FSCmdLine) : analyse des commandes, calculatrice (LLCalc), dés ; exécutées par `app/chat_commands.rs` |
| `agent.rs` | Notre avatar (AgentUpdate, extrapolation depuis vitesse / accélération serveur, lissage comme LLDrawable) |
| `camera/` | La caméra comme LLAgentCamera : vue derrière l'avatar, caméra Alt sur un point ou un objet (`focus.rs`), caméra de siège, vue subjective, transitions, lissage, recul en vol, collision envoyée par le simulateur ; réglages Firestorm (`settings.rs`), scénarios de démo (`demo.rs`) |
| `world/` | État du monde reçu du réseau : objets, régions, terrain, environnement (EEP : cycles et assets de réglages dans `eep.rs`, choix local > parcelle > région > jour par défaut, environnement local et fondus dans `eep_env.rs`, listes du sélecteur d'environnement et chargement des dossiers dans `env_select.rs`), parcelle sélectionnée d'« À propos du terrain » et ses droits (`land.rs`), social, profils des avatars, inventaire, groupes (actif, épinglés), cercles de contacts et surnoms (`contact_sets.rs`), blocages, messages du bridge LSL de Firestorm (cachés), orientation des corps, regard (LookAt), carte, requêtes des fenêtres « Détails de l'emplacement » (`place_details.rs` : région par son nom, RemoteParcelRequest, ParcelInfoRequest)… |
| `scene/` | Ce qu'on envoie au GPU : géométrie et LOD, textures (streaming), avatars (squelette, animations, silhouette), particules, eau, sondes de reflets, sons du monde, imposteurs, assets de réglages d'environnement (`settings.rs` : ciel, eau, cycle du jour) |
| `ui/` | Interface egui : barres, fenêtres, options, chat, contacts (amis, groupes, cercles : `contacts.rs`), profils des avatars, « Détails de l'emplacement » d'un lien de lieu (`place_details.rs`), « À propos du terrain » (`land/`, un fichier par onglet, dialogues), choix d'un résident (`avatar_picker.rs`), page web dans une fenêtre (`web_view.rs`), choix d'une texture, inventaire, sélecteur d'environnement et « Éclairage personnel » (`environment.rs`), cartes, overlays de débogage ; sons causés par les widgets (`sound_cues.rs` : clics, touches refusées, fenêtres) ; menus clic droit : apparence commune (`menu.rs` : icône Phosphor, sous-menus, cases cochées) et entrées comme Firestorm (`context.rs` : monde, noms d'avatars, groupes) |
| `build/` | Outils de construction comme LLFloaterTools : sélection et manipulateurs (`manip.rs`), outils Déplacer (`grab.rs`) et Aligner (`align.rs`), terrain (`land.rs`), modifications de la sélection (`edits.rs`), matériaux et médias (`materials.rs`), contenu des objets (`contents.rs`), impact et poids (`costs.rs`), simulateur de démo (`demo_sim.rs`) ; la fenêtre et ses onglets dans `build/ui/` |
| `interaction.rs`, `cursors.rs`, `ui/object_actions.rs` | Règles des actions de clic 0–9, héritage, permissions, curseurs natifs Firestorm, fenêtres d'achat / paiement et liste du contenu ; transaction après confirmation |
| `app/object_actions.rs` | Déclenchement des actions, toucher maintenu, déplacement physique, lecture de parcelle, ouverture de média et cadrage de caméra |
| `scene/picking.rs` | Rayons contre les triangles partagés avec la géométrie affichée (prims, sculpts, meshes) ; prim réellement visée au survol et au clic gauche malgré des boîtes recouvrantes, IGNORE traverse la géométrie hors construction, informations de surface pour les scripts de toucher |
| `media/` | Médias des prims et des parcelles, cookie OpenID des pages web de la grille (`openid.rs`) |
| `demo.rs`, `demo_land.rs`, `demo_place.rs`, `demo_eep.rs`, `demo_env.rs` | Le mode démo : une scène locale qui simule un serveur (et ses réponses à « À propos du terrain », aux détails d'un lieu des régions de la carte et à ExtEnvironment, une bibliothèque d'environnements pour le sélecteur) |
| `settings.rs`, `keybinds.rs`, `keybinds/layout.rs`, `theme.rs` | Réglages enregistrés, raccourcis, disposition Windows et touches de déplacement par défaut, palette |
| `ui_sound.rs` | Catalogue des sons de l'interface (UISnd* de Firestorm), réglages par son |
| `ui/appearance.rs`, `ui/appearance/items.rs`, `ui/appearance/gallery.rs`, `world/appearance.rs` | Fenêtre Apparence, galerie / tenues / portés, édition et dialogue Enregistrer sous ; menus des éléments et de la galerie, confirmation de sauvegarde / suppression, choix d’image, renommage ; points d’attachement / HUD, profil et original ; règles de changement du COF, sauvegarde par liens, protection des parties du corps, scénario hors ligne |
| `aurora-net/src/outfits.rs`, `aurora-net/src/outfits/categories.rs` | Écriture des liens de tenue, favoris, noms et images par AIS InventoryAPIv3, déplacement dans la corbeille ; confirmation et relecture des dossiers / éléments |
| `slurl.rs`, `link_trust.rs` | Liens des textes : SLURL (barre de navigation, libellés « Région (x,y,z) » des liens de lieu) ; confiance des liens web (site de confiance, inconnu, dangereux : raccourcisseurs, faux noms officiels, adresses IP…), jugée localement sur l'URL |
| `logging.rs`, `cache.rs`, `credentials.rs` | Logs, cache disque, mot de passe retenu (coffre de l'OS) |
| `frame_profile.rs` | Profil des images (AURORA_PROFILE) : temps de chaque étape de l'image, ligne de synthèse par seconde dans le log |
| `scene/animesh.rs` | Squelettes autonomes des objets animés, animations du linkset, limites des poses pour le culling et les ombres, scénario de démo |
| `scene/sync_sets.rs` | Objets que la synchro de la scène visite à chaque image, tenus à jour par événements (objets modifiés notés par `ObjectStore`, ensemble des objets qui bougent d'eux-mêmes et de ce qui les suit, géométries en attente, tranche de LOD) au lieu d'un parcours de tous les objets |
| `scene/sync_plan.rs` | Placement en parallèle (rayon) des objets de l'image, niveau par niveau des chaînes de parents : transformation, LOD, limites ; mise à jour de la seule matrice ou synchro complète |

## Déroulement d'une image

1. **Réseau → monde.** `aurora-net` décode les messages sur son runtime et les
   envoie en `NetEvent` ; `World::apply` met à jour l'état (objets, avatars,
   animations, chat…).
2. **Entrées → agent.** Les touches deviennent des drapeaux de contrôle ;
   `AgentState` et la caméra avancent ; l'`AgentUpdate` est envoyé.
3. **Monde → scène.** Les poses des avatars sont calculées (contrôleur de
   mouvements façon `LLMotionController`), puis `Scene::sync` met à jour la
   géométrie, les textures et les enregistrements GPU de ce qui a changé ou
   bouge (ensembles tenus par événements, placement calculé en parallèle).
4. **Scène → GPU.** `build_lists` fait le culling et prépare les listes de
   dessin ; `aurora-render` encode les passes (ombres, prépasse, scène, eau,
   reflets, post-traitement).
5. **Interface.** egui dessine les fenêtres par-dessus.

Les traitements lourds (décodage d'images, maillage, sons) passent par des
tâches en arrière-plan (`scene/jobs.rs`, rayon) pour garder l'image fluide.

## Données sur la machine

| Quoi | Où (Windows) |
|---|---|
| Réglages, skins, disposition des fenêtres | `%APPDATA%\Aurora\AuroraViewer\config\` |
| Cache (textures, mesh, animations, inventaire…) | `%LOCALAPPDATA%\Aurora\AuroraViewer\cache\` |
| Logs (`aurora.log`, `aurora-demo.log`, `aurora-demo-<titre>.log`) | `%LOCALAPPDATA%\Aurora\AuroraViewer\data\logs\` |
| Mot de passe retenu (empreinte, jamais le mot de passe) | Gestionnaire d'identification Windows, service « Aurora Viewer » |
