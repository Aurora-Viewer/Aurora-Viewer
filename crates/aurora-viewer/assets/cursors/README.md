# Curseurs Firestorm

Les dessins d'origine sont repris à la demande de l'utilisateur (exception
aux icônes Phosphor de l'interface). Sources officielles Firestorm, commit
`a9151948839c4468c8137b547bac07f216855059` :

- `sit.png` : `indra/newview/res/toolsit.cur`.
- `money.png` : `indra/newview/res/toolbuy.cur`, utilisé pour Buy **et** Pay
  dans `LLWindowWin32::initCursors` lorsque `FSUseLegacyCursors` est désactivé.

Les pixels BGRA 32 bits du DIB des fichiers CUR ont été exportés en PNG RGBA,
sans redessiner, recolorer ou redimensionner les images. Taille : 32 × 32 ;
point actif d'origine : (20, 15), conservé dans `src/cursors.rs`.

Origine : Linden Research / Firestorm Viewer ; voir la notice du code porté
dans le [NOTICE du dépôt](../../../../NOTICE.md).
