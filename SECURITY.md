# Security policy

Thank you for helping keep Aurora Viewer and its users safe. If you think you
found a security problem, please tell us **privately** first, so that we can
fix it before it is made public.

## How to report

Please do **not** open a public issue, pull request or feedback post for a
vulnerability.

- **Preferred:** GitHub's private reporting, on the
  [Security tab](https://github.com/Aurora-Viewer/Aurora-Viewer/security/advisories/new)
  of this repository ("Report a vulnerability"). Only the maintainers see it.
- **By e-mail:** [contact@aurora-viewer.org](mailto:contact@aurora-viewer.org),
  with "Security" in the subject.

Helpful details:

- the version of the viewer (Help › About) or the commit you reviewed;
- the affected part (file, feature, network message, release file…);
- the steps to reproduce it, and what an attacker could do with it;
- a proof of concept if you have one. Leave out any password, token,
  capability URL or personal data, yours or anyone else's.

## What happens next

- We confirm that we received the report and tell you how we will handle it.
- We keep you informed while we work on the fix, and may ask you to check it.
- The fix ships in a release. Once users have had the chance to update, we
  publish the details (a GitHub security advisory when it fits) and credit you
  by the name you choose, or not at all if you prefer.

Please give us a reasonable time to release a fix before you publish anything.

## Supported versions

Only the **latest release** receives security fixes: the viewer updates itself
from the [releases](https://github.com/Aurora-Viewer/Aurora-Viewer/releases)
of this repository. Development builds of `main` are fixed through `main`.

## Scope

In scope:

- the viewer itself, on every system it is published for;
- its updater and the files of a release (archives, checksums, update policy);
- the media plugin packs the viewer downloads;
- this repository's build, CI and release workflows.

Out of scope, please report them to their owners:

- Second Life itself (the grid, its servers, its websites, accounts): Linden
  Lab;
- other viewers, including the ones Aurora ports code from: their own
  projects;
- the third-party services and websites the viewer opens on your request.

## Testing rules

- Test with your own accounts, objects and computers only.
- Never access, change or keep the data of other residents.
- No denial of service, spam or social engineering against residents, the
  team or Linden Lab.
- On the grid, follow the Second Life Terms of Service.
