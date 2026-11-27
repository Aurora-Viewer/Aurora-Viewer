---
name: review-next
description: Pick the next pull request of someone else to review from the review queue (re-reviews first, then crash fixes, the fix label, then PRs not reviewed yet), claim it so no other maintainer reviews it at the same time, and review it through to the posted comment and the merge offer. Use when the human asks to review the next PR, asks what to review, or asks to work through the review queue.
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
The only waits that end a turn are a question to your human: the CI
approval (step 3), a decision (step 6), the merge (step 7).

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
   - `conflict: true`: ask the author to merge main, in a plain comment
     with no review heading (a `## Changes requested` would leave the PR
     waiting on them for good, since a merge of main is no answer); it comes
     back here with a new head, to re-review.
   - `ci`: `Approval`, it joins the approval question of step 3, then the
     wait of step 4; `Run` or `None`, wait (step 4); `Fail`, read the failing job (step 4) and ask the author
     to fix it.
   - Otherwise (head reviewed, green CI, no conflict): it goes in the merge
     offer of step 7.

2. **Read the queue and claim**:

   ```
   pwsh ./scripts/review-queue.ps1 -Json -Top 5
   pwsh ./scripts/review-queue.ps1 -Claim <n>
   ```

   Take the first entry (one with `claimed_by_you: true` is a review you
   already started: finish it first), or as many as the human asked for,
   in order. Claim each before reading anything of it. A refused claim
   (another maintainer holds it): take the next entry. Never remove another
   maintainer's assignment, never take a PR the queue left out. An empty
   queue: say so, and go to step 7 with the PRs of step 1.

3. **Before the code** (AGENTS.md section 11, someone else's PR):
   - **Description**: `needs-description` means its author completes it
     first: release the claim (step 8) and take the next one.
   - **Labels**: set its type and area labels if they are missing
     (`gh pr edit <n> --add-label fix --add-label ui`); outside contributors
     cannot.
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
     the end-of-turn question with buttons ("Approve the CI of #3475 and
     #4449?": "Approve them" / "Only #3475"), and stop there: the one
     wait of the review that ends a turn before the code. Meanwhile, PRs
     whose CI is already green go on to step 5 in the same turn. On their
     yes, approve each run (`gh api -X POST
     repos/Aurora-Viewer/Aurora-Viewer/actions/runs/<id>/approve`) and go
     straight to step 4 in that turn. If your tool refuses the call, give
     your human the commands.
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
   diff read in full, the PR merged into the latest `main` to run its tests
   and demo scenarios when it touches the viewer, Legacy Viewer fidelity,
   creators' permissions, no data of our own in shared places, credits,
   secrets, docs. For the merged build, a detached worktree beside the
   others (`git worktree add --detach ../work/review-<n> origin/main`, then
   merge the PR's head into it), removed once the review is posted
   (`git worktree remove ../work/review-<n>`). Draft the comment in a
   `.scratch/review-<n>.md` (ignored by git).

6. **Post it yourself**, without asking first:

   ```
   gh pr comment <n> --repo Aurora-Viewer/Aurora-Viewer --body-file .scratch/review-<n>.md
   ```

   `## Changes requested` when the author has work to do, `## Review` with
   its verdict line otherwise. Every review names the head it read, on its
   own line: ``Reviewed head: `<first 9 characters of the head>` ``; the
   queue offers the merge of that head only. Posting the review ends the
   claim: nothing to release. A point only the human can decide (a product
   choice, a difference from the Legacy Viewer): post the review with the
   ⚠️ verdict and the options, then ask the human in the end-of-turn
   question; post their decision as a new comment once they answer.

7. **Offer the merge**: every PR you reviewed ✅ in this turn, and those of
   step 1 left for it, go in one end-of-turn question with buttons ("Merge
   #4267 and #2297?": "Merge them" / "Not yet"). Merging someone else's PR
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

   The script waits for the outcome:
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
