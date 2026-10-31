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
- Chaque langue a son fichier (fr.md, en.md, ja.md) : ajoute l'actualité
  dans chacun. Une langue qui n'a pas de fichier affiche en.md.
- Mode d'emploi complet : HUMANS.md, « Publier une actualité ».
-->

# Actualités d'Aurora Viewer

## 2026-10-31 — Aurora 0.8.4 : tout copier, construire dans la foule
Cliquez sur un nom dans un profil pour le copier, et sélectionnez le texte d'un champ grisé pour le copier aussi : le nom d'un objet non modifiable, l'adresse média d'une parcelle. Les tenues copient les noms de leurs articles et les crédits des créateurs. Dans une sandbox bondée, les outils de construction peuvent ne sélectionner que vos objets, ou ceux que vous pouvez déplacer, et l'éditeur a une disposition Legacy. Ctrl+Maj+H vous ramène chez vous.
[En savoir plus](https://github.com/Aurora-Viewer/Aurora-Viewer/releases/tag/v0.8.4)

## 2026-10-31 — De la lumière pour les photos, LiveMesh pour Blender 3
Un bouton lumière au-dessus du déclencheur du viseur ouvre l'éclairage personnel, et le ciel, l'eau et le cycle du jour peuvent changer pour la photo seulement, ou rester après. LiveMesh synchronise les textures dans les espaces colorimétriques de Blender 3.x, et les accessoires riggés sur un kit de développement Avastar se placent comme après un upload.

## 2026-10-31 — Inventaire, Mac et Linux
Les objets reçus peuvent être acceptés automatiquement, avec un accusé de réception. Les dossiers de l'inventaire ont leurs propres commandes et une ligne de recherche à part, et les groupes de contacts du Legacy Viewer s'importent. Sur Mac, Commande fait office de Contrôle et les raccourcis s'affichent ⌘ ⌥ ⇧. Ctrl+H ouvre le chat local et Ctrl+Maj+W ferme toutes les fenêtres. Sous Linux, les liens secondlife:// s'ouvrent dans le viewer déjà lancé.

## 2026-10-31 — Plus stable
Une réinitialisation de la carte graphique ferme le viewer proprement, les photos ne le figent plus quand la mémoire vidéo manque, et la mémoire des textures reste sous contrôle dans les lieux bondés. Plus de décollage dans une parcelle interdite au vol, et la parcelle quittée ne prête plus ses règles après une téléportation. Les animesh portés tiennent droit. Merci à Gaalrauch, Marakiinfinity, circuskid, Aznlilly, DarlCat, voidel et seilimited-ship-it.

## 2026-10-29 — Aurora 0.8.3 tourne nativement sur Mac
Aurora tourne désormais nativement sur les Mac Apple Silicon, avec un rendu Metal : chaque release contient une application macOS à glisser dans Applications. Les pages web et les vidéos du monde y fonctionnent aussi : la première fois qu'il en a besoin, le viewer télécharge lui-même ses plugins média. Le guide d'installation est dans docs/MACOS.md.
[En savoir plus](https://github.com/Aurora-Viewer/Aurora-Viewer/releases/tag/v0.8.3)

## 2026-10-29 — Les messages importants, sous vos yeux
Un message qu'un gérant de domaine envoie à la région s'affiche maintenant en carte au milieu de l'écran, comme une offre d'amitié, et reste sous la cloche et dans le chat local. Une notice de groupe montre son groupe, son insigne et sa pièce jointe, prête à enregistrer. Les L$ que vous payez à un résident apparaissent dans votre conversation avec lui, comme ceux qu'il vous paie, et y restent quand vous la rouvrez.

## 2026-10-29 — Pour les photos et les créateurs
Dans le viseur photo, Maj + clic garde nets tous les avatars d'une photo de groupe, vos HUD répondent aux clics, et le bouton Poseur pose l'avatar par-dessus la prise de vue. Le poseur fonctionne maintenant avec toutes les formes, demande avant qu'une fermeture perde la pose, et annule avec Ctrl+Z. LiveMesh prévisualise une animation sur un modèle local enregistré, queues et ailes Bento comprises.

## 2026-10-29 — Inventaire, graphismes et Linux
Survolez un objet de l'inventaire pour voir son image, et construisez une recherche avancée à partir de règles visuelles, enregistrée pour plus tard. Enregistrez vos réglages graphiques en profils nommés. Sous Linux, le son passe par PulseAudio ou PipeWire, les notifications viennent du bureau, et le dock montre le nouveau loup hurlant d'Aurora. Merci à circuskid pour Linux, à logue pour la relecture japonaise native, et à voidel, notre nouveau contributeur.

## 2026-10-29 — Vos idées, sur le tableau des suggestions
Les demandes de fonctionnalités vivent désormais sur notre tableau des suggestions : proposez une idée, votez pour celles que vous voulez le plus et suivez leur avancement. Les demandes faites sur GitHub y ont été déplacées.
[Ouvrir le tableau des suggestions](https://feedback.aurora-viewer.org)

## 2026-10-29 — Aurora 0.8.2 parle japonais, 日本語
Aurora existe désormais en japonais : choisissez 日本語 dans Préférences › Interface › Langue, ou laissez la langue du système le choisir sur un Windows japonais. Chaque message est traduit avec les termes japonais officiels de Second Life, l'interface s'affiche avec des polices japonaises et les dates se lisent à la japonaise.
[En savoir plus](https://github.com/Aurora-Viewer/Aurora-Viewer/releases/tag/v0.8.2)

## 2026-10-29 — Terrain PBR, et l'interrupteur HDR
Les régions dont le sol utilise des matériaux PBR s'affichent désormais avec eux au lieu d'un gris uni, comme dans le Legacy Viewer, et les gérants de domaine voient un changement de revêtement sur le terrain avant de l'appliquer. Préférences › Graphismes › Image reçoit aussi l'interrupteur HDR et émissif du Legacy Viewer, activé par défaut, pour qui préfère l'ancien rendu.

## 2026-10-29 — Chat et plaques de nom
Ctrl+Maj+Entrée commence une nouvelle ligne dans un message, et les paragraphes collés gardent leur forme. Les erreurs de script de vos objets s'affichent en rouge dans le chat local au lieu d'ouvrir une fenêtre. Trois points sautillent sur la photo d'un résident qui vous écrit, les plaques de vos amis prennent la couleur des amis, et un clic droit sur votre solde L$ le masque pendant un stream.

## 2026-10-29 — Pour les créateurs et les tenues
Les permissions par défaut, en bas du menu Nouveau de l'inventaire, s'appliquent à ce que vous créez et importez. Glissez vos calques de tatouage, d'alpha et universels dans l'ordre voulu, déposez des textures de l'inventaire sur les vignettes des éditeurs d'apparence et de matériaux, et agrandissez les poignées d'édition. Les envois de textures 2K refonctionnent, et Apparence › Portée liste ce qu'un script vous a attaché pour un temps.

## 2026-10-29 — Aussi dans la 0.8.2
Le texte peut maintenant doubler de taille, dans Préférences › Apparence. Maintenez la flèche Retour de la barre du haut pour lister tous les lieux de la session. La disposition des fenêtres est enregistrée peu après chaque changement, même si le viewer plante ; les tenues enregistrées se suppriment de nouveau, les véhicules traversent les collisions en douceur, les clickers dans un corps mesh répondent à chaque clic, et le vent ne grésille plus. Merci à circuskid, notre nouveau contributeur : sous Linux, Aurora ne passe plus au premier plan à chaque message.

## 2026-10-27 — Aurora 0.8.1 : les avatars sortent plus vite du nuage
Les téléchargements suivent votre connexion : dans une région chargée, les avatars sortent du nuage environ trois fois plus vite, déjà texturés, et la région se charge deux fois plus vite avec un cache vide. Votre tenue arrive complète : chaque attachement est porté à la connexion et à chaque changement de tenue.
[En savoir plus](https://github.com/Aurora-Viewer/Aurora-Viewer/releases/tag/v0.8.1)

## 2026-10-27 — Figez le monde, gardez vos prises de vue
Dans le mode photo, figez le monde : chaque avatar, animation et objet en mouvement s'immobilise pendant que votre caméra reste libre. Enregistrez toute une prise de vue en préréglage de caméra (sa position, l'objectif, les filtres et la lumière) et retrouvez-la en un clic. Le déclencheur s'entend désormais autour de vous, comme dans le Legacy Viewer, sauf si vous choisissez les photos discrètes.

## 2026-10-27 — Pour les créateurs
Construire › Modifier les particules façonne un effet de particules sur votre prim, en direct, et y place le script correspondant. Monde › Explorateur d'animations montre qui joue une animation sur vous, la prévisualise, et l'arrête ou la bloque. Et les menus du clic droit des objets, du terrain, des avatars, des attachements et des HUD s'organisent maintenant à votre goût.

## 2026-10-27 — Aussi dans la 0.8.1
Maintenez Alt en vue subjective pour libérer le curseur, et choisissez votre réticule. L'arabe et l'hébreu se lisent désormais de droite à gauche, et un paiement en L$ s'affiche en carte dans la conversation de celui qui paie. Rangez vos tenues dans des dossiers depuis l'Apparence. Le mode immersif peut masquer vos HUD, et Alt+Maj+H les montre ou les masque partout. Avec RLVa activé, le nombre d'images par seconde ne chute plus, les souris gamer restent connectées, et Linux se ferme proprement sous Wayland.

## 2026-10-26 — Aurora 0.8.0 : des filtres et des effets pour vos photos
Le mode photo gagne 66 filtres et effets visibles en direct dans le viseur : les 30 filtres du Legacy Viewer et 36 effets propres à Aurora, du grain argentique à la VHS, plus vos propres LUT .cube. Empilez-en jusqu'à six, chacun avec ses réglages, et comparez avant et après. Isolez votre avatar, ou les avatars et objets que vous cliquez, sur fond transparent : un PNG prêt à l'emploi.
[En savoir plus](https://github.com/Aurora-Viewer/Aurora-Viewer/releases/tag/v0.8.0)

## 2026-10-26 — Des portails, et seulement votre parcelle
Ouvrez un portail vers un lieu, ou déposez un repère dans le monde, et traversez-le pour vous téléporter. Les autres utilisateurs d'Aurora le voient aussi : vos amis par défaut. Vous pouvez aussi masquer ce qui se trouve hors de la parcelle actuelle, et le mode immersif cache toute l'interface sauf le chat, pour les photos et les événements.

## 2026-10-26 — Un chat qui écrit avec vous
Un correcteur orthographique avec suggestions, le remplacement automatique au choix, @ pour mentionner quelqu'un, et / dans le chat local pour trouver un geste. Une vignette par conversation attend à côté de la barre de chat, et le chat local peut s'afficher en bulles au-dessus des têtes. Les posts Bluesky et X, les vidéos courtes et les WebP animés s'affichent désormais en aperçu dans la conversation.

## 2026-10-26 — Inventaire et tenues
Tout votre inventaire se charge en arrière-plan dès la connexion. Ouvrez une seconde fenêtre d'inventaire pour trier entre deux dossiers, redimensionnez ses colonnes, et donnez des objets en les déposant sur un profil ou un avatar. Copiez le contenu d'une boîte dans votre inventaire ou portez-le directement, et modifiez votre peau, vos cheveux, vos yeux, vos tatouages, vos alphas et vos vêtements.

## 2026-10-26 — Aussi dans la 0.8.0
Le Sound Explorer trouve quel objet joue un son, Quickjump supprime le délai du saut et de l'atterrissage, et Aide › État de la grille montre les incidents de la grille. Les étiquettes de nom signalent l'équipe Aurora et le personnel de Linden Lab, et le viewer reste dans la mémoire vidéo que votre carte graphique lui accorde.

## 2026-10-25 — Aurora 0.7.5 : des outils pour les créateurs
LiveMesh montre votre mesh Blender dans le viewer pendant que vous le modifiez, sans rien envoyer : les mesh riggés sur votre avatar, les mesh statiques et les linksets comme objets locaux à poser, déplacer ou porter. Le nouveau Poser pose votre avatar, vos animesh ou les avatars proches qui l'acceptent, os par os, pour vos photos, avec une bibliothèque de poses et l'export BVH.
[En savoir plus](https://github.com/Aurora-Viewer/Aurora-Viewer/blob/main/docs/BLENDER_LIVE.md)

## 2026-10-25 — Retrouvez n'importe quel objet autour de vous
La recherche de zone, dans le menu Monde ou en bouton de barre d'outils, liste les objets posés autour de vous et les filtre pendant que vous tapez : nom, description, propriétaire, créateur, groupe ou UUID, la parcelle actuelle ou les régions voisines, à vendre, physiques, fantômes, temporaires ou selon la distance. Triez les résultats, puis inspectez un objet, zoomez dessus ou copiez son UUID.

## 2026-10-25 — Des conversations à votre façon
Détachez une conversation dans sa propre fenêtre, ou faites-la simplement glisser hors de la liste. Les liens vers Gyazo, Flickr, Imgur, Lightshot et Reddit affichent leur image, le lecteur YouTube garde la forme de chaque vidéo, et un sélecteur de GIF apparaît à côté des émojis dès que vous ajoutez votre propre clé gratuite. Les messages des objets peuvent tenir sur une petite ligne discrète.

## 2026-10-25 — Plus confortable pour les yeux
Une nouvelle page Accessibilité réunit les réglages qui aident, avec un bouton sur l'écran de connexion. Essayez la palette à contraste élevé, la police OpenDyslexic, les curseurs de volume perceptifs, ou choisissez vos propres couleurs d'accent, de fond et de texte.

## 2026-10-25 — Corrigé grâce à vos retours
Les PC portables à deux cartes graphiques utilisent maintenant la carte dédiée, Overwolf n'empêche plus le viewer de démarrer, les cheveux en couches ne sont plus transparents et les gestes atteignent vos HUD sur les canaux négatifs. Votre avatar se lève quand vous vous téléportez depuis un siège, et la fenêtre Apparence reste fluide avec des milliers de tenues. Se reconnecter juste après une déconnexion ou un plantage affiche maintenant un court compte à rebours au lieu de bloquer.

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
