<!--
Actualités de l'écran de connexion d'Aurora Viewer, en français.

Le viewer lit ce fichier sur la branche main à chaque démarrage : une fois la
modification fusionnée, l'actualité apparaît chez tout le monde au lancement
suivant (GitHub met quelques minutes à servir la nouvelle version du
fichier), sans publier de nouvelle version du viewer.

Une actualité = un titre « ## », la plus récente en premier :

## 2026-10-08 — Titre de l'actualité
Une ou plusieurs lignes de texte simple (ni gras, ni liste, ni image).
Une ligne vide commence un nouveau paragraphe.
[En savoir plus](https://exemple.org/lien-facultatif)

- La date s'écrit AAAA-MM-JJ, suivie de « — », « - » ou « : ». Elle est
  facultative, comme le lien.
- Le lien doit commencer par https:// ; un clic sur l'actualité l'ouvre.
  Un seul lien par actualité.
- 20 actualités au plus sont affichées ; le titre est coupé à 120
  caractères, le texte à 600.
- Chaque langue a son fichier (fr.md, en.md) : ajoute l'actualité dans les
  deux. Une langue qui n'a pas de fichier affiche en.md.
- Mode d'emploi complet : HUMANS.md, « Publier une actualité ».
-->

# Actualités d'Aurora Viewer

## 2026-10-25 — Aurora 0.7.5 : des outils pour les créateurs
LiveMesh montre votre mesh Blender dans le viewer pendant que vous le modifiez, sans rien envoyer : les mesh riggés sur votre avatar, les mesh statiques et les linksets comme objets locaux à poser, déplacer ou porter. Le nouveau Poser pose votre avatar, vos animesh ou les avatars proches qui l'acceptent, os par os, pour vos photos, avec une bibliothèque de poses et l'export BVH.
[En savoir plus](https://github.com/Aurora-Viewer/Aurora-Viewer/blob/main/docs/BLENDER_LIVE.md)

## 2026-10-25 — Des conversations à votre façon
Détachez une conversation dans sa propre fenêtre, ou faites-la simplement glisser hors de la liste. Les liens vers Gyazo, Flickr, Imgur, Lightshot et Reddit affichent leur image, le lecteur YouTube garde la forme de chaque vidéo, et un sélecteur de GIF apparaît à côté des émojis dès que vous ajoutez votre propre clé gratuite. Les messages des objets peuvent tenir sur une petite ligne discrète.

## 2026-10-25 — Plus confortable pour les yeux
Une nouvelle page Accessibilité réunit les réglages qui aident, avec un bouton sur l'écran de connexion. Essayez la palette à contraste élevé, la police OpenDyslexic, les curseurs de volume perceptifs, ou choisissez vos propres couleurs d'accent, de fond et de texte.

## 2026-10-25 — Corrigé grâce à vos retours
Les PC portables à deux cartes graphiques utilisent maintenant la carte dédiée, Overwolf n'empêche plus le viewer de démarrer, les cheveux en couches ne sont plus transparents et les gestes atteignent vos HUD sur les canaux négatifs. Votre avatar se lève quand vous vous téléportez depuis un siège, et la fenêtre Apparence reste fluide avec des milliers de tenues.

## 2026-10-22 — RLVa, quand vous le voulez
Depuis la version 0.7.4, Aurora comprend RLVa : les restrictions, exceptions et commandes dont se servent les tenues scriptées, les meubles et les expériences. Il reste éteint tant que vous ne l'activez pas dans Préférences › Protections, et l'éteindre libère tout d'un coup.
[En savoir plus](https://github.com/Aurora-Viewer/Aurora-Viewer/blob/main/docs/RLVA.md)

## 2026-10-22 — Aurora sur Linux
Chaque version est désormais publiée aussi pour Linux, avec son guide d'installation.
[Guide d'installation](https://github.com/Aurora-Viewer/Aurora-Viewer/blob/main/docs/LINUX.md)

## 2026-10-22 — Vos réglages à l'abri
Sauvegardez vos réglages dans un fichier et restaurez-les en un clic. Après un plantage, la fenêtre de plantage peut envoyer son rapport sous forme de ticket GitHub. Et vos jeux d'AO d'Alchemy s'importent désormais dans l'AO intégré.

## 2026-10-21 — Des conversations plus vivantes
Les images et les vidéos YouTube s'affichent en aperçu sous leurs liens, les messages faits seulement d'émojis s'affichent en grand, et un bouton vous ramène aux derniers messages. Les alertes de mots-clés vous préviennent quand on parle de ce qui compte pour vous. Alt + flèches passent d'une conversation à l'autre, Ctrl+W les ferme.

## 2026-10-21 — Des préférences où l'on cherche
Les préférences sont désormais rangées en sections et en onglets, avec une recherche dans tous les réglages. Aussi au programme : le suivi des L$ reçus et payés, des infobulles sur les objets, les avatars et le terrain, un filtre média qui demande avant qu'un nouveau site ne joue, et des protections anti-spam au choix.

## 2026-10-20 — Un mode photo comme un vrai appareil
Le mode photo s'ouvre en plein écran sous un viseur : un autofocus qui trouve les visages, des bagues pour la focale, l'ouverture et l'exposition, le focus peaking, un histogramme en direct, une manette de caméra et un retardateur. Les photos sont en 4K par défaut, jusqu'en 8K.

## 2026-10-19 — Lieux, inventaire et recherche redessinés
Lieux réunit vos favoris, repères et historique dans une seule liste avec un aperçu. L'inventaire garde son arbre habituel et s'élargit en explorateur. La recherche trouve résidents, lieux, événements, groupes et petites annonces depuis un seul champ. Chaque fenêtre garde sa version Legacy dans Préférences › Aurora.

## 2026-10-19 — Des avatars plus vivants
La physique du corps, les expressions du visage, les poses des mains et la respiration venues des animations, et le placement des pieds sur le terrain au choix. Aussi au programme : un moniteur de ressources pour les utilisateurs avancés, et des effets d'eau expérimentaux avec vagues, écume et caustiques.

## 2026-10-17 — Une lumière qui ancre chaque scène
L'occlusion ambiante GTAO, les ombres de contact du soleil et l'anticrénelage spéculaire arrivent, avec des niveaux graphiques qui les adaptent à votre machine. Les propriétaires de région ont la fenêtre Région / Domaine complète, en version Aurora ou avec les huit onglets classiques.

## 2026-10-08 — Bienvenue dans Aurora Viewer
Un client Second Life moderne en Rust et Vulkan : rendu PBR, ombres, eau et reflets, interface Aurora entièrement personnalisable.

## 2026-10-08 — Voix WebRTC et audio
Chat vocal des régions, push-to-talk, point de voix animé dans les étiquettes, musique de parcelle et réglage fin des volumes.

## 2026-10-08 — Une interface qui vous ressemble
Raccourcis clavier et souris modifiables, disposition des fenêtres retenue, notifications, SLURL dans la barre de navigation.
