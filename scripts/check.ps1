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
$steps = @(
    @{ Name = 'PowerShell scripts (5.1 parser)'; Run = { & "$PSScriptRoot\check-scripts.ps1" } },
    @{ Name = 'Formatting (rustfmt)'; Run = { cargo fmt --all -- --check } },
    @{ Name = 'Clippy'; Heavy = $true; Run = { cargo clippy --workspace --all-targets --locked -- -D warnings } },
    @{ Name = 'Tests'; Heavy = $true; Run = { cargo test --workspace --locked } }
)
$slot = $null
try {
    foreach ($s in $steps) {
        if ($s.Heavy -and -not $slot) { $slot = Enter-BuildSlot }
        Write-Host "==> $($s.Name)" -ForegroundColor Cyan
        & $s.Run
        if ($LASTEXITCODE -ne 0) {
            Write-Host "FAILED: $($s.Name)" -ForegroundColor Red
            exit 1
        }
    }
} finally {
    Exit-BuildSlot $slot
}
Write-Host "All green." -ForegroundColor Green
