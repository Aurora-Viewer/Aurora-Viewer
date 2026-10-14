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

# scripts/common.ps1 next to these tools, or the repository's when the tools
# run alone from %TEMP% (first installation); $null before the clone
function Get-CommonScript($P) {
    foreach ($c in (Join-Path (Split-Path $PSScriptRoot -Parent) 'common.ps1'), (Join-Path $P.Repo 'scripts\common.ps1')) {
        if (Test-Path $c) { return $c }
    }
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
    & $add (New-Check git 'Git' $git $(if ($git) { 'ok' } else { 'todo' }) $(if ($git) { 'nothing to do' } else { 'installed with winget, for the whole machine' }))
    $gh = if (Get-Command gh -ErrorAction SilentlyContinue) { (gh --version | Select-Object -First 1) -replace '^gh version\s+(\S+).*$', '$1' }
    & $add (New-Check gh 'GitHub CLI' $gh $(if ($gh) { 'ok' } else { 'todo' }) $(if ($gh) { 'nothing to do' } else { 'installed with winget, for the whole machine' }))
    $msvc = Get-MsvcName
    & $add (New-Check msvc 'Visual Studio C++ tools' $msvc $(if ($msvc) { 'ok' } else { 'todo' }) $(if ($msvc) { 'nothing to do' } else { 'installed with winget (several GB)' }))

    $rustup = if (Get-Command rustup -ErrorAction SilentlyContinue) { (rustup --version 2>$null) -replace '^rustup\s+(\S+).*$', 'rustup $1' }
    if (-not $rustup) {
        & $add (New-Check rust 'Rust' $null todo 'installed with winget (rustup, for your account)')
    } elseif (-not $Offline -and ((rustup check 2>$null) -match '^stable.*Update available')) {
        & $add (New-Check rust 'Rust' "$rustup, stable behind" stale 'stable version updated')
    } else {
        & $add (New-Check rust 'Rust' $rustup ok 'nothing to do')
    }

    $hasRepo = Test-Path (Join-Path $P.Repo '.git')
    if (-not $hasRepo) {
        & $add (New-Check repo 'aurora-viewer repository' $null todo 'cloned from GitHub')
    } else {
        $branch = (git -C $P.Repo rev-parse --abbrev-ref HEAD).Trim()
        $behind = 0
        if (-not $Offline) {
            git -C $P.Repo fetch origin --quiet 2>$null
            $behind = [int](git -C $P.Repo rev-list --count 'HEAD..origin/main' 2>$null)
        }
        $dirty = [bool](git -C $P.Repo status --porcelain --untracked-files=no)
        if ($branch -ne 'main' -or $dirty) {
            & $add (New-Check repo 'aurora-viewer repository' "on $branch$(if ($dirty) { ', local changes' })" info 'left as is (your changes)')
        } elseif ($behind -gt 0) {
            & $add (New-Check repo 'aurora-viewer repository' "main, $behind commit(s) behind" stale 'updated (git pull)')
        } else {
            & $add (New-Check repo 'aurora-viewer repository' 'main, up to date' ok 'nothing to do')
        }
    }

    if (-not (Test-Path (Join-Path $P.Firestorm '.git'))) {
        & $add (New-Check firestorm 'Firestorm sources' $null todo 'cloned (latest commit only)')
    } else {
        $stale = $false
        if (-not $Offline) {
            $remote = (git -C $P.Firestorm ls-remote origin HEAD 2>$null) -split '\s+' | Select-Object -First 1
            $stale = $remote -and $remote -ne (git -C $P.Firestorm rev-parse HEAD).Trim()
        }
        $date = (git -C $P.Firestorm log -1 --format=%cs 2>$null)
        if ($stale) { & $add (New-Check firestorm 'Firestorm sources' "from $date, behind" stale 'updated') }
        else { & $add (New-Check firestorm 'Firestorm sources' "from $date" ok 'nothing to do') }
    }

    $work = Test-Path $P.Work
    & $add (New-Check work 'work\ folder (tasks)' $(if ($work) { 'present' }) $(if ($work) { 'ok' } else { 'todo' }) $(if ($work) { 'nothing to do' } else { 'created' }))
    & $add (New-Check crates 'Rust toolchain and crates' $null always 'installed or updated (cargo fetch)')

    # shared compiler cache of the worktrees (scripts/common.ps1)
    $common = Get-CommonScript $P
    if ($common) {
        . $common
        $sccache = Find-Sccache
        $wired = Test-Path (Join-Path $P.Root '.cargo\config.toml')
        if (-not $sccache) {
            & $add (New-Check sccache 'Compiler cache (sccache)' $null todo 'installed with winget (for your account), shared by the tasks')
        } elseif (-not $wired) {
            & $add (New-Check sccache 'Compiler cache (sccache)' 'installed, not used by Cargo' stale 'set up for the repository and the tasks')
        } else {
            $version = (& $sccache --version 2>$null) -replace '^sccache\s+', ''
            & $add (New-Check sccache 'Compiler cache (sccache)' $version ok 'nothing to do')
        }
    }

    $login = $null
    if ($gh) {
        gh auth status *> $null
        if ($LASTEXITCODE -eq 0) { $login = "logged in: $(gh api user --jq .login 2>$null)" }
    }
    & $add (New-Check github 'GitHub login' $login $(if ($login) { 'ok' } else { 'todo' }) $(if ($login) { 'nothing to do' } else { 'gh auth login, in the browser' }))
    $email = if ($git) { if ($hasRepo) { git -C $P.Repo config user.email } else { git config --global user.email } }
    & $add (New-Check identity 'Git identity' $email $(if ($email) { 'ok' } else { 'todo' }) $(if ($email) { 'nothing to do' } else { 'your GitHub username + noreply address' }))

    # the --release viewer of RELEASE\ (menu "Release viewer"): information only
    $version = Join-Path $P.Release 'Aurora-Viewer\version.txt'
    if ($hasRepo -and (Test-Path $version)) {
        # the line after the reported version: "main: <short sha> <subject> (<date>)",
        # or "PR #42: …" (files written before the switch to English: "main : …")
        $built = if (((Get-Content $version -Encoding UTF8) -join "`n") -match '(?m)^(?:main|PR #\d+) ?: ([0-9a-f]{7,}) ') { $Matches[1] }
        $main = (git -C $P.Repo rev-parse --short origin/main 2>$null)
        $same = $built -and $main -and ($main.StartsWith($built) -or $built.StartsWith($main))
        & $add (New-Check release 'Release viewer (RELEASE\)' $(if ($same) { 'up to date with main' } else { "commit $built, main is ahead" }) info $(if ($same) { 'nothing to do' } else { 'menu "Release viewer" to rebuild it' }))
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
        throw "$What is missing, and winget (the Windows package manager) is not available: install it by hand."
    }
    Write-Color "  Installing $What (a Windows permission window may open)…" muted
    winget install --id $Id --exact --source winget --accept-package-agreements --accept-source-agreements @Extra
    if ($LASTEXITCODE -ne 0) { throw "installing $What (code $LASTEXITCODE)." }
    Update-SessionPath
}

# Installs what is missing and updates what is behind; throws on failure.
# Everything already there is kept: running it again is safe.
function Invoke-Repair($P) {
    Write-Title 'Tools'
    if (-not (Get-Command git -ErrorAction SilentlyContinue)) { Install-Tool 'Git.Git' 'Git' @('--scope', 'machine') }
    if (-not (Get-Command gh -ErrorAction SilentlyContinue)) { Install-Tool 'GitHub.cli' 'GitHub CLI' @('--scope', 'machine') }
    # Rust on Windows links with the Visual Studio C++ tools (MSVC and the
    # Windows SDK): install them before Rust.
    if (-not (Get-MsvcName)) {
        Install-Tool 'Microsoft.VisualStudio.BuildTools' 'the Visual Studio C++ tools' @(
            '--override', '--passive --wait --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended')
    }
    # rustup is per user by design: it updates itself and the toolchains
    # without admin rights (%USERPROFILE%\.cargo and .rustup).
    if (-not (Get-Command rustup -ErrorAction SilentlyContinue)) {
        Install-Tool 'Rustlang.Rustup' 'Rust (rustup)'
        $env:Path += ";$env:USERPROFILE\.cargo\bin"
        if (-not (Get-Command rustup -ErrorAction SilentlyContinue)) {
            throw 'rustup not found after its installation: open a new window and run it again.'
        }
    }
    Write-Color '  Git, GitHub CLI, C++ tools and Rust: OK.' success

    Write-Title 'Aurora Viewer repository'
    New-Item -ItemType Directory -Force -Path $P.Root -ErrorAction Stop | Out-Null
    if (Test-Path (Join-Path $P.Repo '.git')) {
        $branch = (git -C $P.Repo rev-parse --abbrev-ref HEAD).Trim()
        if ($branch -eq 'main' -and -not (git -C $P.Repo status --porcelain --untracked-files=no)) {
            git -C $P.Repo pull --ff-only --quiet
            if ($LASTEXITCODE -ne 0) { Write-Color '  main not updated (git pull failed): left as is.' warn }
            else { Write-Color "  $($P.Repo): up to date." success }
        } else {
            Write-Color "  On '$branch' or with local changes: left as is." warn
        }
    } else {
        git clone $RepoUrl $P.Repo
        if ($LASTEXITCODE -ne 0) { throw 'cloning the repository.' }
    }
    New-Item -ItemType Directory -Force -Path $P.Work | Out-Null

    Write-Title 'Firestorm sources (reference for the agents)'
    if (Test-Path (Join-Path $P.Firestorm '.git')) {
        # a read-only reference copy, never edited: take the latest commit as is
        git -C $P.Firestorm fetch --depth 1 --quiet origin HEAD
        if ($LASTEXITCODE -eq 0) { git -C $P.Firestorm reset --hard --quiet FETCH_HEAD }
        if ($LASTEXITCODE -ne 0) { Write-Color '  Not updated: left as is.' warn }
        else { Write-Color "  $($P.Firestorm): up to date." success }
    } else {
        # latest commit only; long paths: some Firestorm files go past 260 characters
        git clone --depth 1 -c core.longpaths=true $FirestormUrl $P.Firestorm
        if ($LASTEXITCODE -ne 0) { throw 'cloning the Firestorm sources.' }
    }

    Write-Title 'Rust toolchain and crates'
    Push-Location $P.Repo
    try {
        # rust-toolchain.toml: the latest stable, with rustfmt and clippy
        rustup toolchain install
        if ($LASTEXITCODE -ne 0) { rustup show }
        rustup update stable --no-self-update
        cargo fetch --locked
        if ($LASTEXITCODE -ne 0) { throw 'downloading the crates (cargo fetch).' }
    } finally {
        Pop-Location
    }

    Write-Title 'Compiler cache (sccache)'
    # the tasks take the dependencies already built by another one from it
    . (Get-CommonScript $P)
    if (-not (Find-Sccache)) {
        try { Install-Tool 'Mozilla.sccache' 'sccache' } catch { Write-Color "  Not installed ($($_.Exception.Message)): the builds work without it." warn }
    } elseif (Get-Command winget -ErrorAction SilentlyContinue) {
        # nothing to do when it is up to date; refused while a build uses it
        winget upgrade --id Mozilla.sccache --exact --source winget --accept-package-agreements --accept-source-agreements *> $null
        Update-SessionPath
    }
    Sync-BuildCache $P.Root
    if (Find-Sccache) { Write-Color "  Shared by the tasks: $(Join-Path $P.Root '.sccache')." success }

    Write-Title 'GitHub'
    gh auth status *> $null
    if ($LASTEXITCODE -ne 0) {
        Write-Color '  GitHub login (for the PRs of the agents): follow the instructions.' ink
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
        Write-Color "  Git identity: $(git -C $P.Repo config user.name) <$(git -C $P.Repo config user.email)>" success
    } else {
        Write-Color '  Not logged in to GitHub: run the repair again later (needed for the PRs).' warn
    }
}
