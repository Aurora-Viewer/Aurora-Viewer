# Aurora Tools: what the development environment needs, what is missing or
# out of date, and how to fix it (dot-sourced by aurora-tools.ps1).

$RepoUrl = 'https://github.com/Aurora-Viewer/Aurora-Viewer'
$FirestormUrl = 'https://github.com/FirestormViewer/phoenix-firestorm'

function Get-Paths([string]$Root) {
    [pscustomobject]@{
        Root = $Root
        Repo = Join-Path $Root 'aurora-viewer'
        Firestorm = Join-Path $Root 'phoenix-firestorm'
        Work = Join-Path $Root 'work'
        Release = Join-Path $Root 'RELEASE'
    }
}

# One row of the table. State: ok, todo (missing), stale (out of date),
# info (nothing to fix), always (done on every repair).
function New-Check([string]$Id, [string]$Item, [string]$Found, [string]$State, [string]$Action) {
    [pscustomobject]@{ Id = $Id; Item = $Item; Found = $Found; State = $State; Action = $Action }
}

function Get-MsvcName {
    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
    if (Test-Path $vswhere) {
        & $vswhere -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property displayName | Select-Object -First 1
    }
}

# Everything the environment needs. -Offline skips the network checks (is
# main, Firestorm or Rust behind?), which take a few seconds.
function Get-Checks($P, [switch]$Offline) {
    $rows = New-Object System.Collections.ArrayList
    $add = { param($r) [void]$rows.Add($r) }

    $git = if (Get-Command git -ErrorAction SilentlyContinue) { (git --version) -replace '^git version\s+', '' -replace '\.windows.*$', '' }
    & $add (New-Check git 'Git' $git $(if ($git) { 'ok' } else { 'todo' }) $(if ($git) { 'rien à faire' } else { 'installé avec winget, pour toute la machine' }))
    $gh = if (Get-Command gh -ErrorAction SilentlyContinue) { (gh --version | Select-Object -First 1) -replace '^gh version\s+(\S+).*$', '$1' }
    & $add (New-Check gh 'GitHub CLI' $gh $(if ($gh) { 'ok' } else { 'todo' }) $(if ($gh) { 'rien à faire' } else { 'installé avec winget, pour toute la machine' }))
    $msvc = Get-MsvcName
    & $add (New-Check msvc 'Outils C++ de Visual Studio' $msvc $(if ($msvc) { 'ok' } else { 'todo' }) $(if ($msvc) { 'rien à faire' } else { 'installés avec winget (plusieurs Go)' }))

    $rustup = if (Get-Command rustup -ErrorAction SilentlyContinue) { (rustup --version 2>$null) -replace '^rustup\s+(\S+).*$', 'rustup $1' }
    if (-not $rustup) {
        & $add (New-Check rust 'Rust' $null todo 'installé avec winget (rustup, pour ton compte)')
    } elseif (-not $Offline -and ((rustup check 2>$null) -match '^stable.*Update available')) {
        & $add (New-Check rust 'Rust' "$rustup, stable en retard" stale 'mise à jour de la version stable')
    } else {
        & $add (New-Check rust 'Rust' $rustup ok 'rien à faire')
    }

    $hasRepo = Test-Path (Join-Path $P.Repo '.git')
    if (-not $hasRepo) {
        & $add (New-Check repo 'Dépôt aurora-viewer' $null todo 'cloné depuis GitHub')
    } else {
        $branch = (git -C $P.Repo rev-parse --abbrev-ref HEAD).Trim()
        $behind = 0
        if (-not $Offline) {
            git -C $P.Repo fetch origin --quiet 2>$null
            $behind = [int](git -C $P.Repo rev-list --count 'HEAD..origin/main' 2>$null)
        }
        $dirty = [bool](git -C $P.Repo status --porcelain --untracked-files=no)
        if ($branch -ne 'main' -or $dirty) {
            & $add (New-Check repo 'Dépôt aurora-viewer' "sur $branch$(if ($dirty) { ', changements en cours' })" info 'laissé tel quel (tes changements)')
        } elseif ($behind -gt 0) {
            & $add (New-Check repo 'Dépôt aurora-viewer' "main, $behind commit(s) de retard" stale 'mis à jour (git pull)')
        } else {
            & $add (New-Check repo 'Dépôt aurora-viewer' 'main, à jour' ok 'rien à faire')
        }
    }

    if (-not (Test-Path (Join-Path $P.Firestorm '.git'))) {
        & $add (New-Check firestorm 'Sources de Firestorm' $null todo 'clonées (dernier commit seulement)')
    } else {
        $stale = $false
        if (-not $Offline) {
            $remote = (git -C $P.Firestorm ls-remote origin HEAD 2>$null) -split '\s+' | Select-Object -First 1
            $stale = $remote -and $remote -ne (git -C $P.Firestorm rev-parse HEAD).Trim()
        }
        $date = (git -C $P.Firestorm log -1 --format=%cs 2>$null)
        if ($stale) { & $add (New-Check firestorm 'Sources de Firestorm' "du $date, en retard" stale 'mises à jour') }
        else { & $add (New-Check firestorm 'Sources de Firestorm' "du $date" ok 'rien à faire') }
    }

    $font = Test-Path (Join-Path $P.Repo 'assets\emoji\Noto-3D-128.ttf')
    & $add (New-Check font 'Police emoji' $(if ($font) { 'présente' }) $(if ($font) { 'ok' } else { 'todo' }) $(if ($font) { 'rien à faire' } else { 'téléchargée (142 Mo)' }))
    $work = Test-Path $P.Work
    & $add (New-Check work 'Dossier work\ (tâches)' $(if ($work) { 'présent' }) $(if ($work) { 'ok' } else { 'todo' }) $(if ($work) { 'rien à faire' } else { 'créé' }))
    & $add (New-Check crates 'Toolchain Rust et crates' $null always 'installées ou mises à jour (cargo fetch)')

    $login = $null
    if ($gh) {
        gh auth status *> $null
        if ($LASTEXITCODE -eq 0) { $login = "connecté : $(gh api user --jq .login 2>$null)" }
    }
    & $add (New-Check github 'Connexion à GitHub' $login $(if ($login) { 'ok' } else { 'todo' }) $(if ($login) { 'rien à faire' } else { 'gh auth login, dans le navigateur' }))
    $email = if ($git) { if ($hasRepo) { git -C $P.Repo config user.email } else { git config --global user.email } }
    & $add (New-Check identity 'Identité git' $email $(if ($email) { 'ok' } else { 'todo' }) $(if ($email) { 'rien à faire' } else { 'ton pseudo GitHub + adresse noreply' }))

    # the --release viewer of RELEASE\ (menu "Viewer release"): information only
    $version = Join-Path $P.Release 'Aurora-Viewer\version.txt'
    if ($hasRepo -and (Test-Path $version)) {
        # first line: "main : <short sha> <subject> (<date>)", or "PR #42 : …"
        $built = if ((Get-Content $version -Encoding UTF8 -TotalCount 1) -match ' : ([0-9a-f]{7,}) ') { $Matches[1] }
        $main = (git -C $P.Repo rev-parse --short origin/main 2>$null)
        $same = $built -and $main -and ($main.StartsWith($built) -or $built.StartsWith($main))
        & $add (New-Check release 'Viewer release (RELEASE\)' $(if ($same) { 'à jour avec main' } else { "commit $built, main a avancé" }) info $(if ($same) { 'rien à faire' } else { 'menu « Viewer release » pour le refaire' }))
    }
    , $rows.ToArray()
}

function Get-Problems($Checks) { @($Checks | Where-Object { $_.State -in 'todo', 'stale' }) }

# --- Repair -------------------------------------------------------------------

# Tools installed by winget land in PATH for new sessions only: reload it here.
function Update-SessionPath {
    $env:Path = [Environment]::GetEnvironmentVariable('Path', 'Machine') + ';' +
        [Environment]::GetEnvironmentVariable('Path', 'User')
}

function Install-Tool([string]$Id, [string]$What, [string[]]$Extra = @()) {
    if (-not (Get-Command winget -ErrorAction SilentlyContinue)) {
        throw "$What manque, et winget (le gestionnaire de paquets de Windows) n'est pas disponible : installe-le à la main."
    }
    Write-Color "  Installation de $What (une fenêtre d'autorisation Windows peut s'ouvrir)…" muted
    winget install --id $Id --exact --source winget --accept-package-agreements --accept-source-agreements @Extra
    if ($LASTEXITCODE -ne 0) { throw "installation de $What (code $LASTEXITCODE)." }
    Update-SessionPath
}

# Installs what is missing and updates what is behind; throws on failure.
# Everything already there is kept: running it again is safe.
function Invoke-Repair($P) {
    Write-Title 'Outils'
    if (-not (Get-Command git -ErrorAction SilentlyContinue)) { Install-Tool 'Git.Git' 'Git' @('--scope', 'machine') }
    if (-not (Get-Command gh -ErrorAction SilentlyContinue)) { Install-Tool 'GitHub.cli' 'GitHub CLI' @('--scope', 'machine') }
    # Rust on Windows links with the Visual Studio C++ tools (MSVC and the
    # Windows SDK): install them before Rust.
    if (-not (Get-MsvcName)) {
        Install-Tool 'Microsoft.VisualStudio.BuildTools' 'les outils C++ de Visual Studio' @(
            '--override', '--passive --wait --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended')
    }
    # rustup is per user by design: it updates itself and the toolchains
    # without admin rights (%USERPROFILE%\.cargo and .rustup).
    if (-not (Get-Command rustup -ErrorAction SilentlyContinue)) {
        Install-Tool 'Rustlang.Rustup' 'Rust (rustup)'
        $env:Path += ";$env:USERPROFILE\.cargo\bin"
        if (-not (Get-Command rustup -ErrorAction SilentlyContinue)) {
            throw 'rustup introuvable après son installation : ouvre une nouvelle fenêtre et relance.'
        }
    }
    Write-Color '  Git, GitHub CLI, outils C++ et Rust : OK.' success

    Write-Title 'Dépôt Aurora Viewer'
    New-Item -ItemType Directory -Force -Path $P.Root -ErrorAction Stop | Out-Null
    if (Test-Path (Join-Path $P.Repo '.git')) {
        $branch = (git -C $P.Repo rev-parse --abbrev-ref HEAD).Trim()
        if ($branch -eq 'main' -and -not (git -C $P.Repo status --porcelain --untracked-files=no)) {
            git -C $P.Repo pull --ff-only --quiet
            if ($LASTEXITCODE -ne 0) { Write-Color '  main pas mis à jour (git pull a échoué) : laissé tel quel.' warn }
            else { Write-Color "  $($P.Repo) : à jour." success }
        } else {
            Write-Color "  Sur '$branch' ou avec des changements en cours : laissé tel quel." warn
        }
    } else {
        git clone $RepoUrl $P.Repo
        if ($LASTEXITCODE -ne 0) { throw 'clonage du dépôt.' }
    }
    New-Item -ItemType Directory -Force -Path $P.Work | Out-Null

    Write-Title 'Sources de Firestorm (référence des agents)'
    if (Test-Path (Join-Path $P.Firestorm '.git')) {
        # a read-only reference copy, never edited: take the latest commit as is
        git -C $P.Firestorm fetch --depth 1 --quiet origin HEAD
        if ($LASTEXITCODE -eq 0) { git -C $P.Firestorm reset --hard --quiet FETCH_HEAD }
        if ($LASTEXITCODE -ne 0) { Write-Color '  Pas mises à jour : laissées telles quelles.' warn }
        else { Write-Color "  $($P.Firestorm) : à jour." success }
    } else {
        # latest commit only; long paths: some Firestorm files go past 260 characters
        git clone --depth 1 -c core.longpaths=true $FirestormUrl $P.Firestorm
        if ($LASTEXITCODE -ne 0) { throw 'clonage des sources de Firestorm.' }
    }

    Write-Title 'Police emoji'
    & (Join-Path $P.Repo 'scripts\fetch-assets.ps1')

    Write-Title 'Toolchain Rust et crates'
    Push-Location $P.Repo
    try {
        # rust-toolchain.toml: the latest stable, with rustfmt and clippy
        rustup toolchain install
        if ($LASTEXITCODE -ne 0) { rustup show }
        rustup update stable --no-self-update
        cargo fetch --locked
        if ($LASTEXITCODE -ne 0) { throw 'téléchargement des crates (cargo fetch).' }
    } finally {
        Pop-Location
    }

    Write-Title 'GitHub'
    gh auth status *> $null
    if ($LASTEXITCODE -ne 0) {
        Write-Color '  Connexion à GitHub (pour les PR des agents) : suis les instructions.' ink
        gh auth login --web --git-protocol https
    }
    gh auth status *> $null
    if ($LASTEXITCODE -eq 0) {
        # commits need an identity: GitHub's "noreply" address keeps the e-mail private
        if (-not (git -C $P.Repo config user.email)) {
            $user = gh api user | ConvertFrom-Json
            git -C $P.Repo config user.name $user.login
            git -C $P.Repo config user.email "$($user.id)+$($user.login)@users.noreply.github.com"
        }
        Write-Color "  Identité git : $(git -C $P.Repo config user.name) <$(git -C $P.Repo config user.email)>" success
    } else {
        Write-Color '  Pas connecté à GitHub : relance la réparation plus tard (nécessaire pour les PR).' warn
    }
}
