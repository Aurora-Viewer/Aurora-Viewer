# Imports d’assets — référence Firestorm

Cette matrice suit les imports depuis le disque et leurs aperçus locaux.
Elle sert à vérifier les options, les valeurs par défaut, le protocole et
les erreurs, avant la validation humaine sur grille. Une fenêtre affichée
ne suffit pas à considérer son import comme terminé.

Périmètre confirmé le 12/10/2026 : textures, matériaux, meshes, sons,
animations, lots et aperçus locaux. Les imports de linksets, EEP et
Windlight sont recensés pour des tâches ultérieures dans `TASKS.md`.

## Parcours commun

- Choisir un ou plusieurs fichiers ; nom et description ; dossier de
  destination par type, y compris les cinq dossiers personnalisés existants.
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
- Texture temporaire gratuite seulement sur une région sans server-side
  baking et si le chargement normal est payant.

Sources : `llfloaterimagepreview.cpp`, `floater_image_preview.xml`,
`llviewertexturelist.cpp`, `llimageraw.cpp`.

## Sons

- WAV : restrictions PCM, canaux, fréquence, profondeur et durée ; conversion
  vers le son Vorbis de Second Life, avec les paramètres Firestorm.
- Nom/description, tarif et destination ; aperçu sonore local.

Sources : `llfloaternamedesc.cpp`, `llaudio/llvorbisencode.cpp`,
`floater_sound_preview.xml`.

## Animations

- `.anim` : validation sans altération des données, notamment des contraintes.
- BVH : hiérarchie, canaux et ordre des axes, pose de référence, traduction
  des articulations, suppression des pistes fixes, réduction des clés.
- Priorité 2 par défaut ; boucle désactivée ; entrée/sortie en pourcentage
  ou en images ; fondus de 0,3 s ; poses des mains et expressions.
- Aperçu local : debout/marche/assis/vol, lecture/pause/arrêt, curseur temporel,
  caméra, rechargement du fichier, statistiques et erreurs de parsing.
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
- Textures externes/embarquées/locales ou UUID d’inventaire ; résolution
  maximale, suppression/remplacement, coût détaillé des textures.
- Réutiliser les textures partagées ; envoyer les textures avant le matériau,
  puis remplacer leurs références par les UUID confirmés.
- Matériau sans frais propres ; création et sauvegarde comme un asset
  `material`, enveloppe LLSD version 1.1 / `GLTF 2.0`.
- Aperçu local et sauvegarde du matériau importé avec les permissions.

Sources : `llmaterialeditor.cpp`, `panel_gltf_material.xml`,
`llprimitive/llgltfmaterial.cpp`, `lllocalgltfmaterials.cpp`.

## Meshes

- Collada et glTF/GLB selon les filtres de cette version Firestorm ; instances,
  matériaux, coordonnées, unités, transformations et rigging.
- Quatre LOD : fichier, génération, réutilisation ; compteurs et validation
  des faces/instances, suffixes et recherche des fichiers associés.
- Génération de LOD : méthodes, triangles, erreur et normales lissées ;
  valeurs et presets de la fenêtre de référence.
- Physique : fichier/LOD/préréglage, analyse convexe, simplification,
  paramètres des méthodes, progression et annulation ; limites des coques,
  sommets, triangles dégénérés et objets minces.
- Échelle/dimensions, inclusion des textures, poids des articulations,
  positions des articulations, verrouillage de l’échelle et décalage du bassin.
- Aperçu : LOD, arêtes, textures, guide UV, physique éclatée, poids,
  articulations et overrides ; caméra et couleurs enregistrées.
- Calcul serveur préalable des poids et du prix, détail du prix/streaming/
  physique, état d’autorisation du compte, remise à zéro et journal.
- Envoi de l’asset et de ses instances par le protocole mesh Firestorm.

Sources : `llfloatermodelpreview.cpp`, `llmodelpreview.cpp`,
`llmeshrepository.cpp`, `llprimitive/llmodel.cpp`,
`floater_model_preview.xml`, `menu_model_import_gear_default.xml`.

## Lots et autres imports de la référence

- Lot hétérogène : textures/sons/`.anim`/matériaux, exclusions annoncées,
  total avant confirmation, limitation 1K, progression et erreurs individuelles.
- Chargements locaux de textures, matériaux et meshes : réactualisation
  après modification du fichier et aucune facturation.
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

État : implémentation en cours dans le worktree `import-assets`. Les codecs,
le parcours réseau moderne, les brouillons, les aperçus GPU et les premières
captures sont présents ; le bilan final de parité et les essais de grille
restent à réaliser. Aucun import réel n’a été validé sur une grille.

Écart volontaire : un transfert payant dont l’issue est inconnue n’est pas
renvoyé automatiquement. Le brouillon reste disponible avec une demande de
vérification de l’inventaire, pour éviter un second paiement après perte de
la réponse du serveur. Les paquets d’un même transfert UDP conservent leur
politique de retransmission normale.
