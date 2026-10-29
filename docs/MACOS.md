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

The viewer's shortcuts are those of Windows and Linux, with the **Control**
key (Ctrl+S, Ctrl+Shift+I…), as in the Legacy Viewer. Text fields follow
macOS (Cmd+C, Cmd+V, Cmd+A, Cmd+arrows). **Cmd+Q** asks before quitting, like
the close button of the window.

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

- Aurora's own media plugins pack (web pages and video in the world). Meanwhile, a
  Legacy Viewer installed on the Mac lends its plugins: in **Preferences › Media**,
  type the path of its application (for example `/Applications/Firestorm.app`) as the
  plugins folder.
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
(PowerShell 7: `brew install powershell`).

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
