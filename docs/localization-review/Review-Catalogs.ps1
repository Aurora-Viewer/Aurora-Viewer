[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$PreviousEnglish,
    [Parameter(Mandatory)][string]$CatalogDirectory,
    [Parameter(Mandatory)][string]$ReportDirectory,
    [string]$CurrentEnglish
)

$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$previous = (Resolve-Path -LiteralPath $PreviousEnglish).Path
$current = if ($CurrentEnglish) {
    (Resolve-Path -LiteralPath $CurrentEnglish).Path
} else {
    Join-Path $repo 'crates/aurora-i18n/locales/en.ftl'
}
$catalogs = @(Get-ChildItem -LiteralPath $CatalogDirectory -Filter '*.ftl' -File |
    Where-Object { $_.BaseName -ne 'en' } | Sort-Object Name)
if ($catalogs.Count -eq 0) {
    throw 'No translated Fluent files were found.'
}
$reports = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($ReportDirectory)

$originalEncoding = [Console]::OutputEncoding
try {
    [Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)
    Push-Location $repo
    try {
        $build = & cargo build --quiet --locked -p aurora-i18n --example catalog_delta --message-format=json
        if ($LASTEXITCODE -ne 0) { throw "Building catalog_delta failed with exit $LASTEXITCODE." }
    } finally {
        Pop-Location
    }
    $artifact = $build | ForEach-Object { $_ | ConvertFrom-Json } |
        Where-Object { $_.reason -eq 'compiler-artifact' -and $_.target.name -eq 'catalog_delta' -and $_.executable } |
        Select-Object -Last 1
    if (!$artifact) { throw 'Cargo did not report a catalog_delta executable.' }

    $null = New-Item -ItemType Directory -Path $reports -Force
    foreach ($catalog in $catalogs) {
        $report = Join-Path $reports ($catalog.BaseName + '.txt')
        $worklist = & $artifact.executable $previous $current $catalog.FullName
        if ($LASTEXITCODE -ne 0) { throw "Checking $($catalog.Name) failed with exit $LASTEXITCODE." }
        Set-Content -LiteralPath $report -Encoding UTF8 -Value $worklist
        Write-Output $report
    }
} finally {
    [Console]::OutputEncoding = $originalEncoding
}
