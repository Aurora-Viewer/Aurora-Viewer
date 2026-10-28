# HUMANS.md — guide for humans

This repository is designed so that two people (or more) each work with
several AI agents (Claude Code, Codex…) in parallel. The agents follow
[AGENTS.md](AGENTS.md); this guide explains **your** role and how everything
fits together.

**To get started:** [clone the repository and double-click
`aurora-tools.cmd`](#installing-the-environment); after that, everything is
done with [the Aurora tools](#the-aurora-tools-aurora-toolscmd).

## What the agents do, what you do

| The agents | You |
|---|---|
| Code in their own working folder (one per task) | Choose the tasks to start |
| Test in **demo mode** (server simulated locally) | **Validate** each feature before the PR |
| Launch the viewer for you to test | **Log in and test on the AGNI grid** (they never do) |
| Open the PR, review it, fix, comment, **merge it** | Decide when the agent asks you to (verdict ⚠️ or ❌) |
| Keep TASKS.md up to date, publish the releases | **Ask** for a release when you want one ("release 0.4.0") |
| Read the issues and their screenshots / files, analyse them, set their type, reply and close them once resolved | Give the details and validations needed, decide the product choices |
| Answer your questions on what is left to do, the issues and the PRs from fresh data (`origin/main`, GitHub), never from memory | Ask whenever you want a status |

You have **nothing to do on GitHub**: your only job is to test what the agent
shows you, and to log in to the grid when needed.

Everything in the repository and on GitHub is written in English. You can talk
to your agent in French or in English: it answers in your language.

## Life of a task

1. You ask an agent for a task ("do gaze tracking like the Legacy Viewer").
2. The agent creates its working folder `..\work\<task>` on a branch of its
   own: it does not see the other agents' code in progress, and vice versa.
3. It codes, tests in demo mode, then **builds and launches the viewer for
   you** with a window named "Aurora Viewer <version> - Test <task> (Dev)";
   the title ends with the build profile: (Dev), (Release) or (Debug).
4. **You test** what it asks you to (in demo mode or on the grid: you are the
   one who logs in) and tell it whether it is good. For a PR with no effect
   on the viewer (documentation, CI, scripts), it just tells you what is
   going to change.
5. The agent opens the **pull request**: a readable summary (what it is for,
   whether it comes from the Legacy Viewer, what changes, screenshots).
6. Right after, without you having to ask, it waits for the end of CI then
   **reviews its PR**: conflicts with `main`, CI, non-regression, secrets…
   and posts a **review comment** ("Conversation" tab of the PR) with a
   verdict:
   - ✅ ready to merge;
   - ⚠️ a point to decide (it proposes solutions);
   - ❌ blocked (it explains why).
7. **If the verdict is ✅, it merges it**: the PR enters the **merge queue**,
   where GitHub tests it with the most recent `main` (tests included) and
   merges it if everything is green. If you and someone else merge at the
   same time, the PRs simply queue up: no more "Update branch". Otherwise
   (⚠️ or ❌), it stops and asks you to decide.
8. The agent cleans up its working folder and tells you: link of the PR, what
   changed, and what is left to test on the grid if anything.

A PR is only opened at the **end of a task**, never for intermediate changes.
If the agent has not finished everything, it tells you before. If a session
is interrupted, the agent of the next session first takes up the PRs without
a review or that left the queue.

## GitHub in practice

- **`main` is protected**, for everyone, admins included: nobody pushes to it
  directly; everything goes through a PR, then through the merge queue.
- **CI** (GitHub Actions, on Windows) has two stages:
  - on each PR, the quick checks: formatting, clippy, secret scan (the
    tests, the agent already ran them on its machine);
  - in the merge queue, everything, tests included, on the PR combined with
    the latest `main`: what lands on `main` is exactly what was tested.
  A PR that only touches documentation or images passes in a minute (no
  build). A red cross = the agent fixes it.
- **Labels**: each PR has a **type** (`feature`, `fix`, `performance`,
  `maintenance`, `docs`, and `breaking` if it breaks a compatibility), which
  files it in the release notes, and an **area** (`render`, `avatar`,
  `world`, `network`, `audio-voice`, `ui`, `chat`, `nameplate`,
  `contact-group`, `inventory`, `build-tools`, `tooling`) to find one's way
  around; the issues carry the same areas.
- **Screenshots**: the images of the PRs are stored on the `captures` branch
  (it contains nothing else).
- **Issues**: the agents also read the screenshots and attached files, set
  each report's type (Bug, Feature, Task or Epic) and area labels, then
  reply directly on GitHub. A Bug is the report, `fix` the PR. An issue
  stays open if information or a validation is missing; it is closed after a
  verified resolution and merge, or with an explanation for a duplicate / an
  out-of-scope request. The rules are detailed in
  [AGENTS.md](AGENTS.md#15-github-issues-read-analyse-type-reply-and-close);
  the labels are created by `scripts/setup-labels.ps1`, which keeps the
  other labels.
- **The project board** (GitHub project Aurora Team Planning): the project
  manager sets each issue's Priority and milestone, which say what to work on
  first. Even when you ask for a task directly, the agent first looks for an
  open issue that already covers it (often the sub-issue of an epic) and
  takes it. An agent that takes an issue assigns it to you and moves it to
  `In progress`; at the end it replies on the issue with the PR link and
  moves it to `Done`, to `Waiting Review` when the fix waits for your test or
  the QA's, or to `Waiting for user response` when it asked the reporter for
  something. `On hold` marks an issue paused on purpose, with a comment
  saying what unblocks it. Each PR names its issues (`Closes #<n>` or
  `Refs #<n>`), or says `No issue`. The agents move the Status with
  `scripts/issue-board.ps1`, which
  needs the `project` right on your `gh` account, once per machine:
  `gh auth refresh -s project`. Every new issue lands on the board in the
  `Triage` Status (the board's "Auto-add to project" and "Item added to
  project" workflows): the QA sorts it (type, areas, Priority) and moves it
  to `Todo`. The agents never take an issue out of `Triage` on their own.
- **Dependabot** opens dependency update PRs every week; an agent can check
  them for you, and merges them if you agree.
- **The wiki** is generated: after each merge that touches a guide, a
  workflow copies the README, this guide, `ARCHITECTURE.md`, `NOTICE.md` and
  the files of `docs/` to the Wiki tab, with a sidebar by audience (testers,
  contributors, technical). Never edit a page in the wiki: it is replaced at
  the next run; edit the Markdown file in the repository. Two settings are
  needed once, by hand, on GitHub: create a first page in the Wiki tab (its
  content does not matter, the wiki does not exist before that), and tick
  "Restrict editing to collaborators only" under Settings › Features ›
  Wikis. The workflow can then be started from the Actions tab ("Wiki" ›
  "Run workflow") or waits for the next change to a guide.
- **The repository** belongs to the `Aurora-Viewer` organisation; the
  developers are in the `Devs` team.

### Publishing a release

We follow [SemVer](https://semver.org/): `vMAJOR.MINOR.PATCH`. Before 1.0, a
feature makes `v0.3.0 → v0.4.0`, a fix `v0.4.0 → v0.4.1`.

The tag is the reference for the viewer's number: `v0.2.0` produces `0.2.0`,
shown in the title, the login screen and the Help menu. Between two tags, a
build also gives the number of commits and the short identifier, for example
`0.2.0+7.gabcdef123`. There is no number to change in `Cargo.toml` before
publishing. The workflow checks the number announced by the exe with
`--version` and refuses an inconsistent release.

The simplest way: the **Manage releases** choice of the Aurora tools (section
below). They check that `main` is green, suggest the number from the labels
of the PRs merged since the last version, show these PRs, create the tag and
follow the publication up to the release link. They are also used to
**delete** a release (you have to type its tag to confirm; `assets-1`, which
holds the 3D emoji font older commits download, is protected). Write access to the repository is
required.

Before the creation is confirmed, two choices set the effect of the update
for users: **keep or invalidate the cache**, then **keep and migrate the
settings automatically** (the usual choice) or **reset them**. Migrations
must be implemented in the viewer; an incompatible break can force a return
to the defaults. The update window announces these effects before installing.
The choices are recorded in the annotated tag, then published with the
release; they still apply to users who skip a version. The other files of
the profile are not reset.

After a release, the agent that published it moves the finished tasks of
`TASKS.md` to `docs/TASKS_ARCHIVE.md` (a small `docs` PR), so that the list
stays short. If you published the release yourself with the tools, ask an
agent: "archive the finished tasks".

You can also ask an agent: "release 0.4.0" (or "make a release": it suggests
the number). In both cases, GitHub Actions builds the viewer, creates the
release with the Windows zip, adds the Linux archive and writes the notes,
sorted by category, from the merged PRs. By hand, it is:

```bash
git switch main && git pull
git tag -a v0.4.0 -m "Aurora Viewer 0.4.0"
git push origin v0.4.0
```

### Publishing a news item

The "News" panel of the login screen displays three files of the repository,
written by hand: [news/en.md](news/en.md), [news/fr.md](news/fr.md) and
[news/ja.md](news/ja.md), one per interface language. The viewer reads them on `main` at each start: **no
need to publish a release** to announce something.

Add the news item **at the top** of each file (the most recent first), under
the comment that recalls the format:

```markdown
## 2026-10-08 — Title of the news item
One or more lines of plain text.
[Learn more](https://example.org/optional-link)
```

- **Title**: `## `, the date `YYYY-MM-DD`, then "—" (a plain "-" or ":" also
  works) and the title. The date is optional.
- **Text**: plain text, without bold, list or image. An empty line starts a
  new paragraph.
- **Link**: optional, alone on its line, and in `https://` (any other link is
  ignored). A click on the news item opens it in the browser.
- **Limits**: the first 20 news items are displayed; a title is cut at 120
  characters, a text at 600. Remove the oldest ones from time to time.
- **Languages**: write the news item in every file. An interface language
  that has no file displays `en.md`.

Since `main` is protected, the change goes through a PR (label `docs`): the
GitHub pencil on the file, then "Create a new branch and start a pull
request", or ask an agent. `en.md` and `fr.md` are also compiled into the
viewer (that is what it displays offline as long as it has downloaded
nothing; Japanese shows `en.md` there until `ja.md` is compiled in too), so
the PR goes through the whole CI in the merge queue; a test there checks
that they read correctly (empty title, text too long).

After the merge, GitHub takes about five minutes to serve the new file; each
viewer displays it at its next start and keeps it for offline launches. To
see the news item in the viewer **before** merging, launch it with
`AURORA_NEWS_URL` (see [docs/TESTING.md](docs/TESTING.md#interface)).

## Installing the environment

1. Create a folder for the project (for example `C:\Aurora-Viewer`), open a
   command prompt in it (in Explorer: type `cmd` in the address bar, then
   Enter) and clone the repository:

   ```bat
   git clone https://github.com/Aurora-Viewer/Aurora-Viewer aurora-viewer
   ```

2. Double-click **`aurora-viewer\aurora-tools.cmd`** (the Aurora tools, next
   section). They check what is missing and offer **Repair / update**:
   - they install what is missing with winget: GitHub CLI (for the whole
     machine), the Visual Studio C++ tools that Rust needs to build (several
     GB), and Rust (rustup, for your account, as Rust wants on Windows);
     Windows permission prompts may open;
   - they clone the Legacy Viewer sources into `phoenix-firestorm\`, next to the
     repository;
   - they download the Rust toolchain and all the crates;
   - they install sccache with winget (for your account), the compiler cache
     shared by the repository and the agents' tasks (see
     [Building and launching by hand](#building-and-launching-by-hand));
   - they log you in to GitHub (`gh auth login`, in the browser) if needed,
     and set your git identity in the repository: your GitHub user name and
     your "noreply" address (your e-mail stays private).

**No Git yet?** In a command prompt opened in the project folder, this
command installs everything, Git included, then opens the tools (it first
shows what it is going to do and asks where to install):

```bat
powershell -NoProfile -ExecutionPolicy Bypass -Command "$f = Join-Path $env:TEMP 'aurora-setup.ps1'; irm https://raw.githubusercontent.com/Aurora-Viewer/Aurora-Viewer/main/scripts/setup.ps1 -OutFile $f; & $f"
```

From here on, **the project folder** is the one of the installation: it
contains `aurora-viewer\`, `phoenix-firestorm\`, `work\` (the agents' tasks)
and, later, `RELEASE\`.

## The Aurora tools: `aurora-tools.cmd`

Double-click **`aurora-tools.cmd`**, at the root of the repository (or the
"Aurora Tools" shortcut the tool can create). Everything is done with the
keyboard: **↑ ↓** to choose, **Enter** to confirm, **Esc** or **Backspace**
to go back, and the key shown in front of each choice as a shortcut.

The launcher loads the scripts present in `scripts/tools/` each time it
opens. It also works when placed in the project folder, next to
`aurora-viewer/`; alone in an empty folder, it downloads the installer from
the `main` of GitHub. On the command line, it passes the arguments to the
tools and returns their exit code, for example
`aurora-tools.cmd -Action diagnose`. Exclamation marks in the path stay valid
even from a prompt with delayed expansion.

The active choice carries a **›** mark on a violet background; the text
cursor stays hidden while navigating. Long lists scroll with the selection
and the menus adapt when the console is resized. Creating a release splits
the number, the cache and the settings into three screens, with the chosen
version and a summary before confirmation.

When it opens, the tool checks the environment: tools, repository behind
GitHub, outdated Legacy Viewer sources, Rust not up to date, font, GitHub
login… If something is missing or outdated, the first choice is **Repair /
update** (the same installation as the one-line command: what is there is
updated, nothing is reinstalled). If that update changes the Aurora Tools
scripts, the tool restarts automatically in the same console and reopens its
menu with the new behaviour. This also applies to the `-Action repair`
action; a repair that does not change the tools keeps the current session.

To check these behaviours without installing or publishing anything:
`powershell -NoProfile -File scripts/test-tools.ps1` tests the menus in
hidden Windows consoles and the restart on isolated temporary scripts.

| Choice | What it does |
|---|---|
| Release viewer | The latest `main` of GitHub in `--release`, in `RELEASE\` (see below), then "launch?" |
| Dev viewer | Your local repository, default profile (the agents' one) |
| Debug viewer | Your local repository with a console, the debug controls and full debug information (`debugging` profile, its own folder of a few GB) |
| Demo | The viewer offline, with the choice of an `AURORA_DEMO_*` scenario (read from [docs/TESTING.md](docs/TESTING.md)) |
| Agent tasks | The folders of `work\`: PR, changes in progress, last build, size; cleanup of those whose PR is merged |
| Disk | The space taken by the builds, and the cleanup (`clean.ps1`) |
| Viewer logs | Open the folder, read the end of the last log, copy its path for an agent |
| PR tracking | The live panel (see below) |
| Manage releases | Create or delete a release (see [Publishing a release](#publishing-a-release)) |
| Shortcuts | "Aurora Tools" and "Aurora PR" (the PR panel directly) on the Desktop and in the Start menu, with the logo |
| Claude permissions | Lets your Claude Code agents follow their PRs and CI, post their review and their screenshots, merge their PR, triage the issues and reply to them (`gh pr view / list / checks / comment`, `gh run view / list / watch`, `gh issue list / view / edit / comment / close / reopen`, `merge-pr.ps1`, `pr-captures.ps1`), which auto mode can otherwise refuse them ("auto-approval", "write to an external system", "interference with workloads"). `gh api`, `git push`, `gh pr merge`, the releases and `end-task.ps1` stay without a rule, left to the judgement of auto mode. Adds these rules to your Claude Code settings (`%USERPROFILE%\.claude\settings.json`) without touching the rest, keeping a copy of the former file; the merge queue and the protection of `main` still apply. Run it again after a tools update that adds a rule: only the missing rules are added. Then open a new Claude Code session |
| Check again | The full table of the environment |
| Reset the repository | Start again from a fresh clone of the `main` of GitHub: the tool first lists everything that would be lost (uncommitted changes, commits never pushed, stashes, agents' tasks with their changes, ignored files such as a `.env`), refuses if a build or a viewer is running, and does nothing unless you type `RESET`. It then deletes the tasks of `work\` and `RELEASE\`, puts the repository back on `origin/main` (untracked files and builds erased, local branches and stashes deleted), then repairs the environment. Your git identity, your GitHub login and the Legacy Viewer sources are kept |

**Live PRs.** As long as the tools are open, the bottom banner sums up the
PRs ("PR: 3 open · 1 CI · 1 queued · 1 red"), and a **Windows notification**
reports a merged PR, a CI that turns red, a PR that left the merge queue, a
new PR, or `main` breaking. The **PR tracking** panel (or the "Aurora PR"
shortcut) has three sections: the **merge queue** (position and duration),
the **open PRs** (CI with its running duration, verdict of the agent's
review) and the **recently merged** ones (how long ago: "04 min"), with for
each PR its author and its added and deleted lines (**+120** in green,
**-8** in red). Each state is a tag preceded by a square of its colour:
`■ CI OK` (green), `■ CI FAIL clippy` (red), `■ CI RUN 4:00` (amber),
`■ QUEUE #1 TESTS`, `■ AUTO-MERGE`, `■ REVIEW OK`, `■ REVIEW WARN` (to
decide), `■ REVIEW FAIL`, `■ REVIEW PENDING` (not reviewed yet). It adapts to
the size of the window: shrink it and keep it in a corner of the screen (too
low, it first hides the latest merges, then scrolls the PRs with the
selection). **↑ ↓** choose, **Enter** open the PR in the browser, **R**
refresh. GitHub is queried every 10 to 20 seconds, no more: GitHub's quota
(5,000 requests per hour) is shared with your agents.

Each choice also exists without the menu, for a script:
`./scripts/tools/aurora-tools.ps1 -Action diagnose` (`repair`, `release`,
`dev`, `debug`, `demo`, `tasks`, `disk`, `logs`, `prs`, `releases`,
`reset`, `shortcut`, `claude`); `-DryRun` shows what `releases`, `reset` and
`claude` would do without doing it.

## The up-to-date viewer of `main`, in `--release`

The **Release viewer** choice of the tools, or a double-click on
`RELEASE\build-release.bat` in the project folder. The ready-to-launch exe is
in `RELEASE\Aurora-Viewer\` (the same folder as the zip of the releases: the
avatar files and the emoji font are inside the exe),
and `version.txt` gives the version announced by the viewer and the commit
built.

- The code **always comes from GitHub**: your local changes and your
  unpushed commits do not go in.
- The first build takes a few minutes; the next ones only rebuild what
  changed.
- The ready-to-launch folder is prepared with Windows PowerShell 5.1 as well
  as with PowerShell 7.
- `build-release.bat 42` builds PR no. 42 (to test its performance before
  merging it).
- Close the viewer of that folder before rebuilding (the script says so).

## Building and launching by hand

```bash
cargo run -p aurora-viewer
```

- The default profile (without `--release`) is the agents' one: it behaves
  like the published version, without the slow optimisation of `--release`.
  `cargo build --release` is still possible to measure performance (it
  rebuilds everything a second time).
- Demo mode (no connection): `$env:AURORA_DEMO="1"` before launching.
- Naming the window: `cargo run -p aurora-viewer -- --title "My test"`.
- Checking everything like CI: `./scripts/check.ps1`.
- The agents build in their own folder `..\work\<task>\target`: their open
  viewers do not block yours.
- **Compiler cache.** With sccache installed (Repair / update), the
  repository and every task share the dependencies they already built, kept
  in `.sccache\` in the project folder (10 GB at most; set
  `AURORA_BUILD_CACHE_SIZE`, for example `20G`, then run `check.ps1` or
  `new-task.ps1` again). Cargo finds it through `.cargo\config.toml` in the
  project folder, which the scripts write while sccache is installed and
  delete once it is uninstalled. To stop using it, uninstall sccache
  (`winget uninstall Mozilla.sccache`), then run `check.ps1` once (until then
  Cargo stops with "could not execute process"). The first build of a new
  task takes about a third less processor time (seven crates in ten come from
  the cache; the others depend on a build script that writes into the
  task's own folder), and a task rebuilt after its `target\` was freed about
  half.
- **One heavy build at a time.** `check.ps1` waits before clippy and the
  tests while two other heavy builds already run on the PC ("Waiting for a
  build slot…"): eight agents compiling at once each take every core and
  all finish later. `AURORA_BUILD_SLOTS` changes the number (2 by default).

### Disk space

Builds take space: count 3 to 4 GB per `target` folder (the repository, each
task in progress, `RELEASE`). It is only cache, which can always be deleted:
it will be rebuilt when needed. A new task frees by itself the `target` of
the tasks that are merged or inactive for 7 days.

The **Disk** choice of the tools does the same as:

```powershell
./scripts/clean.ps1                 # shows the space taken, deletes nothing
./scripts/clean.ps1 -Apply          # frees the builds of the repository (keeps target\release)
./scripts/clean.ps1 -Tasks -Apply   # also frees those of the tasks in progress
```

The script only deletes Cargo build folders, and refuses to work during a
build or when a viewer is open.

## Where the files are

The Windows packages check for updates at launch, in the background. "Skip
this version" hides its automatic offer; "Check for updates", in Help and on
the login screen, brings it back. Preferences → Interface → Updates: ask (by
default), download automatically or required updates only. The automatic
download waits for your agreement to install and restart.

The viewer verifies the ZIP, keeps the former folder until the first frame is
displayed and restores it if the start fails. Use a dedicated folder, with
write access; folders that contain code or personal files require a manual
installation. Settings and cache stay in the usual folders. Development and
demo builds do not check the releases. See [docs/UPDATES.md](docs/UPDATES.md)
for the format of the packages and their policy.

| What | Where |
|---|---|
| Viewer code | `crates/` (see [ARCHITECTURE.md](ARCHITECTURE.md)) |
| Task tracking | [TASKS.md](TASKS.md) (open work and what was finished since the last release); older finished tasks in [docs/TASKS_ARCHIVE.md](docs/TASKS_ARCHIVE.md) |
| Agents' working folders | `work\`, in the project folder |
| Legacy Viewer sources (reference) | `phoenix-firestorm\`, in the project folder |
| Settings, skins, layout | `%APPDATA%\Aurora\AuroraViewer\config\` |
| Conversation history (text files, per account and per grid; Preferences › Chat to open it, turn it off or erase it) | `%APPDATA%\Aurora\AuroraViewer\config\accounts\<account uuid>\chat_logs\<grid>\` |
| Cache (textures, mesh…) — can be deleted safely | `%LOCALAPPDATA%\Aurora\AuroraViewer\cache\` |
| Logs (to send to an agent after a test) | `%LOCALAPPDATA%\Aurora\AuroraViewer\data\logs\` |
| `--release` viewer of the latest `main` | `RELEASE\Aurora-Viewer\aurora-viewer.exe`, in the project folder |
| Program built by hand | `target\debug\aurora-viewer.exe` (or `work\<task>\target\debug\`) |

`aurora.log` is the log of your last real session; the session before is kept
as `aurora.previous.log`. Demo sessions have their own logs
(`aurora-demo*.log`).

## Security

- The agents never connect to the grid and never handle your credentials.
  The remembered password is stored by Windows (Credential Manager), as a
  hash only.
- CI looks for secrets in each PR (and in the merge queue), and so does the
  agent's review. If you see a credential or a token go by, tell the agent
  to stop and report it.
- You can always ask an agent to explain a change to you, or not to merge a
  PR before you have looked at it.
