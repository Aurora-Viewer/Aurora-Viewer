<!-- Keep these headings. The PR description check asks for the issue line
     and for text of your own under What it is for, Legacy Viewer origin,
     What changes and Tests ("No" or "Not applicable" is enough where it
     fits); a pull request is reviewed once they are there.
     One line per issue: "Closes #<n>" if this PR resolves it, "Refs #<n>"
     if a part or a validation stays pending; "Part of #<epic>" in plain
     text when the issue belongs to an epic (a "Refs #<epic>" line invites
     the close of the epic, which only the project manager closes).
     "No issue" is for a feature whose work has no issue, or for a PR with
     no effect on the viewer (documentation, CI, scripts, dependency
     updates); a "fix" PR always names its Bug. -->
Closes #

## What it is for

<!-- 2 or 3 simple sentences, from the user's point of view. -->

## Legacy Viewer origin

<!-- Yes / no. If yes: files and functions followed
     (e.g. indra/llcharacter/llmotioncontroller.cpp, updateMotionsByType). -->

## What changes

<!-- By area, in short bullets. Options added with their default value. -->

- **Interface:**
- **Rendering:**

## Screenshots

<!-- Demo mode screenshots (scripts/pr-captures.ps1). -->

## Tests

- [ ] `scripts/check.ps1` green (fmt, clippy, tests)
- [ ] Demo scenarios: <!-- AURORA_DEMO_* variables used -->
- [ ] Validated by the human before the PR: <!-- date, what was tested, demo or grid; "not applicable" for a PR with no effect on the viewer (docs, CI, scripts) -->

## Limits and follow-up

<!-- What is not done; ⬜ lines added to TASKS.md. -->
