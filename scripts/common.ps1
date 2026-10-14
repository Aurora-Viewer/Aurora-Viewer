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

# --- Shared compiler cache (sccache) ------------------------------------------
# Every worktree builds the same dependencies. When sccache is installed (the
# Aurora tools install it), their compiled crates are kept in one folder next
# to the repository (<root>\.sccache) and a new worktree takes them from
# there. Cargo finds sccache through <root>\.cargo\config.toml, which covers
# the main repository and every task of work\, whatever the command (check,
# demo, tests). The scripts write that file while sccache is there and delete
# it once it is gone: without sccache, Cargo builds as before.

$BuildCacheMarker = '# Written by the Aurora Viewer scripts (Sync-BuildCache).'

function Find-Sccache {
    $cmd = Get-Command sccache -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($cmd) { return $cmd.Source }
    # installed by winget after this window was opened: not in its PATH yet
    Get-ChildItem "$env:LOCALAPPDATA\Microsoft\WinGet\Packages\Mozilla.sccache_*\*\sccache.exe" -ErrorAction SilentlyContinue |
        Sort-Object LastWriteTime -Descending | Select-Object -First 1 -ExpandProperty FullName
}

# Writes or deletes <root>\.cargo\config.toml to match the installed sccache.
# A file there that these scripts did not write is left alone. Size of the
# cache: AURORA_BUILD_CACHE_SIZE (sccache format, 10G by default).
function Sync-BuildCache([string]$Root) {
    if (-not $Root) { $Root = (Get-AuroraPaths).Root }
    $file = Join-Path $Root '.cargo\config.toml'
    if ((Test-Path -LiteralPath $file) -and -not (Select-String -LiteralPath $file -SimpleMatch $BuildCacheMarker -Quiet)) { return }
    $sccache = Find-Sccache
    if (-not $sccache) {
        if (Test-Path -LiteralPath $file) { Remove-Item -LiteralPath $file -Force -ErrorAction SilentlyContinue }
        return
    }
    $size = if ($env:AURORA_BUILD_CACHE_SIZE) { $env:AURORA_BUILD_CACHE_SIZE } else { '10G' }
    # TOML literal strings ('…'): the backslashes of the paths stay as they are
    $text = @(
        $BuildCacheMarker,
        '# Deleted by them when sccache is uninstalled; see scripts/common.ps1.',
        '[build]',
        "rustc-wrapper = '$sccache'",
        '',
        '[env]',
        "SCCACHE_DIR = '$(Join-Path $Root '.sccache')'",
        "SCCACHE_CACHE_SIZE = '$size'",
        '# cache server unreachable: compile without it',
        "SCCACHE_IGNORE_SERVER_IO_ERROR = '1'",
        ''
    ) -join "`n"
    if ((Test-Path -LiteralPath $file) -and ([IO.File]::ReadAllText($file) -eq $text)) { return }
    New-Item -ItemType Directory -Force -Path (Split-Path $file) | Out-Null
    [IO.File]::WriteAllText($file, $text, (New-Object Text.UTF8Encoding $false))
}

# --- One heavy build at a time --------------------------------------------------
# Several agents running check.ps1 at once each take every core, and all of
# them end up slower than one after the other. A heavy build takes one of
# AURORA_BUILD_SLOTS slots (2 by default) and waits for a free one. A slot is
# a named mutex: Windows frees it if the script dies.

function Enter-BuildSlot {
    $count = 2
    if ($env:AURORA_BUILD_SLOTS -match '^\d+$' -and [int]$env:AURORA_BUILD_SLOTS -ge 1) { $count = [int]$env:AURORA_BUILD_SLOTS }
    $slots = [Threading.Mutex[]](1..$count | ForEach-Object { New-Object Threading.Mutex($false, "Local\AuroraViewerBuildSlot$_") })
    $waited = [Diagnostics.Stopwatch]::StartNew()
    $taken = -1
    $timeout = 0
    while ($taken -lt 0) {
        try {
            # short waits: Ctrl+C still stops the script between two
            $i = [Threading.WaitHandle]::WaitAny($slots, $timeout)
            if ($i -ne [Threading.WaitHandle]::WaitTimeout) { $taken = $i }
        } catch {
            # a slot whose holder died: it is now ours
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
