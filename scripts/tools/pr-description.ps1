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

# What each required section is for, told to the author when it is missing
# or empty.
$PrSectionPurpose = @{
    'What it is for'       = 'two or three sentences, from the user''s point of view, on what this PR changes for them'
    'Legacy Viewer origin' = 'whether the change follows the Legacy Viewer, and if so which files and functions (for example `indra/newview/llagent.cpp`, `LLAgent::setFlying`); a plain `No.` is fine'
    'What changes'         = 'short bullets by area (interface, rendering, network...), with any new option and its default value'
    'Tests'                = 'what you ran and its result: `scripts/check.ps1` or its cargo steps, demo scenarios, a test on the grid; a ticked box (`- [x]`) or one line each'
}

# What a body misses, one Markdown sentence each, saying what the check
# found and what to write instead (empty when the body is complete).
function Get-PrDescriptionProblems([string]$Body) {
    $problems = New-Object System.Collections.Generic.List[string]
    $clean = Remove-PrComments $Body
    $issue = '(?im)^\s*(close[sd]?|fix(e[sd])?|resolve[sd]?|refs?)\s*:?\s+(#\d+|https://github\.com/[^\s/]+/[^\s/]+/(issues|pull)/\d+|[\w.-]+/[\w.-]+#\d+)'
    if ($clean -notmatch $issue -and $clean -notmatch '(?im)^\s*no issue\b') {
        $mentioned = @([regex]::Matches($clean, '(?<![\w/&])#(\d+)\b') | ForEach-Object { $_.Groups[1].Value } | Select-Object -Unique)
        if ($clean -match '(?im)^\s*(Closes|Refs)\s*#\s*$') {
            $problems.Add('**Issue line**: the first line is still the template''s `Closes #`, without a number. Write the issue''s number (`Closes #123` if this PR resolves it, `Refs #123` if part of it stays open), or replace the line with `No issue`.')
        } elseif ($mentioned.Count) {
            $n = $mentioned[0]
            $also = ''
            if ($mentioned.Count -gt 1) { $also = " (and one such line for each other issue it addresses: #$(@($mentioned | Select-Object -Skip 1) -join ', #'))" }
            $problems.Add("**Issue line**: no line starts with ``Closes #<n>``, ``Refs #<n>`` or ``No issue``. The description mentions #$n; if this PR addresses that issue, put ``Refs #$n`` on the first line (``Closes #$n`` if it resolves it fully)$also. If it is not an issue this PR works on, write ``No issue``.")
        } else {
            $problems.Add('**Issue line**: no line starts with `Closes #<n>`, `Refs #<n>` or `No issue`. Put one on the first line; `No issue` is perfectly fine when there is none.')
        }
    }
    $sections = Get-PrSections $Body
    $found = @($sections.Keys | Where-Object { $_ })
    $missing = @($PrRequiredSections | Where-Object { $null -eq (Find-PrSection $sections $_) })
    if (-not $found.Count) {
        $problems.Add("**Sections**: the description has no ``##`` heading at all, so none of the template's sections can be found. The quickest fix is to paste the [template](TEMPLATE_URL) into the description and fill in ``## What it is for``, ``## Legacy Viewer origin``, ``## What changes`` and ``## Tests``; your current text can go under the first one.")
        $missing = @()
    }
    foreach ($name in $PrRequiredSections) {
        if ($missing -contains $name) {
            $seen = ($found | ForEach-Object { "``## $_``" }) -join ', '
            $problems.Add("**``## $name``**: this heading is missing (headings found: $seen). Add it with this exact wording, followed by $($PrSectionPurpose[$name]).")
            continue
        }
        $key = Find-PrSection $sections $name
        if ($null -eq $key -or (Get-PrOwnLines $sections[$key])) { continue }
        $lines = @(($sections[$key] -split "\r?\n") | ForEach-Object { $_.Trim() } | Where-Object { $_ })
        if ($name -eq 'Tests' -and ($lines | Where-Object { $_ -match '^[-*+]\s*\[\s\]' })) {
            $problems.Add("**``## $key``**: every box is still unticked, as in the template, so a reviewer cannot tell what was run. Tick (``- [x]``) the checks you ran and add their result, or write one line per check (for example ``- cargo test --workspace: all passed``).")
        } elseif ($lines | Where-Object { $_ -match '^[-*+]\s*\*\*[^*]+:\*\*$' }) {
            $bullets = ($lines | ForEach-Object { '`' + $_ + '`' }) -join ', '
            $problems.Add("**``## $key``**: it only holds the template's empty bullets ($bullets). Complete them, or replace them with $($PrSectionPurpose[$name]).")
        } else {
            $problems.Add("**``## $key``**: the heading is there, but nothing is written under it (the template's guidance is a hidden comment and does not count). Please add $($PrSectionPurpose[$name]).")
        }
    }
    # one string per problem; call it as @(Get-PrDescriptionProblems ...)
    $problems.ToArray()
}

# The authors whose PR is checked but never closed: the team.
function Test-PrTeamAuthor([string]$Association) {
    $Association -in @('OWNER', 'MEMBER', 'COLLABORATOR')
}

# The check's comment while the description is incomplete. $Author: the
# PR author's login, or $null. $Deadline: the date (yyyy-MM-dd) an outside
# contributor's PR is closed, or $null for the team.
function Format-PrDescriptionComment([string[]]$Problems, [string]$TemplateUrl, $Deadline, $Author) {
    $hello = 'Hi, and thank you for this pull request!'
    if ($Author) { $hello = "Hi @$Author, and thank you for this pull request!" }
    $count = 'one thing'
    if ($Problems.Count -gt 1) { $count = "$($Problems.Count) things" }
    $lines = @(
        '## Changes requested',
        '',
        $hello,
        '',
        "Before a maintainer starts the review, its description needs $count from the [pull request template]($TemplateUrl). With the number of pull requests we receive, reviewers rely on the description to understand in a couple of minutes what a change does, where it comes from and how it was tested, so the code is not reviewed until the description is complete.",
        '',
        'Here is exactly what the automatic check found:',
        ''
    )
    $i = 0
    foreach ($p in $Problems) { $i++; $lines += "$i. $($p.Replace('TEMPLATE_URL', $TemplateUrl))" }
    $lines += @(
        '',
        '**How to fix it:** edit the description of this pull request (the **...** menu of its first message, then **Edit**). No new commit is needed: the check runs again on its own, and this comment disappears once everything is there.'
    )
    if ($Deadline) {
        $lines += @('', "If the description is still incomplete on **$Deadline**, this pull request will be closed automatically, to keep the review queue readable. Nothing is lost: you can reopen it, or open a new one, as soon as the description is complete.")
    }
    $lines += @('', 'Thanks again for contributing to Aurora!', '', $PrDescriptionMarker)
    $lines -join "`n"
}

function Format-PrDescriptionClose([string]$TemplateUrl, $Author) {
    $hello = 'Hi,'
    if ($Author) { $hello = "Hi @$Author," }
    @(
        "$hello this pull request is now closed automatically: $PrDescriptionGraceDays days after the comment above, its description still misses the parts it lists.",
        '',
        "We do not review a change without knowing what it does and how it was tested. Complete the description from the [pull request template]($TemplateUrl), then reopen this pull request (or open a new one) to have it reviewed.",
        '',
        $PrDescriptionMarker
    ) -join "`n"
}
