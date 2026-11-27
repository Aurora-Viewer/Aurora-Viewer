---
name: review-next
description: Offer the human a selection of the next pull requests of someone else to review from the review queue (re-reviews first, then crash fixes, the fix label, then PRs not reviewed yet), claim the ones they pick so no other maintainer reviews them at the same time, and review them through to the posted comment and the merge offer. Use when the human asks to review the next PR, asks what to review, or asks to work through the review queue.
---

# Review the next PR of the queue

The rules are in AGENTS.md section 11 ("Which PR of someone else's to review
next", then the review itself) and in `scripts/tools/review-queue.ps1`. The
queue only changes **which** PR you take and adds the claim; the review is
the same as before: get the outside CI approved, wait for it, review, post
the comment yourself, offer the merge.

Commands: Windows `./scripts/review-queue.ps1`; Linux and macOS
`pwsh ./scripts/review-queue.ps1` (the form the "Claude permissions" rules
allow; the same for `merge-pr.ps1`). Run each `gh` and script command
alone, with no `cd` or `;` around it.

**Never end your turn while a review you started waits for CI.** A turn that
ends "I'll review it once CI is green" leaves the review hanging: nothing
wakes you reliably. Wait in the turn (step 4), or release the claim (step 8).

**Ask your human mid-turn.** What only they can say before the reviews go
on goes in a question dialog that keeps the turn going (Claude Code:
`AskUserQuestion`, one question per subject, up to four), never in the
end-of-turn box, which ends the turn: the selection (step 2, with the
merges and conditions of step 1), the CI approvals of the PRs claimed
(step 3, once it has gathered them all), and the demo or grid test of the
PRs whose diff reads clean (step 5). The end-of-turn question is only for what comes
after the reviews: the merge of the PRs reviewed ✅ in this turn and the
decisions of step 6.

1. **Your approved PRs first**:

   ```
   pwsh ./scripts/review-queue.ps1 -ToMerge -Json
   ```

   These are PRs whose last review is your ✅ and that nobody queued: the
   queue leaves them out, so nobody else will merge them. For each:
   - `head_reviewed: false`: the head is not the one your review names (a
     push after it, a merge of main, or a review that names no head). It is
     a re-review: take it through steps 3 to 6, reading what changed since
     the head you reviewed (for a merge of main, that it brings nothing but
     main).
   - A verdict line with a condition (`✅ ready to merge once …`): what it
     waits for is your human's, in the dialog of step 2 ("Have you done the
     check …?"). Not done yet: it stays approved.
   - `conflict: true`: no CI can test it against `main` (GitHub cannot
     build the merge), so ask the author to merge main: a `## Changes
     requested` (AGENTS.md section 11, like every request to an author)
     that names the conflicting files (`git merge-tree --write-tree
     --name-only origin/main <head>`) and asks them to say so in a comment
     once it is pushed (a merge of main alone is no answer to the queue).
     Their answer brings it back as a re-review of the merge. Already asked
     since your ✅ (a comment of yours after it says so): nothing to post.
   - `ci`: `Approval`, it joins the CI approvals of step 3, then the
     wait of step 4; `Run` or `None`, wait (step 4); `Fail`, read the failing job (step 4) and ask the author
     to fix it.
   - Otherwise (head reviewed, green CI, no conflict): offer its merge in
     the dialog of step 2; on their yes, merge it as step 7 says.

   Then the PRs your ⚠️ reviews left waiting on your human (a grid test, a
   product choice): every queue leaves them out, so only you bring them
   back.

   ```
   pwsh ./scripts/review-queue.ps1 -All
   ```

   The lines `waiting on a decision (<your login>'s review)`: read what
   your review left open and put it in the dialog of step 2 (a grid test
   to run now, a choice to make). Their answer becomes a new comment, and
   the ✅ or `## Changes requested` that follows.

2. **Read the queue, offer a selection, claim what your human picks**:

   ```
   pwsh ./scripts/review-queue.ps1 -Json -Top 8
   ```

   Never pick and claim on your own. Offer the first entries, in the
   queue's order, in a question dialog that keeps the turn going (Claude
   Code: `AskUserQuestion`, several choices allowed, one option per PR, up
   to four): number, author, title, kind and notes (`claimed by you`,
   `CI waits for Approve and run`, `conflict with main`, a crash issue),
   the first marked as recommended. Their own answer can name others
   further down the queue. Put the questions of step 1 (merges, conditions)
   in the same dialog. An entry with `claimed_by_you: true` is a review
   your human's account started: say so in its option, and whether another
   of their sessions may still be on it (a `../work/review-<n>` worktree),
   since the claim cannot tell their sessions apart. An empty queue: say
   so, and go to step 7 with the PRs of step 1.

   Then claim each PR they picked, before reading anything of it:

   ```
   pwsh ./scripts/review-queue.ps1 -Claim <n>
   ```

   A refused claim (another maintainer took it since): say so, and offer
   the next entry instead. Never remove another maintainer's assignment,
   never take a PR the queue left out.

3. **Before the code** (AGENTS.md section 11, someone else's PR):
   - **Description**: `needs-description` means its author completes it
     first: release the claim (step 8) and take the next one.
   - **Labels**: set its type and area labels if they are missing
     (`gh pr edit <n> --add-label fix --add-label ui`); outside contributors
     cannot.
   - **Conflict with main** (note `conflict with main`): its CI cannot run
     against the current `main`, and a green run is an older merge's. Do not
     ask for its CI approval, do not wait for CI, do not read the code: post
     a `## Changes requested` (step 6) that names the conflicting files
     (`git merge-tree --write-tree --name-only origin/main <head>`) and asks
     the author to merge (or rebase onto) `main` and to say so in a comment
     when it is pushed (a merge of main alone does not bring the PR back to
     the queue). That comment is the review: the claim ends with it.
   - **Head and files**, not the diff yet (`gh pr view` stops at 100 files):

     ```
     gh pr view <n> --repo Aurora-Viewer/Aurora-Viewer --json headRefOid --jq .headRefOid
     gh api repos/Aurora-Viewer/Aurora-Viewer/pulls/<n>/files --paginate --jq '.[].filename'
     ```

   - **CI waits for "Approve and run"** (note `CI waits for Approve and run`,
     or no CI run at all): your human approves it (AGENTS.md section 11).
     Gather what they need to decide, for every PR you claimed (and those
     of step 1) before asking, so that one question covers them all:
     whether the PR changes a file under `.github/`, `scripts/`, `.cargo/`
     or `packaging/`, a `Cargo.toml`, `Cargo.lock`, `build.rs` or
     `rust-toolchain.toml` (name those files), and the runs waiting:

     ```
     gh api "repos/Aurora-Viewer/Aurora-Viewer/actions/runs?head_sha=<head>" --jq '.workflow_runs[] | select(.conclusion == "action_required") | .id'
     ```

     `gh pr checks` can show only the description check as passed while
     the real CI never ran: always list the runs of the head commit. Ask in
     one mid-turn dialog for all of them ("Approve the CI of #3475 and
     #4449?": "Approve them" / "Not now"). On their yes, approve each run (`gh api -X POST
     repos/Aurora-Viewer/Aurora-Viewer/actions/runs/<id>/approve`) and go on
     to step 4; on a no, release those claims (step 8). If your tool
     refuses the call, give your human the commands.
   - `last reviewed by <someone>`: read their review and the author's
     answers first; a re-review checks every point of it.

4. **Wait for CI, in the turn.** Right after approving, the runs take a
   moment to start, and `gh pr checks --watch` can stop at once on the
   description check alone. Wait on the runs themselves:

   ```
   gh api "repos/Aurora-Viewer/Aurora-Viewer/actions/runs?head_sha=<head>" --jq '.workflow_runs[] | "\(.id) \(.name) \(.status) \(.conclusion)"'
   gh run watch <id> --repo Aurora-Viewer/Aurora-Viewer --exit-status
   gh pr checks <n> --repo Aurora-Viewer/Aurora-Viewer
   ```

   Watch each run of the head (CI and macOS) to its end, then read the
   checks once more. Give each command the longest timeout your tool allows
   (Claude Code: 600000 ms) and run it again while it is still going; with
   several PRs claimed, watch them one after the other. Meanwhile you may
   read the earlier reviews and the Legacy Viewer sources, not the PR's
   code. A run you watch in the background still has to end before the
   turn does.
   - **Green**: step 5.
   - **Red**: `gh run view <id> --log-failed`. When the failure is the
     PR's, the review is a `## Changes requested` with the failing job and
     its error (step 6). When it is not (a runner lost, a job that fails on
     `main` too): rerun it (`gh run rerun <id> --failed`) and wait again.

5. **Review it** as AGENTS.md section 11 says: no conflict with `main`, the
   diff read in full, Legacy Viewer fidelity, creators' permissions, no
   data of our own in shared places, credits, secrets, docs. Draft the
   comment in a `.scratch/review-<n>.md` (ignored by git).

   **Test it in a demo or on the grid**, never skipped, once CI is green
   and the diff reads clean, before any verdict, whenever the change can
   be seen or felt in the viewer: a window, a rendering, a behaviour, a
   demo scenario it adds or touches (`AURORA_DEMO_*`), or something only
   the grid shows (network, teleports, voice, other residents, permissions
   on real assets). Offer which test in a mid-turn dialog, one question per
   PR, saying what you would run and what to look at, with your pick first
   (both when the demo shows part of it and the grid the rest):
   - **Demo** (default for what a scenario shows): you build the PR merged
     into the latest `main` in a detached worktree (`git worktree add
     --detach ../work/review-<n> origin/main`, then merge the PR's head
     into it), run its tests and the scenarios it touches, before and
     after, with `--title "Test review-<n>"` (AGENTS.md section 4), look at
     the captures and put them in the review (`pr-captures.ps1`).
   - **Grid test**: you launch that build for your human with the same
     title; they log in and test (AGENTS.md section 1, rule 1), and you say
     precisely what to do and what they should see. Their result goes in
     the review.

   There is no "skip". When your human cannot do the grid test now, post
   the review with the ⚠️ verdict, "waiting for the maintainer's grid
   test", and what is to test: the queue holds it for you (no other
   maintainer can claim it), step 1 brings it back, and the ✅ follows
   their result. Only a PR with
   nothing to see in the viewer (docs, tests, tooling) gets no test, and
   its review says why. Remove the worktree once the review is posted
   (`git worktree remove ../work/review-<n>`). A problem the test shows is
   a finding like the others: `## Changes requested`.

6. **Post it yourself**, without asking first:

   ```
   gh pr comment <n> --repo Aurora-Viewer/Aurora-Viewer --body-file .scratch/review-<n>.md
   ```

   `## Changes requested` when the author has work to do, a merge of
   `main` included (a conflict found while reviewing: never a ✅ that asks
   for it, which would leave the PR approved with nothing to merge);
   `## Review` with its verdict line otherwise. Every review names the head it read, on its
   own line: ``Reviewed head: `<first 9 characters of the head>` ``; the
   queue offers the merge of that head only. Posting the review ends the
   claim: nothing to release. A point only the human can decide (a product
   choice, a difference from the Legacy Viewer): post the review with the
   ⚠️ verdict and the options, then ask the human in the end-of-turn
   question; post their decision as a new comment once they answer.

7. **Offer the merge**: the PRs of step 1 left for it are in the dialog
   of step 2; every PR you reviewed ✅ in this turn goes in one
   end-of-turn question with buttons ("Merge #4267 and #2297?": "Merge
   them" / "Not yet"). Merging someone else's PR
   needs your human's yes (AGENTS.md section 1, rule 5): never merge
   without it, and never only mention it in prose. On their yes, for each
   PR, first read its head again: a head other than the one you reviewed
   (a push since) is not merged, it goes back to step 1 as a re-review.
   Then list the runs of the head: one at `action_required` gets the PR
   dropped from the merge queue at once; it needs your human's approval
   first (step 3), then green CI. Then:

   ```
   pwsh ./scripts/merge-pr.ps1 -Pr <n>
   ```

   The script waits for the outcome, which can take a while: run it in the
   background and go on with the reviews, but read its outcome before the
   turn ends.
   - **Merged**: check that the issues it closes are closed.
   - **Left the queue**: `gh run view <id> --log-failed`. A failure of the
     PR's code combined with `main`: a `## Changes requested` comment to its
     author with the failing test and the run link. A failure that is not
     the PR's (a runner lost, a job red on `main` too): queue it again once
     `main` is green, and tell your human.

8. **Stopping without a review** (the human sends you elsewhere, a blocker
   you cannot judge, a description to complete):
   `pwsh ./scripts/review-queue.ps1 -Release <n>`, and tell the human why.

9. **Report** to the human: each PR, its verdict and the link of the
   posted comment, what was asked of its author, what was merged or left
   the queue. Then offer the next entry of the queue.
