<#
.SYNOPSIS
    Triage of a crash report issue (docs/BUG_REPORTING.md, "Crash triage"),
    run by the Crash triage workflow: symbolizes its stack with the symbol
    file of its release, signs it, and closes it as a duplicate of the
    first open issue with the same signature.

.EXAMPLE
    ./scripts/crash-triage.ps1 -Issue 900
    (the workflow; GH_TOKEN with the issues permission)

.EXAMPLE
    ./scripts/crash-triage.ps1 -ReportFile public-report.md -SymbolFile aurora_viewer.sym
    Local check of a crash session's public-report.md: prints the section,
    the title and the signature, writes nothing to GitHub.
#>
param(
    [int]$Issue,
    [string]$ReportFile,
    [string]$SymbolFile
)
$ErrorActionPreference = 'Stop'
. "$PSScriptRoot/tools/crash-triage.ps1"

function Show-Triage($Report, $Frames, $Signature, $Section) {
    Write-Host "Signature: $($Signature.Id) ($(@($Signature.Frames) -join ' > '))"
    Write-Host "Title: $(Get-TriageTitle $Report $Signature)"
    Write-Host $Section
}

if ($ReportFile) {
    $report = Read-CrashReport ([IO.File]::ReadAllText($ReportFile))
    if (-not $report) { throw "$ReportFile is not a crash report." }
    $frames = Get-SymbolizedFrames $report $SymbolFile
    $signature = Get-CrashSignature $report $frames
    $name = $null
    if ($SymbolFile) { $name = Split-Path $SymbolFile -Leaf }
    Show-Triage $report $frames $signature (Format-TriageSection $frames $name $signature)
    exit 0
}
if (-not $Issue) { throw 'Give -Issue, or -ReportFile for a local check.' }

$repo = $env:GITHUB_REPOSITORY
if (-not $repo) { throw 'GITHUB_REPOSITORY is not set.' }
function Invoke-Gh {
    $output = & gh @args
    if ($LASTEXITCODE -ne 0) { throw "gh $($args -join ' ') failed." }
    $output
}

$data = Invoke-Gh api "repos/$repo/issues/$Issue" | Out-String | ConvertFrom-Json
if ($data.state -ne 'open') { Write-Host "Issue #$Issue is $($data.state): nothing to do."; exit 0 }
$report = Read-CrashReport $data.body
if (-not $report) { Write-Host "Issue #$Issue is not a crash report: nothing to do."; exit 0 }

# The symbol file of this build: published with its release, and with the
# debug id the report names (a rebuilt or modified exe has another one).
$symbolFile = $null
$symbolsName = $null
$tag = Get-ReleaseTag $report.Version
$own = $report.Modules[$OwnModule]
if ($tag -and $own -and $own.Id) {
    $work = Join-Path ([IO.Path]::GetTempPath()) "crash-triage-$Issue"
    if (Test-Path $work) { Remove-Item -Recurse -Force $work }
    New-Item -ItemType Directory $work | Out-Null
    & gh release download $tag -R $repo -p '*-windows-x64-symbols.zip' -D $work 2>&1 | Out-Host
    $zip = Get-ChildItem $work -Filter '*-windows-x64-symbols.zip' | Select-Object -First 1
    if ($zip) {
        Expand-Archive $zip.FullName (Join-Path $work 'symbols')
        $candidate = Join-Path $work "symbols/$OwnModule.pdb/$($own.Id)/$OwnModule.sym"
        if (Test-Path -LiteralPath $candidate) { $symbolFile = $candidate; $symbolsName = $zip.Name }
        else { Write-Host "The symbols of $tag do not have the debug id $($own.Id)." }
    }
    else { Write-Host "No symbols published for $tag." }
}

$frames = Get-SymbolizedFrames $report $symbolFile
$signature = Get-CrashSignature $report $frames
$section = Format-TriageSection $frames $symbolsName $signature
Show-Triage $report $frames $signature $section

$open = @(Invoke-Gh api "repos/$repo/issues?state=open&per_page=100" --paginate --jq '.[] | select(.pull_request == null) | {Number: .number, Body: .body} | tojson' |
    ForEach-Object { $_ | ConvertFrom-Json })
$duplicate = Find-DuplicateIssue $open $signature.Id $Issue

# Every report gets its symbolized stack, the duplicates too.
$patch = @{ body = (Merge-TriageSection $data.body $section) }
$title = Get-TriageTitle $report $signature
if ($title -and -not $duplicate) { $patch.title = $title }
$json = Join-Path ([IO.Path]::GetTempPath()) "crash-triage-$Issue.json"
[IO.File]::WriteAllText($json, ($patch | ConvertTo-Json -Compress), (New-Object Text.UTF8Encoding $false))
Invoke-Gh api -X PATCH "repos/$repo/issues/$Issue" --input $json | Out-Null

if ($duplicate) {
    $details = @("version $($report.Version)")
    if ($report.System) { $details += $report.System }
    if ($report.Gpu) { $details += $report.Gpu }
    Invoke-Gh issue comment $duplicate -R $repo --body "Another occurrence of this crash: #$Issue ($($details -join ', '))." | Out-Null
    Invoke-Gh issue comment $Issue -R $repo --body "Same crash as #$duplicate (signature ``$($signature.Id)``): closing as a duplicate. Thank you for the report, it was counted there." | Out-Null
    & gh issue close $Issue -R $repo --duplicate-of $duplicate 2>&1 | Out-Host
    # An older gh has no --duplicate-of: the comment above names the issue.
    if ($LASTEXITCODE -ne 0) { Invoke-Gh issue close $Issue -R $repo --reason 'not planned' | Out-Null }
    Write-Host "Issue #$Issue closed as a duplicate of #$duplicate."
}
else {
    # The type is an organisation setting: its absence must not fail the triage.
    & gh api -X PATCH "repos/$repo/issues/$Issue" -f type=Bug 2>&1 | Out-Null
    if ($LASTEXITCODE -ne 0) { Write-Host 'The Bug type could not be set.' }
    Write-Host "Issue #$Issue triaged as a new crash ($($signature.Id))."
}
