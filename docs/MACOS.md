# Installing Aurora Viewer on macOS

Every release of Aurora Viewer comes with a macOS application for Apple
Silicon Macs (M1 and later). It draws through Metal, Apple's graphics API:
nothing else to install. macOS support is new: tell us what does not work
(below).

## Requirements

- A Mac with Apple Silicon (M1, M2, M3, M4 or later). Intel Macs are not
  supported.
- macOS 12 Monterey or later.

## 1. Download and install

1. On the [latest release](https://github.com/Aurora-Viewer/Aurora-Viewer/releases/latest),
   download `Aurora-Viewer-<version>-macos-arm64.dmg`.
2. Open it (a double click in the Finder): a window shows **Aurora Viewer**
   next to your **Applications** folder.
3. Drag **Aurora Viewer** onto **Applications**, then eject the disk image
   (the eject button next to "Aurora Viewer" in the Finder's sidebar).

## 2. Start the viewer

Open **Aurora Viewer** from Applications or the Launchpad.

If macOS says that it cannot check the application or its developer, the
release you downloaded is not notarized by Apple yet. Open it once this way:

1. Click **Done** (or **OK**) on that message.
2. Open **System Settings › Privacy & Security**, scroll down to the
   message about Aurora Viewer and click **Open Anyway**, then confirm with
   your password or Touch ID.

macOS remembers the choice: the next starts are normal. In the Terminal,
this command does the same, once the application is in Applications:

```bash
xattr -dr com.apple.quarantine "/Applications/Aurora Viewer.app"
```

Until the releases are notarized, macOS may also ask again, after an
update, whether Aurora Viewer may use the keychain (your saved password:
choose **Always Allow**) and the microphone.

The first time voice chat opens the microphone, macOS asks whether Aurora
Viewer may use it: allow it to speak in voice chat (you can change it later
in **System Settings › Privacy & Security › Microphone**).

## Keyboard

Keyboard shortcuts follow the Legacy Viewer. **Command and Control both
trigger them** (Command+S and Control+S; Command+W and Control+W close the
front window; Command+Shift+W and Control+Shift+W close every window).
The mouse keeps the two keys apart: Control-click is not Command-click,
and Command does not change the camera wheel.

A few shortcuts are labeled with the Control key (⌃), the same list as the
Legacy Viewer: Nearby Chat (⌃H), Conversations (⌃T), the World Map (⌃M),
Highlight Transparent (⌃⌥T), Snapshot to Disk (⌃`). Command runs those too,
except Nearby Chat. Command+H hides the application, as it does in the
Legacy Viewer, and Control+H opens Nearby Chat (or brings it forward, or
closes it when it is already in front). The others are labeled with
Command (⌘), then Option (⌥) and Shift (⇧).
Text fields accept both keys for copy, paste, undo and the rest.
Select All is Command+A (or Control+A). Command+Shift+A still opens People,
as in the Legacy Viewer, even while a field is focused.
**Command+Q** asks before quitting, like the close button of the window.
macOS takes Command+Q and Command+H before the viewer sees them. The Quit
item is answered here; Command+H stays Hide.

## Web pages and video

Web pages and video in the world need the media plugins, which are not part
of the application: the first time they are needed, the viewer downloads
Aurora's media plugins pack (about 335 MB) into the `media-plugins` folder of
its settings (table below), once. A Legacy Viewer installed on the Mac can
lend its plugins instead: in **Preferences › Media**, type the path of its
application (its `.app` in `/Applications`) as the plugins folder.

## Updates

The viewer tells you when a new version is out, but it cannot replace
itself on macOS yet: its "Open the release page" button leads to the new
disk image. Install it as in step 1, replacing the old application. Your settings,
cache and logs are kept: they are not inside the application.

| What | Where |
|---|---|
| Settings, logs | `~/Library/Application Support/org.Aurora.AuroraViewer` |
| Cache | `~/Library/Caches/org.Aurora.AuroraViewer` |
| Saved passwords | the macOS keychain (Keychain Access) |

To see these folders in the Finder: **Go › Go to Folder…** (Cmd+Shift+G) and
paste the path.

## Not on macOS yet

- Automatic installation of updates (above).
- Opening `secondlife://` links from a web browser.
- The crash report window.

## Building from source

Install Apple's command line tools and Rust, then build:

```bash
xcode-select --install
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
git clone https://github.com/Aurora-Viewer/Aurora-Viewer aurora-viewer
cd aurora-viewer
cargo build --release --locked -p aurora-viewer
./target/release/aurora-viewer
```

`AURORA_DEMO=1 ./target/release/aurora-viewer` starts the offline demo,
without the network. To build the application itself as the release does,
run `pwsh ./scripts/package-macos.ps1 -Exe target/release/aurora-viewer -Out "Aurora Viewer.app"`
(PowerShell 7: `brew install powershell`). The same PowerShell runs the local
checks: `pwsh -File ./scripts/check.ps1` (formatting, clippy, tests).

## Signing (for the maintainers)

The macOS job of the release workflow signs the application with the
project's Developer ID, then has Apple notarize it, when the repository has
these Actions secrets; without them it is signed ad hoc and opens as in
step 2:

| Secret | Value |
|---|---|
| `MACOS_CERTIFICATE_P12` | the Developer ID Application certificate and its key, a `.p12` in base64 |
| `MACOS_CERTIFICATE_PASSWORD` | the password of that `.p12` |
| `APPLE_TEAM_ID` | the team of the Apple Developer account |
| `APPLE_API_KEY_ID` | an App Store Connect API key (Developer role): its id |
| `APPLE_API_ISSUER_ID` | the issuer id of that key |
| `APPLE_API_KEY_P8` | the content of the key's `.p8` file |

## Reporting a problem

See [Reporting bugs](BUG_REPORTING.md), and say that you are on macOS, with
your Mac's chip (Apple menu › About This Mac) and macOS version.
