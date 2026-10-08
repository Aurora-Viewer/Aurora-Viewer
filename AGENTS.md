# AGENTS.md — règles des agents IA

Ce fichier s'adresse aux agents IA (Claude Code, Codex, etc.) qui travaillent
sur Aurora Viewer. **Lis-le en entier au début de chaque session**, puis
[ARCHITECTURE.md](ARCHITECTURE.md) et [TASKS.md](TASKS.md). Il prime sur tes
habitudes par défaut. Les humains ont leur propre guide : [HUMANS.md](HUMANS.md).

Plusieurs humains travaillent sur ce dépôt, chacun avec plusieurs agents en
parallèle sur des modules différents. Ces règles servent à ce que personne ne
casse le travail des autres.

---

## 1. Règles absolues

1. **Jamais de connexion à une grille Second Life** (AGNI, Aditi ou autre) et
   **jamais d'identifiants** : tu ne saisis, ne lis, ne stockes et ne
   transmets aucun nom d'utilisateur, mot de passe, jeton ou clé. Tu peux
   lancer le viewer pour l'humain ; c'est **lui** qui se connecte et qui teste
   sur la grille. Par sécurité, toute manipulation sur AGNI est faite par
   l'humain.
2. **Aucun secret dans le dépôt** : ni mot de passe, ni jeton, ni clé API, ni
   URL de capability, ni donnée personnelle (adresse e-mail, nom réel…).
3. **Ne touche jamais au travail en cours d'un autre agent.** Tu travailles
   dans ton propre worktree (section 3). Si le build échoue dans du code que tu
   n'as pas modifié, ne le « corrige » pas : signale-le.
4. **Le test final d'une fonctionnalité ou d'un correctif est validé par un
   humain**, avant toute PR (section 8).
5. **Seul un humain fusionne** une PR. Tu ne merges jamais.
6. **Réponds à l'humain en français.** Code, commentaires et noms en anglais ;
   documentation et textes de l'interface en français.

## 2. Comportement de référence : Firestorm

Aurora reproduit le comportement de Firestorm. Avant de porter ou de modifier
une fonctionnalité, **regarde comment Firestorm la fait réellement**.

- Les sources officielles doivent se trouver **à côté du dépôt** :
  `../phoenix-firestorm` (lecture seule, jamais dans le dépôt).
- **Si ce dossier est absent**, propose à l'humain de le cloner :

  ```bash
  git clone --depth 1 https://github.com/FirestormViewer/phoenix-firestorm ../phoenix-firestorm
  ```

- Les fichiers utiles sont surtout dans `indra/newview/` (viewer),
  `indra/llcharacter/` (animations), `indra/llmessage/`,
  `indra/llrender/`, et `indra/newview/app_settings/settings.xml` (valeurs par
  défaut des réglages).
- Dans le code porté, indique la source en commentaire de module ou de
  fonction, par exemple : `Port of LLMotionController::updateMotionsByType
  (indra/llcharacter/llmotioncontroller.cpp, originally LGPL 2.1)`. Ce code est
  distribué sous GPL-3.0-or-later (voir [NOTICE.md](NOTICE.md)).
- Écart volontaire par rapport à Firestorm : seulement s'il est justifié, et
  noté dans le code et dans `TASKS.md`.

## 3. Travail en parallèle : un worktree par tâche

Ne travaille **jamais directement dans le dépôt principal**. Chaque tâche a son
worktree git, sa branche, son dossier `target` et ses logs.

```powershell
# depuis le dépôt principal
./scripts/new-task.ps1 -Name "regard-avatars" -Kind feat
# → C:\Aurora_Viewer_v2\work\regard-avatars  (branche feat/regard-avatars)
```

- `-Kind` : `feat` (fonctionnalité), `fix` (correctif), `perf`, `refactor`,
  `docs`, `chore` (outillage, dépendances).
- Le worktree est créé dans `../work/<nom>` à partir de `origin/main` à jour.
  Son `target/` est à lui : tes compilations ne gênent pas les autres agents
  et les leurs ne te gênent pas.
- Le premier build d'un worktree prend quelques minutes ; c'est normal.
- **Compile uniquement dans ton worktree**, avec son `target/` par défaut :
  pas de `CARGO_TARGET_DIR` maison ni de dossier de compilation ailleurs
  (chaque dossier de compilation pèse plusieurs Go et finit oublié).
  `end-task.ps1` le supprime avec le worktree.
- Pour voir la place prise par les compilations et en libérer :
  `./scripts/clean.ps1` (n'efface rien sans `-Apply`).
- Une fois la PR fusionnée par l'humain :
  `./scripts/end-task.ps1 -Name "regard-avatars"` supprime le worktree et la
  branche locale.
- Fichiers temporaires (scripts, captures, notes) : dans le dossier
  `../work/<nom>/.scratch/` (ignoré par git) ou le dossier temporaire de ton
  outil, jamais dans l'arborescence suivie.

## 4. Tester : le mode démo

Le mode démo simule un serveur en local (une place, des avatars, des objets,
des messages…). C'est ton terrain de test : tu peux lancer le viewer autant
que nécessaire.

```powershell
$env:AURORA_DEMO = "1"
cargo run --release -p aurora-viewer -- --title "Test regard-avatars"
```

- **Toujours `--title "Test <tâche>"`** : la fenêtre s'appelle
  « Aurora Viewer — Test <tâche> » et le log est
  `%LOCALAPPDATA%\Aurora\AuroraViewer\data\logs\aurora-demo-test-<tâche>.log`.
  Les fenêtres et logs des agents ne se mélangent pas.
- Captures automatiques : `AURORA_CAPTURE=<fichier.png>`,
  `AURORA_CAPTURE_EXIT=1`, `AURORA_CAPTURE_FRAMES=620` (après le fondu de
  chargement). Regarde tes captures : c'est ta vérification visuelle.
- La liste complète des variables `AURORA_*` est dans le
  [README](README.md#test-switches). **Si tu ajoutes, modifies ou supprimes une
  variable ou un argument, mets le README à jour dans la même PR.**
- Pour une nouvelle fonctionnalité, ajoute si possible un scénario de démo
  (`AURORA_DEMO_<NOM>`) qui la montre et la rend testable sans grille.
- Certaines choses ne se testent que sur la grille (réseau réel, voix,
  téléportations…) : dis précisément à l'humain quoi tester (section 8).

## 5. Qualité du code

Avant chaque commit significatif et obligatoirement avant la PR :

```powershell
./scripts/check.ps1   # cargo fmt --check, clippy -D warnings, tests
```

- **Zéro avertissement clippy** (`-D warnings`), code formaté (`rustfmt.toml`).
- **Pas de `unwrap()` / `expect()` sur des données externes** (réseau, fichiers,
  assets, réglages) : on gère l'erreur et on journalise. `expect` est accepté
  dans les tests et pour des invariants internes documentés.
- Pas de `unsafe` sans commentaire `// SAFETY:` qui le justifie.
- Commentaires utiles et denses, comme le code existant : pourquoi, et quelle
  source Firestorm. Pas de commentaires qui répètent le code.
- Un module = une responsabilité. Garde l'arborescence décrite dans
  [ARCHITECTURE.md](ARCHITECTURE.md) ; si tu ajoutes un module ou un dossier,
  mets ARCHITECTURE.md à jour.
- Des tests unitaires pour la logique pure (calculs, parsing, règles) ; le mode
  démo pour le visuel.
- **Dépendances à jour** : on utilise les dernières versions (Rust stable,
  crates). Une nouvelle dépendance doit être maintenue, de licence compatible
  GPL v3 (MIT, Apache-2.0, BSD, MPL-2.0, LGPL…), et justifiée dans la PR.
- **Pas de régression de la réécriture** : si tu modifies un comportement porté
  de Firestorm, revérifie-le contre les sources de Firestorm.

## 6. TASKS.md

[TASKS.md](TASKS.md) est la liste de référence (en français) : ✅ fait (et
vérifié sur la grille quand ça en dépend), 🟡 fait mais pas encore vérifié sur
la grille, 🔧 en cours, ⬜ à faire.

- Mets-le à jour **dès qu'une étape avance** : passe la ligne en 🔧 au début
  de ta tâche, en ✅ quand c'est validé, avec un résumé de ce qui a été fait
  (règle reprise de Firestorm, options, variable de démo…).
- Ce qui reste à faire va dans une ligne ⬜ « plus tard » claire.
- Ne réécris pas les lignes des autres tâches.

## 7. Interface et branding

Interface plate, propre et moderne, palette Aurora, icônes Phosphor
« regular » uniquement : voir [docs/BRANDING.md](docs/BRANDING.md). Jamais de
couleur en dur : utilise la `Palette`. Réutilise les widgets de
`ui/widgets.rs`.

## 8. Cycle d'une tâche, jusqu'à la PR

1. **Lire** AGENTS.md, ARCHITECTURE.md, TASKS.md ; étudier Firestorm.
2. **Créer le worktree** (`new-task.ps1`), passer la tâche en 🔧 dans TASKS.md.
3. **Développer** par petits commits locaux (messages clairs, section 10).
4. **Tester en démo**, `check.ps1` au vert.
5. **Validation humaine (obligatoire, avant la PR)** :
   - compile le viewer dans ton worktree et lance-le pour l'humain
     (`cargo run --release -p aurora-viewer -- --title "Test <tâche>"`, en
     mode démo ou normal selon ce qu'il faut tester) ;
   - dis-lui précisément quoi tester, où (démo ou grille AGNI) et ce qu'il
     doit voir ; s'il faut la grille, c'est lui qui se connecte ;
   - attends sa réponse. S'il signale un problème, corrige et recommence.
6. **Prévenir s'il reste des choses** : si une partie n'est pas finie, dis-le à
   l'humain et demande s'il veut une PR maintenant ou attendre.
7. **Ouvrir la PR** (section 9) — une PR par tâche terminée et validée, jamais
   pour des changements intermédiaires.
8. **Relire ta PR** (section 11) et poster le commentaire de relecture.
9. **Prévenir l'humain** : lien de la PR, verdict, et ce qu'il doit faire
   (fusionner, trancher un point…).
10. Après la fusion par l'humain : `end-task.ps1`.

## 9. La pull request

Titre court au format `type(zone): résumé` en français, par exemple
`feat(avatar): suivi du regard comme Firestorm`.

```powershell
git push -u origin HEAD
gh pr create --base main --title "feat(avatar): …" --body-file <fichier.md> --label nouveauté --label avatar
```

Le corps suit le modèle `.github/pull_request_template.md` et doit être
**lisible en deux minutes par un humain** :

- **À quoi ça sert** : 2–3 phrases simples, du point de vue de l'utilisateur.
- **Origine Firestorm** : oui/non ; si oui, les fichiers et fonctions suivis.
- **Ce qui change** : par zone (interface, rendu, réseau…), en puces courtes ;
  options ajoutées avec leur valeur par défaut.
- **Captures** : images du mode démo (section 12).
- **Tests faits** : `check.ps1`, scénarios démo, validation humaine (date et
  ce qui a été testé).
- **Limites / suite** : ce qui n'est pas fait, lignes ⬜ ajoutées à TASKS.md.

Étiquettes (créées par `scripts/setup-labels.ps1`) :

- **un type**, qui range la PR dans les notes de version : `nouveauté`,
  `correctif`, `performance`, `maintenance` (refactorisation, dépendances,
  outillage), `docs` ; plus `rupture` si la PR casse une compatibilité
  (réglages, format de cache…) ;
- **une ou plusieurs zones** : `rendu`, `avatar`, `monde`, `réseau`,
  `audio-voix`, `interface`, `construction`.

## 10. Commits, identité et mentions

- Les commits et les PR sont faits **au nom de l'humain** avec qui tu
  travailles (sa configuration git et son compte `gh`). Ne change pas
  `user.name` / `user.email`.
- **Aucune mention d'IA** : pas de « Co-Authored-By: Claude/Codex », pas de
  « Generated with… », pas de « assisté par IA » dans les commits, les PR ou
  les commentaires.
- Messages de commit en français, à l'impératif ou au nom, clairs :
  `Ajoute le fondu de sortie des animations arrêtées`.

## 11. Relecture de ta PR (obligatoire)

Avant de prévenir l'humain, relis ta propre PR comme le ferait un relecteur
exigeant :

1. **À jour avec `main`** : `git fetch origin && git rebase origin/main`.
   Résous les conflits sans perdre le travail des autres, relance
   `check.ps1`, puis `git push --force-with-lease`.
2. **CI verte** : `gh pr checks --watch`.
3. **Non-régression** : relance les scénarios de démo touchés par ta PR **et**
   ceux des fonctionnalités voisines (le travail fusionné des autres agents
   entre-temps ne doit pas être cassé). Compare avant / après en captures.
4. **Fidélité Firestorm** : le comportement correspond aux sources
   consultées ; les écarts sont justifiés.
5. **Secrets et données personnelles** : `git diff origin/main...HEAD` ne
   contient aucun secret, URL de capability, identifiant, chemin personnel.
6. **Qualité** : code lisible, pas de code mort, README / ARCHITECTURE /
   TASKS à jour.

Corrige toi-même tout ce que tu peux corriger **sans rien casser**, puis
recommence la relecture. Si un problème te dépasse (conflit de conception avec
une autre tâche, choix produit, régression que tu ne sais pas résoudre),
**n'improvise pas** : explique-le à l'humain et propose 2–3 solutions.

Poste ensuite la relecture en commentaire (`gh pr comment <n> --body-file …`),
avec cette structure :

```markdown
## Relecture

**Verdict : ✅ prête à fusionner** (ou ⚠️ à trancher / ❌ bloquée)

| Vérification | Résultat |
|---|---|
| À jour avec main, sans conflit | ✅ |
| CI (fmt, clippy, tests, secrets) | ✅ |
| Scénarios démo (avant / après) | ✅ captures ci-dessous |
| Non-régression des fonctionnalités voisines | ✅ … |
| Fidélité Firestorm | ✅ fichiers vérifiés : … |
| Secrets / données personnelles | ✅ aucun |
| Docs (README, ARCHITECTURE, TASKS) | ✅ |

### Points relevés
- Corrigé : …
- À trancher par l'humain : … (solutions proposées : A / B)

### Captures
![…](…)
```

## 12. Captures dans les PR

Les captures de démo vont sur la branche dédiée `captures` (jamais sur
`main`) :

```powershell
./scripts/pr-captures.ps1 -Pr 42 -Files avant.png, apres.png
```

Le script affiche le Markdown des images à coller dans la description ou le
commentaire de relecture.

## 13. Versions et releases

- **SemVer** : `vMAJEUR.MINEUR.CORRECTIF`. Avant la 1.0 : une nouveauté
  augmente MINEUR, un correctif augmente CORRECTIF.
- Les tags et releases sont créés **par l'humain**. Les notes sont générées
  depuis les étiquettes de type des PR (`.github/release.yml`), d'où
  l'importance de bien étiqueter.
- Les PR de Dependabot (mises à jour de dépendances) suivent le même chemin :
  vérifie qu'elles compilent, passent les tests et la démo, puis préviens
  l'humain.

## 14. Communiquer avec l'humain

- En français, clair et court. Dis ce qui est fait, ce qui reste, et **ce
  qu'il doit faire lui** (tester quelque chose, se connecter, fusionner,
  trancher).
- Ne prétends jamais qu'une chose est testée si elle ne l'est pas.
- Garde les logs et les captures pour appuyer ce que tu affirmes.
