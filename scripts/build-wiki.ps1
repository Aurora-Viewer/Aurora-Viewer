<#
.SYNOPSIS
    Generates the pages of the GitHub wiki from the guides of the repository.

.DESCRIPTION
    The Markdown files of the repository are the only source: each page of
    the wiki is a copy of one guide, with its links rewritten (another guide
    becomes its wiki page, any other file becomes a link to the repository,
    images are read from the repository). The sidebar is built from the
    page table below. The workflow .github/workflows/wiki.yml runs this
    script and pushes the result; pages edited by hand in the wiki are
    replaced.

    A new guide goes in the table: section, page name, source file.

.EXAMPLE
    ./scripts/build-wiki.ps1 -Out .scratch\wiki    # look at the result locally
#>
param(
    [Parameter(Mandatory)][string]$Out,
    # owner/name on GitHub, for the links back to the repository
    [string]$Repository = $(if ($env:GITHUB_REPOSITORY) { $env:GITHUB_REPOSITORY } else { 'Aurora-Viewer/Aurora-Viewer' }),
    [string]$Branch = 'main'
)
$ErrorActionPreference = 'Stop'

$pages = @(
    @{ Section = $null; Page = 'Home'; Title = 'Home'; Source = 'README.md' }
    @{ Section = 'For testers'; Page = 'Reporting-bugs'; Title = 'Reporting bugs'; Source = 'docs/BUG_REPORTING.md' }
    @{ Section = 'For testers'; Page = 'Installing-on-Linux'; Title = 'Installing on Linux'; Source = 'docs/LINUX.md' }
    @{ Section = 'For testers'; Page = 'VR'; Title = 'Experimental VR scene viewing'; Source = 'docs/VR.md' }
    @{ Section = 'For testers'; Page = 'Windows-VR'; Title = 'Experimental Windows VR'; Source = 'docs/VR_WINDOWS.md' }
    @{ Section = 'For testers'; Page = 'Installing-on-macOS'; Title = 'Installing on macOS'; Source = 'docs/MACOS.md' }
    @{ Section = 'For testers'; Page = 'Updates'; Title = 'Updates'; Source = 'docs/UPDATES.md' }
    @{ Section = 'For testers'; Page = 'Asset-imports'; Title = 'Asset imports'; Source = 'docs/IMPORTS.md' }
    @{ Section = 'For testers'; Page = 'Blender-Live'; Title = 'LiveMesh'; Source = 'docs/BLENDER_LIVE.md' }
    @{ Section = 'For testers'; Page = 'Hand-contacts'; Title = 'Motorcycle hand contacts'; Source = 'docs/HAND_CONTACTS.md' }
    @{ Section = 'For testers'; Page = 'IK-Edit'; Title = 'IK Edit and bone contacts'; Source = 'docs/IK_EDIT.md' }
    @{ Section = 'For testers'; Page = 'Animation-creation'; Title = 'Create native animations'; Source = 'docs/ANIMATION_CREATION.md' }
    @{ Section = 'For testers'; Page = 'Poser'; Title = 'Local photo poser'; Source = 'docs/POSER.md' }
    @{ Section = 'For testers'; Page = 'Dev-Toolbox'; Title = 'Dev Toolbox'; Source = 'docs/DEV_TOOLBOX.md' }
    @{ Section = 'Contributing'; Page = 'Guide-for-humans'; Title = 'Guide for humans'; Source = 'HUMANS.md' }
    @{ Section = 'Contributing'; Page = 'Test-options'; Title = 'Test options and profiling'; Source = 'docs/TESTING.md' }
    @{ Section = 'Contributing'; Page = 'Translating'; Title = 'Translating the viewer'; Source = 'docs/TRANSLATING.md' }
    @{ Section = 'Contributing'; Page = 'Branding'; Title = 'Branding and interface'; Source = 'docs/BRANDING.md' }
    @{ Section = 'Technical'; Page = 'Architecture'; Title = 'Architecture'; Source = 'ARCHITECTURE.md' }
    @{ Section = 'Technical'; Page = 'RLVa'; Title = 'RLVa implementation status'; Source = 'docs/RLVA.md' }
    @{ Section = 'Technical'; Page = 'Licences'; Title = 'Licences and notices'; Source = 'NOTICE.md' }
)
# kept in the repository only: linked from the sidebar
$repoOnly = @(
    @{ Title = 'Task tracking'; Source = 'TASKS.md' }
    @{ Title = 'Rules for AI agents'; Source = 'AGENTS.md' }
)

# .NET file calls do not follow PowerShell's current folder: use a full path
$Out = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($Out)
$repo = Split-Path $PSScriptRoot -Parent
$utf8 = New-Object Text.UTF8Encoding $false
$blob = "https://github.com/$Repository/blob/$Branch"
$raw = "https://raw.githubusercontent.com/$Repository/$Branch"
$pageOf = @{}
foreach ($p in $pages) { $pageOf[$p.Source] = $p.Page }

# Repository path of a link target written in the file $from ("docs/X.md").
function Resolve-Target([string]$from, [string]$target) {
    $parts = New-Object Collections.Generic.List[string]
    $dir = Split-Path $from -Parent
    if ($dir) { foreach ($s in ($dir -replace '\\', '/').Split('/')) { $parts.Add($s) } }
    foreach ($s in $target.Split('/')) {
        if ($s -eq '..') { if ($parts.Count) { $parts.RemoveAt($parts.Count - 1) } }
        elseif ($s -and $s -ne '.') { $parts.Add($s) }
    }
    $parts -join '/'
}

# Where a link of the guide $from points once in the wiki.
function Convert-Link([string]$from, [string]$target, [bool]$image) {
    if ($target -match '^([a-z][a-z0-9+.-]*:|#|//)') { return $target }
    $path, $anchor = $target.Split('#', 2)
    $file = Resolve-Target $from $path
    $suffix = if ($anchor) { "#$anchor" } else { '' }
    if (-not $image -and $pageOf.ContainsKey($file)) { return $pageOf[$file] + $suffix }
    if (-not (Test-Path -LiteralPath (Join-Path $repo $file))) { throw "$from links to '$target', which is not in the repository." }
    if ($image -or $file -match '\.(png|jpe?g|gif|svg|webp)$') { return "$raw/$file" }
    if (Test-Path -LiteralPath (Join-Path $repo $file) -PathType Container) { return "https://github.com/$Repository/tree/$Branch/$file" }
    "$blob/$file$suffix"
}

if (-not (Test-Path -LiteralPath $Out)) { New-Item -ItemType Directory -Path $Out | Out-Null }
# every page is generated: a page that is no longer in the table disappears
Get-ChildItem -LiteralPath $Out -Filter *.md -File | Remove-Item -Force

foreach ($p in $pages) {
    $source = $p.Source
    $lines = [IO.File]::ReadAllText((Join-Path $repo $source), $utf8) -replace "`r`n", "`n" -split "`n"
    $fenced = $false
    $body = foreach ($line in $lines) {
        if ($line -match '^\s*```') { $fenced = -not $fenced; $line; continue }
        if ($fenced) { $line; continue }
        # Markdown links and images (a badge is an image inside a link, so only
        # the target is matched), then HTML src / href attributes
        $line = [regex]::Replace($line, '\]\(([^)\s]+)\)', {
                param($m)
                '](' + (Convert-Link $source $m.Groups[1].Value $false) + ')'
            })
        [regex]::Replace($line, '\b(src|href)="([^"]+)"', {
                param($m)
                $m.Groups[1].Value + '="' + (Convert-Link $source $m.Groups[2].Value ($m.Groups[1].Value -eq 'src')) + '"'
            })
    }
    $note = "> This page is generated from [$source]($blob/$source). To change it, edit that file in the repository: edits made here are replaced."
    [IO.File]::WriteAllText((Join-Path $Out "$($p.Page).md"), ((@($note, '') + $body) -join "`n"), $utf8)
}

$sidebar = New-Object Collections.Generic.List[string]
$section = ''
foreach ($p in $pages) {
    if ($p.Section -and $p.Section -ne $section) { $section = $p.Section; $sidebar.Add(''); $sidebar.Add("**$section**"); $sidebar.Add('') }
    $sidebar.Add("- [$($p.Title)]($($p.Page))")
}
$sidebar.Add('')
$sidebar.Add('**In the repository**')
$sidebar.Add('')
foreach ($r in $repoOnly) { $sidebar.Add("- [$($r.Title)]($blob/$($r.Source))") }
$sidebar.Add('')
[IO.File]::WriteAllText((Join-Path $Out '_Sidebar.md'), ($sidebar -join "`n"), $utf8)
[IO.File]::WriteAllText((Join-Path $Out '_Footer.md'), "Generated from the [repository](https://github.com/$Repository); the guides are edited there.`n", $utf8)

Write-Host "$($pages.Count) pages written to $Out."
