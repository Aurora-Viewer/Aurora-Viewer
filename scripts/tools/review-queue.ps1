# Rules of the review queue (HUMANS.md "Review queue", AGENTS.md section 11):
# which open pull requests of other people a maintainer reviews next, in
# what order, and how two maintainers stay off each other's PRs. Pure
# functions, no GitHub call: scripts/review-queue.ps1 reads GitHub and
# applies them, scripts/test-review-queue.ps1 tests them.
#
# Order, highest first:
#   1. re-review: a maintainer reviewed it and its author answered since (a
#      commit of their own, a comment, a review reply; after an approval,
#      only a new commit);
#   2. crash: not reviewed yet, and tied to a crash (a linked issue that the
#      crash triage signed, or whose title or the PR's says crash or panic);
#   3. fix: not reviewed yet, with the fix label or a "fix(area):" title;
#   4. the rest not reviewed yet.
# Inside a tier: crash before fix before the rest, then the longest waiting.
#
# No overlap between maintainers:
#   - a claim is the PR's assignee: a maintainer assigns themselves when they
#     start (review-queue.ps1 -Claim), and every other maintainer's queue
#     leaves the PR out until that maintainer posts a review (or releases
#     it); a claim with no word from its maintainer for
#     $ReviewQueueStaleClaimDays days is shown as stale, still respected;
#   - a re-review belongs to the maintainer who wrote the last review, for
#     $ReviewQueueHandOverDays after the author's answer; then anyone may
#     take it (they claim it first).

. "$PSScriptRoot/pr-slots.ps1"

# The days a re-review waits for its previous reviewer before anyone may
# take it, and after which a claim with no word from its maintainer is
# reported as stale (still respected: the human asks the claimer).
$ReviewQueueHandOverDays = 2
$ReviewQueueStaleClaimDays = 3

# Labels that keep a PR out of every queue: put on hold by a human, waiting
# for a review slot (Review slots workflow), description incomplete (PR
# description workflow).
$ReviewQueueSkipLabels = @('on-hold', 'waiting-for-slot', 'needs-description')

# A maintainer's review comment: its heading opens the comment. "Relecture"
# is the French heading of the PRs reviewed before the switch to English.
$ReviewHeading = '^\s*## (Review|Relecture|Maintainer review|Changes requested)\b'

# What a maintainer's review comment says: Approved, Changes (the author has
# work to do) or Decision (reviewed, waiting on the team: a hand test, a
# product choice, or no verdict line).
function Get-ReviewCommentKind([string]$Body) {
    if (-not ($Body -match $ReviewHeading)) { return $null }
    $heading = $Matches[1]
    if ($heading -eq 'Changes requested') { return 'Changes' }
    if ($Body -match 'Verdict ?: ✅') { return 'Approved' }
    if ($heading -eq 'Maintainer review' -or $Body -match 'Verdict ?: ❌') { return 'Changes' }
    'Decision'
}

# Whether a review comment names the head commit $Head as the one it read:
# the word "head", then within the same line a commit in backticks (7 to 40
# characters), as the review routine writes "Reviewed head: `<sha>`" and
# earlier reviews "Re-review of head `<sha>`". Another commit the review
# quotes ("commits `<sha>`") is not its head. An approval counts for the
# merge only on the head it read: a commit's date is set by its author's
# git and a merge of main is no answer, so the date alone cannot tell (a
# commit made before the approval, pushed after it).
function Test-HeadNamed([string]$Body, [string]$Head) {
    if (-not $Head) { return $false }
    $head = $Head.ToLowerInvariant()
    foreach ($m in [regex]::Matches($Body, '(?i)\bhead\b[^`\n]{0,40}`([0-9a-f]{7,40})`')) {
        if ($head.StartsWith($m.Groups[1].Value.ToLowerInvariant())) { return $true }
    }
    $false
}

# A crash in a title: the word, not "crashpad", nor the crash window, its
# reports and their triage, nor a "panic button".
function Test-CrashTitle([string]$Title) {
    $Title -match '(?i)\b(crash(es|ed|ing)?|panic(s|ked)?|device lost)\b' -and
        $Title -notmatch '(?i)\bcrash (window|reports?|reporter|triage|handler)\b|\bpanic button'
}

# A fix: the fix label, or a conventional "fix(area):" title (an outside
# contributor cannot set labels).
function Test-FixPr([string[]]$Labels, [string]$Title) {
    ($Labels -contains 'fix') -or $Title -match '^\s*fix(\([^)]*\))?!?:'
}

# An issue tied to a crash: signed by the crash triage, or a crash title.
function Test-CrashIssue($Issue) {
    $labels = @($Issue.Labels)
    ($labels -contains 'crash-triaged') -or (Test-CrashTitle ([string]$Issue.Title))
}

# The issue numbers a PR body names on its issue lines (AGENTS.md section 9:
# "Closes #n", "Refs #n"; also the other closing words GitHub reads).
function Get-BodyIssueNumbers([string]$Body) {
    $found = [regex]::Matches([string]$Body, '(?im)\b(?:close[sd]?|fix(?:e[sd])?|resolve[sd]?|refs?)\s*:?\s+#(\d{1,9})\b')
    @($found | ForEach-Object { [int]$_.Groups[1].Value } | Sort-Object -Unique)
}

# A GitHub time as UTC: ConvertFrom-Json of PowerShell 7 turns ISO strings
# into local times, 5.1 leaves them as strings.
function ConvertTo-UtcTime($Value) {
    if ($Value -is [datetime]) {
        if ($Value.Kind -eq [DateTimeKind]::Unspecified) { return [datetime]::SpecifyKind($Value, [DateTimeKind]::Utc) }
        return $Value.ToUniversalTime()
    }
    [datetime]::Parse([string]$Value, [Globalization.CultureInfo]::InvariantCulture, [Globalization.DateTimeStyles]'AdjustToUniversal, AssumeUniversal')
}

# One pull request, from the fields review-queue.ps1 reads, to what the
# rules need. $Node: a pullRequest node of the GraphQL query; $Issues: a
# hashtable issue number -> @{ Title; Labels } for the issues its body names.
function ConvertTo-ReviewItem($Node, $Issues) {
    $author = [string]$Node.author.login
    $labels = @($Node.labels.nodes | ForEach-Object { [string]$_.name })
    # events of the conversation, oldest first: maintainers' reviews, the
    # author's answers, the author's own commits (a merge of main is no answer)
    $events = @()
    # every comment and review, by anyone: a claim stays fresh while its
    # maintainer speaks on the PR
    $said = @()
    foreach ($c in $Node.comments.nodes) {
        $by = [string]$c.author.login
        $said += [pscustomobject]@{ By = $by; At = (ConvertTo-UtcTime $c.createdAt) }
        if ($by -eq $author) {
            $events += [pscustomobject]@{ Kind = 'Answer'; By = $by; At = (ConvertTo-UtcTime $c.createdAt) }
        } elseif ($by -and $by -ne 'github-actions' -and (Test-PrSlotTeam ([string]$c.authorAssociation))) {
            $kind = Get-ReviewCommentKind ([string]$c.body)
            if ($kind) {
                $events += [pscustomobject]@{ Kind = $kind; By = $by; At = (ConvertTo-UtcTime $c.createdAt)
                    OnHead = (Test-HeadNamed ([string]$c.body) ([string]$Node.headRefOid)) }
            }
        }
    }
    foreach ($r in $Node.reviews.nodes) {
        if (-not $r.submittedAt) { continue }
        $by = [string]$r.author.login
        $said += [pscustomobject]@{ By = $by; At = (ConvertTo-UtcTime $r.submittedAt) }
        if ($by -eq $author) {
            $events += [pscustomobject]@{ Kind = 'Answer'; By = $by; At = (ConvertTo-UtcTime $r.submittedAt) }
        } elseif ($by -and (Test-PrSlotTeam ([string]$r.authorAssociation))) {
            $kind = switch ([string]$r.state) { 'APPROVED' { 'Approved' } 'CHANGES_REQUESTED' { 'Changes' } default { $null } }
            if ($kind) {
                $events += [pscustomobject]@{ Kind = $kind; By = $by; At = (ConvertTo-UtcTime $r.submittedAt)
                    OnHead = ($r.commit -and [string]$r.commit.oid -eq [string]$Node.headRefOid) }
            }
        }
    }
    foreach ($c in $Node.authored.nodes) {
        if ($c.commit.parents.totalCount -lt 2) {
            $events += [pscustomobject]@{ Kind = 'Commit'; By = $author; At = (ConvertTo-UtcTime $c.commit.committedDate) }
        }
    }
    # CI: Fail (red), Approval (an outside contributor's run waits for "Approve
    # and run"), Run, Ok, or None (nothing ran yet)
    $commit = $Node.commits.nodes[0].commit
    $rollup = if ($commit.statusCheckRollup) { [string]$commit.statusCheckRollup.state } else { '' }
    $waitsApproval = [bool]@($commit.checkSuites.nodes | Where-Object { $_.conclusion -eq 'ACTION_REQUIRED' }).Count
    $ci = if ($rollup -in 'FAILURE', 'ERROR') { 'Fail' }
        elseif ($waitsApproval) { 'Approval' }
        elseif ($rollup -in 'PENDING', 'EXPECTED') { 'Run' }
        elseif ($rollup -eq 'SUCCESS') { 'Ok' }
        else { 'None' }
    # issues: those GitHub links (Closes) and those the body names (Refs)
    $linked = @{}
    foreach ($i in $Node.closingIssuesReferences.nodes) {
        $linked[[int]$i.number] = [pscustomobject]@{ Title = [string]$i.title; Labels = @($i.labels.nodes | ForEach-Object { [string]$_.name }) }
    }
    foreach ($n in Get-BodyIssueNumbers ([string]$Node.body)) {
        if (-not $linked.ContainsKey($n) -and $Issues -and $Issues.ContainsKey($n)) { $linked[$n] = $Issues[$n] }
    }
    $crashIssues = @($linked.Keys | Where-Object { Test-CrashIssue $linked[$_] } | Sort-Object)
    # claims: who is assigned, and since when (the last assignment event);
    # the PR's author assigned to it claims nothing
    $claims = @()
    foreach ($a in $Node.assignees.nodes) {
        $login = [string]$a.login
        if ($login -eq $author) { continue }
        $since = $null
        foreach ($e in $Node.timelineItems.nodes) {
            if ($e.assignee.login -eq $login -and $e.createdAt) { $since = (ConvertTo-UtcTime $e.createdAt) }
        }
        $claims += [pscustomobject]@{ Login = $login; Since = $since }
    }
    [pscustomobject]@{
        Number = [int]$Node.number; Title = [string]$Node.title; Url = [string]$Node.url
        Head = [string]$Node.headRefOid
        Author = $author; Bot = ([string]$Node.author.__typename -eq 'Bot') -or $author.EndsWith('[bot]') -or $author.StartsWith('app/')
        Draft = [bool]$Node.isDraft; Labels = $labels
        Queued = [bool]$Node.mergeQueueEntry -or [bool]$Node.autoMergeRequest
        Conflict = [string]$Node.mergeable -eq 'CONFLICTING'
        Ci = $ci; Created = (ConvertTo-UtcTime $Node.createdAt)
        Crash = [bool]$crashIssues.Count -or (Test-CrashTitle ([string]$Node.title))
        CrashIssues = $crashIssues
        Fix = Test-FixPr $labels ([string]$Node.title)
        Events = @($events | Sort-Object At)
        Said = $said
        Claims = $claims
    }
}

# Where a PR stands in the review cycle, from its events:
#   Unreviewed  no maintainer review yet
#   ReReview    reviewed, and the author answered since (Since: their first
#               answer after the review)
#   WaitAuthor  changes asked, no answer yet
#   Decision    reviewed, waiting on the team (hand test, product choice)
#   Approved    approved, nothing new since: to merge, not to review
# Reviewer: who wrote the last review; OnHead: it names the PR's head commit.
function Get-ReviewState($Item) {
    $reviews = @($Item.Events | Where-Object { $_.Kind -in 'Approved', 'Changes', 'Decision' })
    if (-not $reviews.Count) { return [pscustomobject]@{ State = 'Unreviewed'; Reviewer = $null; Since = $Item.Created } }
    $last = $reviews[-1]
    # after an approval only a new commit reopens the review: a "thanks" does not
    $answers = @($Item.Events | Where-Object {
        $_.At -gt $last.At -and ($_.Kind -eq 'Commit' -or ($_.Kind -eq 'Answer' -and $last.Kind -ne 'Approved'))
    })
    if ($answers.Count) { return [pscustomobject]@{ State = 'ReReview'; Reviewer = $last.By; Since = $answers[0].At; OnHead = $false } }
    $state = switch ($last.Kind) { 'Approved' { 'Approved' } 'Changes' { 'WaitAuthor' } default { 'Decision' } }
    [pscustomobject]@{ State = $state; Reviewer = $last.By; Since = $last.At; OnHead = [bool]$last.OnHead }
}

# The claims that hold, oldest first: a claim holds until its maintainer
# posts a review after taking it; the review ends it, whether or not they
# unassigned themselves. A claim whose time is unknown is ended by any
# review of theirs.
function Get-ActiveClaims($Item) {
    @($Item.Claims | Where-Object {
        $c = $_
        -not @($Item.Events | Where-Object {
            $_.By -eq $c.Login -and $_.Kind -in 'Approved', 'Changes', 'Decision' -and (-not $c.Since -or $_.At -gt $c.Since)
        }).Count
    } | Sort-Object @{ Expression = { $null -eq $_.Since } }, @{ Expression = { $_.Since } }, @{ Expression = { $_.Login } })
}

# The queue of maintainer $Me: every PR with its Tier (1 to 4, 0 when it is
# not in the queue), why it is out (Skip), whose it is (Owner) and notes.
# $Now: the time the rules compare with (a fixed one in the tests).
function Get-ReviewQueue($Items, [string]$Me, [datetime]$Now) {
    foreach ($item in $Items) {
        $review = Get-ReviewState $item
        $skip = $null; $kind = $null; $owner = $null; $notes = @()
        $claims = Get-ActiveClaims $item
        $others = @($claims | Where-Object { $_.Login -ne $Me })
        $mine = [bool]@($claims | Where-Object { $_.Login -eq $Me }).Count
        $held = @($item.Labels | Where-Object { $ReviewQueueSkipLabels -contains $_ })
        # whose it is comes first: a PR another maintainer holds is theirs,
        # whatever else it waits for, so that -Claim refuses it
        if ($item.Author -eq $Me) { $kind = 'Own'; $skip = 'your own PR' }
        elseif ($item.Bot) { $kind = 'Bot'; $skip = 'bot (dependency update: needs the human''s agreement)' }
        elseif ($others.Count -and -not $mine) {
            $owner = $others[0].Login
            $kind = 'Claimed'
            $skip = "claimed by $owner"
            $last = @($item.Said | Where-Object { $_.By -eq $owner } | ForEach-Object { $_.At }) + @($others[0].Since) | Where-Object { $_ } | Sort-Object | Select-Object -Last 1
            if ($last -and ($Now - $last).TotalDays -ge $ReviewQueueStaleClaimDays) { $kind = 'StaleClaim'; $skip += " (stale: nothing from them for $([int]($Now - $last).TotalDays) days)" }
        }
        elseif ($review.State -eq 'ReReview' -and $review.Reviewer -ne $Me -and -not $mine -and
            ($Now - $review.Since).TotalDays -lt $ReviewQueueHandOverDays) {
            $owner = $review.Reviewer
            $kind = 'Held'
            $hours = [int][Math]::Ceiling($ReviewQueueHandOverDays * 24 - ($Now - $review.Since).TotalHours)
            $skip = "re-review for $owner, open to anyone in $hours h"
        }
        elseif ($item.Draft) { $kind = 'Draft'; $skip = 'draft' }
        elseif ($held.Count) { $kind = 'Label'; $skip = $held -join ', ' }
        elseif ($item.Queued) { $kind = 'Queued'; $skip = 'in the merge queue' }
        elseif ($review.State -eq 'Approved') { $kind = 'Approved'; $skip = "approved by $($review.Reviewer), to merge" }
        elseif ($review.State -eq 'WaitAuthor') { $kind = 'WaitAuthor'; $skip = "changes asked by $($review.Reviewer), waiting on its author" }
        elseif ($review.State -eq 'Decision') { $kind = 'Decision'; $skip = "waiting on a decision ($($review.Reviewer)'s review)"; $owner = $review.Reviewer }
        elseif ($item.Ci -eq 'Fail') { $kind = 'CiRed'; $skip = 'CI red: waiting on its author' }
        $tier = 0
        if (-not $skip) {
            $tier = if ($review.State -eq 'ReReview') { 1 } elseif ($item.Crash) { 2 } elseif ($item.Fix) { 3 } else { 4 }
            if ($mine) { $notes += 'claimed by you' }
            if ($review.State -eq 'ReReview' -and $review.Reviewer -ne $Me) { $notes += "last reviewed by $($review.Reviewer)" }
            if ($item.Ci -eq 'Approval') { $notes += 'CI waits for Approve and run' }
            elseif ($item.Ci -eq 'Run') { $notes += 'CI running' }
            elseif ($item.Ci -eq 'None') { $notes += 'no CI yet' }
            if ($item.Conflict) { $notes += 'conflict with main' }
            if ($item.CrashIssues.Count) { $notes += 'crash ' + (($item.CrashIssues | ForEach-Object { "#$_" }) -join ' ') }
        }
        [pscustomobject]@{
            Number = $item.Number; Title = $item.Title; Url = $item.Url; Author = $item.Author
            Tier = $tier; Kind = @('', 're-review', 'crash', 'fix', 'new')[$tier]
            Rank = $(if ($item.Crash) { 0 } elseif ($item.Fix) { 1 } else { 2 })
            Since = $review.Since; State = $review.State; Reviewer = $review.Reviewer
            Mine = $mine; Owner = $owner; Skip = $skip; SkipKind = $kind; Notes = $notes
            Ci = $item.Ci; Conflict = $item.Conflict; Head = $item.Head; OnHead = [bool]$review.OnHead
        }
    }
}

# Why $Me may not claim a PR ($Entry: its line of Get-ReviewQueue), or
# $null: their own PR, or a PR another maintainer holds (a claim, a fresh
# re-review of theirs, a decision their review left open).
function Get-ClaimRefusal($Entry, [string]$Me) {
    if ($Entry.Mine) { return $null }
    if ($Entry.SkipKind -eq 'Own') { return 'it is your own PR' }
    if ($Entry.Owner -and $Entry.Owner -ne $Me) { return "$($Entry.Skip); ask $($Entry.Owner) before taking it" }
    $null
}

# After $Me assigned themselves: the maintainer whose claim came first, if
# another claim holds and is older (two claims a few seconds apart: the
# later one backs off), else $null.
function Get-EarlierClaim($Item, [string]$Me) {
    foreach ($c in Get-ActiveClaims $Item) {
        if ($c.Login -eq $Me) { return $null }
        return $c.Login
    }
    $null
}

# The PRs $Me approved (their last review is a ✅ verdict or a GitHub
# approval, nothing new since) and nobody queued yet: the queue leaves them
# out, and the reviewer offers their human the merge (AGENTS.md section 11,
# the review-next routine), but only of the head the approval names
# (OnHead); another head is re-reviewed first. Oldest approval first.
function Get-ApprovedToMerge($Entries, [string]$Me) {
    @($Entries | Where-Object { $_.SkipKind -eq 'Approved' -and $_.Reviewer -eq $Me } | Sort-Object Since, Number)
}

# The PRs whose last review is $Me's and left a decision open (⚠️ verdict:
# a hand or grid test, a product choice): every queue leaves them out and
# no other maintainer may claim them, so only their reviewer brings them
# back, with what their human decided. Oldest review first.
function Get-WaitingOnDecision($Entries, [string]$Me) {
    @($Entries | Where-Object { $_.SkipKind -eq 'Decision' -and $_.Reviewer -eq $Me } | Sort-Object Since, Number)
}

# The queue in order: the PRs already claimed by $Me first (finish what you
# started), then tier, crash / fix / rest, longest waiting, oldest number.
function Get-OrderedReviewQueue($Entries) {
    @($Entries | Where-Object { $_.Tier -gt 0 } | Sort-Object `
        @{ Expression = { -not $_.Mine } }, `
        @{ Expression = { $_.Tier } }, `
        @{ Expression = { $_.Rank } }, `
        @{ Expression = { $_.Since } }, `
        @{ Expression = { $_.Number } })
}
