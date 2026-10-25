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
  in UTC only; and on demand ("Run workflow" on main). A run whose commit
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
- **Signing.** Betas are not signed: code signing (SignPath, not set up
  yet) asks for a manual approval of every release, which a build every
  12 hours cannot wait for. Its step must skip the tags with a pre-release
  part (`-`). Beta testers are told to expect an unknown-publisher warning.

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
- its `.zip.sha256` file, in the format `hash  zip-name`;
- `aurora-update-policy.json`: `schema: 1`, `version: X.Y.Z`,
  `minimum_supported_version`, `cache_invalidation_before` and
  `settings_reset_before`: `null` or a stable SemVer version ≤ X.Y.Z;
  optionally `grace_hours`, 0 to 72 (absent: 12, above);
- `Aurora-Viewer-vX.Y.Z-windows-x64-symbols.zip` and its `.zip.sha256`: the
  exe's crash symbols ([crash reports](BUG_REPORTING.md#crash-symbols)),
  never downloaded by the viewer, which looks for the package's exact name;
- then `Aurora-Viewer-vX.Y.Z-linux-x64.tar.gz` and its `.tar.gz.sha256`,
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

Old releases without a hash or a manifest go through their page for a manual
installation. macOS remains out of scope: its application bundle is signed
and replaced as a whole. The current ZIP is kept: Velopack was studied, but
its chain of packages and installers is not that of the Aurora releases.

## Code signing

The Windows exe can be signed by [SignPath](https://signpath.io), with the
certificate of the [SignPath Foundation](https://signpath.org), free for
open source projects. A signed exe is what antivirus software (Norton,
Defender) and SmartScreen trust; an unsigned one may be blocked or
quarantined. The publisher Windows shows is "SignPath Foundation". The
policy page is <https://aurora-viewer.org/code-signing/> (website
repository, `code-signing/`).

The Release workflow signs only when the repository has the
`SIGNPATH_ORGANIZATION_ID` variable; without it, releases ship unsigned as
before. When it is set, after the build and the symbols:

1. the exe alone is uploaded as the run's artifact `aurora-viewer-unsigned`;
2. `signpath/github-action-submit-signing-request` sends it to SignPath with
   the version (the tag without its `v`) and waits up to an hour: **an
   approver of the project accepts each release by hand in SignPath**;
3. the signed exe replaces the built one, its signature must be valid and
   `scripts/verify-version.ps1` runs again on it;
4. the package, its `.sha256` and the release are made from the signed exe,
   and the release notes start with the code signing line and the policy
   link (SignPath's terms ask for it on the download pages).

Signing only adds a certificate to the file: its code and debug id do not
change, so the symbols zip, made before, still matches the published exe.
The media plugins pack (separate `media-plugins-<n>` releases) is not
signed: its binaries are built from, or are, upstream projects' code, which
SignPath leaves to their own maintainers.

### Setting it up (once)

What only a maintainer can do, never an agent (accounts, tokens):

1. Apply at <https://signpath.org/apply> for the open source subscription
   (the answers are drafted outside the repository). SignPath's conditions:
   OSI licence for every component, no proprietary component, builds from
   the public repository's CI, the code signing policy page online, MFA for
   every team member on GitHub and SignPath.
2. Once accepted, in SignPath: the project `aurora-viewer` linked to the
   trusted build system **GitHub.com**, the artifact configuration of
   [`.signpath/artifact-configuration.xml`](../.signpath/artifact-configuration.xml)
   (slug `initial`), the signing policy `release-signing` with the
   approvers of the policy page, and a CI user with the submitter role.
   SignPath's pipeline policies can also require GitHub-hosted runners, no
   re-runs and the branch ruleset of `main`.
3. In GitHub, Settings › Secrets and variables › Actions:
   - secret `SIGNPATH_API_TOKEN`: the CI user's API token;
   - variable `SIGNPATH_ORGANIZATION_ID`: the organization id (this one
     turns signing on);
   - optional variables `SIGNPATH_PROJECT_SLUG`,
     `SIGNPATH_SIGNING_POLICY_SLUG`, `SIGNPATH_ARTIFACT_CONFIGURATION_SLUG`
     when the slugs differ from `aurora-viewer`, `release-signing`,
     `initial`.
4. Optionally install the SignPath GitHub App on the repository (origin
   checks from the audit log).

At the next tag, the run waits at "Sign" until an approver accepts the
request in SignPath (link in the step's log). A refused or expired request
fails the run before anything is published: tag again after fixing it.
Properties › Digital Signatures of the published exe shows "SignPath
Foundation".

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
