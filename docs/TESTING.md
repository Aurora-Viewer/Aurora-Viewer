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

Sommaire : [Fenêtres automatisées](#fenêtres-de-test-automatisé-sous-windows) ·
[Général](#général) · [Captures](#captures) ·
[Caméra, déplacements et pointeur](#caméra-déplacements-et-pointeur) ·
[Objets et construction](#objets-et-construction) · [HUDs](#huds) ·
[Avatars et animations](#avatars-et-animations) ·
[Rendu et environnement](#rendu-et-environnement) ·
[Charge et fluidité](#charge-et-fluidité) · [Interface](#interface) ·
[Médias et son](#médias-et-son) ·
[Réglages forcés et diagnostics](#réglages-forcés-et-diagnostics) ·
[Profilage](#profilage)

## Fenêtres de test automatisé sous Windows

Lors d'un test sans intervention humaine, ajouter
`[AUTOMATISATION - NE PAS TOUCHER]` au titre et lancer le contour jaune en
pointillés. Depuis le worktree de la tâche, après compilation :

```powershell
$env:AURORA_DEMO = '1'
$viewer = Start-Process -FilePath '.\target\debug\aurora-viewer.exe' -PassThru `
  -ArgumentList '--title "Test ma-tâche [AUTOMATISATION - NE PAS TOUCHER]"'
$borderScript = (Resolve-Path '.\scripts\automation-window-border.ps1').Path
$borderArgs = @(
  '-NoProfile', '-NonInteractive', '-STA', '-ExecutionPolicy', 'Bypass',
  '-File', ('"{0}"' -f $borderScript),
  '-ViewerProcessId', $viewer.Id
)
Start-Process -FilePath 'powershell.exe' -WindowStyle Hidden -ArgumentList $borderArgs
```

Utiliser le PID de l'exécutable du viewer, jamais celui de `cargo`. Le viewer
et le script doivent tourner dans la même session Windows, sur le bureau
interactif. Le script utilise Windows PowerShell 5.1 en mode STA.

Le contour suit la position et la taille des fenêtres portant le tag, sans
prendre le focus ni intercepter les clics. Il disparaît lorsque la fenêtre
est réduite, cachée ou perd son tag, et le script s'arrête à la fermeture du
processus du viewer. Pour un test avec intervention humaine, garder le titre
`Test <tâche>` sans le tag.

## Général

| Variable | Effet |
|---|---|
| `AURORA_DEMO=1` | Mode démo hors ligne |
| `AURORA_DEMO_POS="x,y"` | Position de départ |
| `AURORA_DEMO_TOD=0..4` | Heure du jour |
| `AURORA_DEMO_ALTITUDE=<m>` | Place tout le lieu de démo à cette altitude |
| `AURORA_FPS_LIMIT=<n>` | Plafond d'images/s pour les essais (10–500) |
| `AURORA_WINDOW_SIZE="l,h"` | Taille de la fenêtre pour ce lancement |
| `AURORA_LOG_NAME=<nom>` | Nom du fichier journal |
| `AURORA_EMOJI_FONT=<chemin>` | Utilise une autre police emoji |
| `AURORA_MAX_AVATARS`, `AURORA_MAX_COMPLEXITY`, `AURORA_AA`, `AURORA_SHADOWS` | Remplacent ces réglages |

Le limiteur d'images/s est activé à **120 images/s par défaut**. Il peut
être désactivé ou réglé dans Préférences › Graphismes › Fluidité ; les
préférences déjà enregistrées sont conservées.

`AURORA_DEMO_ALTITUDE` : terrain, objets et avatars sont relevés d'autant
(0 à 4 000 m). Sert à voir ce que la simple précision fait à l'image en
hauteur : entre 2 048 et 4 096 m, une position ne varie que par pas de
0,24 mm, soit environ un pixel en gros plan.

`AURORA_AA` : 0 aucun, 1 FXAA, 2 à 4 MSAA 2x / 4x / 8x, 5 TAA, 6 SMAA. La
démo garde le réglage par défaut (3, MSAA 4x), qui a sa propre profondeur
multi-échantillonnée, remplie par sa propre prépasse (terrain et faces
opaques, voir `AURORA_MSAA_PREPASS`) ; les autres modes réutilisent la
profondeur de la prépasse à un échantillon dans la passe de scène, faces
masquées comprises. Un changement des vertex shaders ou des passes se
vérifie donc aussi avec `AURORA_AA=6` : c'est avec lui que le terrain
laissait voir le ciel par taches (`AURORA_DEMO_POS="60,128"`,
`AURORA_DEMO_ALTITUDE=86`).

`AURORA_WINDOW_SIZE` : largeur et hauteur de la zone de rendu, en pixels
logiques (au moins 800 × 560), pour mesurer selon la taille de la fenêtre :
sans elle, la démo s'ouvre à la taille qu'avait la fenêtre du dernier
lancement fermé, quel que soit l'agent qui l'a lancé. La fenêtre est placée
comme celle d'une capture (ni position ni agrandissement enregistrés) et sa
taille n'est pas retenue pour le lancement suivant. La taille réelle est en
tête de la ligne `perf settings`.

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
| `AURORA_DEMO_AGENT_JITTER=<mm>` | Petites corrections de position de notre avatar |
| `AURORA_DEMO_KEY=<flèche>` | Tient une flèche du clavier |
| `AURORA_DEMO_SIT=<n>` | Clique n fois sur le bouton S'asseoir |
| `AURORA_DEMO_MMO="x,y[,1\|2]"` | Pilotage de l'avatar à la souris |
| `AURORA_DEMO_RCLICK="x,y"` ou `tag` | Clic droit (menu contextuel) |
| `AURORA_DEMO_PIE=1` | Menu circulaire du clic droit |
| `AURORA_DEMO_POINTER=<points>` | Déplace le pointeur sur l'interface |
| `AURORA_DEMO_HOVER=1` ou `"x,y"` | Curseur balayé, ou posé sur un pixel, au survol |
| `AURORA_DEMO_CLICKS=<ms>` | Clics sur le monde à cadence régulière |
| `AURORA_DEMO_LOOKAT=1` | Suivi du regard et regard d'un autre avatar |
| `AURORA_DEMO_KEYBOARD=<disposition>` | Touches de déplacement selon la disposition |
| `AURORA_DEMO_CHATCMD="<commandes>"` | Tape des commandes dans la barre de chat |

### Détails

- **`AURORA_DEMO_CAM`** : décalage de cap et inclinaison de la caméra autour
  de l'avatar (radians, une inclinaison positive regarde vers le bas) et
  distance (mètres).
- **`AURORA_DEMO_CAMERA=alt,x,y|pan,x,y|zoom,x,y|close,x,y[,n]|drift,x,y[,n]|tag|ml|wheel|fly|sit`** :
  scénario journalisé sous `demo camera` — Alt+clic en (x, y) puis glisser et
  retour en marchant ; `close` : Alt+clic en (x, y) à l'image 600, zoom
  pendant n images (60 par défaut), puis la caméra ne bouge plus (gros plan
  à caméra fixe, pour comparer des captures consécutives) ; `drift` : le
  même gros plan, puis une orbite très lente (un cinquantième de pixel de
  souris par image) jusqu'à l'image 1200 ; appui sur notre propre étiquette de nom puis glisser
  (pilotage) ; entrée / sortie de la vue subjective ; molette ; recul en
  vol ; assise sur un siège qui tourne, avec une caméra de siège et une
  animation lancée par une prim liée (image 300), puis lever et demande de
  son arrêt (image 700 ; `camera/demo.rs`).
- **`AURORA_DEMO_TP`** : téléportation après le fondu du chargement initial
  (image 240 au plus tôt). `1` ou `"x,y,z"` : arrivée locale immédiate, sans
  écran de chargement ; `remote` simule la progression entre régions.
- **`AURORA_DEMO_AGENT_JITTER=<mm>`** : notre avatar reçoit cinq fois par
  seconde (à 60 images/s) une correction de position d'au plus cette taille,
  comme en envoie un simulateur à un avatar debout en altitude. Avec
  `AURORA_DEMO_CAMERA=close,x,y` sur un objet posé, l'image reste identique
  au pixel près ; sur notre avatar, la caméra le suit et tout le décor
  proche saute. À combiner avec `AURORA_DEMO_ALTITUDE` et
  `AURORA_STILL_CHECK=1`.
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
- **`AURORA_DEMO_POINTER="x,y[,l|r|s][;x,y…]"`** : déplace le pointeur de
  l'interface vers ces pixels de la fenêtre à partir de l'image 300, un point
  toutes les 60 images (états de survol, sous-menus) ; `,l` fait un clic gauche,
  `,r` un clic droit sur l'interface à cet endroit. Dans le pie menu, le clic
  gauche entre dans un sous-menu et le clic droit remonte d'un niveau. `,s`
  laisse le pointeur immobile à cet endroit (il n'y est ramené que s'il en a
  bougé) : c'est ce qu'il faut pour capturer une info-bulle, qu'egui n'affiche
  que sous un pointeur au repos.
- **`AURORA_DEMO_HOVER=1`** : à partir de l'image 300, le curseur balaie la
  fenêtre pendant 120 images, reste immobile 120 images, et ainsi de suite
  (recherche de l'objet sous le curseur et réponses gardées d'une image à
  l'autre, `scene/hover.rs`). Avec `"x,y"`, le curseur reste sur ce pixel
  de la fenêtre à partir de l'image 300 (sur un avatar de la foule, sur le
  sol à côté). Lire `hover` et les lignes `hover summary` dans le profil ; à
  combiner avec `AURORA_HOVER_CHECK=1`, `AURORA_DEMO_ACTIONS=1` ou une
  scène de charge.
- **`AURORA_DEMO_CLICKS="<ms>[,x,y|sweep|rigged][,left|alt|menu|build|focus]"`** :
  un clic sur le monde toutes les `ms` millisecondes (250 si la valeur n'est
  pas un nombre) une fois le monde affiché, bouton tenu 60 ms, par le même
  chemin qu'un bouton de la souris (`app/click_pick.rs`), sans jamais
  prendre le vrai curseur (Alt + clic, clic sur notre avatar). Une ligne
  `demo clicks: the click before is not over` signale un clic dont le
  relâchement n'a pas été traité avant le suivant. Au pixel (x, y) de la
  fenêtre (un quart de la largeur, près du bas, par défaut : le sol), ou
  avec `sweep` à un nouvel endroit de la fenêtre à chaque clic, ou avec
  `rigged` sur les mesh riggés à l'écran, tour à tour (au milieu de leurs
  limites posées). `left` (par
  défaut) : clic gauche ; `alt` : Alt + clic (caméra) ; `menu` : clic droit ;
  `build` et `focus` : clic avec l'outil Modifier ou l'outil Focus de la
  construction. Sert à mesurer ce qu'un clic coûte à l'image dans une scène
  chargée (`AURORA_DEMO_CROWD=250`, `AURORA_DEMO_ANIMESH=1`,
  `AURORA_MAX_AVATARS=300`, `AURORA_AA=6` : le GPU y est la limite) : avec
  `AURORA_PROFILE=1`, lire `between` dans `max:` et `off_cpu:`, et les
  lignes `click pick:` (une par clic : images et millisecondes avant la
  réponse). À combiner avec `AURORA_CLICK_PICK=wait` ou `check`.
  - Clics sur ce qui est riggé : `AURORA_DEMO_ANIMESH=1`,
    `AURORA_DEMO_CROWD=4`, `AURORA_DEMO_POS="131,126.6"` et
    `AURORA_DEMO_CLICKS=150,rigged,menu` (ou `left`, `build`) mettent à
    l'écran un animesh posé (objets 9200 et 9202), un animesh porté par un
    autre avatar (9203), le mesh riggé de la foule et un mesh riggé porté
    par notre avatar (9210, ajouté par ce mode), tous rendus sensibles au
    toucher. Chaque clic écrit une ligne `demo drawn pick` : l'objet visé
    (`aimed at … : OK` ou `MISSED`, ce dernier quand autre chose le cache),
    l'objet désigné, sa face, et `inside its posed bounds` (`OUTSIDE`
    serait un défaut : le point lu sur le GPU n'est pas dans les limites
    que le CPU connaît de l'objet). Suivent `demo menu: …` (menu ouvert)
    ou `demo click action: Touch, object=…`. Avec `3000,rigged,menu` et
    `AURORA_CAPTURE_FRAMES=1100,1460`, les captures montrent le menu d'un
    objet porté par un autre avatar puis celui d'un objet porté par nous,
    l'objet entouré de son contour.
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
| `AURORA_DEMO_REPLACE_PRIM=1\|script` | Plateau de remplacement, cas du script seul |
| `AURORA_DEMO_BUILD_FRAME=<n>` | Image de départ du script de construction |
| `AURORA_DEMO_BUILD_FOCUS=off\|on` | Focus et suivi caméra pendant l’édition |
| `AURORA_DEMO_BUILD_EDGE=<bord>` | Déplacement jusqu’au bord, maintien puis relâchement |
| `AURORA_DEMO_BUILD_TAB=<onglet>` | Onglet de la fenêtre de construction |
| `AURORA_DEMO_COLOR_PICKER=<mode>` | Sélecteur de couleur et gestes hors ligne |
| `AURORA_DEMO_SCRIPTS=<mode>` | Édition et enregistrement de scripts hors ligne |
| `AURORA_DEMO_LSL_BRIDGE=1` | Messages du bridge LSL de Firestorm (cachés) |
| `AURORA_DEMO_FLEXIBLE=1` | Prims flexibles : gravité, tension, vent, force et parent mobile |

### Détails

`AURORA_DEMO_COLOR_PICKER=1|left|follow|link|toggle|dock|lsl|hex|invalid|drag|cancel|no-apply|palette|pipette`
ouvre le nuancier de la sélection, avec `AURORA_DEMO_BUILD=move,object:118`
et `AURORA_DEMO_BUILD_TAB=texture:bp`.

La palette s'ouvre contre le bord droit de la construction, au même niveau en
haut. `left` place la construction près du bord droit de l'écran : la palette
s'ouvre alors contre son bord gauche. Capturer vers l'image 660. Déplacer la
construction avec la palette ouverte : celle-ci suit, bascule de côté si la
place manque et reste visible. Le mode `follow` effectue ce glissement vers la
droite puis revient à gauche : captures 800 et 920, sans réouverture du nuancier.
Le mode `link` déplace séparément la palette : capture 800 pour le cône vers la
pastille d'origine et son contour violet, 880 pour son extinction au relâchement.
Le mode `toggle` clique trois fois sur la pastille : palette fermée à 710,
rouverte à 750 puis fermée à 790. La fermeture reprend les règles d'Annuler.
Le mode `dock` détache la palette (760), déplace la construction seule (840),
réaccroche à droite (920), vérifie le suivi et le basculement (1000), détache à
nouveau (1080), réaccroche à gauche (1160) et vérifie le suivi de ce côté (1240).
Le rapprochement doit être à 10 points du bord et du haut ; l'accrochage se
fait au relâchement. « Appliquer » est coché sans choix antérieur ; le décocher,
fermer puis rouvrir la palette conserve ce choix, y compris au redémarrage.

Les modes LSL / Hexa changent le format
par ses vrais boutons ; Hexa saisit `33AA66`, invalid saisit `GG0000` (OK
désactivé). Capturer vers l'image 740. Les modes drag / cancel / no-apply
glissent sur 100 images : capturer à 780 pendant le geste, 830 au relâchement,
900 après OK / Annuler. L'aperçu suit immédiatement, le serveur reçoit la
couleur finale au relâchement ; no-apply attend OK. Palette glisse l'aperçu
actuel dans la case 32 : capture 880 et réouverture pour vérifier sa sauvegarde.
Pipette arme l'outil ; cliquer une face visible pour prendre sa teinte (aucun
changement de sélection), relâcher pour quitter l'outil. Échap quitte d'abord
la pipette, puis annule le nuancier. Les gestes sont en coordonnées des contrôles
réels, sans dépendance à la position enregistrée de la fenêtre.

En mode Modifier, maintenir CTRL et tourner la molette parcourt les objets
réellement sous le pointeur : défilement vers le bas pour aller plus loin,
vers le haut pour revenir plus près, avec retour au début à la fin de la liste. Avec
Modification liée, seuls les prims sous le pointeur du même objet que la
sélection principale participent. Le geste remplace la sélection sans
déplacer la caméra. Sur une zone vide, la sélection reste inchangée.

`AURORA_DEMO_BUILD=cycle,object:99610` ajoute trois objets imbriqués,
dont deux linksets, sélectionne A puis simule deux crans vers le fond et
un vers l’avant : A → B → C → B. Le scénario
`cycle-linked,object:99610` active Modification liée : racine A → partie
A2 → racine A → partie A2, sans passer sur B ou C. Les lignes
`demo build cycle` du journal indiquent les sélections et vérifient que
la caméra et le zoom HUD ne changent pas pendant le geste. Capturer vers
l’image 690 (départ 600 par défaut).

`AURORA_DEMO_BUILD=create-delete,object:118` crée une primitive à l’image 625,
la sélectionne après la réponse simulée et la supprime à l’image 675. Capture
à 660 : nouveau cube sélectionné ; à 700 : cube disparu et message de mise
à la corbeille sans faux avertissement de propriétaire. `create-ready` laisse
le cube pour tester sa suppression à la main.

`AURORA_DEMO_BUILD=contents-copy,object:118` ouvre Contenu et l’inventaire,
puis copie un élément à l’image 645. `contents-move` propose la confirmation
du retrait de la texture non copiable (avec avertissement pour les scripts) ;
`contents-move-plain` montre le cas sans scripts. Capture à 670. Les choix
d’avertissement de l’utilisateur restent applicables. `contents-ready` ouvre
les deux fenêtres pour tester le glissement à la main : script copiable et
texture non copiable. Vérifier copie, annulation et acceptation du retrait,
dépôt sur un dossier, un élément et une zone vide, et refus sur les dossiers
de tenues / bibliothèque / Favoris pour la texture. Sur grille, l’humain
vérifie aussi les permissions de groupe, le contenu d’un autre propriétaire
et les éléments non copiables d’objets portés.

`AURORA_DEMO_BUILD=faces,object:99620` ajoute trois cubes : deux parties
liées (A / A2) et un objet séparé (B). Dans Matériaux (`AURORA_DEMO_BUILD_TAB=texture:bp`),
le script sélectionne une face à 625, en ajoute une sur le même prim à 645,
une sur A2 à 665, une sur B à 685, puis retire les deux premières faces
ajoutées à 705 et 725. Un clic simple sélectionne seulement le dessus de B à 745.
Maj retire cette dernière face à 765 : la sélection et le surlignage disparaissent,
et « Choisir une face » reste accessible pour quitter ce mode.
Capturer juste après ces étapes : une brève surbrillance repère les faces au clic,
puis un halo extérieur violet les identifie sans recouvrir les matériaux.
Le halo suit leur couverture visible, sans diagonales de triangles ni coutures UV.
`faces-invisible` reprend ces gestes sur des prims entièrement transparents :
seules les faces sélectionnées deviennent visibles par leur marquage.
`faces-ready` s’arrête avant les clics pour le test humain. Maj + clic ajoute
ou retire une face, y compris sur un autre objet ; retirer la dernière face
d’un prim le désélectionne. Sans Maj, la sélection précédente est remplacée.

`faces-close,object:99620` cadre le cube de près puis sélectionne une face :
la surbrillance s’efface en 0,7 s et seul le halo extérieur reste. Capturer à 626
et 700 pour comparer la teinte brève et la surface entièrement dégagée.
Le bouton œil « Revoir la sélection » dans Matériaux relance le repérage.

`faces-complex,object:99620` ajoute un tore fermé, un mesh dont une seule face
comprend trois îlots courbes avec des trous, et un cylindre creux coupé.
Il sélectionne le tore à 625 et ajoute la face du mesh avec Maj à 645.
Capturer à 646 puis 730 : surbrillance temporaire, silhouettes intérieures et
extérieures des deux faces sélectionnées, matériaux inchangés après le fondu.

`faces-picker,object:99620` sélectionne une face à 625 et ouvre son sélecteur
de textures à 635, avec un nom très long dans un sous-dossier « Textures du projet ».
Capturer à 660 puis 960 : la fenêtre garde sa largeur initiale de 680 points,
sans croissance autonome. La liste conserve les dossiers, les noms sont tronqués
et leur infobulle affiche le texte entier. Le séparateur ajuste la place donnée
aux noms, l’aperçu reste borné et les commandes sont regroupées en bas.

Dans le sélecteur de construction, un clic sur une texture copiable, un matériau,
« Défaut », « Vierge », « Transparent » ou « Appliquer » pour l’UUID applique
immédiatement le choix aux faces sélectionnées. « OK » conserve cet aperçu sans
renvoi ; « Annuler » et la croix restaurent la texture propre à chaque face.
Un changement de sélection annule également l’aperçu précédent. Vérifier sur
grille avec plusieurs objets et des faces ayant des textures différentes.
Comme dans Firestorm, une texture d’inventaire non copiable attend « OK » et
affiche cette indication. Les sélecteurs de profil et de terrain gardent leur
validation habituelle.

`AURORA_DEMO_FLEXIBLE` remplace les objets de la place par six prims avec
quatre niveaux de souplesse, profils carrés / circulaires, creux, torsion et
tailles non uniformes. La dernière suit une racine en rotation. `rigid` donne
la comparaison sans simulation ; un entier (6–2000) ajoute des prims pour
mesurer le coût. Le vent synthétique passe par le même événement que la grille.
`edit` isole une plaque inclinée pour les outils de construction.
Pour vérifier l'édition, ajouter `AURORA_DEMO_BUILD=move,object:90000` ou
`stretch,object:90000` : pivot sur la base, contour déformé, boîte droite
translucide et poignées de taille. `stretch,object:90000,fz,100,0` tire la face
opposée à la base ; le flexible garde sa courbure pendant le changement de taille.

**`AURORA_DEMO_SCRIPTS=inventory|object|both|save|object-save|error|readonly|syntax|object-syntax|syntax-readonly|indent|object-indent|tabstops|object-tabstops|close-brace|object-close-brace|colors|color-picker|color-accept|color-cancel|comment-lines|comment-block|multiline-indent|multiline-outdent|folding|object-folding|experience|experience-save|workbench|object-workbench|insert-filter|insert-empty|search|preferences|help|preprocessor|object-preprocessor|processed|preprocessor-error`** :
ouvre un éditeur de script depuis l'inventaire, le contenu d'un objet ou
les deux à l'image 280. `save` et `object-save` enregistrent une source
modifiée puis ferment et rouvrent l'éditeur à l'image 360 avec un UUID
d'asset masqué, pour vérifier la relecture ; `error` simule un diagnostic
de compilation ; `readonly`
refuse la lecture d'un script personnel sans droit Modifier. Les sources
enregistrées peuvent être fermées puis rouvertes dans la même session.
Saisir `AURORA_DEMO_SYNTAX_ERROR` simule une erreur du serveur ; la démo
n'exécute pas de compilateur LSL local.
`syntax` et `object-syntax` montrent la coloration des types, variables,
fonctions natives et personnalisées, événements, mots-clés, constantes,
nombres, chaînes et commentaires. `syntax-readonly` montre la même source
colorée et sélectionnable en lecture seule. La coloration suit les
modifications pendant la saisie et les couleurs du thème actif.
`indent` simule Entrée puis une saisie après une instruction indentée ;
`object-indent` fait la même chose avec des tabulations dans un objet.
Le commentaire ajouté doit être aligné sur l'instruction précédente.
`tabstops` et `object-tabstops` placent le curseur après sept espaces,
simulent Retour arrière (quatre espaces), Tab (huit espaces), puis une saisie
et Entrée. Le commentaire et l'instruction doivent être alignés sur huit
espaces. Tab ajoute des espaces jusqu'au prochain multiple de quatre ;
Retour arrière revient au palier précédent quand les espaces le permettent,
sans supprimer un caractère du code pour rejoindre ce palier.
`close-brace` et `object-close-brace` ferment deux blocs imbriqués par saisie
de `}`, avec espaces ou tabulations importées : les fermetures doivent être
alignées sur leurs ouvertures. Le collage conserve exactement ses retraits.
`colors` montre cinq pastilles près des vecteurs numériques entre 0 et 1
inclus ; position hors plage, rotation, chaîne et commentaire n'en ont pas.
`color-picker` ouvre la première pastille et choisit le vert ; `color-accept`
valide et `color-cancel` restaure exactement les valeurs d'origine. Le texte
reste sélectionnable pendant l'aperçu ; OK rend l'édition et l'enregistrement
disponibles et produit une seule action Annuler / Rétablir.
`comment-lines` et `comment-block` sélectionnent deux déclarations puis utilisent
Ctrl+K et Ctrl+Maj+K : `//` est ajouté après le retrait, ou la sélection est
encadrée par `/*` et `*/`. Répéter le raccourci retire les commentaires. Les
commentaires de bloc ne sont pas imbriqués ; utiliser les lignes `//` si la
sélection contient déjà un commentaire de bloc.
`multiline-indent` ajoute quatre espaces aux lignes sélectionnées ;
`multiline-outdent` les retire. Une sélection finissant au début d'une ligne
n'inclut pas celle-ci ; le sens de la sélection et Annuler / Rétablir restent
conservés. Maj+Tab retire aussi une tabulation littérale ou un retrait incomplet.
`folding` et `object-folding` cliquent sur la flèche du bloc `default` dans la
marge. Ses lignes disparaissent visuellement, leurs numéros sautent et un
libellé indique le nombre de lignes cachées. Cliquer à nouveau ou sur le libellé
déplie le bloc. Le menu Édition contient Tout replier / Tout déplier. La source
reste identique, y compris lors de la sauvegarde ou de la copie ; la recherche
et la navigation vers un caractère caché rouvrent son bloc. Les blocs des chaînes
et commentaires n'ont pas de flèche.
`experience` choisit et enregistre l'expérience simulée d'un script d'objet ;
`experience-save` ferme ensuite l'éditeur et le rouvre avec cette association.
Les autres démos d'objet commencent avec « Aucune ». Le sélecteur permet
aussi de retirer une association puis d'enregistrer. L'inventaire personnel
ne propose pas d'association d'expérience, comme Firestorm.

`workbench` et `object-workbench` montrent les menus Fichier / Édition / Aide,
la barre d'outils, les numéros de ligne, le curseur et le sélecteur de fonctions.
`insert-filter` ouvre Insérer, fait défiler les résultats puis saisit `color` :
le filtre doit rester visible et la source inchangée. `insert-empty` saisit
un nom absent et affiche « Aucune fonction correspondante ».
`search`, `preferences` et `help` ouvrent respectivement la recherche/remplacement,
les couleurs/options avec aperçu et l'aide locale de `llSay` avec sa signature.
Les commandes de saisie utilisent l'historique natif ; les réglages ne sont
jamais enregistrés sur le PC en démo. Les sélecteurs de fichiers et l'éditeur
externe restent des actions explicites du testeur.
`preprocessor` et `object-preprocessor` enregistrent un script utilisant une
inclusion de l'inventaire, une macro, une liste étendue et un `switch`, puis
le rouvrent : le texte original doit être conservé. `processed` montre le
résultat envoyé au simulateur, en lecture seule ; `preprocessor-error` montre
une erreur avant envoi, avec conservation du brouillon.
Les nouveaux messages `llOwnerSay` ouvrent le débogage des scripts si leur
routage est activé ; les erreurs
des objets possédés y apparaissent également, après filtrage du bridge et
des objets bloqués. La fenêtre permet de voir tous les objets ou un seul et
d'effacer les messages affichés.

**`AURORA_DEMO_ACTIONS`** — curseurs et clics d'objet comme Firestorm
(mêmes règles ; curseurs dessinés à partir des icônes Tabler).
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
La variante `mode,object:<identifiant local>[,part,dx,dy]` sélectionne un
objet de la démo sans coordonnées de clic fixes.

**`AURORA_DEMO_BUILD_FOCUS=off|on`** — focus sur la sélection avant de
prendre une poignée, avec « Suivre l’objet en édition » désactivé / activé.
Avec `off`, la vue reste fixe au centre de l’écran ; avec `on`, elle suit
les transformations locales sans à-coups ni dérive quand la souris reste
immobile. Tester le déplacement et l’étirement par face et par coin.

**`AURORA_DEMO_BUILD_EDGE=left|right|top|bottom`** — complète un script
`move` : la souris atteint le bord en 40 images, y reste 60 images puis
relâche la poignée. Avec le suivi désactivé, la caméra glisse progressivement
près du bord sur le même axe ou dans le même plan que l’objet, qui reste
visible pendant le maintien de la souris sans rapprochement ni éloignement
progressif. Tester un axe oblique à la caméra pour vérifier cette distance.
Capturer vers l’image de départ +120 pendant le maintien et +170
après le relâchement ; la caméra doit alors être arrêtée. Combiner avec
`AURORA_DEMO_BUILD_FOCUS=off` pour vérifier le focus sur l’objet, puis sans
cette variable pour vérifier le focus initial sur l’avatar. Les HUDs et le
suivi activé conservent leur comportement.

**`AURORA_DEMO_BUILD_TAB=general|object|transform|shape|features|texture[:pbr|bp|media]|contents`**
— onglet de la fenêtre de construction affiché par le script ; `+weights`,
`+grid`, `+media` ouvrent aussi ces fenêtres.

**`AURORA_DEMO_LSL_BRIDGE=1`** — messages du bridge LSL de Firestorm
(cachés) entre deux lignes « owner say ».

#### Remplacer un prim

Pour contrôler le passage à « Modifier les parties liées », combiner
`AURORA_DEMO_REPLACE_PRIM=1` et
`AURORA_DEMO_BUILD=linked-click,object:99688` : objet entier sélectionné,
activation de la modification liée à l'image 625, puis clic sur la partie #9
à l'image 635 (départ 600 par défaut). Capturer à 630 puis à 650 : 15 parties
sélectionnées deviennent une seule, avec « Numéro de lien : 9 » et le bouton
de remplacement. `linked-click-ready,object:99688` s'arrête sur l'objet entier
pour que l'humain puisse cocher l'option puis cliquer lui-même sur la partie.

`AURORA_DEMO_REPLACE_PRIM=1` ajoute un jeu de 15 prims (racine `99680`,
pile de cartes au lien 9 : `99688`) et un remplaçant indépendant (`99700`).
Le remplaçant a une rotation et des dimensions différentes ; son contenu est
vide. Un script du plateau démarre arrêté pour vérifier qu'il reste arrêté.
Les mises à jour des prims ne renseignent pas `OwnerID`, comme pour un prim
sans son ni particules sur la grille : les indicateurs de permission et les
réponses `ObjectProperties` confirment qu'ils appartiennent au résident.
Le choix du remplaçant et l'opération doivent fonctionner dans ce cas.

`AURORA_DEMO_REPLACE_PRIM=script` garde un seul script dans la pile de cartes,
avec les autres contenus vides. Le moteur simulé confirme les changements
d'état des scripts seulement après une première réponse portant l'ancien état.
Avec `AURORA_DEMO_BUILD=replace-copy,object:99688`, vérifier la copie du script,
la conservation de l'original et le retour à leur état actif, sans blocage.
Capturer le résultat à l'image 900 pour laisser les confirmations arriver.
L'avancement distingue la lecture des états, la copie vers l'inventaire et
l'insertion dans le remplaçant ; les erreurs de lecture du contenu remontent
directement, avec l'étape concernée.

À combiner avec `AURORA_DEMO_BUILD` :

- `replace-button,object:99688` : une partie liée sélectionnée, bouton dans la fenêtre de construction ;
- `replace-edit,object:99688` : remplaçant choisi, cinq options indépendantes, sans mutation ;
- `replace-run,object:99688` : remplacement avec les valeurs par défaut ;
- `replace-copy,object:99688` : copie du contenu et de la taille, puis remplacement ;
- `replace-run,object:99680` : remplacement de la racine et contrôle des positions des enfants.

Les scripts de construction commencent à l'image `AURORA_DEMO_BUILD_FRAME`
(600 par défaut). Le choix du remplaçant se fait à +35 images, la validation
à +55 pour les scénarios qui exécutent l'opération. Le simulateur renvoie les
changements de parentage, le contenu et les états des scripts par les mêmes
événements que le réseau. Sur la grille, contrôler aussi les numéros avec
LSL, les droits de copie et les confirmations lors d'une connexion lente.
Le protocole renvoie le parent des prims, sans numéro de lien : après
confirmation du parentage, Aurora conserve l'ordre envoyé dans ObjectLink,
indépendamment de l'ordre d'arrivée des paquets. Les 256 prims sont rattachés
par groupes confirmés, depuis la fin de la liste vers son début.

Le bouton apparaît sous les informations de sélection de l'inspecteur compact.
Le panneau reprend son cadre, ses titres et ses marges ; les actions de validation
restent hors de la zone défilante.

Le remplacement place l'ancien prim à la position initiale du remplaçant,
en conservant sa rotation et sa taille. Avec « Copier la position » activé,
les deux prims échangent donc leurs positions. Les objets
doivent être posés dans le monde, modifiables, appartenir au résident, sans
physique, caractère temporaire ni avatar assis. Maximum : 256 prims et
1 024 éléments copiés / scripts observés. Les conflits de noms et les contenus
non copiables arrêtent l'opération avant toute déliaison. Copier un script
ne conserve pas sa mémoire d'exécution ; remplacer la racine ne copie pas
automatiquement les données Linkset Data.


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
| `AURORA_DEMO_ANIMESH_TRIS=<n>` | Triangles ajoutés au mesh riggé de la démo |
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
- **`AURORA_DEMO_ANIMESH=1|churn`** : mesh racine animé, mesh lié signalé par
  un enfant et animesh porté, avec des squelettes propres indépendants et des
  ombres alpha. `churn` : l'animesh au sol (orange) change en plus d'asset de
  mesh toutes les 240 images, comme les mesh riggés qui vont et viennent sur
  la grille ; il doit rester entier et animé à chaque changement, les deux
  autres aussi, et la ligne `demo animesh: skin bindings <en service> used
  of <dans la liste>` du log (toutes les 120 images) doit cesser de croître
  une fois les premiers mesh libérés (600 images après leur dernier usage).
- **`AURORA_DEMO_ANIMESH_TRIS=<n>`** : avec `AURORA_DEMO_ANIMESH`, ajoute
  environ n triangles au mesh riggé de la démo, en faces opaques de 45 000
  (un tube fin autour du torse chacune, 7 faces au plus). Avec
  `AURORA_DEMO_CROWD=<n>`, chaque avatar de la foule porte ce mesh : 200000
  donne des avatars du poids de ceux de la grille (3,6 millions de
  triangles à l'écran pour 16 avatars dessinés en entier), ce qu'il faut
  pour mesurer les passes qui redessinent les avatars (ombres, prépasse).
- **`AURORA_DEMO_CLOUD=1|self`** : `1` active hors ligne les nuages et les
  barres de chargement. `self` habille en plus notre avatar comme ceux qui
  restaient en nuage pendant les 60 s de la limite (issue #121) : bake de
  tête invisible (calque alpha), haut et bas texturés, un mesh porté que le
  serveur d'assets n'a pas (404) et un chapeau. Il doit apparaître environ
  5 s après son arrivée, sans tête et sans le mesh, et le journal contenir
  « avatar … (own) shown after 5.0 s ». Captures avec `AURORA_FPS_LIMIT=60`
  et `AURORA_CAPTURE_FRAMES=240,1200` : nuage, puis avatar.

## Rendu et environnement

| Variable | Effet |
|---|---|
| `AURORA_DEMO_PARTICLES=<mode>` | Particules de vent et de fumée, ou charge de particules |
| `AURORA_DEMO_PLANAR=1` | Dalles en mapping de texture planaire |
| `AURORA_DEMO_PBR_OVERRIDE=1` | Override de matériau GLTF |
| `AURORA_DEMO_TEXANIM=1` | Animations de texture (llSetTextureAnim) |
| `AURORA_DEMO_SKY=<gamma>[,<brume>]` | Ciel EEP classique avec ce gamma |
| `AURORA_DEMO_OCCLUSION=1` | Mur avec des objets cachés (test d'occlusion) |
| `AURORA_DEMO_DEBUG="<calques>"` | Calques de débogage |
| `AURORA_DEMO_EEP_PARCEL=<mode>` | Environnement EEP de la région et de la parcelle |
| `AURORA_DEMO_ENV_SELECT=<mode>` | Sélecteur d'environnement |

### Détails

- **`AURORA_DEMO_PARTICLES=wind|smoke|both|legacy`** : traits de vent de la
  moto et paramètres du script de fumée reçus par des mises à jour
  compressées, devant un panneau sombre ; format étendu du glow (`legacy` :
  vent sans glow), texture douce par défaut des particules hors ligne.
  `load` : à la place, 32 fontaines de plus en cercle autour de la place,
  soit environ 4 000 particules vivantes, pour mesurer les parties
  `p_part_*` du profil (à combiner avec `AURORA_DEMO_STREAM=<n>` pour le
  nombre d'objets).
- **`AURORA_DEMO_PLANAR=1`** : les carreaux doivent se raccorder d'une dalle
  à l'autre.
- **`AURORA_DEMO_PBR_OVERRIDE=1`** : deux dalles PBR, dont une avec un
  override de matériau GLTF (4 × 4 répétitions, teinte).
- **`AURORA_DEMO_TEXANIM=1`** : deux rangées de panneaux à la place des
  panneaux alpha — défilement lisse, grille de 4 × 4 images, ping-pong,
  rotation, échelle (masque alpha : prépasse et ombres), un cube animé sur
  une seule face, un matériau legacy (la normal map suit) et une face PBR.
- **`AURORA_DEMO_SKY=<gamma>[,<brume>]`** : ciel EEP classique (sans ambiance
  des sondes de reflets) avec ce gamma de ciel et une couleur du soleil
  supérieure à 1 : gamma legacy et lumière des objets normalisée comme
  Firestorm. `brume` : multiplicateur de distance de la brume (0,8 par
  défaut ; `37` donne un ciel brumeux comme en portent beaucoup de régions,
  où ce qui est à quelques dizaines de mètres est voilé). Avec
  `AURORA_DEMO_CROWD=40` et `AURORA_MAX_AVATARS=4`, les imposteurs doivent
  être voilés comme les avatars dessinés en entier à côté d'eux
  (`AURORA_MAX_AVATARS=60` pour comparer).
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
| `AURORA_DEMO_CROWD=<n>[,busy]` ou `<n>,<portés>[,<triangles>]` | Test de charge de la synchro : n avatars ; avec `busy` ce que la synchro place encore elle-même, avec `<portés>` des avatars habillés de mesh riggés |
| `AURORA_DEMO_OVERDRAW=<n>` | Test de recouvrement : n rangées de murs opaques |
| `AURORA_DEMO_STREAM=<n>[,<vague>][,leave][,j2k=<dossier>]` | Test des à-coups du streaming |
| `AURORA_DEMO_HAIR=<couches>[,<segments>[,<vide>[,<période>]]]` | Charge des faces transparentes : coques superposées sur les avatars |
| `AURORA_DEMO_LIGHTS=<n>` | Charge des lumières locales : n petites lumières dans la foule |

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
  Avec `<portés>` (300 au plus) et `AURORA_DEMO_ANIMESH`, la foule d'un
  club : chaque avatar danse (tout le corps se déplace d'un demi-mètre,
  ce qu'il porte bouge à chaque image) et porte en plus `<portés>` mesh
  riggés, dont un sur cent a un script de toucher, des faces transparentes
  et `<triangles>` triangles (20 000 par défaut). C'est la charge de la
  recherche de l'objet sous le curseur parmi des avatars habillés :
  `AURORA_DEMO_CROWD=52,200` donne 13 500 objets dont 10 500 mesh riggés ;
  à combiner avec `AURORA_DEMO_HOVER=1`, `AURORA_DEMO_POS=129,134` et
  `AURORA_FPS_LIMIT=160` (ce qui bouge de plus de 2 mm par image près du
  rayon du curseur fait refaire la recherche : le nombre d'images/s compte).
  Avec `,busy` après le nombre, s'ajoute ce que la synchro place encore
  elle-même dans un lieu plein d'avatars : sur chaque avatar un ruban
  flexible et un ornement qui tourne avec deux prims liés, sur notre avatar
  un HUD de 250 prims (en bas à gauche de l'écran), et au sol 60 objets qui
  tournent avec un prim lié. À combiner avec `AURORA_DEMO_STREAM=12000,400`
  pour les objets immobiles ; `AURORA_SYNC_CHECK=1` donne ce que la synchro
  visite, `AURORA_SYNC_SLICE=alt` compare avec l'ancien chemin.
- **`AURORA_DEMO_HAIR=<couches>[,<segments>[,<vide>[,<période>]]]`** : notre
  avatar et chaque avatar de `AURORA_DEMO_CROWD` portent `couches` mesh
  riggés (8 par défaut, 32 au plus), chacun une coque autour de la tête et
  une autour du torse, un peu plus grandes d'une couche à la suivante : les
  mèches superposées d'une chevelure mesh et les vêtements en couches, soit
  `couches` fragments transparents par pixel couvert. `segments` (24 par
  défaut, 4 à 180) fixe les triangles : 4 × segments² par couche et par
  avatar. La texture est faite de mèches verticales espacées de `période`
  texels (16 par défaut, 4 à 256, sur 256), dont `vide` pour cent sont
  entièrement transparents (60 par défaut, 0 à 90).
  - À lire dans `perf summary` : `g_transparent` + `g_post` (voir
    [Profilage](#profilage)), `blend`, `blend_tris_k`. `32` avec
    `AURORA_DEMO_CROWD=40` et `AURORA_DEMO_POS="129,134"` donne ~970 faces
    transparentes (17 avatars dessinés en entier).
  - Pour l'abandon précoce des fragments invisibles
    (`AURORA_BLEND_EARLY_DISCARD`) : il ne rapporte que là où des zones
    vides sont d'un seul tenant à l'écran. `32,24,90,256` (grandes plages
    vides) le montre, `32,24,60,16` (mèches fines) ne gagne rien.
- **`AURORA_DEMO_LIGHTS=<n>`** : n petites lumières locales (64 si la valeur
  n'est pas un nombre, 256 au plus), des sphères lumineuses de 6 cm à
  hauteur de tête, espacées de 1,6 m sur la place de la foule et au-delà,
  de 3 m de rayon (4,5 m une fois étendues comme les lumières différées de
  Firestorm) : les « face lights » d'une foule et les lampes d'un club. Le
  rendu prend les 64 plus proches (`lights=` dans `perf summary`). Avec
  `AURORA_DEMO_POS="129,134"` elles sont devant la caméra, à 9 m et plus ;
  avec `"140,134"`, la caméra est au milieu d'elles. Sert à mesurer
  `AURORA_LIGHT_TILES`.
- **`AURORA_DEMO_OVERDRAW=<n>`** : n rangées de murs opaques (12 si la
  valeur n'est pas un nombre, 40 au plus) à l'est de la place, l'une
  derrière l'autre et chacune plus haute que celle de devant, envoyées de la
  plus lointaine à la plus proche. Chaque mur dépasse du suivant : l'occlusion
  les garde tous, et une passe de scène qui repart d'une profondeur vide
  calcule la couleur de chacun en entier avant que les plus proches le
  recouvrent. C'est le cas le plus défavorable du recouvrement, pour mesurer
  ce qu'une prépasse épargne (`AURORA_MSAA_PREPASS`) ; une région réelle en
  a moins.
- **`AURORA_DEMO_STREAM=<n>[,<vague>][,leave][,j2k=<dossier>]`** : n objets texturés (2000
  si la valeur n'est pas un nombre) qui arrivent par vagues après le fondu de
  chargement, `vague` objets toutes les 250 ms (100 par défaut, soit ~400 par
  seconde comme sur la grille ; `vague` = n envoie tout d'un coup, comme une
  arrivée par téléportation). Chaque objet a sa propre forme et sa propre
  texture, aux tailles d'une région de la grille (surtout 512 et 1024),
  décodée par les tâches de fond au quart de sa taille, puis en taille
  complète 1,5 s plus tard.
  - Le journal donne la fin du chargement (`demo stream: fully loaded in
    … s`, avec les envois préparés par les tâches / écrits par le thread
    principal, et les pages de textures créées par le thread des pages /
    par le thread principal, avec leur durée) ; avec `AURORA_PROFILE=1`,
    lire `stream`, `results` et les parties `s_*` dans `max:` (`s_pages` :
    les pages créées par le thread principal).
  - Avec `,leave`, la région est quittée une seconde après le chargement,
    comme par une téléportation : tous les objets sont retirés d'un coup et
    leur texture, qui contient alors les données téléchargées de la taille
    de son fichier J2C, est inutilisée et à évincer 2 s plus tard (`demo
    stream: region left`, puis `demo stream: textures evicted`) ; lire
    `s_maintain`, `sync`, `s_sync_list` et `r_submit` dans `max:`.
  - Avec `,j2k=<dossier>` après les nombres, les textures sont les fichiers
    `.j2c` de ce dossier (le `cache\tex` d'un viewer, repris en boucle
    s'il y en a moins que d'objets) au lieu de damiers dessinés : chaque
    fichier est remis au streaming comme une lecture réussie du cache
    disque et suit le vrai chemin à partir du décodage JPEG2000, au
    niveau que demande sa taille à l'écran. C'est le retour dans une
    région déjà en cache, sans la grille : lire `demo stream: loaded in
    … s` puis la ligne `texture load:` (voir [Profilage](#profilage)).
    Le dossier n'est que lu. Lancé deux fois de suite, le scénario rejoue
    aussi le retour avec le cache des textures transcodées : la seconde
    fois, les textures compressées viennent des fichiers `.bc7` laissés
    par la première dans le cache de la démo (`cache\demo\tex`, à
    supprimer pour repartir d'une première visite) et leur fichier J2C
    n'est demandé que s'il faut un niveau plus fin.

## Interface

| Variable | Effet |
|---|---|
| `AURORA_DEMO_WINDOWS=<mode>` | Repli, déplacement et restauration du titre de l’inventaire |
| `AURORA_DEMO_EXIT=<mode>` | Modale de fermeture et choix hors ligne |
| `AURORA_DEMO_UI=<onglet>` | Ouvre Personnes (onglet), l'inventaire et le chat |
| `AURORA_DEMO_RADAR=1` | Résidents autour de soi pour le radar de Personnes (toutes les portées) |
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
| `AURORA_DEMO_NOTIF=1`, `AURORA_DEMO_STATUSMENU=1`, `AURORA_DEMO_MICMENU=1`, `AURORA_DEMO_NAVEDIT=1` | Liste des notifications, menu de statut, panneau du micro, champ de lieu |
| `AURORA_DEMO_COLORPICK=1` | Ouvre le sélecteur de couleur compact de la première pastille affichée |
| `AURORA_DEMO_CONV=1`, `AURORA_DEMO_TALK=1` | Conversation de groupe, micro ouvert |
| `AURORA_DEMO_CONTACTS=<onglet>` | Onglet Contacts de Conversations |
| `AURORA_DEMO_GLYPHS=1` | Noms, titres et chat en caractères étendus (polices du système) |
| `AURORA_DEMO_PROFILE=<avatar>[:onglet]` | Fenêtre de profil d'un avatar |
| `AURORA_DEMO_FEED=<url>` | Onglet « Flux » du profil sur cette page |
| `AURORA_DEMO_APPEARANCE=<mode>` | Fenêtre Apparence et tenues |
| `AURORA_DEMO_INVENTORY=<mode>` | Inventaire synthétique, menus et fenêtres |
| `AURORA_DEMO_REZ=<mode>` | Rez par glisser-déposer depuis l’inventaire |
| `AURORA_DEMO_LAND=<onglet>[:owner]` | « À propos du terrain » sur un onglet |
| `AURORA_DEMO_PLACE=<lieu>` | « Lieux » sur un profil de lieu |
| `AURORA_DEMO_PLACES=<onglet>` | « Lieux » sur un onglet |
| `AURORA_DEMO_PLACE_WINDOW=1` | Profils de lieu en fenêtres séparées |
| `AURORA_DEMO_BAN=1` | Lignes d'interdiction |
| `AURORA_DEMO_RESTRICTED=1` | Parcelle qui interdit tout |

### Détails

- **Parcours de la refonte des fenêtres** : ouvrir l'inventaire et sélectionner
  un dossier dans l'arborescence, puis un élément ; vérifier recherche, permissions,
  menu contextuel et actions en bas. Agrandir et rétrécir la fenêtre :
  l'arborescence reste unique et occupe toute la largeur. Dans Apparence, changer de galerie / liste
  / portés, sélectionner une tenue et essayer Porter / Ajouter hors ligne.
  Dans « À propos du terrain », parcourir les sections en gardant la synthèse
  visible. Ouvrir ensuite Conversations, Personnes, Lieux, un profil,
  Préférences et Environnement ; déplacer, redimensionner, replier et restaurer
  les fenêtres. La construction garde sa largeur dans ses six sections, dont
  Forme pour les paramètres avancés ; les listes longues défilent dans le corps,
  les actions restent visibles. Les confirmations modales gardent le fond flouté.
- **`AURORA_DEMO_WINDOWS=1|restore|drag`**, avec `AURORA_DEMO_INVENTORY=1` :
  double-clic sur le titre réel de l’inventaire aux images 660–665 ; `1`
  garde la barre repliée, `restore` la développe aux images 1000–1005,
  `drag` la déplace aussi de 100 × 60 points aux images 780–782.
  Capturer `620,740,900,1120` pour vérifier contenu, repli, déplacement et
  dimensions restaurées. Le bouton « − » reste utilisable ; les autres
  boutons du titre et le contenu ne déclenchent pas le double-clic du titre.
- **`AURORA_DEMO_NOTIF=1`** : l’offre « Lanterne aurorale » est celle d’un
  résident : l’objet est déjà dans l’inventaire (dossier Objets) à l’arrivée
  de l’offre. « Garder », ou fermer l’offre dans la liste, l’y laisse ;
  « Rejeter » l’envoie à la corbeille. Avec `AURORA_DEMO_INVENTORY=offer`,
  l’inventaire s’ouvre sur l’objet reçu et l’offre reste à répondre ;
  `offer-reject` la rejette à l’image 700, puis montre l’objet dans la
  corbeille. Capturer après l’image 900.
- **`AURORA_DEMO_EXIT=1|cancel|logout|quit`** : ouvre la modale à l’image 650.
  `1` la laisse interactive ; les autres modes font le choix à l’image 800.
  Capturer `740,900` pour comparer le fond flouté puis la session reprise ou
  le retour au login. `quit` termine le processus à l’image 800 (capturer
  seulement 740, sans `AURORA_CAPTURE_EXIT`). Combiner avec
  `AURORA_DEMO_LOGIN=empty` pour l’écran de connexion : Déconnecter est
  désactivé. Les choix sont simulés hors ligne, sans requête de connexion.
- **`AURORA_DEMO_OPTIONS=<section>`** : par exemple `8` pour Raccourcis
  (touche de déplacement des HUDs).
- **`AURORA_DEMO_COLORPICK=1`** : à l'image 300, ouvre le sélecteur de
  couleur compact (`widgets::color_button`) de la première pastille de
  couleur dessinée. À combiner avec `AURORA_DEMO_OPTIONS=5` (Couleurs) : le
  panneau s'ouvre sous « Mon nom », avec le champ saturation / valeur, la
  barre de teinte, la nouvelle couleur au-dessus de l'ancienne, les champs
  R V B et hexadécimal, et la pipette.
- **`AURORA_DEMO_GLYPHS=1|2`** : huit résidents sur deux rangs devant
  l'avatar (le second rang en vol), un cube à texte flottant à droite et la
  fenêtre Conversations sur le chat local. Titre de groupe, nom d'affichage
  et message de chacun utilisent une famille de caractères : géorgien et
  arménien (le titre « ღAurora Friendღ » de l'issue #127), lettres
  mathématiques (gras, script, gothique, double trait), symboles et
  encadrés, cyrillique et grec, japonais / coréen / chinois, arabe et
  hébreu, autres écritures (syllabaire canadien, cherokee, devanagari,
  thaï, tamoul, tibétain…), emojis. `2` : mêmes messages dans l'ordre
  inverse (la fenêtre de chat par défaut montre les quatre derniers).
  Ce qu'il faut voir : aucun carré dans les étiquettes, le texte flottant,
  le chat et sa liste ; tous les caractères d'une ligne posés sur la même
  ligne de base. Comparer avec `AURORA_FONT_FALLBACK=0` (carrés partout
  sauf cyrillique, grec, emojis et quelques symboles). Limites visibles : les mots arabes et
  hébreux sont bien formés, mais ceux d'une phrase se suivent de gauche à
  droite ; drapeaux, tons de peau et séquences ZWJ non composés ; emojis
  monochromes hors du chat. Sur un système sans les polices attendues
  (voir `ui/fonts/fallback.rs`, vérifié sur Windows 11), les carrés
  restent pour les écritures manquantes ; le journal liste les polices
  trouvées (`system fallback fonts: …`).
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
- **`AURORA_DEMO_CONV=1|noms[:etroit|large]`** : `1` ouvre la conversation du
  groupe de démo avec ses participants. `noms` montre des noms plus longs que
  leur place (issue #124) : deux résidents (nom d'affichage long avec nom
  d'utilisateur, lettres larges et accentuées) et une conférence au titre long
  dans la liste de Conversations, avec des messages non lus, et ces résidents
  dans Personnes › Près de vous. Les deux fenêtres sont placées à une taille
  connue, quelle que soit la disposition enregistrée : 600 et 330 px de large,
  400 et 230 avec `:etroit`, 900 et 520 avec `:large`. À voir : chaque nom
  coupé par « … » avant la pastille de non-lu, l'heure du message ou la
  distance, sans rien recouvrir ; les fenêtres gardent la largeur demandée ;
  le nom entier s'affiche au survol d'un nom coupé (pour une capture :
  `AURORA_DEMO_POINTER="85,633,s"` sur la conférence, `"120,377,s"` sur un
  résident proche, fenêtre de 1600 × 900).
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

**`AURORA_DEMO_REZ=1|land|object|contents|nocopy|cancel`** — ouvre l’inventaire
synthétique. `1` permet de tester à la souris ; les autres modes font glisser
une vraie ligne d’inventaire à partir de l’image 680 : terrain, objet opaque,
contenu de l’objet avec Ctrl, exemplaire non copiable ou annulation par Échap.
Le simulateur local affiche un cube synthétique au point de rez (aucun asset
de grille). Capturer après l’image 700. Vérifier que l’exemplaire non copiable
disparaît de l’inventaire, que Ctrl ajoute au contenu sans cube nouveau et
qu’Échap ne crée rien. En ligne, valider les objets réels, le terrain interdit,
la corbeille / bibliothèque, les régions voisines, le groupe du terrain et les
objets déjà portés ; l’humain se connecte et effectue ces essais.

**`AURORA_DEMO_INVENTORY`** — ouvre un inventaire synthétique : dossier
personnel, sous-dossier, tenues, objets, documents, texture, son, geste,
ciel, matériau, lien et permissions variées ; bibliothèque dans l'arbre,
racine ouverte et libellés alignés à gauche. Créations et mutations simulées
hors ligne ; les fichiers et photos chargés en démo restent hors ligne.

- `1` : l'inventaire seul.
- `delete-key`, `delete-button` : sélectionnent un dossier contenant trois
  niveaux et leurs items, puis ouvrent la confirmation par Suppr ou le bouton
  de corbeille du pied de fenêtre. Capturer 660 (sélection et corbeille à droite
  du compteur), puis 720 (confirmation identique au menu contextuel). Supprimer
  déplace le dossier complet dans la corbeille ; Annuler le garde à sa place.
  Vérifier aussi une sélection multiple, un objet porté / protégé (commande
  indisponible), un champ de texte (Suppr édite le texte) et une fenêtre de
  dossier. Une sélection entièrement dans la corbeille demande confirmation
  avant suppression définitive.
- `clipboard` : inventaire déplié pour tester Ctrl+C / Ctrl+X / Ctrl+V sur
  les éléments, les dossiers et les sélections multiples. Choisir le dossier
  de destination (ou un de ses éléments) avant de coller. Les champs de
  recherche et de renommage gardent leurs raccourcis de texte.
  `clipboard-copy` sélectionne deux éléments, copie puis colle deux fois dans
  le sous-dossier ; `clipboard-cut` déplace deux objets, dont un non copiable
  et un non transférable ; `clipboard-folder` copie le sous-dossier, ses deux
  niveaux imbriqués et leurs trois items dans « Destination de test ».
  `clipboard-cut-folder` déplace la même arborescence, avec un objet non copiable
  dans un sous-dossier : tous les éléments doivent rester présents avec les
  mêmes identifiants. `clipboard-window` copie depuis une fenêtre de
  dossier puis colle dans une autre, vide. Gestes aux images 650–781, puis
  ouverture du dossier de destination ; capture après 800. Aucune activation
  de la fenêtre native ni lecture du presse-papiers du système pendant ces
  scénarios automatiques.
  `clipboard-progress` illustre les états possibles de la ligne de statut :
  modification aux images 650–749, copie avec compteur 2/3 aux images
  800–849, vérification aux images 850–919, puis confirmation de fin dès 920.
  Capturer 700,820,890,940 ; l’animation, le compteur et le temps écoulé doivent
  rester visibles dans la ligne de statut, sans agrandir la fenêtre.
  La vérification concerne les mutations de métadonnées ; une copie réelle
  est terminée par la réception de ses items, sans relecture finale du dossier.
  En ligne, le statut doit montrer les étapes, le compteur d’items confirmés et
  le temps écoulé, puis « Modification terminée ». Les items apparaissent dès leur
  arrivée ; il doit être possible de lancer une autre copie ou de déplacer un
  autre item pendant une copie en cours. Un couper/coller change l'arborescence
  dès l'envoi fiable et affiche « Déplacement demandé » sans attendre de relecture
  HTTP. Tester deux copies successives, des dossiers avec plusieurs niveaux,
  puis recharger et vérifier les originaux, les copies et les déplacements.
  Une copie non confirmée demande de recharger la destination ; des références
  incomplètes demandent de recharger la source au lieu d'attendre une expiration.
- `move-menu` : menu d’un élément avec « Déplacer vers… » ; `move-item`,
  `move-folder`, `move-multi` : fenêtre de destination pour un élément, un
  dossier ou trois objets (dont un non copiable et un non transférable).
  `move-filter` : filtre sur le sous-dossier ; `move-window` : même fenêtre
  depuis un inventaire limité à un dossier. Sélectionner un dossier puis
  Déplacer ; Annuler doit laisser la sélection à son emplacement initial.
- `offer`, `offer-reject` : objet offert par un résident, gardé ou rejeté
  (voir `AURORA_DEMO_NOTIF` plus haut).
- `folder`, `object`, `animation`, `script`, `note`, `properties`, `nocopy`,
  `notransfer`, `link` : le menu de l'élément choisi, ou ses propriétés.
  `clothes`, `body`, `settings`, `uploads` sont des alias de `folder`, à
  combiner avec `AURORA_DEMO_POINTER` pour survoler les sous-menus.
- `received`, `received-filtered`, `received-missing` : onglet Achats reçus,
  recherche limitée à sa boîte Marketplace et état sans dossier reçu. La liste
  et les boutons Réduire / Développer gardent la même présentation ; aucun
  dossier personnel ne doit apparaître dans cet onglet.
- `animation-open`, `animation-properties`, `image`, `image-photo`,
  `image-picker` : les cinq panneaux de référence.
- `note-open`, `note-new`, `note-readonly` : éditeur d’une note existante,
  note vide sans asset ou note non modifiable. Modifier le texte puis
  Enregistrer (ou Ctrl+S) ; fermer et rouvrir conserve le texte pendant la
  session de démo. Une fermeture avec modifications propose Enregistrer,
  Ignorer les modifications ou Annuler ; Enregistrer ne ferme qu’après
  confirmation de la sauvegarde.
- Les notes s’ouvrent par défaut en 760 × 600 pixels (adaptés au viewport) ;
  le texte doit remplir la hauteur disponible, avec Enregistrer (Ctrl+S),
  le compteur et l’éventuel message juste en dessous. Vérifier aussi après
  redimensionnement et réduction / restauration de la fenêtre.
- `note-task`, `note-task-readonly`, `note-task-save`, `note-task-error` :
  même éditeur pour une note dans un objet. Les deux derniers ajoutent du
  texte à l’image 650 puis envoient Ctrl+S à l’image 670 ; le refus conserve
  le brouillon. Sauvegarder et fermer / rouvrir conserve le texte pendant
  la session de démo, sans créer d’élément dans l’inventaire personnel.
- `note-edit`, `note-save`, `note-close`, `note-error`, `note-embedded` :
  ajout automatique de texte à l’image 650, puis sauvegarde par Ctrl+S
  (`note-save`), confirmation de fermeture (`note-close`) ou refus simulé
  (`note-error`) à l’image 670. `note-embedded` contient une image, un repère
  et un objet : leurs icônes et noms sont affichés dans le texte ; les pièces jointes
  encore référencées sont conservées lors de l’enregistrement. Capture après
  l’image 720 ; le refus doit garder le brouillon et permettre une nouvelle
  tentative. Un clic gauche ouvre directement l’élément ; le clic droit
  propose Ouvrir et Copier dans l’inventaire. Une image affiche son aperçu,
  un repère ouvre son lieu, un son joue localement ; les autres types proposent
  une copie dans l’inventaire.
  Glisser un élément de l’inventaire dans le texte l’insère à cet endroit
  si le prochain propriétaire dispose de copie / modification / transfert.
- `note-drop` : dépose une texture à l’image 651 puis sauvegarde à l’image 670.
  `note-menu-right` : ouvre le menu sur la texture par clic droit à l’image 651.
  `note-image`, `note-landmark`, `note-copy` : ouvre respectivement
  l’image, le repère ou l’objet par un seul clic gauche à l’image 651 ; `note-copy`
  confirme la copie à l’image 671. `note-task-embedded` permet
  de tester les mêmes pièces jointes dans une note d’objet. Capturer après
  l’image 740. Tout élément ajouté doit être enregistré avant son ouverture.
  Les repères sont récupérés dans l’inventaire avant l’ouverture de leur lieu,
  comme dans Firestorm ; un repère déjà présent pour le même asset est réutilisé.
- `image-photo-save` : capture puis sauvegarde une vignette à l'image 2100
  et ferme la fenêtre Photo en gardant l'éditeur Image ouvert.
- `folder-window` : une fenêtre limitée au contenu du dossier personnel,
  avec son nom en titre et un filtre propre ; `folder-window-search` montre
  cette recherche limitée aux descendants.
  Vérifier les indicateurs compacts C·M·T : droits accordés verts, droits
  refusés rouges et barrés (objet non copiable, objet non transférable), détail du
  propriétaire actuel et du prochain propriétaire au survol. Un lien
  reprend les permissions de son original.
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

Les indicateurs sont également visibles dans le contenu d'un objet :
`AURORA_DEMO_ACTIONS=open` pour la fenêtre Ouvrir, ou
`AURORA_DEMO_BUILD="move,object:100"` avec
`AURORA_DEMO_BUILD_TAB=contents` pour l'onglet de construction. Les contenus
de démo proposent des droits complets, sans modification ou sans copie ;
le prochain propriétaire n'a que le transfert. Vérifier aussi le double-clic
sur une note et la recherche « pas de copie » dans l'onglet Contenu. Survoler et
sélectionner un item : même ligne, mêmes icônes et permissions après le nom que
dans l'inventaire utilisateur ; le menu contextuel et l'ouverture des documents
restent disponibles. Les noms longs défilent horizontalement dans la liste.

Pour valider les notecards sur la grille, ouvrir une note existante puis une
note neuve, modifier leur texte, enregistrer et fermer / rouvrir. Refaire la
lecture après déconnexion / reconnexion pour vérifier la persistance. Tester
aussi une note avec pièce jointe : celle-ci doit rester présente après édition
du texte. La relecture utilise le transfert d’inventaire du simulateur, comme
Firestorm ; les notes et scripts protégés ne sont pas lus via ViewerAsset.
Dans un objet modifiable, onglet Contenu de la construction (ou fenêtre
Ouvrir de l’objet), double-cliquer sur une note puis utiliser Enregistrer
ou Ctrl+S. Le clic droit › Ouvrir doit donner le même résultat ; Renommer
reste une action distincte du menu. Fermer / rouvrir, puis recommencer
après reconnexion. Avec une note ou un objet non modifiable, le texte doit
être consultable quand la copie est permise et rester en lecture seule.

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
| `AURORA_DEMO_VOICE` | États vocaux et couleurs des deux étiquettes d'avatars |
| `AURORA_DEMO_TAGS=voice`, `status` ou `cycle` | Plaques de nom dans tous leurs états, aux trois distances |

### Détails

- **`AURORA_DEMO_VOICE`** : `speaking` anime le contour des étiquettes de
  soi et de l'autre avatar avec des niveaux synthétiques réalistes (0,35 à
  0,61 sur l'échelle WebRTC), `loud` maintient un niveau fort de 0,60 : les
  deux contours doivent être rouges sans nécessiter un signal saturé.
  `listening` garde les deux étiquettes neutres, `off` montre le micro barré rouge. `cycle`
  enchaîne 4 s de parole, 3 s d'écoute, 2 s hors vocal. Sans cette variable,
  notre contour suit le vrai micro quand son bouton / push-to-talk est actif
  et la voix activée. Aucun réglage enregistré n'est changé. Avec les
  plaques de nom (style par défaut), le contour est vert dans le canal vocal
  et un halo vert serré suit le niveau ; hors vocal, la plaque n'a ni contour
  coloré ni badge.
- **`AURORA_DEMO_TAGS`** : neuf résidents devant la position de départ
  (`demo_tags.rs`), pour voir les plaques de nom sans grille. Cinq sont à
  moins de 15 m de la caméra (plaque complète), deux vers 27 m (pastille
  compacte), deux vers 50 m (nom seul). Un mur blanc se dresse derrière
  les deux de gauche, pour juger le verre sur un fond clair à côté des
  mêmes plaques sur la scène sombre. La fenêtre Personnes est fermée.
  - `voice` : Nova hors du canal vocal ; Tess dans le canal, silencieuse
    (contour vert, badge micro) et sans photo de profil (l'icône reste) ;
    Orion, un ami, parle (halo qui suit le niveau, nom violet clair, badges
    micro, cœur et triangle : il est montré comme trop complexe) ; Lune
    écrit (trois points animés) et n'a pas de nom d'affichage (deux lignes) ;
    La Brume est bloquée (plaque atténuée, nom en italique, badge interdit,
    pas de pastille de statut). Plus loin, Sol parle et Iris écrit ; au-delà,
    Vega est dans le canal (contour vert) et Atlas non.
  - `status` : Nova absente (pastille ambre), Tess ne pas déranger (pastille
    rouge), Orion hors du canal (cœur et triangle seuls), Lune écrit en
    étant absente.
  - `cycle` : passe de l'un à l'autre toutes les six secondes.
  - Les photos sont des dégradés générés (`demo_tags::picture_pixels`),
    demandées comme sur la grille : profil, puis texture, au plus quatre par
    seconde, la plus proche d'abord ; notre avatar et La Brume ont la même.
  - À combiner avec `AURORA_DEMO_VOICE` pour notre propre plaque et celle de
    Loup Violet (bloqué dans la démo), et avec `AURORA_NAME_TAGS=classic`
    pour comparer avec les étiquettes classiques.
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
| `AURORA_MSAA_PREPASS=0` ou `alt` | MSAA sans la prépasse de sa profondeur, ou les deux en alternance |
| `AURORA_NOVSYNC=1` | Coupe la synchronisation verticale |
| `AURORA_PRESENT_MODE=<mode>` | Force le mode de présentation de la swapchain |
| `AURORA_CPU_CULL=1` | Listes de dessin construites sur le CPU |
| `AURORA_CPU_ATTACH=1` | Objets portés et mesh riggés placés par la synchro, sans les ancres du GPU |
| `AURORA_SYNC_SLICE=1` ou `alt` | Synchro de la scène par tranche de tous les objets, comme avant |
| `AURORA_SYNC_CHECK=1` | Ce que la synchro de la scène visite à chaque image |
| `AURORA_NAME_TAGS=classic`, `plate` ou `alt` | Style des étiquettes d'avatars de ce lancement |
| `AURORA_TEXTURE_BC7=0` | Textures jamais compressées en BC7 |
| `AURORA_TEXTURE_BC7_CACHE=0` | BC7 sans son cache disque |
| `AURORA_RENDER_THREAD=0` | Images dessinées sur le thread principal |
| `AURORA_EYE_RELATIVE=0` | Sommets des objets et du terrain calculés en coordonnées de région |
| `AURORA_SHADOW_LEGACY=1` | Ombres dessinées par l'ancien chemin |
| `AURORA_PROFILE=1` | Profilage dans le journal (voir [Profilage](#profilage)) |
| `AURORA_PROFILE_FRAMES=1` | Profil détaillé de chaque image |
| `AURORA_GPU_VALIDATION=1` | Couches de validation de wgpu |
| `AURORA_HOVER_CHECK=1` | Diagnostic du curseur de survol |
| `AURORA_CLICK_PICK=wait` ou `check` | Profondeur sous un clic lue en attendant le GPU, ou contrôlée |
| `AURORA_HOVER_LEGACY=1` ou `ab` | Recherche de l'objet sous le curseur comme avant les classes et les hiérarchies de triangles |
| `AURORA_STILL_CHECK=1` | Ce qui change quand l'image devrait être immobile |
| `AURORA_DEBUG_GLOW=1` | Diagnostic du glow |
| `AURORA_GLOW_SKIP=<masque>` | Retire une famille de faces du glow |
| `AURORA_MEDIA_DEBUG=1` | Diagnostic des médias |
| `AURORA_FONT_FALLBACK=0` | Sans les polices de repli du système |
| `AURORA_TEXTURE_BUDGET_MB=<n>` | Force le budget mémoire des textures |
| `AURORA_LIGHT_TILES=0`, `alt` ou `check` | Lumières locales sans tri par case d'écran, ou contrôle du tri |
| `AURORA_BLEND_EARLY_DISCARD=0` ou `alt` | Fragments invisibles des faces transparentes éclairés avant d'être abandonnés |

### Détails

- **`AURORA_NAME_TAGS`** : `classic` ou `plate` impose le style des
  étiquettes d'avatars (classiques ou plaques de nom) sans toucher au
  réglage « Style des étiquettes ». `alt` alterne dans la même session, à
  chaque ligne `perf summary` : une seconde avec les étiquettes classiques
  (`xmode=old`), deux avec les plaques (`xmode=new`). C'est la mesure à faire
  sur une machine partagée : mêmes avatars, même caméra, même charge
  extérieure des deux côtés. À lire : `u_tags` (thread principal),
  `r_tessellate`, `r_egui`, `ui_meshes` et `ui_vertices`. Scène de mesure :
  `AURORA_DEMO_CROWD=40` avec `AURORA_DEMO_POS="135,134"` (dans la foule :
  les huit plaques complètes, le reste en pastilles compactes) ou
  `"129,134"` (devant la foule), `AURORA_FPS_LIMIT=120`. Caméra immobile, une plaque qui n'a
  pas bougé est redessinée sans copie : le coût en mouvement se voit en
  tournant la caméra pendant le relevé.
- **`AURORA_TEXTURE_BUDGET_MB=<n>`** : les pages de textures ne doivent pas
  dépasser n Mo, quels que soient le réglage « Mémoire textures » et la
  mémoire vidéo que le pilote annonce (`scene/texture_budget.rs`). Sert à
  mettre le viewer sous pression mémoire sur n'importe quelle machine, en
  démo comme sur la grille.
  - Au-dessus de la cible, le journal annonce chaque changement de palier
    (`texture memory: pressure 0 -> 1 (… MB in pages for a target of … MB, …
    lowered so far)`) : 1 ramène les textures à leur taille à l'écran, 2 à 7
    retirent ensuite un demi-niveau à la fois, jusqu'à deux niveaux, hors
    champ d'abord. La même ligne donne la mémoire vidéo du viewer : ses
    ressources, ce que le pilote lui compte et le budget accordé. La fenêtre
    Performances complète affiche la cible et le palier (« Budget des
    textures », dans Mémoire et flux).
  - En démo, avec `AURORA_DEMO_STREAM=3000` (4,7 Go de textures) :
    `AURORA_DEMO_POS="168,126"` place l'avatar au milieu des objets ;
    512 reste au palier 1, 96 monte jusqu'au palier 4 ou 5 selon l'essai.
    Les textures de ce
    scénario se rechargent alors à la demande, comme celles de la grille
    (caméra rapprochée après une réduction). Une valeur énorme (1000000)
    retire toute pression, pour comparer.

- **`AURORA_LIGHT_TILES=0|alt|check`** : par défaut, un fragment éclairé ne
  parcourt que les lumières locales qui couvrent à la fois sa colonne
  d'écran, sa rangée et sa tranche de profondeur (32 de chaque ; ensembles
  calculés à chaque image sur le thread de rendu,
  `aurora-render/src/light_tiles.rs`), dans l'ordre de la liste : même
  image, les lumières qui atteignent le fragment sont additionnées comme
  avant et dans le même ordre.
  - `0` : chaque fragment parcourt toute la liste (jusqu'à 64 lumières),
    comme avant, pour comparer.
  - `alt` (avec `AURORA_PROFILE=1`) : l'ancien parcours pendant une ligne
    `perf summary` sur trois, le nouveau pendant les deux autres, dans la
    même session et sur la même scène ; `light_tiles=1.00` ou `0.00` dans
    la ligne dit lequel (une valeur intermédiaire : la seconde du
    changement). Comparer `g_objects`, `g_terrain_sky` et
    `g_transparent` + `g_post` des deux groupes de lignes, avec
    `AURORA_FPS_LIMIT=120` ; `lights=` donne la longueur de la liste.
  - `check` : les shaders peignent en magenta tout fragment qu'une lumière
    de la liste atteint sans figurer dans ses ensembles. Il ne doit y en
    avoir aucun, où que l'on regarde (coûteux : chaque fragment reparcourt
    toute la liste).
  - Les reflets, les faces de sondes et les photos d'imposteurs gardent
    toute la liste.
  - En démo : `AURORA_DEMO_LIGHTS=64` avec `AURORA_DEMO_CROWD=40`.
- **`AURORA_BLEND_EARLY_DISCARD=0|alt`** : par défaut, un fragment de face
  transparente dont l'alpha est sous 0,004 est abandonné avant l'éclairage
  (ombres, lumières, brume, sondes) ; il l'était après. `alphaF.glsl` de
  Firestorm l'abandonne de même avant la brume, les sondes et les lumières.
  Même image. `0` : l'ancien ordre, pour comparer ; `alt` :
  l'ancien ordre pendant une ligne `perf summary` sur trois (`blend_early=`
  dans la ligne), comme `AURORA_LIGHT_TILES=alt` — n'alterner qu'une des
  deux variables à la fois, elles changent aux mêmes secondes. Le gain
  dépend du contenu : voir `AURORA_DEMO_HAIR`.
- **`AURORA_FONT_FALLBACK=0`** : n'utilise que les polices embarquées, comme
  avant l'ajout des polices de repli du système : les caractères qu'elles
  n'ont pas (géorgien, CJK, arabe, lettres mathématiques…) redeviennent des
  carrés. Pour comparer avec `AURORA_DEMO_GLYPHS=1` ou isoler un problème
  de police. Le journal indique `system fallback fonts: off`.
- **`AURORA_PRESENT_MODE=immediate|mailbox|fifo|fifo-relaxed`** : remplace le
  mode de présentation choisi d'après la synchronisation verticale
  (`aurora-render/src/present.rs` : sans synchronisation `Mailbox`, sinon
  `Immediate` ; avec, `FifoRelaxed`, sinon `Fifo`), si la surface le
  propose ; une valeur inconnue est signalée dans le journal et ignorée.
  Sert à comparer les images/s d'un écran à l'autre (écrans de fréquences
  différentes, fréquence variable G-Sync / FreeSync) : `immediate` ne se
  synchronise jamais sur l'écran (déchirures possibles), `fifo` s'aligne sur
  sa fréquence. Le journal donne le mode retenu et ceux de la surface
  (`presentation: Mailbox (vsync off, surface supports […])`), puis l'écran
  sous la fenêtre et sa fréquence à chaque changement
  (`screen under the window: \\.\DISPLAY2, 144 Hz`) ; `present=` et
  `screen_hz=` figurent dans la ligne `perf settings`.
- **`AURORA_TEXTURE_BC7_CACHE=0`** : les textures compressées en BC7 ne
  sont ni écrites dans le cache disque (`cache\tex\<id>.bc7`) ni relues :
  chaque texture est décodée depuis son JPEG2000 comme à une première
  visite, pour mesurer ce que le cache épargne (`read as BC7 without a
  decode` et le temps processeur dans la ligne `texture load:`).
- **`AURORA_TEXTURE_BC7=0`** : toutes les textures restent en RGBA8 en
  mémoire vidéo, comme avant la compression BC7, pour comparer l'image,
  la mémoire (`texture_mb`, `pages_mb` de `perf summary`) et le temps de
  chargement (`bc7` dans la ligne `texture load:`). Par défaut, quand la
  carte graphique a la compression BC, chaque tâche de décodage encode la
  texture en BC7 et vérifie le résultat ; celles qu'il abîmerait restent
  en RGBA8. Le journal l'annonce au démarrage (`textures: BC7 compression
  on|off (GPU support: …)`).
- **`AURORA_CPU_CULL=1`** : construit toutes les listes de dessin sur le CPU
  (le repli quand le GPU n'a pas `MULTI_DRAW_INDIRECT_COUNT`) au lieu de les
  trier sur le GPU, pour comparer ; `cull=cpu|gpu` dans la ligne
  `perf summary`.
- **`AURORA_CPU_ATTACH=1`** : replace à chaque image, sur le thread
  principal, tout ce que portent les avatars et tous les mesh riggés,
  comme avant les ancres (`scene/anchors.rs`), pour comparer l'image et les
  mesures. Par défaut, un objet porté sur le corps est placé une fois par
  rapport à son point d'attache, un mesh riggé (porté, bento, animesh) et
  le corps d'un avatar par rapport à leur squelette, et le GPU fait le
  reste, sphères de culling des mesh riggés comprises ; les HUDs, les prims
  flexibles et les objets qui tournent d'eux-mêmes restent à la synchro. À
  lire dans `perf summary` : `anchored` (objets placés par le GPU, 0 avec
  la variable), `synced`, `sync` et ses parties `s_sync_*`, `records_kb`,
  `journal_kb`. À combiner avec `AURORA_DEMO_CROWD=<n>` et
  `AURORA_DEMO_ANIMESH=1`. Seul écart attendu sur les draws : les faces
  transparentes des mesh riggés, que le CPU liste, sont testées avec
  l'étendue de leur squelette et non plus avec leurs propres limites
  (`blend` un peu plus élevé).
- **`AURORA_SYNC_SLICE=1`** : la synchro de la scène revient à chaque objet
  toutes les 24 images (tranche de LOD), replace les HUDs portés à chaque
  image et envoie les enregistrements par blocs entiers de 64, comme avant,
  pour comparer l'image et les mesures. Par défaut, un objet immobile n'est
  revu que lorsque la caméra a parcouru la distance qui peut changer son
  niveau de détail (`scene/lod_watch.rs` : jamais caméra immobile) ; ce que
  le GPU place depuis un avatar (objets portés, mesh riggés) est revu à
  tour de rôle, toutes les 24 images tant que la caméra bouge (comme
  avant) et toutes les 0,4 s caméra immobile, et tout objet toutes les
  10 s ; un HUD
  porté n'est replacé qu'à ses mises à jour et quand sa place à l'écran
  change (fenêtre redimensionnée, zoom ou décalage local) ; seuls les
  enregistrements modifiés sont envoyés.
  - `AURORA_SYNC_SLICE=alt` alterne dans la même session, à chaque ligne
    `perf summary` : une seconde par l'ancien chemin, deux par le nouveau,
    la ligne se terminant par `xmode=old` ou `xmode=new`. C'est la mesure à
    faire sur une machine partagée ou sur la grille : mêmes objets, même
    caméra, même charge extérieure des deux côtés.
  - À lire : `synced`, `sync` et ses parties `s_sync_*`, `records_kb`,
    `cull_kb`, `journal_kb`, `r_resources`. À combiner avec
    `AURORA_DEMO_CROWD=40,busy`, `AURORA_DEMO_ANIMESH=1` et
    `AURORA_DEMO_STREAM=12000,400`.
- **`AURORA_SYNC_CHECK=1`** : une ligne `sync check` par seconde, en
  moyennes par image : les objets listés par la synchro selon ce qui les y
  a mis (`moving` : ce qui bouge de lui-même et ce qui le suit ; `changed` :
  mises à jour ; `geometry` : géométrie arrivée ; `lod` : contrôles du
  niveau de détail), puis par nature d'objet (`avatar`, `hud`, `flexible`,
  `moving` — vitesse ou rotation donnée par le simulateur —, `rigged` — mesh
  riggé que le GPU ne place pas —, `anchored` — placé par le GPU —, `worn`
  — porté et placé par la synchro —, `follower` — lié sous un objet qui
  bouge —, `still`) ce qu'il en est sorti : placement vérifié sans rien
  écrire / enregistrements réécrits / synchro complète / synchro complète
  remise à plus tard (`scene/sync_check.rs`). Dit quels objets occupent la
  synchro dans un lieu au repos. La ligne se termine par un contrôle de
  tous les objets, fait une fois par seconde : `detail level behind: a of b
  at rest, c of d placed by the GPU` compte ceux dont le niveau de détail
  demandé n'est pas, à cet instant, celui que donne leur distance (un
  contrôle que la synchro n'a pas encore fait) ; à lire caméra en
  mouvement, avec et sans `AURORA_SYNC_SLICE=1`. Ce contrôle coûte un
  parcours de tous les objets par seconde. En démo comme sur la grille.
- **`AURORA_RENDER_THREAD=0`** : dessine les images sur le thread principal,
  comme avant le thread de rendu, pour comparer et déboguer. Par défaut, le
  thread principal remet chaque image au thread de rendu sous la forme d'un
  paquet autonome et passe à l'image suivante ; avec `0`, le même paquet est
  exécuté sur place. `thread=on|off` dans la ligne `perf summary`, et une
  ligne de journal au démarrage (`renderer: frames drawn by…`).
- **`AURORA_MSAA_PREPASS=0|alt`** : avec le MSAA, la passe de scène a sa
  propre profondeur multi-échantillonnée et ne peut pas reprendre celle de
  la prépasse à un échantillon. Par défaut, une seconde prépasse remplit
  cette profondeur juste avant (terrain et faces opaques, les dessins mêmes
  de la passe de scène), et la passe de scène ne calcule plus la couleur des
  fragments cachés. Les faces masquées par alpha n'y sont pas : avec le
  MSAA leur bord vient de l'`alpha to coverage` de la passe de scène, qu'une
  passe de profondeur ne peut pas reproduire échantillon par échantillon.
  - `0` : la passe de scène repart d'une profondeur vide, comme avant, pour
    comparer l'image et les temps (`g_objects`, `g_terrain_sky`,
    `g_depth_msaa` de `perf summary`).
  - `alt` : une seconde sans la prépasse, deux secondes avec, tout au long
    de la session ; à chaque bascule le journal donne la médiane des temps
    GPU des images de la variante qui se termine
    (`msaa prepass alt: prepass=on|off frames=… gpu=… shadows_depth=… …
    objects=… depth=… depth_msaa=…`, en ms). C'est la mesure à utiliser sur
    une carte partagée ou sur la grille : les deux variantes voient la même
    scène et la même charge, et la médiane écarte le tour d'un autre
    processus sur la carte. Comparer les médianes des lignes `on` et `off`,
    de préférence avec `AURORA_FPS_LIMIT=120`.
  - Sans effet hors MSAA (`AURORA_AA` 0, 1, 5, 6) : avec `alt`, les lignes
    `on` et `off` mesurent alors deux fois le même chemin, ce qui donne
    l'écart entre deux mesures égales.
  - À voir avec `AURORA_DEMO_OVERDRAW=12` (gain net) et avec la démo
    seule (peu de recouvrement : la prépasse coûte à peu près ce qu'elle
    épargne).
- **`AURORA_EYE_RELATIVE=0`** : calcule les sommets des objets et du terrain en coordonnées
  de région, comme avant, pour comparer. Par défaut, le vertex shader retire
  la position de la caméra de la translation du modèle avant tout autre
  calcul (`eye_model` dans `object.wgsl`, matrice `eye_view_proj`) : en
  altitude, la simple précision ne tient que 0,24 mm en coordonnées de
  région (entre 2 048 et 4 096 m), et chaque sommet des objets proches
  tombait jusqu'à un pixel à côté, ailleurs à chaque changement de vue. À
  voir avec `AURORA_DEMO_ALTITUDE=2085` et
  `AURORA_DEMO_CAMERA=drift,690,420,84` sur des captures consécutives
  (images 800 à 899) : le bord du panneau dérive régulièrement, ou sautille
  avec `0`.
- **`AURORA_SHADOW_LEGACY=1|splits|ab`** : pour comparer l'image et les
  mesures des ombres. Par défaut, chaque cascade ne dessine que les objets
  dont l'ombre peut tomber sur ce que la vue y lit
  (`aurora-render/src/shadow_cull.rs`), les faces opaques sont dessinées
  sans fragment shader, et la cascade d'un pixel est choisie d'après sa
  propre distance à la caméra.
  - `1` : l'ancien chemin. Tous les objets du carré de chaque cascade,
    toutes les faces avec le test alpha, et la cascade choisie d'après la
    distance interpolée entre les sommets du triangle.
  - `splits` : seulement l'ancien choix de la cascade. L'image est alors
    celle de l'ancien chemin au pixel près ; les objets proches de la
    caméra restent dessinés dans les cascades lointaines.
  - `ab` : avec `AURORA_PROFILE=1`, l'ancien chemin, le nouveau et `splits`
    à tour de rôle, une seconde chacun, dans la même session et sous la
    même charge extérieure. `shadow_path=legacy|new|splits` dans la ligne
    `perf summary` dit lequel a dessiné la seconde ; comparer
    `shadow_draws`, `g_shadows_depth` et `gpu`, avec `AURORA_FPS_LIMIT=120`
    pour les temps GPU.
  - Seule différence d'image attendue avec `1` : sur un triangle qui couvre
    plusieurs cascades (grand sol, grand mur), la limite entre deux
    cascades se déplace, et les ombres proches de la caméra y sont plus
    nettes (elles lisaient une cascade plus lointaine que le terrain et les
    petits objets voisins).
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
  secondes les recherches faites et les réponses réutilisées. Chaque
  recherche vérifie aussi l'index lui-même contre la scène : les sphères
  qu'il tient (`index entries … unlike their object`) et les classes des
  objets testés quel que soit le rayon (`classes unlike their object`) ;
  tous ces compteurs doivent rester à 0. Coûte la recherche complète à
  chaque image.
- **`AURORA_HOVER_LEGACY=1`** : la recherche de l'objet sous le curseur
  redevient celle d'avant (`scene/hover.rs`) : tout objet testé quel que
  soit le rayon (avatar, mesh riggé) est candidat à chaque étape, chaque
  triangle d'un objet rencontré est testé, et le pick des HUDs regarde tous
  les objets de la scène. `ab` alterne les deux façons,
  une seconde chacune dans la même session : c'est la comparaison à faire
  sur une machine chargée. À lire dans les lignes `hover summary` du profil
  (`search=new|old`, voir [Profilage](#profilage)).
- **`AURORA_CLICK_PICK=wait|check`** : par défaut, un clic sur le monde
  (clic gauche, Alt + clic, clic droit, outils de construction) demande la
  profondeur sous son pixel sans l'attendre : la demande part avec l'image
  suivante, la réponse revient une à trois images plus tard (une ou deux
  avec un plafond d'images/s, trois quand le GPU est la limite), et le clic
  est alors traité avec le curseur et les touches qu'il avait ; ce qui le
  suit (relâchement, autres boutons, mouvement de la souris qui déplace la
  caméra) attend derrière lui, dans l'ordre (`app/click_pick.rs`).
  - `wait` : la profondeur est lue tout de suite en attendant le GPU, comme
    avant, pour comparer (sans la lecture des faces dessinées : un mesh
    riggé n'est alors pas désigné, comme avant aussi) : dans une scène où le GPU est la limite, le
    thread principal attend alors 16 à 30 ms à chaque clic (`between` dans
    `max:` et `off_cpu:` de `perf summary`).
  - `check` : les deux lectures sont faites et comparées, une ligne `click
    check` par clic : `depth identical` (c'est le même texel de la même
    image : `DIFFERENT` est un défaut), `target identical` (le point et
    l'objet que le clic trouve ; `DIFFERENT` seulement si ce qui est sous le
    curseur a changé pendant les images d'attente), le temps
    de la lecture qui attend et le délai de la réponse. Coûte l'attente de
    `wait` à chaque clic.
  - Avec `AURORA_PROFILE=1` (ou `AURORA_DEMO_CLICKS`), chaque clic écrit
    une ligne `click pick:` : `answered after 2 frames, 9.14 ms`, ou `the
    read waited 25.56 ms` avec `wait`. En démo comme sur la grille.

- **`AURORA_STILL_CHECK=1`** : une ligne `still check` par seconde, pour
  trouver la cause d'une image qui tremble alors que rien ne devrait bouger
  (caméra verrouillée sur un objet). Elle donne le nombre d'images où la
  caméra, son point visé, son champ et notre avatar ont changé (et de combien
  au plus, en mm), les objets dont les faces ont été reconstruites, avec la
  raison (`geometry` : autre niveau de détail ; `material`, `shape` : mise à
  jour du simulateur ; `faces`, `rigged`, `skeleton`, `material-generation`),
  et les objets posés dont la matrice a changé (`worn` compte à part ceux que
  porte un avatar). Six exemples au plus suivent chaque ligne : identifiant,
  type, niveau de détail, taille, distance, mouvement annoncé par le
  simulateur (`scene/still_check.rs`). En démo comme sur la grille.

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
  renderer (`r_*`), parties du streaming et de la synchro (`s_*`), des
  paramètres de l'image (`p_*`) et de l'interface (`u_*`) et champs
  `thread` compris, ou `-` s'il n'y en a aucune. Une image lente périodique
  y apparaît avec l'étape qui l'a causée, par exemple
  `max: media=6.10 render=2.31`, alors que sa moyenne reste minuscule.
- `off_cpu:` : sur le même principe, les étapes des deux threads qui ont
  passé au moins 1 ms **hors du processeur** dans une image (temps écoulé
  moins temps processeur du thread, Windows seulement), la plus longue
  d'abord. Une étape lente qui y figure a attendu au lieu de travailler : un
  verrou (le tas pendant que les tâches de fond allouent, par exemple), ou
  un cœur pris par un autre thread quand la machine est chargée. Les étapes
  qui confient du travail au pool (`sync`, `lists`, `poses`, `r_finish`) y
  attendent leurs threads par construction ; celles qui attendent par
  nature (`render`, `limiter`, `r_acquire`, `r_present`) n'y sont pas
  listées.
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
  renderer (`r_*` : `tessellate` — les formes d'egui découpées en
  triangles —, `resources` — la relecture du journal d'écritures —,
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
    budget de temps de l'image, `sync_anchors` ancres des squelettes et des
    points d'attache (écriture, envoi avec les palettes posées, centre des
    objets ancrés), `sync_alpha` faces reclassées après le changement de
    classe alpha d'une texture ;
  - les parties des paramètres de l'image (`p_*`, dans `params`) : `screen`
    passage du chargement au monde et écran de téléportation, `env` ciel et
    eau de l'image et leurs textures, `mirror` recherche du miroir visible
    le plus proche, puis les particules — `part_sources` systèmes posés ou
    arrêtés par les mises à jour d'objets, `part_sim` simulation de chaque
    source, `part_list` instances de l'image triées de la plus lointaine à
    la plus proche (`particles` en donne le nombre) —, `lights` lumières
    locales les plus proches et `probes` sondes de reflets (recherche des
    candidates par tranches, emplacements, face à capturer) ;
  - les parties de l'interface (`u_*`, dans `ui`) : `bars` barres du haut et
    du bas, `tags` étiquettes des avatars et textes flottants, `overlays`
    balise, textes des HUDs, outils de construction, débogage et bulles du
    chat, `chat` conversations et contacts, `people`, `inventory`
    (inventaire, apparence, AO), `menus` (menu contextuel, confirmations,
    médias, fenêtres d'objets, notes, scripts, notifications), `places`
    (terrain, profils, lieux), `maps` (mini-carte et carte du monde),
    `perf`, `options` (environnement et préférences), `rest` (écran de
    téléportation, rapport de bug, fermeture ; tout l'écran hors du monde)
    et `pass`, ce qu'egui ajoute autour des fenêtres. Une fenêtre fermée y
    compte ce que son code fait quand même. Puis `ui_shapes`, `ui_meshes`
    (un dessin par mesh) et `ui_vertices` par image, et `ui_vertices_max`,
    le plus grand nombre de sommets d'une image de la seconde : un pic de
    `r_tessellate` sans pic de sommets n'est pas du travail en plus ;
  - le temps GPU par élément (`g_*`). Les marques posées à l'intérieur
    d'une passe n'attendent pas la fin du travail des dessins qui les
    précèdent : quand les faces transparentes sont lourdes, une partie de
    leur éclairage est comptée dans `g_post` (mesuré en démo : une part
    qui change d'une seconde à l'autre). Pour elles, lire la somme
    `g_transparent` + `g_post`. `g_transparent` couvre les faces
    transparentes triées, leur glow intercalé, les calques de débogage et de
    sélection, les particules et la fin de la passe (en MSAA, la résolution
    de l'image) ; l'eau, le glow des faces opaques et les imposteurs n'en
    font pas partie ;
  - ce que les faces transparentes dessinent : `blend` (faces),
    `blend_tris_k` (leurs triangles, en milliers), `blend_glow`,
    `blend_early` (part des images où leurs fragments invisibles sont
    abandonnés avant l'éclairage) ; ce que chaque fragment éclairé peut
    avoir à parcourir : `lights` (lumières locales de l'image, 64 au plus),
    `light_tiles` (part des images où il ne parcourt que celles de sa case
    d'écran et de sa profondeur) et `probes` (sondes de reflets) ;
  - le détail de `g_shadows_depth` : `g_cull` (culling GPU de toutes les
    vues), `g_shadow0` à `g_shadow2` (les cascades d'ombres, de la plus
    proche à la plus lointaine) et `g_depth` (prépasse de profondeur et
    occlusion). Ce partage est indicatif : le GPU chevauche les dessins
    voisins, et le travail des sommets d'une cascade déborde sur ce qui la
    suit ; pour comparer deux chemins, lire leur somme `g_shadows_depth`.
    Puis, par cascade (suffixe 0 à 2), ce que les ombres
    dessinent : `sd` draws, `sd_cpu` ceux des listes du CPU (le terrain),
    `st_k` milliers de triangles, `sd_rig` et `st_rig_k` la part des
    avatars et des mesh riggés (les triangles ne comptent que les draws des
    listes du GPU). `shadow_path` : le chemin des ombres de la seconde
    (`AURORA_SHADOW_LEGACY`) ;
  - `g_depth_msaa` : avec le MSAA, la prépasse de la profondeur
    multi-échantillonnée dont part la passe de scène (0 hors MSAA),
    comptée elle aussi dans `g_shadows_depth` ;
  - les draws et commandes de dessin, les
    objets synchronisés / reconstruits (`synced`, `rebuilt` : des moyennes
    **par image** ; `rebuilt=0.2` à 175 images/s, ce sont 35 reconstructions
    par seconde, `AURORA_STILL_CHECK=1` dit lesquelles), les objets portés et
    mesh riggés placés par le GPU (`anchored`, que la synchro ne visite
    plus), les avatars posés, les octets
    d'enregistrements et de palettes envoyés au GPU, ceux des tables du
    culling (`cull_kb` : faces, objets, boîtes par os) et des sommets
    déformés sur place (`vertices_kb` : prims flexibles), et ceux de tout le
    journal d'écritures de l'image (`journal_kb`), la mémoire des textures
    (textures vivantes, pages de textures et leur mémoire allouée), la
    mémoire de la géométrie, et les objets dont la synchro complète attend
    une image qui a du temps (`sync_backlog` : nouveaux objets en attente
    de leur première synchro compris) ;
  - `xmode=old|new` en fin de ligne avec `AURORA_SYNC_SLICE=alt` : le chemin
    de la synchro pendant cette seconde.

### Objet sous le curseur

Avec `AURORA_PROFILE=1`, une ligne `hover summary` par seconde tant que le
curseur est sur le monde (aucune quand il est sur l'interface, en vue
subjective ou qu'Alt, Ctrl ou Maj est enfoncé) : ce que la recherche de
l'objet sous le curseur a fait (`scene/hover.rs`), à part de l'étape
`hover` de `perf summary`, qui compte aussi les HUDs et la pose du curseur.

- `search=new|old` : la façon de chercher de cette seconde
  (`AURORA_HOVER_LEGACY`) ;
- `hud_ms_per_frame` : le temps du pick des HUDs, fait à chaque image
  avant de chercher dans le monde ;
- `asked` : les images qui ont demandé l'objet, `searches` : celles où il a
  fallu le chercher (les autres ont gardé la réponse), `ms_per_frame` : le
  temps de recherche ramené à chaque image, `longest_ms` : la recherche la
  plus longue ;
- `why:` pourquoi la réponse gardée ne tenait plus (une recherche peut
  compter dans plusieurs) : `none` aucune réponse, `ray` le curseur ou la
  caméra a bougé, `depth` la profondeur lue sous le curseur a changé,
  `index` un objet a bougé ou changé près du rayon, `refresh` la réponse
  avait 100 ms ;
- `index:` ce qui a dérangé l'index en premier : `bounded` un objet avec
  ses limites, `always` un objet testé quel que soit le rayon (avatar, mesh
  riggé), `changed` un objet mis à jour sans bouger, `reset` tout a été
  retiré ;
- `per search:` par recherche, les candidats de chaque étape — `ignore`
  (objets IGNORE qui arrêtent le rayon), `see_through` (faces transparentes
  avec une action), `point` (objet au point trouvé) —, le temps de la passe
  sur l'index (`index_ms`, classes à établir comprises), de la recherche du
  point (`point_ms`) et de l'objet à ce point (`object_ms`), puis les tests
  exacts : `tested` objets dont on a demandé les triangles, `boxes` ceux
  dont le rayon rencontre la boîte, `triangles` triangles testés un par un,
  `hierarchies` faces passées par leur hiérarchie à la place ;
- `ignored` : les recherches qui ont rencontré un objet IGNORE (elles
  reprennent tous les candidats), `classes_set` : les classes établies dans
  la seconde, `repaired` : les classes trouvées fausses par la vérification
  tournante depuis le lancement (doit rester à 0).

### Chargement des textures

Sans variable, le journal reçoit une ligne `texture load:` à la fin de
chaque chargement de textures (`scene/texture_load.rs`) : du premier
fichier lu, téléchargé ou décodé jusqu'à ce qu'il ne reste rien à lire,
télécharger, décoder ni envoyer au GPU pendant 2 s. Les petits chargements
(moins de 200 décodages et moins de 5 s) n'en ont pas. Une région chargée
ne s'arrête jamais tout à fait (il reste toujours quelques téléchargements
en cours) : tant qu'un chargement dure, une ligne `texture load (running):`
donne son avancement toutes les 10 s, et l'arrivée dans une autre région
(connexion, téléportation) clôt celui en cours (`cut by an arrival`) pour
que le suivant ne compte que la nouvelle région. Chaque ligne donne :

- la durée du chargement, le « temps jusqu'à tout chargé » à comparer d'une
  version à l'autre et avec Firestorm au même endroit, et les textures
  chargées sur celles connues ;
- les décodages JPEG2000, dont ceux d'une texture déjà affichée à un
  niveau plus grossier et ceux dont la texture part en BC7, et leur nombre
  par niveau de réduction (`discard 0..5`, 0 = pleine résolution) ; les
  mégapixels décodés ;
- le temps processeur des tâches de décodage, tous threads additionnés,
  et sa répartition : `codec` (OpenJPEG), `convert` (plans 32 bits vers
  pixels 8 bits), `rgba` (passage en RGBA et classe alpha), `mips`,
  `bc7` (encodage BC7 et contrôle de qualité), `staging` (masque de
  picking et copie en mémoire de transit) ;
- les octets téléchargés et le nombre de réponses, les lectures du cache
  disque réussies et manquées (données JPEG2000), et les textures relues
  du cache transcodé (`read as BC7 without a decode`) : envoyées au GPU
  sans décodage ni encodage.

Le coût du décodage seul se mesure hors du viewer sur un dossier de
textures en cache (lecture seule), par classe de taille, sur un thread
puis sur autant de threads que les tâches de fond :

```powershell
cargo run --release -p aurora-assets --example j2k_bench -- "$env:LOCALAPPDATA\Aurora\AuroraViewer\cache\tex"
```

Options : `--sample <n>` (2000 textures par défaut), `--threads <n>`,
`--levels 0,1,2,3`.

Ce que donne la compression BC7 sur le même dossier (part des textures
qui passent le contrôle de qualité, mémoire vidéo avec et sans, coût de
l'encodage face au décodage), textures opaques et avec alpha à part :

```powershell
cargo run --release -p aurora-render --example bc7_check -- "$env:LOCALAPPDATA\Aurora\AuroraViewer\cache\tex"
```

Options : `--sample <n>` (1000 par défaut), `--discard <n>` (niveau
décodé, 0 par défaut).
