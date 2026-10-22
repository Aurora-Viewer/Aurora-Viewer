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
    foreach ($t in 'breaking', 'feature', 'fix', 'performance', 'maintenance', 'docs') { if ($names -contains $t) { return $t } }
    'other'
}

function Show-ReleaseList($Releases) {
    Write-Rule 90
    Write-Color ('  ' + 'Tag'.PadRight(18) + 'Name'.PadRight(40) + 'Published'.PadRight(14) + 'Type') muted
    Write-Rule 90
    if (-not $Releases) { Write-Color '  No release.' muted }
    foreach ($r in $Releases) {
        $kind = if ($r.tagName -notmatch '^v\d') { 'assets (protected)' } elseif ($r.isDraft) { 'draft' } elseif ($r.isPrerelease) { 'pre-release' } elseif ($r.isLatest) { 'latest' } else { 'release' }
        $name = if ($r.name.Length -gt 38) { $r.name.Substring(0, 37) + '…' } else { $r.name }
        Write-Color ('  ' + $r.tagName.PadRight(18)) violet_light -NoNewline
        Write-Color $name.PadRight(40) ink -NoNewline
        Write-Color ([datetime]$r.publishedAt).ToString('yyyy-MM-dd').PadRight(14) muted -NoNewline
        Write-Color $kind $(if ($kind -eq 'latest') { 'success' } elseif ($kind -like 'assets*') { 'teal' } else { 'muted' })
    }
    Write-Rule 90
}

function Invoke-Releases {
    $repo = Get-RepoSlug
    if (-not $repo) { Write-Color '  GitHub repository not found (remote origin).' danger; Wait-Back; return }
    while ($true) {
        Show-Logo 'Releases'
        Write-Color "  Repository: $($repo.Slug)" muted
        Write-Color '  Reading the releases and your rights on GitHub…' muted_dim
        $releases = Get-ReleaseList $repo
        $rights = Test-ReleaseRights $repo
        if (-not $rights) {
            Show-Logo 'Releases'
            Write-Color "  Repository: $($repo.Slug)" muted
            Show-ReleaseList $releases
            Write-Color '  Your GitHub account has no write access to this repository: creating and deleting are unavailable.' warn
            Wait-Back
            return
        }
        $pick = Show-Menu 'Manage releases' @(
            @{ Key = '1'; Label = 'Create a release'; Hint = 'SemVer tag on the latest main, published by CI' }
            @{ Key = '2'; Label = 'Delete a release'; Hint = 'release and tag, cannot be undone' }
        ) { Show-Logo 'Releases'; Write-Color "  Repository: $($repo.Slug)" muted; Show-ReleaseList $releases }
        switch ($pick.Key) {
            '1' { New-AuroraRelease $repo }
            '2' { Remove-AuroraRelease $repo $releases }
            default { return }
        }
    }
}

function New-AuroraRelease($Repo) {
    Show-Logo 'New release'
    Write-Color '  Checking main and the PRs merged since the last release…' muted_dim
    $run = Invoke-GhJson @('run', 'list', '-R', $Repo.Slug, '--workflow', 'CI', '--branch', 'main', '--limit', '1', '--json', 'status,conclusion,displayTitle') | Select-Object -First 1
    if (-not $run -or $run.status -ne 'completed') {
        Write-Color '  CI is still running on main: wait for it to finish, then try again.' warn; Wait-Back; return
    }
    if ($run.conclusion -ne 'success') {
        Write-Color "  CI is red on main ($($run.displayTitle)): a broken version is not published." danger; Wait-Back; return
    }
    $ctx = Get-ReleaseContext $Repo
    $base = if ($ctx.Last) { $ctx.Last.V } else { [pscustomobject]@{ Major = 0; Minor = 0; Patch = 0 } }
    $types = @{}
    foreach ($p in $ctx.Prs) { $t = Get-PrType $p; $types[$t] = 1 + [int]$types[$t] }
    $summary = (($types.GetEnumerator() | Sort-Object Name | ForEach-Object { "$($_.Value) $($_.Name)" }) -join ', ')
    if (-not $summary) { $summary = 'no PR' }
    # SemVer before 1.0 (AGENTS.md): a new feature or a break bumps MINOR, the rest PATCH
    $minor = "v$($base.Major).$($base.Minor + 1).0"
    $patch = "v$($base.Major).$($base.Minor).$($base.Patch + 1)"
    $major = "v$($base.Major + 1).0.0"
    $suggested = if ($types['feature'] -or $types['breaking']) { $minor } else { $patch }

    $items = @(@{ Key = '1'; Label = "$suggested (suggested)"; Hint = 'from the PR labels'; Tag = $suggested })
    foreach ($t in @($patch, $minor, $major) | Where-Object { $_ -ne $suggested }) {
        $hint = if ($t -eq $patch) { 'fixes only' } elseif ($t -eq $minor) { 'new features' } else { 'major version' }
        $items += @{ Key = [string]($items.Count + 1); Label = $t; Hint = $hint; Tag = $t }
    }
    $items += @{ Key = 'n'; Label = 'Other number…'; Hint = 'for example v0.3.0-beta.1 (pre-release)'; Tag = $null }
    $pick = Show-Menu 'Step 1/3 · Which number?' $items {
        Show-Logo 'New release'
        Write-Color "  Last release: $(if ($ctx.Last) { $ctx.Last.Tag } else { 'none' })" muted
        Write-Color "  PRs merged since: $($ctx.Prs.Count) ($summary)" muted
        $limit = if ($Interactive) { [Math]::Max(0, [Math]::Min(12, $Host.UI.RawUI.WindowSize.Height - 17)) } else { 12 }
        foreach ($p in $ctx.Prs | Select-Object -First $limit) {
            $line = "    #$($p.number)".PadRight(10) + "$($p.title)"
            if ($Interactive -and $line.Length -ge $Host.UI.RawUI.WindowSize.Width) { $line = $line.Substring(0, [Math]::Max(0, $Host.UI.RawUI.WindowSize.Width - 2)) + '…' }
            Write-Color $line muted_dim
        }
        if ($ctx.Prs.Count -gt $limit) { Write-Color "    … and $($ctx.Prs.Count - $limit) more" muted_dim }
    }
    if (-not $pick) { return }
    $tag = $pick.Tag
    if (-not $tag) {
        $tag = Read-Text 'Number (vMAJOR.MINOR.PATCH, suffix -… for a pre-release; empty: cancel):'
        if (-not $tag) { return }
        if ($tag -notmatch '^v') { $tag = "v$tag" }
        if (-not (ConvertTo-Version $tag)) { Write-Color "  '$tag' is not a SemVer number." danger; Wait-Back; return }
    }
    if (git -C $Paths.Repo tag --list $tag) { Write-Color "  The tag $tag already exists." danger; Wait-Back; return }
    $pre = [bool](ConvertTo-Version $tag).Suffix
    $policy = Get-UpdatePolicy -RepoPath $Paths.Repo -Tag $tag -BaseTag $(if ($ctx.Last) { $ctx.Last.Tag }) -Ref origin/main
    $policy = Read-ReleaseMaintenance $policy
    if (-not $policy) { return }
    Show-Logo 'Confirmation of the release'
    Write-Color "  $tag, on the latest main$(if ($pre) { ', as a pre-release' }): the Release workflow builds the viewer and publishes the Windows zip with the notes, then adds the Linux archive." ink
    Write-Color "  Cache: $(if ($policy.cache_invalidation_before) { 'cleared for versions before ' + $policy.cache_invalidation_before } else { 'kept' })." ink
    Write-Color "  Settings: $(if ($policy.settings_reset_before) { 'reset for versions before ' + $policy.settings_reset_before } else { 'kept and migrated automatically if needed' })." ink
    if (-not (Read-YesNo "Create the release ${tag}?")) { return }

    $message = "Aurora Viewer $($tag.TrimStart('v'))`n`nAurora-Update-Policy: $($policy | ConvertTo-Json -Compress -Depth 4)"

    if ($DryRun) {
        Write-Color "  [dry run] tag $tag on origin/main with policy: $($policy | ConvertTo-Json -Compress -Depth 4)" teal
        Wait-Back; return
    }
    git -C $Paths.Repo fetch origin --quiet
    $tagMessageFile = [IO.Path]::GetTempFileName()
    try {
        [IO.File]::WriteAllText($tagMessageFile, $message, [Text.UTF8Encoding]::new($false))
        git -C $Paths.Repo tag -a $tag origin/main -F $tagMessageFile
        $tagExit = $LASTEXITCODE
    } finally {
        Remove-Item -LiteralPath $tagMessageFile -Force
    }
    if ($tagExit -ne 0) { Write-Color '  Could not create the tag.' danger; Wait-Back; return }
    git -C $Paths.Repo push origin $tag
    if ($LASTEXITCODE -ne 0) {
        git -C $Paths.Repo tag -d $tag | Out-Null
        Write-Color '  GitHub refused the tag push: nothing was published.' danger; Wait-Back; return
    }
    Write-Color "  Tag $tag pushed. Building and publishing the release…" success
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
            Write-Color "  Release published: $url" success
            if ($url -and (Read-YesNo 'Open it in the browser?')) { Start-Process $url }
        } else {
            $url = gh release view $tag -R $Repo.Slug --json url --jq .url 2>$null
            if ($url) {
                # the Windows release is out: only the Linux archive is missing
                Write-Color "  Release published for Windows: $url" success
                Write-Color "  The Linux build failed: gh run view $id --log-failed (a passing failure: gh run rerun $id --failed)." warn
            } else {
                Write-Color "  The Release workflow failed: gh run view $id --log-failed (the tag $tag exists; delete it here to start again)." danger
            }
        }
    } else {
        Write-Color '  Release workflow not found yet: follow it on GitHub (Actions tab).' warn
    }
    Wait-Back
}

function Remove-AuroraRelease($Repo, $Releases) {
    if (-not $Releases) { Write-Color '  No release to delete.' muted; Wait-Back; return }
    $shortcuts = @(1..9 | ForEach-Object { [string]$_ }) + @(97..122 | ForEach-Object { [string][char]$_ })
    $items = @()
    for ($i = 0; $i -lt $Releases.Count -and $i -lt $shortcuts.Count; $i++) {
        $r = $Releases[$i]
        $items += @{ Key = $shortcuts[$i]; Label = $r.tagName; Hint = $r.name; Release = $r }
    }
    $pick = Show-Menu 'Which release to delete?' $items { Show-Logo 'Delete a release' }
    if (-not $pick) { return }
    $tag = $pick.Release.tagName
    # releases that hold assets (assets-1: the 3D emoji font that older commits download)
    if ($tag -notmatch '^v\d') {
        Write-Color "  $tag is not a version: it holds files older commits need (the 3D emoji font for assets-1). Protected." warn
        Wait-Back; return
    }
    Write-Host ''
    Write-Color "  Deleting $tag erases the release, its files and the tag, on GitHub and here. This cannot be undone." warn
    $typed = Read-Text "To confirm, type '$tag' (empty: cancel):"
    if ($typed -cne $tag) { Write-Color '  Cancelled: nothing was deleted.' muted; Wait-Back; return }
    if ($DryRun) {
        Write-Color "  [dry run] gh release delete $tag --cleanup-tag --yes ; git tag -d $tag" teal
        Wait-Back; return
    }
    gh release delete $tag -R $Repo.Slug --cleanup-tag --yes
    if ($LASTEXITCODE -eq 0) {
        git -C $Paths.Repo tag -d $tag 2>$null | Out-Null
        Write-Color "  Release $tag deleted." success
    } else {
        Write-Color '  GitHub refused the deletion (see above).' danger
    }
    Wait-Back
}
