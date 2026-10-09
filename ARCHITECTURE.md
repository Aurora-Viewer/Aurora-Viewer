# Architecture d'Aurora Viewer

## Périmètre

Aurora Viewer est un viewer Second Life complet écrit en Rust : connexion,
réseau UDP et HTTP, monde (objets, terrain, avatars, environnement), rendu
moderne (wgpu sur Vulkan), son, voix, médias et interface (egui).

**Référence de comportement : Firestorm.** Chaque fonctionnalité reproduit
ce que fait Firestorm (et le viewer de Linden Lab dont il dérive) : mêmes
messages réseau, mêmes règles, mêmes valeurs par défaut, sauf choix explicite
noté dans `TASKS.md`. L'interface, elle, est propre à Aurora (voir
[docs/BRANDING.md](docs/BRANDING.md)).

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
| `aurora-net` | Connexion (XML-RPC), circuits UDP, capabilities, file d'événements, téléchargement HTTP des assets ; tourne sur son propre runtime tokio. `session/object_actions.rs` : toucher / saisir / relâcher et inventaire d'objet (capability ou transfert UDP borné en mémoire) ; `task_inventory.rs` : parsing LLSD et fichier historique |
| `aurora-msg` | Système de messages UDP de SL : trames, zerocoding, messages typés générés depuis `message_template.msg` (build.rs) |
| `aurora-llsd` | Type LLSD et ses formats XML / binaire / notation |
| `aurora-prim` | Modèle des prims : paramètres de volume, faces, paramètres étendus, génération de la géométrie (port de `llvolume`) |
| `aurora-assets` | Décodeurs d'assets : JPEG2000, mesh, animations, matériaux, maillages d'avatar `.llm`, squelette |
| `aurora-render` | Moteur de rendu wgpu / Vulkan : textures bindless, géométrie sous-allouée, multi-draw-indirect, ombres, reflets, post-traitement |
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
| `main.rs`, `app.rs` | Démarrage, boucle d'événements, une image du viewer, actions de l'interface |
| `cli.rs` | Arguments de ligne de commande (`--title`) |
| `agent.rs` | Notre avatar (AgentUpdate, extrapolation depuis vitesse / accélération serveur, lissage comme LLDrawable) |
| `camera/` | La caméra comme LLAgentCamera : vue derrière l'avatar, caméra Alt sur un point ou un objet (`focus.rs`), caméra de siège, vue subjective, transitions, lissage, recul en vol, collision envoyée par le simulateur ; réglages Firestorm (`settings.rs`), scénarios de démo (`demo.rs`) |
| `world/` | État du monde reçu du réseau : objets, régions, terrain, environnement (EEP), social, profils des avatars, inventaire, groupes, blocages, messages du bridge LSL de Firestorm (cachés), orientation des corps, regard (LookAt), carte… |
| `scene/` | Ce qu'on envoie au GPU : géométrie et LOD, textures (streaming), avatars (squelette, animations, silhouette), particules, eau, sondes de reflets, sons du monde, imposteurs |
| `ui/` | Interface egui : barres, fenêtres, options, chat, profils des avatars, page web dans une fenêtre (`web_view.rs`), choix d'une texture, inventaire, cartes, overlays de débogage ; sons causés par les widgets (`sound_cues.rs` : clics, touches refusées, fenêtres) |
| `build/` | Outils de construction (édition d'objets, terrain) |
| `interaction.rs`, `cursors.rs`, `ui/object_actions.rs` | Règles des actions de clic 0–9, héritage, permissions, curseurs natifs Firestorm, fenêtres d'achat / paiement et liste du contenu ; transaction après confirmation |
| `app/object_actions.rs` | Déclenchement des actions, toucher maintenu, déplacement physique, lecture de parcelle, ouverture de média et cadrage de caméra |
| `scene/picking.rs` | Rayons contre les triangles partagés avec la géométrie affichée (prims, sculpts, meshes) ; IGNORE traverse la géométrie hors construction, informations de surface pour les scripts de toucher |
| `media/` | Médias des prims et des parcelles, cookie OpenID des pages web de la grille (`openid.rs`) |
| `demo.rs` | Le mode démo : une scène locale qui simule un serveur |
| `settings.rs`, `keybinds.rs`, `keybinds/layout.rs`, `theme.rs` | Réglages enregistrés, raccourcis, disposition Windows et touches de déplacement par défaut, palette |
| `ui_sound.rs` | Catalogue des sons de l'interface (UISnd* de Firestorm), réglages par son |
| `logging.rs`, `cache.rs`, `credentials.rs` | Logs, cache disque, mot de passe retenu (coffre de l'OS) |
| `scene/animesh.rs` | Squelettes autonomes des objets animés, animations du linkset, limites des poses pour le culling et les ombres, scénario de démo |

## Déroulement d'une image

1. **Réseau → monde.** `aurora-net` décode les messages sur son runtime et les
   envoie en `NetEvent` ; `World::apply` met à jour l'état (objets, avatars,
   animations, chat…).
2. **Entrées → agent.** Les touches deviennent des drapeaux de contrôle ;
   `AgentState` et la caméra avancent ; l'`AgentUpdate` est envoyé.
3. **Monde → scène.** `Scene::sync` met à jour la géométrie, les textures et les
   enregistrements GPU qui ont changé (incrémental) ; les poses des avatars
   sont calculées (contrôleur de mouvements façon `LLMotionController`).
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
