# Curseurs Firestorm

Les dessins d'origine sont repris à la demande de l'utilisateur (exception
aux icônes Phosphor de l'interface). Sources officielles Firestorm, commit
`a9151948839c4468c8137b547bac07f216855059` :

- `sit.png` : `indra/newview/res/toolsit.cur`.
- `money.png` : `indra/newview/res/toolbuy.cur`, utilisé pour Buy **et** Pay
  dans `LLWindowWin32::initCursors` lorsque `FSUseLegacyCursors` est désactivé.
- `open.png` : `toolopen.cur`, point actif (20, 15).
- `play.png`, `pause.png`, `media-open.png` : `toolplay.cur`, `toolpause.cur`,
  `toolmediaopen.cur`, point actif (1, 1).
- `zoom.png`, `grab.png` : `lltoolzoomin.cur`, `lltoolgrab.cur`, points actifs
  (7, 5) et (2, 13).

Les pixels du DIB des fichiers CUR (BGRA 32 bits, ou palette 8 / 1 bit avec
masque de transparence AND) ont été exportés en PNG RGBA,
sans redessiner, recolorer ou redimensionner les images. Taille : 32 × 32 ;
points actifs d'origine conservés dans `src/cursors.rs`. Aucun de ces dessins
ne contient de pixel monochrome demandant l'inversion du fond.

Touch utilise la main native du système, comme Firestorm sous Windows.
NONE / TOUCH sur un objet non interactif, DISABLED sur un objet non physique et les actions indisponibles
gardent la flèche. IGNORE laisse le curseur et le clic atteindre ce qui est derrière.
DISABLED supprime le toucher et l'héritage de l'action, mais ne verrouille pas
à lui seul la saisie d'un objet physique : le curseur Grab reste alors disponible,
comme dans `LLToolPie::useClickAction` / `handleHover` de Firestorm.

Origine : Linden Research / Firestorm Viewer ; voir la notice du code porté
dans le [NOTICE du dépôt](../../../../NOTICE.md).
