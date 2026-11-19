# Rules of the review slots (HUMANS.md "Review slots", AGENTS.md section 9):
# an outside contributor has at most $PrSlotLimit pull requests in review at
# a time; the others keep the waiting-for-slot label (they stay open as
# they are) and come back on their own when a slot frees up. A label, not a
# draft: the workflow's token may not turn someone else's PR into a draft.
# Pure functions, no GitHub call: scripts/pr-slots.ps1 applies them,
# scripts/test-pr-slots.ps1 tests them.

$PrSlotLimit = 10
$PrSlotLabel = 'waiting-for-slot'
# Marker of the workflow's own comments, at their end.
$PrSlotMarker = '<!-- aurora-review-slot -->'

# One author's open pull requests -> what to do. Each item: Number, Draft,
# Waiting (carries the label), OnHold, Protected (in the merge queue or set
# to merge automatically), Reviewed (a maintainer's review comment), Created
# (sortable). $Preferred: the PR whose label a maintainer has just removed.
#
# - Drafts and PRs on hold take no slot. A draft that still carries the
#   label only loses it.
# - Too many in review: the lowest ranked wait. Rank: protected, the
#   preferred one, those a maintainer started reviewing, then the oldest.
#   Protected ones never wait.
# - Free slots: the oldest waiting ones come back.
# - Nothing is swapped while the count stays at the limit.
# Returns @{ Wait; Back; Unlabel } (PR numbers).
function Get-PrSlotPlan($Items, [int]$Limit = $PrSlotLimit, $Preferred = $null) {
    $active = @($Items | Where-Object { -not $_.OnHold -and -not $_.Draft })
    $ready = @($active | Where-Object { -not $_.Waiting } | Sort-Object `
        @{ Expression = { -not $_.Protected } }, `
        @{ Expression = { $_.Number -ne $Preferred } }, `
        @{ Expression = { -not $_.Reviewed } }, `
        @{ Expression = { $_.Created } }, `
        @{ Expression = { $_.Number } })
    $waiting = @($active | Where-Object { $_.Waiting } | Sort-Object @{ Expression = { $_.Created } }, @{ Expression = { $_.Number } })
    $wait = @()
    $back = @()
    if ($ready.Count -gt $Limit) {
        $wait = @($ready | Select-Object -Skip $Limit | Where-Object { -not $_.Protected } | ForEach-Object { $_.Number })
    } elseif ($ready.Count -lt $Limit) {
        $back = @($waiting | Select-Object -First ($Limit - $ready.Count) | ForEach-Object { $_.Number })
    }
    $unlabel = @($Items | Where-Object { $_.Waiting -and ($_.Draft -or $_.OnHold) } | ForEach-Object { $_.Number })
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
        "Each outside contributor can have up to **$PrSlotLimit pull requests in review** at the same time. You have **$Open** open right now, so this one is **waiting for a review slot** (label ``$PrSlotLabel``). It stays open exactly as it is, nothing is lost, and you do not need to do anything; the maintainers simply do not review it until it comes back.",
        '',
        "- **It comes back on its own.** When one of [your pull requests in review]($($Links.Review)) is merged or closed, the oldest one [waiting]($($Links.Waiting)) goes back to review, and you get a comment here.",
        '- **Rather have this one reviewed first?** Say so in a comment here: a maintainer can give it a slot, and your most recent other pull request in review then waits instead (one a maintainer has started reviewing keeps its slot).',
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
        "Hi @$Author, a review slot is free: this pull request is back in the maintainers' review queue.",
        '',
        $PrSlotMarker
    ) -join "`n"
}

# Search links for the comment: the author's PRs in review, and waiting.
function Get-PrSlotLinks([string]$Repo, [string]$Author) {
    $base = "https://github.com/$Repo/pulls?q=is%3Apr+is%3Aopen+author%3A$Author"
    @{ Review = "$base+-is%3Adraft+-label%3A$PrSlotLabel"; Waiting = "$base+label%3A$PrSlotLabel" }
}
