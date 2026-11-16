# Shared helpers for the scripts (dot-sourced: . "$PSScriptRoot\common.ps1").

# Native tools (cargo, git, gh) write progress to stderr: Windows PowerShell 5.1
# turns it into errors when the output is redirected. They are judged on their
# exit code instead (Invoke-Checked / $LASTEXITCODE); cmdlets that must not
# fail silently use -ErrorAction Stop.
$ErrorActionPreference = 'Continue'

# Main repository (also when called from a worktree) and its parent folder,
# which holds work\ and phoenix-firestorm\. Found from the scripts' own
# folder, so a script works whatever the current folder (double-click…).
function Get-AuroraPaths {
    $common = (git -C $PSScriptRoot rev-parse --path-format=absolute --git-common-dir).Trim()
    if ($LASTEXITCODE -ne 0 -or -not $common) { throw "Not in an Aurora Viewer git repository." }
    $main = Split-Path $common -Parent
    $root = Split-Path $main -Parent
    [pscustomobject]@{ Main = $main; Root = $root; Work = (Join-Path $root 'work') }
}

# Task name -> folder / branch name: lowercase letters, digits and dashes.
function Get-TaskSlug([string]$Name) {
    $slug = ($Name.ToLowerInvariant() -replace '[^a-z0-9]+', '-').Trim('-')
    if (-not $slug) { throw "Invalid task name: '$Name'." }
    $slug
}

# A build or a viewer is running from $Dir: compiler command lines name
# their output folder, the viewer runs from its own exe.
function Test-InUse([string]$Dir) {
    $full = [IO.Path]::GetFullPath($Dir).TrimEnd('\') + '\'
    $procs = Get-CimInstance Win32_Process -ErrorAction SilentlyContinue -Filter (
        "Name='cargo.exe' OR Name='rustc.exe' OR Name='clippy-driver.exe' OR Name='aurora-viewer.exe'")
    [bool]($procs | Where-Object {
        ($_.ExecutablePath -and $_.ExecutablePath.StartsWith($full, [StringComparison]::OrdinalIgnoreCase)) -or
        ($_.CommandLine -and $_.CommandLine.IndexOf($full, [StringComparison]::OrdinalIgnoreCase) -ge 0)
    })
}

# The incremental\ folders of a Cargo target directory (one per profile:
# debug, ci, debugging…), when it holds the CACHEDIR.TAG Cargo creates. They
# are three quarters of a task's target\ and only speed up its next small
# rebuild; the compiled dependencies are in deps\ next to them.
function Get-IncrementalDirs([string]$Target) {
    if (-not (Test-Path -LiteralPath (Join-Path $Target 'CACHEDIR.TAG'))) { return }
    Get-ChildItem -LiteralPath $Target -Directory -Force -ErrorAction SilentlyContinue |
        ForEach-Object { Join-Path $_.FullName 'incremental' } |
        Where-Object { Test-Path -LiteralPath $_ -PathType Container }
}

# --- Shared Cargo configuration: build gate and compiler cache -----------------
# <root>\.cargo\config.toml covers the main repository and every task of
# work\, whatever the command (check, demo, tests). The scripts write it with
# what they find:
#
# - the build gate (crates/aurora-build-gate): Cargo runs every compiler
#   through it, and a heavy crate waits there for a build slot. A cargo
#   command typed by hand cannot go around it, unlike the slot of check.ps1
#   (below). It is built here with rustc alone and kept in <root>\.cargo\.
# - sccache, when it is installed (the Aurora tools install it): every
#   worktree builds the same dependencies, and their compiled crates are kept
#   in one folder next to the repository (<root>\.sccache), where a new
#   worktree takes them. The gate hands the compiler over to it.
#
# With neither, the file is deleted: Cargo builds as before.

# The marker changed when the gate came: a script from before it does not
# know the file as its own and leaves it alone, instead of writing the gate
# out of it at each run of an older worktree.
$BuildCacheMarker = '# Written by the Aurora Viewer scripts (Sync-BuildCache, build gate).'
$BuildCacheMarkerBeforeGate = '# Written by the Aurora Viewer scripts (Sync-BuildCache).'

function Find-Sccache {
    $cmd = Get-Command sccache -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($cmd) { return $cmd.Source }
    # installed by winget after this window was opened: not in its PATH yet
    Get-ChildItem "$env:LOCALAPPDATA\Microsoft\WinGet\Packages\Mozilla.sccache_*\*\sccache.exe" -ErrorAction SilentlyContinue |
        Sort-Object LastWriteTime -Descending | Select-Object -First 1 -ExpandProperty FullName
}

# The build gate of this checkout: <root>\.cargo\aurora-build-gate-<hash>,
# built at its first use. The name follows the source: a changed gate is a
# new file, and a build started with the old one keeps it. Returns nothing
# when it cannot be built (no rustc, compile error): the builds then run
# without a gate.
function Get-BuildGate([string]$Root) {
    $repo = Split-Path $PSScriptRoot -Parent
    $source = Join-Path $repo 'crates/aurora-build-gate/src/main.rs'
    if (-not (Test-Path -LiteralPath $source)) { return }
    $rustc = Get-Command rustc -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
    if (-not $rustc) { return }
    # line endings aside: git may check the file out with CRLF
    $text = [IO.File]::ReadAllText($source) -replace "`r`n", "`n"
    $sha = [Security.Cryptography.SHA256]::Create()
    try {
        $hash = -join ($sha.ComputeHash([Text.Encoding]::UTF8.GetBytes($text))[0..5] | ForEach-Object { $_.ToString('x2') })
    } finally {
        $sha.Dispose()
    }
    # $IsWindows is unset on Windows PowerShell 5.1; that edition is Windows.
    $ext = ''
    if (($PSVersionTable.PSEdition -eq 'Desktop') -or $IsWindows) { $ext = '.exe' }
    $dir = Join-Path $Root '.cargo'
    $gate = Join-Path $dir "aurora-build-gate-$hash$ext"
    if (-not (Test-Path -LiteralPath $gate)) {
        # built apart: on Windows rustc leaves a .pdb next to the exe
        $build = Join-Path ([IO.Path]::GetTempPath()) "aurora-build-gate-$PID"
        New-Item -ItemType Directory -Force -Path $build, $dir | Out-Null
        try {
            $built = Join-Path $build "gate$ext"
            # from the repository: rustup takes the toolchain of rust-toolchain.toml
            Push-Location -LiteralPath $repo
            try {
                & $rustc.Source --edition 2024 -C opt-level=2 -C debuginfo=0 -o $built $source
            } finally {
                Pop-Location
            }
            if ($LASTEXITCODE -ne 0) {
                Write-Host "Build gate not built (rustc, code $LASTEXITCODE): the builds run without it." -ForegroundColor Yellow
                # not the result of the calling script
                $global:LASTEXITCODE = 0
                return
            }
            # another script may have built it in the meantime: keep that one
            Move-Item -LiteralPath $built -Destination $gate -ErrorAction SilentlyContinue
        } finally {
            Remove-Item -LiteralPath $build -Recurse -Force -ErrorAction SilentlyContinue
        }
        if (-not (Test-Path -LiteralPath $gate)) { return }
    }
    # A gate's date says when a script last chose it. One that none has
    # chosen for a week is no longer the gate of a running build: delete it.
    $now = Get-Date
    $item = Get-Item -LiteralPath $gate
    if ($item.LastWriteTime -lt $now.AddDays(-1)) { try { $item.LastWriteTime = $now } catch { } }
    Get-ChildItem -LiteralPath $dir -File -Filter 'aurora-build-gate-*' -ErrorAction SilentlyContinue |
        Where-Object { $_.FullName -ne $item.FullName -and $_.LastWriteTime -lt $now.AddDays(-7) } |
        Remove-Item -Force -ErrorAction SilentlyContinue
    $gate
}

# Writes or deletes <root>\.cargo\config.toml to match the build gate and the
# installed sccache. A file there that these scripts did not write is left
# alone. Size of the cache: AURORA_BUILD_CACHE_SIZE (sccache format, 10G by
# default).
function Sync-BuildCache([string]$Root) {
    if (-not $Root) { $Root = (Get-AuroraPaths).Root }
    $file = Join-Path $Root '.cargo\config.toml'
    if ((Test-Path -LiteralPath $file) -and
        -not (Select-String -LiteralPath $file -Pattern $BuildCacheMarker, $BuildCacheMarkerBeforeGate -SimpleMatch -Quiet)) { return }
    $sccache = Find-Sccache
    $gate = Get-BuildGate $Root
    if (-not $sccache -and -not $gate) {
        if (Test-Path -LiteralPath $file) { Remove-Item -LiteralPath $file -Force -ErrorAction SilentlyContinue }
        return
    }
    $wrapper = $sccache
    if ($gate) { $wrapper = $gate }
    # TOML literal strings ('…'): the backslashes of the paths stay as they are
    $lines = @(
        $BuildCacheMarker,
        '# Deleted by them when neither the build gate nor sccache is there; see scripts/common.ps1.',
        '[build]',
        "rustc-wrapper = '$wrapper'"
    )
    if ($sccache) {
        $size = if ($env:AURORA_BUILD_CACHE_SIZE) { $env:AURORA_BUILD_CACHE_SIZE } else { '10G' }
        $lines += '', '[env]'
        if ($gate) { $lines += '# the build gate hands the compiler over to sccache', "AURORA_GATE_NEXT = '$sccache'" }
        $lines += @(
            "SCCACHE_DIR = '$(Join-Path $Root '.sccache')'",
            "SCCACHE_CACHE_SIZE = '$size'",
            '# cache server unreachable: compile without it',
            "SCCACHE_IGNORE_SERVER_IO_ERROR = '1'"
        )
    }
    $text = ($lines + '') -join "`n"
    if ((Test-Path -LiteralPath $file) -and ([IO.File]::ReadAllText($file) -eq $text)) { return }
    New-Item -ItemType Directory -Force -Path (Split-Path $file) | Out-Null
    [IO.File]::WriteAllText($file, $text, (New-Object Text.UTF8Encoding $false))
}

# --- One heavy build at a time --------------------------------------------------
# Several agents running check.ps1 at once each take every core, and all of
# them end up slower than one after the other. A heavy build takes one of
# AURORA_BUILD_SLOTS slots (2 by default) and waits for a free one. A slot is
# a named mutex: the OS frees it if the script dies.
#
# These slots hold a whole run of check.ps1 (clippy, then the tests). The
# build gate (above) has slots of its own for each compile of the viewer
# crate, whatever the command that started it.
#
# Windows waits on every slot at once. PowerShell 7 on Linux and macOS cannot:
# WaitAny of several named mutexes throws PlatformNotSupportedException.
# Each slot is waited on its own there (WaitOne), with the same names.

function Enter-BuildSlot {
    $count = 2
    if ($env:AURORA_BUILD_SLOTS -match '^\d+$' -and [int]$env:AURORA_BUILD_SLOTS -ge 1) { $count = [int]$env:AURORA_BUILD_SLOTS }
    $slots = [Threading.Mutex[]](1..$count | ForEach-Object { New-Object Threading.Mutex($false, "Local\AuroraViewerBuildSlot$_") })
    # $IsWindows is unset on Windows PowerShell 5.1; that edition is Windows.
    $waitAny = ($PSVersionTable.PSEdition -eq 'Desktop') -or $IsWindows
    $waited = [Diagnostics.Stopwatch]::StartNew()
    $taken = -1
    $timeout = 0
    while ($taken -lt 0) {
        try {
            if ($waitAny) {
                # short waits: Ctrl+C still stops the script between two
                $i = [Threading.WaitHandle]::WaitAny($slots, $timeout)
                if ($i -ne [Threading.WaitHandle]::WaitTimeout) { $taken = $i }
            } else {
                $slice = 0
                if ($timeout -gt 0) { $slice = [Math]::Max(1, [int][Math]::Floor($timeout / $slots.Count)) }
                for ($i = 0; $i -lt $slots.Count; $i++) {
                    try {
                        if ($slots[$i].WaitOne($slice)) { $taken = $i; break }
                    } catch {
                        # this slot's holder died: it is now ours
                        $e = $_.Exception
                        while ($e -and -not ($e -is [Threading.AbandonedMutexException])) { $e = $e.InnerException }
                        if (-not $e) { throw }
                        $taken = $i
                        break
                    }
                }
            }
        } catch {
            # a slot whose holder died during WaitAny: it is now ours
            $e = $_.Exception
            while ($e -and -not ($e -is [Threading.AbandonedMutexException])) { $e = $e.InnerException }
            if (-not $e) { throw }
            $taken = $e.MutexIndex
        }
        if ($taken -lt 0 -and $timeout -eq 0) {
            Write-Host "Waiting for a build slot: $count heavy build(s) already running on this PC (AURORA_BUILD_SLOTS)…" -ForegroundColor Yellow
            $timeout = 1000
        }
    }
    if ($waited.Elapsed.TotalSeconds -ge 1) { Write-Host ("Build slot free after {0:N0} s." -f $waited.Elapsed.TotalSeconds) -ForegroundColor DarkGray }
    for ($k = 0; $k -lt $slots.Count; $k++) { if ($k -ne $taken) { $slots[$k].Dispose() } }
    $slots[$taken]
}

function Exit-BuildSlot([Threading.Mutex]$Slot) {
    if ($Slot) { $Slot.ReleaseMutex(); $Slot.Dispose() }
}

# Run a native command and stop on failure.
function Invoke-Checked([string]$What, [scriptblock]$Command) {
    & $Command
    if ($LASTEXITCODE -ne 0) { throw "Failed: $What (code $LASTEXITCODE)." }
}
