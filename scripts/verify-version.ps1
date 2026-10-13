<#
.SYNOPSIS
    Checks that a built viewer reports exactly the number of the release
    tag. --version exits before the logs, the window and the network.
#>
param(
    [Parameter(Mandatory)][string]$Exe,
    [Parameter(Mandatory)][string]$Tag
)
$ErrorActionPreference = 'Stop'
if ($Tag -notmatch '^v[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$') {
    throw "Invalid version tag: $Tag"
}
$path = (Resolve-Path -LiteralPath $Exe -ErrorAction Stop).Path
$reported = (& $path --version | Out-String).Trim()
if ($LASTEXITCODE -ne 0) { throw "The viewer could not report its version (code $LASTEXITCODE)." }
$expected = "Aurora Viewer $($Tag.Substring(1))"
if ($reported -cne $expected) { throw "Built version mismatch: expected '$expected', got '$reported'." }
Write-Host "Version verified: $reported"
