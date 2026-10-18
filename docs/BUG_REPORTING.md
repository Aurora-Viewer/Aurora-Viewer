# Alpha bug reports

"Report a bug", in the top bar, the Help menu and on the login screen, opens
the project's
[GitHub issue forms](https://github.com/Aurora-Viewer/Aurora-Viewer/issues/new/choose)
in the browser. The tester needs a GitHub account, chooses the bug report
template and fills it in on GitHub. After a crash, "Send a crash report" in
the standalone window opens the same page; "Copy trace" and "Open report
folder" give the technical details to paste or attach, after reviewing them
(the full trace may contain personal paths).

The issue templates are in `.github/ISSUE_TEMPLATE/`; they set the issue
type (Bug or Feature), and issues carry no labels. The disabled form below
still sends the former `bug` and `alpha` labels (`bug_report/github.rs`):
switch it to the Bug type when it is brought back.

## The built-in form (disabled)

Until 2026-10-15 these buttons opened a report form inside the viewer. It is
disabled for now; its code stays in the tree, out of the build, to be
brought back later: uncomment every line marked "Bug report form (disabled,
see main.rs)" (`main.rs` lists its files). The rest of this page describes
that form, as it works once enabled, and its GitHub / API setup.
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
next launch. The public diagnostics and the private ZIP are unticked at
first; the full traces can join the private ZIP if the tester chooses so.
An image can be attached from a file. As the viewer's GitHub authorisation
is kept in memory only, sending directly from the crash window requires a
new authorisation from the tester.

### Setting up direct sending

Create a **GitHub App**, in the developer settings of the Aurora-Viewer
organisation, then:

- turn on **Enable Device Flow**, keep the expiry of the authorisations;
- for the Device Flow alone, leave the callback and setup URLs empty; for
  the private ZIPs, add the callback described further down. Disable the
  webhook;
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
A sending error also keeps the draft. After the issue and its files have
been fully published, the link of the report and its folder stay available;
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
local diagnostics and must never be attached to a public issue. The images
of the direct sending are uploaded to `api.auroraviewer.com`, then displayed
in the issue. The tester's GitHub authorisation is never sent to this
service. The browser mode keeps the manual addition of the screenshot from
`public`.

### Public images and private logs

The PHP service is in the private repository
[aurora-api](https://github.com/Aurora-Viewer/aurora-api), linked to Clever Cloud.
The Cellar bucket stays private: the API exposes the screenshots and
protects the ZIPs. The viewer checks that the service is available before
creating an issue with files, declares their SHA-256 hashes in the report,
then sends them once the issue number has been received. The service
accepts only the file announced in a recent issue of the Aurora-Viewer
repository. Images: 16 MiB, 1920 pixels a side at most, PNG re-encoding on
the server side.

"Share complete logs with developers only", in Data, is unticked by default
and offered for sending from Aurora. The ZIP contains the log of this
session and the full diagnostics; it can therefore include fields unticked
for the public report and personal information. It stays in
`private-do-not-upload`, separate from the public copy. Limits: 64 MiB
before compression and 16 MiB compressed; a missing or oversized log stops
the preparation and keeps the draft. No real ZIP is sent in demo mode.

The link of the ZIP appears in the public issue, but downloading it requires
the GitHub authorisation of an account that has access to the private
`aurora-api` repository. The GitHub App must include this repository in its
installation and have the callback URL
`https://api.auroraviewer.com/auth/callback`. Its Client secret is
configured only in the Clever Cloud variable `GITHUB_CLIENT_SECRET`.
The developer sessions last fifteen minutes and the access right is checked
again at each download. No public cache receives the ZIPs.

After a partial error, the issue number, the files and the draft stay in
memory, including after the form is closed. "Finish uploading files"
resumes the same report and keeps the changes made in GitHub; it does not
create a second issue. A new form is prepared only after the whole sending
has succeeded.

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
