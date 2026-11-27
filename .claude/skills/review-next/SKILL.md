---
name: review-next
description: Pick the next pull request of someone else to review from the review queue (re-reviews first, then crash fixes, the fix label, then PRs not reviewed yet), claim it so no other maintainer reviews it at the same time, and review it through to the posted comment and the merge offer. Use when the human asks to review the next PR, asks what to review, or asks to work through the review queue.
---

# Review the next PR of the queue

The rules are in AGENTS.md section 11 ("Which PR of someone else's to review
next", then the review itself) and in `scripts/tools/review-queue.ps1`. The
queue only changes **which** PR you take and adds the claim; the review is
the same as before: approve the outside CI, wait for it, review, post the
comment yourself, offer the merge.

Commands: Windows `./scripts/review-queue.ps1`; Linux and macOS
`pwsh -NoProfile -File ./scripts/review-queue.ps1`. Run each `gh` and
`review-queue.ps1` command alone, with no `cd` or `;` around it.

**Never end your turn while a review you started waits for CI.** A turn that
ends "I'll review it once CI is green" leaves the review hanging: nothing
wakes you reliably. Wait in the turn (step 4), or release the claim (step 8).

1. **Your approved PRs first**:

   ```
   pwsh -NoProfile -File ./scripts/review-queue.ps1 -ToMerge -Json
   ```

   These are PRs whose last review is your ✅ and that nobody queued: the
   queue leaves them out, so nobody else will merge them. Keep them for the
   merge offer of step 7: green CI and no conflict, offer the merge; CI
   waiting for "Approve and run", approve it as in step 3 and wait as in
   step 4; a conflict, ask the author to merge main (`## Changes
   requested`); red CI, ask the author to fix it with the failing job.

2. **Read the queue and claim**:

   ```
   pwsh -NoProfile -File ./scripts/review-queue.ps1 -Json -Top 5
   pwsh -NoProfile -File ./scripts/review-queue.ps1 -Claim <n>
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
   - **Files**: list them (`gh pr view <n> --json files --jq
     '.files[].path'`), not the diff yet.
   - **CI waits for "Approve and run"** (note `CI waits for Approve and run`,
     or no CI run at all): if no file is under `.github/`, `scripts/`,
     `.cargo/` or `packaging/`, and none is a `Cargo.toml`, `Cargo.lock`,
     `build.rs` or `rust-toolchain.toml`, approve the runs yourself: your
     human is the maintainer who asked for the review.

     ```
     gh api "repos/Aurora-Viewer/Aurora-Viewer/actions/runs?head_sha=<head sha>" --jq '.workflow_runs[] | select(.conclusion == "action_required") | .id'
     gh api -X POST repos/Aurora-Viewer/Aurora-Viewer/actions/runs/<id>/approve
     ```

     `gh pr checks` can show only the description check as passed while
     the real CI never ran: always list the runs of the head commit. If the
     PR changes one of those files, ask your human before approving (the
     end-of-turn question, then stop: that is the only CI wait that ends a
     turn). If your tool refuses the `gh api` call, give the human the
     commands, and wait for their word.
   - `last reviewed by <someone>`: read their review and the author's
     answers first; a re-review checks every point of it.

4. **Wait for CI, in the turn**:

   ```
   gh pr checks <n> --repo Aurora-Viewer/Aurora-Viewer --watch --interval 60
   ```

   Give the command the longest timeout your tool allows (Claude Code:
   600000 ms) and run it again while checks are still pending; with several
   PRs claimed, watch them one after the other. Meanwhile you may read the
   earlier reviews and the Legacy Viewer sources, not the PR's code. A run
   you started in the background still has to be waited for before the turn
   ends.
   - **Green**: step 5.
   - **Red**: `gh run view <id> --log-failed`; when the failure is the
     PR's, the review is a `## Changes requested` with the failing job and
     its error (step 6). A failure that is not the PR's (a runner lost, a
     job that fails on `main` too): rerun it (`gh run rerun <id> --failed`)
     and wait again.

5. **Review it** as AGENTS.md section 11 says: no conflict with `main`, the
   diff read in full, the PR merged into the latest `main` in a review
   worktree (`../work/review-<n>`) to run its tests and demo scenarios when
   it touches the viewer, Legacy Viewer fidelity, creators' permissions, no
   data of our own in shared places, credits, secrets, docs. Draft the
   comment in `.scratch/review-<n>.md` of the worktree you work from.

6. **Post it yourself**, without asking first:

   ```
   gh pr comment <n> --repo Aurora-Viewer/Aurora-Viewer --body-file .scratch/review-<n>.md
   ```

   `## Changes requested` when the author has work to do, `## Review` with
   its verdict line otherwise. Posting the review ends the claim: nothing to
   release. A point only the human can decide (a product choice, a
   difference from the Legacy Viewer): post the review with the ⚠️ verdict
   and the options, then ask the human in the end-of-turn question; post
   their decision as a new comment once they answer.

7. **Offer the merge**: every PR you reviewed ✅ in this turn, and those of
   step 1 with green CI and no conflict, go in one end-of-turn question
   with buttons ("Merge #4267 and #2297?": "Merge them" / "Not yet").
   Merging someone else's PR needs your human's yes (AGENTS.md section 1,
   rule 5): never merge without it, and never only mention it in prose.
   On their yes, for each PR:

   ```
   pwsh -NoProfile -File ./scripts/merge-pr.ps1 -Pr <n>
   ```

   Before it, list the runs of the head commit again: a run left at
   `action_required` (a new commit since you approved) gets the PR dropped
   from the queue at once; approve it and wait for green first. The script
   waits for the outcome:
   - **Merged**: check that the issues it closes are closed.
   - **Left the queue**: `gh run view <id> --log-failed`, then a `## Changes
     requested` comment to its author with the failing test and the run
     link.

8. **Stopping without a review** (the human sends you elsewhere, a blocker
   you cannot judge, a description to complete):
   `pwsh -NoProfile -File ./scripts/review-queue.ps1 -Release <n>`, and tell
   the human why.

9. **Report** to the human: each PR, its verdict and the link of the
   posted comment, what was asked of its author, what was merged or left
   the queue. Then offer the next entry of the queue.
