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

## 2026-11-12 — Aurora 0.9.8 : des correctifs pour la construction, le chat et les cartes AMD
Choisir une face sélectionne de nouveau les faces de ce que vous portez, et Faire disparaître avec Modifier les parties liées ne cache que la partie choisie. Le sélecteur de canal s'affiche près de chaque saisie du chat local et garde le canal choisi. Sur les cartes AMD, les feuilles des arbres ne sont plus des carrés et les cheveux mesh n'ont plus de traînées, et les cils en fondu restent visibles derrière les cheveux mesh. Le cache est vidé une fois au premier lancement et se remplit tout seul ; vos réglages sont gardés.
[En savoir plus](https://github.com/Aurora-Viewer/Aurora-Viewer/releases/tag/v0.9.8)

## 2026-11-12 — Démarrage, voix et poser
Une petite fenêtre Aurora s'affiche pendant le démarrage et après le redémarrage d'une mise à jour. La voix locale se connecte sur les liaisons lentes, les boutons invisibles des HUD répondent de nouveau au clic gauche, et l'avis de complexité de l'avatar ne se répète plus (désactivé par défaut, Préférences › Graphismes › Avatars › Avis de complexité). Dans le poser, Ctrl enfoncé surligne la partie des meshes de l'avatar que Ctrl + clic choisira. Merci à Bruica, iGao, KanikaBreed, Nowalski et Odessa-Draekavik.

## 2026-11-12 — Aurora 0.9.7 : la fenêtre Construction revue, la récupération de scripts et Destinations
La fenêtre Construction est réorganisée : la sélection tient dans un en-tête repliable, les options forment des colonnes au-dessus des onglets, à ranger avec Organiser les options…, position, taille et rotation gardent toute la largeur, et Contenu a son menu d'affichage. Après un plantage ou une connexion perdue, la Récupération de scripts rend vos modifications non enregistrées à la connexion suivante, dans Objets perdus et trouvés. Destinations ouvre la fenêtre d'Aurora sur le Guide des destinations : recherche, catégories, notes et téléport en un clic. Cache et réglages sont gardés.
[En savoir plus](https://github.com/Aurora-Viewer/Aurora-Viewer/releases/tag/v0.9.7)

## 2026-11-12 — Poser, apparence et avatars
Le poser gagne une fenêtre compacte qui laisse la scène libre, Maj + clic pour bouger plusieurs articulations ensemble, et un anneau qui tourne une articulation autour de l'axe de la caméra. Exporter XML, à côté d'Importer XML dans les éditeurs de silhouette, de physique et de vêtements, enregistre un fichier d'apparence, et le menu ⚙ d'Apparence exporte toute votre tenue. Votre avatar joue les animations de parole, de cri et de chuchotement dans le chat local, Recharger l'avatar répare un avatar qui s'affiche mal, et les couches mouillées et brillantes des corps mesh s'affichent.

## 2026-11-12 — Encore plus proche du Legacy Viewer, et des correctifs
Dialogues de script par objet garde un seul menu par objet au lieu d'une pile. Avancé › Libérer les touches rend les contrôles qu'un véhicule ou un HUD cassé retient, et un téléport en cours peut s'annuler. Les notices de groupe s'affichent aussi dans le chat du groupe, Navigateur préféré choisit où s'ouvrent les liens, et la Voie lactée est plus discrète, avec son curseur. Sur OpenSim, Demander le statut d'administrateur donne les pouvoirs d'administrateur. Merci à anne-skydancer, Bruica, circuskid, Gaalrauch, iGao, KanikaBreed, Marakiinfinity, Nowalski et Odessa-Draekavik.

## 2026-11-11 — Aurora 0.9.6 : la vitrine des tenues, les portraits auto et 65 looks photo
La Galerie des tenues d'Apparence devient une vitrine : de grandes photos de vos tenues, un rail pour les favoris, les tenues sans photo et les dossiers, et on porte en un clic (les petites vignettes restent dans Préférences › Aurora). Portrait auto vous cadre en studio pour la photo gratuite de la tenue, Faire les portraits les fait toutes d'un coup, et une Photo HD en option coûte le prix d'envoi de la grille. L'onglet LUT du mode photo compte 20 looks de Czar Fenris et 45 de Nanner Pudding, merci à vous deux ! Cache et réglages sont gardés.
[En savoir plus](https://github.com/Aurora-Viewer/Aurora-Viewer/releases/tag/v0.9.6)

## 2026-11-11 — La Voie lactée des vraies étoiles, et les Types de rendu
La Voie lactée est faite des 2,5 millions d'étoiles du catalogue Tycho-2, avec ses nuages d'étoiles et ses bandes sombres, et l'Heure des étoiles peut suivre la vraie horloge. Avancé › Types de rendu masque le terrain, l'eau, les objets, les avatars ou les particules pour voir derrière, comme le Legacy Viewer. Un Mac démarre à la résolution logique de l'écran en qualité Mid-High, les cartes AMD RX 9000 gardent leurs ombres de contact, et les médias des HUD passent en premier dans les régions chargées.

## 2026-11-11 — Poser, construction, inventaire et correctifs
Maintenez Espace pour freiner votre avatar, comme le Legacy Viewer. Le poser gagne Enregistrer sous… et montre où va une pose. Les poignées de déplacement se saisissent mieux, les sélections de terrain s'alignent sur la grille des parcelles, et le glTF garde ses cartes 2K. Favoris liste les éléments étoilés sans leurs dossiers, et la confirmation de suppression peut se couper. Région / Domaine confirme tout changement qui renvoie des gens chez eux. Merci à anne-skydancer, circuskid, emmelinedevere, Gaalrauch, KanikaBreed, LisaScheers, Marakiinfinity, Memekiller, Nowalski et Odessa-Draekavik.

## 2026-11-10 — Aurora 0.9.5 : le Suivi de région, les marqueurs des cercles et l'apparence en XML
Monde › Suivi de région surveille une liste de régions, comme le Legacy Viewer : si chacune est en ligne et combien d'avatars s'y trouvent, sans ouvrir la carte ; Suivre la région, sur la Carte du monde, en ajoute une. Un cercle de contacts peut marquer ses membres dans le monde, à travers les murs et au bord de l'écran : réglez son Marqueur dans le monde dans Personnes › Cercles (désactivé par défaut). Importer XML, dans les éditeurs de silhouette, de physique et de vêtements, charge un fichier d'apparence d'un site de silhouettes ou d'un autre viewer. Cache et réglages sont gardés.
[En savoir plus](https://github.com/Aurora-Viewer/Aurora-Viewer/releases/tag/v0.9.5)

## 2026-11-10 — Emojis, chat et L$
Vos emojis récents forment une rangée près du champ de chat, et le sélecteur de GIF montre ceux que vous avez envoyés. Activez Emojis favoris dans Préférences › Chat pour garder les vôtres en tête, avec un onglet étoile dans le sélecteur. Alt+Entrée envoie du chat ((OOC)), et Demander une téléportation s'ajoute à Proposer une téléportation dans la conversation. Un bouton replie ou déplie la liste des conversations. Les L$ reçus ou dépensés s'affichent sous votre solde, et un résident qui vous paie affiche une notification.

## 2026-11-10 — Votre vue, le monde et des correctifs
Préférences › Graphismes peut masquer les lumières portées par les autres avatars en gardant les vôtres, et Monde › Masquer les étiquettes des avatars les cache toutes en un clic. Les infobulles des avatars peuvent rester sans celles du monde, et la question du filtre média devient un bandeau qui laisse voler. Le sol reste aux passages de région, une mémoire vidéo pleine ne fige plus le monde, et les radios de parcelle tiennent aux téléportations. Merci à circuskid, ClifHofmann, dash-sl, Gaalrauch, iGao, KanikaBreed, Nowalski, Odessa-Draekavik et RheaMercury.

## 2026-11-08 — Aurora 0.9.4 : les Paramètres de débogage, et plus d'options du Legacy Viewer
Le bouton Paramètres de débogage, ou Ctrl+Alt+Maj+S, ouvre une fenêtre avec chaque réglage par son nom : une recherche, une étoile sur ceux que vous avez changés et Rétablir la valeur par défaut. Préférences › Construire règle ce que devient une nouvelle prim : couleur, transparence, lueur, lumineux, physique, temporaire ou fantôme. Préférences › Graphismes gagne une distance d'affichage progressive (désactivée par défaut), le détail des Arbres et les Particules des objets portés, et Préférences › Protections peut masquer les objets à la surface démesurée. Cache et réglages sont gardés.
[En savoir plus](https://github.com/Aurora-Viewer/Aurora-Viewer/releases/tag/v0.9.4)

## 2026-11-08 — Les régions voisines, des objets qui restent visibles, des appels qui tiennent
Aurora demande désormais à la région votre propre distance d'affichage : les régions voisines apparaissent à 256 m et au-delà, comme dans le Legacy Viewer. Les objets bien en vue ne disparaissent plus sous certains angles de caméra, et le monde revient de lui-même quand la mémoire vidéo était pleine à la connexion. Les écrans média des régions chargées se remplissent et suivent leurs télécommandes, et les textures restées blanches reviennent sans vous reconnecter. Les appels privés tiennent pendant les téléportations, et la musique de la parcelle ne coupe plus quand vous bougez la caméra.

## 2026-11-08 — Chat, caméra, avatars et correctifs
Le lissage de caméra va jusqu'à 300 sans à-coups, et Préférences › Chat règle le nombre de lignes au-dessus du chat. Ctrl+Tab change de conversation, l'IME japonais montre le segment en conversion, et les zones de texte des scripts prennent plusieurs lignes. Les changements d'À propos du terrain tiennent, l'AO ne laisse plus de pose assise après une téléportation, et les outils RLVa ont un onglet Exceptions. Merci à AiAiCaptainn, anne-skydancer, circuskid, ClifHofmann, emmelinedevere, Gaalrauch, iGao, KanikaBreed, Marakiinfinity, MilkIsPrettyGood, Nowalski, Odessa-Draekavik et RheaMercury.

## 2026-11-06 — Aurora 0.9.3 : AMD FSR 3.1, et plus stable sur toutes les cartes graphiques
Choisissez FSR 3.1 sous Anticrénelage dans Préférences › Graphismes : des bords lissés à la taille de la fenêtre, ou un monde dessiné plus petit puis reconstruit net, pour plus d'images par seconde. Après une réinitialisation de la carte graphique, Aurora rouvre sa fenêtre sans vous déconnecter (désactivable dans les Préférences). Les GPU intégrés Intel de 6e à 9e génération démarrent de nouveau, comme les portables AMD + NVIDIA ; les GPU intégrés comptent leur mémoire partagée dans le budget des textures. Le cache est vidé une fois au premier lancement de la 0.9.3 ; vos réglages sont gardés.
[En savoir plus](https://github.com/Aurora-Viewer/Aurora-Viewer/releases/tag/v0.9.3)

## 2026-11-06 — Mises à jour, votre ordinateur, et des clics qui tombent juste
Aurora vous prévient désormais qu'une nouvelle version est sortie, pendant qu'il tourne. Préférences › Langue et mises à jour › Retirer Aurora de cet ordinateur enlève ce qu'Aurora a inscrit hors de ses propres dossiers, puis liste les dossiers restants. Le pack web et médias ne se télécharge qu'après votre accord. Les clics traversent les textures alpha et les prims invisibles comme dans le Legacy Viewer, et Maj ou Ctrl + clic droit étend la sélection de construction.

## 2026-11-06 — Caméra, inventaire et correctifs
Les limites de la caméra sont de nouveau actives par défaut, comme dans le Legacy Viewer : une carte à la connexion indique où les désactiver. L'historique des téléportations survit à un plantage, et ni l'écriture du journal ni la coupure de la voix ne peuvent figer le viewer. Les bords du terrain rejoignent la région voisine, les photos de profil restent nettes, les Favoris listent les éléments étoilés et le survol de l'inventaire a un interrupteur. Merci à ClifHofmann, chrzang, circuskid, emmelinedevere, Gaalrauch, iGao, KanikaBreed, Marakiinfinity, Odessa-Draekavik et xWynt3r.

## 2026-11-05 — Aurora 0.9.2 : votre historique de chat vous suit, et un mode plein écran
Préférences › Chat › Historique importe vos conversations du Legacy Viewer et des autres viewers approuvés qui écrivent les mêmes fichiers. Les messages rejoignent votre historique Aurora dans l'ordre, chaque connexion n'ajoute que les nouveaux, et les fichiers de l'autre viewer sont seulement lus. Préférences › Graphismes gagne un plein écran sans bordure, et Alt+Entrée l'active ou le coupe aussitôt, sans redémarrer : une image nette pour les vidéos, et la fenêtre revient telle qu'elle était.
[En savoir plus](https://github.com/Aurora-Viewer/Aurora-Viewer/releases/tag/v0.9.2)

## 2026-11-05 — Mouvements plus nets, clics plus légers
Avec le TAA, les avatars, animesh et objets qui bougent ne bavent plus : l'anticrénelage suit le mouvement propre de chaque pixel. L'occlusion ambiante passe aussi en demi-résolution dans une fenêtre en 1440p, soit environ 14 % d'images par seconde en plus dans une foule, pour la même image. Dans les régions chargées, un clic sur le monde coûte dix fois moins, et le curseur de survol ignore les maillages riggés portés qu'il ne peut jamais viser. Les escalators et autres objets en boucle keyframed bouclent sans glisser en arrière.

## 2026-11-05 — Construction, carte graphique et correctifs
La pipette de texture et les textures locales reviennent dans les éditeurs de construction et d'apparence, les dépôts sur une face clignotent et atteignent les faces transparentes, et la fenêtre de construction Legacy montre le numéro de lien et de face. La capture de jeu d'OBS Studio reste à l'écart sauf overlays autorisés, une fenêtre derrière met sa 3D en pause quand un autre programme occupe la carte, et Qualité photo se coupe après un plantage. Plus de triangles étirés sur les cheveux et les plantes. Merci à circuskid, Gaalrauch, KanikaBreed, Marakiinfinity et Odessa-Draekavik.

## 2026-11-04 — Aurora 0.9.1 : connexions, téléportations et tenues plus sûres
Une région lente à confirmer votre arrivée ne termine plus la session, et un code de double authentification refusé ne bloque plus le champ du code à la connexion. Les changements de tenue envoient une demande d'apparence à la fois, pour la version la plus récente : les produits et RLVa qui changent vite la tenue ne noient plus votre apparence sous les mises à jour ratées. Une touche jamais vue enfoncée ne fait plus sauter ni voler l'avatar après un changement de fenêtre. Les ciels Windlight importés s'enregistrent dans l'inventaire.
[En savoir plus](https://github.com/Aurora-Viewer/Aurora-Viewer/releases/tag/v0.9.1)

## 2026-11-04 — Pellicule photo, ombres des projecteurs, lieux chargés plus fluides
Le mode photo garde les 10 dernières photos de la session dans une petite pile du viseur : parcourez-les, puis envoyez, expédiez par e-mail ou supprimez l'une d'elles. Choisissez combien de projecteurs portent une ombre, de 1 à 16, dans Préférences › Graphismes › Lumière (2 par défaut). Dans les régions chargées, la carte graphique place les objets qui tournent ou bougent seuls, les prims flexibles hors de vue gardent leur maillage, et les écritures de l'image partent ensemble : plus d'images par seconde, la même image.

## 2026-11-04 — Construction, avatars et rapports de plantage
Les lanternes en papier et les autres sphères tordues gardent leur forme, la construction Legacy retrouve Inclinaison, Rayon et Révolutions, et les objets posés depuis l'inventaire s'éditent et se cadrent tout de suite. Les objets portés suivent les animations qui déplacent leur point d'attache, un avatar caché par une animation quitte l'écran comme dans le Legacy Viewer, et le Poser marche sur toutes les formes. Les rapports après une carte graphique perdue disent ce qui tournait. Merci à anne-skydancer, Gaalrauch, Marakiinfinity et Odessa-Draekavik.

## 2026-11-04 — Aurora 0.9.0 : OpenSim et l'Hypergrid, pour de bon
Sur les grilles OpenSim, votre avatar est maintenant cuit et se voit tel que vous l'habillez, le terrain des grandes régions va au-delà de 256 m, et l'inventaire, les tenues et les profils s'enregistrent sur les grilles sans AIS. Les achats et les prix prennent la monnaie et le symbole de la grille. Après une téléportation Hypergrid, Aurora suit la grille où vous êtes, demande avant un voyage qui laisserait des objets portés, et propose d'ajouter une grille inconnue depuis un lien.
[En savoir plus](https://github.com/Aurora-Viewer/Aurora-Viewer/releases/tag/v0.9.0)

## 2026-11-04 — Accrocher aux objets, poser sur une surface, hologrammes
La construction gagne trois aides au placement. Réglez Accrocher sur Objets et la sélection se plaque contre les objets voisins, des guides violets sur les faces qui se touchent, sans jamais les traverser. Appuyez sur P pour la poser sur une surface ou le sol, montrée en hologramme. Un hologramme peut aussi montrer l'objet déplacé, ou son point de départ (Préférences › Aurora › Comportement › Construction). Étirer les textures tient au redimensionnement, et les contrôles de caméra inclinent la vue.

## 2026-11-04 — La barre d'outils Legacy, au complet
Les boutons Préférences rapides, Navigateur web et Voix de la barre d'outils ouvrent leurs fenêtres du Legacy Viewer : les réglages les plus changés, un navigateur intégré sur une page d'accueil, les contrôles de la voix. Statistiques, Favoris du profil, Balises, Photo sur disque et Ne montrer que les amis les rejoignent, un bouton s'allume quand sa fenêtre est ouverte, et les raccourcis de menu manquants du Legacy Viewer sont là. Moi › Ignorer l'interdiction de voler (Ctrl+Alt+V) fait voler là où c'est interdit, après une mise en garde.

## 2026-11-04 — Inventaire, chat et groupes
Donnez une couleur à vous à un dossier de l'inventaire, visible de vous seul, et sauvegardez une tenue dans un fichier JSON pour la restaurer plus tard. Moi › Expériences liste vos expériences et ce que leurs scripts ont fait. Portez un titre de groupe choisi par région, et connectez-vous directement à un repère favori. Le chat a sa propre taille de texte, un long MI part en plusieurs messages au lieu d'être coupé, et l'heure du chat suit le format 12 ou 24 h de l'horloge.

## 2026-11-04 — Plus stable sur votre carte graphique
Les Intel Iris Xe redémarrent, et Aurora partage la carte graphique avec les autres viewers et programmes. La qualité photo monte autant que la mémoire vidéo le permet, sans perdre la carte, et les textures du monde se limitent à 512, 1024 ou 2048 pixels. Liens, journaux et connexions aux simulateurs sont renforcés après un audit de sécurité de Butter Roux, merci ! Merci aussi à Aznlilly, anne-skydancer, circuskid, ddagetracker, emmelinedevere, Gaalrauch, iGao, jumpinmimi, KanikaBreed, Marakiinfinity, Memekiller, Odessa-Draekavik, vexxbyte, voidel, wolfosbournesl, xWynt3r et zdub0is.

## 2026-11-02 — Aurora 0.8.5 : son immersif et VR, tous deux expérimentaux
Les sons du monde s'assourdissent derrière un mur ou dans une boîte fermée, passent derrière votre tête au casque (HRTF), et résonnent comme le lieu où vous êtes : secs en plein air, une longue traîne douce dans une grande salle. Trois interrupteurs dans Préférences › Son ; HRTF et réverbération sont désactivés par défaut. Avec un casque OpenXR, le bouton casque de la barre du bas montre le monde en relief, depuis les yeux de l'avatar ou la caméra. Visite seulement pour l'instant : Échap ramène le bureau.
[En savoir plus](https://github.com/Aurora-Viewer/Aurora-Viewer/releases/tag/v0.8.5)

## 2026-11-02 — Des démarrages rapides, des lieux bondés plus fluides
Aurora garde son propre cache des pipelines graphiques compilés : un démarrage n'attend plus vingt secondes devant une fenêtre noire quand le pilote a vidé le sien. Sur un écran 4K, l'occlusion ambiante coûte environ un tiers de ce qu'elle coûtait, les textures restent nettes après une téléportation dans une région chargée, une fenêtre du viewer en arrière-plan rend sa mémoire vidéo à celle de devant, et les cartes rapides retrouvent les FPS de la 0.8.3 dans les lieux bondés.

## 2026-11-02 — Étoiles, portails et construction
Choisissez d'où sur Terre le vrai ciel nocturne est vu, et l'éclat de ses étoiles. Glissez un lieu des Favoris, des Lieux, de l'historique des téléportations ou de la recherche dans le monde pour ouvrir un portail. Un rectangle de sélection ne prend que ce qu'il entoure, les objets se créent avec votre groupe actif, les textures BOM vont dans toutes les maps PBR, et les PNG en niveaux de gris s'importent. Une expérience autorisée peut maintenant changer votre ciel et votre eau.

## 2026-11-02 — Tenues, achats et Linux
Sélectionnez plusieurs articles d'une tenue ou de Porté pour les mettre ou les retirer d'un coup ; Retirer de la tenue actuelle marche sur n'importe quel dossier. Les vendeurs vendent en un clic, et un solde insuffisant ouvre Acheter des L$. Videz l'historique des notifications, voyez un ami se connecter ou partir dans sa conversation ouverte, et attribuez les boutons de la manette en les pressant. Sous Linux, un second viewer garde les pages web du premier, et les graphismes de secours sont proposés après un souci de GPU.

## 2026-11-02 — Plus stable
Vos logs et réglages survivent à un arrêt brutal du PC, À propos du terrain enregistre ses listes d'accès et de bannis, l'eau ne devient plus blanche sous certains ciels de région, et les photos attendent une carte graphique occupée au lieu d'échouer. Merci à anne-skydancer, circuskid, Gaalrauch, KanikaBreed, lambskin, Marakiinfinity, Odessa-Draekavik, vexxbyte, voidel, wolfosbournesl et zdub0is.

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
