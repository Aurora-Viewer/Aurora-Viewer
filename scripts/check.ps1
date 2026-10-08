<#
.SYNOPSIS
    Les vérifications de la CI, en local : formatage, clippy (avertissements
    = erreurs) et tests. À lancer avant chaque PR.
#>
. "$PSScriptRoot\common.ps1"

Set-Location (git rev-parse --show-toplevel).Trim()
$steps = @(
    @{ Name = 'Formatage (rustfmt)'; Run = { cargo fmt --all -- --check } },
    @{ Name = 'Clippy'; Run = { cargo clippy --release --workspace --all-targets --locked -- -D warnings } },
    @{ Name = 'Tests'; Run = { cargo test --release --workspace --locked } }
)
foreach ($s in $steps) {
    Write-Host "==> $($s.Name)" -ForegroundColor Cyan
    & $s.Run
    if ($LASTEXITCODE -ne 0) {
        Write-Host "ÉCHEC : $($s.Name)" -ForegroundColor Red
        exit 1
    }
}
Write-Host "Tout est vert." -ForegroundColor Green
