# Branding et interface d'Aurora Viewer

L'interface d'Aurora est **propre, moderne et légèrement vitrée** : surfaces sombres et
calmes, une seule couleur d'accent forte (le violet), du texte lisible, des
icônes fines et cohérentes. Chaque écran doit avoir l'air soigné et fini.

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

- **Verre discret.** Un flou GPU partagé apparaît seulement dans les fenêtres
  et dialogues, sous une teinte sombre. Une ombre douce et une bordure fine
  séparent la fenêtre du monde ; les champs restent calmes et lisibles.
  Une skin avec `panel_alpha = 255` garde des surfaces opaques sans ce flou.
  Les dégradés sont réservés aux éléments de marque (fond de connexion, aurores).
- **Arrondis discrets.** Boutons : rayon 5 ; fenêtres et cartes : rayon 3,
  avec des coins proches de ceux de la construction, y compris au repli.
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

## Icônes : Phosphor

Toutes les icônes de l'interface sont des [Phosphor Icons](https://phosphoricons.com)
(MIT), **graisse « regular »**, dessinées en blanc puis teintées avec la palette.
Ne pas mélanger avec un autre jeu d'icônes.

- La bibliothèque complète est dans
  [`assets/phosphor-icons/SVGs/`](../assets/phosphor-icons/SVGs/) (6 graisses,
  plus de 1 500 icônes) : c'est là qu'on choisit une icône.
- Seules les icônes utilisées sont embarquées dans le viewer :
  `crates/aurora-viewer/assets/icons/phosphor/<nom>.svg`.

**Ajouter une icône :**

1. Choisir le nom dans `assets/phosphor-icons/SVGs/regular/` (ex.
   `microphone.svg`).
2. La copier dans `crates/aurora-viewer/assets/icons/phosphor/`.
3. Ajouter son nom dans la liste `svg_list!` de
   `crates/aurora-viewer/src/ui/icons.rs`.
4. L'utiliser avec `icons.get("microphone")`, teintée par la palette.

Les graisses « fill » ou « bold » ne servent qu'à un état précis (élément
sélectionné, alerte) et uniquement si c'est cohérent avec le reste de l'écran.
Exception : le micro barré des étiquettes d'avatars hors vocal est en « fill »
et en `danger`, pour garder ce statut lisible à petite taille.
Les icônes des autorisations de la parcelle (bout droit du champ de
lieu) sont en « fill », car ces petites formes colorées en vert / rouge s'y
lisent mieux qu'au trait. De même pour l'icône devant les liens web du chat et
des profils : `check-circle-fill` en `success` (site de confiance) et
`warning-fill` en `amber` (lien externe) et `x-circle-fill` en `danger` (lien dangereux), aussi dans leurs avertissements.
