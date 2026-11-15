<#
.SYNOPSIS
    Builds the latest main of GitHub (or a PR) in --release, next to the
    repository: ..\RELEASE\Aurora-Viewer\aurora-viewer.exe, ready to launch.

.DESCRIPTION
    The code built always comes from GitHub, never from the local
    repository: neither uncommitted changes nor unpushed commits go into it.
    ..\RELEASE\src is a reserved worktree, reset each time on the requested
    commit; its target\ is kept from one build to the next (only what
    changed is rebuilt). version.txt gives the version reported by the
    viewer and the commit built; the git tag is authoritative.
    The first run creates ..\RELEASE\build-release.bat (double-click).

.EXAMPLE
    ./scripts/build-release.ps1          # latest main of GitHub
    ./scripts/build-release.ps1 -Pr 42   # PR 42
#>
param([int]$Pr)
. "$PSScriptRoot\common.ps1"

$p = Get-AuroraPaths
$rel = Join-Path $p.Root 'RELEASE'
$src = Join-Path $rel 'src'
$out = Join-Path $rel 'Aurora-Viewer'
New-Item -ItemType Directory -Force -Path $rel -ErrorAction Stop | Out-Null

# Double-click launcher (build-release.bat 42 builds PR 42). It runs the
# script of the last build, so it is always main's latest version; when that
# build lacks it (a PR branched before it existed), it first puts src back on
# GitHub's main, and only then falls back to the repository's copy, which
# it also runs when src is no longer a Git worktree (recreated below).
# Only written once: cmd reads a .bat from disk while running it.
# On Linux the same launcher is build-release.sh.
$bat = Join-Path $rel $(if ($IsLinux) { 'build-release.sh' } else { 'build-release.bat' })
if ($IsLinux -and -not (Test-Path -LiteralPath $bat)) {
    @(
        '#!/bin/sh'
        'self=$(readlink -f -- "$0")'
        '# double-clicked: open a terminal, as Windows opens a console for a .bat'
        'if [ ! -t 1 ] && [ -z "$AURORA_TOOLS_IN_TERMINAL" ] && { [ -n "$WAYLAND_DISPLAY" ] || [ -n "$DISPLAY" ]; }; then'
        '    export AURORA_TOOLS_IN_TERMINAL=1'
        '    for t in ${TERMINAL:+"$TERMINAL -e"} xdg-terminal-exec "konsole -e" "gnome-terminal --" "kgx --" "ptyxis --" kitty foot "alacritty -e" "wezterm start --" "ghostty -e" "xterm -e"; do'
        '        command -v "${t%% *}" >/dev/null 2>&1 && exec $t "$self" "$@"'
        '    done'
        'fi'
        'd=$(dirname -- "$self")'
        's=$d/src/scripts/build-release.ps1'
        '[ -f "$s" ] || { git -C "$d/src" fetch origin main --quiet && git -C "$d/src" checkout --detach --force --quiet origin/main; }'
        'git -C "$d/src" rev-parse --git-dir >/dev/null 2>&1 || s='
        "[ -f `"`$s`" ] || s='$((Join-Path $p.Main 'scripts/build-release.ps1') -replace "'", "'\''")'"
        'pwsh -NoProfile -File "$s" "$@"'
        'printf "Press Enter to close. "; read -r _'
    ) -join "`n" | Set-Content -LiteralPath $bat -NoNewline -ErrorAction Stop
    Add-Content -LiteralPath $bat -Value '' -ErrorAction Stop
    chmod +x $bat
}
if (-not $IsLinux -and -not (Test-Path -LiteralPath $bat)) {
    @(
        '@echo off'
        'set "S=%~dp0src\scripts\build-release.ps1"'
        'if not exist "%S%" (git -C "%~dp0src" fetch origin main --quiet && git -C "%~dp0src" checkout --detach --force --quiet origin/main)'
        'git -C "%~dp0src" rev-parse --git-dir >nul 2>nul || set "S="'
        "if not exist `"%S%`" set `"S=$(Join-Path $p.Main 'scripts\build-release.ps1')`""
        'powershell -NoProfile -ExecutionPolicy Bypass -File "%S%" %*'
        'pause'
    ) | Set-Content -LiteralPath $bat -Encoding ASCII -ErrorAction Stop
}

if (Test-InUse $out) { throw "The viewer in $out is open: close it, then run again." }

# the commit to build, straight from GitHub
if ($Pr) {
    Invoke-Checked 'git fetch' { git -C $p.Main fetch origin "pull/$Pr/head" --tags --quiet }
    $sha = (git -C $p.Main rev-parse FETCH_HEAD).Trim()
    $what = "PR #$Pr"
} else {
    Invoke-Checked 'git fetch' { git -C $p.Main fetch origin main --tags --quiet }
    $sha = (git -C $p.Main rev-parse origin/main).Trim()
    $what = 'main'
}

# A src\ that is no longer a worktree of this repository (repository
# recloned or moved, worktree pruned: its .git points to metadata Git no
# longer has) is recreated; its target\ is set aside, then put back.
$kept = Join-Path $rel 'target.kept'
if (Test-Path -LiteralPath $src) {
    $common = $null
    if (Test-Path -LiteralPath (Join-Path $src '.git')) {
        $common = git -C $src rev-parse --path-format=absolute --git-common-dir 2>$null
        if ($LASTEXITCODE -ne 0) { $common = $null }
    }
    if (-not $common -or [IO.Path]::GetFullPath($common.Trim()) -ne [IO.Path]::GetFullPath((Join-Path $p.Main '.git'))) {
        Write-Host "$(Join-Path 'RELEASE' 'src') is no longer a worktree of this repository: recreating it (its build cache is kept)." -ForegroundColor Yellow
        $srcTarget = Join-Path $src 'target'
        if ((Test-Path -LiteralPath $srcTarget) -and -not (Test-Path -LiteralPath $kept)) {
            Move-Item -LiteralPath $srcTarget -Destination $kept -ErrorAction Stop
        }
        Remove-Item -LiteralPath $src -Recurse -Force -ErrorAction Stop
    }
}

# reset the reserved worktree on it; target\ is ignored by git, so kept
if (Test-Path -LiteralPath $src) {
    Invoke-Checked 'git checkout' { git -C $src checkout --detach --force --quiet $sha }
    Invoke-Checked 'git clean' { git -C $src clean -fd --quiet }
} else {
    # forgets a src\ deleted by hand, whose registration would block the add
    Invoke-Checked 'git worktree prune' { git -C $p.Main worktree prune }
    Invoke-Checked 'git worktree add' { git -C $p.Main worktree add --detach $src $sha }
    if (Test-Path -LiteralPath $kept) { Move-Item -LiteralPath $kept -Destination (Join-Path $src 'target') -ErrorAction Stop }
}

Write-Host "Building $what ($($sha.Substring(0, 9))) in --release…" -ForegroundColor Cyan
$sw = [Diagnostics.Stopwatch]::StartNew()
Push-Location $src
# A shell inherited from a release job must not stamp another commit with
# its tag. The local build resolves the version from this checkout's Git.
$previousVersionTag = [Environment]::GetEnvironmentVariable('AURORA_VERSION_TAG', 'Process')
try {
    # Not SetEnvironmentVariable(…, $null): PowerShell passes '', which .NET 9 and
    # later keep as an empty variable, and the build refuses an empty tag.
    Remove-Item Env:AURORA_VERSION_TAG -ErrorAction SilentlyContinue
    Invoke-Checked 'cargo build' { cargo build --release --locked -p aurora-viewer }
} finally {
    if ($null -ne $previousVersionTag) { [Environment]::SetEnvironmentVariable('AURORA_VERSION_TAG', $previousVersionTag, 'Process') }
    Pop-Location
}

# Use the built commit's verifier when available, just like package.ps1.
# Older revisions do not understand --version: invoking it would open a
# viewer, so keep their existing packaging path without querying the exe.
$exeName = if ($IsLinux) { 'aurora-viewer' } else { 'aurora-viewer.exe' }
$builtExe = Join-Path $src "target\release\$exeName"
$reportedVersion = $null
$verifyVersion = Join-Path $src 'scripts\verify-version.ps1'
if (Test-Path -LiteralPath $verifyVersion) {
    $reportedVersion = (& $builtExe --version | Out-String).Trim()
    if ($LASTEXITCODE -ne 0 -or $reportedVersion -notmatch '^Aurora Viewer (\S+)$') {
        throw "The built viewer did not report a valid version: '$reportedVersion'."
    }
    $exactTag = git -C $src describe --tags --match 'v[0-9]*.[0-9]*.[0-9]*' --exact-match 2>$null
    if ($LASTEXITCODE -eq 0) { & $verifyVersion -Exe $builtExe -Tag $exactTag.Trim() }
}

# package.ps1 of the built commit when it has one (older PR branches do not)
$package = Join-Path $src 'scripts\package.ps1'
if (-not (Test-Path -LiteralPath $package)) { $package = Join-Path $PSScriptRoot 'package.ps1' }
if (Test-Path -LiteralPath $out) { Remove-Item -LiteralPath $out -Recurse -Force -ErrorAction Stop }
& $package -Exe $builtExe -Out $out -Source $src
@(
    if ($reportedVersion) { $reportedVersion }
    "${what}: $((git -C $src log -1 --format='%h %s (%ci)').Trim())"
    "built on $(Get-Date -Format 'yyyy-MM-dd HH:mm')"
) | Set-Content -LiteralPath (Join-Path $out 'version.txt') -Encoding UTF8 -ErrorAction Stop

Write-Host ""
Write-Host "Ready in $([math]::Floor($sw.Elapsed.TotalMinutes)) min $($sw.Elapsed.Seconds) s: $(Join-Path $out $exeName)" -ForegroundColor Green
Write-Host "Double-click $bat to rebuild ($(Split-Path $bat -Leaf) 42: PR 42)."
