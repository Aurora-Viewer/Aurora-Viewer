# Aurora Tools: manage the GitHub releases — create one (a SemVer tag on the
# latest main: the Release workflow builds and publishes it) or delete one
# (dot-sourced by aurora-tools.ps1). Needs write access to the repository.
# $DryRun (aurora-tools.ps1 -DryRun) prints the GitHub writes instead.

# gh's JSON output as objects, one per array item: Windows PowerShell 5.1
# passes a whole JSON array down the pipeline as a single object, so
# @(gh … | ConvertFrom-Json) would count 1. Writing out the assigned value
# enumerates it.
function Invoke-GhJson([string[]]$Arguments) {
    $json = gh @Arguments 2>$null
    if ($LASTEXITCODE -ne 0 -or -not $json) { return }
    $items = ($json -join "`n") | ConvertFrom-Json
    $items
}

function Test-ReleaseRights($Repo) {
    $perm = gh api "repos/$($Repo.Slug)" --jq '.permissions' 2>$null | ConvertFrom-Json
    [bool]($perm -and $perm.push)
}

function Get-ReleaseList($Repo) {
    @(Invoke-GhJson @('release', 'list', '-R', $Repo.Slug, '--limit', '30', '--json', 'tagName,name,isPrerelease,isLatest,isDraft,publishedAt'))
}

# A version tag: vMAJOR.MINOR.PATCH, optionally -suffix (a pre-release).
function ConvertTo-Version([string]$Tag) {
    if ($Tag -match '^v?(\d+)\.(\d+)\.(\d+)(-[0-9A-Za-z.-]+)?$') {
        [pscustomobject]@{ Major = [int]$Matches[1]; Minor = [int]$Matches[2]; Patch = [int]$Matches[3]; Suffix = $Matches[4] }
    }
}

# The last stable version (no suffix), its date, and the PRs merged since.
function Get-ReleaseContext($Repo) {
    git -C $Paths.Repo fetch origin --tags --quiet 2>$null
    $last = git -C $Paths.Repo tag --list 'v*' | ForEach-Object { $v = ConvertTo-Version $_; if ($v -and -not $v.Suffix) { [pscustomobject]@{ Tag = $_; V = $v } } } |
        Sort-Object { $_.V.Major }, { $_.V.Minor }, { $_.V.Patch } | Select-Object -Last 1
    $since = if ($last) { (git -C $Paths.Repo log -1 --format=%cI $last.Tag).Trim() }
    $filter = if ($since) { @('--search', "merged:>$since") } else { @() }
    $prs = @(Invoke-GhJson (@('pr', 'list', '-R', $Repo.Slug, '--state', 'merged', '--limit', '200', '--json', 'number,title,labels') + $filter))
    [pscustomobject]@{ Last = $last; Since = $since; Prs = $prs }
}

function Get-PrType($Pr) {
    $names = @($Pr.labels | ForEach-Object name)
    foreach ($t in 'rupture', 'nouveauté', 'correctif', 'performance', 'maintenance', 'docs') { if ($names -contains $t) { return $t } }
    'autre'
}

function Show-ReleaseList($Releases) {
    Write-Rule 90
    Write-Color ('  ' + 'Tag'.PadRight(18) + 'Nom'.PadRight(40) + 'Publiée le'.PadRight(14) + 'Type') muted
    Write-Rule 90
    if (-not $Releases) { Write-Color '  Aucune release.' muted }
    foreach ($r in $Releases) {
        $kind = if ($r.tagName -notmatch '^v\d') { 'assets (protégée)' } elseif ($r.isDraft) { 'brouillon' } elseif ($r.isPrerelease) { 'pré-release' } elseif ($r.isLatest) { 'dernière' } else { 'release' }
        $name = if ($r.name.Length -gt 38) { $r.name.Substring(0, 37) + '…' } else { $r.name }
        Write-Color ('  ' + $r.tagName.PadRight(18)) violet_light -NoNewline
        Write-Color $name.PadRight(40) ink -NoNewline
        Write-Color ([datetime]$r.publishedAt).ToString('dd/MM/yyyy').PadRight(14) muted -NoNewline
        Write-Color $kind $(if ($kind -eq 'dernière') { 'success' } elseif ($kind -like 'assets*') { 'teal' } else { 'muted' })
    }
    Write-Rule 90
}

function Invoke-Releases {
    $repo = Get-RepoSlug
    if (-not $repo) { Write-Color '  Dépôt GitHub introuvable (remote origin).' danger; Wait-Back; return }
    while ($true) {
        Show-Logo 'Releases'
        Write-Color "  Dépôt : $($repo.Slug)" muted
        Write-Color '  Lecture des releases et de tes droits sur GitHub…' muted_dim
        $releases = Get-ReleaseList $repo
        $rights = Test-ReleaseRights $repo
        Show-Logo 'Releases'
        Write-Color "  Dépôt : $($repo.Slug)" muted
        Show-ReleaseList $releases
        if (-not $rights) {
            Write-Color '  Ton compte GitHub n''a pas le droit d''écriture sur ce dépôt : création et suppression indisponibles.' warn
            Wait-Back
            return
        }
        $pick = Show-Menu 'Gérer les releases' @(
            @{ Key = '1'; Label = 'Créer une release'; Hint = 'tag SemVer sur le dernier main, publiée par la CI' }
            @{ Key = '2'; Label = 'Supprimer une release'; Hint = 'release et tag, irréversible' }
        )
        switch ($pick.Key) {
            '1' { New-AuroraRelease $repo }
            '2' { Remove-AuroraRelease $repo $releases }
            default { return }
        }
    }
}

function New-AuroraRelease($Repo) {
    Show-Logo 'Nouvelle release'
    Write-Color '  Vérification de main et des PR fusionnées depuis la dernière version…' muted_dim
    $run = Invoke-GhJson @('run', 'list', '-R', $Repo.Slug, '--workflow', 'CI', '--branch', 'main', '--limit', '1', '--json', 'status,conclusion,displayTitle') | Select-Object -First 1
    if (-not $run -or $run.status -ne 'completed') {
        Write-Color '  La CI de main tourne encore : attends qu''elle finisse, puis recommence.' warn; Wait-Back; return
    }
    if ($run.conclusion -ne 'success') {
        Write-Color "  La CI de main est rouge ($($run.displayTitle)) : on ne publie pas une version cassée." danger; Wait-Back; return
    }
    $ctx = Get-ReleaseContext $Repo
    $base = if ($ctx.Last) { $ctx.Last.V } else { [pscustomobject]@{ Major = 0; Minor = 0; Patch = 0 } }
    $types = @{}
    foreach ($p in $ctx.Prs) { $t = Get-PrType $p; $types[$t] = 1 + [int]$types[$t] }
    $summary = (($types.GetEnumerator() | Sort-Object Name | ForEach-Object { "$($_.Value) $($_.Name)" }) -join ', ')
    if (-not $summary) { $summary = 'aucune PR' }
    # SemVer before 1.0 (AGENTS.md): a new feature or a break bumps MINOR, the rest PATCH
    $minor = "v$($base.Major).$($base.Minor + 1).0"
    $patch = "v$($base.Major).$($base.Minor).$($base.Patch + 1)"
    $major = "v$($base.Major + 1).0.0"
    $suggested = if ($types['nouveauté'] -or $types['rupture']) { $minor } else { $patch }

    Show-Logo 'Nouvelle release'
    Write-Color "  Dernière version : $(if ($ctx.Last) { $ctx.Last.Tag } else { 'aucune' })" muted
    Write-Color "  PR fusionnées depuis : $($ctx.Prs.Count) ($summary)" muted
    foreach ($p in $ctx.Prs | Select-Object -First 12) { Write-Color ("    #$($p.number)".PadRight(10) + "$($p.title)") muted_dim }
    if ($ctx.Prs.Count -gt 12) { Write-Color "    … et $($ctx.Prs.Count - 12) autres" muted_dim }
    $items = @(@{ Key = '1'; Label = "$suggested (proposée)"; Hint = 'd''après les étiquettes des PR'; Tag = $suggested })
    foreach ($t in @($patch, $minor, $major) | Where-Object { $_ -ne $suggested }) {
        $hint = if ($t -eq $patch) { 'correctifs seulement' } elseif ($t -eq $minor) { 'nouveautés' } else { 'version majeure' }
        $items += @{ Key = [string]($items.Count + 1); Label = $t; Hint = $hint; Tag = $t }
    }
    $items += @{ Key = 'n'; Label = 'Autre numéro…'; Hint = 'par exemple v0.3.0-beta.1 (pré-release)'; Tag = $null }
    $pick = Show-Menu 'Quel numéro ?' $items
    if (-not $pick) { return }
    $tag = $pick.Tag
    if (-not $tag) {
        $tag = Read-Text 'Numéro (vMAJEUR.MINEUR.CORRECTIF, suffixe -… pour une pré-release ; vide : annuler) :'
        if (-not $tag) { return }
        if ($tag -notmatch '^v') { $tag = "v$tag" }
        if (-not (ConvertTo-Version $tag)) { Write-Color "  « $tag » n'est pas un numéro SemVer." danger; Wait-Back; return }
    }
    if (git -C $Paths.Repo tag --list $tag) { Write-Color "  Le tag $tag existe déjà." danger; Wait-Back; return }
    $pre = [bool](ConvertTo-Version $tag).Suffix
    Write-Host ''
    Write-Color "  $tag, sur le dernier main$(if ($pre) { ', en pré-release' }) : le workflow Release compile le viewer et publie le zip avec les notes." ink
    if (-not (Read-YesNo "Créer la release $tag ?")) { return }

    if ($DryRun) {
        Write-Color "  [simulation] git tag -a $tag origin/main ; git push origin $tag" teal
        Wait-Back; return
    }
    git -C $Paths.Repo fetch origin --quiet
    git -C $Paths.Repo tag -a $tag origin/main -m "Aurora Viewer $($tag.TrimStart('v'))"
    if ($LASTEXITCODE -ne 0) { Write-Color '  Création du tag impossible.' danger; Wait-Back; return }
    git -C $Paths.Repo push origin $tag
    if ($LASTEXITCODE -ne 0) {
        git -C $Paths.Repo tag -d $tag | Out-Null
        Write-Color '  Envoi du tag refusé par GitHub : rien n''a été publié.' danger; Wait-Back; return
    }
    Write-Color "  Tag $tag envoyé. Compilation et publication de la release…" success
    $id = $null
    for ($i = 0; $i -lt 30 -and -not $id; $i++) {
        Start-Sleep -Seconds 2
        $id = Invoke-GhJson @('run', 'list', '-R', $Repo.Slug, '--workflow', 'Release', '--limit', '5', '--json', 'databaseId,headBranch') |
            Where-Object { $_.headBranch -eq $tag } | Select-Object -First 1 -ExpandProperty databaseId
    }
    if ($id) {
        gh run watch $id -R $Repo.Slug --exit-status
        if ($LASTEXITCODE -eq 0) {
            $url = gh release view $tag -R $Repo.Slug --json url --jq .url 2>$null
            Write-Color "  Release publiée : $url" success
            if ($url -and (Read-YesNo 'L''ouvrir dans le navigateur ?')) { Start-Process $url }
        } else {
            Write-Color "  Le workflow Release a échoué : gh run view $id --log-failed (le tag $tag existe ; supprime-le ici pour recommencer)." danger
        }
    } else {
        Write-Color '  Workflow Release introuvable pour l''instant : suis-le sur GitHub (onglet Actions).' warn
    }
    Wait-Back
}

function Remove-AuroraRelease($Repo, $Releases) {
    if (-not $Releases) { Write-Color '  Aucune release à supprimer.' muted; Wait-Back; return }
    $shortcuts = @(1..9 | ForEach-Object { [string]$_ }) + @(97..122 | ForEach-Object { [string][char]$_ })
    $items = @()
    for ($i = 0; $i -lt $Releases.Count -and $i -lt $shortcuts.Count; $i++) {
        $r = $Releases[$i]
        $items += @{ Key = $shortcuts[$i]; Label = $r.tagName; Hint = $r.name; Release = $r }
    }
    $pick = Show-Menu 'Quelle release supprimer ?' $items
    if (-not $pick) { return }
    $tag = $pick.Release.tagName
    # releases that hold assets (assets-1: the emoji font fetch-assets.ps1 downloads)
    if ($tag -notmatch '^v\d') {
        Write-Color "  $tag n'est pas une version : elle contient des fichiers dont le dépôt a besoin (la police emoji pour assets-1). Protégée." warn
        Wait-Back; return
    }
    Write-Host ''
    Write-Color "  Supprimer $tag efface la release, ses fichiers et le tag, sur GitHub et ici. C'est irréversible." warn
    $typed = Read-Text "Pour confirmer, tape « $tag » (vide : annuler) :"
    if ($typed -cne $tag) { Write-Color '  Annulé : rien n''a été supprimé.' muted; Wait-Back; return }
    if ($DryRun) {
        Write-Color "  [simulation] gh release delete $tag --cleanup-tag --yes ; git tag -d $tag" teal
        Wait-Back; return
    }
    gh release delete $tag -R $Repo.Slug --cleanup-tag --yes
    if ($LASTEXITCODE -eq 0) {
        git -C $Paths.Repo tag -d $tag 2>$null | Out-Null
        Write-Color "  Release $tag supprimée." success
    } else {
        Write-Color '  Suppression refusée par GitHub (voir ci-dessus).' danger
    }
    Wait-Back
}
