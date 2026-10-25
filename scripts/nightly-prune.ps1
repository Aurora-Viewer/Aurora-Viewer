<#
.SYNOPSIS
    Run by the Nightly workflow after a beta is published: deletes the beta
    releases (and their tags) beyond the newest -Keep, so the release list
    the viewer and the website read stays short. Stable releases and other
    pre-releases are never touched.

.EXAMPLE
    ./scripts/nightly-prune.ps1 -Keep 10 -DryRun
#>
param([int]$Keep = 10, [switch]$DryRun)
$ErrorActionPreference = 'Stop'
. "$PSScriptRoot/tools/nightly.ps1"

$json = gh release list --limit 200 --json tagName,isPrerelease
if ($LASTEXITCODE -ne 0) { throw 'gh release list failed.' }
$tags = @(($json | Out-String | ConvertFrom-Json) | Where-Object { $_.isPrerelease } | ForEach-Object { $_.tagName })
foreach ($tag in Get-BetasToPrune $tags $Keep) {
    Write-Host "Deleting the old beta $tag"
    if ($DryRun) { continue }
    gh release delete $tag --cleanup-tag --yes
    if ($LASTEXITCODE -ne 0) { throw "gh release delete $tag failed." }
}
