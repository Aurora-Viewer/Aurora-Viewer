<#
.SYNOPSIS
    PR description check (HUMANS.md "PR description check"), run by the PR
    description workflow: a pull request whose description misses a part of
    .github/pull_request_template.md gets one comment opening with
    "## Changes requested" and the needs-description label; both go away
    once the description is complete. An outside contributor's PR still
    incomplete 3 days after that comment is closed by the daily sweep; the
    team's PRs never are.

.EXAMPLE
    ./scripts/check-pr-description.ps1 -BodyFile .scratch/pr.md
    Local check of a draft description before `gh pr create`: lists what it
    misses, writes nothing to GitHub (exit code 1 when incomplete).

.EXAMPLE
    ./scripts/check-pr-description.ps1 -Pr 2892
    ./scripts/check-pr-description.ps1 -Sweep
    (the workflow; GH_TOKEN with the issues and pull-requests permissions)
#>
param(
    [int]$Pr,
    [switch]$Sweep,
    [string]$BodyFile
)
$ErrorActionPreference = 'Stop'
. "$PSScriptRoot/tools/pr-description.ps1"

if ($BodyFile) {
    $problems = @(Get-PrDescriptionProblems ([IO.File]::ReadAllText($BodyFile)))
    if (-not $problems.Count) { Write-Host 'The description is complete.'; exit 0 }
    Write-Host 'The description misses:'
    $problems | ForEach-Object { Write-Host "- $_" }
    exit 1
}
if (-not $Pr -and -not $Sweep) { throw 'Give -Pr, -Sweep, or -BodyFile for a local check.' }

$repo = $env:GITHUB_REPOSITORY
if (-not $repo) { throw 'GITHUB_REPOSITORY is not set.' }
$templateUrl = "https://github.com/$repo/blob/main/.github/pull_request_template.md"
$bot = 'github-actions[bot]'
function Invoke-Gh {
    $output = & gh @args
    if ($LASTEXITCODE -ne 0) { throw "gh $($args -join ' ') failed." }
    $output
}
# A body goes to the API through a file: never through a command line.
function Send-Body([string]$Method, [string]$Path, [string]$Body) {
    $file = [IO.Path]::GetTempFileName()
    try {
        [IO.File]::WriteAllText($file, $Body)
        Invoke-Gh api -X $Method $Path -F "body=@$file" | Out-Null
    } finally {
        Remove-Item $file -ErrorAction SilentlyContinue
    }
}
function Get-CheckComment([int]$Number) {
    # one JSON object per line (@json), whatever the pages; no quote in the
    # filter (PowerShell 5.1 drops the inner quotes of a native argument)
    $lines = @(Invoke-Gh api "repos/$repo/issues/$Number/comments" --paginate --jq '.[] | {id, created_at, body, login: .user.login} | @json')
    $lines | Where-Object { $_ } | ForEach-Object { $_ | ConvertFrom-Json } |
        Where-Object { $_.login -eq $bot -and $_.body.Contains($PrDescriptionMarker) -and $_.body.TrimStart().StartsWith('## Changes requested') } |
        Select-Object -First 1
}
function Set-Label([int]$Number, [bool]$On, $Labels) {
    $has = [bool]@($Labels | Where-Object { $_.name -eq $PrDescriptionLabel }).Count
    if ($On -and -not $has) {
        & gh api "repos/$repo/labels/$PrDescriptionLabel" --silent 2>$null
        if ($LASTEXITCODE -ne 0) {
            Invoke-Gh api "repos/$repo/labels" -f "name=$PrDescriptionLabel" -f 'color=94A3B8' -f 'description=PR description incomplete: not reviewed until it follows the template' | Out-Null
        }
        Invoke-Gh api -X POST "repos/$repo/issues/$Number/labels" -f "labels[]=$PrDescriptionLabel" | Out-Null
    } elseif (-not $On -and $has) {
        Invoke-Gh api -X DELETE "repos/$repo/issues/$Number/labels/$PrDescriptionLabel" | Out-Null
    }
}

function Update-Pr([int]$Number, [bool]$MayClose) {
    $data = Invoke-Gh api "repos/$repo/pulls/$Number" | Out-String | ConvertFrom-Json
    if ($data.state -ne 'open') { Write-Host "#${Number}: $($data.state), nothing to do."; return }
    if ($data.user.type -eq 'Bot') { Write-Host "#${Number}: opened by $($data.user.login), not checked."; return }
    if ($data.draft) { Write-Host "#${Number}: draft, checked once it is ready for review."; return }
    $problems = @(Get-PrDescriptionProblems ([string]$data.body))
    $comment = Get-CheckComment $Number
    if (-not $problems.Count) {
        if ($comment) { Invoke-Gh api -X DELETE "repos/$repo/issues/comments/$($comment.id)" | Out-Null }
        Set-Label $Number $false $data.labels
        Write-Host "#${Number}: the description is complete."
        return
    }
    $team = Test-PrTeamAuthor $data.author_association
    # PowerShell 7's ConvertFrom-Json already turns the ISO date into a DateTime
    $asked = [datetime]::UtcNow
    if ($comment) {
        $asked = $comment.created_at
        if ($asked -isnot [datetime]) {
            $asked = [datetime]::Parse([string]$asked, [Globalization.CultureInfo]::InvariantCulture, [Globalization.DateTimeStyles]'AssumeUniversal, AdjustToUniversal')
        }
        $asked = $asked.ToUniversalTime()
    }
    $deadline = $asked.AddDays($PrDescriptionGraceDays)
    $deadlineText = $null
    if (-not $team) { $deadlineText = $deadline.ToString('yyyy-MM-dd') }
    $body = Format-PrDescriptionComment $problems $templateUrl $deadlineText $data.user.login
    if (-not $comment) {
        Send-Body POST "repos/$repo/issues/$Number/comments" $body
    } elseif ($comment.body -ne $body) {
        Send-Body PATCH "repos/$repo/issues/comments/$($comment.id)" $body
    }
    Set-Label $Number $true $data.labels
    Write-Host "#${Number} ($($data.user.login), $($data.author_association)): the description misses $($problems.Count) part(s)."
    $problems | ForEach-Object { Write-Host "  - $_" }
    if ($MayClose -and -not $team -and $comment -and [datetime]::UtcNow -gt $deadline) {
        Send-Body POST "repos/$repo/issues/$Number/comments" (Format-PrDescriptionClose $templateUrl $data.user.login)
        Invoke-Gh api -X PATCH "repos/$repo/pulls/$Number" -f 'state=closed' | Out-Null
        Write-Host "#${Number}: closed, incomplete since $($asked.ToString('yyyy-MM-dd'))."
    }
}

if ($Pr) { Update-Pr $Pr $false; exit 0 }

# The sweep: the open PRs that carry the label (one per line).
$numbers = @(Invoke-Gh api "repos/$repo/issues?state=open&labels=$PrDescriptionLabel&per_page=100" --paginate --jq '.[] | select(.pull_request) | .number')
Write-Host "$($numbers.Count) open pull request(s) with $PrDescriptionLabel."
$failed = 0
foreach ($n in ($numbers | Where-Object { $_ })) {
    # one PR that fails (deleted meanwhile, API hiccup) does not stop the others
    try { Update-Pr ([int]$n) $true } catch { $failed++; Write-Warning "#${n}: $($_.Exception.Message)" }
}
if ($failed) { exit 1 }
