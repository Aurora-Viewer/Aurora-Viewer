# Aurora Tools: the menu actions — viewers, demo, agents' tasks, disk, logs,
# shortcut (dot-sourced by aurora-tools.ps1).

# The viewer's executable in a build folder.
function Get-ViewerExe([string]$Dir) {
    Join-Path $Dir $(if ($IsLinux) { 'aurora-viewer' } else { 'aurora-viewer.exe' })
}

# A script of scripts/, from the checkout these tools run from: a worktree's
# branch can differ from main.
function Get-ToolScript([string]$Name) {
    Join-Path (Split-Path $PSScriptRoot -Parent) $Name
}

# Linux: the command that opens a terminal window on a program, for the Debug
# viewer's console. $TERMINAL first, then the usual ones; $null without any.
function Get-TerminalCommand {
    $known = @(
        if ($env:TERMINAL) { , @($env:TERMINAL, '-e') }
        , @('xdg-terminal-exec'); , @('konsole', '-e'); , @('gnome-terminal', '--'); , @('kgx', '--'); , @('ptyxis', '--')
        , @('kitty'); , @('foot'); , @('alacritty', '-e'); , @('wezterm', 'start', '--'); , @('ghostty', '-e'); , @('xterm', '-e')
    )
    foreach ($k in $known) {
        if (Get-Command $k[0] -CommandType Application -ErrorAction SilentlyContinue) { return , $k }
    }
}

# Starts a viewer in its own window; the console stays on the tools. -Console:
# on Linux the viewer gets a terminal window of its own, as the Debug viewer
# has a console on Windows; with no terminal known, it runs in this one.
function Start-Viewer([string]$Exe, [string]$Title, [hashtable]$Env = @{}, [switch]$Console) {
    if (-not (Test-Path $Exe)) { Write-Color "  Not found: $Exe" danger; return }
    $saved = @{}
    foreach ($k in $Env.Keys) { $saved[$k] = [Environment]::GetEnvironmentVariable($k); Set-Item "env:$k" $Env[$k] }
    try {
        if ($IsLinux) {
            Push-Location (Split-Path $Exe -Parent)
            try {
                $terminal = if ($Console) { Get-TerminalCommand }
                if ($Console -and -not $terminal) {
                    Write-Color "  Started: $Title (no terminal program found: close it to get back to the tools)" warn
                    Set-CursorVisible $true
                    & $Exe --title $Title
                    return
                }
                if ($terminal) {
                    # through env: a terminal server (gnome-terminal) does not
                    # pass on this process's environment or folder
                    $vars = @($Env.Keys | ForEach-Object { "$_=$($Env[$_])" })
                    $inner = @('sh', '-c', 'cd "$1" && shift && exec env "$@"', 'sh', (Split-Path $Exe -Parent)) + $vars + @($Exe, '--title', $Title)
                    Start-Detached $terminal[0] (@($terminal | Select-Object -Skip 1) + $inner)
                } else {
                    Start-Detached $Exe @('--title', $Title)
                }
            } finally { Pop-Location }
        } else {
            Start-Process -FilePath $Exe -ArgumentList '--title', "`"$Title`"" -WorkingDirectory (Split-Path $Exe -Parent) | Out-Null
        }
        Write-Color "  Started: $Title" success
    } finally {
        foreach ($k in $saved.Keys) { if ($null -eq $saved[$k]) { Remove-Item "env:$k" -ErrorAction SilentlyContinue } else { Set-Item "env:$k" $saved[$k] } }
    }
}

# cargo build in the repository; $true when it succeeded.
function Invoke-Cargo([string[]]$Arguments) {
    Push-Location $Paths.Repo
    try {
        $sw = [Diagnostics.Stopwatch]::StartNew()
        & cargo @Arguments | Out-Host
        $ok = $LASTEXITCODE -eq 0
        Write-Host ''
        if ($ok) { Write-Color "  Built in $([math]::Floor($sw.Elapsed.TotalMinutes)) min $($sw.Elapsed.Seconds) s." success }
        else { Write-Color '  The build failed (see above).' danger }
        $ok
    } finally {
        Pop-Location
    }
}

function Invoke-ReleaseViewer {
    Write-Title 'Release viewer: the latest main from GitHub, with --release'
    Write-Color "  Built in RELEASE$DirSep; your local changes are not included." muted
    & (Get-ToolScript 'build-release.ps1')
    if ($LASTEXITCODE -eq 0 -and (Read-YesNo 'Start this viewer?')) {
        Start-Viewer (Get-ViewerExe (Join-Path $Paths.Release 'Aurora-Viewer')) 'Release'
    }
}

function Invoke-DevViewer {
    Write-Title 'Dev viewer: your local repository, default profile'
    if (Invoke-Cargo @('build', '-p', 'aurora-viewer')) {
        if (Read-YesNo 'Start this viewer?') { Start-Viewer (Get-ViewerExe (Join-Path $Paths.Repo 'target\debug')) 'Dev' }
    }
}

function Invoke-DebugViewer {
    Write-Title 'Debug viewer: console, debugging controls, full information'
    Write-Color "  Profile `"debugging`" (Cargo.toml): its own target$($DirSep)debugging folder, a few more GB." muted
    if (Invoke-Cargo @('build', '--profile', 'debugging', '-p', 'aurora-viewer')) {
        if (Read-YesNo 'Start this viewer (with its console)?') {
            Start-Viewer (Get-ViewerExe (Join-Path $Paths.Repo 'target\debugging')) 'Debug' @{ RUST_BACKTRACE = '1' } -Console
        }
    }
}

# Demo scenarios switched on with "=1", read from the tables of docs/TESTING.md.
function Get-DemoScenarios {
    $switches = Join-Path $Paths.Repo 'docs\TESTING.md'
    if (-not (Test-Path $switches)) { return @() }
    Get-Content $switches -Encoding UTF8 | ForEach-Object {
        if ($_ -match '^\| `(AURORA_DEMO_[A-Z_]+)=1`[^|]*\| ([^|]+)\|') {
            [pscustomobject]@{ Var = $Matches[1]; Label = $Matches[2].Trim() }
        }
    }
}

function Invoke-Demo {
    # shortcuts: 1 to 9, then a to z
    $shortcuts = @(1..9 | ForEach-Object { [string]$_ }) + @(97..122 | ForEach-Object { [string][char]$_ })
    $items = @(@{ Key = '0'; Label = 'Simple demo'; Hint = 'the plaza, the avatars, the chat'; Var = $null })
    $i = 0
    foreach ($s in Get-DemoScenarios) {
        $items += @{ Key = $shortcuts[$i]; Label = $s.Label; Hint = $s.Var; Var = $s.Var }
        $i++
    }
    $pick = Show-Menu 'Offline demo: which scenario?' $items { Show-Logo 'Demo' }
    if (-not $pick) { return $false }
    Write-Title "Demo: $($pick.Label)"
    if (Invoke-Cargo @('build', '-p', 'aurora-viewer')) {
        $vars = @{ AURORA_DEMO = '1' }
        if ($pick.Var) { $vars[$pick.Var] = '1' }
        Start-Viewer (Get-ViewerExe (Join-Path $Paths.Repo 'target\debug')) "Demo $($pick.Label)" $vars
    }
    $true
}

function Get-FolderSize([string]$Dir) {
    if (-not (Test-Path $Dir)) { return 0 }
    (Get-ChildItem -LiteralPath $Dir -Recurse -File -Force -ErrorAction SilentlyContinue | Measure-Object Length -Sum).Sum
}

function Show-Tasks {
    Write-Title "Agent tasks ($($Paths.Work))"
    $dirs = @(Get-ChildItem -LiteralPath $Paths.Work -Directory -Force -ErrorAction SilentlyContinue)
    if (-not $dirs) { Write-Color '  No task in progress.' muted; return @() }
    Write-Color '  Computing (PRs on GitHub, sizes)…' muted_dim
    $rows = foreach ($d in $dirs) {
        $branch = (git -C $d.FullName rev-parse --abbrev-ref HEAD 2>$null)
        $pr = if ($branch) { gh pr view $branch.Trim() --json number,state 2>$null | ConvertFrom-Json }
        $last = Get-ChildItem -LiteralPath (Join-Path $d.FullName 'target') -Directory -Force -ErrorAction SilentlyContinue | ForEach-Object {
            Get-Item -LiteralPath (Join-Path $_.FullName 'deps') -Force -ErrorAction SilentlyContinue
        } | Sort-Object LastWriteTime -Descending | Select-Object -First 1
        [pscustomobject]@{
            Name = $d.Name
            Branch = $branch
            Pr = if ($pr) { "#$($pr.number) $($pr.state.ToLower())" } else { 'no PR' }
            Merged = $pr -and $pr.state -eq 'MERGED'
            Changes = [bool](git -C $d.FullName status --porcelain 2>$null)
            Build = if ($last) { $last.LastWriteTime.ToString('MM-dd HH:mm') } else { '-' }
            Size = '{0:N1} GB' -f ((Get-FolderSize (Join-Path $d.FullName 'target')) / 1GB)
        }
    }
    Write-Rule
    Write-Color ('  ' + 'Task'.PadRight(28) + 'PR'.PadRight(16) + 'Changes'.PadRight(14) + 'Built on'.PadRight(14) + 'target') muted
    Write-Rule
    foreach ($r in $rows) {
        Write-Color ('  ' + $r.Name.PadRight(28)) ink -NoNewline
        Write-Color $r.Pr.PadRight(16) $(if ($r.Merged) { 'teal' } elseif ($r.Pr -eq 'no PR') { 'muted' } else { 'violet_light' }) -NoNewline
        Write-Color $(if ($r.Changes) { 'yes' } else { 'no' }).PadRight(14) $(if ($r.Changes) { 'warn' } else { 'muted' }) -NoNewline
        Write-Color $r.Build.PadRight(14) muted -NoNewline
        Write-Color $r.Size muted
    }
    Write-Rule
    @($rows)
}

function Invoke-Tasks {
    $rows = Show-Tasks
    $merged = @($rows | Where-Object { $_.Merged })
    if (-not $merged) { Wait-Back; return }
    if (Read-YesNo "Clean up the $($merged.Count) merged task(s) (end-task.ps1)?") {
        foreach ($r in $merged) { & (Get-ToolScript 'end-task.ps1') -Name $r.Name }
    }
    Wait-Back
}

function Invoke-Disk {
    Write-Title 'Space taken by builds'
    $clean = Get-ToolScript 'clean.ps1'
    & $clean
    $pick = Show-Menu 'Free some space?' @(
        @{ Key = '1'; Label = 'Builds of the repository'; Hint = "keeps target$($DirSep)release" }
        @{ Key = '2'; Label = 'Also those of the tasks'; Hint = 'the agents will have to rebuild everything' }
        @{ Key = '3'; Label = 'Incremental caches of the tasks'; Hint = 'three quarters of the space, dependencies kept' }
    )
    switch ($pick.Key) {
        '1' { & $clean -Apply }
        '3' { & $clean -Apply -Incremental }
        '2' { if (Read-YesNo 'The tasks in progress will have to rebuild everything. Continue?') { & $clean -Apply -Tasks } }
    }
    if ($pick) { Wait-Back }
}

function Invoke-Logs {
    $dir = if ($IsLinux) {
        # same rule as the viewer (logging.rs, directories' data_local_dir)
        Join-Path $(if ($env:XDG_DATA_HOME) { $env:XDG_DATA_HOME } else { Join-Path $HOME '.local/share' }) 'auroraviewer/logs'
    } else {
        Join-Path $env:LOCALAPPDATA 'Aurora\AuroraViewer\data\logs'
    }
    Write-Title "Viewer logs ($dir)"
    $logs = @(Get-ChildItem -LiteralPath $dir -Filter *.log -ErrorAction SilentlyContinue | Sort-Object LastWriteTime -Descending | Select-Object -First 8)
    if (-not $logs) { Write-Color '  No log yet.' muted; Wait-Back; return }
    foreach ($l in $logs) {
        $name = if ($l.Name.Length -gt 46) { $l.Name.Substring(0, 45) + '…' } else { $l.Name }
        Write-Color ('  ' + $name.PadRight(48)) ink -NoNewline
        Write-Color ('{0}   {1,8:N0} KB' -f $l.LastWriteTime.ToString('MM-dd HH:mm'), ($l.Length / 1KB)) muted
    }
    $last = $logs[0]
    $pick = Show-Menu 'What to do?' @(
        @{ Key = '1'; Label = 'Open the logs folder'; Hint = $(if ($IsLinux) { 'file manager' } else { 'Explorer' }) }
        @{ Key = '2'; Label = "Show the end of $($last.Name)"; Hint = 'last 40 lines' }
        @{ Key = '3'; Label = "Copy the path of $($last.Name)"; Hint = 'to send it to an agent' }
    )
    switch ($pick.Key) {
        '1' { if ($IsLinux) { Open-External $dir } else { Start-Process explorer.exe $dir } }
        '2' { Write-Host ''; Get-Content -LiteralPath $last.FullName -Tail 40 -Encoding UTF8 | ForEach-Object { Write-Color "  $_" muted }; Wait-Back }
        '3' {
            if (Copy-ToClipboard $last.FullName) { Write-Color "  Copied: $($last.FullName)" success }
            else { Write-Color "  No clipboard tool (wl-clipboard, xclip or xsel). The path: $($last.FullName)" warn }
            Wait-Back
        }
    }
}

# The text of a desktop entry that opens the tools in a terminal. Quoting: the
# Exec key of the Desktop Entry specification.
function Get-DesktopEntry([string]$Name, [string]$Description, [string]$Launcher, [string]$Arguments, [string]$Icon) {
    $exec = '"' + ($Launcher -replace '([\\"`$])', '\$1') + '"' + $(if ($Arguments) { " $Arguments" })
    @(
        '[Desktop Entry]'
        'Type=Application'
        "Name=$Name"
        "Comment=$Description"
        "Exec=$exec"
        "Icon=$Icon"
        'Terminal=true'
        'Categories=Development;'
        ''
    ) -join "`n"
}

# Linux: desktop entries instead of .lnk files, in the applications menu and
# on the Desktop (which only starts the ones that are executable).
function Invoke-LinuxShortcut {
    $repo = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
    $launcher = Join-Path $repo 'aurora-tools.sh'
    $icon = Join-Path $repo 'assets/branding/aurora-logo.svg'
    $data = if ($env:XDG_DATA_HOME) { $env:XDG_DATA_HOME } else { Join-Path $HOME '.local/share' }
    $desktop = if (Get-Command xdg-user-dir -ErrorAction SilentlyContinue) { xdg-user-dir DESKTOP } else { Join-Path $HOME 'Desktop' }
    if (-not (Read-YesNo 'Create "Aurora Tools" and "Aurora PR" on the Desktop and in the applications menu?')) { return }
    $places = @(
        @{ Name = 'Applications menu'; Dir = Join-Path $data 'applications' }
        # a desktop-less setup has HOME as its Desktop: no icons to put there
        if ($desktop -and $desktop.TrimEnd('/') -ne $HOME.TrimEnd('/')) { @{ Name = 'Desktop'; Dir = $desktop } }
    )
    $links = @(
        @{ File = 'aurora-tools.desktop'; Name = 'Aurora Tools'; Arguments = ''; Description = 'Aurora Viewer development tools' }
        @{ File = 'aurora-pr.desktop'; Name = 'Aurora PR'; Arguments = '-Action prs'; Description = 'Aurora Viewer PR tracking, live' }
    )
    foreach ($pl in $places) {
        New-Item -ItemType Directory -Force -Path $pl.Dir -ErrorAction Stop | Out-Null
        foreach ($l in $links) {
            $file = Join-Path $pl.Dir $l.File
            $text = Get-DesktopEntry $l.Name $l.Description $launcher $l.Arguments $icon
            [IO.File]::WriteAllText($file, $text, (New-Object Text.UTF8Encoding $false))
            chmod +x $file
        }
        Write-Color "  Created: $($pl.Name) ($($pl.Dir))" success
    }
    Wait-Back
}

function Invoke-Shortcut {
    Write-Title 'Shortcuts to the Aurora tools'
    if ($IsLinux) { Invoke-LinuxShortcut; return }
    $target = Join-Path $Paths.Repo 'aurora-tools.cmd'
    $icon = Join-Path $Paths.Repo 'assets\branding\aurora.ico'
    $places = @(
        @{ Name = 'Desktop'; Dir = [Environment]::GetFolderPath('Desktop') }
        @{ Name = 'Start menu'; Dir = [Environment]::GetFolderPath('Programs') }
    )
    # "Aurora PR" opens the PR panel straight away (to keep in a corner)
    $links = @(
        @{ Name = 'Aurora Tools'; Arguments = ''; Description = 'Aurora Viewer development tools' }
        @{ Name = 'Aurora PR'; Arguments = '-Action prs'; Description = 'Aurora Viewer PR tracking, live' }
    )
    if (-not (Read-YesNo 'Create "Aurora Tools" and "Aurora PR" on the Desktop and in the Start menu?')) { return }
    $shell = New-Object -ComObject WScript.Shell
    foreach ($pl in $places) {
        foreach ($l in $links) {
            $lnk = $shell.CreateShortcut((Join-Path $pl.Dir "$($l.Name).lnk"))
            $lnk.TargetPath = $target
            $lnk.Arguments = $l.Arguments
            $lnk.WorkingDirectory = $Paths.Repo
            $lnk.Description = $l.Description
            if (Test-Path $icon) { $lnk.IconLocation = "$icon,0" }
            $lnk.Save()
        }
        Write-Color "  Created: $($pl.Name)" success
    }
    Wait-Back
}
