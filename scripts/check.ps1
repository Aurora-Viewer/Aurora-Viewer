<#
.SYNOPSIS
    The CI checks, locally: formatting, clippy (warnings = errors) and
    tests. Run it before each PR.
#>
. "$PSScriptRoot\common.ps1"

Set-Location (git rev-parse --show-toplevel).Trim()
$steps = @(
    @{ Name = 'Formatting (rustfmt)'; Run = { cargo fmt --all -- --check } },
    @{ Name = 'Clippy'; Run = { cargo clippy --workspace --all-targets --locked -- -D warnings } },
    @{ Name = 'Tests'; Run = { cargo test --workspace --locked } }
)
foreach ($s in $steps) {
    Write-Host "==> $($s.Name)" -ForegroundColor Cyan
    & $s.Run
    if ($LASTEXITCODE -ne 0) {
        Write-Host "FAILED: $($s.Name)" -ForegroundColor Red
        exit 1
    }
}
Write-Host "All green." -ForegroundColor Green
