<#
.SYNOPSIS
    Sets up the signing of the updates, by an admin of the repository
    (docs/UPDATES.md, "Signed updates").
.DESCRIPTION
    1. Creates or updates the `release` environment of the repository: only
       the branch main and the stable version tags (v1.2.3, no pre-release
       part: the ones the tag ruleset reserves to the admins) may use it. A
       beta tag (v*-beta.*) is left out: anyone with write access can push
       one, from any commit.
    2. Makes a new Ed25519 key on this PC and stores it in the first empty
       secret of the environment, UPDATE_SIGNING_KEY or (while the key
       changes) UPDATE_SIGNING_KEY_NEXT, then deletes the file (or copies it
       to -Backup first). The publish jobs sign with every key that is set.
    3. Writes its public half into RELEASE_KEYS
       (crates/aurora-updater/src/signature.rs) of this checkout, for the
       pull request that brings it to the viewer.

    The private key is never printed. Run it from a task's worktree, with
    an admin's `gh` account. -EnvironmentOnly only does step 1 (the rules
    again, the keys kept).
.PARAMETER Backup
    A file to copy the private key to before it is deleted (an encrypted
    drive, a password manager's attachment...). Without it, the key only
    lives in the GitHub secret, which no one can read back.
#>
param(
    [string]$Repo = 'Aurora-Viewer/Aurora-Viewer',
    [string]$Backup,
    [switch]$EnvironmentOnly
)
$ErrorActionPreference = 'Stop'
$environment = 'release'
$slots = @('UPDATE_SIGNING_KEY', 'UPDATE_SIGNING_KEY_NEXT')

function Invoke-Gh {
    param([string[]]$Arguments, [string]$InputText)
    if ($InputText) { $out = $InputText | & gh @Arguments } else { $out = & gh @Arguments }
    if ($LASTEXITCODE -ne 0) { throw "gh $($Arguments -join ' ') failed." }
    $out
}

# 1. The environment and who may deploy to it.
$body = '{"deployment_branch_policy":{"protected_branches":false,"custom_branch_policies":true}}'
Invoke-Gh @('api', '-X', 'PUT', "repos/$Repo/environments/$environment", '--input', '-') $body | Out-Null
$wanted = @(@{ name = 'main'; type = 'branch' })
# vMAJOR.MINOR.PATCH with no pre-release part: GitHub's patterns have no
# "anything but a dash", so each length of number is listed (up to 99.99.999).
foreach ($major in @('[0-9]', '[0-9][0-9]')) {
    foreach ($minor in @('[0-9]', '[0-9][0-9]')) {
        foreach ($patch in @('[0-9]', '[0-9][0-9]', '[0-9][0-9][0-9]')) {
            $wanted += @{ name = "v$major.$minor.$patch"; type = 'tag' }
        }
    }
}
# tab-separated: no double quote in the jq filter (Windows PowerShell 5.1
# passes them to a program badly)
$existing = @(Invoke-Gh @('api', "repos/$Repo/environments/$environment/deployment-branch-policies", '--paginate', '--jq', '.branch_policies[] | [(.id | tostring), .type, .name] | @tsv') | Where-Object { $_ })
foreach ($line in $existing) {
    $id, $type, $name = $line -split "`t", 3
    if (-not ($wanted | Where-Object { $_.name -eq $name -and $_.type -eq $type })) {
        Invoke-Gh @('api', '-X', 'DELETE', "repos/$Repo/environments/$environment/deployment-branch-policies/$id") | Out-Null
        Write-Output "Removed the rule $type $name"
    }
}
foreach ($rule in $wanted) {
    # compared as text: `-like` would read the brackets as a pattern
    if (-not ($existing | Where-Object { $parts = $_ -split "`t", 3; $parts[1] -eq $rule.type -and $parts[2] -eq $rule.name })) {
        Invoke-Gh @('api', '-X', 'POST', "repos/$Repo/environments/$environment/deployment-branch-policies", '-f', "name=$($rule.name)", '-f', "type=$($rule.type)") | Out-Null
    }
}
Write-Output "Environment '$environment': branch main and the stable tags only."
if ($EnvironmentOnly) { return }

# 2. The key, in the first empty secret.
$secrets = @(Invoke-Gh @('secret', 'list', '--repo', $Repo, '--env', $environment, '--json', 'name', '--jq', '.[].name'))
$slot = $slots | Where-Object { $secrets -notcontains $_ } | Select-Object -First 1
if (-not $slot) {
    throw "Both $($slots -join ' and ') are set: retire the old key first (docs/UPDATES.md, `"Changing the key`")."
}
$openssl = (Get-Command openssl -ErrorAction SilentlyContinue).Source
if (-not $openssl) {
    $git = Split-Path (Split-Path (Get-Command git).Source)
    $openssl = @('mingw64\bin\openssl.exe', 'usr\bin\openssl.exe') | ForEach-Object { Join-Path $git $_ } | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
}
if (-not $openssl) { throw 'openssl not found (it comes with Git for Windows).' }
$work = Join-Path ([IO.Path]::GetTempPath()) "aurora-key-$([Guid]::NewGuid().ToString('N'))"
New-Item -ItemType Directory -Path $work | Out-Null
try {
    $key = Join-Path $work 'key.pem'
    & $openssl genpkey -algorithm ed25519 -out $key
    if ($LASTEXITCODE -ne 0) { throw 'openssl genpkey failed.' }
    & $openssl pkey -in $key -pubout -outform DER -out (Join-Path $work 'public.der')
    if ($LASTEXITCODE -ne 0) { throw 'openssl pkey failed.' }
    $bytes = [IO.File]::ReadAllBytes((Join-Path $work 'public.der'))
    $public = -join ($bytes[12..43] | ForEach-Object { $_.ToString('x2') })
    Invoke-Gh @('secret', 'set', $slot, '--repo', $Repo, '--env', $environment) ([IO.File]::ReadAllText($key)) | Out-Null
    if ($Backup) {
        Copy-Item -LiteralPath $key -Destination $Backup
        Write-Output "Private key copied to ${Backup}: keep it offline."
    }
} finally {
    Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue
}
Write-Output "Secret $slot set in '$environment'."

# 3. The public key, for the viewer.
$file = Join-Path (Split-Path -Parent $PSScriptRoot) 'crates/aurora-updater/src/signature.rs'
$source = [IO.File]::ReadAllText($file)
$newline = if ($source.Contains("`r`n")) { "`r`n" } else { "`n" }
# the secret's name on a line of its own: rustfmt would align a comment at
# the end of the key's line
$entry = '${1}// ' + $slot + ', ' + (Get-Date -Format 'yyyy-MM-dd') + $newline + '${1}"' + $public + '",' + $newline + '${1}// release-keys-end'
$source = [regex]::Replace($source, '(?m)^( *)// release-keys-end', $entry)
[IO.File]::WriteAllText($file, $source, (New-Object Text.UTF8Encoding $false))
Write-Output "Public key $public added to RELEASE_KEYS ($file)."
