# Windows package updates

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
  The update manifest is another matter: betas carry a signed one like
  every release ([Update signature](#update-signature)), with no approval.

`scripts/test-nightly.ps1` holds the fixture tests of the versions, the
next tag, the pruning and the policy line; the Nightly workflow runs them on
the pull requests that change these files. `./scripts/nightly-tag.ps1
-DryRun` prints the tag the next nightly would create. A beta update is
tried locally with `scripts/test-update.ps1 -Beta`
([TESTING.md](TESTING.md#interface)).

## Package and release policy

The Release workflow publishes together:

- `Aurora-Viewer-vX.Y.Z-windows-x64.zip`, with a single root folder of that name;
- its `.zip.sha256` file, in the format `hash  zip-name`;
- `aurora-update-policy.json`: `schema: 1`, `version: X.Y.Z`,
  `minimum_supported_version`, `cache_invalidation_before` and
  `settings_reset_before`: `null` or a stable SemVer version ≤ X.Y.Z;
- `Aurora-Viewer-vX.Y.Z-windows-x64-symbols.zip` and its `.zip.sha256`: the
  exe's crash symbols ([crash reports](BUG_REPORTING.md#crash-symbols)),
  never downloaded by the viewer, which looks for the package's exact name;
- `aurora-update-manifest.json` and `aurora-update-manifest.json.sig`: the
  [signed manifest](#update-signature) of the zip and of the policy, added
  by the sign job, which then publishes the release (created as a draft);
- then `Aurora-Viewer-vX.Y.Z-linux-x64.tar.gz` and its `.tar.gz.sha256`,
  built on Ubuntu 24.04 by a second job once the release exists: a failed
  Linux build leaves the Windows release published.

On Linux the viewer checks the releases the same way, with the same policy
(a required update still blocks the login), but does not install them: its
"Open the release page" button leads to the archive, extracted by hand
([Installing on Linux](LINUX.md#updates)).

`scripts/package.ps1` adds `aurora-install.json` (`schema: 1`, version read
by `--version`). The exe, the assets and the licence are mandatory in the
package. The default thresholds come from `update-policy.json` at the root.
`null` makes every update optional. Below a minimum version, the update is
required: neither "Skip this version" nor Later, login blocked until the
update. The preferences and the skipped tag do not remove this check.
Changing this policy is a release decision reviewed in a PR; it takes
effect at the next release.

In **aurora-tools.cmd → Manage releases → Create a release**, the Cache and
Settings choices come before the confirmation. Keeping the cache and the
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
installation. Platforms other than Windows remain out of scope. The current
ZIP is kept: Velopack was studied, but its chain of packages and installers
is not that of the Aurora releases.

## Update signature

Anyone able to publish a GitHub release could otherwise ship a package, or
a policy that blocks the login, to every viewer that updates. Each release
therefore carries `aurora-update-manifest.json`: its version, then the
name, size and SHA-256 of the Windows zip and of `aurora-update-policy.json`,
signed with Ed25519 (`aurora-update-manifest.json.sig`, the signature in
hex). The viewer holds the public keys (`UPDATE_KEYS`,
`crates/aurora-updater/src/signature.rs`) and accepts a release only when:

- the manifest and its signature are there and the signature holds against
  one of those keys (a list, so that a key can be replaced: the new one is
  added in a release still signed with the old one);
- the manifest names the release's version;
- the zip has the manifest's size, and its download the manifest's SHA-256
  (the `.sha256` files stay for people checking a download by hand; the
  viewer no longer reads them);
- the policy has the manifest's bytes, and is there exactly when the
  manifest lists it.

Otherwise it logs `update refused: …` and offers nothing: the check fails as
an integrity failure (shown on a manual check; an automatic check only logs
it and does not block the login), and a package that does not match at
download is not installed. Linux and macOS read the policy the same way.
The media plugins pack needs no manifest: its size and SHA-256 are compiled
into the viewer. Viewers published before this change do not check
signatures; nothing changes for them.

**Signing.** The Release workflow creates the release as a draft. Its
`sign` job runs in the GitHub environment `release`, the only place that
holds the private key (secret `UPDATE_SIGNING_KEY`): it downloads the zip
and the policy from the draft, writes and signs the manifest with
`aurora-update-sign`, uploads both files and publishes the draft. Without
the secret the job fails and the release stays a draft. A pull request's
dry run signs nothing and says so. The tool refuses a key whose public half
is not in `UPDATE_KEYS`, and while that list is empty a build with
`AURORA_VERSION_TAG` (the Release workflow) does not compile: no release can
go out that its viewers could never update from.

**Setting up the key** (a maintainer with admin rights, once, before the
first signed release):

1. Generate the key on your own PC, into a folder only you can read and
   outside the repository:

   ```powershell
   cargo run -p aurora-updater --bin aurora-update-sign -- keygen --out <private folder>urora-update-key.txt
   ```

   The private key goes to that file and is never shown. The tool prints a
   `key("…")` line: the public key.
2. Paste that line into `UPDATE_KEYS` in
   `crates/aurora-updater/src/signature.rs`, replacing the TODO, in a pull
   request.
3. In the repository's Settings › Environments, create the environment
   `release`. Under "Deployment branches and tags", choose "Selected
   branches and tags" and add the branch `main` (the nightly betas come
   from it) and the tag pattern `v*` (the stable tags). Make sure the tag
   ruleset lets only administrators (and the nightly's GitHub Actions) create
   `v*` tags, betas included: a workflow started by a tag runs that tag's
   own copy of the workflow, so whoever may push a `v*` tag may reach the
   key. Required reviewers would stop the nightly betas at every run: leave
   them out, or accept approving each beta.
4. Add the private key as a secret **of that environment** (never a
   repository secret, which every workflow could read), without it appearing
   on screen:

   ```powershell
   Get-Content -Raw -LiteralPath <private folder>urora-update-key.txt | gh secret set UPDATE_SIGNING_KEY --env release --repo Aurora-Viewer/Aurora-Viewer
   ```

5. Keep the key file offline (an encrypted backup); delete other copies.
   Without it, no release can be signed for the installed viewers any
   more: each would have to be updated by hand. A key that leaks is
   replaced the same way as an old one (new key added, then the old one
   removed), as soon as possible.

`aurora-update-sign verify --manifest aurora-update-manifest.json --tag
vX.Y.Z <zip> <policy>` checks a downloaded release against the compiled
keys. `scripts/test-update.ps1` signs its local release with a throwaway
key that stays in memory; only its public half goes into the test
configuration (`update_key`), trusted on that loopback feed alone.

## Replacement and rollback

Download, hash and extraction run in the background. Extraction refuses
path traversals, links, Windows duplicates, reserved files and archives
exceeding the size limits. The staging is done next to the installation, on
the same volume. The installation refuses repositories, build trees, folder
links and folders containing files foreign to the package.

After the agreement to restart, a copy of the viewer acts as a helper
outside the installed folder. It waits for the end of the parent with a
Windows handle, takes a lock, backs up the old folder and puts the new one
in place. The arguments are kept. The new instance signals a presented
frame; the backup is then deleted. If it exits or shows nothing for
90 seconds, the old version is restored and restarted with a message. The
errors offer the release page.

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

`cargo test -p aurora-updater`: loopback HTTP server and throwaway packages
for SemVer, required update, signed manifest (missing or altered signature,
wrong key, another version, altered or removed policy, altered package),
tampered hash, incomplete transfer,
cancellation, dangerous ZIP, lock, backup and rollback of the package and of
the data. `scripts/test-update-policy.ps1` checks the release choices, their
carry-over into an annotated tag and the refusal of an invalid threshold,
without touching the current repository. The visual demo downloads nothing
(`AURORA_DEMO_UPDATE`). The human test with two real executables is
described in [TESTING.md](TESTING.md#interface).
