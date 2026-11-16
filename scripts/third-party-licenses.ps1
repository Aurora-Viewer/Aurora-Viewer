<#
.SYNOPSIS
    Writes THIRD-PARTY-LICENSES.md: the Rust crates built into the viewer,
    their licences and the notices their authors ask to keep.

.DESCRIPTION
    Runs cargo-about with packaging/licenses/about.toml (the accepted
    licences, the published systems) and the template
    packaging/licenses/about.hbs. The viewer shows the file in Help > About,
    Libraries tab, so a published build carries every notice.

    Run it whenever Cargo.lock changes (a dependency added, removed or
    updated) and commit the file with the change. A test of check.ps1 fails
    when the file names a crate version that Cargo.lock no longer has.

    cargo-about reads the licence files of the crates and, for the ones
    packaged without them, their repositories (network). It is installed
    once with: cargo install cargo-about --locked --features cli

.EXAMPLE
    ./scripts/third-party-licenses.ps1
#>
$ErrorActionPreference = 'Stop'

$repo = Split-Path $PSScriptRoot -Parent
$config = Join-Path $repo 'packaging\licenses\about.toml'
$template = Join-Path $repo 'packaging\licenses\about.hbs'
$out = Join-Path $repo 'THIRD-PARTY-LICENSES.md'

if (-not (Get-Command cargo-about -ErrorAction SilentlyContinue)) {
    throw 'cargo-about is not installed: cargo install cargo-about --locked --features cli'
}

$tmp = [IO.Path]::GetTempFileName()
try {
    Push-Location $repo
    try {
        & cargo about generate --fail -c $config $template -o $tmp
        if ($LASTEXITCODE -ne 0) { throw "cargo about failed ($LASTEXITCODE)" }
    } finally {
        Pop-Location
    }
    # LF line endings, no trailing blank lines, UTF-8 without BOM.
    $utf8 = New-Object Text.UTF8Encoding $false
    $text = [IO.File]::ReadAllText($tmp, $utf8) -replace "`r`n", "`n"
    $text = $text.TrimEnd() + "`n"

    # cargo-about splits a licence file into the licences it recognises and
    # drops the rest. These crates' files carry more (the notices of the
    # projects they come from), so they are added whole.
    $whole = @(
        @{ Crate = 'aws-lc-sys'; File = 'LICENSE' }
    )
    $cargoHome = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $HOME '.cargo' }
    $registry = Join-Path $cargoHome 'registry\src'
    $text += "`n## Full licence files`n"
    foreach ($w in $whole) {
        $m = [regex]::Match($text, '(?m)^\| ' + [regex]::Escape($w.Crate) + ' (\S+) \|')
        if (-not $m.Success) { throw "$($w.Crate) is no longer in the list: update the script" }
        $version = $m.Groups[1].Value
        $file = Get-ChildItem $registry -Directory |
            ForEach-Object { Join-Path $_.FullName "$($w.Crate)-$version\$($w.File)" } |
            Where-Object { Test-Path $_ } | Select-Object -First 1
        if (-not $file) { throw "$($w.Crate) $version not found under $registry (cargo fetch)" }
        $body = ([IO.File]::ReadAllText($file, $utf8) -replace "`r`n", "`n").TrimEnd()
        $text += "`n### $($w.Crate) $version, $($w.File)`n`n" + '````text' + "`n$body`n" + '````' + "`n"
    }
    [IO.File]::WriteAllText($out, $text, $utf8)
} finally {
    Remove-Item $tmp -ErrorAction SilentlyContinue
}
$crates = ([regex]::Matches($text, '(?m)^\| \S+ \S+ \|')).Count
Write-Host "THIRD-PARTY-LICENSES.md: $crates crates"
