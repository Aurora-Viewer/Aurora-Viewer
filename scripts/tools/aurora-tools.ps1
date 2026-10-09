<#
.SYNOPSIS
    Les outils de développement d'Aurora Viewer, en menu au clavier
    (flèches, Entrée, Échap) : diagnostic et réparation de l'environnement,
    viewers release / dev / debug, démo, tâches des agents, disque, logs,
    suivi des PR en direct (bandeau du bas, notifications), gestion des
    releases, raccourcis, réinitialisation du dépôt. Se lance par aurora-tools.cmd, à la racine du dépôt.

.DESCRIPTION
    Sans dépôt (première installation : la commande en une ligne lance
    scripts/setup.ps1, qui télécharge ces outils), il installe tout dans le
    dossier choisi, puis ouvre le menu du dépôt installé.

.EXAMPLE
    ./scripts/tools/aurora-tools.ps1                     # le menu
    ./scripts/tools/aurora-tools.ps1 -Action diagnose    # une action, sans menu
#>
param(
    [ValidateSet('menu', 'install', 'diagnose', 'repair', 'release', 'dev', 'debug', 'demo', 'tasks', 'disk', 'logs', 'shortcut', 'prs', 'releases', 'reset')]
    [string]$Action = 'menu',
    # project folder for -Action install (asked for when not given)
    [string]$Path,
    # -Action install without questions
    [switch]$Yes,
    # releases, reset: show what would be written or erased instead of doing it
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
    Write-Color "  ÉCHEC : $($Err.Exception.Message)" danger
    Write-Color '  Corrige le problème puis relance : ce qui est fait est gardé.' muted
}

# --- Installation ---------------------------------------------------------------

function Start-Install {
    Show-Logo 'Installation de l''environnement de développement'
    $here = (Get-Location).Path
    # in the repository itself: propose its parent (no clone inside a clone)
    $proposed = if (Test-Path (Join-Path $here 'AGENTS.md')) { Split-Path $here -Parent } else { $here }
    if ($Path) {
        $root = [IO.Path]::GetFullPath($Path)
        Show-Checks (Get-Checks (Get-Paths $root)) "Dossier du projet : $root"
    } else {
        $root = $proposed
        Show-Checks (Get-Checks (Get-Paths $root)) "Dossier du projet proposé : $root"
        if (-not $Yes -and -not (Read-YesNo "Tout installer dans $root ?")) {
            do {
                $answer = Read-Text 'Chemin du dossier Aurora-Viewer (créé s''il n''existe pas), par exemple D:\Dev\Aurora-Viewer ; vide : annuler'
                if (-not $answer) { Write-Color '  Rien n''a été installé.' muted; exit 0 }
                $root = [IO.Path]::GetFullPath($answer)
                if (Test-Path -LiteralPath $root -PathType Leaf) { Write-Color "  $root est un fichier, pas un dossier." warn; $root = $null }
            } until ($root)
            Write-Host ''
            Show-Checks (Get-Checks (Get-Paths $root)) "Dossier du projet : $root"
            if (-not (Read-YesNo "Tout installer dans $root ?")) { Write-Color '  Rien n''a été installé.' muted; exit 0 }
        }
    }
    $p = Get-Paths $root
    $started = Get-Date
    try { Invoke-Repair $p } catch { Show-Failure $_; exit 1 }
    Write-Host ''
    Show-Checks (Get-Checks $p) 'Récapitulatif' -Summary
    $took = (Get-Date) - $started
    Write-Host ''
    Write-Aurora "  » Tout est prêt, en $([math]::Floor($took.TotalMinutes)) min $($took.Seconds) s."
    Write-Color "    $($p.Repo)" ink
    Write-Color '    Ouvre ton agent (Claude Code…) dans ce dossier ; ton rôle est expliqué dans HUMANS.md.' muted
    $tools = Join-Path $p.Repo 'scripts\tools\aurora-tools.ps1'
    if (-not $Yes -and (Test-Path $tools) -and (Read-YesNo 'Ouvrir les outils Aurora (aurora-tools.cmd) ?')) {
        & $tools
    }
}

# --- Menu -------------------------------------------------------------------------

function Show-Status($Checks) {
    $problems = Get-Problems $Checks
    Write-Color "  Projet : $($Paths.Root)" muted
    if (-not $problems) {
        Write-Color '  Environnement : tout est en place et à jour.' success
    } else {
        Write-Color "  Environnement : $($problems.Count) point(s) à réparer ou mettre à jour" warn
        foreach ($c in $problems) { Write-Color "    • $($c.Item) : $(if ($c.State -eq 'todo') { 'absent' } else { $c.Found })" warn }
    }
    foreach ($c in $Checks | Where-Object { $_.Id -eq 'release' -and $_.Action -ne 'rien à faire' }) {
        Write-Color "  $($c.Item) : $($c.Found)" teal
    }
}

function Show-Main {
    $checks = $null
    $selected = 0
    while ($true) {
        if (-not $checks) {
            Show-Logo 'Outils'
            Write-Color '  Vérification de l''environnement (GitHub, Firestorm, Rust)…' muted_dim
            $checks = Get-Checks $Paths
        }
        $problems = Get-Problems $checks
        $items = @()
        if ($problems) { $items += @{ Key = 'r'; Label = "Réparer / mettre à jour ($($problems.Count))"; Hint = 'installe ce qui manque, met à jour le reste' } }
        $items += @(
            @{ Key = '1'; Label = 'Viewer release'; Hint = 'dernier main de GitHub, --release, dans RELEASE\' }
            @{ Key = '2'; Label = 'Viewer de dev'; Hint = 'ton dépôt local, profil par défaut' }
            @{ Key = '3'; Label = 'Viewer de debug'; Hint = 'avec console et contrôles de débogage' }
            @{ Key = '4'; Label = 'Démo'; Hint = 'hors ligne, choix du scénario' }
            @{ Key = '5'; Label = 'Tâches des agents'; Hint = 'dossiers de work\, PR, ménage' }
            @{ Key = '6'; Label = 'Disque'; Hint = 'place des compilations, nettoyage' }
            @{ Key = '7'; Label = 'Logs du viewer'; Hint = 'ouvrir, lire la fin, copier le chemin' }
            @{ Key = 'p'; Label = 'Suivi des PR'; Hint = 'en direct, à garder dans un coin de l''écran' }
            @{ Key = 'g'; Label = 'Gérer les releases'; Hint = 'créer ou supprimer une version' }
            @{ Key = '8'; Label = 'Raccourcis'; Hint = 'Bureau et menu Démarrer' }
            @{ Key = 'v'; Label = 'Vérifier à nouveau'; Hint = 'le tableau complet de l''environnement' }
            @{ Key = 'x'; Label = 'Réinitialiser le dépôt'; Hint = 'clone neuf de GitHub, efface tout le local (inventaire d''abord)' }
            @{ Key = '0'; Label = 'Quitter'; Hint = '' }
        )
        $pick = Show-Menu 'Que veux-tu faire ?' $items { Show-Logo 'Outils'; Show-Status $checks } $selected
        if (-not $pick -or $pick.Key -eq '0') { Clear-Screen; return }
        $selected = [array]::IndexOf(@($items | ForEach-Object { $_.Key }), $pick.Key)
        Show-Logo $pick.Label
        try {
            switch ($pick.Key) {
                'r' { Invoke-Repair $Paths; $checks = $null; Wait-Back }
                '1' { Invoke-ReleaseViewer; $checks = $null; Wait-Back }
                '2' { Invoke-DevViewer; Wait-Back }
                '3' { Invoke-DebugViewer; Wait-Back }
                '4' { if (Invoke-Demo) { Wait-Back } }
                '5' { Invoke-Tasks }
                '6' { Invoke-Disk }
                '7' { Invoke-Logs }
                'p' { Show-PrPanel }
                'g' { Invoke-Releases }
                '8' { Invoke-Shortcut }
                'x' { Invoke-Reset; $checks = $null }
                'v' { $checks = Get-Checks $Paths; Show-Checks $checks "Environnement : $($Paths.Root)"; Wait-Back }
            }
        } catch {
            Show-Failure $_
            Wait-Back
        }
    }
}

# --- Entry ------------------------------------------------------------------------

$Paths = Find-Project
if ($Action -eq 'install' -or -not $Paths) { Start-Install; exit }
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
    'diagnose' { Show-Checks (Get-Checks $Paths) "Environnement : $($Paths.Root)" }
    'repair' { try { Invoke-Repair $Paths } catch { Show-Failure $_; exit 1 } }
    'release' { Invoke-ReleaseViewer }
    'dev' { Invoke-DevViewer }
    'debug' { Invoke-DebugViewer }
    'demo' { [void](Invoke-Demo) }
    'tasks' { [void](Show-Tasks) }
    'disk' { Invoke-Disk }
    'logs' { Invoke-Logs }
    'shortcut' { Invoke-Shortcut }
}
Stop-GitHubWatch
