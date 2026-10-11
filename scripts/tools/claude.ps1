# Aurora Tools: Claude Code permissions for the agents' PR and issue workflow
# (dot-sourced by aurora-tools.ps1).
#
# AGENTS.md has the agent post its review on its own PR, merge it with
# merge-pr.ps1, read the open issues at the start of each session and answer
# the ones it handles; Claude Code's auto mode classifier refuses the first two
# as "self-approval", the issue comment as an "external system write" and even
# the listing of the issues ("interfere with workloads"), even after the human
# agrees in the chat. Allow rules in the user settings let these commands
# through (the merge queue and the protection of main still apply). User
# settings, not the repository's: the agents work in worktrees of ..\work\,
# where a project file would not be read.
#
# The rest of the follow-up that AGENTS.md asks for is allowed ahead of a
# refusal, which costs a stalled session: reading the PRs and the CI runs,
# triaging the issues (labels, closing, reopening: all can be undone) and
# publishing the captures of a PR (pr-captures.ps1 only pushes to the captures
# branch). Left to the classifier on purpose: gh api and git push (a wildcard
# would cover deletions, --force and tags), gh pr merge (merge-pr.ps1 goes
# through the merge queue), releases and end-task.ps1.

$ClaudeAllowRules = @(
    'PowerShell(gh pr comment *)'
    'PowerShell(./scripts/merge-pr.ps1 *)'
    'PowerShell(gh issue comment *)'
    'PowerShell(gh issue list *)'
    'PowerShell(gh issue view *)'
    'PowerShell(gh pr view *)'
    'PowerShell(gh pr list *)'
    'PowerShell(gh pr checks *)'
    'PowerShell(gh run view *)'
    'PowerShell(gh run list *)'
    'PowerShell(gh run watch *)'
    'PowerShell(gh issue edit *)'
    'PowerShell(gh issue close *)'
    'PowerShell(gh issue reopen *)'
    'PowerShell(./scripts/pr-captures.ps1 *)'
)
# For the classifier, should one of these commands still reach it (a listing
# piped into a PowerShell loop does not match the allow rule). One rule per
# subject: a settings file that already has the first ones only gains the new.
$ClaudeAutoModeRules = @(
    'In the Aurora-Viewer repository, posting the review comment on the user''s own pull request and merging it with scripts/merge-pr.ps1 is allowed once the human has validated the change and CI is green (AGENTS.md workflow).'
    'In the Aurora-Viewer repository, posting a comment on an issue of that repository with gh issue comment is allowed: answering the issues is part of the agents'' work (AGENTS.md, section 15).'
    'In the Aurora-Viewer repository, listing and reading the issues of that repository with gh issue list or gh issue view, including formatting their JSON output in PowerShell, is allowed: these commands only read, and following the issues is part of the agents'' work (AGENTS.md, sections 8 and 15).'
    'In the Aurora-Viewer repository, reading the state of its pull requests and CI runs with gh pr view, gh pr list, gh pr checks, gh run view, gh run list or gh run watch, including formatting their JSON output in PowerShell, is allowed: these commands only read (AGENTS.md, sections 8 and 11).'
    'In the Aurora-Viewer repository, labelling, editing, closing and reopening an issue of that repository with gh issue edit, gh issue close or gh issue reopen is allowed: triaging the issues is part of the agents'' work (AGENTS.md, section 15).'
    'In the Aurora-Viewer repository, publishing the demo captures of a pull request with scripts/pr-captures.ps1 is allowed: the script only pushes image files to the dedicated captures branch, never to main (AGENTS.md, section 12).'
)

function Get-ClaudeSettingsFile {
    $dir = if ($env:CLAUDE_CONFIG_DIR) { $env:CLAUDE_CONFIG_DIR } else { Join-Path $HOME '.claude' }
    Join-Path $dir 'settings.json'
}

# The settings with the rules merged in: @{ Json; Added; Error }. Nothing
# already there is removed or reordered; an unreadable file is left alone.
function Merge-ClaudeSettings([string]$File) {
    $settings = [pscustomobject]@{}
    if (Test-Path -LiteralPath $File) {
        $text = [IO.File]::ReadAllText($File)
        if ($text.Trim()) {
            try { $settings = $text | ConvertFrom-Json -ErrorAction Stop }
            catch { return @{ Error = "JSON illisible : $($_.Exception.Message)" } }
            if ($settings -isnot [pscustomobject]) { return @{ Error = 'le fichier ne contient pas un objet JSON' } }
        }
    }
    $added = @()
    # adds the missing values to the array property $Name of $Object
    $addTo = {
        param($Object, [string]$Name, [string[]]$Values, [string[]]$Initial)
        $current = $Object.PSObject.Properties[$Name]
        if (-not $current) {
            $Object | Add-Member -NotePropertyName $Name -NotePropertyValue ([object[]]@($Initial + $Values))
            return $Values
        }
        $list = @($current.Value)
        $missing = @($Values | Where-Object { $_ -notin $list })
        if ($missing) { $current.Value = [object[]]@($list + $missing) }
        $missing
    }
    foreach ($section in 'permissions', 'autoMode') {
        if (-not $settings.PSObject.Properties[$section]) {
            $settings | Add-Member -NotePropertyName $section -NotePropertyValue ([pscustomobject]@{})
        } elseif ($settings.$section -isnot [pscustomobject]) {
            return @{ Error = "« $section » n'est pas un objet" }
        }
    }
    $added += & $addTo $settings.permissions 'allow' $ClaudeAllowRules @()
    # '$defaults' keeps the built-in classifier rules (only for a new list:
    # an existing one without it was trimmed on purpose)
    $added += & $addTo $settings.autoMode 'allow' $ClaudeAutoModeRules @('$defaults')
    @{ Json = ($settings | ConvertTo-Json -Depth 64); Added = @($added | Where-Object { $_ }) }
}

function Invoke-ClaudePermissions {
    $file = Get-ClaudeSettingsFile
    Write-Title 'Permissions de Claude Code pour les PR et les issues des agents'
    Write-Color '  Les agents suivent leurs PR et la CI, postent leur relecture et leurs captures,' muted
    Write-Color '  fusionnent, trient les issues et y répondent eux-mêmes (AGENTS.md) ; sans ces' muted
    Write-Color '  règles, le mode auto de Claude Code peut le leur refuser. Autorisés ensuite :' muted
    foreach ($r in $ClaudeAllowRules) { Write-Color "    • $r" ink }
    Write-Color '  La file de fusion et la protection de main s''appliquent toujours.' muted
    Write-Host ''
    Write-Color "  Fichier : $file" muted
    $merge = Merge-ClaudeSettings $file
    if ($merge.Error) {
        Write-Color "  Rien n'a été modifié : $($merge.Error)" danger
        Write-Color '  Corrige le fichier (ou renomme-le) puis relance.' muted
        Wait-Back
        return
    }
    if (-not $merge.Added) {
        Write-Color '  Déjà en place : rien à faire.' success
        Wait-Back
        return
    }
    Write-Color "  À ajouter ($($merge.Added.Count)) :" ink
    foreach ($a in $merge.Added) { Write-Color "    + $(if ($a.Length -gt 90) { $a.Substring(0, 89) + '…' } else { $a })" teal }
    if ($DryRun) {
        Write-Host ''
        Write-Color '  -DryRun : le fichier n''est pas modifié. Il deviendrait :' warn
        $merge.Json -split "`n" | ForEach-Object { Write-Color "    $_" muted }
        Wait-Back
        return
    }
    if (-not (Read-YesNo 'Mettre à jour le fichier (une copie de l''ancien est gardée) ?')) { return }
    $dir = Split-Path $file -Parent
    if (-not (Test-Path -LiteralPath $dir)) { New-Item -ItemType Directory -Path $dir -Force | Out-Null }
    if (Test-Path -LiteralPath $file) {
        $backup = "$file.aurora-$(Get-Date -Format 'yyyyMMdd-HHmmss').bak"
        Copy-Item -LiteralPath $file -Destination $backup
        Write-Color "  Copie de l'ancien fichier : $backup" muted
    }
    # UTF-8 without BOM: Claude Code reads it as plain JSON
    [IO.File]::WriteAllText($file, $merge.Json, (New-Object Text.UTF8Encoding $false))
    Write-Color '  Fait. Ouvre une nouvelle session de Claude Code pour qu''elle lise ces règles.' success
    Wait-Back
}
