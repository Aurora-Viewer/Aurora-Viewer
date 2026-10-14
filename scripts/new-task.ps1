<#
.SYNOPSIS
    Creates the work folder of a task: a git worktree on its own branch,
    next to the repository (..\work\<task>), with its own target\. Its
    first build takes the dependencies from the shared compiler cache when
    sccache is installed (common.ps1).

    On its way, frees the target\ of the other tasks that no longer need
    it (PR merged, or no build for -IdleDays days): the code stays, only
    the build goes; a task that resumes rebuilds.

.EXAMPLE
    ./scripts/new-task.ps1 -Name "avatar gaze" -Kind feat
    # -> ..\work\avatar-gaze, branch feat/avatar-gaze
#>
param(
    [Parameter(Mandatory)][string]$Name,
    [ValidateSet('feat', 'fix', 'perf', 'refactor', 'docs', 'chore')][string]$Kind = 'feat',
    [int]$IdleDays = 7
)
. "$PSScriptRoot\common.ps1"

$p = Get-AuroraPaths
$slug = Get-TaskSlug $Name
$branch = "$Kind/$slug"
$dir = Join-Path $p.Work $slug
if (Test-Path $dir) { throw "The folder $dir already exists (task in progress?)." }

# Free the build folders of the other tasks that no longer need them. The
# last build is when Cargo last wrote into a deps\ or incremental\ folder.
if (Test-Path -LiteralPath $p.Work) {
    foreach ($t in Get-ChildItem -LiteralPath $p.Work -Directory -Force) {
        $target = Join-Path $t.FullName 'target'
        if (-not (Test-Path -LiteralPath (Join-Path $target 'CACHEDIR.TAG'))) { continue }
        $why = $null
        $b = git -C $t.FullName rev-parse --abbrev-ref HEAD 2>$null
        if ($LASTEXITCODE -eq 0 -and (gh pr view $b.Trim() --json state --jq .state 2>$null) -eq 'MERGED') {
            $why = 'PR merged'
        } else {
            $last = Get-ChildItem -LiteralPath $target -Directory -Force | ForEach-Object {
                Get-Item -LiteralPath (Join-Path $_.FullName 'deps'), (Join-Path $_.FullName 'incremental') -Force -ErrorAction SilentlyContinue
            } | Sort-Object LastWriteTime -Descending | Select-Object -First 1
            $idle = if ($last) { ((Get-Date) - $last.LastWriteTime).Days } else { [int]::MaxValue }
            if ($idle -ge $IdleDays) { $why = "no build for $idle days" }
        }
        if (-not $why) { continue }
        if (Test-InUse $t.FullName) { Write-Warning "$($t.Name): target kept (a build or a viewer is running)."; continue }
        Remove-Item -LiteralPath $target -Recurse -Force -ErrorAction SilentlyContinue
        if (Test-Path -LiteralPath $target) {
            Write-Warning "$($t.Name): target not fully deleted (locked file?)."
        } else {
            Write-Host "$($t.Name): target freed ($why)." -ForegroundColor DarkGray
        }
    }
}

# start from the latest main
$base = 'main'
git -C $p.Main remote get-url origin *> $null
if ($LASTEXITCODE -eq 0) {
    Invoke-Checked 'git fetch' { git -C $p.Main fetch origin main --quiet }
    $base = 'origin/main'
}
New-Item -ItemType Directory -Force -Path $p.Work -ErrorAction Stop | Out-Null
Invoke-Checked 'git worktree add' { git -C $p.Main worktree add -b $branch $dir $base }
New-Item -ItemType Directory -Force -Path (Join-Path $dir '.scratch') | Out-Null
# the first build of the task takes the dependencies from the shared cache
Sync-BuildCache

Write-Host ""
Write-Host "Task ready:" -ForegroundColor Green
Write-Host "  folder  $dir"
Write-Host "  branch  $branch (from $base)"
Write-Host ""
Write-Host "Test:   `$env:AURORA_DEMO='1'; cargo run -p aurora-viewer -- --title `"Test $slug`""
Write-Host "Check:  ./scripts/check.ps1"
Write-Host "Finish: ./scripts/end-task.ps1 -Name $slug (after the PR is merged)"
