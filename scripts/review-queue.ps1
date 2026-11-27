<#
.SYNOPSIS
    The review queue of a maintainer (HUMANS.md "Review queue", AGENTS.md
    section 11): the open pull requests of other people to review next, in
    order, without the ones another maintainer has in hand. The rules are in
    scripts/tools/review-queue.ps1; this script reads GitHub (one GraphQL
    query per 25 PRs) and only writes with -Claim or -Release.

    Order: re-reviews (the author answered a review), then PRs tied to a
    crash, then the fix label, then the rest not reviewed yet. A maintainer
    claims a PR (assigns themselves) before reading it: the others' queues
    leave it out.

.EXAMPLE
    ./scripts/review-queue.ps1
    Your queue, top 20 first, and what is left out and why (counts).

.EXAMPLE
    ./scripts/review-queue.ps1 -All
    Also every PR left out, with its reason (claimed by whom, waiting on its
    author, held for its previous reviewer…).

.EXAMPLE
    ./scripts/review-queue.ps1 -Claim 2158
    Assigns you to #2158, unless another maintainer holds it. -Release 2158
    when you hand it back unreviewed; after your review the claim stays (it
    records who reviewed it).

.EXAMPLE
    ./scripts/review-queue.ps1 -Json -Top 5
    The first five, for an agent: number, kind, notes, URL.

.EXAMPLE
    ./scripts/review-queue.ps1 -ToMerge -Json
    The PRs you approved (✅) that nobody queued yet, with their CI state and
    whether your approval names their head commit: the queue leaves them
    out, so the reviewer offers the merge (of a reviewed head only).
#>
param(
    [int]$Top = 20,
    [switch]$All,
    [switch]$Json,
    [int]$Claim,
    [int]$Release,
    # only the PRs you approved that wait for their merge
    [switch]$ToMerge,
    # see the queue as another maintainer sees it (their login)
    [string]$As
)
. "$PSScriptRoot/common.ps1"
. "$PSScriptRoot/tools/review-queue.ps1"
$ErrorActionPreference = 'Stop'

$repo = Get-MainRepo $PSScriptRoot
if (-not $repo) { throw 'No remote names the main repository on GitHub.' }
$owner, $name = $repo -split '/', 2
function Invoke-Gh {
    $output = & gh @args
    if ($LASTEXITCODE -ne 0) { throw "gh $($args[0..1] -join ' ') failed." }
    $output
}
if ($As -and ($Claim -or $Release)) { throw '-As only shows a queue: claim and release act as you.' }
if ($Claim -and $Release) { throw 'Give -Claim or -Release, not both.' }
$me = if ($As) { $As } else { [string](Invoke-Gh api user --jq .login) }

# The fields of a pull request the rules read (ConvertTo-ReviewItem). No
# double quote in the queries: PowerShell 5.1 drops the inner quotes of a
# native argument.
$prFields = @'
        number title url body state isDraft createdAt mergeable headRefOid
        author { __typename login }
        labels(first: 20) { nodes { name } }
        mergeQueueEntry { id }
        autoMergeRequest { enabledAt }
        assignees(first: 10) { nodes { login } }
        timelineItems(itemTypes: [ASSIGNED_EVENT], last: 10) { nodes { ... on AssignedEvent { createdAt assignee { ... on User { login } } } } }
        closingIssuesReferences(first: 10) { nodes { number title labels(first: 20) { nodes { name } } } }
        commits(last: 1) { nodes { commit { statusCheckRollup { state } checkSuites(first: 20) { nodes { conclusion } } } } }
        authored: commits(last: 30) { nodes { commit { committedDate parents { totalCount } } } }
        comments(last: 60) { nodes { author { login } authorAssociation createdAt body } }
        reviews(last: 30) { nodes { author { login } authorAssociation state submittedAt commit { oid } } }
'@

# One PR, through the same fields, or $null when there is no such PR. A
# local 'Continue': gh writes GraphQL errors on stderr, which Windows
# PowerShell 5.1 turns into a terminating error under 'Stop', 2>$null or not.
function Read-Pr([int]$Number) {
    $ErrorActionPreference = 'Continue'
    $out = & gh api graphql -f "query=query(`$o: String!, `$n: String!, `$p: Int!) { repository(owner: `$o, name: `$n) { pullRequest(number: `$p) { $prFields } } }" -f "o=$owner" -f "n=$name" -F "p=$Number" 2>$null
    if (-not $out) { return $null }
    ($out | Out-String | ConvertFrom-Json).data.repository.pullRequest
}
function Test-Assigned($Pr) { @($Pr.assignees.nodes | ForEach-Object { $_.login }) -contains $me }

# Release: you hand a PR back without reviewing it.
if ($Release) {
    $one = Read-Pr $Release
    if (-not $one) { Write-Host "#${Release}: no such pull request." -ForegroundColor Yellow; exit 1 }
    if (-not (Test-Assigned $one)) { Write-Host "#${Release} is not assigned to you: nothing to release."; exit 0 }
    Invoke-Gh pr edit $Release --repo $repo --remove-assignee '@me' | Out-Null
    Write-Host "#${Release}: released."
    exit 0
}

# Claim: the PR read again just before, through the same rules as the queue,
# so a PR another maintainer holds is refused (Get-ClaimRefusal). Its body's
# "Refs" issues are not read: they only change the order.
if ($Claim) {
    $one = Read-Pr $Claim
    if (-not $one) { Write-Host "#${Claim}: no such pull request." -ForegroundColor Yellow; exit 1 }
    if ($one.state -ne 'OPEN') { Write-Host "#${Claim} is $(([string]$one.state).ToLower()): nothing to claim." -ForegroundColor Yellow; exit 1 }
    $entry = Get-ReviewQueue @(ConvertTo-ReviewItem $one @{}) $me (Get-Date).ToUniversalTime()
    if ($entry.Mine) { Write-Host "#${Claim} is already yours."; exit 0 }
    $refusal = Get-ClaimRefusal $entry $me
    if ($refusal) { Write-Host "#${Claim} not claimed: $refusal." -ForegroundColor Yellow; exit 1 }
    # still assigned from your last review, which ended that claim: a new
    # assignment starts a new one (an assignee added again may leave no
    # new assignment event)
    if (Test-Assigned $one) { Invoke-Gh pr edit $Claim --repo $repo --remove-assignee '@me' | Out-Null }
    Invoke-Gh pr edit $Claim --repo $repo --add-assignee '@me' | Out-Null
    # two maintainers claiming within seconds: the later one backs off
    $again = Read-Pr $Claim
    if ($again) {
        $first = Get-EarlierClaim (ConvertTo-ReviewItem $again @{}) $me
        if ($first) {
            Invoke-Gh pr edit $Claim --repo $repo --remove-assignee '@me' | Out-Null
            Write-Host "#${Claim} not claimed: $first claimed it a moment before you." -ForegroundColor Yellow
            exit 1
        }
    }
    $why = if ($entry.Skip) { " Note: $($entry.Skip)." } else { '' }
    Write-Host "#${Claim}: claimed by $me.$why https://github.com/$repo/pull/$Claim"
    exit 0
}

# The open PRs, 25 at a time (a page of 50 with these comments can pass
# GitHub's time limit).
$query = "query(`$o: String!, `$n: String!, `$after: String) { repository(owner: `$o, name: `$n) { pullRequests(states: OPEN, first: 25, after: `$after, orderBy: {field: CREATED_AT, direction: ASC}) { pageInfo { hasNextPage endCursor } nodes { $prFields } } } }"
$nodes = @()
$after = $null
do {
    $ghArgs = @('api', 'graphql', '-f', "query=$query", '-f', "o=$owner", '-f', "n=$name")
    if ($after) { $ghArgs += @('-f', "after=$after") }
    $page = (Invoke-Gh @ghArgs | Out-String | ConvertFrom-Json).data.repository.pullRequests
    $nodes += @($page.nodes)
    $after = $page.pageInfo.endCursor
} while ($page.pageInfo.hasNextPage)

# The issues the bodies name ("Refs #n"), which GitHub does not link: their
# title and labels, 50 per query. A number that is no issue gives nothing.
$closing = @{}
foreach ($node in $nodes) { foreach ($i in $node.closingIssuesReferences.nodes) { $closing[[int]$i.number] = $true } }
$wanted = @($nodes | ForEach-Object { Get-BodyIssueNumbers ([string]$_.body) } | Where-Object { -not $closing.ContainsKey($_) } | Sort-Object -Unique)
$issues = @{}
for ($i = 0; $i -lt $wanted.Count; $i += 50) {
    $chunk = $wanted[$i..([Math]::Min($i + 49, $wanted.Count - 1))]
    $fields = ($chunk | ForEach-Object { "i${_}: issueOrPullRequest(number: $_) { ... on Issue { number title labels(first: 20) { nodes { name } } } }" }) -join ' '
    # a number that is no issue makes gh exit 1 with the rest of the data:
    # read it anyway, under a local 'Continue' (Read-Pr says why)
    $ErrorActionPreference = 'Continue'
    $out = & gh api graphql -f "query=query(`$o: String!, `$n: String!) { repository(owner: `$o, name: `$n) { $fields } }" -f "o=$owner" -f "n=$name" 2>$null
    $ErrorActionPreference = 'Stop'
    if (-not $out) { Write-Warning 'Could not read the issues the PRs name: crash links from "Refs #n" are missing.'; continue }
    $data = ($out | Out-String | ConvertFrom-Json).data.repository
    if (-not $data) { continue }
    foreach ($p in $data.PSObject.Properties) {
        $v = $p.Value
        if ($v -and $v.number) { $issues[[int]$v.number] = [pscustomobject]@{ Title = [string]$v.title; Labels = @($v.labels.nodes | ForEach-Object { [string]$_.name }) } }
    }
}

# Days and hours a PR has waited, short.
function Format-Wait([datetime]$Since) {
    $t = (Get-Date).ToUniversalTime() - $Since.ToUniversalTime()
    if ($t.TotalDays -ge 1) { '{0} d' -f [int][Math]::Floor($t.TotalDays) } else { '{0} h' -f [int][Math]::Floor($t.TotalHours) }
}
function Format-Cut([string]$Text, [int]$Width) {
    if ($Text.Length -gt $Width) { $Text = $Text.Substring(0, $Width - 1) + '…' }
    $Text.PadRight($Width)
}

$items = @($nodes | ForEach-Object { ConvertTo-ReviewItem $_ $issues })
$entries = @(Get-ReviewQueue $items $me (Get-Date).ToUniversalTime())
$queue = @(Get-OrderedReviewQueue $entries)
$approvedList = @(Get-ApprovedToMerge $entries $me)
$ciText = @{ Fail = 'CI red'; Approval = 'CI waits for Approve and run'; Run = 'CI running'; Ok = 'CI green'; None = 'no CI yet' }
function Format-MergeState($Entry) {
    $ciText[[string]$Entry.Ci] + $(if ($Entry.Conflict) { ' · conflict with main' } else { '' }) +
        $(if (-not $Entry.OnHead) { ' · head not named in the review: re-review it first' } else { '' })
}

if ($ToMerge) {
    if ($Json) {
        $list = @($approvedList | ForEach-Object {
            [pscustomobject]@{ number = $_.Number; title = $_.Title; author = $_.Author; url = $_.Url
                approved_at = $_.Since.ToString('o'); head = [string]$_.Head; head_reviewed = [bool]$_.OnHead
                ci = [string]$_.Ci; conflict = [bool]$_.Conflict }
        })
        ConvertTo-Json -InputObject $list -Depth 3
        exit 0
    }
    Write-Host "Approved by $me, to merge ($($approvedList.Count)):" -ForegroundColor Cyan
    foreach ($e in $approvedList) { Write-Host (' {0,-6} {1}  ({2})' -f "#$($e.Number)", (Format-Cut $e.Title 60), (Format-MergeState $e)) }
    if (-not $approvedList.Count) { Write-Host ' Nothing.' }
    exit 0
}

if ($Json) {
    $list = @($queue | Select-Object -First $Top | ForEach-Object {
        [pscustomobject]@{ number = $_.Number; kind = $_.Kind; title = $_.Title; author = $_.Author; url = $_.Url
            claimed_by_you = $_.Mine; last_reviewer = $_.Reviewer; waiting_since = $_.Since.ToString('o'); notes = @($_.Notes) }
    })
    ConvertTo-Json -InputObject $list -Depth 4
    exit 0
}

Write-Host "Review queue of $me, ${repo}: $($queue.Count) to review out of $($entries.Count) open PRs." -ForegroundColor Cyan
Write-Host ''
if ($queue.Count) {
    Write-Host (' {0,-6} {1,-9} {2,6}  {3,-18} {4}' -f 'PR', 'KIND', 'WAITS', 'AUTHOR', 'TITLE · NOTES') -ForegroundColor DarkGray
    $color = @{ 1 = 'Magenta'; 2 = 'Red'; 3 = 'Yellow'; 4 = 'Gray' }
    foreach ($e in $queue | Select-Object -First $Top) {
        $notes = if ($e.Notes.Count) { '  · ' + ($e.Notes -join ' · ') } else { '' }
        Write-Host (' {0,-6} ' -f "#$($e.Number)") -NoNewline
        Write-Host ('{0,-9} ' -f $e.Kind) -ForegroundColor $color[$e.Tier] -NoNewline
        Write-Host ('{0,6}  {1} ' -f (Format-Wait $e.Since), (Format-Cut $e.Author 18)) -NoNewline
        Write-Host (Format-Cut $e.Title 60) -NoNewline
        Write-Host $notes -ForegroundColor DarkGray
    }
    if ($queue.Count -gt $Top) { Write-Host " … and $($queue.Count - $Top) more (-Top $($queue.Count))." -ForegroundColor DarkGray }
} else {
    Write-Host ' Nothing to review.'
}

# What is left out: the other maintainers' PRs by name (the overlap), the
# rest as counts.
$out = @($entries | Where-Object { $_.Tier -eq 0 })
if ($approvedList.Count) {
    Write-Host ''
    Write-Host "Approved by $me, to merge ($($approvedList.Count)): ./scripts/merge-pr.ps1 -Pr <n> once CI is green" -ForegroundColor Green
    foreach ($e in $approvedList) { Write-Host (' {0,-6} {1}  ({2})' -f "#$($e.Number)", (Format-Cut $e.Title 60), (Format-MergeState $e)) }
}
$theirs = @($out | Where-Object { $_.SkipKind -in 'Claimed', 'StaleClaim', 'Held' })
if ($theirs.Count) {
    Write-Host ''
    Write-Host "In other maintainers' hands ($($theirs.Count)):" -ForegroundColor DarkGray
    foreach ($e in $theirs | Sort-Object Owner, Number) {
        $c = if ($e.SkipKind -eq 'StaleClaim') { 'Yellow' } else { 'DarkGray' }
        Write-Host (' {0,-6} {1}  ({2})' -f "#$($e.Number)", (Format-Cut $e.Title 60), $e.Skip) -ForegroundColor $c
    }
}
Write-Host ''
$labels = [ordered]@{ WaitAuthor = 'waiting on their author'; CiRed = 'CI red'; Decision = 'waiting on a team decision'; Approved = 'approved, to merge'
    Label = 'on hold, waiting for a slot or a description'; Draft = 'drafts'; Queued = 'in the merge queue'; Own = 'yours'; Bot = 'bots' }
$counts = foreach ($k in $labels.Keys) {
    $c = @($out | Where-Object { $_.SkipKind -eq $k }).Count
    if ($c) { "$c $($labels[$k])" }
}
if ($counts) { Write-Host "Left out: $($counts -join ', ')." -ForegroundColor DarkGray }
if ($All) {
    foreach ($e in $out | Where-Object { $_.SkipKind -notin 'Claimed', 'StaleClaim', 'Held' } | Sort-Object SkipKind, Number) {
        Write-Host (' {0,-6} {1}  ({2})' -f "#$($e.Number)", (Format-Cut $e.Title 60), $e.Skip) -ForegroundColor DarkGray
    }
}
Write-Host ''
Write-Host 'Claim one before reading it: ./scripts/review-queue.ps1 -Claim <n>' -ForegroundColor DarkGray
