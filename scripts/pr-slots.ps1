<#
.SYNOPSIS
    Review slots (HUMANS.md "Review slots"), run by the Review slots
    workflow: an outside contributor has at most 10 pull requests in review;
    the others get the waiting-for-slot label and a comment, and come back
    on their own, oldest first, when a slot frees up. The team's PRs never
    wait. Rules in scripts/tools/pr-slots.ps1.

.EXAMPLE
    ./scripts/pr-slots.ps1 -Sweep -DryRun
    What the sweep would do for every outside contributor: writes nothing
    (GITHUB_REPOSITORY, or the origin remote).

.EXAMPLE
    ./scripts/pr-slots.ps1 -Author someone -Pr 2900 -Preferred 2900
    (the workflow, on a PR event: GH_TOKEN with the issues and pull-requests
    permissions; -Preferred when a maintainer has just removed its label)
#>
param(
    [string]$Author,
    [int]$Pr,
    [int]$Preferred,
    [switch]$Sweep,
    [switch]$DryRun
)
$ErrorActionPreference = 'Stop'
. "$PSScriptRoot/tools/pr-slots.ps1"
if (-not $Author -and -not $Sweep) { throw 'Give -Author (and -Pr), or -Sweep.' }

$repo = $env:GITHUB_REPOSITORY
if (-not $repo) {
    $repo = (gh repo view (git -C $PSScriptRoot remote get-url origin).Trim() --json nameWithOwner --jq .nameWithOwner)
    if (-not $repo) { throw 'GITHUB_REPOSITORY is not set and the origin remote is unknown.' }
}
$owner, $name = $repo -split '/', 2
function Invoke-Gh {
    $output = & gh @args
    if ($LASTEXITCODE -ne 0) { throw "gh $($args -join ' ') failed." }
    $output
}
# A body goes to the API through a file: never through a command line.
function Send-Comment([int]$Number, [string]$Body) {
    $file = [IO.Path]::GetTempFileName()
    try {
        [IO.File]::WriteAllText($file, $Body)
        Invoke-Gh api -X POST "repos/$repo/issues/$Number/comments" -F "body=@$file" | Out-Null
    } finally {
        Remove-Item $file -ErrorAction SilentlyContinue
    }
}

# The fields of a pull request the plan needs. No double quote in the
# queries: PowerShell 5.1 drops the inner quotes of a native argument.
$fields = 'number isDraft createdAt authorAssociation author { login } labels(first: 30) { nodes { name } } mergeQueueEntry { id } autoMergeRequest { enabledAt } comments(last: 50) { nodes { databaseId authorAssociation author { login } body } }'
function ConvertTo-SlotItem($node) {
    $labels = @($node.labels.nodes | ForEach-Object { $_.name })
    $reviewed = [bool]@($node.comments.nodes | Where-Object {
        $_.author.login -ne 'github-actions' -and (Test-PrSlotTeam $_.authorAssociation) -and
        $_.body -match '^\s*## (Review|Relecture|Maintainer review|Changes requested)'
    }).Count
    [pscustomobject]@{
        Number = [int]$node.number; Draft = [bool]$node.isDraft
        Waiting = $labels -contains $PrSlotLabel; OnHold = $labels -contains 'on-hold'
        Protected = [bool]$node.mergeQueueEntry -or [bool]$node.autoMergeRequest
        Reviewed = $reviewed; Created = [string]$node.createdAt
        Association = [string]$node.authorAssociation
        Comments = @($node.comments.nodes | Where-Object { $_.author.login -eq 'github-actions' -and $_.body.Contains($PrSlotMarker) -and $_.body.TrimStart().StartsWith('## Waiting') })
    }
}
function Get-AuthorPrs([string]$Login, [int]$Also) {
    $query = "query(`$q: String!, `$after: String) { search(query: `$q, type: ISSUE, first: 50, after: `$after) { pageInfo { hasNextPage endCursor } nodes { ... on PullRequest { $fields } } } }"
    $items = @()
    $after = $null
    do {
        $ghArgs = @('api', 'graphql', '-f', "query=$query", '-f', "q=repo:$repo is:pr is:open author:$Login")
        if ($after) { $ghArgs += @('-f', "after=$after") }
        $data = (Invoke-Gh @ghArgs | Out-String | ConvertFrom-Json).data.search
        $items += @($data.nodes | Where-Object { $_.number } | ForEach-Object { ConvertTo-SlotItem $_ })
        $after = $data.pageInfo.endCursor
    } while ($data.pageInfo.hasNextPage)
    # the search index lags a few seconds behind a PR just opened
    if ($Also -and -not ($items | Where-Object { $_.Number -eq $Also })) {
        $one = (Invoke-Gh api graphql -f "query=query(`$o: String!, `$n: String!, `$p: Int!) { repository(owner: `$o, name: `$n) { pullRequest(number: `$p) { state $fields } } }" -f "o=$owner" -f "n=$name" -F "p=$Also" | Out-String | ConvertFrom-Json).data.repository.pullRequest
        if ($one -and $one.state -eq 'OPEN' -and $one.author.login -eq $Login) { $items += ConvertTo-SlotItem $one }
    }
    $items
}
function Set-SlotLabel([int]$Number, [bool]$On) {
    if ($On) {
        & gh api "repos/$repo/labels/$PrSlotLabel" --silent 2>$null
        if ($LASTEXITCODE -ne 0) {
            Invoke-Gh api "repos/$repo/labels" -f "name=$PrSlotLabel" -f 'color=94A3B8' -f 'description=Outside contributor over 10 PRs in review: draft until a review slot frees up' | Out-Null
        }
        Invoke-Gh api -X POST "repos/$repo/issues/$Number/labels" -f "labels[]=$PrSlotLabel" | Out-Null
    } else {
        & gh api -X DELETE "repos/$repo/issues/$Number/labels/$PrSlotLabel" --silent 2>$null
    }
}
function Remove-WaitComments($Item) {
    foreach ($c in $Item.Comments) { Invoke-Gh api -X DELETE "repos/$repo/issues/comments/$($c.databaseId)" | Out-Null }
}

function Update-Author([string]$Login, [int]$Event, [int]$Prefer) {
    $items = @(Get-AuthorPrs $Login $Event)
    if (-not $items.Count) { Write-Host "${Login}: no open pull request."; return }
    if (Test-PrSlotTeam $items[0].Association) { Write-Host "${Login}: team ($($items[0].Association)), no limit."; return }
    $preferredArg = $null
    if ($Prefer) { $preferredArg = $Prefer }
    $plan = Get-PrSlotPlan $items $PrSlotLimit $preferredArg
    $open = @($items | Where-Object { -not $_.OnHold -and -not $_.Draft }).Count
    $inReview = @($items | Where-Object { -not $_.OnHold -and -not $_.Draft -and -not $_.Waiting }).Count
    Write-Host ("{0}: {1} open, {2} in review; to wait: {3}; back to review: {4}; label to drop: {5}" -f $Login, $open, $inReview,
        (($plan.Wait | ForEach-Object { "#$_" }) -join ' '), (($plan.Back | ForEach-Object { "#$_" }) -join ' '), (($plan.Unlabel | ForEach-Object { "#$_" }) -join ' '))
    if ($DryRun) { return }
    $links = Get-PrSlotLinks $repo $Login
    foreach ($n in $plan.Wait) {
        $item = $items | Where-Object { $_.Number -eq $n }
        Set-SlotLabel $n $true
        if (-not $item.Comments.Count) { Send-Comment $n (Format-PrSlotWait $Login $open $links) }
    }
    foreach ($n in $plan.Back) {
        Set-SlotLabel $n $false
        Remove-WaitComments ($items | Where-Object { $_.Number -eq $n })
        Send-Comment $n (Format-PrSlotBack $Login)
    }
    foreach ($n in $plan.Unlabel) {
        Set-SlotLabel $n $false
        Remove-WaitComments ($items | Where-Object { $_.Number -eq $n })
    }
}

if ($Author) { Update-Author $Author $Pr $Preferred; exit 0 }

# The sweep: every outside author with an open PR (one line each: login,
# association, account type).
$rows = @(Invoke-Gh api "repos/$repo/pulls?state=open&per_page=100" --paginate --jq '.[] | [.user.login, .author_association, .user.type] | @tsv')
$authors = @($rows | Where-Object { $_ } | ForEach-Object { , ($_ -split "`t") } |
    Where-Object { $_[2] -ne 'Bot' -and -not (Test-PrSlotTeam $_[1]) } | ForEach-Object { $_[0] } | Sort-Object -Unique)
Write-Host "$($authors.Count) outside author(s) with open pull requests."
$failed = 0
foreach ($a in $authors) {
    # one author that fails (API hiccup) does not stop the others
    try { Update-Author $a 0 0 } catch { $failed++; Write-Warning "${a}: $($_.Exception.Message)" }
}
if ($failed) { exit 1 }
