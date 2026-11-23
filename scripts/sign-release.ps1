<#
.SYNOPSIS
    Signs release files for the updater: writes <file>.sig, the 64 raw bytes
    of an Ed25519 signature over the whole file per signing key
    (docs/UPDATES.md, "Signed updates").
.DESCRIPTION
    The publish jobs of the Release workflow sign each package's .sha256
    file and the update policy with the keys of the `release` environment:
    UPDATE_SIGNING_KEY and, while the key changes, UPDATE_SIGNING_KEY_NEXT
    (one signature after the other in the same .sig file). Each key must be
    one the viewer is built with (RELEASE_KEYS in
    crates/aurora-updater/src/signature.rs): one that is not stops the run
    before anything is published, instead of a release no viewer accepts.

    The update tests (test-update.ps1) sign their own releases with a key
    made for the run: -NewTestKey makes it and returns its public key, then
    -KeyFile and -PublicKey sign with it.
.PARAMETER Files
    The files to sign.
.PARAMETER KeyFile
    PEM Ed25519 private keys. Without it: the UPDATE_SIGNING_KEY and
    UPDATE_SIGNING_KEY_NEXT variables, those that are set.
.PARAMETER PublicKey
    The public keys (hex) the signing keys must be. Without it: the
    RELEASE_KEYS of signature.rs.
.PARAMETER NewTestKey
    Makes a new key in this file, for a test, and returns its public key
    (hex). Nothing is signed.
#>
param(
    [string[]]$Files,
    [string[]]$KeyFile,
    [string[]]$PublicKey,
    [string]$NewTestKey
)
$ErrorActionPreference = 'Stop'

function Get-OpenSsl {
    $command = Get-Command openssl -ErrorAction SilentlyContinue
    if ($command) { return $command.Source }
    # Windows: the one of Git for Windows
    $git = Get-Command git -ErrorAction SilentlyContinue
    if ($git) {
        $root = Split-Path (Split-Path $git.Source)
        foreach ($candidate in @('mingw64\bin\openssl.exe', 'usr\bin\openssl.exe')) {
            $path = Join-Path $root $candidate
            if (Test-Path -LiteralPath $path) { return $path }
        }
    }
    throw 'openssl not found (it comes with Git for Windows).'
}

function Invoke-OpenSsl {
    param([string[]]$Arguments)
    & $script:openssl @Arguments
    if ($LASTEXITCODE -ne 0) { throw "openssl $($Arguments[0]) failed." }
}

# The public half of a private key, in hex (the last 32 bytes of its DER form).
function Get-PublicKey {
    param([string]$Key, [string]$Der)
    Invoke-OpenSsl @('pkey', '-in', $Key, '-pubout', '-outform', 'DER', '-out', $Der)
    $bytes = [IO.File]::ReadAllBytes((Resolve-Path -LiteralPath $Der).Path)
    Remove-Item -LiteralPath $Der
    if ($bytes.Length -ne 44) { throw 'The signing key is not an Ed25519 key.' }
    -join ($bytes[12..43] | ForEach-Object { $_.ToString('x2') })
}

$openssl = Get-OpenSsl
if ($NewTestKey) {
    Invoke-OpenSsl @('genpkey', '-algorithm', 'ed25519', '-out', $NewTestKey)
    return Get-PublicKey $NewTestKey "$NewTestKey.der"
}
if (-not $Files) { throw 'No file to sign.' }
$repo = Split-Path -Parent $PSScriptRoot
if ($PublicKey) {
    $trusted = @($PublicKey | ForEach-Object { $_.Trim().ToLowerInvariant() })
} else {
    $source = [IO.File]::ReadAllText((Join-Path $repo 'crates/aurora-updater/src/signature.rs'))
    $block = [regex]::Match($source, '(?s)release-keys-begin(.*?)release-keys-end').Groups[1].Value
    $trusted = @([regex]::Matches($block, '"([0-9a-fA-F]{64})"') | ForEach-Object { $_.Groups[1].Value.ToLowerInvariant() })
}
if ($trusted.Count -eq 0) { throw 'No public key to check the signing keys against.' }

$work = Join-Path ([IO.Path]::GetTempPath()) "aurora-sign-$([Guid]::NewGuid().ToString('N'))"
New-Item -ItemType Directory -Path $work | Out-Null
try {
    $keys = @($KeyFile | Where-Object { $_ })
    if ($keys.Count -eq 0) {
        foreach ($name in @('UPDATE_SIGNING_KEY', 'UPDATE_SIGNING_KEY_NEXT')) {
            $value = [Environment]::GetEnvironmentVariable($name)
            if (-not $value) { continue }
            $path = Join-Path $work "$name.pem"
            # only this account reads it, for the few seconds it lives
            New-Item -ItemType File -Path $path | Out-Null
            if (Get-Command chmod -ErrorAction SilentlyContinue) { & chmod 600 $path }
            [IO.File]::WriteAllText($path, ($value.Trim() + "`n"))
            $keys += $path
        }
        if ($keys.Count -eq 0) { throw 'UPDATE_SIGNING_KEY is empty: the release environment has no signing key (docs/UPDATES.md, "Signed updates").' }
    }
    # each key must be one the viewer knows
    $i = 0
    $signers = @()
    foreach ($key in $keys) {
        $i++
        $public = Get-PublicKey $key (Join-Path $work "public-$i.der")
        if ($trusted -notcontains $public) { throw "A signing key (public $public) is none of the viewer's release keys." }
        $pem = Join-Path $work "public-$i.pem"
        Invoke-OpenSsl @('pkey', '-in', $key, '-pubout', '-out', $pem)
        $signers += @{ key = $key; pem = $pem }
    }
    foreach ($file in $Files) {
        $path = (Resolve-Path -LiteralPath $file).Path
        $all = New-Object System.Collections.Generic.List[byte]
        foreach ($signer in $signers) {
            $one = Join-Path $work 'one.sig'
            Invoke-OpenSsl @('pkeyutl', '-sign', '-rawin', '-inkey', $signer.key, '-in', $path, '-out', $one)
            Invoke-OpenSsl @('pkeyutl', '-verify', '-rawin', '-pubin', '-inkey', $signer.pem, '-in', $path, '-sigfile', $one) | Out-Null
            $bytes = [IO.File]::ReadAllBytes($one)
            if ($bytes.Length -ne 64) { throw "Unexpected signature size for $file." }
            $all.AddRange($bytes)
            Remove-Item -LiteralPath $one
        }
        [IO.File]::WriteAllBytes("$path.sig", $all.ToArray())
        Write-Output "Signed $(Split-Path -Leaf $path) ($($signers.Count) key(s))"
    }
} finally {
    Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue
}
