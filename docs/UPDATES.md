# Mises à jour des paquets Windows

Le viewer interroge les releases publiques en arrière-plan et affiche immédiatement
l’écran de connexion. Son bouton reste désactivé pendant la vérification initiale
(huit secondes au maximum), puis si la mise à jour est obligatoire. Le motif est
affiché sous le bouton et vaut aussi pour Entrée et l’acceptation des conditions.
Seuls les
tags SemVer stables sont retenus ; brouillons, préversions et `assets-1` sont
ignorés. Les métadonnées Git après un tag ne changent pas sa priorité SemVer.
Sans réseau, la recherche automatique journalise l’échec sans fenêtre et libère
ce contrôle de connexion. Les builds
de développement et la démo ne vérifient pas les releases. La recherche manuelle
affiche son résultat et contourne le tag ignoré.

Le dialogue reste net devant un fond flouté sur le GPU, sans voile gris.
Une proposition obligatoire affiche uniquement Quitter et Mettre à jour ;
ni Échap ni un clic à l’extérieur ne permettent de la fermer. Le bouton de
connexion reste désactivé tant que cette mise à jour est requise.

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
  `minimum_supported_version`, `cache_invalidation_before` et
  `settings_reset_before` : `null` ou une version SemVer stable ≤ X.Y.Z.

`scripts/package.ps1` ajoute `aurora-install.json` (`schema: 1`, version lue par
`--version`). L’exe, les assets et la licence sont obligatoires dans le paquet.
Les seuils par défaut viennent d’`update-policy.json` à la racine. `null` rend toutes
les mises à jour optionnelles. Sous une version minimale, la mise à jour est
obligatoire : ni Ignorer ni Plus tard, connexion bloquée jusqu’à la mise à jour.
Les préférences et le tag ignoré ne suppriment pas ce contrôle. Modifier cette
politique est une décision de publication revue dans une PR ; elle prend effet
à la release suivante.

Dans **aurora-tools.cmd → Manage releases → Create a release**, les choix
Cache et Paramètres précèdent la confirmation. Conserver le cache et les
paramètres est le choix habituel ; `Settings::sanitized` applique les migrations
codées dans le viewer. Forcer une action inscrit la version de publication comme
seuil : toute installation antérieure devra l’appliquer. Les seuils précédents
sont repris du dernier tag stable annoté, y compris pour un utilisateur qui saute
des releases. Ils ne s’appliquent plus une fois cette version installée.
Le tag contient une ligne `Aurora-Update-Policy: {…}` ; le workflow la valide avec
`scripts/write-update-policy.ps1` avant de publier le fichier de politique.
Un tag créé manuellement sans cette ligne utilise les seuils du fichier versionné.
Pour conserver aussi les choix faits précédemment dans les outils, utiliser un
tag annoté avec cette ligne (ou reporter les seuils dans le fichier versionné).
L’interface annonce séparément le sort du cache et des paramètres, aussi bien
avant le téléchargement qu’avant « Installer et relancer Aurora ».

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

Réglages, disposition, comptes et cache sont hors du dossier remplacé. Si la
politique l’exige, l’auxiliaire déplace le cache et/ou `settings.json` vers une
sauvegarde sœur `*.aurora-before-update-<UUID>`, après la fermeture du parent.
Cela fonctionne aussi si le profil et l’installation sont sur des disques différents.
Le nouveau viewer recrée son cache et ses paramètres par défaut ; les autres
fichiers du profil restent conservés. En cas d’échec au premier démarrage, les
données nouvellement créées sont retirées et les sauvegardes restaurées. En cas
de succès, ces sauvegardes sont supprimées ; une erreur de nettoyage les garde
pour récupération et ne déclenche pas de retour arrière. Une
interruption brutale du PC peut empêcher le retour arrière automatique ;
`.aurora-update-<UUID>/backup`, à côté de l’installation, garde l’ancienne version.
Ne pas le supprimer avant d’avoir restauré le dossier installé. Les transactions
échouées sont gardées pour diagnostic ; la nouvelle instance nettoie les
transactions réussies après la fin de l’auxiliaire.

## Validation

`cargo test -p aurora-updater` : serveur HTTP loopback et paquets jetables pour
SemVer, mise à jour obligatoire, hash altéré, transfert incomplet, annulation,
ZIP dangereux, verrou, sauvegarde et retour arrière du paquet et des données.
`scripts/test-update-policy.ps1` vérifie les choix de publication, leur reprise
dans un tag annoté et le refus d’un seuil invalide, sans toucher au dépôt courant.
La démo visuelle ne
télécharge rien (`AURORA_DEMO_UPDATE`). L’essai humain avec deux vrais exécutables
est décrit dans [TESTING.md](TESTING.md#interface).
