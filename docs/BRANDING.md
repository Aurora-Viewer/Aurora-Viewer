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

### États

| Nom | Hex | Usage |
|---|---|---|
| success | `#4ADE80` | réussite, validé |
| warn | `#FB923C` | avertissement |
| danger | `#F87171` | erreur, action destructrice |
| amber | `#FCD34D` | mise en avant douce (favoris, notes) |
| rose | `#F472B6` | rare : catégories, accents ludiques |

## Règles de design

- **Verre discret.** Un flou GPU partagé apparaît dans les fenêtres, les
  dialogues et les panneaux posés sur le monde (notifications, volume, emoji,
  contrôles des médias), sous une teinte sombre. Une ombre douce et une
  bordure fine séparent la surface du monde ; les champs restent calmes et
  lisibles. Une skin avec `panel_alpha = 255` garde des surfaces opaques sans
  ce flou. Hors d'une fenêtre, une surface se dessine avec
  `widgets::overlay` (ou `widgets::overlay_frame` sans monde derrière, comme
  sur l'écran de connexion), jamais avec un fond, une bordure ou un rayon
  écrits à la main. Les barres du haut et du bas restent pleines (`bar`).
  Les dégradés sont réservés aux éléments de marque (fond de connexion, aurores).
- **Arrondis discrets.** Design plat, angles presque carrés. Boutons :
  rayon 5 ; fenêtres, cartes et pastilles de la barre d'outils : rayon 3,
  avec des coins proches de ceux de la construction, y compris au repli.
  Les rayons viennent du thème, jamais d'une valeur plus grande dans le code.
- **Boutons pleins.** Une action avec icône utilise `widgets::action_button` :
  une face pleine sans bordure, neutre, violette pour l'action principale,
  `danger` pour une action destructrice. La barre d'outils du bas n'a pas de
  traits de séparation : l'espacement et la pastille du bouton survolé ou
  actif suffisent.
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
  `violet_light`).
- **Séparateurs très subtils mais visibles.** Une ligne fine d'un neutre à
  peine plus clair que le fond, pour que l'on voie où commence et finit chaque
  élément sans alourdir.
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

## Logos

Fichiers dans [`assets/branding/`](../assets/branding/) :

| Fichier | Usage |
|---|---|
| `logo-loup-couleur.svg` / `-4096.png` | logo complet : écran de connexion, README, icône de fenêtre |
| `silhouette-loup-blanche.svg` / `-4096.png` | silhouette blanche : petites icônes, barre d'outils, monochrome |
| `silhouette-loup-blanche-fond-noir.svg` | silhouette sur fond noir (seul fichier avec un fond opaque) |
| `aurora.ico` | logo complet en icône Windows (16 à 256 px, tiré du PNG 4096) : raccourci « Aurora Tools » |

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
