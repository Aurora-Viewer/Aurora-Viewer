<#
.SYNOPSIS
    Publishes screenshots for a PR on the `captures` branch (never on main)
    and prints the Markdown to paste into the PR or the review.

.EXAMPLE
    ./scripts/pr-captures.ps1 -Pr 42 -Files .scratch\before.png, .scratch\after.png
#>
param(
    [Parameter(Mandatory)][int]$Pr,
    [Parameter(Mandatory)][string[]]$Files
)
. "$PSScriptRoot\common.ps1"

$p = Get-AuroraPaths
$nwo = (gh repo view --json nameWithOwner --jq .nameWithOwner).Trim()
$files = $Files | ForEach-Object { (Resolve-Path $_ -ErrorAction Stop).Path }
$tmp = Join-Path $p.Work ".captures-$Pr-$PID"
$orphan = "captures-tmp-$PID"

git -C $p.Main fetch origin captures --quiet 2>$null
$exists = $LASTEXITCODE -eq 0
try {
    if ($exists) {
        Invoke-Checked 'worktree' { git -C $p.Main worktree add --detach $tmp origin/captures }
    } else {
        # first captures ever: an orphan branch with no code in it
        Invoke-Checked 'worktree' { git -C $p.Main worktree add --detach $tmp }
        Invoke-Checked 'orphan' { git -C $tmp checkout --quiet --orphan $orphan }
        Invoke-Checked 'clean' { git -C $tmp rm -r --quiet --force . }
    }
    $sub = "pr-$Pr"
    New-Item -ItemType Directory -Force -Path (Join-Path $tmp $sub) | Out-Null
    $names = @()
    foreach ($f in $files) {
        $name = ([IO.Path]::GetFileName($f).ToLowerInvariant() -replace '[^a-z0-9._-]+', '-')
        Copy-Item $f (Join-Path $tmp "$sub\$name") -Force -ErrorAction Stop
        $names += $name
    }
    Invoke-Checked 'add' { git -C $tmp add $sub }
    Invoke-Checked 'commit' { git -C $tmp commit --quiet -m "Add the screenshots of PR #$Pr" }
    git -C $tmp push --quiet origin HEAD:captures
    if ($LASTEXITCODE -ne 0) {
        # another agent pushed captures meanwhile: replay ours on top
        Invoke-Checked 'pull' { git -C $tmp pull --quiet --rebase origin captures }
        Invoke-Checked 'push' { git -C $tmp push --quiet origin HEAD:captures }
    }
    Write-Host ""
    Write-Host "Markdown to paste:" -ForegroundColor Green
    foreach ($n in $names) {
        Write-Host "![$n](https://raw.githubusercontent.com/$nwo/captures/$sub/$n)"
    }
} finally {
    git -C $p.Main worktree remove --force $tmp 2>$null
    git -C $p.Main branch -D $orphan 2>$null | Out-Null
    git -C $p.Main worktree prune
}
