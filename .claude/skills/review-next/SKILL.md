---
name: review-next
description: Pick the next pull request of someone else to review from the review queue (re-reviews first, then crash fixes, the fix label, then PRs not reviewed yet), claim it so no other maintainer reviews it at the same time, and review it. Use when the human asks to review the next PR, asks what to review, or asks to work through the review queue.
---

# Review the next PR of the queue

The rules are in AGENTS.md section 11 ("Which PR of someone else's to review
next") and in `scripts/tools/review-queue.ps1`; this is the routine.

1. **Read the queue** (Windows: `./scripts/review-queue.ps1`; Linux and
   macOS: `pwsh ./scripts/review-queue.ps1`):

   ```
   pwsh ./scripts/review-queue.ps1 -Json -Top 5
   ```

   Take the first entry. An entry with `claimed_by_you: true` is a review
   your human already started: it comes first, finish it. An empty list
   means nothing to review: say so and stop.

2. **Claim it before reading anything else**:

   ```
   pwsh ./scripts/review-queue.ps1 -Claim <n>
   ```

   If it is refused (another maintainer claimed it since, or it is their
   re-review), go back to step 1 and take the next entry. Never remove
   another maintainer's assignment, never take a PR the queue left out.

3. **Before reading the code**, the checks of AGENTS.md section 11 for
   someone else's PR:
   - a note `CI waits for Approve and run`: look first that the PR does not
     change `.github/`, then approve its run; wait for it to be green. While
     it runs you may claim and start the next entry, and come back.
   - `CI running`: wait for it (`gh pr checks <n> --watch`).
   - `conflict with main`: the review asks the author to merge main.
   - `last reviewed by <someone>`: read their review first, and check that
     each of its points is answered.

4. **Review it** as AGENTS.md section 11 says, and post the comment
   (`gh pr comment <n> --body-file .scratch/review-<n>.md`): `## Changes
   requested` when the author has work to do, `## Review` with its verdict
   line otherwise. A re-review checks every point of the previous review.
   Posting the review ends the claim: nothing to release.

5. **Stopping without a review** (the human sends you elsewhere, a blocker
   you cannot judge): `pwsh ./scripts/review-queue.ps1 -Release <n>`, and
   tell the human why.

6. **Report** to the human: the PR, the verdict, what was asked. Merging
   someone else's PR still needs their explicit agreement (AGENTS.md
   section 1, rule 5). Then offer the next entry of the queue.
