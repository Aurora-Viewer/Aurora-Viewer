# Branding et interface d'Aurora Viewer

L'interface d'Aurora est **propre, moderne et légèrement vitrée** : surfaces sombres et
calmes, une seule couleur d'accent forte (le violet), du texte lisible, des
icônes nettes et cohérentes. Chaque écran doit avoir l'air soigné et fini.

## Palette

Les couleurs viennent de `crates/aurora-viewer/src/theme.rs` (`Theme::default`).
Dans le code, on utilise toujours la `Palette` résolue (`p.violet`, `p.ink`…),
**jamais une couleur écrite en dur** : une skin peut les changer.

### Couleurs de la marque

| Nom | Hex | Usage |
|---|---|---|
| violet | `#8B5CF6` | accent principal : sélection, bouton actif, focus, liens |
| indigo | `#4F46E5` | accent secondaire, dégradés de la marque (logo, fond de connexion) |
| teal | `#5EEAD4` | petite touche : états « actif / en ligne », points d'attention positifs ; liens cliquables des textes (web, lieux, avatars, groupes dans le chat, les bulles et les profils) |
| navy | `#070B1F` | fond profond (écran de connexion, aurores) |
| violet_light | `#A78BFA` | violet sur fond sombre (texte d'accent, survol) |
| violet_pale | `#C4B5FD` | violet très clair (détails discrets) |
| indigo_light | `#818CF8` | variante claire de l'indigo |

### Neutres de l'interface

| Nom | Hex | Usage |
|---|---|---|
| panel | `#252830` | fond des fenêtres (opacité `panel_alpha` = 230) |
| bar | `#1B1E26` | barres du haut et du bas |
| field | `#181B23` | champs de saisie, zones creuses |
| raised | `#363B47` | boutons, surfaces survolées |
| ink | `#E8EDFB` | texte principal |
| muted | `#9AA6C6` | texte secondaire, libellés |
| muted_dim | `#6F7BA0` | texte tertiaire, désactivé |
| on_accent | `#FFFFFF` | texte et icônes posés sur une face violette : bouton principal, outil actif, coche d'une case, point d'un bouton radio, curseur d'un interrupteur allumé |

### États

| Nom | Hex | Usage |
|---|---|---|
| success | `#4ADE80` | réussite, validé |
| warn | `#FB923C` | avertissement |
| danger | `#F87171` | erreur, action destructrice |
| amber | `#FCD34D` | mise en avant douce (favoris, notes) |
| rose | `#F472B6` | rare : catégories, accents ludiques |

## Règles de design

- **L'apparence vient du thème.** Une fenêtre n'écrit ni couleur, ni rayon,
  ni trait : elle prend les rôles de la `Palette`, les rayons du thème et
  les widgets partagés de `ui/widgets.rs`. Ce qu'une fenêtre dessine avec
  ses propres valeurs est du contenu, pas de l'habillage : couleurs d'une
  carte et de sa légende, couleurs de classification, intérieur d'une
  pastille de couleur, image affichée telle quelle
  (`widgets::PICTURE_TINT`). Un test le vérifie (`ui/theme_guard.rs`, lancé
  par `check.ps1`) ; une ligne de contenu s'y déclare par un commentaire
  `// theme-guard: content — la raison`.
- **Verre discret.** Un flou GPU partagé apparaît dans les fenêtres, les
  dialogues, les barres du haut et du bas et tout ce qui est posé sur le
  monde (notifications, volume, emoji, contrôles des médias, bulles du chat
  local, pie menu), sous une teinte sombre. Une ombre douce et une
  bordure fine séparent la surface du monde ; les champs restent calmes et
  lisibles. Une skin avec `panel_alpha = 255` garde des surfaces opaques sans
  ce flou. Hors d'une fenêtre, une surface se dessine avec
  `widgets::overlay` (ou `widgets::overlay_frame` sans monde derrière, comme
  sur l'écran de connexion), jamais avec un fond, une bordure ou un rayon
  écrits à la main. Les barres gardent la couleur `bar` comme teinte
  (`widgets::finish_bar_glass`) ; la mini-carte garde l'opacité de Firestorm
  avec la bordure et l'ombre des fenêtres. Les menus, sous-menus, menus du
  clic droit et listes déroulantes reçoivent ce verre d'un seul endroit
  (`menu::glaze`, en fin d'image, dans le monde) : un nouveau menu n'a rien
  à faire pour l'avoir.
  Les dégradés sont réservés aux éléments de marque (fond de connexion,
  aurores, remplissage de la barre de progression).
- **Écran de connexion.** C'est la vitrine du viewer. Le flou des fenêtres
  ne connaît que la scène 3D, absente ici : chaque carte (`login::card`)
  redessine donc sous sa teinte le ciel qu'elle recouvre, hors de mise au
  point (`aurora_bg::frosted` : sans étoiles, rayons des aurores fondus,
  crêtes adoucies), avec deux lueurs de l'aurore (violet, turquoise) qui
  débordent autour d'elle. Son bord supérieur accroche la lumière ; la carte
  de connexion porte le filet au dégradé de la marque. Les actualités se
  tiennent à côté de la carte de connexion, sur sa ligne et à sa hauteur
  (la liste défile), la version au-dessus d'elles. Les boutons du bas
  et le numéro de version partagent la même ligne.
- **Barres de progression.** `widgets::progress` : une piste plate (celle
  des curseurs) remplie au dégradé de la marque, qui glisse vers sa valeur
  au lieu de sauter ; le pourcentage écrit dessous glisse avec elle. Les
  bandeaux de chargement et de téléportation glissent de la même façon. Un
  niveau instantané (micro) passe par `widgets::level`, qui ne lisse que le
  tremblement entre deux mesures. Jamais `egui::ProgressBar`.
- **Bouton et panneau du micro.** Le micro reste au centre de son bouton,
  comme les icônes voisines ; la flèche qui le suit pointe vers le panneau
  qu'elle ouvre au-dessus de la barre. Le panneau (verre des fenêtres)
  donne l'état de la voix, les deux façons de parler en cartes (celle en
  cours allumée comme un bouton actif de la barre et cochée, la touche à
  maintenir dessinée en touche), puis le micro et la sortie audio l'un sous
  l'autre, et l'accès aux préférences.
- **Chargement et téléportation.** Un « bandeau cinéma » sans panneau
  (`ui/loading.rs`) : la dernière vue floutée occupe tout l'écran, le nom de
  la région s'écrit en grand en bas à gauche avec ce qu'on sait d'elle
  (classification, point d'arrivée, résidents présents) et l'étape en cours ;
  en bas à droite, la tuile de la région sur la carte du monde avec le point
  d'arrivée, et la progression en chiffres ; la progression elle-même est un
  filet lumineux au dégradé de la marque sur tout le bord inférieur. La
  marque (loup et nom) reste discrète en haut à gauche.
- **Arrondis discrets.** Design plat, angles presque carrés. Boutons :
  rayon 5 ; fenêtres, cartes et pastilles de la barre d'outils : rayon 3,
  avec des coins proches de ceux de la construction, y compris au repli.
  Les rayons viennent du thème, jamais d'un nombre dans le code :
  `ui.visuals().window_corner_radius` pour les cartes, les pastilles, les
  surlignages de liste et les cadres d'image ou de couleur,
  `ui.visuals().widgets.inactive.corner_radius` pour les boutons,
  `widgets::inner_radius` (un pixel de moins que les fenêtres) pour une
  petite marque dans un contrôle ou une ligne : curseur d'interrupteur,
  touche, carré d'état, lettre de classification. Restent tels quels : les
  angles droits voulus (rayon 0), les ronds (points d'état, pastille des
  notifications) et les barres fines (piste d'une progression ou d'un
  niveau, repère sur un rail), dont les bouts de 2 px viennent d'une seule
  constante, `widgets::TRACK_RADIUS`.
- **Plaque de nom.** L'étiquette au-dessus d'un avatar (`ui/nameplate.rs`)
  est la **seule exception** aux angles presque carrés : une plaque de verre
  aux extrémités entièrement arrondies. Ce n'est pas un précédent : rien
  d'autre dans l'interface ne s'arrondit davantage.
  - *Verre* : la teinte de la plaque (à mi-chemin entre `navy` et `bar`, à
    52 %) sur le flou des fenêtres, une bordure claire d'un pixel (`ink` à
    14 %), une ombre douce dessous. Le flou va aux 16 plaques les plus
    proches ; les autres, et les pastilles lointaines, n'ont que la teinte,
    plus dense. Une skin opaque (`panel_alpha = 255`) n'a pas de flou.
  - *Trois niveaux selon la distance à la caméra* : jusqu'à 15 m, la plaque
    complète (photo de profil dans un cercle, pastille de statut, nom
    d'affichage en semi-gras, nom d'utilisateur en `muted`, titre de groupe
    en `teal`, badges ronds flottant sur le bord supérieur droit) ; jusqu'à
    40 m, une pastille compacte (nom et badges sur une ligne) ; au-delà, le
    nom seul. Une plaque ne change de niveau qu'à 1,5 m au-delà d'un seuil et
    glisse d'un niveau à l'autre en 0,18 s. Dans une foule, seules les huit
    plaques les plus proches sont complètes, les suivantes sont des pastilles
    compactes. Une ligne masquée par les préférences est absente : jamais de
    ligne vide.
  - *Statut* : pastille en bas à droite de la photo, `success` (présent),
    `amber` (absent), `danger` (ne pas déranger) ; aucune pour un résident
    bloqué.
  - *Badges* : micro `success` (dans le canal vocal), trois points animés
    (écrit), cœur `violet_light` (ami), triangle `amber` (avatar trop
    complexe, affiché en silhouette), interdit `danger` (bloqué).
  - *Voix* : dans le canal vocal, le contour de la plaque prend la couleur
    `success`, à tous les niveaux ; pendant que la personne parle, un halo
    serré autour de la plaque suit le niveau de sa voix (montée rapide,
    retombée plus lente, aucun halo au silence).
  - *Couleurs fixées par le thème.* Nom d'affichage en `ink` (le nôtre
    aussi), `violet_light` pour un ami, `rose` pour le personnel de Linden
    Lab, `muted_dim` en italique sur une plaque à 62 % pour un résident
    bloqué. Les couleurs d'étiquette des préférences (page Couleurs)
    appartiennent aux étiquettes classiques, que l'on retrouve dans
    Préférences › Interface › « Style des étiquettes ».
  - *Taille* : le nom d'affichage garde la taille des étiquettes classiques
    (13 points) sur la plaque complète ; tout le reste en découle par les
    proportions de la maquette.
- **Boutons pleins.** Tous les boutons et listes déroulantes sont des faces
  pleines sans bordure visible : le thème (`theme.rs`) donne à leur bordure la
  couleur de la face, seuls les champs de saisie et les cases à cocher gardent
  un contour. Un bouton ne fixe donc ni fond ni rayon lui-même, sauf pour
  l'action principale (violet) ou destructrice (`danger`). Une action avec
  icône utilise `widgets::action_button`. La barre d'outils du bas n'a pas de
  traits de séparation : l'espacement et la pastille du bouton survolé ou
  actif suffisent.
- **Interrupteurs et cases.** L'interrupteur (`widgets::switch`) est presque
  carré : piste au rayon du thème, curseur carré qui glisse. Il sert à un
  réglage isolé (seul avec son libellé : `widgets::switch_labeled`) ; les
  listes de drapeaux (droits, données d'un rapport, cellules d'un tableau)
  gardent des cases, dessinées par `widgets::checkbox` (carré plat, violet et
  coché quand il est actif), jamais par `ui.checkbox`.
- **Boutons radio.** Un choix parmi plusieurs passe par `widgets::radio`
  (`radio_value` pour une valeur), jamais par `ui.radio` : la ligne de la
  case à cocher (même taille, même libellé, même survol), mais ronde pour
  qu'on ne la prenne pas pour une case : anneau fin au repos, violet autour
  d'un point quand le choix est fait. C'est le seul contrôle rond de
  l'interface ; les autres ronds sont des marques (points d'état, pastille
  des notifications).
- **Sections repliables.** `widgets::collapsing` : une ligne pleine largeur
  avec un chevron Tabler et le titre de la section, allumée en pastille au
  survol, sans trait ; jamais `egui::CollapsingHeader`.
- **Couleurs.** Une couleur se choisit avec `widgets::color_button` : la
  pastille ouvre un petit panneau vitré (champ saturation / valeur, barre de
  teinte, nouvelle couleur au-dessus de l'ancienne, champs R V B et
  hexadécimal, pipette), jamais les `color_edit_button_*` d'egui. La palette
  de couleur des outils de construction reste une fenêtre à part.
- **Hiérarchie commune.** Titre et commandes de fenêtre en haut, navigation
  stable, contenu regroupé par fonction et actions accessibles.
  L’inventaire garde une grande liste et une seule ligne de statut en bas,
  sans fiche de détails de la sélection.
  Les listes défilent dans leur zone et ne font pas grandir la fenêtre.
  La construction utilise un inspecteur compact avec un rail de sections.
- **Navigation légère.** Les onglets et commandes de navigation de l’inventaire
  sont soulignés en gris au repos, en violet à l’activation, sans contour.
  Le survol reste discret. Les actions secondaires des dialogues gardent
  leur surface sombre. La barre de commandes suit directement le titre ;
  la croix d’effacement est intégrée au filtre, sans déplacer les contrôles.
- **Une seule couleur d'accent par zone.** Le violet signale ce qui est actif
  ou important ; le reste est neutre.
- **Contraste et lisibilité.** Texte `ink` sur `panel` ; texte secondaire en
  `muted`. Jamais de texte violet foncé sur fond sombre (utiliser
  `violet_light`). Sur une face violette pleine, texte et icônes en
  `on_accent` (ni blanc écrit en dur, ni `ink`, qu'une skin peut assombrir) ;
  sur une face `danger` ou `amber`, claires, le texte reste en `navy`.
- **Séparateurs très subtils mais visibles.** Une ligne fine d'un neutre à
  peine plus clair que le fond, pour que l'on voie où commence et finit chaque
  élément sans alourdir. Entre deux zones d'une fenêtre : `widgets::divider`
  (horizontal dans une colonne, vertical dans une ligne) ; entre deux groupes
  d'entrées d'un menu : `menu::separator` ; un trait placé à la main (bord
  d'un rail, limite entre deux volets) prend `widgets::divider_stroke`.
  Jamais `ui.separator()`, et aucun trait entre des boutons ou les éléments
  d'une barre d'outils.
- **Typographie.** Inter (régulier, semi-gras) ; Noto Sans et Roboto en secours.
  Pas d'autre police sans raison. Les caractères qu'elles n'ont pas (noms
  d'affichage et messages en géorgien, japonais, arabe, lettres
  mathématiques…) sont dessinés avec les polices du système, jamais choisies
  pour un texte de l'interface elle-même.
- **Espacement régulier.** Réutiliser les widgets existants
  (`ui/widgets.rs` : boutons plats, interrupteurs, pastilles d'aide, triangle
  d'avertissement…) plutôt que d'en recréer.
- **Textes de l'interface en français**, courts et clairs. Une info-bulle
  explique toute option dont l'effet n'est pas évident.
- **Fenêtres aimantées.** Une fenêtre déplacée près d'une autre, d'une barre
  ou d'un bord suit le pointeur sans saut ; ce qui va se passer au relâchement
  est montré par un guide : un trait de 2 px en `violet_light` le long des
  bords qui vont se toucher (sur la longueur qu'ils partagent) ou s'aligner,
  et le contour très léger de l'endroit où la fenêtre va se poser, au rayon
  des fenêtres. Pas de guide, pas de collage. Les fenêtres collées se
  touchent, bord contre bord, entre elles comme avec les barres et les bords,
  sans écart, comme celles de la disposition du premier lancement
  (`layout::GAP`, à 0). Pendant le geste, une
  petite pastille sur le verre des fenêtres rappelle la touche qui libère :
  la touche Alt en keycap (`widgets::keycap`) suivie de « ne pas coller » ou
  « détacher cette fenêtre », en texte `muted`, posée près de la fenêtre, hors
  du guide et du pointeur, après un court délai. Aucun bouton ni décor
  permanent pour les fenêtres collées.

## Logos

Fichiers dans [`assets/branding/`](../assets/branding/) :

| Fichier | Usage |
|---|---|
| `logo-loup-couleur.svg` / `-4096.png` | logo complet : écran de connexion, README, icône de fenêtre |
| `silhouette-loup-blanche.svg` / `-4096.png` | silhouette blanche : petites icônes, barre d'outils, monochrome |
| `silhouette-loup-blanche-fond-noir.svg` | silhouette sur fond noir (seul fichier avec un fond opaque) |
| `aurora.ico` | logo complet en icône Windows (16 à 256 px, tiré du PNG 4096) : icône de `aurora-viewer.exe` (intégrée par son `build.rs`), raccourci « Aurora Tools » |

Le logo est une tête de loup de profil, tournée vers la droite, en 4 couleurs :
violet `#8B5CF6`, indigo `#4F46E5`, turquoise `#5EEAD4` et bleu nuit `#070B1F`.
Ne pas le déformer, le recolorer ni lui ajouter d'effet.

## Icônes : Tabler

Toutes les icônes de l'interface sont des [Tabler Icons](https://tabler.io/icons)
(MIT), dessinées en blanc puis teintées avec la palette.
Ne pas mélanger avec un autre jeu d'icônes.

- La bibliothèque complète (version 3.49.0) est dans
  [`assets/tabler-icons/`](../assets/tabler-icons/) : `outline/` (5 184 icônes
  au trait) et `filled/` (1 054 icônes pleines). C'est là qu'on choisit une
  icône ; le site de Tabler permet de chercher par mot-clé.
- Seules les icônes utilisées sont embarquées dans le viewer :
  `crates/aurora-viewer/assets/icons/tabler/outline/<nom>.svg` et
  `…/tabler/filled/<nom>.svg`.
- Dans le code, une icône porte son nom Tabler : `eye-off` pour la version au
  trait, `eye-filled` pour la version pleine.

**La version pleine d'abord.** Tabler ne propose une version pleine que pour
une icône sur cinq : on la prend chaque fois qu'elle existe et qu'elle dit
bien la fonction. La version au trait sert dans trois cas :

- **glyphes linéaires**, qui n'ont pas de surface à remplir : croix, plus,
  moins, coche, flèches, annuler / rétablir, actualiser, pivoter, `</>` ;
- **icônes barrées** (`eye-off`, `microphone-off`, `volume-off`,
  `message-off`) : Tabler ne les dessine qu'au trait ;
- **fonctions sans icône pleine qui convienne** : objet (`box`), personnes au
  pluriel (`users`, `users-group`, `user-plus`), haut-parleur (`volume`), carte
  (`map`), mains (`hand-grab`, `hand-stop`), s'asseoir (`armchair`), marcher /
  courir (`walk`, `run`)…

Ne pas forcer une icône pleine qui ne dit pas la fonction. Deux icônes qui
vont par paire (lier / délier, plus / moins) restent dans le même style.
Une même fonction garde la même icône partout (script = `code`, objet = `box`,
tenue = `hanger-2-filled`, profil = `user-filled`…) : chercher l'icône déjà
utilisée avant d'en ajouter une.

**Ajouter une icône :**

1. Choisir le nom dans `assets/tabler-icons/filled/`, sinon dans
   `assets/tabler-icons/outline/` (ex. `microphone.svg`).
2. La copier dans `crates/aurora-viewer/assets/icons/tabler/filled/` (ou
   `outline/`).
3. Ajouter son nom dans la liste `FILLED` (ou `OUTLINE`) de
   `crates/aurora-viewer/src/ui/icons.rs`.
4. L'utiliser avec `icons.get("microphone-filled")` (ou `icons.get("volume")`
   pour une icône au trait), teintée par la palette.

Les icônes des autorisations de la parcelle (bout droit du champ de lieu) sont
colorées en vert / rouge ; une barre dessinée par le code raye celles qui sont
interdites. L'icône devant les liens web du chat et des profils est
`circle-check-filled` en `success` (site de confiance), `alert-triangle-filled`
en `amber` (lien externe) ou `circle-x-filled` en `danger` (lien dangereux),
aussi dans leurs avertissements.

## Curseurs

Les curseurs du monde sont dessinés à partir des mêmes icônes Tabler au trait,
à la couleur `ink` cernée de `bar` pour rester lisibles sur n'importe quel
décor (`crates/aurora-viewer/src/cursors.rs`). Les règles sont celles de
Firestorm, seuls les dessins changent :

| Situation | Curseur |
|---|---|
| Objet à toucher | `hand-finger` seule, le bout du doigt est le point cliqué |
| S'asseoir, acheter, payer, ouvrir | flèche + pastille `armchair`, `shopping-cart`, `coin`, `package` |
| Média : lire, mettre en pause, ouvrir | flèche + pastille `player-play`, `player-pause`, `device-tv` |
| Zoomer sur l'objet | flèche + pastille `zoom-in` |
| Saisir un objet physique, déplacer un HUD | `hand-grab` seule |
| Caméra : Alt, Ctrl + Alt, Ctrl + Alt + Maj | `focus-centered`, `view-360`, `arrows-move`, centrés sur le point visé |
| Pipette du nuancier | `color-picker`, la pointe est le point prélevé |

Avec la flèche + pastille, la pointe de la flèche est le point cliqué.

## Touches des raccourcis

Dans Préférences › Raccourcis, chaque touche d'un raccourci est une capsule
claire sur le bouton sombre, sans « + » entre elles. Les lettres, les
chiffres, F1 à F9 et les flèches utilisent les icônes de touche de Tabler
(`square-letter-w-filled`, `square-number-1-filled`, `square-f5-filled`,
`square-arrow-up-filled`). Les autres touches (Ctrl, Alt, Maj, Échap, F10 à
F12, pavé numérique, lettres accentuées…) sont une capsule de la même forme
autour de leur nom ; un bouton de souris y ajoute l'icône `mouse-filled`
avant son nom, Tabler n'ayant pas d'icône par bouton.
