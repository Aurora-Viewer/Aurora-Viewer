# Review queue fixtures (scripts/tools/review-queue.ps1): pull requests as
# GitHub's GraphQL gives them, and where the rules put them. No GitHub call;
# check.ps1 and the CI run it.
$ErrorActionPreference = 'Stop'
. "$PSScriptRoot/tools/review-queue.ps1"
$failures = 0
function Assert-Queue([bool]$Condition, [string]$Message) {
    if (-not $Condition) { Write-Host "FAIL: $Message" -ForegroundColor Red; $script:failures++ }
}

$now = [datetime]::SpecifyKind([datetime]'2026-11-27T12:00:00', [DateTimeKind]::Utc)
function At([double]$HoursAgo) { $now.AddHours(-$HoursAgo).ToString('yyyy-MM-ddTHH:mm:ssZ') }

# A GraphQL pullRequest node. -Comments / -Reviews / -Commits: lists of
# @(login, association, hours ago, body or state); commits: @(hours ago,
# parents). Defaults: an outside contributor's PR, green CI, nothing said.
function New-Node([int]$Number, [string]$Title = 'feat(ui): something', [string]$Author = 'alice',
    [string[]]$Labels = @(), [object[]]$Comments = @(), [object[]]$Reviews = @(), [object[]]$Commits = @(, @(200, 1)),
    [string]$Rollup = 'SUCCESS', [string[]]$Suites = @('SUCCESS'), [string]$Body = 'No issue',
    [object[]]$Closing = @(), [string[]]$Assignees = @(), [object[]]$Assigned = @(),
    [switch]$Draft, [switch]$Queued, [switch]$Bot, [double]$CreatedHoursAgo = 300,
    [string]$Head = 'abc1234def5678901234567890abcdef12345678') {
    [pscustomobject]@{
        number = $Number; title = $Title; url = "https://example.invalid/pull/$Number"; body = $Body
        isDraft = [bool]$Draft; createdAt = (At $CreatedHoursAgo); mergeable = 'MERGEABLE'; headRefOid = $Head
        author = [pscustomobject]@{ __typename = $(if ($Bot) { 'Bot' } else { 'User' }); login = $Author }
        labels = [pscustomobject]@{ nodes = @($Labels | ForEach-Object { [pscustomobject]@{ name = $_ } }) }
        mergeQueueEntry = $(if ($Queued) { [pscustomobject]@{ id = 'q' } } else { $null }); autoMergeRequest = $null
        assignees = [pscustomobject]@{ nodes = @($Assignees | ForEach-Object { [pscustomobject]@{ login = $_ } }) }
        timelineItems = [pscustomobject]@{ nodes = @($Assigned | ForEach-Object { [pscustomobject]@{ createdAt = (At $_[1]); assignee = [pscustomobject]@{ login = $_[0] } } }) }
        closingIssuesReferences = [pscustomobject]@{ nodes = @($Closing | ForEach-Object {
            [pscustomobject]@{ number = $_[0]; title = $_[1]; labels = [pscustomobject]@{ nodes = @($_[2] | Where-Object { $_ } | ForEach-Object { [pscustomobject]@{ name = $_ } }) } } }) }
        commits = [pscustomobject]@{ nodes = @([pscustomobject]@{ commit = [pscustomobject]@{
            statusCheckRollup = $(if ($Rollup) { [pscustomobject]@{ state = $Rollup } } else { $null })
            checkSuites = [pscustomobject]@{ nodes = @($Suites | ForEach-Object { [pscustomobject]@{ conclusion = $_ } }) } } }) }
        authored = [pscustomobject]@{ nodes = @($Commits | ForEach-Object { [pscustomobject]@{ commit = [pscustomobject]@{ committedDate = (At $_[0]); parents = [pscustomobject]@{ totalCount = $_[1] } } } }) }
        comments = [pscustomobject]@{ nodes = @($Comments | ForEach-Object { [pscustomobject]@{ author = [pscustomobject]@{ login = $_[0] }; authorAssociation = $_[1]; createdAt = (At $_[2]); body = $_[3] } }) }
        reviews = [pscustomobject]@{ nodes = @($Reviews | ForEach-Object { [pscustomobject]@{ author = [pscustomobject]@{ login = $_[0] }; authorAssociation = $_[1]; submittedAt = (At $_[2]); state = $_[3]
            commit = [pscustomobject]@{ oid = $(if ($_.Count -gt 4) { $_[4] } else { '0000000000000000000000000000000000000000' }) } } }) }
    }
}
function Get-Entry($Node, [string]$Me = 'maya', $Issues = @{}) {
    $item = ConvertTo-ReviewItem $Node $Issues
    Get-ReviewQueue @($item) $Me $now
}
$changes = "## Changes requested`n`nTwo points."
$approved = "## Review`n`n**Verdict: ✅ ready to merge**`n`n| Check | Result |`n|---|---|`n| CI | ✅ |"
$warn = "## Review`n`n**Verdict: ⚠️ needs the macOS hand test**`n`n| CI | ✅ |"

# --- the four tiers --------------------------------------------------------
$e = Get-Entry (New-Node 1)
Assert-Queue ($e.Tier -eq 4 -and $e.Kind -eq 'new') 'an untouched PR is new (tier 4)'
$e = Get-Entry (New-Node 2 -Labels 'fix')
Assert-Queue ($e.Tier -eq 3) 'the fix label is tier 3'
$e = Get-Entry (New-Node 3 -Title 'fix(render): no crash when the GPU is lost' -Labels 'fix')
Assert-Queue ($e.Tier -eq 2) 'a crash in the title is tier 2, before fix'
$e = Get-Entry (New-Node 4 -Body 'Closes #900' -Closing @(, @(900, 'Viewer stops at login', @('crash-triaged'))))
Assert-Queue ($e.Tier -eq 2 -and ($e.Notes -join ' ') -match '#900') 'a linked issue signed by the crash triage is tier 2, named in the notes'
$e = Get-Entry (New-Node 5 -Body "Refs #901`n")
Assert-Queue ($e.Tier -eq 4) 'a Refs to an unknown issue changes nothing'
$e = Get-Entry (New-Node 5 -Body "Refs #901`n") 'maya' @{ 901 = [pscustomobject]@{ Title = 'Panic in the mesh loader'; Labels = @() } }
Assert-Queue ($e.Tier -eq 2) 'a Refs to a crash issue is tier 2'
$e = Get-Entry (New-Node 6 -Title 'feat(ui): a crash window with more details')
Assert-Queue ($e.Tier -eq 4) 'the crash window is no crash'
$e = Get-Entry (New-Node 7 -Comments @(, @('maya', 'MEMBER', 100, $changes)) -Commits @(@(200, 1), @(50, 1)))
Assert-Queue ($e.Tier -eq 1 -and $e.Kind -eq 're-review') 'a commit of the author after changes asked is a re-review (tier 1)'

# --- what counts as a review and as an answer ------------------------------
$e = Get-Entry (New-Node 10 -Comments @(, @('maya', 'MEMBER', 100, $changes)))
Assert-Queue ($e.Tier -eq 0 -and $e.SkipKind -eq 'WaitAuthor') 'changes asked with no answer: waiting on the author'
$e = Get-Entry (New-Node 11 -Comments @(@('maya', 'MEMBER', 100, $changes), @('alice', 'CONTRIBUTOR', 20, 'Thanks, done.')))
Assert-Queue ($e.Tier -eq 1) 'an author comment after changes asked is an answer'
$e = Get-Entry (New-Node 12 -Comments @(, @('maya', 'MEMBER', 100, $changes)) -Commits @(@(200, 1), @(50, 2)))
Assert-Queue ($e.SkipKind -eq 'WaitAuthor') 'a merge of main is no answer'
$e = Get-Entry (New-Node 13 -Comments @(, @('alice', 'CONTRIBUTOR', 100, "## Review`n`n**Verdict: ✅ ready to merge**")))
Assert-Queue ($e.Tier -eq 4) 'the author''s own review comment is not a maintainer review'
$e = Get-Entry (New-Node 14 -Comments @(, @('github-actions', 'NONE', 100, "## Changes requested`n`nThe description misses…")))
Assert-Queue ($e.Tier -eq 4) 'the description workflow''s comment is not a review'
$e = Get-Entry (New-Node 15 -Comments @(, @('bob', 'CONTRIBUTOR', 100, $changes)))
Assert-Queue ($e.Tier -eq 4) 'an outsider''s review comment is not a maintainer review'
$e = Get-Entry (New-Node 16 -Reviews @(, @('maya', 'MEMBER', 100, 'CHANGES_REQUESTED')) -Commits @(@(200, 1), @(10, 1)))
Assert-Queue ($e.Tier -eq 1) 'a GitHub review asking changes, then a commit: re-review'
$e = Get-Entry (New-Node 17 -Comments @(, @('maya', 'MEMBER', 100, $approved)))
Assert-Queue ($e.SkipKind -eq 'Approved') 'approved and nothing new: to merge, not to review'
$e = Get-Entry (New-Node 18 -Comments @(@('maya', 'MEMBER', 100, $approved), @('alice', 'CONTRIBUTOR', 90, 'Thanks!')))
Assert-Queue ($e.SkipKind -eq 'Approved') 'a thanks after an approval reopens nothing'
$e = Get-Entry (New-Node 19 -Comments @(, @('maya', 'MEMBER', 100, $approved)) -Commits @(@(200, 1), @(30, 1)))
Assert-Queue ($e.Tier -eq 1) 'a commit after an approval needs a re-review'
$e = Get-Entry (New-Node 20 -Comments @(, @('maya', 'MEMBER', 100, $warn)))
Assert-Queue ($e.SkipKind -eq 'Decision' -and $e.Owner -eq 'maya') 'a ⚠️ verdict waits on the team, owned by its reviewer'
$e = Get-Entry (New-Node 21 -Comments @(@('maya', 'MEMBER', 100, $changes), @('maya', 'MEMBER', 50, $approved)) -Commits @(@(200, 1), @(70, 1)))
Assert-Queue ($e.SkipKind -eq 'Approved') 'the last review decides: a commit before the approval is answered already'
$e = Get-Entry (New-Node 22 -Comments @(, @('maya', 'MEMBER', 100, "## Maintainer review`n`nPlease rename the setting.")) -Commits @(@(200, 1), @(10, 1)))
Assert-Queue ($e.Tier -eq 1) 'a "## Maintainer review" without ✅ asks for changes'

# --- left out --------------------------------------------------------------
Assert-Queue ((Get-Entry (New-Node 30 -Author 'maya')).SkipKind -eq 'Own') 'your own PR'
Assert-Queue ((Get-Entry (New-Node 31 -Author 'dependabot' -Bot)).SkipKind -eq 'Bot') 'a bot''s PR'
Assert-Queue ((Get-Entry (New-Node 32 -Author 'carrot')).SkipKind -ne 'Bot') 'a login ending in t is no bot'
Assert-Queue ((Get-Entry (New-Node 33 -Draft)).SkipKind -eq 'Draft') 'a draft'
foreach ($l in 'on-hold', 'waiting-for-slot', 'needs-description') {
    Assert-Queue ((Get-Entry (New-Node 34 -Labels $l, 'fix')).SkipKind -eq 'Label') "the $l label"
}
Assert-Queue ((Get-Entry (New-Node 35 -Queued)).SkipKind -eq 'Queued') 'in the merge queue'
Assert-Queue ((Get-Entry (New-Node 36 -Rollup 'FAILURE')).SkipKind -eq 'CiRed') 'a red CI waits on its author'
$e = Get-Entry (New-Node 37 -Suites 'SUCCESS', 'ACTION_REQUIRED')
Assert-Queue ($e.Tier -eq 4 -and ($e.Notes -join ' ') -match 'Approve and run') 'a run waiting for approval stays in, with a note'

# --- no overlap between maintainers ----------------------------------------
$e = Get-Entry (New-Node 40 -Labels 'fix' -Assignees 'odessa' -Assigned @(, @('odessa', 5)))
Assert-Queue ($e.SkipKind -eq 'Claimed' -and $e.Owner -eq 'odessa') 'a PR assigned to another maintainer is theirs'
$e = Get-Entry (New-Node 41 -Assignees 'odessa' -Assigned @(, @('odessa', 120)))
Assert-Queue ($e.SkipKind -eq 'StaleClaim') 'a claim with no word for days is reported stale, still left out'
$e = Get-Entry (New-Node 42 -Assignees 'odessa' -Assigned @(, @('odessa', 120)) -Comments @(, @('odessa', 'MEMBER', 10, 'Looking at the shader part now.')))
Assert-Queue ($e.SkipKind -eq 'Claimed') 'a claim whose maintainer spoke recently is not stale'
$e = Get-Entry (New-Node 43 -Assignees 'maya' -Assigned @(, @('maya', 5)))
Assert-Queue ($e.Tier -eq 4 -and $e.Mine) 'your own claim stays in your queue'
$e = Get-Entry (New-Node 44 -Comments @(, @('odessa', 'MEMBER', 100, $changes)) -Commits @(@(200, 1), @(10, 1)))
Assert-Queue ($e.SkipKind -eq 'Held' -and $e.Owner -eq 'odessa') 'a fresh re-review belongs to its previous reviewer'
$e = Get-Entry (New-Node 44 -Comments @(, @('odessa', 'MEMBER', 100, $changes)) -Commits @(@(200, 1), @(10, 1))) 'odessa'
Assert-Queue ($e.Tier -eq 1) '…and is in that reviewer''s queue'
$e = Get-Entry (New-Node 45 -Comments @(, @('odessa', 'MEMBER', 200, $changes)) -Commits @(@(300, 1), @(60, 1)))
Assert-Queue ($e.Tier -eq 1 -and ($e.Notes -join ' ') -match 'odessa') 'after the hand-over days anyone may take it, with its reviewer named'
$e = Get-Entry (New-Node 46 -Comments @(, @('odessa', 'MEMBER', 100, $changes)) -Commits @(@(200, 1), @(10, 1)) -Assignees 'maya' -Assigned @(, @('maya', 1)))
Assert-Queue ($e.Tier -eq 1) 'a re-review you claimed (handed over) is yours'
$e = Get-Entry (New-Node 47 -Assignees 'odessa' -Assigned @(, @('odessa', 100)) -Comments @(, @('odessa', 'MEMBER', 50, $changes)))
Assert-Queue ($e.SkipKind -eq 'WaitAuthor') 'a claim ends with its maintainer''s review, even left assigned'
$e = Get-Entry (New-Node 48 -Assignees 'odessa' -Assigned @(, @('odessa', 100)) -Comments @(, @('odessa', 'MEMBER', 50, $changes)) -Commits @(@(200, 1), @(10, 1)))
Assert-Queue ($e.SkipKind -eq 'Held') '…and the re-review then follows the hand-over rule'
$e = Get-Entry (New-Node 49 -Assignees 'odessa' -Assigned @(, @('odessa', 10)) -Comments @(, @('odessa', 'MEMBER', 50, $changes)) -Commits @(@(200, 1), @(20, 1)))
Assert-Queue ($e.SkipKind -eq 'Claimed') 'a claim taken after their last review holds (the re-review in progress)'

# a claim comes before what else the PR waits for (-Claim refuses it)
$e = Get-Entry (New-Node 60 -Rollup 'FAILURE' -Assignees 'odessa' -Assigned @(, @('odessa', 5)))
Assert-Queue ($e.SkipKind -eq 'Claimed' -and (Get-ClaimRefusal $e 'maya')) 'a claimed PR whose CI turned red stays theirs'
$e = Get-Entry (New-Node 61 -Draft -Assignees 'odessa' -Assigned @(, @('odessa', 5)))
Assert-Queue ($e.SkipKind -eq 'Claimed') 'a claimed PR turned draft stays theirs'
$e = Get-Entry (New-Node 62 -Rollup 'FAILURE' -Comments @(, @('odessa', 'MEMBER', 100, $changes)) -Commits @(@(200, 1), @(10, 1)))
Assert-Queue ($e.SkipKind -eq 'Held' -and (Get-ClaimRefusal $e 'maya')) 'a fresh re-review with a red CI stays with its reviewer'
$e = Get-Entry (New-Node 63 -Comments @(, @('odessa', 'MEMBER', 100, $warn)))
Assert-Queue ((Get-ClaimRefusal $e 'maya') -and -not (Get-ClaimRefusal $e 'odessa')) 'a decision left by a review: only its reviewer may claim it'
Assert-Queue ((Get-ClaimRefusal (Get-Entry (New-Node 64 -Author 'maya')) 'maya') -match 'own') 'your own PR cannot be claimed'
Assert-Queue (-not (Get-ClaimRefusal (Get-Entry (New-Node 65 -Rollup 'FAILURE')) 'maya')) 'a free PR can be claimed, whatever it waits for'
$e = Get-Entry (New-Node 66 -Assignees 'alice' -Assigned @(, @('alice', 5)))
Assert-Queue ($e.Tier -eq 4) 'the PR''s author assigned to it claims nothing'
$e = Get-Entry (New-Node 67 -Assignees 'odessa' -Comments @(, @('odessa', 'MEMBER', 50, $changes)))
Assert-Queue ($e.SkipKind -eq 'WaitAuthor') 'a claim of unknown time is ended by any review of its maintainer'
$e = Get-Entry (New-Node 68 -Assignees 'odessa', 'tom' -Assigned @(@('tom', 2), @('odessa', 9)))
Assert-Queue ($e.Owner -eq 'odessa') 'with two claims, the oldest is the owner'
# two claims seconds apart: the later one backs off
$item = ConvertTo-ReviewItem (New-Node 69 -Assignees 'maya', 'odessa' -Assigned @(@('odessa', 0.002), @('maya', 0.001))) @{}
Assert-Queue ((Get-EarlierClaim $item 'maya') -eq 'odessa' -and -not (Get-EarlierClaim $item 'odessa')) 'the later of two claims backs off'
# a reviewer claiming their re-review again: the new assignment holds
$e = Get-Entry (New-Node 70 -Assignees 'odessa' -Assigned @(, @('odessa', 1)) -Comments @(, @('odessa', 'MEMBER', 100, $changes)) -Commits @(@(200, 1), @(60, 1)))
Assert-Queue ($e.SkipKind -eq 'Claimed') 'a reviewer who claims again holds their re-review past the hand-over'

# --- more on reviews and answers ------------------------------------------
$e = Get-Entry (New-Node 71 -Comments @(@('odessa', 'MEMBER', 100, $warn), @('alice', 'CONTRIBUTOR', 70, 'Tested on macOS, it works.'))) 'odessa'
Assert-Queue ($e.Tier -eq 1) 'an answer after a decision is a re-review'
$e = Get-Entry (New-Node 72 -Comments @(, @('maya', 'MEMBER', 100, $changes)) -Reviews @(, @('alice', 'CONTRIBUTOR', 20, 'COMMENTED')))
Assert-Queue ($e.Tier -eq 1) 'a review reply of the author is an answer'
$e = Get-Entry (New-Node 73 -Comments @(, @('odessa', 'MEMBER', 100, 'Nice idea, will look later.')))
Assert-Queue ($e.Tier -eq 4) 'a maintainer comment without a review heading is no review'
$e = Get-Entry (New-Node 74 -Title 'fix(ui): the field keeps its focus')
Assert-Queue ($e.Tier -eq 3) 'a fix(area): title without the label is tier 3'
$e = Get-Entry (New-Node 75 -Title 'feat(ui): fixes for the crash triage page')
Assert-Queue ($e.Tier -eq 4) 'crash triage is no crash, a feat title no fix'

# --- order -----------------------------------------------------------------
$nodes = @(
    (New-Node 50 -CreatedHoursAgo 900)                                                      # new, oldest
    (New-Node 51 -Labels 'fix' -CreatedHoursAgo 100)                                        # fix
    (New-Node 52 -Title 'fix(ui): crash on start' -CreatedHoursAgo 50)                      # crash
    (New-Node 53 -Comments @(, @('maya', 'MEMBER', 100, $changes)) -Commits @(@(200, 1), @(5, 1)))      # re-review, recent
    (New-Node 54 -Labels 'fix' -Comments @(, @('maya', 'MEMBER', 100, $changes)) -Commits @(@(200, 1), @(4, 1)))  # re-review of a fix
    (New-Node 55 -Comments @(, @('maya', 'MEMBER', 100, $changes)) -Commits @(@(200, 1), @(40, 1)))     # re-review, older answer
    (New-Node 56 -CreatedHoursAgo 20 -Assignees 'maya' -Assigned @(, @('maya', 2)))       # new, claimed by maya
    (New-Node 57 -Labels 'fix' -CreatedHoursAgo 400)                                        # fix, older
)
$items = @($nodes | ForEach-Object { ConvertTo-ReviewItem $_ @{} })
$order = (Get-OrderedReviewQueue (Get-ReviewQueue $items 'maya' $now) | ForEach-Object { $_.Number }) -join ','
Assert-Queue ($order -eq '56,54,55,53,52,57,51,50') "order: claimed, re-reviews (fix first, then longest waiting), crash, fix (oldest first), new; got $order"

# --- approved, to merge ----------------------------------------------------
$onHead = "## Review`n`n**Verdict: ✅ ready to merge**`n`nReviewed head: ``abc1234de``"
$nodes = @(
    (New-Node 60 -Comments @(, @('maya', 'MEMBER', 100, $onHead)))                         # maya's ✅ on the head
    (New-Node 61 -Comments @(, @('maya', 'MEMBER', 120, $onHead)) -Rollup 'PENDING' -Suites 'ACTION_REQUIRED')  # CI to approve
    (New-Node 62 -Comments @(, @('olga', 'MEMBER', 100, $onHead)))                         # another maintainer's ✅
    (New-Node 63 -Comments @(, @('maya', 'MEMBER', 100, $onHead)) -Queued)                 # already in the merge queue
    (New-Node 64 -Comments @(, @('maya', 'MEMBER', 100, $onHead)) -Commits @(@(200, 1), @(5, 1)))  # a commit since: re-review
    (New-Node 65 -Reviews @(, @('maya', 'MEMBER', 100, 'APPROVED', 'abc1234def5678901234567890abcdef12345678')))  # GitHub approval of the head
    (New-Node 66 -Comments @(, @('maya', 'MEMBER', 90, $approved)))                        # ✅ naming no head
    (New-Node 67 -Comments @(, @('maya', 'MEMBER', 90, $onHead)) -Head 'fff0000aaaa1111bbbb2222cccc3333dddd4444')  # pushed after: another head
    (New-Node 68 -Reviews @(, @('maya', 'MEMBER', 90, 'APPROVED', 'fff0000aaaa1111bbbb2222cccc3333dddd4444')))  # GitHub approval of an older commit
    (New-Node 69 -Comments @(, @('maya', 'MEMBER', 100, $onHead)) -Labels 'on-hold')       # on hold
    (New-Node 70 -Comments @(, @('maya', 'MEMBER', 100, $onHead)) -Draft)                  # back to draft
    (New-Node 71 -Comments @(@('maya', 'MEMBER', 100, $onHead), @('olga', 'MEMBER', 50, $changes)))  # another maintainer asked changes since
    (New-Node 72 -Comments @(, @('maya', 'MEMBER', 100, $onHead)) -Commits @(@(200, 1), @(5, 2)) -Head '9990000aaaa1111bbbb2222cccc3333dddd4444')  # a merge of main since: a new head
)
$items = @($nodes | ForEach-Object { ConvertTo-ReviewItem $_ @{} })
$merge = @(Get-ApprovedToMerge (Get-ReviewQueue $items 'maya' $now) 'maya')
$list = ($merge | ForEach-Object { $_.Number }) -join ','
Assert-Queue ($list -eq '61,60,65,72,66,67,68') "to merge: your approvals only, oldest first, not queued, held, draft nor answered; got $list"
$reviewed = ($merge | Where-Object { $_.OnHead } | ForEach-Object { $_.Number }) -join ','
Assert-Queue ($reviewed -eq '61,60,65') "only an approval naming the head counts for the merge (no head named, a later push, a merge of main: re-review); got $reviewed"
Assert-Queue (($merge | Where-Object { $_.Number -eq 61 }).Ci -eq 'Approval') 'an approved PR keeps its CI state (runs to approve before the merge)'
$decision = "## Review`n`n**Verdict: ⚠️ needs a decision**"
$nodes = @(
    (New-Node 90 -Comments @(, @('maya', 'MEMBER', 100, $decision)))                       # maya's ⚠️
    (New-Node 91 -Comments @(, @('olga', 'MEMBER', 100, $decision)))                       # another maintainer's ⚠️
    (New-Node 92 -Comments @(@('maya', 'MEMBER', 100, $decision), @('alice', 'CONTRIBUTOR', 50, 'Done')))  # answered: a re-review
    (New-Node 93 -Comments @(, @('maya', 'MEMBER', 100, $onHead)))                         # maya's ✅: to merge, not here
)
$items = @($nodes | ForEach-Object { ConvertTo-ReviewItem $_ @{} })
$waiting = (@(Get-WaitingOnDecision (Get-ReviewQueue $items 'maya' $now) 'maya') | ForEach-Object { $_.Number }) -join ','
Assert-Queue ($waiting -eq '90') "waiting on your decision: your own ⚠️ reviews with no answer since; got $waiting"
Assert-Queue (Test-HeadNamed 'Re-review of head `abc1234`.' 'abc1234def') 'a short sha names the head'
Assert-Queue (-not (Test-HeadNamed 'Re-review of head `abc123`.' 'abc1234def')) 'six characters name no head'
Assert-Queue (-not (Test-HeadNamed 'see 1234567 tests' 'abc1234def')) 'another number is not the head'
Assert-Queue (-not (Test-HeadNamed 'abc1234' '')) 'no head known: not reviewed'

# --- helpers ---------------------------------------------------------------
$n = (Get-BodyIssueNumbers "Closes #12`nRefs #34`nrefs: #56`nFixes #12`nSee #78") -join ','
Assert-Queue ($n -eq '12,34,56') "issue lines of a body; got $n"
Assert-Queue ((Get-ReviewCommentKind "Some text`n## Changes requested") -eq $null) 'a heading further down is no review'
Assert-Queue ((ConvertTo-UtcTime '2026-11-27T10:00:00Z').Hour -eq 10) 'GitHub times are read as UTC'
$local = [datetime]::SpecifyKind([datetime]'2026-11-27T10:00:00', [DateTimeKind]::Utc).ToLocalTime()
Assert-Queue ((ConvertTo-UtcTime $local).Hour -eq 10 -and (ConvertTo-UtcTime $local).Kind -eq 'Utc') 'a time PowerShell 7 already made local is put back in UTC'
$n = @(Get-BodyIssueNumbers 'Refs #123456789012').Count
Assert-Queue ($n -eq 0) 'a number too long for an issue is ignored'
$one = @(Get-OrderedReviewQueue (Get-ReviewQueue @(ConvertTo-ReviewItem (New-Node 80) @{}) 'maya' $now))
Assert-Queue ($one.Count -eq 1) 'a queue of one PR counts one'

if ($failures) { Write-Host "$failures review queue test(s) failed." -ForegroundColor Red; exit 1 }
Write-Host 'Review queue rules: all tests pass.' -ForegroundColor Green
