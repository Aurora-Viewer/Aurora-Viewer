# Options de test et profilage

Toutes les variables d'environnement `AURORA_*` du viewer : le mode démo, les
captures, les scénarios de test, les réglages forcés et le profilage. Les
arguments de ligne de commande sont dans le [README](../README.md#command-line).

Le **mode démo** simule un serveur en local (une place, des avatars, des
objets, du chat, des notifications…) : aucune connexion, aucun compte. Il a
ses propres réglages et son propre cache (`…\config\demo`, `…\cache\demo`).

```powershell
$env:AURORA_DEMO = "1"
$env:AURORA_DEMO_MAP = "1"          # un scénario, si besoin
cargo run -p aurora-viewer -- --title "Test ma-tâche"
```

Les journaux sont dans `%LOCALAPPDATA%\Aurora\AuroraViewer\data\logs\`.

Pour la localisation, lancer le même scénario avec `--language en`, puis
`--language fr`. `AURORA_DEMO_OPTIONS=1` ouvre les préférences d'interface,
avec le sélecteur de langue. Retirer l'argument pour vérifier le changement
immédiat et le choix enregistré au prochain lancement. Le détail des
catalogues, dates et pluriels est dans [TRANSLATING.md](TRANSLATING.md).

**Pour ajouter une variable** : une ligne courte dans le tableau de son
thème (le nom seul dans la première colonne, une phrase dans la seconde) ;
s'il faut plus d'une phrase, ou s'il y a plusieurs modes, un paragraphe
« Détails » sous le tableau. Le choix « Démo » des outils Aurora lit dans ce
fichier les lignes de tableau qui commencent par `AURORA_DEMO_<NOM>=1` et
affiche leur seconde colonne : elle doit rester courte.

Sommaire : [Général](#général) · [Captures](#captures) ·
[Caméra, déplacements et pointeur](#caméra-déplacements-et-pointeur) ·
[Objets et construction](#objets-et-construction) · [HUDs](#huds) ·
[Avatars et animations](#avatars-et-animations) ·
[Rendu et environnement](#rendu-et-environnement) ·
[Charge et fluidité](#charge-et-fluidité) · [Interface](#interface) ·
[Médias et son](#médias-et-son) ·
[Réglages forcés et diagnostics](#réglages-forcés-et-diagnostics) ·
[Profilage](#profilage)

## Général

| Variable | Effet |
|---|---|
| `AURORA_DEMO=1` | Mode démo hors ligne |
| `AURORA_DEMO_POS="x,y"` | Position de départ |
| `AURORA_DEMO_TOD=0..4` | Heure du jour |
| `AURORA_FPS_LIMIT=<n>` | Plafond d'images/s pour les essais (10–500) |
| `AURORA_LOG_NAME=<nom>` | Nom du fichier journal |
| `AURORA_EMOJI_FONT=<chemin>` | Utilise une autre police emoji |
| `AURORA_MAX_AVATARS`, `AURORA_MAX_COMPLEXITY`, `AURORA_AA`, `AURORA_SHADOWS` | Remplacent ces réglages |

Le limiteur d'images/s est activé à **120 images/s par défaut**. Il peut
être désactivé ou réglé dans Préférences › Graphismes › Fluidité ; les
préférences déjà enregistrées sont conservées.

`AURORA_FPS_LIMIT` : plafond utilisateur pour les essais (10–500 images/s ;
les valeurs positives sont ramenées à cette plage, une valeur invalide est
ignorée). Le plafond inférieur en arrière-plan reste applicable hors captures.

## Captures

| Variable | Effet |
|---|---|
| `AURORA_CAPTURE=<fichier.png>` | Enregistre une capture de l'image |
| `AURORA_CAPTURE_FRAMES=<n>` | Image capturée (240 par défaut) |
| `AURORA_CAPTURE_EXIT=1` | Quitte après la capture |

`AURORA_CAPTURE_FRAMES` : compter environ 620 pour passer le fondu de
chargement. Plusieurs images séparées par des virgules (`2500,2600`)
enregistrent un fichier chacune, `<fichier>-<image>.png`. Le plafond
d'images/s en arrière-plan (« Limiter hors focus ») est ignoré pendant une
capture.

## Caméra, déplacements et pointeur

| Variable | Effet |
|---|---|
| `AURORA_DEMO_CAM="yaw,pitch,dist"` | Place la caméra autour de l'avatar |
| `AURORA_DEMO_CAMERA=<scénario>` | Scénario de caméra (voir Détails) |
| `AURORA_DEMO_TP=1` ou `"x,y,z"` ou `remote` | Téléportation après le chargement |
| `AURORA_DEMO_KEY=<flèche>` | Tient une flèche du clavier |
| `AURORA_DEMO_SIT=<n>` | Clique n fois sur le bouton S'asseoir |
| `AURORA_DEMO_MMO="x,y[,1\|2]"` | Pilotage de l'avatar à la souris |
| `AURORA_DEMO_RCLICK="x,y"` ou `tag` | Clic droit (menu contextuel) |
| `AURORA_DEMO_PIE=1` | Menu circulaire du clic droit |
| `AURORA_DEMO_POINTER=<points>` | Déplace le pointeur sur l'interface |
| `AURORA_DEMO_HOVER=1` | Balayage du curseur au survol |
| `AURORA_DEMO_LOOKAT=1` | Suivi du regard et regard d'un autre avatar |
| `AURORA_DEMO_KEYBOARD=<disposition>` | Touches de déplacement selon la disposition |
| `AURORA_DEMO_CHATCMD="<commandes>"` | Tape des commandes dans la barre de chat |

### Détails

- **`AURORA_DEMO_CAM`** : décalage de cap et inclinaison de la caméra autour
  de l'avatar (radians, une inclinaison positive regarde vers le bas) et
  distance (mètres).
- **`AURORA_DEMO_CAMERA=alt,x,y|pan,x,y|zoom,x,y|tag|ml|wheel|fly|sit`** :
  scénario journalisé sous `demo camera` — Alt+clic en (x, y) puis glisser et
  retour en marchant ; appui sur notre propre étiquette de nom puis glisser
  (pilotage) ; entrée / sortie de la vue subjective ; molette ; recul en
  vol ; assise sur un siège qui tourne, avec une caméra de siège et une
  animation lancée par une prim liée (image 300), puis lever et demande de
  son arrêt (image 700 ; `camera/demo.rs`).
- **`AURORA_DEMO_TP`** : téléportation après le fondu du chargement initial
  (image 240 au plus tôt). `1` ou `"x,y,z"` : arrivée locale immédiate, sans
  écran de chargement ; `remote` simule la progression entre régions.
- **`AURORA_DEMO_KEY=down|up|left|right`** : tient une flèche à partir de
  l'image 235 (déplacement, orientation du corps).
- **`AURORA_DEMO_SIT=n`** : clics aux images 240, 300, 360…
- **`AURORA_DEMO_MMO="x,y[,1|2]"`** : appui gauche sur l'avatar puis bouton
  droit maintenu (pilotage à la souris) ; `1` : flèche droite à la place,
  `2` : double clic droit (course).
- **`AURORA_DEMO_RCLICK`** : clic droit en (x, y) à l'image 225, ou avec
  `tag` sur l'étiquette de nom de l'autre avatar le plus proche à
  l'image 600.
- **`AURORA_DEMO_PIE`** : `1` force le pie menu, `0` force la liste,
  uniquement en démo et sans modifier le choix enregistré. Combiner avec
  `AURORA_DEMO_RCLICK="x,y"`, `tag` ou `AURORA_DEMO_HUDS=menu` pour comparer
  les mêmes cibles. Pour le test humain du sélecteur, retirer cette variable :
  Préférences › Interface › Disposition › Menu du clic droit ; basculer entre
  Menu en liste et Pie menu (circulaire), puis redémarrer pour vérifier le choix.
- **`AURORA_DEMO_POINTER="x,y[,l|r][;x,y…]"`** : déplace le pointeur de
  l'interface vers ces pixels de la fenêtre à partir de l'image 300, un point
  toutes les 60 images (états de survol, sous-menus) ; `,l` fait un clic gauche,
  `,r` un clic droit sur l'interface à cet endroit. Dans le pie menu, le clic
  gauche entre dans un sous-menu et le clic droit remonte d'un niveau.
- **`AURORA_DEMO_HOVER=1`** : à partir de l'image 300, le curseur balaie la
  fenêtre pendant 120 images, reste immobile 120 images, et ainsi de suite
  (recherche de l'objet sous le curseur et réponses gardées d'une image à
  l'autre, `scene/hover.rs`). Lire `hover` dans le profil ; à combiner avec
  `AURORA_HOVER_CHECK=1`, `AURORA_DEMO_ACTIONS=1` ou une scène de charge.
- **`AURORA_DEMO_LOOKAT=1`** : suivi du regard activé (en mémoire seulement)
  et regard d'un avatar distant.
- **`AURORA_DEMO_KEYBOARD=system|wasd|zqsd|fallback`** : valeurs par défaut
  neuves des déplacements avec la disposition de Windows, un QWERTY / AZERTY
  simulé, ou une détection en échec ; à combiner avec `AURORA_DEMO_OPTIONS=8`
  pour examiner les raccourcis secondaires.
- **`AURORA_DEMO_CHATCMD="calc 2+2;rolld 2 20"`** : lignes tapées dans la
  barre de chat à l'image 240, séparées par `;` (commandes de la barre de
  chat : `calc`, `rolld`, `gtp`…).

## Objets et construction

| Variable | Effet |
|---|---|
| `AURORA_DEMO_ACTIONS=<mode>` | Curseurs, clics d'objet, achat et paiement |
| `AURORA_DEMO_DIALOG=<mode>` | Menu llDialog |
| `AURORA_DEMO_BUILD="<script>"` | Script des outils de construction |
| `AURORA_DEMO_BUILD_FRAME=<n>` | Image de départ du script de construction |
| `AURORA_DEMO_BUILD_TAB=<onglet>` | Onglet de la fenêtre de construction |
| `AURORA_DEMO_LSL_BRIDGE=1` | Messages du bridge LSL de Firestorm (cachés) |

### Détails

**`AURORA_DEMO_ACTIONS`** — curseurs et clics d'objet comme Firestorm.
Aucun L$ réel ni connexion à une grille.

- `1` : tous les objets, pour un test manuel.
- `sit`, `buy`, `pay`, `none`, `touch`, `disabled`, `ignore`, `open`,
  `play`, `pause`, `open-media`, `zoom`, `grab` : une cible isolée ; survol
  et clic par le chemin habituel.
- `none` / `touch` envoient appui, déplacement et relâché ; `grab` simule le
  déplacement physique.
- `ignore` traverse un écran visible et ouvre Buy derrière ; `disabled`
  arrête le clic sans toucher ; `ignore-build` sélectionne cet écran en
  construction.
- `open` affiche trois éléments de contenu simulés.
- `play` lance une page HTML locale ; `pause` et `open-media-playing`
  injectent un état de lecture simulé avant le clic ; `open-media` simule
  l'ouverture du navigateur externe.
- `zoom` cadre le cube.
- `buy-confirm`, `pay-confirm` : confirment la transaction simulée.
- `linked-touch`, `linked-sit` : racine creuse avec Touch et siège enfant
  Sit, boîtes englobantes recouvrantes ; vérifie la prim effectivement visée.

Fenêtres de transaction, avec la même variable :

- `pay-layout`, `pay-hidden`, `pay-large` : quatre montants à quatre
  chiffres, champ libre et boutons masqués, montant maximal.
- `buy-original`, `buy` (copie), `buy-contents` : achat d'un original, d'une
  copie ou du contenu, liste des éléments inclus et permissions du prochain
  propriétaire.
- `buy-empty` : propose un contenu non vendable et désactive l'achat.

**`AURORA_DEMO_DIALOG=12|4|long`** — menu llDialog de douze réponses sur
trois colonnes, rangées du bas vers le haut comme Firestorm ; `4` : quatre
réponses, pour vérifier la dernière rangée incomplète ; `long` : libellé
long tronqué, avec le texte complet au survol. Boutons Bloquer et Ignorer
sous le menu.

**`AURORA_DEMO_BUILD="mode,x,y[,part,dx,dy]"`** — script des outils de
construction. Modes : `move`, `rotate`, `stretch`, `face`, `align`, `grab`,
`focus`, `create`, `land`, `select`. `AURORA_DEMO_BUILD_FRAME` change son
image de départ.

**`AURORA_DEMO_BUILD_TAB=general|object|features|texture[:pbr|bp|media]|contents`**
— onglet de la fenêtre de construction affiché par le script ; `+weights`,
`+grid`, `+media` ouvrent aussi ces fenêtres.

**`AURORA_DEMO_LSL_BRIDGE=1`** — messages du bridge LSL de Firestorm
(cachés) entre deux lignes « owner say ».

## HUDs

| Variable | Effet |
|---|---|
| `AURORA_DEMO_HUDS=<mode>` | HUDs portés : affichage, toucher, déplacement, édition |

### Détails

Huit HUDs texturés sur les points 31–38, avec texte flottant et bouton
enfant translucide ; le HUD d'un autre avatar est caché. Aucun accès à une
grille.

- `1` : affichage seul.
- `touch` : simule appui, déplacement et relâché sur l'enfant (images
  720–900), puis change la couleur du HUD.
- `zoom` : réduit à 50 % ; `hidden` : masque les HUDs ; `media` : affiche la
  page locale de test sur le HUD central.
- `scale` : cinq crans de molette avec la touche HUD (image 720), sur un
  HUD sans droits Modifier / Déplacer ; cadre et pourcentage jusqu’à l’image
  840. `scale-ctrl`, `scale-key`, `scale-secondary` utilisent Ctrl, B ou la
  touche secondaire. `scale-reset` rétablit 100 % ; `scale-forget` oublie le
  profil sans modifier la copie actuelle. `scale-copy` remplace le HUD par
  un nouvel UUID au même nom / créateur (image 900) et journalise l’échelle
  restaurée. `scale-touch` clique ensuite le HUD agrandi (images 880–900).
  Les profils de ces scénarios restent dans le dossier de compte de démo.
- `drag` : saisit l'enfant avec ALT (image 720), déplace tout le HUD,
  relâche ALT avant le clic (image 800), puis enregistre la position au
  relâchement gauche (image 840) sans toucher de script.
- `drag-zoom` : de même à 50 % ; `drag-ignore` : sur un enfant avec l'action
  IGNORE ; `drag-ctrl` : Ctrl à la place de ALT ; `drag-key` : la touche B à
  la place de ALT ; `drag-secondary` : ALT en touche principale et Ctrl en
  touche secondaire, déplacement avec cette dernière.
- `menu`, `edit`, `edit-zoom` : clic droit sur l'enfant, Modifier et
  déplacement du linkset par la poignée Y, aussi à 50 %.
- `edit-scroll` : sélectionne un HUD agrandi localement à 200 %, puis le
  place hors écran (image 740). Sans touche HUD, cinq crans arrière dans
  une zone vide dézooment tous les HUDs (image 760) ; avec la touche HUD,
  un cran agrandit seulement le HUD survolé (image 820). Cinq crans avant
  reviennent à la vue globale normale (image 900). La caméra et les facteurs
  locaux sont conservés pendant le zoom global ; la touche HUD conserve
  sa priorité pour le zoom local. Les images 920 et 940 ferment l’édition,
  puis sélectionnent un objet du monde : la molette n’est plus interceptée
  par les HUDs. Captures conseillées : 750,790,850,910.
- `faces`, `faces-turned` : neuf panneaux HUD à une seule face visible —
  colonnes avant / arrière / matériau PBR explicitement double face, lignes
  opaque / translucide / masqué. La colonne arrière doit rester vide ;
  `faces-turned` retourne tous les panneaux de 180° : la colonne avant
  devient vide, l'arrière apparaît, le double face reste visible. Les
  libellés flottants restent visibles dans les deux sens.

À vérifier avec ces scénarios :

- ALT + clic gauche maintenu sur la géométrie visible déplace le HUD sans
  ouvrir les outils ; main de saisie au survol et pendant le déplacement.
- Préférences › Raccourcis › HUDs, entre Fenêtres et Débogage : deux touches
  configurables comme les autres raccourcis (ALT en principale par défaut,
  secondaire vide), clic pour saisir, clic droit pour effacer et Échap pour
  annuler. Pour choisir Ctrl, ALT ou Maj seuls, appuyer puis relâcher ; les
  combinaisons et les boutons de souris affectables sont aussi acceptés.
  Les deux choix sont enregistrés et appliqués au survol comme au déplacement ;
  « Rétablir les raccourcis par défaut » réinitialise aussi cette ligne.
- Le droit Déplacer suffit, y compris sans droit Modifier.
- Touche HUD + molette : taille locale de 10 % à 800 %, Maj pour affiner,
  sans zoom de caméra ni défilement de média. Clic droit : « Revenir à
  100 % » et « Oublier le réglage mémorisé ». Les nouvelles copies reprennent
  le profil du même nom d’inventaire / créateur sur ce compte ; les copies
  déjà portées gardent leurs tailles indépendantes. Vérifier après relance,
  détachement / réattachement, clic sur les boutons et déplacement du HUD.
- Inventaire de démo lié à la tenue actuelle, pour tester le détachement et
  l'affichage dans l'inventaire.
- Menu Monde › HUDs : afficher / masquer, réduire / agrandir et taille
  normale.

## Avatars et animations

| Variable | Effet |
|---|---|
| `AURORA_DEMO_ANIM_LOOP=1` | Boucles d'animation, séquences et arrêts |
| `AURORA_DEMO_AO=1` | AO intégré et inventaire d’animations synthétique |
| `AURORA_DEMO_ANIMESH=1` | Objets animés (animesh) |
| `AURORA_DEMO_CLOUD=1` | Nuages de chargement des avatars |
| `AURORA_DEMO_DISPLAYNAME="<nom>"` | Changement de nom d'affichage (simulé) |
| `AURORA_DEMO_DISPLAYNAME_ERROR=<error_tag>` | Erreur du serveur au changement de nom |

### Détails

- **`AURORA_DEMO_ANIM_LOOP=1`** : boucle dont le premier intervalle manque,
  séquence changée toutes les 120 images, arrêt / redémarrage aux images
  960 / 1020 toutes les 1200 images.
- **`AURORA_DEMO_AO=1`** : ouvre la gestion AO, avec deux poses debout en
  cycle, marche, course, assise, vol, frappe et une notecard importable.
  Les touches de déplacement changent l’état. `transitions` enchaîne les
  états automatiquement après le chargement, toutes les 180 images ;
  `off` commence avec l’AO désactivé. Vérifier le bouton AO en bas, les
  poses, la pause, les cycles et l’ajout / import depuis l’inventaire.
- **`AURORA_DEMO_ANIMESH=1`** : mesh racine animé, mesh lié signalé par un
  enfant et animesh porté, avec des squelettes propres indépendants et des
  ombres alpha.

## Rendu et environnement

| Variable | Effet |
|---|---|
| `AURORA_DEMO_PARTICLES=<mode>` | Particules de vent et de fumée |
| `AURORA_DEMO_PLANAR=1` | Dalles en mapping de texture planaire |
| `AURORA_DEMO_PBR_OVERRIDE=1` | Override de matériau GLTF |
| `AURORA_DEMO_TEXANIM=1` | Animations de texture (llSetTextureAnim) |
| `AURORA_DEMO_SKY=<gamma>` | Ciel EEP classique avec ce gamma |
| `AURORA_DEMO_OCCLUSION=1` | Mur avec des objets cachés (test d'occlusion) |
| `AURORA_DEMO_DEBUG="<calques>"` | Calques de débogage |
| `AURORA_DEMO_EEP_PARCEL=<mode>` | Environnement EEP de la région et de la parcelle |
| `AURORA_DEMO_ENV_SELECT=<mode>` | Sélecteur d'environnement |

### Détails

- **`AURORA_DEMO_PARTICLES=wind|smoke|both|legacy`** : traits de vent de la
  moto et paramètres du script de fumée reçus par des mises à jour
  compressées, devant un panneau sombre ; format étendu du glow (`legacy` :
  vent sans glow), texture douce par défaut des particules hors ligne.
- **`AURORA_DEMO_PLANAR=1`** : les carreaux doivent se raccorder d'une dalle
  à l'autre.
- **`AURORA_DEMO_PBR_OVERRIDE=1`** : deux dalles PBR, dont une avec un
  override de matériau GLTF (4 × 4 répétitions, teinte).
- **`AURORA_DEMO_TEXANIM=1`** : deux rangées de panneaux à la place des
  panneaux alpha — défilement lisse, grille de 4 × 4 images, ping-pong,
  rotation, échelle (masque alpha : prépasse et ombres), un cube animé sur
  une seule face, un matériau legacy (la normal map suit) et une face PBR.
- **`AURORA_DEMO_SKY=<gamma>`** : ciel EEP classique (sans ambiance des
  sondes de reflets) avec ce gamma de ciel et une couleur du soleil
  supérieure à 1 : gamma legacy et lumière des objets normalisée comme
  Firestorm.
- **`AURORA_DEMO_DEBUG`** : liste séparée par des virgules parmi `bounds`,
  `culling`, `lights`, `probes`, `skeletons`, `alpha`, `wire`, `complexity`,
  `glow`, `glow_view`, `freeze`.
- **`AURORA_DEMO_EEP_PARCEL=1|parcel|default|stale`** : réponses EEP rejouées
  comme la grille (`demo_eep.rs`) — un jour de région au crépuscule puis,
  0,4 s plus tard, la parcelle de l'agent sans jour (`1` : le crépuscule
  reste), avec son propre jour à midi (`parcel` : fondu de 5 s), une région
  sans jour (`default` : l'asset de jour par défaut de Firestorm, le ciel par
  défaut intégré hors ligne) ou des réponses pour une autre parcelle et une
  autre région (`stale` : ignorées).
- **`AURORA_DEMO_ENV_SELECT=1|lighting|list`** : sélecteur d'environnement
  (`demo_env.rs`), avec une bibliothèque contenant un dossier
  « Environments » et un dossier Paramètres dans l'inventaire, dont les
  assets de réglages sont connus localement. `1` : la fenêtre s'ouvre, puis
  un ciel est choisi (image 2600), une eau (2900), le ciel suivant avec ›
  (3200), un cycle du jour (3500) et « Environnement partagé » (3800, fondu
  de 5 s). `lighting` : « Éclairage personnel » s'ouvre aussi, un ciel est
  choisi (2600) puis modifié (2900). `list` : la liste des ciels déroulée
  (2500).

## Charge et fluidité

| Variable | Effet |
|---|---|
| `AURORA_DEMO_TEXTURES=<n>` | Test de charge des textures : n cubes |
| `AURORA_DEMO_TEXTURES_CHURN=1` | Avec `AURORA_DEMO_TEXTURES` : textures renouvelées |
| `AURORA_DEMO_CROWD=<n>` | Test de charge de la synchro : n avatars |
| `AURORA_DEMO_STREAM=<n>[,<vague>][,leave]` | Test des à-coups du streaming |

### Détails

- **`AURORA_DEMO_TEXTURES=<n>`** : n petits cubes (9000 si la valeur n'est
  pas un nombre), chacun avec sa propre texture, de plusieurs tailles (dont
  une qui n'est pas une puissance de deux), envoyée d'abord en basse
  résolution puis complète, comme dans une région chargée.
- **`AURORA_DEMO_TEXTURES_CHURN=1`** : un tiers des cubes retirés à
  l'image 300 (leurs textures évincées 2 s plus tard : couches libérées,
  compactage des pages), puis de retour à l'image 700 avec de nouvelles
  textures (emplacements libérés réutilisés, streaming à nouveau) ; capturer
  après l'image ~1000.
- **`AURORA_DEMO_CROWD=<n>`** : n avatars en plus (40 si la valeur n'est pas
  un nombre) qui jouent l'animation d'attente, chacun portant huit linksets
  de sept prims sur des os de tout le corps (avec `AURORA_DEMO_ANIMESH`,
  aussi le mesh riggé de la démo), comme dans une boutique fréquentée.
- **`AURORA_DEMO_STREAM=<n>[,<vague>][,leave]`** : n objets texturés (2000
  si la valeur n'est pas un nombre) qui arrivent par vagues après le fondu de
  chargement, `vague` objets toutes les 250 ms (100 par défaut, soit ~400 par
  seconde comme sur la grille ; `vague` = n envoie tout d'un coup, comme une
  arrivée par téléportation). Chaque objet a sa propre forme et sa propre
  texture, aux tailles d'une région de la grille (surtout 512 et 1024),
  décodée par les tâches de fond au quart de sa taille, puis en taille
  complète 1,5 s plus tard.
  - Le journal donne la fin du chargement (`demo stream: fully loaded in
    … s`, avec les envois préparés par les tâches / écrits par le thread
    principal) ; avec `AURORA_PROFILE=1`, lire `stream`, `results` et les
    parties `s_*` dans `max:`.
  - Avec `,leave`, la région est quittée une seconde après le chargement,
    comme par une téléportation : tous les objets sont retirés d'un coup et
    leur texture, qui contient alors les données téléchargées de la taille
    de son fichier J2C, est inutilisée et à évincer 2 s plus tard (`demo
    stream: region left`, puis `demo stream: textures evicted`) ; lire
    `s_maintain`, `sync`, `s_sync_list` et `r_submit` dans `max:`.

## Interface

| Variable | Effet |
|---|---|
| `AURORA_DEMO_WINDOWS=<mode>` | Repli, déplacement et restauration du titre de l’inventaire |
| `AURORA_DEMO_EXIT=<mode>` | Modale de fermeture et choix hors ligne |
| `AURORA_DEMO_UI=<onglet>` | Ouvre Personnes (onglet), l'inventaire et le chat |
| `AURORA_DEMO_OPTIONS=<section>` | Ouvre les préférences sur une section |
| `AURORA_DEMO_PERF=compact` ou `full` | Ouvre la fenêtre Performances, compacte ou complète |
| `AURORA_DEMO_LOGIN=<mode>` | Écran de connexion hors ligne |
| `AURORA_DEMO_CRASH=<mode>` | Plantage hors ligne et fenêtre de récupération |
| `AURORA_DEMO_CRASH_DIR=<dossier>` | Isole les rapports de plantage des essais hors ligne |
| `AURORA_CRASH_CAPTURE=<fichier.png>` | Capture de la fenêtre de récupération en démo |
| `AURORA_CRASH_CAPTURE_EXIT=1` | Ferme la fenêtre après sa capture en démo |
| `AURORA_DEMO_UPDATE=<mode>` | Maquette interactive de mise à jour sur la connexion |
| `AURORA_DEMO_UPDATE_MAINTENANCE=<mode>` | Annonce d’une invalidation du cache et/ou remise à zéro |
| `AURORA_UPDATE_TEST_ROOT=<dossier>` | Installation jetable servie par un flux local |
| `AURORA_UPDATE_TEST_VERSION=<tag>` | Compilation dev à version synthétique pour cet essai |
| `AURORA_DEMO_BUG_REPORT=<mode>` | Rapport de bug public avec données de démo, sans envoi |
| `AURORA_DEMO_MAP=1` ou `mini` | Carte du monde et mini-carte |
| `AURORA_DEMO_NOTIF=1`, `AURORA_DEMO_STATUSMENU=1`, `AURORA_DEMO_NAVEDIT=1` | Liste des notifications, menu de statut, champ de lieu |
| `AURORA_DEMO_CONV=1`, `AURORA_DEMO_TALK=1` | Conversation de groupe, micro ouvert |
| `AURORA_DEMO_CONTACTS=<onglet>` | Onglet Contacts de Conversations |
| `AURORA_DEMO_PROFILE=<avatar>[:onglet]` | Fenêtre de profil d'un avatar |
| `AURORA_DEMO_FEED=<url>` | Onglet « Flux » du profil sur cette page |
| `AURORA_DEMO_APPEARANCE=<mode>` | Fenêtre Apparence et tenues |
| `AURORA_DEMO_INVENTORY=<mode>` | Inventaire synthétique, menus et fenêtres |
| `AURORA_DEMO_LAND=<onglet>[:owner]` | « À propos du terrain » sur un onglet |
| `AURORA_DEMO_PLACE=<lieu>` | « Lieux » sur un profil de lieu |
| `AURORA_DEMO_PLACES=<onglet>` | « Lieux » sur un onglet |
| `AURORA_DEMO_PLACE_WINDOW=1` | Profils de lieu en fenêtres séparées |
| `AURORA_DEMO_BAN=1` | Lignes d'interdiction |
| `AURORA_DEMO_RESTRICTED=1` | Parcelle qui interdit tout |

### Détails

- **`AURORA_DEMO_WINDOWS=1|restore|drag`**, avec `AURORA_DEMO_INVENTORY=1` :
  double-clic sur le titre réel de l’inventaire aux images 660–665 ; `1`
  garde la barre repliée, `restore` la développe aux images 1000–1005,
  `drag` la déplace aussi de 100 × 60 points aux images 780–782.
  Capturer `620,740,900,1120` pour vérifier contenu, repli, déplacement et
  dimensions restaurées. Le bouton « − » reste utilisable ; les autres
  boutons du titre et le contenu ne déclenchent pas le double-clic du titre.
- **`AURORA_DEMO_EXIT=1|cancel|logout|quit`** : ouvre la modale à l’image 650.
  `1` la laisse interactive ; les autres modes font le choix à l’image 800.
  Capturer `740,900` pour comparer le fond flouté puis la session reprise ou
  le retour au login. `quit` termine le processus à l’image 800 (capturer
  seulement 740, sans `AURORA_CAPTURE_EXIT`). Combiner avec
  `AURORA_DEMO_LOGIN=empty` pour l’écran de connexion : Déconnecter est
  désactivé. Les choix sont simulés hors ligne, sans requête de connexion.
- **`AURORA_DEMO_OPTIONS=<section>`** : par exemple `8` pour Raccourcis
  (touche de déplacement des HUDs).
- **`AURORA_DEMO_CRASH=panic|native|missing|normal|recovery`**, seulement
  avec `AURORA_DEMO` : déclenche une panique Rust, une exception native
  Windows, une sortie anormale sans trace, une fermeture normale ou une
  sortie sans surveillant. Le dernier mode permet de vérifier la
  récupération au lancement suivant (relancer avec `normal`). Aucune
  connexion ni initialisation du GPU. La fenêtre doit conserver la langue
  et le thème de la session arrêtée, permettre la sélection et la copie de
  la trace et ouvrir son dossier ; aucune fenêtre après une fermeture
  normale. Les captures sont prises après trois images de la fenêtre
  indépendante. `AURORA_DEMO_CRASH_DIR` permet d'utiliser un dossier
  temporaire par scénario, sans toucher aux rapports réels.
  Le bouton « Envoyer un rapport de plantage » doit ouvrir le formulaire
  partagé, avec le titre et les informations de cette session. Pour le
  capturer automatiquement, combiner avec `AURORA_DEMO_BUG_REPORT=1`
  (Description) ou `details` (Données). Le mode démo reste sans envoi,
  autorisation GitHub ou ouverture de navigateur. L'extrait public et le
  ZIP privé commencent décochés. Décocher une information doit la retirer
  de l'aperçu. Fermer et rouvrir le formulaire conserve le brouillon.
- **`AURORA_DEMO_BUG_REPORT=1|details|image|image-ui|direct`** : ouvre le
  rapport après le chargement (image 550). `1` montre la description,
  `details` les informations avec le lieu décoché et les diagnostics filtrés,
  `image` capture la scène seule, `image-ui` conserve l’interface, `direct`
  sélectionne l’envoi direct. Dans tous ces modes, l’envoi reste local et
  n’ouvre ni GitHub ni le navigateur. Voir [BUG_REPORTING.md](BUG_REPORTING.md).
  Modes de régression : `refresh` prend une capture puis active l’interface
  à l’image 700 et la désactive à l’image 900 ; `language` bascule le formulaire
  ouvert en anglais à l’image 700, puis en français à l’image 900. Capturer aux
  images `650,850,1050` pour comparer. Pour tester le choix de langue dans les
  Préférences, ne pas fournir l’argument `--language`.
  Validation humaine de l’envoi réel, sans mode démo : autoriser GitHub,
  sélectionner une image et, dans Données, cocher explicitement le ZIP privé
  (décoché au départ), puis envoyer. L’issue doit afficher l’image et le lien
  des journaux, sans contenu du ZIP dans le texte public. Le téléchargement
  du ZIP doit demander GitHub et refuser un compte sans accès au dépôt privé
  `Aurora-Viewer/aurora-api`. Un développeur autorisé doit pouvoir l’ouvrir.
  Après un échec d’envoi des fichiers, fermer et rouvrir la fenêtre doit garder
  la même issue et proposer de terminer son envoi, sans créer de doublon.
- **`AURORA_DEMO_LOGIN=remembered|empty`** : écran de connexion hors ligne
  avec un marqueur synthétique de mot de passe enregistré, ou un champ de
  mot de passe vide (aucun accès à une grille ni au coffre d'identifiants).
- **`AURORA_DEMO_UPDATE=1|progress|ready|ignored`**, avec `AURORA_DEMO=1` : proposition
  d'une version synthétique, téléchargement à 42 % ou téléchargement terminé.
  « Mettre à jour » anime une progression simulée ; « Annuler » revient à la
  proposition. Aucun téléchargement ni remplacement de fichier. La connexion
  est inerte et les préférences de connexion restent intactes.
  Après « Ignorer cette version » ou « Plus tard », le bouton « Vérifier les
  mises à jour » du panneau Version rouvre la proposition. Le mode `ignored`
  démarre directement avec le dialogue fermé pour essayer ce parcours.
  Dans une petite fenêtre, le bouton reste accessible en haut à droite.
  `AURORA_DEMO_UPDATE_MAINTENANCE=cache|settings|both` affiche les effets forcés ;
  absent, le dialogue annonce la conservation et les migrations habituelles.
- **Installation réelle isolée** : `scripts/test-update.ps1` prépare les paquets
  0.2.100 et 0.2.101 dans `.scratch`, sert le second sur loopback et lance le premier.
  Le serveur ralentit le ZIP pour tester Annuler. Compilation dans le `target`
  ordinaire du worktree, profil dev, puis restauration du binaire normal à sa
  version Git ; aucun tag ni release publié. `AURORA_UPDATE_TEST_ROOT` n’est
  accepté que pour un exe dans son sous-dossier `install`, avec un marqueur
  `aurora-update-test.json` définissant un flux HTTP loopback. Réglages, cache et
  logs sont isolés sous cette racine ; connexion et coffre d’identifiants inertes.
  `AURORA_UPDATE_TEST_VERSION` est réservé à ces binaires et refusé en release
  ou avec `AURORA_VERSION_TAG`. Les binaires synthétiques refusent tout lancement
  normal. Tester Ignorer → Vérifier, Mettre à jour → Annuler, puis Installer et
  relancer : le titre doit passer de 0.2.100 à 0.2.101. Fermer le viewer et arrêter
  le PID du serveur affiché avant le ménage. Les arguments de titre, langue et
  limite de FPS fournis au script sont conservés après relance.
  Ajouter `-ClearCache` et/ou `-ResetSettings` au script pour tester les choix
  de publication. Le dialogue doit annoncer les effets avant téléchargement
  et relance. Après fermeture du nouveau viewer, `cache/update-marker.txt`
  doit avoir disparu si le cache était à vider ; `profile/settings.json`
  doit revenir aux valeurs par défaut si demandé (distance initiale de test :
  300 m), et `profile/update-marker.txt` doit rester présent dans tous les cas.
  Ajouter `-Required` pour imposer la version suivante : aucun choix Ignorer ou
  Plus tard, Échap ne ferme pas le dialogue, et le bouton de connexion reste
  désactivé avec le motif de blocage même si les champs sont remplis. Dans
  toute installation isolée, un libellé signale que la connexion à une grille
  est désactivée ; ne saisir que des valeurs synthétiques pour cet essai.
  `AURORA_UPDATE_ACK` et `AURORA_UPDATE_ROLLBACK` sont des signaux internes entre
  processus, pas des scénarios à définir à la main.
- **`AURORA_DEMO_CONTACTS=amis|groupes|cercles|detache|ajout`** : onglet
  Contacts de Conversations sur Amis, Groupes ou Cercles de contacts (cercles
  et surnoms de démo), la fenêtre Contacts détachée, ou le sélecteur de
  résident de « Ajouter... ».
- **`AURORA_DEMO_PROFILE=loup|nova|friend|self[:onglet]`** : une fenêtre de
  profil (onglet 0 Vie SL, 1 Flux, 2 Favoris, 3 Annonces, 4 Vie RL, 5 Notes).
- **`AURORA_DEMO_FEED=<url>`** : le nom d'utilisateur et `/?feed_only=true`
  sont ajoutés à l'adresse ; une URL `data:` peut les mettre en commentaire.
- **`AURORA_DEMO_LAND=<onglet>[:owner]`** : onglets `general`, `reglement`,
  `objets`, `options`, `medias`, `son`, `acces`, `experiences`,
  `environnement`, ou leur index ; `:owner` rend l'avatar propriétaire de la
  parcelle (contrôles activés), sinon elle appartient à un groupe sans
  pouvoirs.
- **`AURORA_DEMO_PLACE=1|lagune|nordheim|pinede|faille|inconnue|repere|historique`** :
  « Lieux » sur un profil de lieu, comme après un clic sur un lien de lieu.
  `1` : le domicile de Loup Violet sur Aurora Démo (la parcelle d'« À propos
  du terrain ») ; `lagune` : une parcelle Modérée ; `nordheim` : une parcelle
  Adulte appartenant à un groupe ; `pinede` : une parcelle sans nom, ni
  description, ni photo ; `faille` : RemoteParcelRequest répond HTTP 404 ;
  `inconnue` : une région à laquelle personne ne répond (erreur après 10 s) ;
  `repere` : le profil d'un repère ; `historique` : celui d'une entrée de
  l'historique de téléportation.
- **`AURORA_DEMO_PLACES=favoris|reperes|historique`** : les dossiers Favoris
  et Repères de la démo (avec un sous-dossier) et un historique de
  téléportation sur plusieurs mois.
- **`AURORA_DEMO_PLACE_WINDOW=1`** : avec `AURORA_DEMO_PLACE`, les fenêtres
  de lieu séparées au lieu de « Lieux » (option « Repères et profils de
  lieux », non enregistrée).
- **`AURORA_DEMO_RESTRICTED=1`** : icônes de parcelle rouges, dégâts
  activés, santé à 72 %.

**`AURORA_DEMO_APPEARANCE=gallery|outfits|worn|save|edit|menu|duplicates`**
— ouvre Apparence avec quatre tenues synthétiques :

- `gallery` : la galerie (le clic droit comprend renommer, favoris, choix
  d'image, enregistrer dans la tenue choisie et déplacer vers la Corbeille) ;
- `outfits` : la liste des tenues ; `worn` : les éléments portés ;
- `save` : le dialogue Enregistrer sous ; `edit` : l'éditeur de tenue ;
- `menu` : une tenue dépliée avec un objet non porté, pour les menus du clic
  droit ;
- `duplicates` : rejoue des réponses de dossiers avec des liens répétés et
  des originaux venant d'autres parents.

Ces opérations restent hors ligne ; la création de nouveaux vêtements et de
nouvelles parties du corps n'est pas encore disponible.

Les objets de démo utilisent des points distincts (crâne, oreille gauche,
annulaire gauche, pied gauche et HUD en haut à droite), affichés dans les
listes d’Apparence et d’Inventaire.

**`AURORA_DEMO_INVENTORY`** — ouvre un inventaire synthétique : dossier
personnel, sous-dossier, tenues, objets, documents, texture, son, geste,
ciel, matériau, lien et permissions variées ; bibliothèque dans l'arbre,
racine ouverte et libellés alignés à gauche. Créations et mutations simulées
hors ligne ; les fichiers et photos chargés en démo restent hors ligne.

- `1` : l'inventaire seul.
- `folder`, `object`, `animation`, `script`, `note`, `properties`, `nocopy`,
  `notransfer`, `link` : le menu de l'élément choisi, ou ses propriétés.
  `clothes`, `body`, `settings`, `uploads` sont des alias de `folder`, à
  combiner avec `AURORA_DEMO_POINTER` pour survoler les sous-menus.
- `animation-open`, `animation-properties`, `image`, `image-photo`,
  `image-picker` : les cinq panneaux de référence.
- `image-photo-save` : capture puis sauvegarde une vignette à l'image 2100
  et ferme la fenêtre Photo en gardant l'éditeur Image ouvert.
- `folder-window` : une fenêtre limitée au contenu du dossier personnel,
  avec son nom en titre et un filtre propre ; `folder-window-search` montre
  cette recherche limitée aux descendants.
- `rename`, `new-script`, `new-note`, `new-folder` : le renommage dans la
  ligne, sans fenêtre (leurs captures simulent le focus du champ sans
  activer la fenêtre Windows).
- `multi-add`, `multi-detach`, `delete` : la sélection multiple et la
  confirmation de suppression.
- `sort`, `filters`, `preferences`, `recent`, `worn`, `filtered`, `large` :
  tri système / date, filtres, préférences, arborescence Récent / Porté,
  filtre excluant les objets ou recherche de 1 500 éléments.
- `long`, `long-filtered` : les noms longs et le défilement horizontal dans
  l'arbre complet ou filtré.
- `resize-left`, `resize-right` : avec une capture, tirent le bord
  correspondant au-delà de la taille minimale puis relâchent la souris.

Données synthétiques uniquement ; aucun benchmark de rendu.

Ce que l'inventaire doit montrer avec ces scénarios :

- **Filtres** sélectionne les types, permissions, liens, créateur et
  ancienneté ; **Préférences** règle le tri, les onglets, les dossiers
  inclus dans la recherche et le double-clic. Ces options sont aussi dans
  Préférences › Interface.
- Les dossiers système viennent en premier ; les éléments sont triés du
  plus récent au plus ancien et les dossiers par nom.
- La recherche peut porter sur le nom, la description, le créateur ou
  l'UUID ; `+` combine des termes, `"mot"` cherche un mot exact.
- Les éléments restent dans leurs dossiers, y compris dans Récent, Porté,
  les recherches et les vues filtrées. Réduire / Développer agit sur ces
  dossiers ; le sélecteur des champs est à droite de la saisie.
- « Afficher aussi les dossiers sans résultat » ajoute les dossiers vides à
  la vue, les dossiers parents des résultats étant toujours présents.
- La liste défile aussi horizontalement quand un nom dépasse la largeur
  disponible ; les noms longs ne bloquent pas la réduction de la fenêtre.
  Les commandes et onglets s'adaptent aux petites largeurs. À la taille
  minimale, le bord déplacé s'arrête sans déplacer le bord opposé de la
  fenêtre.
- Le chemin et la date restent disponibles en info-bulle. Récent part de la
  dernière déconnexion (24 h au premier lancement).
- Les filtres restent propres à chaque fenêtre ; « Garder par défaut » les
  mémorise pour les prochaines ouvertures.

## Médias et son

| Variable | Effet |
|---|---|
| `AURORA_DEMO_MEDIA=<url>` ou `1` | Média sur une prim |
| `AURORA_DEMO_PARCEL_MEDIA=<url>` ou `1` | Média de la parcelle |
| `AURORA_DEMO_MEDIA_CLICK="x,y[,x2,y2]"` | Clics sur le média |
| `AURORA_DEMO_MEDIA_CLICK_FRAME=<n>` | Image de départ de ces clics |
| `AURORA_DEMO_MUSIC=<url>` ou `1` | Musique de la parcelle de démo |
| `AURORA_DEMO_SOUND=1` | Sons du monde et de l'interface audibles |

### Détails

- **`AURORA_DEMO_MEDIA_CLICK`** : clic de focus à l'image 700 par défaut,
  deuxième clic 60 images après, clic facultatif sur le champ 120 images
  après, puis saisie « aurora ». `AURORA_DEMO_MEDIA_CLICK_FRAME` change
  l'image de départ pour laisser charger le plugin ; fonctionne aussi avec
  `AURORA_DEMO_HUDS=media`.
- **`AURORA_DEMO_MUSIC`** : `1` désigne une adresse factice injoignable ; la
  musique attend le bouton radio.
- **`AURORA_DEMO_SOUND=1`** : sons du monde (carillon en boucle) et sons de
  l'interface (pris dans le vrai cache de sons s'il existe, sinon un bref
  tic par son).

## Réglages forcés et diagnostics

| Variable | Effet |
|---|---|
| `AURORA_OCCLUSION=0` ou `1` | Remplace le réglage « Occlusion » (Graphismes › Qualité) |
| `AURORA_NO_OCCLUSION=1` | Coupe l'occlusion GPU |
| `AURORA_NOVSYNC=1` | Coupe la synchronisation verticale |
| `AURORA_CPU_CULL=1` | Listes de dessin construites sur le CPU |
| `AURORA_RENDER_THREAD=0` | Images dessinées sur le thread principal |
| `AURORA_PROFILE=1` | Profilage dans le journal (voir [Profilage](#profilage)) |
| `AURORA_PROFILE_FRAMES=1` | Profil détaillé de chaque image |
| `AURORA_GPU_VALIDATION=1` | Couches de validation de wgpu |
| `AURORA_HOVER_CHECK=1` | Diagnostic du curseur de survol |
| `AURORA_DEBUG_GLOW=1` | Diagnostic du glow |
| `AURORA_GLOW_SKIP=<masque>` | Retire une famille de faces du glow |
| `AURORA_MEDIA_DEBUG=1` | Diagnostic des médias |

### Détails

- **`AURORA_CPU_CULL=1`** : construit toutes les listes de dessin sur le CPU
  (le repli quand le GPU n'a pas `MULTI_DRAW_INDIRECT_COUNT`) au lieu de les
  trier sur le GPU, pour comparer ; `cull=cpu|gpu` dans la ligne
  `perf summary`.
- **`AURORA_RENDER_THREAD=0`** : dessine les images sur le thread principal,
  comme avant le thread de rendu, pour comparer et déboguer. Par défaut, le
  thread principal remet chaque image au thread de rendu sous la forme d'un
  paquet autonome et passe à l'image suivante ; avec `0`, le même paquet est
  exécuté sur place. `thread=on|off` dans la ligne `perf summary`, et une
  ligne de journal au démarrage (`renderer: frames drawn by…`).
- **`AURORA_PROFILE=1`** : une ligne `perf summary` par seconde, plus une
  ligne `perf settings` quand les réglages changent. Le plafond d'images/s
  en arrière-plan (« Limiter hors focus ») est ignoré : une fenêtre sans le
  focus est quand même mesurée à pleine vitesse.
- **`AURORA_PROFILE_FRAMES=1`** : en plus, une ligne `render profile` et une
  ligne `gpu profile` par image (étapes du renderer et temps GPU par élément
  de chaque image).
- **`AURORA_HOVER_CHECK=1`** : chaque réponse sur l'objet sous le curseur
  (gardée d'une image précédente, ou cherchée parmi les candidats de l'index
  de picking) est comparée à la recherche complète dans la scène ; les
  différences sont journalisées sous `hover check`, avec toutes les dix
  secondes les recherches faites et les réponses réutilisées. Coûte la
  recherche complète à chaque image.

## Profilage

Avec `AURORA_PROFILE=1`, chaque ligne `perf summary` couvre une seconde.
Deux threads se partagent une image : le **thread principal** fait tourner
la simulation et l'interface et clôt l'image sous la forme d'un paquet ; le
**thread de rendu** transforme le paquet en travail GPU pendant que le
thread principal est déjà sur l'image suivante. Avec
`AURORA_RENDER_THREAD=0`, tout ce qui suit tourne sur le thread principal.

- `fps`, `frame` (temps moyen d'une image, en ms), `p95`, `max` et `slow` :
  le nombre d'images au-delà de 1,5 × la médiane de cette seconde (les
  à-coups que montre le graphe des images). L'image est celle du thread
  principal : le temps entre deux tours de sa boucle, attentes comprises.
- **Thread principal** : le temps CPU moyen de chaque étape de l'image
  (`events`, `social`, `sync`, `media`, `lists`, `stream`, `params`, `ui`,
  `render`…), dont la somme donne `frame`. `render` est ce que le rendu
  coûte au thread principal : le paquet d'image et l'attente que le thread
  de rendu le prenne (tout l'encodage, attente de la swapchain comprise,
  avec `AURORA_RENDER_THREAD=0`).
- `max:` : les étapes dont le temps le plus long sur une seule image a
  atteint 1 ms dans cette seconde, la plus longue d'abord, étapes du
  renderer (`r_*`), parties du streaming et de la synchro (`s_*`) et champs
  `thread` compris, ou `-` s'il n'y en a aucune. Une image lente périodique
  y apparaît avec l'étape qui l'a causée, par exemple
  `max: media=6.10 render=2.31`, alors que sa moyenne reste minuscule.
- `thread=on|off`, puis la rencontre des deux threads :
  - `packet` (thread principal, dans `render`) : la clôture de l'image —
    les plages modifiées des tables de la scène dans le journal d'écritures
    de l'image, une copie des listes de dessin CPU — et sa remise ;
  - `wait_render` (thread principal, dans `render`) : le temps passé à
    attendre que le thread de rendu finisse l'image précédente
    (contre-pression : le thread de rendu tient au plus un paquet, aucun
    n'attend derrière). Une image qui porte une capture est attendue
    entièrement ;
  - `rt_frame` (thread de rendu) : tout son temps sur une image, du paquet à
    la fin de la présentation, attente de la swapchain comprise (0 avec
    `thread=off`) ;
  - `rt_idle` (thread de rendu) : le temps passé à attendre le paquet du
    thread principal.

  Une image coûte à peu près le plus long du travail du thread principal
  (`frame` moins `wait_render` et `limiter`) et de `rt_frame`. `rt_idle`
  au-dessus de 0 avec `wait_render` proche de 0 : c'est le thread principal
  qui limite ; l'inverse : c'est le thread de rendu, ou le GPU / la vsync
  derrière lui quand `r_acquire` ou `r_present` tiennent l'essentiel de
  `rt_frame`.
- **Thread de rendu** (thread principal avec `thread=off`) : les étapes du
  renderer (`r_*` : `resources` — la relecture du journal d'écritures —,
  passes, egui, `finish`, `submit`, `present`, `acquire`, l'attente de
  l'image de la swapchain). Avec le thread de rendu, ce sont celles de
  l'image précédente : le résultat d'une image revient avec la remise
  suivante.
- Thread principal de nouveau :
  - les parties du travail de streaming (`s_*`, dans `results` et
    `stream`) : `fetched` téléchargements remis aux streamers, `geometry`
    géométrie construite et placée dans l'arène, `decoded` autres tâches
    terminées, `tex_update` téléchargements et décodages lancés, `assets`
    mesh / animations / sons / matériaux, `skin` liaisons de skin, `upload`
    textures envoyées au GPU, dont `pages` nouvelles pages de textures,
    `maintain` éviction et écritures du cache, `diag` ;
  - les parties de la synchro de la scène (dans `sync`) : `sync_list` objets
    retirés et modifiés et liste de l'image, `sync_plan` leur placement,
    `sync_apply` déplacements et synchros complètes qui tiennent dans le
    budget de temps de l'image, `sync_alpha` faces reclassées après le
    changement de classe alpha d'une texture ;
  - le temps GPU par élément (`g_*`), les draws et commandes de dessin, les
    objets synchronisés / reconstruits, les avatars posés, les octets
    d'enregistrements et de palettes envoyés au GPU et ceux de tout le
    journal d'écritures de l'image (`journal_kb`), la mémoire des textures
    (textures vivantes, pages de textures et leur mémoire allouée), la
    mémoire de la géométrie, et les objets dont la synchro complète attend
    une image qui a du temps (`sync_backlog`).
