# HUMANS.md — guide des humains

Ce dépôt est conçu pour que deux personnes (ou plus) travaillent chacune avec
plusieurs agents IA (Claude Code, Codex…) en parallèle. Les agents suivent
[AGENTS.md](AGENTS.md) ; ce guide explique **ton** rôle et comment tout
s'enchaîne.

## Ce que font les agents, ce que tu fais

| Les agents | Toi |
|---|---|
| Codent dans leur propre dossier de travail (un par tâche) | Choisis les tâches à lancer |
| Testent en **mode démo** (serveur simulé en local) | **Valides** chaque fonctionnalité avant la PR |
| Te lancent le viewer pour que tu testes | **Te connectes et testes sur la grille AGNI** (eux ne le font jamais) |
| Ouvrent la PR, la relisent, corrigent, commentent | **Fusionnes** la PR quand le verdict te convient |
| Tiennent TASKS.md à jour | Crées les **releases** (tags de version) |

## Le cycle d'une tâche

1. Tu demandes une tâche à un agent (« fais le suivi du regard comme Firestorm »).
2. L'agent crée son dossier de travail `..\work\<tâche>` sur une branche à lui :
   il ne voit pas le code en cours des autres agents, et réciproquement.
3. Il code, teste en démo, puis **compile et te lance le viewer** avec une
   fenêtre nommée « Aurora Viewer — Test <tâche> ».
4. **Tu testes** ce qu'il te demande (en démo ou sur la grille : c'est toi qui
   te connectes) et tu lui dis si c'est bon.
5. L'agent ouvre la **pull request** : un résumé lisible (à quoi ça sert, si ça
   vient de Firestorm, ce qui change, captures).
6. Il **relit sa PR** : mise à jour avec `main`, CI, non-régression,
   secrets… et poste un **commentaire de relecture** avec un verdict :
   - ✅ prête à fusionner ;
   - ⚠️ un point à trancher (il te propose des solutions) ;
   - ❌ bloquée (il t'explique pourquoi).
7. **Tu fusionnes** sur GitHub (bouton « Squash and merge »).
8. L'agent nettoie son dossier de travail.

Une PR n'est ouverte qu'à la **fin d'une tâche**, jamais pour des changements
intermédiaires. Si l'agent n'a pas tout fini, il te le dit avant.

## GitHub en pratique

- **`main` est protégée** : personne ne pousse directement dessus ; tout passe
  par une PR dont la CI est verte.
- **La CI** (GitHub Actions, sous Windows) vérifie chaque PR : formatage,
  clippy, tests, recherche de secrets. Une PR qui ne touche que la
  documentation ou les images passe en une minute (pas de compilation).
  Une croix rouge = ne pas fusionner ; l'agent doit corriger.
- **Étiquettes** : chaque PR a un **type** (`nouveauté`, `correctif`,
  `performance`, `maintenance`, `docs`, et `rupture` si elle casse une
  compatibilité), qui la range dans les notes de version, et une **zone**
  (`rendu`, `avatar`, `monde`, `réseau`, `audio-voix`, `interface`,
  `construction`) pour s'y retrouver.
- **Captures** : les images des PR sont rangées sur la branche `captures`
  (elle ne contient que ça).
- **Dependabot** ouvre chaque semaine des PR de mise à jour des dépendances ;
  un agent peut les vérifier pour toi.

### Publier une version

On suit [SemVer](https://semver.org/lang/fr/) : `vMAJEUR.MINEUR.CORRECTIF`.
Avant la 1.0, une nouveauté fait `v0.3.0 → v0.4.0`, un correctif
`v0.4.0 → v0.4.1`.

```bash
git switch main && git pull
git tag -a v0.2.0 -m "Aurora Viewer 0.2.0"
git push origin v0.2.0
```

GitHub Actions compile alors le viewer, crée la release avec le zip Windows et
écrit les notes, classées par catégorie, à partir des PR fusionnées.

## Installer l'environnement

1. **Rust** (stable) : <https://rustup.rs>. La version est gérée par
   `rust-toolchain.toml`, rien d'autre à faire.
2. **Git** et **GitHub CLI** (`gh auth login`).
3. Cloner le dépôt et les sources de Firestorm **côte à côte** :

   ```bash
   mkdir C:\Aurora_Viewer_v2 && cd C:\Aurora_Viewer_v2
   git clone https://github.com/odessadraekavik/Aurora_Viewer aurora-viewer
   git clone --depth 1 https://github.com/FirestormViewer/phoenix-firestorm
   ```

4. Ton identité git, avec ton adresse « noreply » GitHub pour ne pas exposer
   ton e-mail (Settings → Emails sur GitHub) :

   ```bash
   cd aurora-viewer
   git config user.name "Ton pseudo"
   git config user.email "12345+tonpseudo@users.noreply.github.com"
   ```

5. Télécharger la police emoji (trop lourde pour git) :

   ```powershell
   ./scripts/fetch-assets.ps1
   ```

## Compiler et lancer à la main

```bash
cargo build --release -p aurora-viewer
./target/release/aurora-viewer.exe
```

- Mode démo (aucune connexion) : `$env:AURORA_DEMO="1"` avant de lancer.
- Nommer la fenêtre : `aurora-viewer.exe --title "Mon test"`.
- Tout vérifier comme la CI : `./scripts/check.ps1`.
- Si un agent a son viewer ouvert, le `.exe` de `target/release` est verrouillé :
  les agents compilent dans leur propre dossier `..\work\<tâche>\target`.

### Place sur le disque

Les compilations prennent beaucoup de place : comptez 4 à 8 Go par dossier
`target` (le dépôt et chaque tâche en cours). Ce n'est que du cache, qu'on peut
toujours supprimer : il sera recompilé au besoin.

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
| Dossiers de travail des agents | `C:\Aurora_Viewer_v2\work\` |
| Sources Firestorm (référence) | `C:\Aurora_Viewer_v2\phoenix-firestorm\` |
| Réglages, skins, disposition | `%APPDATA%\Aurora\AuroraViewer\config\` |
| Cache (textures, mesh…) — peut être supprimé sans risque | `%LOCALAPPDATA%\Aurora\AuroraViewer\cache\` |
| Logs (à envoyer à un agent après un test) | `%LOCALAPPDATA%\Aurora\AuroraViewer\data\logs\` |
| Programme compilé | `target\release\aurora-viewer.exe` (ou `work\<tâche>\target\release\`) |

`aurora.log` est le log de ta dernière session réelle ; la session d'avant est
gardée en `aurora.previous.log`. Les sessions de démo ont leurs propres logs
(`aurora-demo*.log`).

## Sécurité

- Les agents ne se connectent jamais à la grille et ne manipulent jamais tes
  identifiants. Le mot de passe retenu est stocké par Windows (Gestionnaire
  d'identification), sous forme d'empreinte uniquement.
- La CI cherche des secrets dans chaque PR, et la relecture de l'agent aussi.
  Si tu vois passer un identifiant ou un jeton, ne fusionne pas et
  signale-le.
- Tu peux toujours demander à un agent de t'expliquer un changement avant de
  fusionner.
