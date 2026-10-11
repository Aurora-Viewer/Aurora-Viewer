# Mises à jour des paquets Windows

Le viewer interroge les releases publiques sans retarder la connexion. Seuls les
tags SemVer stables sont retenus ; brouillons, préversions et `assets-1` sont
ignorés. Les métadonnées Git après un tag ne changent pas sa priorité SemVer.
Sans réseau, la recherche automatique journalise l’échec sans fenêtre. Les builds
de développement et la démo ne vérifient pas les releases. La recherche manuelle
affiche son résultat et contourne le tag ignoré.

Les modes reprennent les choix d’`UpdaterServiceSetting` de Firestorm :
obligatoires uniquement, demander, télécharger automatiquement. Aurora choisit
Demander par défaut pour répondre à la demande de proposition au lancement ;
Firestorm choisit 0. Le mode automatique prépare les fichiers et attend l’accord
de l’utilisateur avant de quitter et relancer sa session.

## Paquet et politique de publication

Le workflow Release publie ensemble :

- `Aurora-Viewer-vX.Y.Z-windows-x64.zip`, avec un seul dossier racine de ce nom ;
- son fichier `.zip.sha256`, au format `empreinte  nom-du-zip` ;
- `aurora-update-policy.json` : `schema: 1`, `version: X.Y.Z`,
  `minimum_supported_version: null` ou une version SemVer stable ≤ X.Y.Z.

`scripts/package.ps1` ajoute `aurora-install.json` (`schema: 1`, version lue par
`--version`). L’exe, les assets et la licence sont obligatoires dans le paquet.
La version minimale vient d’`update-policy.json` à la racine. `null` rend toutes
les mises à jour optionnelles. Sous une version minimale, la mise à jour est
obligatoire : ni Ignorer ni Plus tard, connexion bloquée jusqu’à la mise à jour.
Les préférences et le tag ignoré ne suppriment pas ce contrôle. Modifier cette
politique est une décision de publication revue dans une PR ; elle prend effet
à la release suivante.

Les anciennes releases sans empreinte ou manifeste passent par leur page pour
une installation manuelle. Les plateformes autres que Windows restent hors
périmètre. Le ZIP actuel est conservé : Velopack a été étudié, mais sa chaîne de
paquets et d’installeurs n’est pas celle des releases Aurora.

## Remplacement et retour arrière

Téléchargement, hash et extraction tournent en arrière-plan. L’extraction refuse
traversées, liens, doublons Windows, fichiers réservés et archives dépassant les
limites de taille. La préparation se fait à côté de l’installation, sur le même
volume. L’installation refuse dépôts, arbres de compilation, liens de dossiers
et dossiers contenant des fichiers étrangers au paquet.

Après accord de relance, une copie du viewer sert d’auxiliaire hors du dossier
installé. Elle attend la fin du parent avec un handle Windows, prend un verrou,
sauvegarde l’ancien dossier et place le nouveau. Les arguments sont conservés.
La nouvelle instance signale une image présentée ; la sauvegarde est alors
supprimée. Si elle sort ou n’affiche rien pendant 90 secondes, l’ancienne version
est restaurée et relancée avec un message. Les erreurs proposent la page de release.

Réglages, disposition, comptes et cache sont hors du dossier remplacé. Une
interruption brutale du PC peut empêcher le retour arrière automatique ;
`.aurora-update-<UUID>/backup`, à côté de l’installation, garde l’ancienne version.
Ne pas le supprimer avant d’avoir restauré le dossier installé. Les transactions
échouées sont gardées pour diagnostic ; la nouvelle instance nettoie les
transactions réussies après la fin de l’auxiliaire.

## Validation

`cargo test -p aurora-updater` : serveur HTTP loopback et paquets jetables pour
SemVer, mise à jour obligatoire, hash altéré, transfert incomplet, annulation,
ZIP dangereux, verrou, sauvegarde et retour arrière. La démo visuelle ne
télécharge rien (`AURORA_DEMO_UPDATE`). L’essai humain avec deux vrais exécutables
est décrit dans [TESTING.md](TESTING.md#interface).
