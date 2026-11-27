---
name: review-next
description: Review pull requests of someone else from the review queue, the maintainer's way of AGENTS.md section 11. First what already waits on the human (approvals to merge, reviews left on a decision), then a selection from the queue (re-reviews, crash fixes, the fix label, PRs not reviewed yet) for the human to pick from, claimed so no other maintainer reviews them at the same time, then each review through to the posted comment and the merge. Use when the human asks to review the next PR, asks what to review, or asks to work through the review queue.
---

# Review from the queue

AGENTS.md is the rule; this is the routine. Section 11 says how a review is
done, section 8 the follow-up and the merge, section 4 the demo, section 1
who logs in on the grid. The queue (`scripts/review-queue.ps1`, rules in
`scripts/tools/review-queue.ps1`) only says **which** PR and keeps
maintainers apart with a claim; everything else uses the repository's own
scripts, as for your own task:

| Script | In this routine |
|---|---|
| `review-queue.ps1` | `-Pending` (step 1), the queue and `-Claim` (step 2), `-Release` (step 9) |
| `new-task.ps1` | the review worktree `../work/review-<n>` (step 3) |
| `check.ps1` | on the PR merged with the latest `main` (step 5), and after a maintainer's fix (step 6) |
| `pr-captures.ps1` | the demo captures in the review (step 5) |
| `merge-pr.ps1` | the merge, on your human's yes (step 8) |
| `issue-board.ps1` | the Status of the issues a merged PR closes (step 8) |
| `end-task.ps1` | the review worktree, once the review is posted (step 7) |

Linux and macOS run them as `pwsh ./scripts/<script>.ps1 …` (the form the
"Claude permissions" rules allow). Run each `gh` and script command alone,
with no `cd` or `;` around it.

**Three rules hold all the way through.**
- **Never end a turn while a review waits on CI or on a build.** Wait in the
  turn (step 4); a turn that ends "I'll review it once CI is green" leaves
  the review hanging, since nothing wakes you reliably. Stopping for good:
  release the claim (step 9).
- **Ask your human in a dialog that keeps the turn going** (Claude Code:
  `AskUserQuestion`, one question per subject, up to four; more subjects:
  the checks they owe together in one question with several answers
  allowed). The end-of-turn box ends the turn: it only carries the merge of
  the PRs reviewed ✅ in this turn and what comes after them.
- **Every request to an author is a `## Changes requested`** (AGENTS.md
  section 11), a merge of `main` included, and only after your human chose
  it over fixing it yourself (step 6).

## 0. Follow-up first

Once per session, before anything else: AGENTS.md section 8, step 0 (merged
task folders through `end-task.ps1`, your human's own PRs without a review,
the issues). Your own PRs come before anyone else's.

## 1. What already waits on your human

```
pwsh ./scripts/review-queue.ps1 -Pending -Json
```

The PRs whose last review is your human's and that the queue leaves out:

- `kind: merge`, your ✅, nobody queued it:
  - `head_reviewed: false`: the head is not the one your review names (a
    push since, a merge of `main`, or a review that names no head). It is a
    re-review: steps 3 to 8, reading what changed since the reviewed head
    (for a merge of `main`, that it brings nothing but `main`).
  - `conflict: true`: step 3's conflict (fix it or ask the author). Already
    asked since your ✅ (a comment of yours after it says so): nothing new.
  - `ci`: `Approval`, ask for it as step 3 says; `Run` or `None`, wait
    (step 4); `Fail`, read the failing job (step 4).
  - A verdict with a condition (`✅ ready to merge once …`): the condition
    is your human's, asked in the dialog below.
  - Otherwise (head reviewed, green, no conflict): the merge, in the dialog.
- `kind: decision`, your ⚠️ left a decision open: read what (a hand or grid
  test, a product choice) and put it in the dialog. Their answer is posted
  at once: a `## Changes requested` when it asks the author for anything
  (the decision, then every point still open), a `## Review` ✅ naming the
  head when nothing is left (after the test it waited for). A test not done
  yet: nothing to post.

Ask all of it in one dialog, then carry every item through to its end
(merged, posted, re-reviewed) before step 2. **Nothing of step 2 starts
while an item of step 1 is open**, other than what waits on your human
(a test not done yet) or on an author.

## 2. A selection from the queue, claimed when picked

```
pwsh ./scripts/review-queue.ps1 -Json -Top 8
```

Never pick and claim on your own. Offer the first entries, in the queue's
order, in a dialog (several answers allowed, one option per PR, the first
marked as recommended): number, author, title, kind, and what taking it
leads to (a full review, or first a conflict to settle; `CI waits for
Approve and run`, a crash issue). Your human may name others further down.
An entry with `claimed_by_you: true` was started from your human's account:
say so, and whether another of their sessions may still be on it (a
`../work/review-<n>` folder); the claim cannot tell their sessions apart.
An empty queue: say so and stop.

Claim each PR picked, before reading anything of it:

```
pwsh ./scripts/review-queue.ps1 -Claim <n>
```

A refused claim (another maintainer took it since): say so and offer the
next entry. Never remove another maintainer's assignment, never take a PR
the queue left out.

## 3. Diff and conflicts, before any CI

For each PR claimed, before its CI is asked for:

- **Description**: `needs-description` means its author completes it
  first: release the claim (step 9).
- **Labels**: its type and area labels, set if missing (`gh pr edit <n>
  --add-label fix --add-label ui`); outside contributors cannot.
- **The review worktree**, through `new-task.ps1` (its own branch, its own
  `target/`, the shared compiler cache), then the PR's head with the latest
  `main` merged into it: the code the merge queue will test, and a branch a
  maintainer's fix can be pushed from.

  ```
  pwsh ./scripts/new-task.ps1 -Name review-<n> -Kind chore
  git -C ../work/review-<n> fetch origin pull/<n>/head
  git -C ../work/review-<n> reset --hard FETCH_HEAD
  git -C ../work/review-<n> merge --no-ff --no-edit origin/main
  ```

  A merge that stops on conflicts names them (`git -C ../work/review-<n>
  diff --name-only --diff-filter=U`): that is a finding of step 6.
- **The diff**, read in full: `git -C ../work/review-<n> diff
  origin/main...FETCH_HEAD` (and, for a re-review, from the head your last
  review named). Look first at what CI would run: anything under
  `.github/`, `scripts/`, `.cargo/` or `packaging/`, a `Cargo.toml`,
  `Cargo.lock`, `build.rs` or `rust-toolchain.toml`, and what the tests and
  build scripts execute. Then the rest, as AGENTS.md section 11 says:
  Legacy Viewer fidelity (the sources in `../phoenix-firestorm`), creators'
  permissions, no data of our own in shared places, credits, secrets, the
  interface rules (section 7), docs.
- **Conflicts, or findings in the diff**: step 6 first (fix it or ask the
  author). A conflict means no CI can test the PR against `main`: do not ask
  for its CI before it is settled.
- **CI waits for "Approve and run"** (note `CI waits for Approve and run`,
  or no run at all on the head): your human approves it (AGENTS.md section
  11). Ask once for every PR claimed, in one dialog, naming any that changes
  the files above and what you read in them. List the runs of the head, not
  `gh pr checks`, which can show only the description check passed while
  the real CI never ran:

  ```
  gh api "repos/Aurora-Viewer/Aurora-Viewer/actions/runs?head_sha=<head>" --jq '.workflow_runs[] | "\(.id) \(.name) \(.status) \(.conclusion)"'
  gh api -X POST repos/Aurora-Viewer/Aurora-Viewer/actions/runs/<id>/approve
  ```

  On their yes, approve each run at `action_required`; on a no, release
  that claim (step 9). If your tool refuses the call, give your human the
  commands. A head pushed by a maintainer runs without an approval.

## 4. Wait for CI, in the turn

```
gh run watch <id> --repo Aurora-Viewer/Aurora-Viewer --exit-status
gh pr checks <n> --repo Aurora-Viewer/Aurora-Viewer
```

Watch each run of the head (CI, macOS) to its end, then read the checks
once more: right after an approval the runs take a moment to start, and
`gh pr checks --watch` can stop at once on the description check alone.
Give each command the longest timeout your tool allows (Claude Code:
600000 ms) and run it again while it is still going; with several PRs, one
after the other. Meanwhile, start step 5's build in the review worktree.

- **Green**: step 5.
- **Red**: `gh run view <id> --log-failed`. The PR's failure: a finding of
  step 6. Not the PR's (a runner lost, a job red on `main` too): rerun it
  (`gh run rerun <id> --failed`) and wait again.

Approved runs test the merge with `main` as it was when they started: a PR
far behind can be green and still break on today's `main`. Step 5 is what
catches it.

## 5. check.ps1, then a demo or a grid test

In the review worktree, on the PR merged with the latest `main`:

```
pwsh ./scripts/check.ps1
```

It takes a build slot like any other build of the PC (AGENTS.md section 5)
and runs what the CI runs: scripts, formatting, clippy, every test. A
failure is a finding of step 6.

Then **test it in a demo or on the grid**, never skipped, before any
verdict, whenever the change can be seen or felt in the viewer: a window, a
rendering, a behaviour, a demo scenario it adds or touches
(`AURORA_DEMO_*`), or what only the grid shows (network, teleports, voice,
other residents, permissions on real assets). Ask which in a dialog, one
question per PR, with what you would run and what to look at, your pick
first (both when the demo shows part of it and the grid the rest):

- **Demo** (AGENTS.md section 4): the scenarios it touches, before
  (`main`) and after, from the review worktree, `--title "Test
  review-<n>"`, captured late enough that the world has loaded
  (`AURORA_CAPTURE_FRAMES=1500` on a slow PC; a capture of the loading
  screen is taken again). Look at every capture, then publish them:
  `pwsh -NoProfile -Command "./scripts/pr-captures.ps1 -Pr <n> -Files a.png, b.png"`
  (on Linux a list needs `-Command`).
- **Grid test**: you launch the review worktree's viewer for your human
  with the same title; they log in and test (AGENTS.md section 1, rule 1),
  and you say precisely what to do and what they should see. Their result
  goes in the review. They cannot now: the review is posted with the ⚠️
  verdict "waiting for the maintainer's grid test" and what to test; the
  queue holds it for you, step 1 brings it back.

Only a PR with nothing to see in the viewer (docs, tests, tooling) gets no
test, and its review says why.

## 6. Fix it or ask the author

Anything the author has to change (a conflict, a test or `check.ps1` broken
on the merge with `main`, a finding of the diff, a failing demo or grid
test) goes to your human in a dialog, before anything is posted:

- **Fix it yourself**, offered only when the branch allows maintainers'
  pushes (`gh pr view <n> --json maintainerCanModify,headRefName,headRepositoryOwner,headRepository`)
  and the fix is small and sure (a merge of `main` and its conflicts, a
  field a test of `main` lacks, a TASKS.md line, a typo). In the review
  worktree: resolve, commit, `check.ps1` again, then push to the author's
  branch:

  ```
  git -C ../work/review-<n> push https://github.com/<owner>/<repo>.git HEAD:refs/heads/<headRefName>
  ```

  Then step 4 on the new head, and the review names it and says what the
  maintainer pushed.
- **Ask the author**: a `## Changes requested` (step 7) with every point;
  for a conflict, the files, and a request to say so in a comment once it
  is pushed (a merge of `main` alone does not bring the PR back to the
  queue).

## 7. Post the review, end the worktree

Draft it in the review worktree's `.scratch/review-<n>.md`, then:

```
gh pr comment <n> --repo Aurora-Viewer/Aurora-Viewer --body-file ../work/review-<n>/.scratch/review-<n>.md
```

- `## Review` with its verdict line (✅, or ⚠️ for what only the team can
  decide, with the options), or `## Changes requested`. Never a ✅ that asks
  for anything.
- It names the head it read, on its own line: ``Reviewed head: `<first 9
  characters>` ``; the queue offers the merge of that head only.
- The check table of AGENTS.md section 11, filled from what you ran: the
  conflict check, CI, `check.ps1`, the demo or grid test with its captures.
- Posting it ends the claim: nothing to release.

Then end the review worktree. `end-task.ps1` looks for a merged PR of the
worktree's own branch, which a review never has: check that nothing is
left to push (`git -C ../work/review-<n> status --porcelain` empty, and no
commit of yours outside the PR and `main`: `git -C ../work/review-<n>
rev-list --no-merges HEAD ^FETCH_HEAD ^origin/main` empty, after a fetch
of the PR's head), then:

```
pwsh ./scripts/end-task.ps1 -Name review-<n> -Force
```

## 8. Offer the merge

Every PR reviewed ✅ in this turn goes in one end-of-turn question with
buttons ("Merge #4267 and #2297?": "Merge them" / "Not yet"); the merges of
step 1 were asked there. Merging someone else's PR needs your human's yes
(AGENTS.md section 1, rule 5): never without it, never only in prose. On
their yes, for each PR:

1. Its head again: another head than the one reviewed (a push since) is
   not merged, it goes back to step 1 as a re-review.
2. The runs of that head: one at `action_required` gets the PR dropped from
   the merge queue at once; it needs your human's approval and green CI
   first.
3. `pwsh ./scripts/merge-pr.ps1 -Pr <n>`: it waits for the outcome, which
   can take longer than one command: run it in the background, watch the
   merge queue's run (`gh run list --event merge_group`, `gh run watch`),
   and read the outcome before the turn ends.
   - **Merged**: the issues it closes are closed, and their Status on the
     board says so (AGENTS.md section 15): `pwsh ./scripts/issue-board.ps1
     -Issue <i>`, then `Done`, or `Waiting Review` while a 🟡 line of
     TASKS.md waits for the QA.
   - **Left the queue**: `gh run view <id> --log-failed`. The PR's failure
     on the code combined with `main`: step 6 (fix it or ask). Not the
     PR's (a runner lost, a job red on `main` too): queue it again once
     `main` is green, and tell your human.

## 9. Stopping without a review

Your human sends you elsewhere, a blocker you cannot judge, a description
to complete, a CI approval refused:

```
pwsh ./scripts/review-queue.ps1 -Release <n>
pwsh ./scripts/end-task.ps1 -Name review-<n> -Force
```

and tell your human why.

## 10. Report

Each PR: its verdict and the link of the posted comment, what was asked of
its author or pushed by the maintainer, what was merged or left the queue,
what waits on your human. Then offer the next entries of the queue.
