# Rules of the review slots (HUMANS.md "Review slots", AGENTS.md section 9):
# an outside contributor has at most $PrSlotLimit pull requests in review at
# a time; the others wait as drafts with the waiting-for-slot label and come
# back on their own when a slot frees up. Pure functions, no GitHub call:
# scripts/pr-slots.ps1 applies them, scripts/test-pr-slots.ps1 tests them.

$PrSlotLimit = 10
$PrSlotLabel = 'waiting-for-slot'
# Marker of the workflow's own comments, at their end.
$PrSlotMarker = '<!-- aurora-review-slot -->'

# One author's open pull requests -> what to do. Each item: Number, Draft,
# Waiting (carries the label), OnHold, Protected (in the merge queue or set
# to merge automatically), Reviewed (a maintainer's review comment), Created
# (sortable). $Preferred: the PR its author has just marked ready for review.
#
# - Drafts the author made (no label) and PRs on hold are left alone and
#   take no slot.
# - Too many in review: the lowest ranked wait. Rank: protected, the one
#   just marked ready, those a maintainer started reviewing, then the
#   oldest. Protected ones never wait.
# - Free slots: the oldest waiting ones come back.
# - A PR in review that still carries the label (its author marked it ready
#   while slots were free) only loses the label.
# Returns @{ Wait; Back; Unlabel } (PR numbers).
function Get-PrSlotPlan($Items, [int]$Limit = $PrSlotLimit, $Preferred = $null) {
    $active = @($Items | Where-Object { -not $_.OnHold -and (-not $_.Draft -or $_.Waiting) })
    $ready = @($active | Where-Object { -not $_.Draft } | Sort-Object `
        @{ Expression = { -not $_.Protected } }, `
        @{ Expression = { $_.Number -ne $Preferred } }, `
        @{ Expression = { -not $_.Reviewed } }, `
        @{ Expression = { $_.Created } }, `
        @{ Expression = { $_.Number } })
    $waiting = @($active | Where-Object { $_.Draft -and $_.Waiting } | Sort-Object @{ Expression = { $_.Created } }, @{ Expression = { $_.Number } })
    $wait = @()
    $back = @()
    if ($ready.Count -gt $Limit) {
        $wait = @($ready | Select-Object -Skip $Limit | Where-Object { -not $_.Protected } | ForEach-Object { $_.Number })
    } elseif ($ready.Count -lt $Limit) {
        $back = @($waiting | Select-Object -First ($Limit - $ready.Count) | ForEach-Object { $_.Number })
    }
    $unlabel = @($ready | Where-Object { $_.Waiting -and $wait -notcontains $_.Number } | ForEach-Object { $_.Number })
    @{ Wait = $wait; Back = $back; Unlabel = $unlabel }
}

# The authors whose PRs never wait: the team.
function Test-PrSlotTeam([string]$Association) {
    $Association -in @('OWNER', 'MEMBER', 'COLLABORATOR')
}

# The comment on a PR that starts waiting. $Open: how many PRs the author
# has open (in review and waiting); $Links: @{ Review; Waiting } search URLs.
function Format-PrSlotWait([string]$Author, [int]$Open, $Links) {
    @(
        '## Waiting for a review slot',
        '',
        "Hi @$Author, thank you for this pull request!",
        '',
        "Each outside contributor can have up to **$PrSlotLimit pull requests in review** at the same time. You have **$Open** open right now, so this one was moved to **draft** and waits for a free slot. It is not closed, nothing is lost, and you do not need to do anything.",
        '',
        "- **It comes back on its own.** When one of [your pull requests in review]($($Links.Review)) is merged or closed, the oldest one [waiting]($($Links.Waiting)) goes back to review, and you get a comment here.",
        '- **To have this one reviewed first,** click **Ready for review**: it takes a slot, and your most recent other pull request in review waits instead (one a maintainer has started reviewing keeps its slot).',
        '- **You can keep pushing commits** to it while it waits.',
        '',
        "Why: every change is reviewed by hand, and the team cannot review $Open pull requests from one person at once; those in review would wait weeks. A short queue gets each of them reviewed sooner.",
        '',
        'Thanks for contributing to Aurora!',
        '',
        $PrSlotMarker
    ) -join "`n"
}

# The comment on a PR that comes back to review.
function Format-PrSlotBack([string]$Author) {
    @(
        "Hi @$Author, a review slot is free: this pull request is ready for review again and back in the maintainers' queue.",
        '',
        $PrSlotMarker
    ) -join "`n"
}

# Search links for the comment: the author's PRs in review, and waiting.
function Get-PrSlotLinks([string]$Repo, [string]$Author) {
    $base = "https://github.com/$Repo/pulls?q=is%3Apr+is%3Aopen+author%3A$Author"
    @{ Review = "$base+-is%3Adraft"; Waiting = "$base+label%3A$PrSlotLabel" }
}
