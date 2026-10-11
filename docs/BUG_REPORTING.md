# Rapports de bugs de l’alpha

Le formulaire commun est accessible depuis la barre du haut, Aide et la
connexion. « Envoyer depuis Aurora » est le premier choix et le mode par défaut
des nouveaux rapports. Les options d’envoi sont indépendantes du modèle du rapport.
Le mode navigateur est disponible sans configuration d’application ; les
deux modes demandent un compte GitHub au testeur. Sans compte, une copie
locale reste possible.

## Configuration de l’envoi direct

Créer une **GitHub App**, dans les réglages développeur de l’organisation
Aurora-Viewer, puis :

- activer **Enable Device Flow**, garder l’expiration des autorisations ;
- laisser les URL de retour et de configuration vides, désactiver le webhook ;
- permissions du dépôt : **Issues — Read and write**, métadonnées en lecture
  seule, aucune autre permission ;
- application publique (**Any account**) pour les testeurs ; l’installer
  sur l’organisation avec uniquement le dépôt Aurora-Viewer sélectionné ;
- reporter le **Client ID public** dans `bug_report/github.rs::CLIENT_ID`.
  L’App ID numérique ne convient pas. Aucun Client secret ni clé privée
  n’est nécessaire et aucun ne doit être ajouté au viewer.

[Création d’une GitHub App](https://docs.github.com/en/apps/creating-github-apps/registering-a-github-app/registering-a-github-app)
et [Device Flow](https://docs.github.com/en/apps/creating-github-apps/authenticating-with-a-github-app/generating-a-user-access-token-for-a-github-app#using-the-device-flow-to-generate-a-user-access-token).

Le testeur clique sur Se connecter, copie le code et l’autorise lui-même
sur GitHub dans son navigateur. Le viewer ne demande jamais son mot de passe.
L’autorisation reste en mémoire pour cette session, au plus huit heures ;
elle n’est écrite ni dans les réglages, ni dans les journaux. Une autorisation
ne déclenche pas l’envoi : le testeur clique ensuite sur Envoyer le rapport
public. Annulation, refus, expiration et ralentissement demandé par GitHub
sont traités. Une création d’issue n’est jamais relancée automatiquement après
une réponse perdue, pour éviter les doublons.

Créer les étiquettes `bug` et `alpha` avec `scripts/setup-labels.ps1` lors de
la configuration du dépôt. Le modèle d’issue est dans `.github/ISSUE_TEMPLATE/`.

## Données et fichiers

Le formulaire présente deux étapes : préparer le rapport, puis choisir l’envoi.
Il tient sur une colonne avec trois onglets : Description, Image et Données.
Les quatre champs de Description restent visibles ensemble sans défilement
du formulaire à la taille normale. Les textes longs défilent dans leur propre
zone de saisie.
Les onglets restent visibles pendant le défilement. L’onglet Données permet
de lire et de décocher chaque information automatique. Les libellés et les
diagnostics suivent la langue de l’interface à chaque affichage. Le texte
saisi est conservé tel quel. Les titres et valeurs techniques du fichier
exporté restent en anglais pour les outils qui lisent les rapports. Les actions
de copie et d’enregistrement sont dans « Autres actions ».
Un nouveau rapport commence sur Description. Fermer la fenêtre conserve en
mémoire le texte, l’onglet, le défilement, les cases, l’image et le mode d’envoi,
y compris l’autorisation GitHub en cours, jusqu’à la fin de la session du viewer.
Une erreur d’envoi conserve également le brouillon. Après création confirmée
par l’API, le lien du rapport et son dossier restent disponibles ; rouvrir la
fenêtre ou choisir « Nouveau rapport » prépare ensuite un formulaire neuf.
Le navigateur ne confirme pas la publication au viewer : le testeur clique
sur « J’ai publié le rapport » après l’envoi sur GitHub pour vider le brouillon.
Ouvrir GitHub ou enregistrer une copie locale ne le vide pas.
Le nom de l’avatar et les diagnostics sont
décochés au départ. Aucun état de connexion SL ou réglage complet n’est
sérialisé. Les diagnostics publics ne gardent que les catégories reconnues
d’avertissement / erreur : une simple suppression des URL ou UUID ne suffirait
pas à retirer les noms et les textes libres venant du réseau.

Les captures sont limitées au monde, avec ou sans interface ; l’écran de
connexion ne peut pas être capturé par ce bouton. Les fichiers choisis sont
limités à 16 Mio, décodés avec des limites de dimensions/mémoire et réencodés
en PNG de 1920 pixels de côté au plus. Métadonnées, chemin et nom du fichier
d’origine ne sont jamais joints. Une capture peut encore contenir des noms
et du chat visibles : le testeur doit la relire.
Après une première capture, changer « Inclure l’interface » refait la capture
automatiquement, sans recocher son inclusion si le testeur l’avait exclue.
Cette option ne remplace jamais une image importée depuis un fichier.

Chaque enregistrement crée un dossier sous `data/bug-reports/`, à côté de
`data/logs/`. Le sous-dossier `public` contient le texte choisi et, si incluse,
`screenshot.png`. Le dossier `private-do-not-upload` contient les diagnostics
locaux privés et ne doit jamais être joint à une issue publique. Les images
ne sont pas téléversées par l’API : après création, ouvrir l’issue et glisser
la capture depuis `public`. Le mode navigateur propose la même opération.

## Validation

`AURORA_DEMO_BUG_REPORT=1` avec le mode démo ouvre un rapport exemple. Les deux
envois restent locaux, sans autorisation GitHub ni ouverture du navigateur.
Les modes `details`, `image`, `image-ui` et `direct` montrent respectivement
les informations avec le lieu décoché, une capture de la scène, une capture
avec l’interface et le choix d’envoi direct. Le formulaire s’ouvre après le
chargement de la scène (image 550).
`refresh` prend une capture sans interface puis active l’option à l’image 700
et la désactive à l’image 900. `language` passe la fenêtre déjà ouverte en
anglais à l’image 700, puis en français à l’image 900, sans changer les
préférences enregistrées. Pour tester manuellement la langue dans Préférences,
lancer sans argument `--language`, qui forcerait la langue du processus.
Tester les champs décochés, la copie, l’export, une image avec retrait puis
réajout, une capture avec et sans interface, le redimensionnement et les deux
langues. Modifier le texte, choisir Image ou Données, fermer puis rouvrir depuis
Signaler un bug : vérifier le texte, les cases, l’image et l’onglet conservés.
Tester également l’entrée depuis la connexion et le menu Aide.

L’autorisation réelle et la publication d’un rapport de test sur GitHub sont
validées par l’humain, avec son compte. Vérifier aussi l’annulation et le refus
d’autorisation. Les tests automatiques n’ouvrent aucune issue réelle.
