# Aurora Tools: reset the repository to the state of a fresh clone of
# GitHub's main (dot-sourced by aurora-tools.ps1). What would be lost is
# listed first; nothing happens without typing the confirmation word.
# $DryRun (aurora-tools.ps1 -DryRun) stops after the list.
#
# The folder itself is kept: the tools run from it, and Windows does not
# delete a folder a program has open. A hard reset plus a clean of every
# untracked and ignored file leaves exactly what a clone holds.

$ResetWord = 'REINITIALISER'

# Commits of $Branch that exist nowhere on GitHub: ahead of its upstream, or
# not in origin/main when it was never pushed. A branch whose upstream was
# deleted on GitHub (merged PR) is safe.
function Get-UnpushedCount([string]$Repo, [string]$Branch, [string]$Upstream) {
    if ($Upstream) {
        git -C $Repo rev-parse --verify --quiet "refs/remotes/$Upstream" *> $null
        if ($LASTEXITCODE -ne 0) { return 0 }
        return [int](git -C $Repo rev-list --count "$Upstream..$Branch" 2>$null)
    }
    [int](git -C $Repo rev-list --count "origin/main..$Branch" 2>$null)
}

function Get-ResetInventory {
    $repo = $Paths.Repo
    git -C $repo fetch origin --prune --quiet 2>$null
    $changes = @(git -C $repo status --porcelain 2>$null)
    $branches = @(foreach ($line in git -C $repo for-each-ref --format='%(refname:short)|%(upstream:short)' refs/heads) {
        $name, $up = $line -split '\|', 2
        $n = Get-UnpushedCount $repo $name $up
        if ($n -gt 0) { [pscustomobject]@{ Branch = $name; Count = $n; Pushed = [bool]$up } }
    })
    $stashes = @(git -C $repo stash list 2>$null)
    # ignored files that are not builds or fetched assets (.env, editor settings…)
    $ignored = @(git -C $repo clean -ndX 2>$null | ForEach-Object { $_ -replace '^Would remove ', '' } |
        Where-Object { $_ -notmatch '^(target/|\.scratch/|assets/emoji/)' })
    $worktrees = @()
    $current = $null
    foreach ($line in git -C $repo worktree list --porcelain) {
        if ($line -like 'worktree *') { $current = [pscustomobject]@{ Path = ($line.Substring(9) -replace '/', '\'); Branch = $null } ; $worktrees += $current }
        elseif ($line -like 'branch *' -and $current) { $current.Branch = $line.Substring(7) -replace '^refs/heads/', '' }
    }
    $worktrees = @($worktrees | Where-Object { [IO.Path]::GetFullPath($_.Path).TrimEnd('\') -ne [IO.Path]::GetFullPath($repo).TrimEnd('\') } | ForEach-Object {
        $up = if ($_.Branch) { (git -C $repo for-each-ref --format='%(upstream:short)' "refs/heads/$($_.Branch)") }
        [pscustomobject]@{
            Path = $_.Path
            Name = Split-Path $_.Path -Leaf
            Branch = $_.Branch
            Changes = @(git -C $_.Path status --porcelain 2>$null).Count
            Unpushed = if ($_.Branch) { Get-UnpushedCount $repo $_.Branch $up } else { 0 }
        }
    })
    [pscustomobject]@{ Changes = $changes; Branches = $branches; Stashes = $stashes; Ignored = $ignored; Worktrees = $worktrees }
}

# $true when something would be lost.
function Show-ResetInventory($Inv) {
    $lost = $false
    Write-Rule 90
    Write-Color '  Ce qui serait perdu' ink
    Write-Rule 90
    if ($Inv.Changes) {
        $lost = $true
        Write-Color "  • $($Inv.Changes.Count) changement(s) non commité(s) dans le dépôt :" warn
        foreach ($c in $Inv.Changes | Select-Object -First 8) { Write-Color "      $c" muted }
        if ($Inv.Changes.Count -gt 8) { Write-Color "      … et $($Inv.Changes.Count - 8) autres" muted_dim }
    }
    foreach ($b in $Inv.Branches) {
        $lost = $true
        Write-Color "  • branche $($b.Branch) : $($b.Count) commit(s) $(if ($b.Pushed) { 'pas encore poussés' } else { 'jamais poussés sur GitHub' })" warn
    }
    if ($Inv.Stashes) {
        $lost = $true
        Write-Color "  • $($Inv.Stashes.Count) stash(s) (de côté avec git stash)" warn
    }
    foreach ($w in $Inv.Worktrees) {
        $risk = @()
        if ($w.Changes) { $risk += "$($w.Changes) changement(s) non commité(s)" }
        if ($w.Unpushed) { $risk += "$($w.Unpushed) commit(s) pas sur GitHub" }
        if ($risk) {
            $lost = $true
            Write-Color "  • tâche $($w.Name) ($($w.Branch)) : $($risk -join ', ')" warn
        } else {
            Write-Color "  • tâche $($w.Name) : rien de perdu (tout est sur GitHub), dossier supprimé" muted
        }
    }
    if ($Inv.Ignored) {
        $lost = $true
        Write-Color "  • fichiers ignorés par git, hors compilations :" warn
        foreach ($f in $Inv.Ignored | Select-Object -First 8) { Write-Color "      $f" muted }
        if ($Inv.Ignored.Count -gt 8) { Write-Color "      … et $($Inv.Ignored.Count - 8) autres" muted_dim }
    }
    if (-not $lost) { Write-Color '  Rien : tout ton travail est déjà sur GitHub.' success }
    Write-Color '  Effacés aussi, mais refaits ensuite : compilations (target), police emoji, dossier RELEASE.' muted_dim
    Write-Color '  Gardés : ton identité git, ta connexion GitHub, les sources de Firestorm (mises à jour).' muted_dim
    Write-Rule 90
    $lost
}

function Invoke-Reset {
    Write-Title 'Réinitialiser le dépôt'
    Write-Color "  Remet $($Paths.Repo) dans l'état d'un clone neuf du main de GitHub," ink
    Write-Color '  supprime les tâches des agents (work\) et RELEASE\, puis répare l''environnement.' ink
    Write-Color '  Inventaire (GitHub, branches, tâches)…' muted_dim
    $inv = Get-ResetInventory
    $lost = Show-ResetInventory $inv
    # Test-InUse (scripts\common.ps1): a build or a viewer running in the project
    . (Join-Path $Paths.Repo 'scripts\common.ps1')
    if (Test-InUse $Paths.Root) {
        Write-Color '  Une compilation ou un viewer tourne dans le projet : ferme-le, puis recommence.' danger
        Wait-Back; return
    }
    $typed = Read-Text "$(if ($lost) { 'Ce qui est listé plus haut sera perdu. ' })Pour confirmer, tape $ResetWord (vide : annuler) :"
    if ($typed -cne $ResetWord) { Write-Color '  Annulé : rien n''a été touché.' muted; Wait-Back; return }
    if ($DryRun) {
        Write-Color '  [simulation] git worktree remove --force (chaque tâche, RELEASE\src) ; dossiers work\* et RELEASE\ supprimés' teal
        Write-Color '  [simulation] git checkout -B main origin/main ; git reset --hard origin/main ; git clean -ffdx ; branches locales et stashs supprimés' teal
        Write-Color '  [simulation] réparation (police, toolchain, crates, Firestorm)' teal
        Wait-Back; return
    }

    $repo = $Paths.Repo
    Write-Title 'Tâches des agents et RELEASE\'
    foreach ($w in $inv.Worktrees) {
        git -C $repo worktree remove --force $w.Path 2>$null
        if (Test-Path -LiteralPath $w.Path) { Remove-Item -LiteralPath $w.Path -Recurse -Force -ErrorAction SilentlyContinue }
        if (Test-Path -LiteralPath $w.Path) { Write-Color "  $($w.Name) : dossier verrouillé (un programme l'utilise), à supprimer plus tard." warn }
        else { Write-Color "  $($w.Name) : supprimé." success }
    }
    git -C $repo worktree prune
    foreach ($dir in @(Get-ChildItem -LiteralPath $Paths.Work -Directory -Force -ErrorAction SilentlyContinue) + @(Get-Item -LiteralPath $Paths.Release -ErrorAction SilentlyContinue)) {
        if ($dir) { Remove-Item -LiteralPath $dir.FullName -Recurse -Force -ErrorAction SilentlyContinue }
    }

    Write-Title 'Dépôt : clone neuf du main de GitHub'
    git -C $repo checkout --force --quiet -B main origin/main
    git -C $repo reset --hard --quiet origin/main
    git -C $repo clean -ffdxq
    git -C $repo branch --quiet --set-upstream-to origin/main main 2>$null
    foreach ($b in git -C $repo for-each-ref --format='%(refname:short)' refs/heads) {
        if ($b -ne 'main') { git -C $repo branch -D --quiet $b }
    }
    git -C $repo stash clear
    $left = @(git -C $repo status --porcelain --ignored)
    if ($left) { Write-Color "  $($left.Count) fichier(s) n'ont pas pu être effacés (verrouillés) : $($left[0])…" warn }
    else { Write-Color "  $repo : identique à origin/main." success }

    Invoke-Repair $Paths
    Write-Host ''
    Write-Aurora '  » Dépôt réinitialisé.'
    Write-Color '    Ferme et relance aurora-tools.cmd pour utiliser les outils de ce main.' muted
    Wait-Back
}
