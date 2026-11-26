# Aurora Viewer on iPad and iPhone (prototype)

Aurora Viewer builds as an iPadOS app, drawn through Metal, with touch
controls made for it. **This is a demo, a prototype**: it is here to be
tried and broken, not to replace the desktop viewer. It has not been run on
a real iPad yet: the first testers are the ones who find out.

The same app installs on an **iPhone**, with a layout of its own for the
small screen ([On an iPhone](#on-an-iphone)). Everything this page says of
the iPad holds there unless that section says otherwise.

- **No download is published.** Apple only lets an iPad run an app that is
  signed for it, so each tester signs the app with their own Apple ID. This
  page explains how, with a free Apple ID.
- **What has been checked**: the touch controls with scripted fingers in the
  demo, and the same build drawn on an Apple GPU as a Mac app (Mac
  Catalyst). The first runs on a real iPad are the testers': tell us what
  you see ([Reporting a problem](#reporting-a-problem)).
- **Use an account you can afford to break things with**, and keep the
  desktop viewer for anything that matters.

## Requirements

- An iPad on **iPadOS 26**. It is developed for the iPads with an M-series
  chip (iPad Pro, iPad Air); it has not been run on the others. On
  iPadOS 27 the app is not expected to open yet
  ([Known limits](#known-limits)).
- A USB cable, and an **Apple ID**. A free one works, with Apple's limits
  for free accounts: the app stops opening after **7 days** and is then
  installed again (your settings stay), and an iPad holds three such apps at
  a time. A paid developer account signs for a year.
- For the route that builds the app: a **Mac** with Xcode 26 or later.

## Two ways to get it on your iPad

| | You need | For |
|---|---|---|
| **A. Build and install it from a Mac** | a Mac with Xcode, 30 to 60 minutes the first time | anyone with a Mac; the route this page details |
| **B. Sign an `.ipa` someone built** | a Windows PC or a Mac, and a sideloading tool | testers without a Mac, when a developer hands them the `.ipa` |

## Before either route: Developer Mode on the iPad

iPadOS only runs an app signed with a personal Apple ID once Developer Mode
is on.

1. Connect the iPad to the computer with the cable, unlock it and tap
   **Trust** when it asks whether to trust this computer.
2. On the iPad: **Settings › Privacy & Security › Developer Mode**, turn it
   on, and let the iPad restart. (The line only appears once the iPad has
   been connected to a Mac with Xcode open, or once an app signed this way
   has been installed.)
3. After the restart, unlock the iPad and confirm **Turn On**.

Developer Mode stays on until you turn it off at the same place.

## A. Build and install it from a Mac

### 1. Install the tools

```bash
xcode-select --install          # nothing to do when Xcode is already installed
sudo xcode-select -s /Applications/Xcode.app   # the tools of Xcode itself, not the command line tools alone
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
brew install powershell         # the packaging script is a PowerShell 7 script
git clone https://github.com/Aurora-Viewer/Aurora-Viewer aurora-viewer
cd aurora-viewer
rustup target add aarch64-apple-ios
```

Open Xcode once so that it finishes installing its components, and check
that **Xcode › Settings › Components** has the iOS platform.

### 2. Let Xcode make your signing identity

A free Apple ID gets its certificate and its provisioning profile from
Xcode, when Xcode runs an app on the iPad. An empty app does it:

1. **Xcode › Settings › Accounts**: add your Apple ID with **+**.
2. **File › New › Project… › iOS › App**. Give it a product name, for
   example `AuroraSideload`, choose your name followed by **(Personal
   Team)** as the team, and an organization identifier of your own, for
   example `com.yourname`. The app's identifier is then
   `com.yourname.AuroraSideload`: Aurora Viewer will be installed under it.
3. At the top of the window, choose your iPad as the destination, then
   **Product › Run**. If Xcode says that the iPad's system is older than the
   app's, set **Minimum Deployments** to iOS 26.0 in the target's
   **General** tab and run again.
4. The first time, the iPad refuses to open the app: on the iPad, **Settings
   › General › VPN & Device Management**, tap your Apple ID under
   "Developer App", then **Trust**. Run again from Xcode: an empty app
   opens. Close it.

Xcode now has a certificate ("Apple Development: …") and a profile that
names your iPad. Find both:

```bash
security find-identity -v -p codesigning
ls -t ~/Library/Developer/Xcode/UserData/Provisioning\ Profiles/*.mobileprovision | head -1
```

The first command prints the identity between quotes; the second, the
newest profile, which is the one Xcode just made.

### 3. Build, sign and install

```bash
IPHONEOS_DEPLOYMENT_TARGET=26.0 cargo build --release --locked -p aurora-viewer --target aarch64-apple-ios
pwsh ./scripts/package-ios.ps1 -Exe target/aarch64-apple-ios/release/aurora-viewer -Out dist \
    -Identity "Apple Development: Your Name (ABCDE12345)" \
    -Provision ~/Library/Developer/Xcode/UserData/Provisioning\ Profiles/<the file>.mobileprovision
xcrun devicectl list devices
xcrun devicectl device install app --device "<your iPad's name>" "dist/Aurora Viewer.app"
```

The build takes about 6 minutes on an M2 Max the first time, longer on a
smaller Mac.
`IPHONEOS_DEPLOYMENT_TARGET` is required: without it the build stops at the
link, or asks for a newer iPadOS than yours. The script signs the app under
the identifier of the profile, so Aurora Viewer takes the place of the empty
app of step 2.

Aurora Viewer is now on the Home Screen. Start with
[the offline demo](#the-offline-demo): it needs no account. The login
screen is the computer's; a login from an iPad has not been tried yet, so
use an account you can afford to break things with.

**To quit**, use the viewer's own menu (the Aurora button, top left, then
"Quit Aurora Viewer"): it logs out and saves. Swiping the app away from
the app switcher ends it at once, without the logout.

**After 7 days** (free Apple ID) the app no longer opens: run the empty
project again from Xcode, which renews the profile, then
`package-ios.ps1` again with the new profile (the `ls -t` command above
finds it) and the `install app` command. Your settings stay.

### The offline demo

The demo world (a plaza, avatars, objects, without the network or an
account) is started from the Mac, the iPad unlocked:

```bash
xcrun devicectl device process launch --device "<your iPad's name>" --console --terminate-existing \
    --environment-variables '{"AURORA_DEMO": "1"}' com.yourname.AuroraSideload
```

`--console` shows the viewer's messages in the Terminal, and
`--terminate-existing` ends the viewer first if it is still open. The other
`AURORA_*` variables of [TESTING.md](TESTING.md) are given the same way.

### The log

The viewer's log is inside the app, where Files does not show it. From the
Mac:

```bash
xcrun devicectl device copy from --device "<your iPad's name>" \
    --domain-type appDataContainer --domain-identifier com.yourname.AuroraSideload \
    --source "Library/Application Support/org.Aurora.AuroraViewer/logs" --destination aurora-logs
```

Never post a whole log: [Reporting bugs](BUG_REPORTING.md) says what to
send.

## B. Sign an `.ipa` someone built

Without `-Identity`, the packaging script writes
`Aurora-Viewer-<version>-ios-unsigned.ipa`: the app, not signed for any
device. A sideloading tool signs it with your Apple ID and installs it, from
a Windows PC or a Mac. AltStore, SideStore and Sideloadly are such tools.
They are other projects' programs, not ours: follow their own instructions,
and know that they ask for your Apple ID (some testers make a second Apple
ID for this).

1. Turn Developer Mode on (above).
2. In the tool, choose the `.ipa`, your iPad and your Apple ID, and install.
3. On the iPad: **Settings › General › VPN & Device Management**, tap your
   Apple ID, then **Trust**.
4. Open Aurora Viewer from the Home Screen.

The same 7 days apply; these tools can renew the app for you.

Only install an `.ipa` from someone you trust: it is a program that will
hold your Second Life password.

## Touch controls

| Gesture | Does |
|---|---|
| Tap | a click: touch an object, sit, press a button of a HUD |
| Touch and hold (half a second) | the menu of what is under the finger (an avatar, an object, the ground); in a text field, Cut, Copy and Paste; on a row of the inventory or a name, its menu |
| Drag with one finger | turns the camera around your avatar; in mouselook, looks around |
| Pinch | zooms |
| Drag with two fingers | moves the camera sideways and up and down |
| Tap with two fingers | puts the camera back behind the avatar; when the interface is hidden, brings it back |
| Left thumb, lower left of the world | the movement stick: up and down walk forward and back, sideways turns, the rim of the stick runs. In mouselook, sideways steps aside. |
| Buttons, lower right | up (jump, or rise when flying), down (crouch, or descend), fly, mouselook, reset view |

The stick appears where the thumb lands. A thumb on the ring drawn at rest
is always the stick's; beside it, in the same corner, a thumb that does not
move taps or holds the world as anywhere else. The windows, the bars and
the menus work with a finger as with a mouse, and the touch buttons answer
a mouse or a trackpad too. A first launch starts with the People and
Conversations windows closed, so that the world and the controls show: the
bar at the bottom opens them.

One difference from the computer: in the People and Contacts lists, holding
a row opens its menu (the right click), not the contact's settings, which
the menu gives.

**Preferences › Controls › Touch screen** turns the touch controls on or
off; "Automatic", its default, shows them on a tablet from the start and on
a computer from its first touch.

## On an iPhone

The package is one app for both: install it on an iPhone the same way
(Developer Mode, then route A or B, choosing the iPhone as the device).

- **Held sideways only.** The viewer stays in landscape: upright, a phone's
  screen is too narrow for the bars.
- **The touch buttons take two rows** in the lower right, under the
  mini-map, and the lines of the nearby chat start beside the movement
  stick, not over it.
- **The interface is the computer's, made smaller**, not a phone's: the
  windows open over most of the screen, and one at a time is the comfortable
  number. Close a window to get the world and the controls back.
- **It is developed for the recent iPhones** (the Pro models of the last
  years): a busy region asks for more memory than iOS gives an app on a
  smaller one, and the system then closes the viewer without a message.
- It has been checked at an iPhone's screen size in the demo on a computer,
  with scripted fingers. **It has not been run on a real iPhone.**

## Files

- **Save as** (a photo, a chat log, an export) has no dialog on iPadOS: the
  file goes to the app's **Exports** folder, which the Files app shows under
  **On My iPad › Aurora Viewer**.
- **Open** shows the system's file picker. A file you pick is handed to the
  viewer as a copy; a folder you pick stays readable until the app quits.
- **Photos** saved to the disk go to the app's **Photos** folder, at the
  same place in Files. There is no bin on iPadOS: a photo deleted from the
  viewer's photo reel is deleted for good, and the viewer says so before.

## Known limits

- Web pages and video inside the viewer (media on a prim, the web windows)
  are not available: iPadOS does not let an app start the plugin programs
  they run in. Links open in Safari.
- A hardware keyboard types text, but its shortcuts (Ctrl or Cmd with a
  letter, the arrow keys to walk) do not reach the viewer yet.
- A window can cover the touch buttons: move it, or close it.
- Held upright, an iPad's top bar is crowded: the bars are laid out for a
  wide screen. Turn the iPad sideways.
- The build tools open, but an object is not dragged with a finger yet: a
  drag turns the camera. The movement stick is off while they are open.
- iPadOS pauses the app when you leave it, and the region logs you out after
  about a minute of it: come back quickly, or log in again.
- **iPadOS 27**: Apple asks the apps built with its newest tools for a way
  of starting (scenes) that winit, the library the viewer's window comes
  from, does not have yet. The app is expected not to open there until it
  does. Not tried.
- Voice: the microphone and the sound are set up for it, and nobody has
  spoken through it on an iPad yet. With a Bluetooth headset, the sound
  goes to the headset and the iPad's own microphone listens: taking the
  headset's microphone would drop the sound to telephone quality.
- A window cannot be moved out of the viewer's (the "outside" windows of
  the computer): iPadOS gives the app one window.
- A busy region can ask for more memory than iPadOS gives an app: the
  system then closes the viewer without a message.
- No update notice installs anything: build or sign the new version again.
- A pencil, a trackpad or a mouse acts as a finger: their hover and their
  wheel are not delivered to the viewer (winit), so nothing lights up under
  the pointer and the lists scroll by dragging.
- What starts another program stays on the computer: the video recorder
  (ffmpeg), the external script editor, opening a folder. Game controllers
  and 3D mice too: their libraries have no iPadOS support.
- Not in the App Store nor on TestFlight.

## Reporting a problem

See [Reporting bugs](BUG_REPORTING.md). Say that you are on an iPad, which
one (**Settings › General › About**: model name and iPadOS version), whether
you use a keyboard, a trackpad or a pencil, and how the app was installed
(route A or B). A screenshot (top button and volume up) or a screen
recording says the most about a touch problem.

## For developers

- The touch controls run on every system: `AURORA_DEVICE=tablet` gives a
  computer the tablet's interface, and `AURORA_DEMO_TOUCH=<scenario>` plays
  fingers in the demo ([TESTING.md](TESTING.md), "Camera, movement and
  pointer").
- The same code runs as a Mac app through Mac Catalyst
  (`rustup target add aarch64-apple-ios-macabi`, then `cargo build -p
  aurora-viewer --target aarch64-apple-ios-macabi`): put the executable in
  `Aurora Viewer.app/Contents/MacOS/`, `packaging/ios/Info.plist` with its
  version filled in as `Contents/Info.plist`, sign it with `codesign --force
  --sign -`, and start it with `open` (started as a bare executable it never
  becomes the active app, and draws nothing).
- The iOS simulator cannot draw the viewer: its GPU driver lacks the
  texture binding arrays the renderer needs. It is useful to check that the
  app installs and starts, no further.
- What the viewer asks of UIKit is in `crates/aurora-viewer/src/ios.rs` and
  `crates/aurora-ios-files`; [ARCHITECTURE.md](../ARCHITECTURE.md) has the
  rest ("Touch screens, iPadOS").
