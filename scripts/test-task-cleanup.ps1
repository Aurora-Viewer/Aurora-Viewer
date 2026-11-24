<#
.SYNOPSIS
    Tests the last build date of a task's target\ (Get-LastBuildTime and
    Set-LastBuildMarker, common.ps1) and the lookup of a task's PR, its head
    on origin or on a fork (Get-BranchPr's offline parts), which the
    housekeeping of new-task.ps1, end-task.ps1 and the task list of
    aurora-tools.cmd read, and the main repository's remote (Get-MainRepo),
    which those and the scripts that work on GitHub read. Fixture folders and a fixture repository only, in
    .scratch\: no real task, build, setting or GitHub call is touched.
#>
$ErrorActionPreference = 'Stop'
. "$PSScriptRoot\common.ps1"
$ErrorActionPreference = 'Stop'

function Assert-CleanupTest([bool]$Condition, [string]$Message) {
    if (-not $Condition) { throw $Message }
}

# Creates the folders (and files, for names with an extension) of a fake
# target, then dates each entry: children first, since creating an entry
# changes the date of its folder.
function New-FakeTarget([string]$Root, [hashtable]$Ages) {
    New-Item -ItemType Directory -Force -Path $Root | Out-Null
    [IO.File]::WriteAllText((Join-Path $Root 'CACHEDIR.TAG'), "Signature: 8a477f597d28d172789f06886806bc55`n")
    foreach ($rel in $Ages.Keys) {
        $path = Join-Path $Root $rel
        if ([IO.Path]::GetExtension($rel)) {
            New-Item -ItemType Directory -Force -Path (Split-Path $path -Parent) | Out-Null
            [IO.File]::WriteAllText($path, 'fixture')
        } else {
            New-Item -ItemType Directory -Force -Path $path | Out-Null
        }
    }
    foreach ($rel in $Ages.Keys | Sort-Object { $_.Split('\').Count } -Descending) {
        $path = Join-Path $Root $rel
        $when = (Get-Date).AddHours(-$Ages[$rel])
        if (Test-Path -LiteralPath $path -PathType Container) { [IO.Directory]::SetLastWriteTime($path, $when) }
        else { [IO.File]::SetLastWriteTime($path, $when) }
    }
}

function Get-AgeHours([string]$Target) {
    $last = Get-LastBuildTime $Target
    if ($null -eq $last) { return $null }
    ((Get-Date) - $last).TotalHours
}

function Test-Near($Hours, [double]$Expected) { $null -ne $Hours -and [math]::Abs($Hours - $Expected) -lt 0.1 }

$scratch = Join-Path (Split-Path $PSScriptRoot -Parent) '.scratch'
$fixture = Join-Path $scratch "cleanup-test-$([Guid]::NewGuid().ToString('N'))"
New-Item -ItemType Directory -Path $fixture -Force | Out-Null
try {
    # Cargo 1.100: no deps\, the crates are in build\<crate>\<hash>\. A task
    # idle for 30 hours, whose crates were first built a month ago.
    $new = Join-Path $fixture 'new-layout'
    New-FakeTarget $new @{
        'debug\build\serde\0123abcd' = 720; 'debug\build\serde' = 720; 'debug\build\aurora-viewer\4567ef01' = 700
        'debug\build\aurora-viewer' = 700; 'debug\build' = 700; 'debug\incremental\aurora_viewer-1' = 30
        'debug\incremental' = 30; 'debug\.cargo-lock' = 720; 'debug' = 30
    }
    Assert-CleanupTest (Test-Near (Get-AgeHours $new) 30) 'New layout: the last build is the incremental folder.'

    # The trim of new-task.ps1: marker first, then the incremental caches go.
    # Deleting them redates debug\ to now; the age must stay 30 hours, and
    # above all must not become unknown (the 2026-11-22 bug: "infinitely"
    # idle, whole target deleted at the next run).
    Set-LastBuildMarker $new (Get-LastBuildTime $new)
    foreach ($d in Get-IncrementalDirs $new) { Remove-Item -LiteralPath $d -Recurse -Force }
    Assert-CleanupTest (-not (Get-IncrementalDirs $new)) 'The fixture trim must remove incremental\.'
    Assert-CleanupTest (Test-Near (Get-AgeHours $new) 30) 'After a trim, the age must still count from the last build.'

    # Without the marker, the per-crate folders still date it (no unknown).
    Remove-Item -LiteralPath (Join-Path $new $LastBuildMarker)
    Assert-CleanupTest (Test-Near (Get-AgeHours $new) 700) 'Without incremental\ nor marker, the build\ folders date the target.'

    # A build after the trim is newer than the marker.
    Set-LastBuildMarker $new (Get-Date).AddHours(-30)
    New-FakeTarget $new @{ 'debug\incremental\aurora_viewer-2' = 1; 'debug\incremental' = 1 }
    Assert-CleanupTest (Test-Near (Get-AgeHours $new) 1) 'A build after the trim must win over the marker.'

    # Older Cargo: deps\ and .fingerprint\; and a release build, without
    # incremental\, dated by the exe Cargo writes into the profile folder.
    $old = Join-Path $fixture 'old-layout'
    New-FakeTarget $old @{ 'debug\deps' = 5; 'debug\.fingerprint' = 6; 'debug\incremental' = 9; 'release\deps' = 50; 'release\aurora-viewer.exe' = 3 }
    Assert-CleanupTest (Test-Near (Get-AgeHours $old) 3) 'Older layout: deps\, .fingerprint\ and the profile files date the target.'

    # Nothing to date it: unknown, never "infinitely" old.
    $empty = Join-Path $fixture 'empty'
    New-FakeTarget $empty @{}
    Assert-CleanupTest ($null -eq (Get-LastBuildTime $empty)) 'A target with no build entry has no date.'
    Assert-CleanupTest ($null -eq (Get-LastBuildTime (Join-Path $fixture 'missing'))) 'A missing target has no date.'

    # The PR of a task branch (Get-BranchPr): the GitHub slug of a remote.
    Assert-CleanupTest ((Get-GitHubSlug 'https://github.com/Upstream/Aurora-Viewer.git') -eq 'Upstream/Aurora-Viewer') 'Slug of an https remote.'
    Assert-CleanupTest ((Get-GitHubSlug 'git@github.com:Me/Aurora-Viewer.git') -eq 'Me/Aurora-Viewer') 'Slug of an ssh remote.'
    Assert-CleanupTest ((Get-GitHubSlug 'https://github.com/Me/Aurora-Viewer/') -eq 'Me/Aurora-Viewer') 'Slug without .git.'
    Assert-CleanupTest ($null -eq (Get-GitHubSlug 'https://gitlab.com/Me/Aurora-Viewer.git')) 'Another host has no GitHub slug.'

    # The PRs gh lists for a branch name, newest first: another contributor's
    # PR from a branch of the same name is never ours.
    $prs = '[{"number":3,"state":"OPEN","headRefOid":"c3","headRepositoryOwner":{"login":"someone"}},' +
        '{"number":4,"state":"CLOSED","headRefOid":"c4","headRepositoryOwner":null},' +
        '{"number":2,"state":"MERGED","headRefOid":"c2","headRepositoryOwner":{"login":"me"}},' +
        '{"number":1,"state":"CLOSED","headRefOid":"c1","headRepositoryOwner":{"login":"Upstream"}}]' | ConvertFrom-Json
    Assert-CleanupTest ((Select-BranchPr $prs @('Me')).number -eq 2) 'A fork PR is found by its head owner, case ignored.'
    Assert-CleanupTest ((Select-BranchPr $prs @('Upstream')).number -eq 1) 'A PR from a branch of origin is found.'
    Assert-CleanupTest ((Select-BranchPr $prs @('Upstream', 'Me')).number -eq 2) 'The newest of our PRs wins.'
    Assert-CleanupTest ($null -eq (Select-BranchPr $prs @('nobody'))) 'Only other contributors: no PR.'
    Assert-CleanupTest ($null -eq (Select-BranchPr ('[]' | ConvertFrom-Json) @('Me'))) 'No PR at all.'
    # -AllCommits: a PR whose head lacks the local commits is skipped
    Assert-CleanupTest ((Select-BranchPr $prs @('Upstream', 'Me') { param($Oid) $Oid -eq 'c1' }).number -eq 1) 'A head without the local commits is skipped.'
    Assert-CleanupTest ($null -eq (Select-BranchPr $prs @('Me') { param($Oid) $false })) 'No head with the local commits: no PR.'

    # Where a branch was pushed, in a fixture repository whose git reads no
    # global or system setting (a pushDefault of the PC would change it).
    $repo = Join-Path $fixture 'repo'
    $savedGlobal = $env:GIT_CONFIG_GLOBAL; $savedNoSystem = $env:GIT_CONFIG_NOSYSTEM
    $env:GIT_CONFIG_GLOBAL = Join-Path $fixture 'empty.gitconfig'; $env:GIT_CONFIG_NOSYSTEM = '1'
    [IO.File]::WriteAllText($env:GIT_CONFIG_GLOBAL, '')
    try {
        git init -q $repo
        git -C $repo -c user.name=fixture -c user.email=fixture@example.invalid commit -q --allow-empty -m fixture
        git -C $repo remote add origin https://github.com/Upstream/Aurora-Viewer.git
        git -C $repo remote add fork git@github.com:Me/Aurora-Viewer.git
        git -C $repo remote add mirror https://gitlab.com/Me/Aurora-Viewer.git
        $head = (git -C $repo rev-parse HEAD).Trim()
        # new-task.ps1's tracking of origin/main says nothing: every GitHub owner
        git -C $repo config branch.feat/new.remote origin
        git -C $repo config branch.feat/new.merge refs/heads/main
        $all = @(Get-BranchHeadOwners $repo 'feat/new' | Sort-Object)
        Assert-CleanupTest (($all -join ',') -eq 'Me,Upstream') "Unpushed branch: every GitHub owner (got '$($all -join ',')')."
        # pushed to the fork without -u: the remote-tracking ref tells
        git -C $repo update-ref refs/remotes/fork/fix/pushed $head
        Assert-CleanupTest ((@(Get-BranchHeadOwners $repo 'fix/pushed') -join ',') -eq 'Me') 'Branch pushed to the fork.'
        # pushed with -u: it tracks the branch of the same name on the fork
        git -C $repo config branch.fix/tracked.remote fork
        git -C $repo config branch.fix/tracked.merge refs/heads/fix/tracked
        Assert-CleanupTest ((@(Get-BranchHeadOwners $repo 'fix/tracked') -join ',') -eq 'Me') 'Branch tracking the fork.'
        # pushed to origin itself
        git -C $repo update-ref refs/remotes/origin/fix/upstream $head
        Assert-CleanupTest ((@(Get-BranchHeadOwners $repo 'fix/upstream') -join ',') -eq 'Upstream') 'Branch pushed to origin.'
        git -C $repo config branch.fix/push-remote.pushRemote fork
        Assert-CleanupTest ((@(Get-BranchHeadOwners $repo 'fix/push-remote') -join ',') -eq 'Me') 'Branch with a push remote.'
        # a contributor's fork fetched for a review holds a branch of the
        # same name: the branch's own push settings win over it
        git -C $repo remote add review https://github.com/Other/Aurora-Viewer.git
        git -C $repo update-ref refs/remotes/review/fix/tracked $head
        Assert-CleanupTest ((@(Get-BranchHeadOwners $repo 'fix/tracked') -join ',') -eq 'Me') 'Push settings win over a fetched ref.'

        # the main repository: origin here, whatever the other remotes
        Assert-CleanupTest ((Get-MainRepo $repo) -eq 'Upstream/Aurora-Viewer') 'Clone of the main repository: origin.'
        # a clone of a personal fork: origin is the fork, upstream the main one
        $forkClone = Join-Path $fixture 'fork-clone'
        git init -q $forkClone
        git -C $forkClone remote add origin git@github.com:Me/Aurora-Viewer.git
        Assert-CleanupTest ((Get-MainRepo $forkClone) -eq 'Me/Aurora-Viewer') 'Only origin: origin.'
        git -C $forkClone remote add upstream https://github.com/Upstream/Aurora-Viewer.git
        Assert-CleanupTest ((Get-MainRepo $forkClone) -eq 'Upstream/Aurora-Viewer') 'Clone of a fork: upstream.'
        # gh repo set-default marks its choice, which wins over the names
        git -C $forkClone remote add team https://github.com/Team/Aurora-Viewer.git
        git -C $forkClone config remote.team.gh-resolved base
        Assert-CleanupTest ((Get-MainRepo $forkClone) -eq 'Team/Aurora-Viewer') 'The remote gh repo set-default chose wins.'
        # a remote off GitHub is skipped, the next one taken
        git -C $forkClone config --unset remote.team.gh-resolved
        git -C $forkClone remote set-url upstream https://gitlab.com/Upstream/Aurora-Viewer.git
        Assert-CleanupTest ((Get-MainRepo $forkClone) -eq 'Me/Aurora-Viewer') 'An upstream off GitHub is skipped.'
        git -C $forkClone remote set-url origin https://gitlab.com/Me/Aurora-Viewer.git
        Assert-CleanupTest ($null -eq (Get-MainRepo $forkClone)) 'No main repository on GitHub: nothing.'

        # every commit of the local branch must be in the PR's head
        git -C $repo -c user.name=fixture -c user.email=fixture@example.invalid commit -q --allow-empty -m later
        $later = (git -C $repo rev-parse HEAD).Trim()
        Assert-CleanupTest (Test-CommitIn $repo $head $later) 'An older commit is in a later head.'
        Assert-CleanupTest (Test-CommitIn $repo $later $later) 'A commit is in itself.'
        Assert-CleanupTest (-not (Test-CommitIn $repo $later $head)) 'A local commit made after the head is not in it.'
        Assert-CleanupTest (-not (Test-CommitIn $repo $head ('0' * 40))) 'A head missing here holds nothing.'
    } finally {
        $env:GIT_CONFIG_GLOBAL = $savedGlobal; $env:GIT_CONFIG_NOSYSTEM = $savedNoSystem
    }

    # git config of an unset key and show-ref of a missing ref exit with 1:
    # expected above, but check.ps1 judges the step by the last exit code
    $global:LASTEXITCODE = 0
    Write-Output 'Task cleanup: new and older Cargo layouts, trim marker, later build, unknown date, fork PR lookup and main repository passed.'
} finally {
    $resolved = [IO.Path]::GetFullPath($fixture)
    if ((Split-Path $resolved -Parent) -ne [IO.Path]::GetFullPath($scratch) -or (Split-Path $resolved -Leaf) -notlike 'cleanup-test-*') { throw 'Cleanup outside fixture refused.' }
    Remove-Item -LiteralPath $resolved -Recurse -Force
}
