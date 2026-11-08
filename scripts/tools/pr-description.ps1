# Rules of the PR description check (AGENTS.md section 9, HUMANS.md "PR
# description check"): what a pull request's body must hold so that a
# maintainer reads it. They follow .github/pull_request_template.md: change
# both together. Pure functions, no GitHub call: scripts/check-pr-description.ps1
# applies them, scripts/test-pr-description.ps1 tests them.

# The template's sections that must hold text of the author's own. The
# others (Screenshots, Limits and follow-up) may stay empty.
$PrRequiredSections = @('What it is for', 'Legacy Viewer origin', 'What changes', 'Tests')

# Marker of the check's own comment, at its end: the comment opens with
# "## Changes requested", which the Aurora Tools PR panel reads as "waiting on
# the author" (scripts/tools/prs.ps1).
$PrDescriptionMarker = '<!-- aurora-pr-description -->'
$PrDescriptionLabel = 'needs-description'
# An outside contributor's PR whose description is still incomplete this
# long after the check's first comment is closed.
$PrDescriptionGraceDays = 3

function Remove-PrComments([string]$Text) {
    if (-not $Text) { return '' }
    [regex]::Replace($Text, '(?s)<!--.*?-->', '')
}

# The "## " (or "### ") sections of a body: heading (trimmed) -> text below
# it, up to the next such heading. Text before the first heading is under ''.
function Get-PrSections([string]$Body) {
    $sections = [ordered]@{ '' = '' }
    $current = ''
    foreach ($line in ((Remove-PrComments $Body) -split "\r?\n")) {
        if ($line -match '^\s{0,3}#{2,3}\s+(.+?)\s*#*\s*$') {
            $current = $Matches[1].Trim()
            if (-not $sections.Contains($current)) { $sections[$current] = '' }
            continue
        }
        $sections[$current] += "$line`n"
    }
    $sections
}

# The lines of a section that the author wrote: without the template's
# placeholders (an area bullet with nothing after it, a Tests box left as the
# template has it, an empty bullet, an issue line with no number). An
# unticked box the author completed ("Validated by the human before the PR:
# 2026-11-08, on the grid") counts.
function Get-PrOwnLines([string]$Text) {
    @(($Text -split "\r?\n") | Where-Object {
        $l = $_.Trim()
        $l -and
        $l -notmatch '^[-*+]$' -and
        $l -notmatch '^[-*+]\s*\*\*[^*]+:\*\*\s*$' -and
        $l -notmatch '^[-*+]\s*\[\s\]\s*[^:]*:$' -and
        $l -notmatch '^[-*+]\s*\[\s\]\s*`scripts/check\.ps1` green \(fmt, clippy, tests\)$' -and
        $l -notmatch '^(Closes|Refs)\s*#\s*$'
    })
}

# The heading of a required section: the template's own, or a longer one
# that starts with it ("Tests and validation"), or the past tense of
# "What changes".
function Find-PrSection($Sections, [string]$Name) {
    $names = @($Name)
    if ($Name -eq 'What changes') { $names += 'What changed' }
    foreach ($key in $Sections.Keys) {
        foreach ($n in $names) {
            if ($key -ieq $n -or $key -like "$n *" -or $key -like "${n}:*") { return $key }
        }
    }
    $null
}

# What a body misses, one sentence each (empty when the body is complete).
function Get-PrDescriptionProblems([string]$Body) {
    $problems = New-Object System.Collections.Generic.List[string]
    $clean = Remove-PrComments $Body
    $issue = '(?im)^\s*(close[sd]?|fix(e[sd])?|resolve[sd]?|refs?)\s*:?\s+(#\d+|https://github\.com/[^\s/]+/[^\s/]+/(issues|pull)/\d+|[\w.-]+/[\w.-]+#\d+)'
    if ($clean -notmatch $issue -and $clean -notmatch '(?im)^\s*no issue\b') {
        $problems.Add('The issue line: `Closes #<n>` or `Refs #<n>` (one line per issue), or `No issue`.')
    }
    $sections = Get-PrSections $Body
    foreach ($name in $PrRequiredSections) {
        $key = Find-PrSection $sections $name
        if ($null -eq $key) {
            $problems.Add("The ``## $name`` section, with the template's heading.")
            continue
        }
        if (Get-PrOwnLines $sections[$key]) { continue }
        if ($name -eq 'Tests') {
            $problems.Add('Under `## Tests`, what you ran and its result: `scripts/check.ps1` or its cargo steps, demo scenarios, a test on the grid (a ticked box or a line each).')
        } else {
            $problems.Add("Text of your own under ``## $name``.")
        }
    }
    # one string per problem; call it as @(Get-PrDescriptionProblems ...)
    $problems.ToArray()
}

# The authors whose PR is checked but never closed: the team.
function Test-PrTeamAuthor([string]$Association) {
    $Association -in @('OWNER', 'MEMBER', 'COLLABORATOR')
}

# The check's comment while the description is incomplete. $Deadline: the
# date (yyyy-MM-dd) an outside contributor's PR is closed, or $null.
function Format-PrDescriptionComment([string[]]$Problems, [string]$TemplateUrl, $Deadline) {
    $lines = @(
        '## Changes requested',
        '',
        "The description of this pull request does not follow the [pull request template]($TemplateUrl) yet, so the maintainers have not started its review. It misses:",
        ''
    )
    $lines += @($Problems | ForEach-Object { "- $_" })
    $lines += @(
        '',
        'Edit the description (no new commit needed): this check runs again on its own, and this comment goes away once it is complete. A short sentence is enough when a section does not apply (for example `Legacy Viewer origin: no`).'
    )
    if ($Deadline) {
        $lines += @('', "If it is still incomplete on $Deadline, the pull request is closed. You can then reopen it, or open a new one, with the description completed.")
    }
    $lines += @('', $PrDescriptionMarker)
    $lines -join "`n"
}

function Format-PrDescriptionClose([string]$TemplateUrl) {
    @(
        "Closed: the description still misses the parts listed above, $PrDescriptionGraceDays days after they were asked for. Complete it from the [pull request template]($TemplateUrl), then reopen this pull request or open a new one: it is welcome back.",
        '',
        $PrDescriptionMarker
    ) -join "`n"
}
