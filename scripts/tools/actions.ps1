# Aurora Tools: the menu actions — viewers, demo, agents' tasks, disk, logs,
# shortcut (dot-sourced by aurora-tools.ps1).

# Starts a viewer in its own window; the console stays on the tools.
function Start-Viewer([string]$Exe, [string]$Title, [hashtable]$Env = @{}) {
    if (-not (Test-Path $Exe)) { Write-Color "  Introuvable : $Exe" danger; return }
    $saved = @{}
    foreach ($k in $Env.Keys) { $saved[$k] = [Environment]::GetEnvironmentVariable($k); Set-Item "env:$k" $Env[$k] }
    try {
        Start-Process -FilePath $Exe -ArgumentList '--title', "`"$Title`"" -WorkingDirectory (Split-Path $Exe -Parent) | Out-Null
        Write-Color "  Lancé : $Title" success
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
        if ($ok) { Write-Color "  Compilé en $([math]::Floor($sw.Elapsed.TotalMinutes)) min $($sw.Elapsed.Seconds) s." success }
        else { Write-Color '  La compilation a échoué (voir ci-dessus).' danger }
        $ok
    } finally {
        Pop-Location
    }
}

function Invoke-ReleaseViewer {
    Write-Title 'Viewer release : le dernier main de GitHub, en --release'
    Write-Color '  Compilé dans RELEASE\ ; tes changements locaux n''y entrent pas.' muted
    & (Join-Path $Paths.Repo 'scripts\build-release.ps1')
    if ($LASTEXITCODE -eq 0 -and (Read-YesNo 'Lancer ce viewer ?')) {
        Start-Viewer (Join-Path $Paths.Release 'Aurora-Viewer\aurora-viewer.exe') 'Release'
    }
}

function Invoke-DevViewer {
    Write-Title 'Viewer de dev : ton dépôt local, profil par défaut'
    if (Invoke-Cargo @('build', '-p', 'aurora-viewer')) {
        if (Read-YesNo 'Lancer ce viewer ?') { Start-Viewer (Join-Path $Paths.Repo 'target\debug\aurora-viewer.exe') 'Dev' }
    }
}

function Invoke-DebugViewer {
    Write-Title 'Viewer de debug : console, contrôles de débogage, infos complètes'
    Write-Color '  Profil « debugging » (Cargo.toml) : son propre dossier target\debugging, quelques Go de plus.' muted
    if (Invoke-Cargo @('build', '--profile', 'debugging', '-p', 'aurora-viewer')) {
        if (Read-YesNo 'Lancer ce viewer (avec sa console) ?') {
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
    $items = @(@{ Key = '0'; Label = 'Démo simple'; Hint = 'la place, les avatars, le chat'; Var = $null })
    $i = 0
    foreach ($s in Get-DemoScenarios) {
        $items += @{ Key = $shortcuts[$i]; Label = $s.Label; Hint = $s.Var; Var = $s.Var }
        $i++
    }
    $pick = Show-Menu 'Démo hors ligne : quel scénario ?' $items { Show-Logo 'Démo' }
    if (-not $pick) { return $false }
    Write-Title "Démo : $($pick.Label)"
    if (Invoke-Cargo @('build', '-p', 'aurora-viewer')) {
        $vars = @{ AURORA_DEMO = '1' }
        if ($pick.Var) { $vars[$pick.Var] = '1' }
        Start-Viewer (Join-Path $Paths.Repo 'target\debug\aurora-viewer.exe') "Démo $($pick.Label)" $vars
    }
    $true
}

function Get-FolderSize([string]$Dir) {
    if (-not (Test-Path $Dir)) { return 0 }
    (Get-ChildItem -LiteralPath $Dir -Recurse -File -Force -ErrorAction SilentlyContinue | Measure-Object Length -Sum).Sum
}

function Show-Tasks {
    Write-Title "Tâches des agents ($($Paths.Work))"
    $dirs = @(Get-ChildItem -LiteralPath $Paths.Work -Directory -Force -ErrorAction SilentlyContinue)
    if (-not $dirs) { Write-Color '  Aucune tâche en cours.' muted; return @() }
    Write-Color '  Calcul (PR sur GitHub, tailles)…' muted_dim
    $rows = foreach ($d in $dirs) {
        $branch = (git -C $d.FullName rev-parse --abbrev-ref HEAD 2>$null)
        $pr = if ($branch) { gh pr view $branch.Trim() --json number,state 2>$null | ConvertFrom-Json }
        $last = Get-ChildItem -LiteralPath (Join-Path $d.FullName 'target') -Directory -Force -ErrorAction SilentlyContinue | ForEach-Object {
            Get-Item -LiteralPath (Join-Path $_.FullName 'deps') -Force -ErrorAction SilentlyContinue
        } | Sort-Object LastWriteTime -Descending | Select-Object -First 1
        [pscustomobject]@{
            Name = $d.Name
            Branch = $branch
            Pr = if ($pr) { "#$($pr.number) $($pr.state.ToLower())" } else { 'pas de PR' }
            Merged = $pr -and $pr.state -eq 'MERGED'
            Changes = [bool](git -C $d.FullName status --porcelain 2>$null)
            Build = if ($last) { $last.LastWriteTime.ToString('dd/MM HH:mm') } else { '-' }
            Size = '{0:N1} Go' -f ((Get-FolderSize (Join-Path $d.FullName 'target')) / 1GB)
        }
    }
    Write-Rule
    Write-Color ('  ' + 'Tâche'.PadRight(28) + 'PR'.PadRight(16) + 'Changements'.PadRight(14) + 'Compilé le'.PadRight(14) + 'target') muted
    Write-Rule
    foreach ($r in $rows) {
        Write-Color ('  ' + $r.Name.PadRight(28)) ink -NoNewline
        Write-Color $r.Pr.PadRight(16) $(if ($r.Merged) { 'teal' } elseif ($r.Pr -eq 'pas de PR') { 'muted' } else { 'violet_light' }) -NoNewline
        Write-Color $(if ($r.Changes) { 'oui' } else { 'non' }).PadRight(14) $(if ($r.Changes) { 'warn' } else { 'muted' }) -NoNewline
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
    if (Read-YesNo "Faire le ménage des $($merged.Count) tâche(s) fusionnée(s) (end-task.ps1) ?") {
        foreach ($r in $merged) { & (Join-Path $Paths.Repo 'scripts\end-task.ps1') -Name $r.Name }
    }
    Wait-Back
}

function Invoke-Disk {
    Write-Title 'Place prise par les compilations'
    $clean = Join-Path $Paths.Repo 'scripts\clean.ps1'
    & $clean
    $pick = Show-Menu 'Libérer de la place ?' @(
        @{ Key = '1'; Label = 'Compilations du dépôt'; Hint = 'garde target\release' }
        @{ Key = '2'; Label = 'Aussi celles des tâches'; Hint = 'les agents devront tout recompiler' }
    )
    switch ($pick.Key) {
        '1' { & $clean -Apply }
        '2' { if (Read-YesNo 'Les tâches en cours devront tout recompiler. Continuer ?') { & $clean -Apply -Tasks } }
    }
    if ($pick) { Wait-Back }
}

function Invoke-Logs {
    $dir = Join-Path $env:LOCALAPPDATA 'Aurora\AuroraViewer\data\logs'
    Write-Title "Logs du viewer ($dir)"
    $logs = @(Get-ChildItem -LiteralPath $dir -Filter *.log -ErrorAction SilentlyContinue | Sort-Object LastWriteTime -Descending | Select-Object -First 8)
    if (-not $logs) { Write-Color '  Aucun log pour l''instant.' muted; Wait-Back; return }
    foreach ($l in $logs) {
        $name = if ($l.Name.Length -gt 46) { $l.Name.Substring(0, 45) + '…' } else { $l.Name }
        Write-Color ('  ' + $name.PadRight(48)) ink -NoNewline
        Write-Color ('{0}   {1,8:N0} Ko' -f $l.LastWriteTime.ToString('dd/MM HH:mm'), ($l.Length / 1KB)) muted
    }
    $last = $logs[0]
    $pick = Show-Menu 'Que faire ?' @(
        @{ Key = '1'; Label = 'Ouvrir le dossier des logs'; Hint = 'Explorateur' }
        @{ Key = '2'; Label = "Afficher la fin de $($last.Name)"; Hint = '40 dernières lignes' }
        @{ Key = '3'; Label = "Copier le chemin de $($last.Name)"; Hint = 'pour l''envoyer à un agent' }
    )
    switch ($pick.Key) {
        '1' { Start-Process explorer.exe $dir }
        '2' { Write-Host ''; Get-Content -LiteralPath $last.FullName -Tail 40 -Encoding UTF8 | ForEach-Object { Write-Color "  $_" muted }; Wait-Back }
        '3' { Set-Clipboard -Value $last.FullName; Write-Color "  Copié : $($last.FullName)" success; Wait-Back }
    }
}

function Invoke-Shortcut {
    Write-Title 'Raccourcis vers les outils Aurora'
    $target = Join-Path $Paths.Repo 'aurora-tools.cmd'
    $icon = Join-Path $Paths.Repo 'assets\branding\aurora.ico'
    $places = @(
        @{ Name = 'Bureau'; Dir = [Environment]::GetFolderPath('Desktop') }
        @{ Name = 'menu Démarrer'; Dir = [Environment]::GetFolderPath('Programs') }
    )
    # "Aurora PR" opens the PR panel straight away (to keep in a corner)
    $links = @(
        @{ Name = 'Aurora Tools'; Arguments = ''; Description = 'Outils de développement d''Aurora Viewer' }
        @{ Name = 'Aurora PR'; Arguments = '-Action prs'; Description = 'Suivi des PR d''Aurora Viewer, en direct' }
    )
    if (-not (Read-YesNo 'Créer « Aurora Tools » et « Aurora PR » sur le Bureau et dans le menu Démarrer ?')) { return }
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
        Write-Color "  Créés : $($pl.Name)" success
    }
    Wait-Back
}
