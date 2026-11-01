# Alpha bug reports

"Report a bug", in the top bar, the Help menu and on the login screen, opens
the project's
[GitHub issue forms](https://github.com/Aurora-Viewer/Aurora-Viewer/issues/new/choose)
in the browser. The tester needs a GitHub account, chooses the bug report
template and fills it in on GitHub. "Copy trace" and "Open report folder",
in the crash window, give the full technical details, to review before
sharing them (the full trace may contain personal paths).

## Crash reports

After a crash, "Send a crash report" in the standalone window shows the
issue to publish: exactly `public-report.md` (below), preceded by what the
tester writes in "What were you doing when it happened?" (at least a few
words: publishing waits for them, since a stack alone rarely says what led
to it) and their answer to "My graphics card drivers are up to date" (the
driver found is shown beside it). Nothing is published without those
words, whatever the Preferences say. "Connect to
GitHub" starts the GitHub App's Device Flow (`crash/github.rs`): the tester
enters the code shown on github.com, in their browser; the viewer never sees
a password. "Publish on GitHub" then creates the issue in the tester's
name, with no label (the crash triage below sets the type). "Open in the
browser" opens a new issue with the same title and text instead (text in
the clipboard when the link would be too long).

The authorization is kept in the system credential store (Windows
Credential Manager, entry "Aurora Viewer GitHub"), never in the settings,
the reports or the logs: an expiring user token and its refresh token,
refreshed without any client secret (allowed for the Device Flow) and
replaced at each refresh. "Forget the GitHub connection", in the crash
window or Preferences › Privacy, deletes it; the tester can also revoke the
app on github.com. A revoked or expired authorization is forgotten at the
next attempt. A creation is never retried automatically (a lost reply may
mean the issue exists); `sent.txt`, in the session folder, keeps the issue
number so a reopened window never sends twice. No server of our own and no
file upload: nothing else leaves the PC.

Preferences › Privacy › Crash reports chooses Ask (the default), Always or
Never. Always opens the crash window straight on the report to publish; it
still waits for the tester's words (a report without them could rarely be
reproduced), so nothing is ever sent by itself. Never removes the send
button: the report stays on the PC. The Legacy Viewer always sends by
default, to its own server; here a report is a public issue in the
tester's name, hence Ask.

Each crash session folder also holds `public-report.md`: what an automatic
crash report would publish (version, commit, crash kind, system, GPU, the
stack as `module+0xoffset` with the modules' PDB identities, the panic
location inside the sources, recognised warning categories). It never
contains raw addresses, panic messages, paths or log lines; the
`<!-- aurora-crash v1 -->` marker at its top is for the repository's crash
triage.

What each part tells the one who fixes it:

- **Crash**: `Rust panic`, `Native exception 0x…`, `Startup or event loop
  failure`, `GPU not responding` (the graphics card still held a frame when
  the session ended, the window answering: closed from the stall notice or
  by other means; the stack is the render thread's, waiting in the driver,
  and the row "GPU not responding" says for how long), `Freeze` (the window
  was not answering when the session ended: the stacks are those of the main
  thread, then of the render thread, read while it was frozen),
  `Out of memory (N bytes)` (an allocation failed and the process failed
  fast: the size asked for and the stack of the allocation, written by the
  allocator itself before the abort) or `Unexpected shutdown` (the process
  vanished with no word: killed, or a fast fail other than out of memory).
  A stack overflow is a `Native exception 0xC00000FD` with its stack,
  written by a vectored handler on the stack Windows keeps for it.
- **Thread**, when a long-lived background thread the session cannot do
  without panicked (network runtime, texture pages, frees, disk caches,
  chat history): the viewer ends the session at its next frame instead of
  going on without it, and the report adds the thread's name to the
  panic's stack.
- **Exit code**, when the watcher saw the end, with its usual meaning:
  `0x00000001` is what Task Manager's End task gives (often after a
  freeze), `0xC0000409` a fast fail (abort, out of memory), `0xC00000FD`
  a stack overflow.
- **End of session**, when the watcher did not see it (it stopped with the
  viewer): the duration then comes from the last sign of life (`alive.txt`,
  every 5 s), and a Windows start right after it says the system went down
  (system crash, power loss). A shutdown or restart of Windows itself is
  heard by the watcher and never reported. On Linux, a viewer stopped by
  SIGTERM, SIGHUP or SIGINT (log out, shutdown, a closed terminal, Ctrl+C)
  is not reported either.
- **Window**: answering until the end, or not answering for the last N s.
- **GPU device lost**: the renderer's device-lost callback fired (driver
  reset, GPU removed).
- **Overlay hooks**: the overlay or capture programs that injected their
  hook into the viewer during the session (Discord, Steam, RivaTuner, OBS
  game capture), named from the viewer's fixed list only.
- **Last state**: screen, frame rate, frame time, window size, graphics
  preset, memory, video memory used / budget, texture memory pressure,
  objects, avatars, regions, as noted every 2 seconds, and the swapchain
  recreations of the last 10 seconds (window resizes among them), frozen
  when the graphics card is lost.
- **Recent warnings and errors**: the last 20, as
  `-<seconds before the end> s | level | Rust module | kind`, repeats folded
  (`(x3)`); the messages themselves stay in the local log.

### Crash symbols

Each Windows release carries `Aurora-Viewer-vX.Y.Z-windows-x64-symbols.zip`:
the Breakpad symbol file of the exe, made by `dump_syms` from the release
build's PDB (line tables only) and stored as
`aurora_viewer.pdb/<debug id>/aurora_viewer.sym`. The debug id is the
`id=` of the `aurora_viewer` line in a report's modules: the file of the
version that crashed turns each `aurora_viewer+0xoffset` into a function
and a source line. A pull request that changes the Release workflow keeps
the file of its dry run as the run's artifact. The PDB itself is not
published. To resolve a stack by hand, unzip the file and look up, for
`offset - 1` (a return address follows its call), the `FUNC` record that
contains it and the line record under it; or use the triage script below.

### Crash triage

The Crash triage workflow (`.github/workflows/crash-triage.yml`) runs on
every new issue whose body carries the `<!-- aurora-crash v1 -->` marker.
`scripts/crash-triage.ps1` reads the issue through the API and validates
each value again (anyone can write an issue). For a released version whose
symbol file has the report's debug id, it resolves the viewer's frames,
inlined functions included, to functions and source lines. It then:

- appends a "Symbolized stack" section between
  `<!-- crash-triage:start -->` and `<!-- crash-triage:end -->` (a new run
  replaces it), ending with the crash's signature;
- computes the signature from the crash kind (panic, freeze, GPU stall, or
  the exception code) and the first three meaningful frames by name (for a
  freeze, those of the main thread), without generic arguments
  or closure numbers. The panic and exception machinery, the crash handler,
  the process start and the system modules are skipped, so the same crash
  keeps its signature across versions and offsets. Without symbols
  (development build, release older than the symbols), the signature is the
  build's debug id and its first offsets;
- when an open issue already has the same signature, comments there with
  the new occurrence (version, system, GPU) and closes the new issue as its
  duplicate; otherwise renames the issue `Crash: <kind> in <function>` and
  sets the Bug type.

No label is needed: the signature lives in a hidden comment of the body. A
closed issue is not a reference: a crash that comes back after its fix
opens a new issue. "Run workflow" with an issue number triages it again,
for example once the symbols of its version are published. Locally, on a
crash session's `public-report.md` and an unzipped symbol file:
`./scripts/crash-triage.ps1 -ReportFile <public-report.md> -SymbolFile <aurora_viewer.sym>`
prints the section, title and signature without writing to GitHub.
`scripts/test-crash-triage.ps1` holds the fixture tests; the workflow runs
them on the pull requests that change the triage.

The issue templates are in `.github/ISSUE_TEMPLATE/`; they set the issue
type (Bug or Feature); the area labels (`chat`, `ui`, `avatar`…) are added
during triage. The disabled form below
still sends the former `bug` and `alpha` labels (`bug_report/github.rs`):
switch it to the Bug type when it is brought back.

## The built-in form (disabled)

Until 2026-10-15 these buttons opened a report form inside the viewer. It is
disabled for now; its code stays in the tree, out of the build, to be
brought back later: uncomment every line marked "Bug report form (disabled,
see main.rs)" (`main.rs` lists its files). The rest of this page describes
that form, as it works once enabled, and its GitHub setup.
`AURORA_DEMO_BUG_REPORT` has no effect while it is disabled.

The shared form can be reached from the top bar, Help and the login screen.
"Send from Aurora" is the first choice and the default mode of new reports.
The sending options are independent of the report template.
The browser mode is available without any application setup; both modes
require the tester to have a GitHub account. Without an account, a local
copy remains possible.

After a crash, the standalone window offers "Send a crash report". This
button opens the same form with a title and the data kept from the stopped
session. The tester states what they were doing, chooses the information to
publish, then authorises the sending. The language, the version and the log
come from the session that crashed, even when the window is recovered at the
next launch. The public diagnostics are unticked at first; the full traces
stay in the local `private-do-not-upload` folder of the saved copy.
An image can be attached from a file. As the viewer's GitHub authorisation
is kept in memory only, sending directly from the crash window requires a
new authorisation from the tester.

### Setting up direct sending

Create a **GitHub App**, in the developer settings of the Aurora-Viewer
organisation, then:

- turn on **Enable Device Flow**, keep the expiry of the authorisations;
- leave the callback and setup URLs empty and disable the webhook;
- repository permissions: **Issues — Read and write**, read-only metadata,
  no other permission;
- public application (**Any account**) for the testers; install it on the
  organisation with only the Aurora-Viewer repository selected;
- copy the **public Client ID** into `bug_report/github.rs::CLIENT_ID`.
  The numeric App ID is not suitable. No Client secret and no private key
  may be added to the viewer.

[Creating a GitHub App](https://docs.github.com/en/apps/creating-github-apps/registering-a-github-app/registering-a-github-app)
and [Device Flow](https://docs.github.com/en/apps/creating-github-apps/authenticating-with-a-github-app/generating-a-user-access-token-for-a-github-app#using-the-device-flow-to-generate-a-user-access-token).

The tester clicks Connect to GitHub, copies the code and authorises it
themselves on GitHub in their browser. The viewer never asks for their
password. The authorisation stays in memory for this session, eight hours at
most; it is written neither to the settings nor to the logs. An
authorisation does not trigger the sending: the tester then clicks Send
public report. Cancellation, refusal, expiry and a slow-down requested by
GitHub are handled. The creation of an issue is never retried automatically
after a lost response, to avoid duplicates.

### Data and files

The form has two steps: prepare the report, then choose how to send it.
The window offers three tabs: Description, Picture and Data. It is larger
to leave room for the input fields; its dimensions stay limited to the
space available on the screen.
The four fields of Description stay visible together without scrolling the
form at the normal size. Long texts scroll in their own input area. In
Picture, the controls are on the left and the preview on the right.
The tabs stay visible while scrolling. The Data tab lets the tester read
and untick each piece of automatic information. The labels and the
diagnostics follow the interface language each time they are displayed. The
typed text is kept as it is. The headings and technical values of the
exported file stay in English for the tools that read the reports. The copy
and save actions are in "More actions".
A new report starts on Description. Closing the window keeps in memory the
text, the tab, the scroll position, the checkboxes, the image and the
sending mode, including the GitHub authorisation in progress, until the end
of the viewer session.
A sending error also keeps the draft. After the issue has been published,
the link of the report and its folder stay available;
reopening the window or choosing "New report" then prepares a fresh form.
The browser does not confirm the publication to the viewer: the tester
clicks "I have published the report" after sending on GitHub to clear the
draft. Opening GitHub or saving a local copy does not clear it.
The avatar name and the diagnostics are unticked at first. No SL login
state and no full settings are serialised. The public diagnostics keep only
the recognised warning / error categories: simply removing the URLs or
UUIDs would not be enough to remove the names and the free texts coming
from the network.

Screenshots are limited to the world, with or without the interface; the
login screen cannot be captured with this button. The chosen files are
limited to 16 MiB, decoded with dimension/memory limits and re-encoded as a
PNG of at most 1920 pixels a side. Metadata, path and name of the original
file are never attached. A screenshot can still contain visible names and
chat: the tester must review it.
After a first screenshot, changing "Include the interface in the picture"
takes the screenshot again automatically, without re-ticking its inclusion
if the tester had excluded it.
This option never replaces an image imported from a file.

Each save creates a folder under `data/bug-reports/`, next to `data/logs/`.
The `public` subfolder contains the chosen text and, if included,
`screenshot.png`. The `private-do-not-upload` folder contains the private
local diagnostics and must never be attached to a public issue. Nothing
leaves the PC besides the text of the issue: GitHub's API takes no
attachment, so in both modes the viewer opens the `public` folder and the
tester drags `screenshot.png` into the issue themselves. Aurora runs no
server of its own for the reports.

### Validation

`AURORA_DEMO_BUG_REPORT=1` with demo mode opens a sample report. Both
sending modes stay local, without GitHub authorisation and without opening
the browser. The `details`, `image`, `image-ui` and `direct` modes show
respectively the information with the location unticked, a screenshot of
the scene, a screenshot with the interface and the direct sending choice.
The form opens after the scene has loaded (frame 550).
`refresh` takes a screenshot without the interface then enables the option
at frame 700 and disables it at frame 900. `language` switches the already
open window to English at frame 700, then to French at frame 900, without
changing the saved preferences. To test the language manually in
Preferences, launch without the `--language` argument, which would force
the language of the process.
Test the unticked fields, the copy, the export, an image with removal then
re-adding, a screenshot with and without the interface, the resizing and
both languages. Change the text, choose Picture or Data, close then reopen
from Report a bug: check that the text, the checkboxes, the image and the
tab are kept. Also test the entry from the login screen and the Help menu.

The real authorisation and the publication of a test report on GitHub are
validated by the human, with their account. Also check the cancellation and
the refusal of the authorisation. The automated tests open no real issue.
