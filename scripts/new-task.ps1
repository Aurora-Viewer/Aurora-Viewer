<#
.SYNOPSIS
    Crée le dossier de travail d'une tâche : un worktree git sur sa propre
    branche, à côté du dépôt (..\work\<tâche>), avec son propre target\.

.EXAMPLE
    ./scripts/new-task.ps1 -Name "regard des avatars" -Kind feat
    # -> ..\work\regard-des-avatars, branche feat/regard-des-avatars
#>
param(
    [Parameter(Mandatory)][string]$Name,
    [ValidateSet('feat', 'fix', 'perf', 'refactor', 'docs', 'chore')][string]$Kind = 'feat'
)
. "$PSScriptRoot\common.ps1"

$p = Get-AuroraPaths
$slug = Get-TaskSlug $Name
$branch = "$Kind/$slug"
$dir = Join-Path $p.Work $slug
if (Test-Path $dir) { throw "Le dossier $dir existe déjà (tâche en cours ?)." }

# start from the latest main
$base = 'main'
git -C $p.Main remote get-url origin *> $null
if ($LASTEXITCODE -eq 0) {
    Invoke-Checked 'git fetch' { git -C $p.Main fetch origin main --quiet }
    $base = 'origin/main'
}
New-Item -ItemType Directory -Force -Path $p.Work -ErrorAction Stop | Out-Null
Invoke-Checked 'git worktree add' { git -C $p.Main worktree add -b $branch $dir $base }

# the emoji font is not in git: hard link it from the main repository
$font = Join-Path $p.Main 'assets\emoji\Noto-3D-128.ttf'
if (Test-Path $font) {
    try {
        New-Item -ItemType HardLink -Path (Join-Path $dir 'assets\emoji\Noto-3D-128.ttf') -Target $font -ErrorAction Stop | Out-Null
    } catch {
        Write-Warning "Police emoji non liée ($($_.Exception.Message)) : lance scripts/fetch-assets.ps1 dans le worktree."
    }
}
New-Item -ItemType Directory -Force -Path (Join-Path $dir '.scratch') | Out-Null

Write-Host ""
Write-Host "Tâche prête :" -ForegroundColor Green
Write-Host "  dossier  $dir"
Write-Host "  branche  $branch (depuis $base)"
Write-Host ""
Write-Host "Tester :  `$env:AURORA_DEMO='1'; cargo run --release -p aurora-viewer -- --title `"Test $slug`""
Write-Host "Vérifier: ./scripts/check.ps1"
Write-Host "Finir   : ./scripts/end-task.ps1 -Name $slug (après la fusion de la PR)"
