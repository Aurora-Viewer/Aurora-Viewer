<#
.SYNOPSIS
    Vérifie qu'un viewer compilé annonce exactement le numéro du tag de
    release. --version quitte avant les journaux, la fenêtre et le réseau.
#>
param(
    [Parameter(Mandatory)][string]$Exe,
    [Parameter(Mandatory)][string]$Tag
)
$ErrorActionPreference = 'Stop'
if ($Tag -notmatch '^v[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$') {
    throw "Tag de version invalide : $Tag"
}
$path = (Resolve-Path -LiteralPath $Exe -ErrorAction Stop).Path
$reported = (& $path --version | Out-String).Trim()
if ($LASTEXITCODE -ne 0) { throw "Le viewer n'a pas pu annoncer sa version (code $LASTEXITCODE)." }
$expected = "Aurora Viewer $($Tag.Substring(1))"
if ($reported -cne $expected) { throw "Version compilée incohérente : attendu '$expected', reçu '$reported'." }
Write-Host "Version vérifiée : $reported"
