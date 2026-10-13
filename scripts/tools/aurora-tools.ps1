<#
.SYNOPSIS
    The development tools of Aurora Viewer, as a keyboard menu
    (arrows, Enter, Esc): diagnosis and repair of the environment,
    release / dev / debug viewers, demo, agent tasks, disk, logs,
    live PR tracking (bottom bar, notifications), release
    management, shortcuts, Claude Code permissions, repository reset. Started by aurora-tools.cmd, at the repository root.

.DESCRIPTION
    Without a repository (first installation: the one-line command runs
    scripts/setup.ps1, which downloads these tools), it installs everything in
    the chosen folder, then opens the menu of the installed repository.

.EXAMPLE
    ./scripts/tools/aurora-tools.ps1                     # the menu
    ./scripts/tools/aurora-tools.ps1 -Action diagnose    # one action, without the menu
#>
param(
    [ValidateSet('menu', 'install', 'diagnose', 'repair', 'release', 'dev', 'debug', 'demo', 'tasks', 'disk', 'logs', 'shortcut', 'prs', 'releases', 'reset', 'claude')]
    [string]$Action = 'menu',
    # project folder for -Action install (asked for when not given)
    [string]$Path,
    # -Action install without questions
    [switch]$Yes,
    # releases, reset, claude: show what would be written or erased instead of doing it
    [switch]$DryRun
)
$ErrorActionPreference = 'Continue'
# every module of this folder: ui.ps1 first, the others use it
foreach ($m in @('ui.ps1') + @(Get-ChildItem -LiteralPath $PSScriptRoot -Filter *.ps1 | Where-Object { $_.Name -notin 'ui.ps1', 'aurora-tools.ps1' } | ForEach-Object Name)) {
    . (Join-Path $PSScriptRoot $m)
}
if ($Interactive) { $Host.UI.RawUI.WindowTitle = 'Aurora Tools' }
# gh, git and cargo write UTF-8; the console decodes their output with its
# OEM code page by default (accents of PR titles came out garbled)
try { [Console]::OutputEncoding = [Text.Encoding]::UTF8 } catch { }

# The project folder of the repository these tools belong to (also from an
# agent's worktree: the main repository is the one in git's common folder).
function Find-Project {
    $repo = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
    if (-not (Test-Path (Join-Path $repo 'AGENTS.md'))) { return $null }
    $common = if (Get-Command git -ErrorAction SilentlyContinue) { git -C $repo rev-parse --path-format=absolute --git-common-dir 2>$null }
    if ($common) { $repo = Split-Path $common.Trim() -Parent }
    $p = Get-Paths (Split-Path $repo -Parent)
    $p.Repo = $repo
    $p
}

function Show-Failure($Err) {
    Write-Host ''
    Write-Color "  FAILED: $($Err.Exception.Message)" danger
    Write-Color '  Fix the problem then run it again: what is done is kept.' muted
}

# Compare the scripts on disk with the session's snapshot, including module
# additions/removals. A new PowerShell process must load them after repair.
function Get-ToolsFingerprint([string]$RepoPath) {
    $folder = Join-Path $RepoPath 'scripts\tools'
    ((Get-ChildItem -LiteralPath $folder -Filter *.ps1 | Sort-Object Name | ForEach-Object {
        "$($_.Name):$((Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash)"
    }) -join '|')
}

function Invoke-ToolsRepair($P) {
    try { Invoke-Repair $P } finally {
        # A later repair step may fail after git has already updated the tools.
        if ($ToolsFingerprint -ne (Get-ToolsFingerprint $P.Repo)) {
            $script:ToolsRestartPath = Join-Path $P.Repo 'scripts\tools\aurora-tools.ps1'
        }
    }
}

function Restart-AuroraTools([string]$ToolsPath) {
    Write-Color '  Aurora Tools was updated. Restarting automatically…' success
    # Run in this console, with the same PowerShell edition. The old session
    # has stopped its watcher and will exit when the replacement exits.
    $engine = (Get-Process -Id $PID).Path
    $arguments = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $ToolsPath, '-Action', 'menu')
    if ($DryRun) { $arguments += '-DryRun' }
    & $engine @arguments
}

# --- Installation ---------------------------------------------------------------

function Start-Install {
    Show-Logo 'Development environment setup'
    $here = (Get-Location).Path
    # in the repository itself: propose its parent (no clone inside a clone)
    $proposed = if (Test-Path (Join-Path $here 'AGENTS.md')) { Split-Path $here -Parent } else { $here }
    if ($Path) {
        $root = [IO.Path]::GetFullPath($Path)
        Show-Checks (Get-Checks (Get-Paths $root)) "Project folder: $root"
    } else {
        $root = $proposed
        Show-Checks (Get-Checks (Get-Paths $root)) "Proposed project folder: $root"
        if (-not $Yes -and -not (Read-YesNo "Install everything in ${root}?")) {
            do {
                $answer = Read-Text 'Path of the Aurora-Viewer folder (created if missing), for example D:\Dev\Aurora-Viewer; empty: cancel'
                if (-not $answer) { Write-Color '  Nothing was installed.' muted; exit 0 }
                $root = [IO.Path]::GetFullPath($answer)
                if (Test-Path -LiteralPath $root -PathType Leaf) { Write-Color "  $root is a file, not a folder." warn; $root = $null }
            } until ($root)
            Write-Host ''
            Show-Checks (Get-Checks (Get-Paths $root)) "Project folder: $root"
            if (-not (Read-YesNo "Install everything in ${root}?")) { Write-Color '  Nothing was installed.' muted; exit 0 }
        }
    }
    $p = Get-Paths $root
    $started = Get-Date
    try { Invoke-Repair $p } catch { Show-Failure $_; exit 1 }
    Write-Host ''
    Show-Checks (Get-Checks $p) 'Summary' -Summary
    $took = (Get-Date) - $started
    Write-Host ''
    Write-Aurora "  » All set, in $([math]::Floor($took.TotalMinutes)) min $($took.Seconds) s."
    Write-Color "    $($p.Repo)" ink
    Write-Color '    Open your agent (Claude Code…) in this folder; your role is explained in HUMANS.md.' muted
    $tools = Join-Path $p.Repo 'scripts\tools\aurora-tools.ps1'
    if (-not $Yes -and (Test-Path $tools) -and (Read-YesNo 'Open the Aurora tools (aurora-tools.cmd)?')) {
        & $tools
    }
}

# --- Menu -------------------------------------------------------------------------

function Show-Status($Checks) {
    $problems = Get-Problems $Checks
    Write-Color "  Project: $($Paths.Root)" muted
    if (-not $problems) {
        Write-Color '  Environment: everything is in place and up to date.' success
    } else {
        Write-Color "  Environment: $($problems.Count) item(s) to repair or update" warn
        foreach ($c in $problems) { Write-Color "    • $($c.Item): $(if ($c.State -eq 'todo') { 'missing' } else { $c.Found })" warn }
    }
    foreach ($c in $Checks | Where-Object { $_.Id -eq 'release' -and $_.Action -ne 'nothing to do' }) {
        Write-Color "  $($c.Item): $($c.Found)" teal
    }
}

function Show-Main {
    $checks = $null
    $selected = 0
    while ($true) {
        if (-not $checks) {
            Show-Logo 'Tools'
            Write-Color '  Checking the environment (GitHub, Firestorm, Rust)…' muted_dim
            $checks = Get-Checks $Paths
        }
        $problems = Get-Problems $checks
        $items = @()
        if ($problems) { $items += @{ Key = 'r'; Label = "Repair / update ($($problems.Count))"; Hint = 'installs what is missing, updates the rest' } }
        $items += @(
            @{ Key = '1'; Label = 'Release viewer'; Hint = 'latest main from GitHub, --release, in RELEASE\' }
            @{ Key = '2'; Label = 'Dev viewer'; Hint = 'your local repository, default profile' }
            @{ Key = '3'; Label = 'Debug viewer'; Hint = 'with console and debugging controls' }
            @{ Key = '4'; Label = 'Demo'; Hint = 'offline, choice of scenario' }
            @{ Key = '5'; Label = 'Agent tasks'; Hint = 'folders of work\, PRs, cleanup' }
            @{ Key = '6'; Label = 'Disk'; Hint = 'space taken by builds, cleanup' }
            @{ Key = '7'; Label = 'Viewer logs'; Hint = 'open, read the end, copy the path' }
            @{ Key = 'p'; Label = 'PR tracking'; Hint = 'live, to keep in a corner of the screen' }
            @{ Key = 'm'; Label = 'Manage releases'; Hint = 'create or delete a release' }
            @{ Key = '8'; Label = 'Shortcuts'; Hint = 'Desktop and Start menu' }
            @{ Key = 'c'; Label = 'Claude permissions'; Hint = 'let the agents follow and merge their PRs, handle the issues' }
            @{ Key = 'v'; Label = 'Check again'; Hint = 'the full table of the environment' }
            @{ Key = 'x'; Label = 'Reset the repository'; Hint = 'fresh clone from GitHub, erases everything local (inventory first)' }
            @{ Key = '0'; Label = 'Quit'; Hint = '' }
        )
        $pick = Show-Menu 'What do you want to do?' $items { Show-Logo 'Tools'; Show-Status $checks } $selected
        if (-not $pick -or $pick.Key -eq '0') { Clear-Screen; return }
        $selected = [array]::IndexOf(@($items | ForEach-Object { $_.Key }), $pick.Key)
        Show-Logo $pick.Label
        try {
            switch ($pick.Key) {
                'r' { Invoke-ToolsRepair $Paths; if ($ToolsRestartPath) { return }; $checks = $null; Wait-Back }
                '1' { Invoke-ReleaseViewer; $checks = $null; Wait-Back }
                '2' { Invoke-DevViewer; Wait-Back }
                '3' { Invoke-DebugViewer; Wait-Back }
                '4' { if (Invoke-Demo) { Wait-Back } }
                '5' { Invoke-Tasks }
                '6' { Invoke-Disk }
                '7' { Invoke-Logs }
                'p' { Show-PrPanel }
                'm' { Invoke-Releases }
                '8' { Invoke-Shortcut }
                'c' { Invoke-ClaudePermissions }
                'x' { Invoke-Reset; $checks = $null }
                'v' { $checks = Get-Checks $Paths; Show-Checks $checks "Environment: $($Paths.Root)"; Wait-Back }
            }
        } catch {
            Show-Failure $_
            if ($ToolsRestartPath) { return }
            Wait-Back
        }
    }
}

# --- Entry ------------------------------------------------------------------------

$Paths = Find-Project
if ($Action -eq 'install' -or -not $Paths) { Start-Install; exit }
$ToolsFingerprint = Get-ToolsFingerprint $Paths.Repo
$ToolsRestartPath = $null
$sessionCursorVisible = $true
if ($Interactive) { try { $sessionCursorVisible = [Console]::CursorVisible } catch { } }
try {
    # GitHub followed in the background: PR summary in the footer, notifications
    if ($Action -in 'menu', 'prs') {
        Start-GitHubWatch
        $FooterStatus = { Get-PrSummary }
        $TickHook = { Update-PrNotifications }
    }
    switch ($Action) {
        'menu' { Show-Main }
        'prs' { Show-PrPanel }
        'releases' { Invoke-Releases }
        'reset' { Invoke-Reset }
        'diagnose' { Show-Checks (Get-Checks $Paths) "Environment: $($Paths.Root)" }
        'repair' { try { Invoke-ToolsRepair $Paths } catch { Show-Failure $_; if (-not $ToolsRestartPath) { exit 1 } } }
        'release' { Invoke-ReleaseViewer }
        'dev' { Invoke-DevViewer }
        'debug' { Invoke-DebugViewer }
        'demo' { [void](Invoke-Demo) }
        'tasks' { [void](Show-Tasks) }
        'disk' { Invoke-Disk }
        'logs' { Invoke-Logs }
        'shortcut' { Invoke-Shortcut }
        'claude' { Invoke-ClaudePermissions }
    }
} finally {
    Stop-GitHubWatch
    Set-CursorVisible $sessionCursorVisible
}
if ($ToolsRestartPath) {
    Restart-AuroraTools $ToolsRestartPath
    exit $LASTEXITCODE
}
