<#
.SYNOPSIS
    Installe l'environnement de développement d'Aurora Viewer : les outils qui
    manquent (Git, GitHub CLI, outils C++ de Visual Studio, Rust), le dépôt,
    les sources de Firestorm, la police emoji, la toolchain Rust et toutes les
    crates. Montre d'abord ce qu'il a trouvé et ce qu'il va faire, puis
    demande où installer. Relançable sans risque : ce qui est déjà là est mis
    à jour, rien n'est réinstallé.

.DESCRIPTION
    En une ligne, dans une invite de commandes :

      powershell -NoProfile -ExecutionPolicy Bypass -Command "$f = Join-Path $env:TEMP 'aurora-setup.ps1'; irm https://raw.githubusercontent.com/Aurora-Viewer/Aurora-Viewer/main/scripts/setup.ps1 -OutFile $f; & $f"

    Résultat, dans le dossier du projet :
      aurora-viewer\       le dépôt
      phoenix-firestorm\   sources de Firestorm (référence, lecture seule)

    Lancé depuis un clone (aurora-viewer\scripts\setup.ps1), il propose le
    dossier parent du dépôt.

.EXAMPLE
    ./scripts/setup.ps1 -Path D:\Dev\Aurora-Viewer -Yes   # sans question
#>
param(
    # project folder (created when missing); asked for when not given
    [string]$Path,
    # no questions: install in -Path, or in the proposed folder
    [switch]$Yes
)
$ErrorActionPreference = 'Continue'
$repoUrl = 'https://github.com/Aurora-Viewer/Aurora-Viewer'
$firestormUrl = 'https://github.com/FirestormViewer/phoenix-firestorm'

# --- Aurora palette (docs/BRANDING.md) ---------------------------------------
# True colours when the terminal understands them, the 16 console colours
# otherwise (and nothing but text when the output is redirected).
$vt = $Host.UI.SupportsVirtualTerminal -and -not [Console]::IsOutputRedirected
$palette = @{
    violet = '8B5CF6'; indigo = '4F46E5'; teal = '5EEAD4'; violet_light = 'A78BFA'
    ink = 'E8EDFB'; muted = '9AA6C6'; muted_dim = '6F7BA0'
    success = '4ADE80'; warn = 'FB923C'; danger = 'F87171'
}
$basic = @{
    violet = 'Magenta'; indigo = 'Blue'; teal = 'Cyan'; violet_light = 'Magenta'
    ink = 'White'; muted = 'Gray'; muted_dim = 'DarkGray'
    success = 'Green'; warn = 'Yellow'; danger = 'Red'
}
$esc = [char]27

function Get-Ansi([string]$Hex) {
    $r = [Convert]::ToInt32($Hex.Substring(0, 2), 16)
    $g = [Convert]::ToInt32($Hex.Substring(2, 2), 16)
    $b = [Convert]::ToInt32($Hex.Substring(4, 2), 16)
    "$esc[38;2;$r;$g;${b}m"
}

function Write-Color([string]$Text, [string]$Color = 'ink', [switch]$NoNewline) {
    if ($vt) {
        Write-Host "$(Get-Ansi $palette[$Color])$Text$esc[0m" -NoNewline:$NoNewline
    } else {
        Write-Host $Text -ForegroundColor $basic[$Color] -NoNewline:$NoNewline
    }
}

# The aurora of the logo: violet, then indigo, then teal, across the text.
function Write-Aurora([string]$Text) {
    if (-not $vt) { Write-Host $Text -ForegroundColor Magenta; return }
    $stops = @(@(139, 92, 246), @(79, 70, 229), @(94, 234, 212))
    $out = ''
    for ($i = 0; $i -lt $Text.Length; $i++) {
        $t = if ($Text.Length -gt 1) { $i / ($Text.Length - 1) } else { 0 }
        $seg = [Math]::Min([int][Math]::Floor($t * 2), 1)
        $u = $t * 2 - $seg
        $a = $stops[$seg]; $b = $stops[$seg + 1]
        $c = 0..2 | ForEach-Object { [int]($a[$_] + ($b[$_] - $a[$_]) * $u) }
        $out += "$esc[38;2;$($c[0]);$($c[1]);$($c[2])m$($Text[$i])"
    }
    Write-Host "$out$esc[0m"
}

function Show-Banner {
    if (-not $Yes -and -not [Console]::IsOutputRedirected) { Clear-Host }
    Write-Host ''
    Write-Aurora '    ▄▀█ █ █ █▀█ █▀█ █▀█ ▄▀█'
    Write-Aurora '    █▀█ █▄█ █▀▄ █▄█ █▀▄ █▀█'
    Write-Color  '    V  I  E  W  E  R' violet_light
    Write-Host ''
    Write-Color  '    Installation de l''environnement de développement' ink
    Write-Color  '    Outils, dépôt, sources de Firestorm, police emoji, Rust et crates.' muted
    Write-Host ''
}

function Write-Step([string]$Text) {
    Write-Host ''
    Write-Color '  » ' violet -NoNewline
    Write-Color $Text violet_light
}

function Stop-Setup([string]$Text) {
    Write-Host ''
    Write-Color "  ÉCHEC : $Text" danger
    Write-Color '  Corrige le problème, puis relance la même commande : ce qui est fait est gardé.' muted
    exit 1
}

# --- Checks -----------------------------------------------------------------

function New-Check([string]$Item, [string]$Found, [string]$Todo, [string]$Done = 'rien à faire', [switch]$Always) {
    [pscustomobject]@{
        Item = $Item
        Found = $Found
        State = if ($Always) { 'always' } elseif ($Found) { 'ok' } else { 'todo' }
        Action = if ($Always -or -not $Found) { $Todo } else { $Done }
    }
}

function Get-Paths([string]$Root) {
    [pscustomobject]@{
        Root = $Root
        Repo = Join-Path $Root 'aurora-viewer'
        Firestorm = Join-Path $Root 'phoenix-firestorm'
    }
}

function Get-Checks($P) {
    $git = if (Get-Command git -ErrorAction SilentlyContinue) { (git --version) -replace '^git version\s+', '' -replace '\.windows.*$', '' }
    $gh = if (Get-Command gh -ErrorAction SilentlyContinue) { (gh --version | Select-Object -First 1) -replace '^gh version\s+(\S+).*$', '$1' }
    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
    $msvc = if (Test-Path $vswhere) {
        & $vswhere -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property displayName | Select-Object -First 1
    }
    $rust = if (Get-Command rustup -ErrorAction SilentlyContinue) { (rustup --version 2>$null) -replace '^rustup\s+(\S+).*$', 'rustup $1' }

    $repoBranch = if (Test-Path (Join-Path $P.Repo '.git')) { "présent ($((git -C $P.Repo rev-parse --abbrev-ref HEAD).Trim()))" }
    $firestorm = if (Test-Path (Join-Path $P.Firestorm '.git')) { 'présentes' }
    $font = if (Test-Path (Join-Path $P.Repo 'assets\emoji\Noto-3D-128.ttf')) { 'présente' }
    $login = $null
    if ($gh) {
        gh auth status *> $null
        if ($LASTEXITCODE -eq 0) { $login = "connecté : $(gh api user --jq .login 2>$null)" }
    }
    $identity = if ($git) {
        $email = if (Test-Path (Join-Path $P.Repo '.git')) { git -C $P.Repo config user.email } else { git config --global user.email }
        if ($email) { $email }
    }

    @(
        New-Check 'Git' $git 'installé avec winget, pour toute la machine'
        New-Check 'GitHub CLI' $gh 'installé avec winget, pour toute la machine'
        New-Check 'Outils C++ de Visual Studio' $msvc 'installés avec winget (compilateur et SDK Windows, plusieurs Go)'
        New-Check 'Rust' $rust 'installé avec winget (rustup, pour ton compte)'
        New-Check 'Dépôt aurora-viewer' $repoBranch 'cloné depuis GitHub' 'mis à jour s''il est sur main sans changement'
        New-Check 'Sources de Firestorm' $firestorm 'clonées (dernier commit seulement)' 'mises à jour'
        New-Check 'Police emoji' $font 'téléchargée (142 Mo)'
        New-Check 'Toolchain Rust et crates' $null 'installées ou mises à jour (cargo fetch)' -Always
        New-Check 'Connexion à GitHub' $login 'gh auth login, dans le navigateur'
        New-Check 'Identité git' $identity 'ton pseudo GitHub + adresse noreply (e-mail privé)'
    )
}

# The plan (with what will be done) before installing; -Summary afterwards
# shows only what is there.
function Show-Checks($Checks, [string]$Title, [switch]$Summary) {
    $w1 = 30; $w2 = 34
    $rule = '  ' + ('─' * ($w1 + $w2 + $(if ($Summary) { 0 } else { 34 })))
    Write-Color "  $Title" ink
    Write-Color $rule muted_dim
    Write-Color ('  ' + 'Élément'.PadRight($w1) + 'Trouvé'.PadRight($w2) + $(if (-not $Summary) { 'Ce que je fais' })) muted
    Write-Color $rule muted_dim
    foreach ($c in $Checks) {
        if ($Summary -and $c.State -eq 'always') { continue }
        $found = switch ($c.State) { 'ok' { "• $($c.Found)" } 'todo' { '• absent' } default { '-' } }
        if ($found.Length -gt $w2 - 2) { $found = $found.Substring(0, $w2 - 3) + '…' }
        $color = switch ($c.State) { 'ok' { 'success' } 'todo' { 'warn' } default { 'muted_dim' } }
        Write-Color ('  ' + $c.Item.PadRight($w1)) ink -NoNewline
        if ($Summary) {
            Write-Color $found $color
        } else {
            Write-Color $found.PadRight($w2) $color -NoNewline
            Write-Color $c.Action $(if ($c.State -eq 'ok') { 'muted_dim' } else { 'ink' })
        }
    }
    Write-Color $rule muted_dim
}

function Read-YesNo([string]$Question) {
    Write-Host ''
    Write-Color "  $Question " ink -NoNewline
    Write-Color '[O/n] ' violet_light -NoNewline
    $a = Read-Host
    -not ($a -match '^\s*(n|non|no)\s*$')
}

# Tools installed by winget land in PATH for new sessions only: reload it here.
function Update-SessionPath {
    $env:Path = [Environment]::GetEnvironmentVariable('Path', 'Machine') + ';' +
        [Environment]::GetEnvironmentVariable('Path', 'User')
}

function Install-Tool([string]$Id, [string]$What, [string[]]$Extra = @()) {
    if (-not (Get-Command winget -ErrorAction SilentlyContinue)) {
        Stop-Setup "$What manque, et winget (le gestionnaire de paquets de Windows) n'est pas disponible : installe-le à la main."
    }
    Write-Color "  Installation de $What (une fenêtre d'autorisation Windows peut s'ouvrir)…" muted
    winget install --id $Id --exact --source winget --accept-package-agreements --accept-source-agreements @Extra
    if ($LASTEXITCODE -ne 0) { Stop-Setup "installation de $What (code $LASTEXITCODE)." }
    Update-SessionPath
}

# --- Where ------------------------------------------------------------------

Show-Banner

# Proposed folder: the parent of the clone this script runs from, or the
# current folder (the one-liner runs a copy saved in %TEMP%).
$fromClone = if ($PSScriptRoot) { Split-Path $PSScriptRoot -Parent }
$proposed = if ($fromClone -and (Test-Path (Join-Path $fromClone 'AGENTS.md')) -and (Test-Path (Join-Path $fromClone 'Cargo.toml'))) {
    Split-Path $fromClone -Parent
} else {
    (Get-Location).Path
}

if ($Path) {
    $root = [IO.Path]::GetFullPath($Path)
    Show-Checks (Get-Checks (Get-Paths $root)) "Dossier du projet : $root"
} else {
    $root = $proposed
    Show-Checks (Get-Checks (Get-Paths $root)) "Dossier du projet proposé : $root"
    if (-not $Yes -and -not (Read-YesNo "Tout installer dans $root ?")) {
        do {
            Write-Host ''
            Write-Color '  Chemin du dossier Aurora-Viewer (créé s''il n''existe pas), par exemple D:\Dev\Aurora-Viewer :' ink
            Write-Color '  > ' violet -NoNewline
            $answer = (Read-Host).Trim().Trim('"')
            $root = if ($answer) { [IO.Path]::GetFullPath($answer) }
            if ($root -and (Test-Path -LiteralPath $root -PathType Leaf)) {
                Write-Color "  $root est un fichier, pas un dossier." warn
                $root = $null
            }
        } until ($root)
        Write-Host ''
        Show-Checks (Get-Checks (Get-Paths $root)) "Dossier du projet : $root"
        if (-not (Read-YesNo "Tout installer dans $root ?")) {
            Write-Color '  Rien n''a été installé.' muted
            exit 0
        }
    }
}
$p = Get-Paths $root
New-Item -ItemType Directory -Force -Path $root -ErrorAction Stop | Out-Null
$started = Get-Date

# --- Tools ------------------------------------------------------------------

Write-Step 'Outils'
if (-not (Get-Command git -ErrorAction SilentlyContinue)) {
    Install-Tool 'Git.Git' 'Git' @('--scope', 'machine')
}
if (-not (Get-Command gh -ErrorAction SilentlyContinue)) {
    Install-Tool 'GitHub.cli' 'GitHub CLI' @('--scope', 'machine')
}
# Rust on Windows links with the Visual Studio C++ tools (MSVC and the
# Windows SDK): install them before Rust.
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
$msvc = (Test-Path $vswhere) -and (& $vswhere -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath)
if (-not $msvc) {
    Install-Tool 'Microsoft.VisualStudio.BuildTools' 'les outils C++ de Visual Studio' @(
        '--override', '--passive --wait --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended')
}
# rustup is per user by design: it updates itself and the toolchains without
# admin rights (%USERPROFILE%\.cargo and .rustup).
if (-not (Get-Command rustup -ErrorAction SilentlyContinue)) {
    Install-Tool 'Rustlang.Rustup' 'Rust (rustup)'
    $env:Path += ";$env:USERPROFILE\.cargo\bin"
    if (-not (Get-Command rustup -ErrorAction SilentlyContinue)) {
        Stop-Setup 'rustup introuvable après son installation : ouvre une nouvelle invite de commandes et relance.'
    }
}
Write-Color '  Git, GitHub CLI, outils C++ et Rust : OK.' success

# --- Repositories -------------------------------------------------------------

Write-Step 'Dépôt Aurora Viewer'
if (Test-Path (Join-Path $p.Repo '.git')) {
    $branch = (git -C $p.Repo rev-parse --abbrev-ref HEAD).Trim()
    if ($branch -eq 'main' -and -not (git -C $p.Repo status --porcelain)) {
        git -C $p.Repo pull --ff-only --quiet
        if ($LASTEXITCODE -ne 0) { Write-Color '  main pas mis à jour (git pull a échoué) : laissé tel quel.' warn }
        else { Write-Color "  $($p.Repo) : à jour." success }
    } else {
        Write-Color "  Sur '$branch' ou avec des changements en cours : laissé tel quel." warn
    }
} else {
    git clone $repoUrl $p.Repo
    if ($LASTEXITCODE -ne 0) { Stop-Setup 'clonage du dépôt.' }
}

Write-Step 'Sources de Firestorm (référence des agents)'
if (Test-Path (Join-Path $p.Firestorm '.git')) {
    # a read-only reference copy, never edited: take the latest commit as is
    git -C $p.Firestorm fetch --depth 1 --quiet origin HEAD
    if ($LASTEXITCODE -eq 0) { git -C $p.Firestorm reset --hard --quiet FETCH_HEAD }
    if ($LASTEXITCODE -ne 0) { Write-Color '  Pas mises à jour : laissées telles quelles.' warn }
    else { Write-Color "  $($p.Firestorm) : à jour." success }
} else {
    # latest commit only; long paths: some Firestorm files go past 260 characters
    git clone --depth 1 -c core.longpaths=true $firestormUrl $p.Firestorm
    if ($LASTEXITCODE -ne 0) { Stop-Setup 'clonage des sources de Firestorm.' }
}

Write-Step 'Police emoji'
try {
    & (Join-Path $p.Repo 'scripts\fetch-assets.ps1')
} catch {
    Stop-Setup "police emoji : $($_.Exception.Message)"
}

Write-Step 'Toolchain Rust et crates'
Push-Location $p.Repo
try {
    # rust-toolchain.toml: the latest stable, with rustfmt and clippy
    rustup toolchain install
    if ($LASTEXITCODE -ne 0) { rustup show }
    rustup update stable --no-self-update
    cargo fetch --locked
    if ($LASTEXITCODE -ne 0) { Stop-Setup 'téléchargement des crates (cargo fetch).' }
} finally {
    Pop-Location
}

# --- GitHub -------------------------------------------------------------------

Write-Step 'GitHub'
gh auth status *> $null
if ($LASTEXITCODE -ne 0) {
    Write-Color '  Connexion à GitHub (pour les PR des agents) : suis les instructions.' ink
    gh auth login --web --git-protocol https
}
gh auth status *> $null
if ($LASTEXITCODE -eq 0) {
    # commits need an identity: GitHub's "noreply" address keeps the e-mail private
    if (-not (git -C $p.Repo config user.email)) {
        $user = gh api user | ConvertFrom-Json
        git -C $p.Repo config user.name $user.login
        git -C $p.Repo config user.email "$($user.id)+$($user.login)@users.noreply.github.com"
    }
    Write-Color "  Identité git : $(git -C $p.Repo config user.name) <$(git -C $p.Repo config user.email)>" success
} else {
    Write-Color '  Pas connecté à GitHub : lance « gh auth login » plus tard (nécessaire pour les PR).' warn
}

# --- Summary ------------------------------------------------------------------

Write-Host ''
Show-Checks (Get-Checks $p) 'Récapitulatif' -Summary
$took = (Get-Date) - $started
Write-Host ''
Write-Aurora "  » Tout est prêt, en $([math]::Floor($took.TotalMinutes)) min $($took.Seconds) s."
Write-Color  "    $($p.Repo)" ink
Write-Color  '    Ouvre ton agent (Claude Code…) dans ce dossier ; ton rôle est expliqué dans HUMANS.md.' muted
Write-Host ''
