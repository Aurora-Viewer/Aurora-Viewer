<#
.SYNOPSIS
    The CI checks, locally: syntax of the PowerShell scripts, formatting,
    clippy (warnings = errors) and tests. Run it before each PR.

    Clippy and the tests wait for a build slot when other heavy builds
    already run on the PC (AURORA_BUILD_SLOTS, 2 by default; common.ps1).
#>
. "$PSScriptRoot\common.ps1"

Set-Location (git rev-parse --show-toplevel).Trim()
Sync-BuildCache
# Off Windows, clippy allows dead code, as the Linux and macOS CI jobs do:
# Windows-only helpers have no caller in those builds. Windows PowerShell
# (and check.ps1 there) still rejects it. $IsWindows is unset on 5.1.
$clippyArgs = @('-D', 'warnings')
if ($PSVersionTable.PSEdition -ne 'Desktop' -and -not $IsWindows) { $clippyArgs += @('-A', 'dead-code') }
$steps = @(
    @{ Name = 'PowerShell scripts (5.1 parser)'; Run = { & "$PSScriptRoot\check-scripts.ps1" } },
    @{ Name = 'Formatting (rustfmt)'; Run = { cargo fmt --all -- --check } },
    @{ Name = 'Clippy'; Heavy = $true; Run = { cargo clippy --workspace --all-targets --locked -- @clippyArgs } },
    @{ Name = 'Tests'; Heavy = $true; Run = { cargo test --workspace --locked } }
)
$slot = $null
try {
    foreach ($s in $steps) {
        # A step that throws under PowerShell 7 (a missing command under a
        # child script's 'Stop') only ends this foreach. The script would
        # then fall through and print "All green." (#1659): catch it and
        # fail like a non-zero exit code. Reset the code first, so a stale
        # one cannot fail the next step.
        $global:LASTEXITCODE = 0
        $failed = $false
        try {
            if ($s.Heavy -and -not $slot) { $slot = Enter-BuildSlot }
            Write-Host "==> $($s.Name)" -ForegroundColor Cyan
            & $s.Run
            if ($LASTEXITCODE -ne 0) { $failed = $true }
        } catch {
            $detail = $_.Exception.Message
            if ($_.InvocationInfo.ScriptName) {
                $detail = "{0}:{1}: {2}" -f $_.InvocationInfo.ScriptName, $_.InvocationInfo.ScriptLineNumber, $detail
            }
            Write-Host $detail -ForegroundColor Red
            $failed = $true
        }
        if ($failed) {
            Write-Host "FAILED: $($s.Name)" -ForegroundColor Red
            exit 1
        }
    }
} finally {
    Exit-BuildSlot $slot
}
Write-Host "All green." -ForegroundColor Green
