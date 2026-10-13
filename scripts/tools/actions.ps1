# Aurora Tools: the menu actions — viewers, demo, agents' tasks, disk, logs,
# shortcut (dot-sourced by aurora-tools.ps1).

# Starts a viewer in its own window; the console stays on the tools.
function Start-Viewer([string]$Exe, [string]$Title, [hashtable]$Env = @{}) {
    if (-not (Test-Path $Exe)) { Write-Color "  Not found: $Exe" danger; return }
    $saved = @{}
    foreach ($k in $Env.Keys) { $saved[$k] = [Environment]::GetEnvironmentVariable($k); Set-Item "env:$k" $Env[$k] }
    try {
        Start-Process -FilePath $Exe -ArgumentList '--title', "`"$Title`"" -WorkingDirectory (Split-Path $Exe -Parent) | Out-Null
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
    Write-Color '  Built in RELEASE\; your local changes are not included.' muted
    & (Join-Path $Paths.Repo 'scripts\build-release.ps1')
    if ($LASTEXITCODE -eq 0 -and (Read-YesNo 'Start this viewer?')) {
        Start-Viewer (Join-Path $Paths.Release 'Aurora-Viewer\aurora-viewer.exe') 'Release'
    }
}

function Invoke-DevViewer {
    Write-Title 'Dev viewer: your local repository, default profile'
    if (Invoke-Cargo @('build', '-p', 'aurora-viewer')) {
        if (Read-YesNo 'Start this viewer?') { Start-Viewer (Join-Path $Paths.Repo 'target\debug\aurora-viewer.exe') 'Dev' }
    }
}

function Invoke-DebugViewer {
    Write-Title 'Debug viewer: console, debugging controls, full information'
    Write-Color '  Profile "debugging" (Cargo.toml): its own target\debugging folder, a few more GB.' muted
    if (Invoke-Cargo @('build', '--profile', 'debugging', '-p', 'aurora-viewer')) {
        if (Read-YesNo 'Start this viewer (with its console)?') {
            Start-Viewer (Join-Path $Paths.Repo 'target\debugging\aurora-viewer.exe') 'Debug' @{ RUST_BACKTRACE = '1' }
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
        Start-Viewer (Join-Path $Paths.Repo 'target\debug\aurora-viewer.exe') "Demo $($pick.Label)" $vars
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
        foreach ($r in $merged) { & (Join-Path $Paths.Repo 'scripts\end-task.ps1') -Name $r.Name }
    }
    Wait-Back
}

function Invoke-Disk {
    Write-Title 'Space taken by builds'
    $clean = Join-Path $Paths.Repo 'scripts\clean.ps1'
    & $clean
    $pick = Show-Menu 'Free some space?' @(
        @{ Key = '1'; Label = 'Builds of the repository'; Hint = 'keeps target\release' }
        @{ Key = '2'; Label = 'Also those of the tasks'; Hint = 'the agents will have to rebuild everything' }
    )
    switch ($pick.Key) {
        '1' { & $clean -Apply }
        '2' { if (Read-YesNo 'The tasks in progress will have to rebuild everything. Continue?') { & $clean -Apply -Tasks } }
    }
    if ($pick) { Wait-Back }
}

function Invoke-Logs {
    $dir = Join-Path $env:LOCALAPPDATA 'Aurora\AuroraViewer\data\logs'
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
        @{ Key = '1'; Label = 'Open the logs folder'; Hint = 'Explorer' }
        @{ Key = '2'; Label = "Show the end of $($last.Name)"; Hint = 'last 40 lines' }
        @{ Key = '3'; Label = "Copy the path of $($last.Name)"; Hint = 'to send it to an agent' }
    )
    switch ($pick.Key) {
        '1' { Start-Process explorer.exe $dir }
        '2' { Write-Host ''; Get-Content -LiteralPath $last.FullName -Tail 40 -Encoding UTF8 | ForEach-Object { Write-Color "  $_" muted }; Wait-Back }
        '3' { Set-Clipboard -Value $last.FullName; Write-Color "  Copied: $($last.FullName)" success; Wait-Back }
    }
}

function Invoke-Shortcut {
    Write-Title 'Shortcuts to the Aurora tools'
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
