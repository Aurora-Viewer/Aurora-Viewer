# Imports d’assets — référence Firestorm

Cette matrice suit les imports depuis le disque et leurs aperçus locaux.
Elle sert à vérifier les options, les valeurs par défaut, le protocole et
les erreurs, avant la validation humaine sur grille. Une fenêtre affichée
ne suffit pas à considérer son import comme terminé.

Périmètre confirmé le 12/10/2026 : textures, matériaux, meshes, sons,
animations, lots et aperçus locaux. Les imports de linksets, EEP et
Windlight sont recensés pour des tâches ultérieures dans `TASKS.md`.

Référence consultée : Firestorm `d942e1118d01c3383ef7d6b64f248af77452b0dc`
(12/10/2026). Les choix visibles suivent cette révision des sources.

## Parcours commun

- Choisir un ou plusieurs fichiers ; nom et description ; dossier de
  destination par type, y compris les cinq dossiers personnalisés existants.
  Le sélecteur de destination propose un filtre par nom, sans modifier le
  dossier sélectionné pendant la saisie. Les destinations respectent les
  protections de l’inventaire ; leur liste est mise en cache et virtualisée.
- Liste compacte : une ligne par fichier, sans mention « Prêt » ; progression,
  succès, annulation et erreur indiqués par une icône avec infobulle.
- Permissions des chargements : prochain propriétaire (modifier, copier,
  transférer), groupe et tout le monde ; ne pas accorder des droits que le
  serveur n’a pas confirmés.
- Prix issu des avantages du compte et des capacités de la région ; solde
  suffisant ; confirmation explicite avant chaque opération payante.
- Analyse, conversion et encodage en arrière-plan ; progression, annulation,
  erreurs par fichier ; pas de nouvelle tentative automatique d’un envoi
  dont le résultat est inconnu (risque de double facturation).
- Réponse du serveur puis relecture d’inventaire avant de déclarer le succès.
- Fichiers invalides, dimensions/durées/tailles hors limites, capabilities
  absentes, refus et expiration : conserver le brouillon et expliquer l’erreur.

Sources : `llviewermenufile.cpp`, `llviewerassetupload.cpp`,
`llfloaterperms.cpp`, `llagentbenefits.cpp`, `panel_preferences_uploads.xml`.

## Textures

- Formats de la boîte de fichiers Firestorm et validation RGB/RGBA.
- Dimensions avant/après redimensionnement, dimensions maximales X/Y,
  puissances de deux, 2K et tarif associé ; lot limité à 1024 au choix.
- Compression sans perte pour les petites textures (surface ≤ 128²),
  réglage `LosslessJ2CUpload` ; JPEG2000 de chargement pour les autres.
- Détection du canal alpha presque opaque, suppression proposée et
  conservation volontaire après avertissement.
- Aperçu image avec déplacement/zoom ; cheveux, tête/corps masculin ou
  féminin, jupe et sculpted prim avec rotation/zoom.
- Ancienne case de texture temporaire : visible dans Firestorm seulement sans
  server-side baking, mais `TemporaryUpload` n’est plus lu par son parcours
  d’envoi dans cette révision. Les régions SL actuelles utilisent le baking ;
  aucun faux chargement gratuit n’est proposé par Aurora. Les aperçus locaux
  restent gratuits et n’envoient aucun asset au serveur.

Sources : `llfloaterimagepreview.cpp`, `floater_image_preview.xml`,
`llviewertexturelist.cpp`, `llimageraw.cpp`.

## Sons

- WAV : restrictions PCM, canaux, fréquence, profondeur et durée ; conversion
  vers le son Vorbis de Second Life, avec les paramètres Firestorm.
- Nom/description, tarif et destination ; aperçu sonore local.
  Les commandes Jouer et Arrêter sont placées côte à côte.

Sources : `llfloaternamedesc.cpp`, `llaudio/llvorbisencode.cpp`,
`floater_sound_preview.xml`.

## Animations

- `.anim` : validation sans altération des données, notamment des contraintes.
  Les marqueurs de boucle négatifs et autres valeurs temporelles finies des
  exporteurs sont acceptés comme par `LLKeyframeMotion::deserialize`.
- BVH : hiérarchie, canaux et ordre des axes, pose de référence, traduction
  des articulations, suppression des pistes fixes, réduction des clés.
- Priorité 2 par défaut ; boucle désactivée ; entrée/sortie en pourcentage
  ou en images ; fondus de 0,3 s ; poses des mains et expressions.
- Aperçu local : debout/marche/assis/vol, lecture/pause/arrêt, curseur temporel,
  caméra, rechargement du fichier, statistiques et erreurs de parsing.
  Cliquer une animation dans la liste relance sa lecture depuis le début,
  y compris si elle avait déjà été jouée ou arrêtée.
  « Jouer aussi sur mon avatar » est commun à toutes les animations et
  mémorisé dans les réglages du viewer, activé ou désactivé. La case est
  placée à droite des commandes de lecture.
  La pause et le curseur pilotent également l’aperçu sur notre avatar ;
  les fins de ces mouvements locaux ne produisent aucun arrêt réseau.
- Imports par lot : `.anim` acceptés, BVH exclus comme Firestorm.

Sources : `llfloaterbvhpreview.cpp`,
`llcharacter/llbvhloader.cpp`, `llcharacter/llkeyframemotion.cpp`,
`floater_animation_bvh_preview.xml`, `floater_animation_anim_preview.xml`.

## Matériaux PBR

- glTF/GLB : choix du matériau ou import de tous les matériaux du document.
- Couleur de base/RGBA, normal, métallique/rugosité (canaux G/B), émission,
  facteurs, opacité/masque/mélange, seuil alpha et double face. Les transformations
  des textures sont remises à zéro pour l’asset d’inventaire comme dans
  `sanitizeAssetMaterial` ; elles appartiennent aux overrides des faces.
- Retrait de l’alpha des cartes techniques et occlusion distincte redimensionnée
  puis fusionnée en R dans ORM ; R vaut 255 sans occlusion. Une carte
  d’occlusion seule crée une carte ORM blanche en G/B.
- Textures externes/embarquées/locales ou UUID d’inventaire ; résolution
  maximale, suppression/remplacement, coût détaillé des textures.
- Réutiliser les textures partagées ; envoyer les textures avant le matériau,
  puis remplacer leurs références par les UUID confirmés.
- Matériau sans frais propres ; création et sauvegarde comme un asset
  `material`, enveloppe LLSD version 1.1 / `GLTF 2.0`.
- Aperçu local et sauvegarde du matériau importé avec les permissions.
  Les propriétés sont placées à côté de l’aperçu ; les quatre cartes
  utilisent des colonnes alignées dans une fenêtre adaptée au contenu ;
  les noms longs sont tronqués avec leur texte complet au survol.

Sources : `llmaterialeditor.cpp`, `panel_gltf_material.xml`,
`llprimitive/llgltfmaterial.cpp`, `lllocalgltfmaterials.cpp`.

## Meshes

- Collada et glTF/GLB selon les filtres de cette version Firestorm ; instances,
  matériaux, coordonnées, unités, transformations et rigging.
- Quatre LOD : fichier, génération, réutilisation ; compteurs et validation
  des faces/instances, suffixes et recherche des fichiers associés.
  La fenêtre présente l’aperçu à droite des champs nom, description et
  destination ; trois sections LOD, Collisions et Import se trouvent dessous.
  Le LOD Haut est affiché par défaut, sans second sélecteur de niveau.
  Aides, compteurs et statistiques utilisent la taille normale du texte.
  Dans Import, les options du modèle et celles de chargement occupent
  deux colonnes ; les cinq suffixes sont alignés dessous dans une grille
  adaptée à la largeur disponible. Le préréglage physique reste sur une
  ligne complète. Les commandes de caméra sont indiquées au survol du modèle.
  Chaque LOD possède un bloc encadré avec titre, source, fichier et nombre de
  triangles ; cliquer son niveau l’affiche dans l’aperçu. Modifier une source,
  un mode, un seuil ou un budget affiche aussi le LOD concerné. Le survol
  du compteur indique sa proportion de triangles par rapport à l’original.
  Le glissement des champs numériques reste continu au bord de l’écran,
  avec le curseur maintenu près du champ ; le clic permet toujours la saisie.
  Les outils de visualisation et les réglages avancés restent accessibles sans masquer
  le modèle. Le calcul des frais se trouve dans le pied de fenêtre.
  Les cases sont carrées et contrastées ; les lignes des menus conservent
  leur taille au survol et lors d’un changement de sélection.
- Génération de LOD : méthodes, triangles, erreur et normales lissées ;
  valeurs et presets de la fenêtre de référence.
  Le mode triangles utilise un seuil interne de 100 % comme Firestorm ;
  le seuil d’erreur enregistré ne s’applique qu’au mode erreur. AUTO soude
  les sommets exportés identiques dans les deux modes, comme Firestorm,
  puis retire les copies inutilisées avant simplification pour éviter
  qu’elles ne soient encore traitées comme des raccords bloqués.
  Le seuil est affiché en pourcentage de la taille du modèle ; son aide
  explique le compromis entre fidélité et simplification. Il limite la
  déformation, sans imposer un nombre de triangles ; préserver les coutures
  de normales et d’UV peut laisser presque tous les triangles, même avec un
  seuil élevé. La méthode rapide accepte davantage de simplification aux
  coutures. AUTO réduit aussi
  progressivement les raccords de normales et d’UV, puis utilise la méthode
  rapide si nécessaire, comme Firestorm lorsque le budget est dépassé de
  plus de 80 %. Les faces gardent leur emplacement de matériau.
  Le choix « Fichier » ouvre le sélecteur avant de modifier une source sans fichier ;
  annuler conserve l’ancien LOD. Charger un fichier valide ou changer
  de niveau débloque un aperçu ayant échoué, sans accepter de résultat ancien.
  Pendant le recalcul, l’ancien modèle reste affiché et l’emplacement garde
  sa hauteur ; les changements rapprochés sont regroupés avant le lancement
  du worker. Les onglets de type et de section utilisent ceux de l’inventaire.
  Un refus mesh conserve le statut HTTP, le code et les messages de validation
  fournis par le serveur, dans une zone bornée. Seuls ces champs sont lus ;
  URLs, chemins et caractères de contrôle sont retirés.
  les quatre codes de validation MAV expliqués par Firestorm sont aussi
  expliqués en français et en anglais.
  Les droits d’import mesh sont signalés séparément. Un échec après transfert garde l’avertissement
  d’issue inconnue, ses détails disponibles, et ne déclenche aucun nouvel envoi.
- Palettes glTF : articulations reconnues, joints pondérés prioritaires puis
  joints inutilisés des groupes actifs, limite de 110 comme la référence.
- Physique : fichier/LOD/préréglage, analyse convexe, simplification,
  paramètres des méthodes, progression et annulation ; limites des coques,
  sommets, triangles dégénérés et objets minces.
- Échelle/dimensions, inclusion des textures, poids des articulations,
  positions des articulations, verrouillage de l’échelle et décalage du bassin.
- Aperçu : LOD, arêtes, textures, guide UV, physique éclatée, poids,
  articulations et overrides ; caméra et couleurs enregistrées.
- Triangles de physique dégénérés : tolérance `ll_is_degenerate`, diagnostic
  et couleurs de surbrillance ; devis refusé tant que la physique brute reste
  invalide. Les paramètres d’instance gardent les LOD et coques en cache.
- Calcul serveur préalable des poids et du prix, détail du prix/streaming/
  physique, état d’autorisation du compte, remise à zéro et journal.
- Envoi de l’asset et de ses instances par le protocole mesh Firestorm.

Sources : `llfloatermodelpreview.cpp`, `llmodelpreview.cpp`,
`llmeshrepository.cpp`, `llprimitive/llmodel.cpp`,
`floater_model_preview.xml`, `menu_model_import_gear_default.xml`.

## Lots et autres imports de la référence

- Lot hétérogène : textures/sons/`.anim`/matériaux, exclusions annoncées,
  total avant confirmation, limitation 1K, progression et erreurs individuelles.
- Chargements locaux de textures, matériaux et meshes dans un même sélecteur,
  sans changer de mode ; les documents glTF/GLB sont reconnus selon leur contenu.
  Commandes communes : appliquer à la sélection, retirer de la sélection et
  placer un prim support. Les textures et matériaux s’appliquent aux faces
  choisies (ou à toutes les faces), sans modifier les données serveur ; le
  retrait restaure leur état courant. Réactualisation après modification du
  fichier et aucune facturation. Le bouton Fermer du pied de fenêtre est retiré.
  La préparation utilise le pool de staging conservé par la scène et la
  publication GPU passe explicitement par la boucle de rendu, y compris lorsque
  l’interface ne détient pas le contexte graphique. Les slots retirés sont
  libérés par cette même boucle.
- La bibliothèque locale reste enregistrée entre les lancements dans
  `local_previews.json`, au sein du profil privé du viewer. Elle conserve les
  références aux sources, leurs identifiants, les réglages de texture/PBR/LOD,
  les cartes personnalisées, la surveillance et la caméra d’aperçu ; aucun
  pixel ni mesh source n’est recopié. Les applications aux objets sont
  séparées par compte et grille et se rétablissent lorsque les objets arrivent.
  Un fichier absent reste dans la liste avec une erreur et garde ses réglages.
  « Enlever » supprime sa référence et ses applications enregistrées, sans
  effacer le fichier source. Les changements sont regroupés sur un writer
  dédié, remplacent atomiquement le fichier et sont vidés à la déconnexion
  ou fermeture normale. Une bibliothèque illisible est préservée.
  Cette persistance est une amélioration Aurora demandée par l’utilisateur.
- Le sélecteur de construction expose les textures diffuses et matériaux PBR
  locaux publiés. Choisir leur ligne applique la surcouche sans commande réseau ;
  OK conserve le lien, Annuler restaure les apparences locales précédentes,
  y compris sur une sélection mixte. Passer à un asset d’inventaire retire
  la surcouche des seules faces concernées. Ces liens suivent la même persistance
  que les commandes de la fenêtre d’import. Les sélecteurs de cartes techniques,
  sculpt et projecteur restent réservés aux assets d’inventaire.
- Mesh local : choix du modèle, unités en mètres optionnelles, dimensions
  importées, application/retrait de la sélection et prim support ; surveillance
  désactivée par défaut, période de 3 s réglable dès 0,1 s comme Firestorm.
  La préparation et l’installation des données mesh sont indépendantes du
  contexte graphique détenu temporairement par la boucle de rendu : appliquer,
  recharger ou placer un prim support fonctionne aussi depuis l’interface.
- Environnements : réglages EEP et anciens presets Windlight, imports multiples.
- Sauvegardes de linksets Firestorm : reconstruction des prims et du contenu,
  réutilisation/envoi des assets, options de textures temporaires, décalage.
  Ce parcours repose aussi sur les outils de construction et leurs accusés.
- Les imports CSV des accès parcelle et les imports AO existants restent
  des parcours distincts et doivent garder leur comportement.

Sources : `llfloaterbulkupload.cpp`, `llviewermenufile.cpp`,
`lllocalbitmaps.cpp`, `fslocalmesh*.cpp`, `llsettings*.cpp`,
`fsfloaterimport.cpp`, `floater_fs_import.xml`.

## Validation

Pour chaque ligne : règles pures et codecs testés hors ligne ; serveur HTTP
local pour négociation/envoi/confirmation/refus ; captures des fenêtres et
parcours de démo ; puis essais par l’humain des prix, droits et résultats sur
grille. Aucune connexion ni opération réelle sur grille par l’agent.

État : parcours et viewer de test validés par l’humain le 12/10/2026,
avec accord de fusion. Les codecs,
les parcours réseau HTTP/UDP, les brouillons et les aperçus GPU sont présents.
Les captures texture, son, animation, matériau, mesh et application locale ont
été examinées : aperçu à droite des champs pour le mesh et des paramètres pour le matériau,
au-dessus pour les autres types ; actions et coût accessibles
dans le pied de fenêtre. Les tests couvrent notamment
la préparation ORM, les trois modes V-HACD, les poids de LOD, les palettes glTF,
les limites d’échelle, le rechargement des dépendances et la conservation
des accusés après annulation. La validation globale du viewer ne constitue
pas un relevé détaillé des envois payants sur grille : prix, permissions,
inventaire créé et rendu des assets restent dans le protocole d’essai
ci-dessous. Aucun résultat par type n’est affirmé sans compte rendu humain.

Vérifications locales : `check.ps1` vert, 1 498 tests réussis  22 ignorés,
Clippy sans avertissement. Profilage de la scène démo : moyenne CPU de
l’interface sur les trois dernières périodes, 0,37 ms sans la fenêtre puis
0,57 ms avec l’aperçu PBR (0,51 ms animation, 0,51 ms mesh local). Les
traitements des fichiers et de la physique restent sur les workers.
Après clarification de la fenêtre mesh : captures LOD / collisions /
options d’import, application locale et libellé d’animation examinés ;
moyenne CPU de l’interface de 0,57 ms sur les trois dernières périodes
du scénario mesh. Les réglages réseau et la préparation des assets restent
identiques.

Après unification des aperçus locaux : sélection mixte des trois types,
application GPU, retrait et détection du prim support vérifiés en démo
pour les textures et matériaux ; application du mesh local vérifiée aussi.
Les tests couvrent les boutons des trois types, la détection glTF et la
restauration de l’état serveur après mise à jour d’un objet.

Les fixtures humaines ont aussi été lues et préparées hors ligne : neuf
meshes avec leurs quatre LOD et leur physique, 58 textures, une image
1024 × 1536 et un WAV de 0,3 s. Les fichiers associés sont chargés avec les
suffixes personnalisés `lowest/low/medium/high/physics`. Les triangles de
chaque LOD sont conservés et les requêtes mesh sont encodées sans envoi.
Un autre lot de 79 animations natives a été validé : octets originaux
conservés et poses calculées sans valeur non finie. Les comptes agrégés
restent distincts de `check.ps1` ; les fichiers personnels et leur inventaire
ne sont pas incorporés aux fixtures du dépôt ni aux traces publiées.

La branche a été synchronisée avec les 26 PR fusionnées depuis la PR 154,
jusqu’à la PR 181 incluse. Les imports conservent le sélecteur d’inventaire
hiérarchique, la sélection des faces invisibles, les nouvelles ancres GPU et
l’évaluation temporelle de leurs aperçus d’animation. Les tests de persistance
vérifient redémarrage, réglages des trois types, cartes personnalisées,
isolation des applications, source absente/retrouvée, suppression explicite
et sauvegarde corrompue conservée.

### Essais humains

Cliquer sur **Contenu → Importer…** ouvre la fenêtre sur **Texture**. Survoler
**Importer…** affiche les types d’assets : cliquer sur un type ouvre son
onglet et propose immédiatement de choisir les fichiers. Les onglets de la
fenêtre permettent ensuite de changer de type sans ouvrir le sélecteur.

Parcourir les onglets :

- **Textures / matériaux** : choisir un fichier, vérifier dimensions et
  alpha, modifier les cartes et facteurs PBR, faire pivoter et zoomer
  l’aperçu. Les matériaux d’inventaire utilisent glTF/GLB ; les recettes
  JSON d’un paquet d’assemblage ne sont pas des assets glTF importables.
- **Meshes** : ouvrir un HIGH, choisir ses LOD et sa physique, vérifier
  compteurs, dimensions, normales, analyse convexe et couleurs. Pour les
  associations automatiques, régler les cinq suffixes dans les préférences
  d’import avant d’ouvrir le fichier. Modifier un paramètre après un devis
  doit imposer un nouveau calcul, sans réutiliser une offre périmée.
- **Animations / sons** : lecture, arrêt et rechargement ; pour une
  animation, pause, curseur, boucle et aperçu sur notre avatar. La pose
  doit suivre le curseur immédiatement et rester fixe pendant la pause.
- **Lots / local** : vérifier le total et la confirmation, annuler les
  fichiers restants, conserver les reçus acquis. Pour un mesh local,
  appliquer puis retirer sur un objet sélectionné ; le retrait restitue
  son apparence d’origine. Tester une modification du fichier avec
  surveillance activée.

Sur la grille, l’humain se connecte puis valide prix et débit du solde,
dossiers, droits et assets réellement créés pour chaque type. Contrôler
aussi le refus serveur et la reprise du brouillon : les réglages restent
présents, avec l’avertissement de vérifier l’inventaire lorsque l’issue
d’un transfert payant est inconnue.

La physique utilise la branche libre V-HACD 4.1 de Firestorm : résolution en
nombre de voxels, limite de coques/sommets, erreur de volume et modes
Flood/Surface/Raycast. Le code natif est figé à une révision publique BSD,
avec annulation et progression. Les traitements de modèles sont parallèles
sur les workers Rust, sans multiplier les threads natifs par modèle.

Le brouillon garde au maximum 512 assets et 1 Gio de sources, avec 256 fichiers
par sélection. Le premier fichier au-delà de la limite produit une ligne
d’erreur explicite ; le reste peut être ajouté après retrait de fichiers.

Écart volontaire : un transfert payant dont l’issue est inconnue n’est pas
renvoyé automatiquement. Le brouillon reste disponible avec une demande de
vérification de l’inventaire, pour éviter un second paiement après perte de
la réponse du serveur. Les paquets d’un même transfert UDP conservent leur
politique de retransmission normale.
