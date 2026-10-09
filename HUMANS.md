# HUMANS.md — guide des humains

Ce dépôt est conçu pour que deux personnes (ou plus) travaillent chacune avec
plusieurs agents IA (Claude Code, Codex…) en parallèle. Les agents suivent
[AGENTS.md](AGENTS.md) ; ce guide explique **ton** rôle et comment tout
s'enchaîne.

**Pour commencer :** [clone le dépôt et double-clique sur
`aurora-tools.cmd`](#installer-lenvironnement) ; ensuite, tout se fait avec
[les outils Aurora](#les-outils-aurora--aurora-toolscmd).

## Ce que font les agents, ce que tu fais

| Les agents | Toi |
|---|---|
| Codent dans leur propre dossier de travail (un par tâche) | Choisis les tâches à lancer |
| Testent en **mode démo** (serveur simulé en local) | **Valides** chaque fonctionnalité avant la PR |
| Te lancent le viewer pour que tu testes | **Te connectes et testes sur la grille AGNI** (eux ne le font jamais) |
| Ouvrent la PR, la relisent, corrigent, commentent, **la fusionnent** | Tranches quand l'agent te le demande (verdict ⚠️ ou ❌) |
| Tiennent TASKS.md à jour, publient les releases | **Demandes** une release quand tu veux (« sors la 0.4.0 ») |

Tu n'as **aucune action à faire sur GitHub** : ton seul travail est de tester
ce que l'agent te montre, et de te connecter à la grille quand il le faut.

## Le cycle d'une tâche

1. Tu demandes une tâche à un agent (« fais le suivi du regard comme Firestorm »).
2. L'agent crée son dossier de travail `..\work\<tâche>` sur une branche à lui :
   il ne voit pas le code en cours des autres agents, et réciproquement.
3. Il code, teste en démo, puis **compile et te lance le viewer** avec une
   fenêtre nommée « Aurora Viewer — Test <tâche> ».
4. **Tu testes** ce qu'il te demande (en démo ou sur la grille : c'est toi qui
   te connectes) et tu lui dis si c'est bon. Pour une PR sans effet sur le
   viewer (documentation, CI, scripts), il te dit juste ce qui va changer.
5. L'agent ouvre la **pull request** : un résumé lisible (à quoi ça sert, si ça
   vient de Firestorm, ce qui change, captures).
6. Dans la foulée, sans que tu aies à le demander, il attend la fin de la CI
   puis **relit sa PR** : conflits avec `main`, CI, non-régression,
   secrets… et poste un **commentaire de relecture** (onglet « Conversation »
   de la PR) avec un verdict :
   - ✅ prête à fusionner ;
   - ⚠️ un point à trancher (il te propose des solutions) ;
   - ❌ bloquée (il t'explique pourquoi).
7. **Si le verdict est ✅, il la fusionne** : la PR entre dans la **file de
   fusion**, où GitHub la teste avec le `main` le plus récent (tests
   compris) et la fusionne si tout est vert. Si toi et un autre fusionnez en
   même temps, les PR font simplement la queue : plus de « Update branch ».
   Sinon (⚠️ ou ❌), il s'arrête et te demande de trancher.
8. L'agent nettoie son dossier de travail et te prévient : lien de la PR, ce
   qui a changé, et ce qu'il reste à tester sur la grille s'il y en a.

Une PR n'est ouverte qu'à la **fin d'une tâche**, jamais pour des changements
intermédiaires. Si l'agent n'a pas tout fini, il te le dit avant. Si une
session s'interrompt, l'agent de la session suivante reprend en priorité les
PR sans relecture ou sorties de la file.

## GitHub en pratique

- **`main` est protégée**, pour tout le monde, admins compris : personne ne
  pousse directement dessus ; tout passe par une PR, puis par la file de
  fusion.
- **La CI** (GitHub Actions, sous Windows) a deux étages :
  - sur chaque PR, les vérifications rapides : formatage, clippy, recherche
    de secrets (les tests, l'agent les a déjà passés sur sa machine) ;
  - dans la file de fusion, tout, tests compris, sur la PR combinée avec le
    dernier `main` : ce qui arrive sur `main` est exactement ce qui a été
    testé.
  Une PR qui ne touche que la documentation ou les images passe en une
  minute (pas de compilation). Une croix rouge = l'agent corrige.
- **Étiquettes** : chaque PR a un **type** (`nouveauté`, `correctif`,
  `performance`, `maintenance`, `docs`, et `rupture` si elle casse une
  compatibilité), qui la range dans les notes de version, et une **zone**
  (`rendu`, `avatar`, `monde`, `réseau`, `audio-voix`, `interface`,
  `construction`) pour s'y retrouver.
- **Captures** : les images des PR sont rangées sur la branche `captures`
  (elle ne contient que ça).
- **Dependabot** ouvre chaque semaine des PR de mise à jour des dépendances ;
  un agent peut les vérifier pour toi, et les fusionne si tu es d'accord.
- **Le dépôt** appartient à l'organisation `Aurora-Viewer` ; les
  développeurs sont dans l'équipe `Devs`.

### Publier une version

On suit [SemVer](https://semver.org/lang/fr/) : `vMAJEUR.MINEUR.CORRECTIF`.
Avant la 1.0, une nouveauté fait `v0.3.0 → v0.4.0`, un correctif
`v0.4.0 → v0.4.1`.

Le plus simple : choix **Gérer les releases** des outils Aurora (section
plus bas). Ils vérifient que `main` est vert, proposent le numéro d'après les
étiquettes des PR fusionnées depuis la dernière version, montrent ces PR,
créent le tag et suivent la publication jusqu'au lien de la release. Ils
servent aussi à **supprimer** une release (il faut taper son tag pour
confirmer ; `assets-1`, qui contient la police emoji, est protégée). Il faut
le droit d'écriture sur le dépôt.

Tu peux aussi le demander à un agent : « sors la 0.4.0 » (ou « sors une
version » : il te propose le numéro). Dans les deux cas, GitHub Actions
compile le viewer, crée la release avec le zip Windows et écrit les notes,
classées par catégorie, à partir des PR fusionnées. À la main, c'est :

```bash
git switch main && git pull
git tag -a v0.4.0 -m "Aurora Viewer 0.4.0"
git push origin v0.4.0
```

## Installer l'environnement

1. Crée un dossier pour le projet (par exemple `C:\Aurora-Viewer`), ouvre une
   invite de commandes dedans (dans l'Explorateur : tape `cmd` dans la barre
   d'adresse, puis Entrée) et clone le dépôt :

   ```bat
   git clone https://github.com/Aurora-Viewer/Aurora-Viewer aurora-viewer
   ```

2. Double-clic sur **`aurora-viewer\aurora-tools.cmd`** (les outils Aurora,
   section suivante). Ils vérifient ce qui manque et proposent **Réparer** :
   - ils installent ce qui manque avec winget : GitHub CLI (pour toute la
     machine), les outils C++ de Visual Studio dont Rust a besoin pour
     compiler (plusieurs Go), et Rust (rustup, pour ton compte, comme le veut
     Rust sous Windows) ; des fenêtres d'autorisation Windows peuvent
     s'ouvrir ;
   - ils clonent les sources de Firestorm dans `phoenix-firestorm\`, à côté
     du dépôt ;
   - ils téléchargent la police emoji, la toolchain Rust et toutes les crates ;
   - ils te connectent à GitHub (`gh auth login`, dans le navigateur) si
     besoin, et règlent ton identité git dans le dépôt : ton pseudo GitHub et
     ton adresse « noreply » (ton e-mail reste privé).

**Pas encore de Git ?** Dans une invite de commandes ouverte dans le dossier
du projet, cette commande installe tout, Git compris, puis ouvre les outils
(elle montre d'abord ce qu'elle va faire et demande où installer) :

```bat
powershell -NoProfile -ExecutionPolicy Bypass -Command "$f = Join-Path $env:TEMP 'aurora-setup.ps1'; irm https://raw.githubusercontent.com/Aurora-Viewer/Aurora-Viewer/main/scripts/setup.ps1 -OutFile $f; & $f"
```

Dans la suite, **le dossier du projet** est celui de l'installation : il
contient `aurora-viewer\`, `phoenix-firestorm\`, `work\` (les tâches des
agents) et, plus tard, `RELEASE\`.

## Les outils Aurora : `aurora-tools.cmd`

Double-clic sur **`aurora-tools.cmd`**, à la racine du dépôt (ou sur le
raccourci « Aurora Tools » que l'outil peut créer). Tout se fait au clavier :
**↑ ↓** pour choisir, **Entrée** pour valider, **Échap** ou **Retour
arrière** pour revenir, et la touche affichée devant chaque choix comme
raccourci.

À l'ouverture, l'outil vérifie l'environnement : outils, dépôt en retard sur
GitHub, sources de Firestorm périmées, Rust pas à jour, police, connexion
GitHub… S'il manque ou s'il est périmé quelque chose, le premier choix est
**Réparer / mettre à jour** (la même installation que la commande en une
ligne : ce qui est là est mis à jour, rien n'est réinstallé).

| Choix | Ce que ça fait |
|---|---|
| Viewer release | Le dernier `main` de GitHub en `--release`, dans `RELEASE\` (voir plus bas), puis « lancer ? » |
| Viewer de dev | Ton dépôt local, profil par défaut (celui des agents) |
| Viewer de debug | Ton dépôt local avec une console, les contrôles de débogage et les infos de débogage complètes (profil `debugging`, son propre dossier de quelques Go) |
| Démo | Le viewer hors ligne, avec le choix d'un scénario `AURORA_DEMO_*` (lus dans le README) |
| Tâches des agents | Les dossiers de `work\` : PR, changements en cours, dernière compilation, taille ; ménage de celles dont la PR est fusionnée |
| Disque | La place prise par les compilations, et le nettoyage (`clean.ps1`) |
| Logs du viewer | Ouvrir le dossier, lire la fin du dernier log, copier son chemin pour un agent |
| Suivi des PR | Le panneau en direct (voir ci-dessous) |
| Gérer les releases | Créer ou supprimer une version (voir [Publier une version](#publier-une-version)) |
| Raccourcis | « Aurora Tools » et « Aurora PR » (le panneau des PR directement) sur le Bureau et dans le menu Démarrer, avec le logo |
| Vérifier à nouveau | Le tableau complet de l'environnement |

**Les PR en direct.** Tant que les outils sont ouverts, le bandeau du bas
résume les PR (« PR : 3 ouvertes · 1 CI · 1 en file · 1 rouge »), et une
**notification Windows** signale une PR fusionnée, une CI qui passe au rouge,
une PR sortie de la file de fusion, une nouvelle PR, ou `main` qui casse. Le
panneau **Suivi des PR** (ou le raccourci « Aurora PR ») montre chaque PR :
sa CI avec la durée qui défile, sa place dans la file de fusion, le verdict
de la relecture de l'agent, et les dernières fusions. Il s'adapte à la
largeur de la fenêtre : réduis-la et garde-la dans un coin de l'écran.
**↑ ↓** choisir, **Entrée** ouvrir la PR dans le navigateur, **R**
rafraîchir. GitHub est interrogé toutes les 10 à 20 secondes, pas plus :
le quota de GitHub (5 000 requêtes par heure) est partagé avec tes agents.

Chaque choix existe aussi sans menu, pour un script :
`./scripts/tools/aurora-tools.ps1 -Action diagnose` (`repair`, `release`,
`dev`, `debug`, `demo`, `tasks`, `disk`, `logs`, `prs`, `releases`,
`shortcut`) ; `-DryRun` montre ce que `releases` ferait sur GitHub sans le
faire.

## Le viewer à jour de `main`, en `--release`

Choix **Viewer release** des outils, ou double-clic sur
`RELEASE\build-release.bat` dans le dossier du projet. L'exe prêt à lancer
est dans `RELEASE\Aurora-Viewer\` (avec ses assets, comme le zip des
releases), et `version.txt` dit quel commit a été compilé.

- Le code vient **toujours de GitHub** : tes changements locaux et tes
  commits non poussés n'y entrent pas.
- Le premier build prend quelques minutes ; les suivants ne recompilent que
  ce qui a changé.
- `build-release.bat 42` compile la PR n°42 (pour tester ses performances
  avant de la fusionner).
- Ferme le viewer de ce dossier avant de recompiler (le script le signale).

## Compiler et lancer à la main

```bash
cargo run -p aurora-viewer
```

- Le profil par défaut (sans `--release`) est celui des agents : il se
  comporte comme la version publiée, sans l'optimisation lente de
  `--release`. `cargo build --release` reste possible pour mesurer des
  performances (il recompile tout une deuxième fois).
- Mode démo (aucune connexion) : `$env:AURORA_DEMO="1"` avant de lancer.
- Nommer la fenêtre : `cargo run -p aurora-viewer -- --title "Mon test"`.
- Tout vérifier comme la CI : `./scripts/check.ps1`.
- Les agents compilent dans leur propre dossier `..\work\<tâche>\target` :
  leurs viewers ouverts ne bloquent pas les tiens.

### Place sur le disque

Les compilations prennent de la place : comptez 3 à 4 Go par dossier
`target` (le dépôt, chaque tâche en cours, `RELEASE`). Ce n'est que du
cache, qu'on peut toujours supprimer : il sera recompilé au besoin. Une
nouvelle tâche libère d'elle-même le `target` des tâches fusionnées ou
inactives depuis 7 jours.

Le choix **Disque** des outils fait la même chose que :

```powershell
./scripts/clean.ps1                 # montre la place prise, n'efface rien
./scripts/clean.ps1 -Apply          # libère les compilations du dépôt (garde target\release)
./scripts/clean.ps1 -Tasks -Apply   # libère aussi celles des tâches en cours
```

Le script ne supprime que des dossiers de compilation Cargo, et refuse de
travailler pendant une compilation ou quand un viewer est ouvert.

## Où sont les fichiers

| Quoi | Où |
|---|---|
| Code du viewer | `crates/` (voir [ARCHITECTURE.md](ARCHITECTURE.md)) |
| Suivi des tâches | [TASKS.md](TASKS.md) |
| Dossiers de travail des agents | `work\`, dans le dossier du projet |
| Sources Firestorm (référence) | `phoenix-firestorm\`, dans le dossier du projet |
| Réglages, skins, disposition | `%APPDATA%\Aurora\AuroraViewer\config\` |
| Cache (textures, mesh…) — peut être supprimé sans risque | `%LOCALAPPDATA%\Aurora\AuroraViewer\cache\` |
| Logs (à envoyer à un agent après un test) | `%LOCALAPPDATA%\Aurora\AuroraViewer\data\logs\` |
| Viewer `--release` du dernier `main` | `RELEASE\Aurora-Viewer\aurora-viewer.exe`, dans le dossier du projet |
| Programme compilé à la main | `target\debug\aurora-viewer.exe` (ou `work\<tâche>\target\debug\`) |

`aurora.log` est le log de ta dernière session réelle ; la session d'avant est
gardée en `aurora.previous.log`. Les sessions de démo ont leurs propres logs
(`aurora-demo*.log`).

## Sécurité

- Les agents ne se connectent jamais à la grille et ne manipulent jamais tes
  identifiants. Le mot de passe retenu est stocké par Windows (Gestionnaire
  d'identification), sous forme d'empreinte uniquement.
- La CI cherche des secrets dans chaque PR (et dans la file de fusion), et la
  relecture de l'agent aussi. Si tu vois passer un identifiant ou un jeton,
  dis à l'agent d'arrêter et signale-le.
- Tu peux toujours demander à un agent de t'expliquer un changement, ou de
  ne pas fusionner une PR avant que tu l'aies regardée.
