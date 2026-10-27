<#
.SYNOPSIS
    Creates (or updates) the labels on GitHub: types, for PRs only, and
    areas, for PRs and issues. The nature of an issue is its issue type (Bug,
    Feature, Task, Epic; AGENTS.md section 15). Keeps the other labels and
    their history. A
    label still under its former French name (Was) is renamed, so the PRs
    that carry it keep it. Run again when the list changes.

.EXAMPLE
    ./scripts/setup-labels.ps1 -DryRun    # show the changes without making them
#>
param([switch]$DryRun)
. "$PSScriptRoot\common.ps1"
# gh writes UTF-8; the console decodes it with its OEM code page by default
# (the accents of the former names would not match)
try { [Console]::OutputEncoding = [Text.Encoding]::UTF8 } catch { }

$origin = git -C $PSScriptRoot remote get-url origin
if ($LASTEXITCODE -ne 0 -or -not $origin) { throw 'Remote origin not found: no label changed.' }
$repo = gh repo view $origin.Trim() --json nameWithOwner --jq .nameWithOwner
if ($LASTEXITCODE -ne 0 -or -not $repo) { throw 'Could not determine the GitHub repository: no label changed.' }

$labels = @(
    # type of change (release notes, .github/release.yml)
    @{ Name = 'feature'; Was = 'nouveauté'; Color = '8B5CF6'; Desc = 'New feature' },
    @{ Name = 'fix'; Was = 'correctif'; Color = 'F87171'; Desc = 'Bug fix' },
    @{ Name = 'performance'; Color = '5EEAD4'; Desc = 'Faster, less memory' },
    @{ Name = 'maintenance'; Color = '6F7BA0'; Desc = 'Refactoring, dependencies, tooling' },
    @{ Name = 'docs'; Color = 'A78BFA'; Desc = 'Documentation' },
    @{ Name = 'breaking'; Was = 'rupture'; Color = 'FB923C'; Desc = 'Breaks a compatibility (settings, cache, format...)' },
    # area of the viewer
    @{ Name = 'render'; Was = 'rendu'; Color = '4F46E5'; Desc = 'Rendering engine, shaders, GPU' },
    @{ Name = 'avatar'; Color = '818CF8'; Desc = 'Avatars: animations, appearance, movement' },
    @{ Name = 'world'; Was = 'monde'; Color = '4ADE80'; Desc = 'Objects, terrain, environment, world sounds' },
    @{ Name = 'network'; Was = 'réseau'; Color = '070B1F'; Desc = 'Protocol, connection, assets' },
    @{ Name = 'audio-voice'; Was = 'audio-voix'; Color = 'F472B6'; Desc = 'Sound, music, voice' },
    @{ Name = 'ui'; Was = 'interface'; Color = 'C4B5FD'; Desc = 'Windows, options, usability' },
    @{ Name = 'chat'; Color = '9F6DCF'; Desc = 'Features related to chat window' },
    @{ Name = 'nameplate'; Color = 'F3C07F'; Desc = 'Features regarding the nameplate above avatars' },
    @{ Name = 'contact-group'; Color = 'C3C33F'; Desc = 'Contact Groups/Sets and related features' },
    @{ Name = 'inventory'; Color = 'AAAAAA'; Desc = 'Inventory window, folders and items' },
    @{ Name = 'build-tools'; Was = 'construction'; Color = 'FCD34D'; Desc = 'In-world building and editing tools' },
    @{ Name = 'tooling'; Was = 'outillage'; Color = '6F7BA0'; Desc = 'GitHub repository, CI, scripts and development environment' }
)

$existing = @(gh label list --repo $repo --limit 500 --json name --jq '.[].name')
if ($LASTEXITCODE -ne 0) { throw 'Could not list the labels: no label changed.' }
foreach ($l in $labels) {
    if ($l.Was -and $existing -contains $l.Was) {
        if ($existing -contains $l.Name) {
            # both exist: GitHub cannot merge two labels, a human moves the PRs and issues
            Write-Warning "'$($l.Was)' and '$($l.Name)' both exist: move the PRs and issues to '$($l.Name)', then delete '$($l.Was)'."
        } elseif ($DryRun) {
            Write-Host "rename  $($l.Was) -> $($l.Name)"
        } else {
            Invoke-Checked "rename $($l.Was) -> $($l.Name)" { gh label edit $l.Was --repo $repo --name $l.Name }
        }
    }
    if ($DryRun) {
        Write-Host "update  $($l.Name): $($l.Desc)"
    } else {
        Invoke-Checked "label $($l.Name)" { gh label create $l.Name --repo $repo --color $l.Color --description $l.Desc --force }
    }
}
if ($DryRun) { Write-Host 'Dry run: nothing changed.' -ForegroundColor Yellow } else { Write-Host 'Labels up to date.' -ForegroundColor Green }
