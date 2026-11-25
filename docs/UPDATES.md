# Package updates

The viewer queries the public releases in the background and shows the login
screen immediately. Its button stays disabled during the initial check
(eight seconds at most), then if the update is required. The reason is
shown under the button and also applies to Enter and to accepting the terms.
Only stable SemVer tags are considered, plus the betas on the Beta channel
(below); drafts, other pre-releases and `assets-1` are ignored. The Git
metadata after a tag does not change its SemVer priority. Without a
network, the automatic check logs the failure without a window and releases
this login check. Development builds and the demo do not check the
releases. The manual check shows its result and bypasses the skipped tag.

The dialog stays sharp in front of a background blurred on the GPU, without
a grey veil. A required offer shows only Quit and Update; neither Esc nor a
click outside can close it. The login button stays disabled as long as this
update is required.

**Grace period** ([#3014](https://github.com/Aurora-Viewer/Aurora-Viewer/issues/3014)).
Below a new minimum version, the viewer may still log in for the release's
grace period: 12 hours unless the policy says otherwise (`grace_hours`).
It counts from the first time this installation sees that minimum (saved in
the settings, `update_grace`), so a relaunch or a later release with the
same minimum does not start it again, and a clock set back does not stretch
it. During it, the offer shows "Sign in anyway (N h left)" and Update, with
no "Skip this version"; it comes back at each launch. Once it is over, the
offer is the required one above. A session already in the world is never
cut. The Legacy Viewer blocks at once, as a release with `grace_hours: 0`
does.

The modes take up the choices of the Legacy Viewer's `UpdaterServiceSetting`:
required only, ask, download automatically. Aurora chooses "Ask before
downloading" by default, to meet the request for an offer at launch;
The Legacy Viewer chooses 0. The automatic mode prepares the files and waits for
the user's agreement before quitting and restarting their session.

## Beta channel

Preferences › Language and updates › Versions chooses **Stable** (the
default) or **Beta** (`update_channel`). Beta testers get the changes of
main a few days before a stable version, to catch crashes before everyone.

- **Builds.** The Nightly workflow (`.github/workflows/nightly.yml`) runs
  every 12 hours, at 11:00 and 23:00 UTC: noon and midnight in France in
  winter (CET), 1 pm and 1 am in summer (CEST), since a GitHub schedule is
  in UTC only; and on demand ("Run workflow" on main, by an admin of the
  repository: anyone else's run, or a beta tag they push, stops before
  anything is tagged or published). A run whose commit
  already has a version tag (nothing new since the last beta or release)
  does nothing. When main has a commit without a version tag, `scripts/nightly-tag.ps1` pushes
  the next beta tag on it and the Release workflow builds and publishes it
  (it is called by the Nightly workflow: a tag pushed with the workflow
  token starts no workflow by itself), as a GitHub pre-release with the
  same files as a stable release: the Windows zip, its symbols (crash
  reports of betas are symbolized like the others), the update policy and
  the Linux archive.
- **Versions.** The patch after the newest stable version, then
  `beta.<UTC date>.<n>`: after v0.7.6, `v0.7.7-beta.20261025.1` at 11:00
  UTC, `.2` at 23:00 UTC the same day. SemVer ranks every beta above the earlier ones and
  below the stable version of its number, so a beta tester moves on to
  v0.7.7 (or any later stable) once it is out, then to the next betas.
  The beta's update policy takes over the thresholds of the newest stable
  tag (`Aurora-Update-Policy` line), under its own version, raised to those
  of `update-policy.json` at the tagged commit, as a release does; a
  minimum the beta itself does not reach is left out.
- **Retention.** `scripts/nightly-prune.ps1` keeps the ten newest betas and
  deletes the older ones with their tags; stable releases and other
  pre-releases are never touched.
- **What follows the channel.** The Stable channel, the website and the
  login news read stable releases only (GitHub's pre-release flag must
  agree with the version: a stable number marked pre-release is skipped).
  A beta build keeps checking for updates; a development build (Git
  distance after the tag) never does.
- **Back to Stable.** The installed beta stays until a stable version newer
  than it is out, usually the stable of the same number a few days later:
  no downgrade, since a beta may already have migrated the settings or the
  cache in a way the older stable does not read. Switching channel checks
  again at once.
- **Signing.** Betas are not signed: code signing ("Code signing" below)
  asks for a manual approval of every release, which a build every 12 hours
  cannot wait for, so the Release workflow skips the tags with a
  pre-release part (`-`). Beta testers are told to expect an
  unknown-publisher warning.

`scripts/test-nightly.ps1` holds the fixture tests of the versions, the
next tag, the pruning and the policy line; the Nightly workflow runs them on
the pull requests that change these files. `./scripts/nightly-tag.ps1
-DryRun` prints the tag the next nightly would create. A beta update is
tried locally with `scripts/test-update.ps1 -Beta` (`--beta` for
`scripts/test-update-linux.py`), in an isolated installation
([TESTING.md](TESTING.md#interface)).

## Package and release policy

The Release workflow publishes together:

- `Aurora-Viewer-vX.Y.Z-windows-x64.zip`, with a single root folder of that name;
- its `.zip.sha256` file, in the format `hash  zip-name`, and the file's
  signature, `.zip.sha256.sig` ([signed updates](#signed-updates));
- `aurora-update-policy.json` and its signature,
  `aurora-update-policy.json.sig`; the policy holds `schema: 1`, `version: X.Y.Z`,
  `minimum_supported_version`, `cache_invalidation_before` and
  `settings_reset_before`: `null` or a stable SemVer version ≤ X.Y.Z;
  optionally `grace_hours`, 0 to 72 (absent: 12, above);
- `Aurora-Viewer-vX.Y.Z-windows-x64-symbols.zip` and its `.zip.sha256`: the
  exe's crash symbols ([crash reports](BUG_REPORTING.md#crash-symbols)),
  never downloaded by the viewer, which looks for the package's exact name;
- then `Aurora-Viewer-vX.Y.Z-linux-x64.tar.gz`, its `.tar.gz.sha256` and
  `.tar.gz.sha256.sig`,
  built on Ubuntu 24.04 by a second job and added once the release exists:
  a failed Linux build leaves the Windows release published. The tarball's
  single root folder, of that name, holds the viewer (`aurora-viewer`), its
  desktop entry and icon, with the executable bit kept.

The viewer installs the package of its system, named after the release tag:
the ZIP on Windows, the tar.gz on Linux. The format is read from the asset's
name, and each one names its own root folder and executable. macOS installs
its signed application by hand: the offer links the release page
([Installing on macOS](MACOS.md#updates)).

`scripts/package.ps1` adds `aurora-install.json` (`schema: 1`, version read
by `--version`). The exe, the assets and the licence are mandatory in the
package. The default thresholds come from `update-policy.json` at the root.
`null` makes every update optional. Below a minimum version, the update is
required: neither "Skip this version" nor Later, login blocked until the
update. The preferences and the skipped tag do not remove this check.
Changing this policy is a release decision reviewed in a PR; it takes
effect at the next release.

In **aurora-tools.cmd → Manage releases → Create a release**, the Minimum
version, Cache and Settings choices come before the confirmation. The
minimum keeps the current one by default (the higher of `update-policy.json`
and the last stable tag), can require the release's own version (every
earlier installation must update) or take an earlier X.Y.Z typed by hand,
never below the current minimum. A new minimum then asks for its **grace
period**: 12 hours (the usual choice, written as no `grace_hours`), 24, 72
or 6 hours, or none (`grace_hours: 0`, login blocked at once, for a release
that cannot wait). A release, or a beta, that keeps the minimum keeps its
grace period without asking. A pre-release is never offered its own
number: `0.9.2-beta.1` comes before `0.9.2`, so its users would be refused
it, and an inherited minimum it does not reach is left out, as in the
nightly. Keeping the cache and the
settings is the usual choice; `Settings::sanitized` applies the migrations
coded in the viewer. Forcing an action records the release version as the
threshold: every earlier installation will have to apply it. The previous
thresholds are taken over from the last annotated stable tag, including for
a user who skips releases. They no longer apply once this version is
installed.
The tag contains an `Aurora-Update-Policy: {…}` line; the workflow validates
it with `scripts/write-update-policy.ps1` before publishing the policy file.
A tag created by hand without this line uses the thresholds of the versioned
file. To also keep the choices made earlier in the tools, use an annotated
tag with this line (or copy the thresholds into the versioned file).
The interface announces separately what happens to the cache and to the
settings, both before the download and before "Install and restart Aurora".

A release without a valid signature is not offered at all (below). macOS
remains out of scope: its application bundle is signed
and replaced as a whole. The current ZIP is kept: Velopack was studied, but
its chain of packages and installers is not that of the Aurora releases.

## Signed updates

Anyone with write access on the repository can create or edit a release by
hand on GitHub (GitHub has no separate right for it), and replace a package
and its `.sha256` file. So the viewer trusts neither on their own
(`crates/aurora-updater/src/signature.rs`):

- the publish jobs of the Release workflow (also run by the Nightly) sign
  each package's `.sha256` file (it names the package, so its version) and
  the update policy with Ed25519, through `scripts/sign-release.ps1`:
  `<file>.sig` holds the 64 raw bytes of the signature over the whole file,
  one per signing key (two while the key changes, below). The macOS disk
  image's `.sha256` is signed too;
- the key is the `UPDATE_SIGNING_KEY` secret (and, while it changes,
  `UPDATE_SIGNING_KEY_NEXT`) of the repository's `release` environment,
  which only the branch `main` and the stable tags
  (`vX.Y.Z`, reserved to the admins by the tag ruleset) may use. A beta
  tag (`v*-beta.*`) is left out: anyone with write access can push one,
  from any commit, and the Nightly's betas are built from `main`. No agent
  ever handles the private key;
- the viewer is built with the public keys (`RELEASE_KEYS`). A check looks
  at the three newest releases above the installed version, newest first,
  and offers the first whose `.sha256` signature checks out, whose
  `.sha256` names this system's file, and whose policy, when there is one,
  is signed too. A release made by hand, or whose files were replaced, is
  skipped and logged (`update vX.Y.Z: not offered, …`); the genuine one
  below it is offered instead. The download must then match the signed
  hash. A signature file that is listed but fails to download fails the
  check, which is tried again later;
- `sign-release.ps1` checks each signing key against `RELEASE_KEYS` before
  anything is published: a secret that is not listed fails the run rather
  than publishing a release no viewer accepts. A pull request that
  changes the release files tries the script on a key of its own (the
  Linux job's dry run).

The releases published before the signatures carry none; a viewer of an
earlier version still installs any package whose `.sha256` matches, until
it has updated once. From the first signed version on, an unsigned release
is never offered: only the releases above the installed version are looked
at, and all of them are signed.

### Setting up the key

Once, by an admin, from the worktree of the pull request that brings the
key in (`gh` logged in with the admin's account, openssl from Git for
Windows):

```powershell
./scripts/setup-release-signing.ps1
```

It creates the `release` environment with its rules (branch `main`, the
stable tags up to `v99.99.999`), makes the key on the PC, stores it in the
first empty secret (`UPDATE_SIGNING_KEY`, then `UPDATE_SIGNING_KEY_NEXT`)
and deletes the file (`-Backup <file>` copies it first, to keep offline),
and writes its public half into `RELEASE_KEYS`, with the secret's name and
the date. The private key is never printed; no one can read the secret
back. `-EnvironmentOnly` puts the environment's rules back without touching
the keys. The environment and its secret must exist before the pull request
that names it is merged: a job naming an environment that does not exist
makes GitHub create it without rules, and the Nightly would fail without
the key.

### Changing the key

The viewers trust only the keys they were built with, so a new key comes
in beside the old one:

1. run `./scripts/setup-release-signing.ps1` again: the new key goes into
   the empty secret, and its public half into `RELEASE_KEYS`. From the
   merge of that pull request on, every release is signed with both keys;
2. once most viewers run a version that knows the new key, delete the old
   key's secret first (`gh secret delete <name> --env release`), then
   remove its line from `RELEASE_KEYS` in a pull request: a secret whose
   key is no longer listed stops the publish jobs.

A viewer that never got a version of step 1 no longer updates by itself:
its user downloads the new version from the release page. If a key leaks,
change it the same way at once, without waiting at step 2, and say so in
the release notes.

## Code signing

The Windows exe can be signed by [SignPath](https://signpath.io), with the
certificate of the [SignPath Foundation](https://signpath.org), free for
open source projects. A signed exe is what antivirus software (Norton,
Defender) and SmartScreen trust; an unsigned one may be blocked or
quarantined. The publisher Windows shows is "SignPath Foundation". The
policy page is <https://aurora-viewer.org/code-signing/> (website
repository, `code-signing/`).

Two workflows sign, both through the composite action
`.github/actions/signpath-sign` (the SignPath project `Aurora-Viewer`, the
artifact configuration `initial` of
[`.signpath/artifact-configuration.xml`](../.signpath/artifact-configuration.xml)):

- **Sign test** (`sign-test.yml`), run by hand from the Actions tab on any
  branch: it builds the release exe, has it signed with the project's
  **test certificate** (signing policy `test-signing`, no approval), checks
  that the exe carries a signature and keeps it as the run's artifact
  `aurora-viewer-test-signed` for three days. It publishes nothing. The
  test certificate chains to no trusted root: Windows shows that signature
  as not trusted, which is expected. It needs the variable and the secret
  below, not `SIGNPATH_SIGN_RELEASES`.
- **Release** (`release.yml`), for a stable tag once the repository has the
  variable `SIGNPATH_SIGN_RELEASES` set to `true`; without it, releases ship
  unsigned as before. After the build and the symbols:
  1. the exe alone is uploaded as an artifact of the run;
  2. `signpath/github-action-submit-signing-request` sends it to SignPath
     under the signing policy `release-signing`, with the version (the tag
     without its `v`) that the artifact configuration checks against the
     exe, and waits up to an hour: **an approver of the project accepts
     each release by hand in SignPath** (link in the step's log);
  3. the signed exe replaces the built one: its signature must be one
     Windows trusts, and `scripts/verify-version.ps1` runs again on it;
  4. the package, its `.sha256` and the release are made from the signed
     exe, and the release notes start with the code signing line and the
     policy link (the SignPath Foundation's terms ask for it on the
     download pages).

  A refused or expired request fails the run before anything is published:
  tag again after fixing it. Betas (tags with a pre-release part, the
  nightly's) are never signed, see "Beta channel" above.

Signing only adds a certificate to the file: its code and debug id do not
change, so the symbols zip, made before, still matches the published exe.
The media plugins pack (separate `media-plugins-<n>` releases) is not
signed: its binaries are built from, or are, upstream projects' code, which
SignPath leaves to their own maintainers.

### Setting it up

What only a maintainer can do, never an agent (accounts, tokens):

1. In SignPath (the open source subscription is granted): the project
   `Aurora-Viewer` linked to the trusted build system **GitHub.com**, the
   artifact configuration `initial` replaced by the content of
   `.signpath/artifact-configuration.xml` (a zip with the exe at its root:
   GitHub hands its artifacts over as zips), and the CI user as a
   submitter of `test-signing` and `release-signing`. Both policies
   require the trusted build system; `release-signing` also uses the
   approval process, with the approvers of the policy page.
2. In GitHub, Settings › Secrets and variables › Actions:
   - secret `SIGNPATH_API_TOKEN`: the CI user's API token (Regenerate token
     in SignPath, pasted straight into GitHub);
   - variable `SIGNPATH_ORGANIZATION_ID`: the organization id, shown in the
     CI Integration tab of a signing policy.
3. Run **Sign test**, download its artifact and check Properties › Digital
   Signatures of the exe: a signature by the test certificate.
4. Once the SignPath Foundation has reviewed the setup and imported the
   production certificate (`release-signing` turns valid): the variable
   `SIGNPATH_SIGN_RELEASES` = `true`. The next stable tag waits at "Sign
   (SignPath)" for an approver; Properties › Digital Signatures of the
   published exe then shows "SignPath Foundation".
5. Optionally, the SignPath GitHub App on the repository (origin checks:
   branch, tag, build definition) and the origin verification of
   `release-signing`.

## Replacement and rollback

Download, hash and extraction run in the background. Extraction refuses
path traversals, links, Windows duplicates, reserved files and archives
exceeding the size limits, in the ZIP as in the tar.gz; the tar.gz accepts
only regular files and folders, never a link, a device node or a sparse
file, and keeps the permission bits of the archive without setuid. The
staging is done next to the installation, on the same volume. Only the
package's own entries are replaced (the viewer, `assets`, the notices,
`aurora-install.json`): whatever else the folder holds, a shortcut or the
user's notes, stays where it is. A folder without `aurora-install.json`
(the viewer copied alone, onto the Desktop for example) has only the
viewer's executable replaced. Links among those entries, and a folder at the
root of a drive, are refused, as is a viewer without its executable bit.
Development builds never update.

After the agreement to restart, a copy of the viewer acts as a helper
outside the installed folder. It waits for the end of the parent (a Windows
handle, or the parent's `/proc` entry on Linux, with its start time so that a
reused PID is not taken for it), takes a lock, moves the old package entries
into the backup and puts the new ones in place. The arguments are kept. The
new instance signals a presented frame; the backup is then deleted. While
it starts, it also signals each step (a pipeline compiled, the next phase):
if it exits, goes 90 seconds without a step, or shows no frame after 15
minutes, the old version is restored and restarted with a message. A new
version that does not signal its steps gets 90 seconds in all, as before.
The rule is the helper's, a copy of the version being updated: it applies
from the updates made by a version that has it. The errors offer the
release page.

Where the folder cannot be replaced, the viewer does not try: a sandbox owns
it as a read-only package (Flatpak, Snap, AppImage), or the account cannot
write beside it (a system package installed by root). The window then says
so and links the release page, for the package manager or the bundle to
update.

Settings, layout, accounts and cache are outside the replaced folder. If the
policy requires it, the helper moves the cache and/or `settings.json` to a
sibling backup `*.aurora-before-update-<UUID>`, after the parent has closed.
This also works if the profile and the installation are on different disks.
The new viewer recreates its cache and its default settings; the other files
of the profile are kept. If the first start fails, the newly created data
is removed and the backups are restored. On success, these backups are
deleted; a clean-up error keeps them for recovery and does not trigger a
rollback. A sudden interruption of the PC can prevent the automatic
rollback; `.aurora-update-<UUID>/backup`, next to the installation, keeps
the old version. Do not delete it before having restored the installed
folder. Failed transactions are kept for diagnosis; the new instance cleans
up the successful transactions after the helper has finished.

## Validation

The workspace tests: loopback HTTP server and throwaway packages
for SemVer, required update, tampered hash, incomplete transfer,
cancellation, dangerous archives (ZIP and tar.gz, with their permission
bits), lock, backup and rollback of the package and of the data.
`scripts/test-update-policy.ps1` checks the release choices (minimum
version included, typed or refused), their carry-over into an annotated tag and the refusal of an invalid threshold,
without touching the current repository. The visual demo downloads nothing
(`AURORA_DEMO_UPDATE`). The human test with two real packages is
described in [TESTING.md](TESTING.md#interface).
