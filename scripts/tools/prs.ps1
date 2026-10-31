# Aurora Tools: pull requests followed live — background refresh from GitHub,
# footer summary, Windows notifications, full panel (dot-sourced by
# aurora-tools.ps1).

# One GraphQL query per refresh (one point of the 5000/hour quota the agents
# share with the tools): open PRs with their CI, merge queue entry and review
# comment, the last merged ones, and main's CI.
$PrQuery = @'
query($o: String!, $r: String!) {
  repository(owner: $o, name: $r) {
    defaultBranchRef { target { ... on Commit { statusCheckRollup { state } } } }
    open: pullRequests(states: OPEN, first: 30, orderBy: {field: UPDATED_AT, direction: DESC}) {
      nodes {
        number title url isDraft additions deletions reviewDecision
        author { login }
        labels(first: 20) { nodes { name } }
        mergeQueueEntry { position state enqueuedAt }
        autoMergeRequest { enabledAt }
        commits(last: 1) { nodes { commit { committedDate statusCheckRollup {
          state
          contexts(first: 20) { nodes {
            ... on CheckRun { name status conclusion startedAt }
            ... on StatusContext { context state }
          } }
        } } } }
        comments(last: 10) { nodes { body createdAt } }
      }
    }
    merged: pullRequests(states: MERGED, first: 5, orderBy: {field: UPDATED_AT, direction: DESC}) {
      nodes { number title url mergedAt additions deletions author { login } }
    }
    latestRelease { tagName releaseAssets(first: 30) { nodes { name downloadCount } } }
  }
}
'@

# A PR a human put on hold carries this label (AGENTS.md section 9): the
# tools hide it until the label is removed, unless it is in the merge queue.
$OnHoldLabel = 'on-hold'

# Shared with the background runspace: the last answer, its time, an error.
$GitHub = [hashtable]::Synchronized(@{ Data = $null; Error = $null; Updated = $null; Version = 0; Force = $false; Stop = $false })
$GitHubWatch = $null
$PrSeen = $null       # last snapshot, to notice what changed (notifications)
$PrSeenVersion = 0

# owner/name of the repository, from its origin remote
function Get-RepoSlug {
    $origin = (git -C $Paths.Repo remote get-url origin 2>$null)
    if ($origin -match 'github\.com[:/](?<owner>[^/]+)/(?<name>[^/]+?)(\.git)?$') {
        [pscustomobject]@{ Owner = $Matches.owner; Name = $Matches.name; Slug = "$($Matches.owner)/$($Matches.name)" }
    }
}

# Refreshes $GitHub in the background: every 10 s while a CI runs or a PR
# waits in the merge queue, every 20 s otherwise, at once when Force is set.
function Start-GitHubWatch {
    if ($script:GitHubWatch -or -not (Get-Command gh -ErrorAction SilentlyContinue)) { return }
    $repo = Get-RepoSlug
    if (-not $repo) { return }
    gh auth status *> $null
    if ($LASTEXITCODE -ne 0) { $GitHub.Error = 'not logged in to GitHub'; return }
    $ps = [powershell]::Create()
    [void]$ps.AddScript({
        param($State, $Query, $Owner, $Name)
        while (-not $State.Stop) {
            try {
                $out = gh api graphql -f "query=$Query" -F "o=$Owner" -F "r=$Name" 2>&1
                if ($LASTEXITCODE -eq 0) {
                    $State.Data = ($out -join "`n") | ConvertFrom-Json
                    $State.Error = $null
                } else {
                    $State.Error = [string]($out | Select-Object -Last 1)
                }
            } catch {
                $State.Error = $_.Exception.Message
            }
            $State.Updated = Get-Date
            $State.Version++
            $busy = $false
            if ($State.Data) {
                foreach ($p in $State.Data.data.repository.open.nodes) {
                    $rollup = $p.commits.nodes[0].commit.statusCheckRollup
                    if ($p.mergeQueueEntry -or ($rollup -and $rollup.state -in 'PENDING', 'EXPECTED')) { $busy = $true }
                }
            }
            $ticks = if ($busy) { 40 } else { 80 }
            for ($i = 0; $i -lt $ticks -and -not $State.Stop -and -not $State.Force; $i++) { Start-Sleep -Milliseconds 250 }
            $State.Force = $false
        }
    }).AddArgument($GitHub).AddArgument($PrQuery).AddArgument($repo.Owner).AddArgument($repo.Name)
    [void]$ps.BeginInvoke()
    $script:GitHubWatch = $ps
}

function Stop-GitHubWatch {
    $GitHub.Stop = $true
    if ($script:GitHubWatch) { $script:GitHubWatch.Dispose(); $script:GitHubWatch = $null }
}

function Format-Duration([TimeSpan]$T) {
    if ($T.TotalHours -ge 1) { '{0}h{1:00}' -f [int][Math]::Floor($T.TotalHours), $T.Minutes }
    else { '{0}:{1:00}' -f [int][Math]::Floor($T.TotalMinutes), $T.Seconds }
}

# Short age for a column: '55 sec', '04 min', '02 hrs', '03 day', '02 wks',
# '05 mos', '01 yrs'. Units all take 3 letters so numbers and units line up
# from one row to the next.
function Format-Age([datetime]$When) {
    $t = (Get-Date) - $When
    if ($t.TotalMinutes -lt 1) { '{0:00} sec' -f [int][Math]::Floor($t.TotalSeconds) }
    elseif ($t.TotalHours -lt 1) { '{0:00} min' -f [int][Math]::Floor($t.TotalMinutes) }
    elseif ($t.TotalDays -lt 1) { '{0:00} hrs' -f [int][Math]::Floor($t.TotalHours) }
    elseif ($t.TotalDays -lt 7) { '{0:00} day' -f [int][Math]::Floor($t.TotalDays) }
    elseif ($t.TotalDays -lt 30) { '{0:00} wks' -f [int][Math]::Floor($t.TotalDays / 7) }
    elseif ($t.TotalDays -lt 365) { '{0:00} mos' -f [int][Math]::Floor($t.TotalDays / 30) }
    else { '{0:00} yrs' -f [int][Math]::Floor($t.TotalDays / 365) }
}

function Format-Ago([datetime]$When) {
    $t = (Get-Date) - $When
    if ($t.TotalMinutes -lt 1) { "$([int]$t.TotalSeconds) s ago" }
    elseif ($t.TotalHours -lt 1) { "$([int]$t.TotalMinutes) min ago" }
    elseif ($t.TotalDays -lt 1) { "$([int]$t.TotalHours) h ago" }
    else { "$([int]$t.TotalDays) d ago" }
}

# What a pull request looks like on screen.
function Get-PrRows {
    $repo = if ($GitHub.Data) { $GitHub.Data.data.repository }
    if (-not $repo) { return @() }
    foreach ($p in $repo.open.nodes) {
        # CI: Ok, Fail, Run or None; its label is a short tag ('CI OK')
        $rollup = $p.commits.nodes[0].commit.statusCheckRollup
        $ciState = 'None'; $ci = 'NO CI'; $ciColor = 'muted_dim'; $since = $null
        if ($rollup) {
            switch ($rollup.state) {
                'SUCCESS' { $ciState = 'Ok'; $ci = 'CI OK'; $ciColor = 'success' }
                { $_ -in 'FAILURE', 'ERROR' } {
                    $failed = @($rollup.contexts.nodes | Where-Object { $_.conclusion -in 'FAILURE', 'TIMED_OUT', 'CANCELLED' -or $_.state -in 'FAILURE', 'ERROR' } | ForEach-Object { if ($_.name) { $_.name } else { $_.context } })
                    $ciState = 'Fail'; $ci = 'CI FAIL'; $ciColor = 'danger'
                    if ($failed) { $ci = "CI FAIL $($failed[0])" }
                }
                default {
                    $ciState = 'Run'; $ci = 'CI RUN'; $ciColor = 'amber'
                    $starts = @($rollup.contexts.nodes | Where-Object { $_.status -in 'IN_PROGRESS', 'QUEUED' -and $_.startedAt } | ForEach-Object { [datetime]$_.startedAt })
                    if ($starts) { $since = ($starts | Sort-Object | Select-Object -First 1) }
                }
            }
        }
        # merge queue: Queued, Blocked, Auto (auto-merge requested) or ''
        $queueState = ''; $queue = ''; $queueColor = 'muted_dim'; $queueSince = $null; $position = 0
        if ($p.mergeQueueEntry) {
            $queueSince = [datetime]$p.mergeQueueEntry.enqueuedAt
            $position = [int]$p.mergeQueueEntry.position
            $queueState = 'Queued'; $queueColor = 'teal'
            $queue = switch ($p.mergeQueueEntry.state) {
                'AWAITING_CHECKS' { "QUEUE #$position TESTS" }
                'MERGEABLE' { "QUEUE #$position MERGING" }
                'UNMERGEABLE' { 'QUEUE BLOCKED' }
                default { "QUEUE #$position" }
            }
            if ($p.mergeQueueEntry.state -eq 'UNMERGEABLE') { $queueState = 'Blocked'; $queueColor = 'danger' }
        } elseif ($p.autoMergeRequest) {
            $queueState = 'Auto'; $queue = 'AUTO-MERGE'; $queueColor = 'teal'
        }
        # the agent's review comment: Ok, Warn, Fail, Done (no verdict read) or Pending
        # (PRs opened before the switch to English: "## Relecture", "Verdict : ✅")
        $review = $p.comments.nodes | Where-Object { $_.body -match '^\s*## (Review|Relecture)' } | Select-Object -Last 1
        $reviewState = 'Pending'
        if ($review) {
            $reviewState = if ($review.body -match 'Verdict ?: ✅') { 'Ok' }
                elseif ($review.body -match 'Verdict ?: ⚠') { 'Warn' }
                elseif ($review.body -match 'Verdict ?: ❌') { 'Fail' }
                else { 'Done' }
        }
        $verdict, $verdictColor = switch ($reviewState) {
            'Ok' { 'REVIEW OK', 'success' }
            'Warn' { 'REVIEW WARN', 'warn' }
            'Fail' { 'REVIEW FAIL', 'danger' }
            'Done' { 'REVIEWED', 'muted' }
            default { 'REVIEW PENDING', 'violet_light' }
        }
        # waiting on its author: a draft, a red CI, or changes asked for since
        # its last commit (a GitHub review requesting changes, or a comment
        # opening with "## Maintainer review" or "## Changes requested" that
        # the agent's later "## Review" ✅ has not answered). A new commit
        # brings it back among the PRs to review.
        $lastCommit = $p.commits.nodes[0].commit.committedDate
        $lastWord = $p.comments.nodes | Where-Object { $_.body -match '^\s*## (Review|Relecture|Maintainer review|Changes requested)' } | Select-Object -Last 1
        $asked = $lastWord -and $lastWord.body -match '^\s*## (Maintainer review|Changes requested)' -and $lastWord.body -notmatch 'Verdict ?: ✅' -and
            (-not $lastCommit -or [datetime]$lastWord.createdAt -gt [datetime]$lastCommit)
        $waiting = if ($p.mergeQueueEntry -or $p.autoMergeRequest) { '' }
            elseif ($p.isDraft) { 'DRAFT' }
            elseif ($ciState -eq 'Fail') { 'CI FAIL' }
            elseif ($asked -or $p.reviewDecision -eq 'CHANGES_REQUESTED') { 'CHANGES ASKED' }
            else { '' }
        # one colour for the whole PR (the square before its number), worst first
        $status = if ($p.isDraft) { 'muted_dim' }
            elseif ($ciState -eq 'Fail' -or $queueState -eq 'Blocked' -or $reviewState -eq 'Fail') { 'danger' }
            elseif ($queueState) { 'teal' }
            elseif ($ciState -eq 'Run') { 'amber' }
            elseif ($reviewState -eq 'Warn') { 'warn' }
            elseif ($ciState -eq 'Ok' -and $reviewState -eq 'Ok') { 'success' }
            else { 'violet_light' }
        [pscustomobject]@{
            Number = $p.number; Title = $p.title; Url = $p.url; Author = $p.author.login; Draft = $p.isDraft
            Additions = [int]$p.additions; Deletions = [int]$p.deletions
            CiState = $ciState; Ci = $ci; CiColor = $ciColor; CiRunning = $ciState -eq 'Run'; CiSince = $since
            QueueState = $queueState; Queue = $queue; QueueColor = $queueColor; QueueSince = $queueSince; QueuePosition = $position
            ReviewState = $reviewState; Verdict = $verdict; VerdictColor = $verdictColor; Status = $status
            Waiting = $waiting
            OnHold = -not $queueState -and [bool]@($p.labels.nodes | Where-Object { $_.name -eq $OnHoldLabel }).Count
        }
    }
}

# Downloads of the Windows zip of the latest stable release (GitHub's
# latestRelease skips pre-releases, so the nightly betas never count), and
# how many since the tools opened: @{ Tag; Count; Since } or $null.
$DownloadsAtStart = $null
function Get-WindowsDownloads {
    $release = if ($GitHub.Data) { $GitHub.Data.data.repository.latestRelease }
    if (-not $release) { return $null }
    $asset = $release.releaseAssets.nodes | Where-Object { $_.name -match '-windows-x64\.zip$' } | Select-Object -First 1
    if (-not $asset) { return $null }
    $count = [int]$asset.downloadCount
    if (-not $script:DownloadsAtStart -or $script:DownloadsAtStart.Tag -ne $release.tagName) {
        $script:DownloadsAtStart = @{ Tag = $release.tagName; Count = $count }
    }
    [pscustomobject]@{ Tag = $release.tagName; Count = $count; Since = $count - $script:DownloadsAtStart.Count }
}

# The square drawn before a count or a PR number, in the status colour.
$StatusMark = '■'

# Footer, as coloured segments: "PR ■ 3 open ■ 1 CI ■ 1 queued ■ 1 ready
# ■ 1 red", or why there is nothing.
function Get-PrSummary {
    if (-not $GitHub.Data) {
        if ($GitHub.Error) { return , @("GitHub: $($GitHub.Error)", 'warn') }
        return , @('GitHub…', 'muted')
    }
    $rows = @(Get-PrRows | Where-Object { -not $_.OnHold })
    if (-not $rows) { return , @('PR: none open', 'muted') }
    $counts = @(
        @(@($rows | Where-Object { -not $_.Waiting -and -not $_.QueueState }).Count, 'to review', 'violet_light'),
        @(@($rows | Where-Object Waiting).Count, 'waiting', 'muted'),
        @(@($rows | Where-Object CiRunning).Count, 'CI', 'amber'),
        @(@($rows | Where-Object { $_.QueueState -in 'Queued', 'Blocked' }).Count, 'queued', 'teal'),
        @(@($rows | Where-Object { $_.Status -eq 'success' }).Count, 'ready', 'success'),
        @(@($rows | Where-Object { $_.CiState -eq 'Fail' }).Count, 'red', 'danger')
    )
    $segments = @(, @('PR', 'muted'))
    foreach ($c in $counts) {
        if (-not $c[0]) { continue }
        $segments += , @("  $StatusMark ", $c[2])
        $segments += , @("$($c[0]) $($c[1])", 'ink')
    }
    $dl = Get-WindowsDownloads
    if ($dl) { $segments += , @("   ↓ $($dl.Count.ToString('N0')) $($dl.Tag)", 'teal') }
    $segments
}

# A Windows notification (toast); a beep when they are not available.
function Send-Toast([string]$Title, [string]$Text) {
    try {
        [void][Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime]
        [void][Windows.Data.Xml.Dom.XmlDocument, Windows.Data.Xml.Dom.XmlDocument, ContentType = WindowsRuntime]
        $xml = New-Object Windows.Data.Xml.Dom.XmlDocument
        $t = [Security.SecurityElement]::Escape($Title); $b = [Security.SecurityElement]::Escape($Text)
        $xml.LoadXml("<toast><visual><binding template='ToastGeneric'><text>$t</text><text>$b</text></binding></visual></toast>")
        # the AppUserModelID of Windows PowerShell: notifications show under its name
        $app = '{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\WindowsPowerShell\v1.0\powershell.exe'
        [Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier($app).Show([Windows.UI.Notifications.ToastNotification]::new($xml))
    } catch {
        [Console]::Beep(880, 150)
    }
}

# Called every second (Read-Key's $TickHook): after each refresh, notifies
# what changed since the previous one.
function Update-PrNotifications {
    if ($GitHub.Version -eq $script:PrSeenVersion -or -not $GitHub.Data) { return }
    $script:PrSeenVersion = $GitHub.Version
    $now = @{}
    foreach ($r in Get-PrRows) { $now[[int]$r.Number] = $r }
    $repo = $GitHub.Data.data.repository
    $mainState = $repo.defaultBranchRef.target.statusCheckRollup.state
    if ($null -ne $script:PrSeen) {
        $before = $script:PrSeen.Rows
        foreach ($n in $before.Keys) {
            if (-not $now.ContainsKey($n)) {
                $m = $repo.merged.nodes | Where-Object { $_.number -eq $n } | Select-Object -First 1
                if ($m) { Send-Toast "PR #$n merged" $m.title }
                continue
            }
            $was = $before[$n]; $is = $now[$n]
            # on hold: silent, but kept in the snapshot, so taking it off
            # hold is not reported as a new PR
            if ($is.OnHold) { continue }
            if ($is.CiState -eq 'Fail' -and $was.CiState -ne 'Fail') { Send-Toast "CI red on PR #$n" $is.Title }
            if ($was.QueueState -in 'Queued', 'Blocked' -and $is.QueueState -notin 'Queued', 'Blocked') { Send-Toast "PR #$n left the merge queue" $is.Title }
        }
        foreach ($n in $now.Keys) {
            if (-not $before.ContainsKey($n) -and -not $now[$n].OnHold) { Send-Toast "New PR #$n ($($now[$n].Author))" $now[$n].Title }
        }
        if ($mainState -in 'FAILURE', 'ERROR' -and $script:PrSeen.Main -notin 'FAILURE', 'ERROR') {
            Send-Toast 'main is red' 'CI failed on main: check the last merge.'
        }
    }
    $script:PrSeen = @{ Rows = $now; Main = $mainState }
}

# --- Panel ------------------------------------------------------------------------
# Redrawn in place every second (timers move), adapted to the window size so
# it can stay open in a corner of the screen. A frame never has more lines
# than the window and its last line ends without a newline: otherwise the
# console scrolls at each frame and the bottom lines come out twice.
$PanelRow = 0                  # lines of the current frame written so far
$PanelMax = [int]::MaxValue    # lines the window can show

# Text cut to fit a column of $Width characters (with one space after).
function Format-Cell([string]$Text, [int]$Width) {
    if ($Text.Length -ge $Width) { $Text = $Text.Substring(0, $Width - 2) + '…' }
    $Text.PadRight($Width)
}

function Write-PanelLine([object[]]$Segments, [int]$Width, [switch]$Selected) {
    if ($script:PanelRow -ge $script:PanelMax) { return }
    # the newline before the line, not after: none after the last one
    if ($script:PanelRow -gt 0) { Write-Host '' }
    $script:PanelRow++
    # @(@('text', 'color')) is unrolled by PowerShell into a single segment
    if ($Segments.Count -gt 0 -and $Segments[0] -isnot [array]) { $Segments = @(, $Segments) }
    # selected: a violet bar in the first column and a dark grey background,
    # every segment keeping its own colour (green CI stays green)
    $used = 0
    if ($Selected -and $Width -gt 0) {
        Write-Color '▌' violet -Background raised -NoNewline
        $used = 1
        $first = [string]$Segments[0][0]
        if ($first.StartsWith(' ')) { $Segments = @(, @($first.Substring(1), $Segments[0][1])) + @($Segments | Select-Object -Skip 1) }
    }
    foreach ($s in $Segments) {
        $text = [string]$s[0]
        if ($used + $text.Length -gt $Width) { $text = $text.Substring(0, [Math]::Max(0, $Width - $used)) }
        if ($text) {
            # a segment's third item is a link: Ctrl + click opens it in
            # Windows Terminal (OSC 8); the escapes take no column
            $link = if ($Vt -and $s.Count -gt 2 -and $s[2]) { [string]$s[2] }
            if ($link) { Write-Host "$Esc]8;;$link$Esc\" -NoNewline }
            if ($Selected) { Write-Color $text $s[1] -Background raised -NoNewline }
            else { Write-Color $text $s[1] -NoNewline }
            if ($link) { Write-Host "$Esc]8;;$Esc\" -NoNewline }
        }
        $used += $text.Length
    }
    $pad = ' ' * [Math]::Max(0, $Width - $used)
    if ($Selected) { Write-Color $pad ink -Background raised -NoNewline } else { Write-Host $pad -NoNewline }
}

# The PRs in the panel's order: the merge queue (by position, auto-merge
# requests after), then the others as GitHub sorts them (last updated first).
# The PRs on hold are left out.
function Get-PanelRows {
    $rows = @(Get-PrRows | Where-Object { -not $_.OnHold })
    foreach ($r in $rows) { $r | Add-Member -NotePropertyName InQueue -NotePropertyValue ([bool]$r.QueueState) -Force }
    $position = { if ($_.QueueState -eq 'Auto') { 1000 } else { $_.QueuePosition } }
    @($rows | Where-Object InQueue | Sort-Object $position) + @($rows | Where-Object { -not $_.InQueue -and -not $_.Waiting }) +
        @($rows | Where-Object { -not $_.InQueue -and $_.Waiting })
}

function Show-PrPanel {
    $sel = 0
    $first = 0                 # first body line shown when they do not all fit
    $lastDraw = [DateTime]::MinValue
    $lastHeight = 0
    $size = if ($Interactive) { Get-WindowSize }
    $sizeAt = [DateTime]::MinValue
    $GitHub.Force = $true
    if ($Interactive) { Clear-Host; [Console]::CursorVisible = $false }
    else {
        # one frame only: wait (15 s at most) for the first answer
        $deadline = (Get-Date).AddSeconds(15)
        while ($GitHub.Version -eq 0 -and (Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 200 }
    }
    try {
        while ($true) {
            if ($Interactive -and [Console]::KeyAvailable) {
                $k = [Console]::ReadKey($true)
                $rows = @(Get-PanelRows)
                switch ($k.Key) {
                    'UpArrow' { if ($rows) { $sel = ($sel - 1 + $rows.Count) % $rows.Count } }
                    'DownArrow' { if ($rows) { $sel = ($sel + 1) % $rows.Count } }
                    { $_ -in 'Escape', 'Backspace' } { return }
                    { $_ -in 'Enter', 'O' } { if ($rows) { Start-Process $rows[[Math]::Min($sel, $rows.Count - 1)].Url } }
                    'R' { $GitHub.Force = $true }
                }
                $lastDraw = [DateTime]::MinValue
            }
            # resized: the console has reflowed the old frame, start from a
            # clear screen once the size holds still
            if ($Interactive) {
                $now = Get-WindowSize
                if ($now -ne $size) { $size = $now; $sizeAt = Get-Date; $resized = $true }
                if ($resized) {
                    if (((Get-Date) - $sizeAt).TotalMilliseconds -lt 200) { Start-Sleep -Milliseconds 50; continue }
                    Clear-Host; [Console]::CursorVisible = $false
                    $resized = $false; $lastHeight = 0; $lastDraw = [DateTime]::MinValue
                }
            }
            if (((Get-Date) - $lastDraw).TotalMilliseconds -lt 1000) { Start-Sleep -Milliseconds 50; continue }
            $lastDraw = Get-Date
            Update-PrNotifications

            $w = if ($Interactive) { $Host.UI.RawUI.WindowSize.Width - 1 } else { 110 }
            $h = if ($Interactive) { $Host.UI.RawUI.WindowSize.Height } else { [int]::MaxValue }
            $compact = $w -lt 96
            $rows = @(Get-PanelRows)
            if ($sel -ge $rows.Count) { $sel = [Math]::Max(0, $rows.Count - 1) }
            if ($Interactive) { [Console]::SetCursorPosition(0, 0) }
            $script:PanelRow = 0
            $script:PanelMax = $h

            $mainState = if ($GitHub.Data) { $GitHub.Data.data.repository.defaultBranchRef.target.statusCheckRollup.state }
            # switch skips a null value entirely, default included: test it first
            $main = if (-not $mainState) { '…', 'muted' } else { switch ($mainState) { 'SUCCESS' { 'CI OK', 'success' } { $_ -in 'FAILURE', 'ERROR' } { 'CI FAIL', 'danger' } { $_ -in 'PENDING', 'EXPECTED' } { 'CI RUN', 'amber' } default { '?', 'muted' } } }
            $fresh = if ($GitHub.Updated) { Format-Ago $GitHub.Updated } else { 'loading…' }
            $header = @(@(' Aurora · PR', 'violet_light'), @('   main  ', 'muted'), @("$StatusMark $($main[0])", $main[1]))
            # the Windows downloads of the latest stable release, live
            $dl = Get-WindowsDownloads
            if ($dl) {
                $header += , @("   ↓ $($dl.Count.ToString('N0')) Windows $($dl.Tag)", 'teal')
                if ($dl.Since -gt 0) { $header += , @(" (+$($dl.Since))", 'success') }
            }
            $header += @(@("   updated $fresh", 'muted_dim'), @("   $(Get-Date -Format 'HH:mm:ss')", 'muted_dim'))
            Write-PanelLine $header $w
            if ($GitHub.Error) { Write-PanelLine @(@(" GitHub: $($GitHub.Error)", 'warn')) $w }
            Write-PanelLine @(, @((' ' + ('─' * ($w - 2))), 'muted_dim')) $w

            # The body, as lines to write (S: segments, Pr: index of the PR in
            # $rows on its first line, -1 elsewhere), in three sections one
            # above the other: the titles keep the whole width.
            $body = [Collections.Generic.List[object]]::new()
            $line = { param($Segments, $Pr = -1) $body.Add([pscustomobject]@{ S = $Segments; Pr = $Pr }) }
            # a status tag, '■ CI OK': the square and the text in its colour
            $tag = { param($Text, $Color, $Width = 0) @("$StatusMark ", $Color), @($(if ($Width) { Format-Cell $Text ($Width - 2) } else { $Text }), $Color) }
            $heading = { param($Text, $Count) & $line @(@(" $Text", 'violet_light'), @($(if ($null -ne $Count) { " ($Count)" } else { '' }), 'muted_dim')) }
            # the texts of this frame (durations move), then column widths fitted
            # to them: the title gets all the room the other columns leave
            $cells = foreach ($r in $rows) {
                $ci = $r.Ci
                if ($r.CiRunning -and $r.CiSince) { $ci = "$ci $(Format-Duration ((Get-Date) - $r.CiSince.ToLocalTime()))" }
                $queue = $r.Queue
                if ($r.QueueSince) { $queue = "$queue $(Format-Duration ((Get-Date) - $r.QueueSince.ToLocalTime()))" }
                [pscustomobject]@{ Row = $r; Ci = $ci; Queue = $queue; Author = "$($r.Author)$(if ($r.Draft) { ', draft' })" }
            }
            $merged = if ($GitHub.Data) { @($GitHub.Data.data.repository.merged.nodes | Select-Object -First $(if ($compact) { 3 } else { 5 })) } else { @() }
            $whens = @($merged | ForEach-Object { Format-Age ([datetime]$_.mergedAt).ToLocalTime() })
            $fit = { param($texts, $min, $max) [Math]::Min($max, [Math]::Max($min, (@($texts | ForEach-Object { ([string]$_).Length }) + 0 | Measure-Object -Maximum).Maximum + 2)) }
            $ciW = 2 + (& $fit ($cells | ForEach-Object Ci) 8 26)
            $queueW = 2 + (& $fit ($cells | Where-Object { $_.Row.InQueue } | ForEach-Object Queue) 8 26)
            $authorW = & $fit ($cells | ForEach-Object Author) 6 22
            $verdictW = 2 + (& $fit ($rows | ForEach-Object Verdict) 6 16)
            $whenW = & $fit $whens 6 16
            $mergedAuthorW = & $fit ($merged | ForEach-Object { $_.author.login }) 6 18
            # lines added and removed, '+120 -8', one column for all three sections
            $addL = (@($rows | ForEach-Object { "+$($_.Additions)".Length }) + @($merged | ForEach-Object { "+$($_.additions)".Length }) + 2 | Measure-Object -Maximum).Maximum
            $delL = (@($rows | ForEach-Object { "-$($_.Deletions)".Length }) + @($merged | ForEach-Object { "-$($_.deletions)".Length }) + 2 | Measure-Object -Maximum).Maximum
            $diffW = $addL + $delL + 3
            $diff = { param($Add, $Del) @((("+$Add").PadLeft($addL) + ' '), 'success'), @((("-$Del").PadRight($delL) + '  '), 'danger') }
            $queued = @(for ($i = 0; $i -lt $rows.Count; $i++) { if ($rows[$i].InQueue) { $i } })
            $open = @(for ($i = 0; $i -lt $rows.Count; $i++) { if (-not $rows[$i].InQueue -and -not $rows[$i].Waiting) { $i } })
            $waiting = @(for ($i = 0; $i -lt $rows.Count; $i++) { if (-not $rows[$i].InQueue -and $rows[$i].Waiting) { $i } })
            # lists of indexes: test their .Count, @(0) alone is false

            if (-not $rows) {
                & $line @(, @($(if ($GitHub.Data) { ' No open PR.' } else { ' Loading…' }), 'muted'))
            }
            if ($queued.Count) {
                & $heading 'MERGE QUEUE' $queued.Count
                foreach ($i in $queued) {
                    $c = $cells[$i]; $r = $c.Row
                    $start = @(@(" $StatusMark ", $r.Status), @(("#$($r.Number)").PadRight(6), 'violet_light', $r.Url))
                    if ($compact) {
                        & $line ($start + @(, @((Format-Cell $r.Title ($w - 9)), 'ink'))) $i
                        & $line (@(, @('         ', 'muted')) + (& $diff $r.Additions $r.Deletions) + (& $tag "$($c.Queue)  " $r.QueueColor) + @(, @($c.Author, 'muted_dim')))
                    } else {
                        $titleW = [Math]::Max(20, $w - 9 - $authorW - $diffW - $queueW)
                        & $line ($start + @(@((Format-Cell $r.Title $titleW), 'ink'), @((Format-Cell $c.Author $authorW), 'muted_dim')) + (& $diff $r.Additions $r.Deletions) + (& $tag $c.Queue $r.QueueColor)) $i
                    }
                }
            }
            if ($open.Count) {
                if ($queued.Count) { & $line @(, @('', 'muted')) }
                & $heading 'READY TO REVIEW / MERGE' $open.Count
                foreach ($i in $open) {
                    $c = $cells[$i]; $r = $c.Row
                    $start = @(@(" $StatusMark ", $r.Status), @(("#$($r.Number)").PadRight(6), 'violet_light', $r.Url))
                    if ($compact) {
                        $titleW = [Math]::Max(10, $w - 9 - $ciW)
                        & $line ($start + @(, @((Format-Cell $r.Title $titleW), 'ink')) + (& $tag $c.Ci $r.CiColor)) $i
                        & $line (@(, @('         ', 'muted')) + (& $diff $r.Additions $r.Deletions) + (& $tag $r.Verdict $r.VerdictColor) + @(, @("  $($c.Author)", 'muted_dim')))
                    } else {
                        $titleW = [Math]::Max(20, $w - 9 - $authorW - $diffW - $ciW - $verdictW)
                        & $line ($start + @(@((Format-Cell $r.Title $titleW), 'ink'), @((Format-Cell $c.Author $authorW), 'muted_dim')) + (& $diff $r.Additions $r.Deletions) +
                            (& $tag $c.Ci $r.CiColor $ciW) + (& $tag $r.Verdict $r.VerdictColor)) $i
                    }
                }
            }
            # waiting on their author: dimmed, with why instead of CI and review
            if ($waiting.Count) {
                if ($queued.Count -or $open.Count) { & $line @(, @('', 'muted')) }
                & $heading 'WAITING ON THE AUTHOR' $waiting.Count
                $whyW = 2 + (& $fit ($waiting | ForEach-Object { $rows[$_].Waiting }) 6 16)
                foreach ($i in $waiting) {
                    $c = $cells[$i]; $r = $c.Row
                    $whyColor = switch ($r.Waiting) { 'CI FAIL' { 'danger' } 'CHANGES ASKED' { 'warn' } default { 'muted_dim' } }
                    $start = @(@(" $StatusMark ", $whyColor), @(("#$($r.Number)").PadRight(6), 'muted', $r.Url))
                    if ($compact) {
                        & $line ($start + @(, @((Format-Cell $r.Title ($w - 9)), 'muted'))) $i
                        & $line (@(, @('         ', 'muted')) + (& $diff $r.Additions $r.Deletions) + (& $tag $r.Waiting $whyColor) + @(, @("  $($r.Author)", 'muted_dim')))
                    } else {
                        $titleW = [Math]::Max(20, $w - 9 - $authorW - $diffW - $whyW)
                        & $line ($start + @(@((Format-Cell $r.Title $titleW), 'muted'), @((Format-Cell $r.Author $authorW), 'muted_dim')) + (& $diff $r.Additions $r.Deletions) +
                            (& $tag $r.Waiting $whyColor)) $i
                    }
                }
            }
            $prLines = $body.Count
            if ($merged) {
                & $line @(, @('', 'muted'))
                & $heading 'RECENTLY MERGED'
                for ($j = 0; $j -lt $merged.Count; $j++) {
                    $m = $merged[$j]
                    $titleW = [Math]::Max(10, $w - 9 - $mergedAuthorW - $diffW - $whenW)
                    & $line (@(@(" $StatusMark ", 'teal'), @(("#$($m.number)").PadRight(6), 'teal', $m.url), @((Format-Cell $m.title $titleW), 'muted'),
                        @((Format-Cell $m.author.login $mergedAuthorW), 'muted_dim')) + (& $diff $m.additions $m.deletions) + @(, @($whens[$j].PadLeft($whenW - 1), 'muted_dim')))
                }
            }

            # a window too short for everything: first without the merged
            # ones, then the lines around the selected PR, scrolled with it
            $room = $h - $script:PanelRow - 3
            if ($body.Count -gt $room) { $body.RemoveRange($prLines, $body.Count - $prLines) }
            $from = 0; $count = $body.Count
            if ($body.Count -gt $room) {
                $count = [Math]::Max(1, $room - 1)
                $at = 0
                for ($j = 0; $j -lt $body.Count; $j++) { if ($body[$j].Pr -eq $sel) { $at = $j; break } }
                # the selected PR, its second line (compact), and the heading
                # of its section when it comes first
                $firstOfSection = $sel -eq $(if ($rows[$sel].InQueue) { $queued[0] } elseif ($rows[$sel].Waiting) { $waiting[0] } else { $open[0] })
                $top = if ($firstOfSection -and $at -gt 0) { $at - 1 } else { $at }
                $bottom = if ($compact) { $at + 1 } else { $at }
                if ($top -lt $first) { $first = $top }
                if ($bottom -ge $first + $count) { $first = $bottom - $count + 1 }
                $first = [Math]::Max(0, [Math]::Min($first, $body.Count - $count))
                # not a blank line (between two sections) at the top
                if ($first -lt $top -and -not $body[$first].S[0][0]) { $first++ }
                $from = $first
                $count = [Math]::Min($count, $body.Count - $from)
            }
            for ($j = $from; $j -lt $from + $count; $j++) { Write-PanelLine $body[$j].S $w -Selected:($rows -and $body[$j].Pr -eq $sel) }
            if ($count -lt $body.Count) {
                $hidden = $rows.Count - @($body.GetRange($from, $count) | Where-Object { $_.Pr -ge 0 }).Count
                Write-PanelLine @(, @(" ↕ $hidden more PR(s), ↑↓ to see them", 'muted_dim')) $w
            }
            Write-PanelLine @(, @((' ' + ('─' * ($w - 2))), 'muted_dim')) $w
            # what the square before a PR number says
            Write-PanelLine @(@(" $StatusMark", 'danger'), @(' FAIL  ', 'muted_dim'), @($StatusMark, 'amber'), @(' CI RUN  ', 'muted_dim'), @($StatusMark, 'teal'), @(' QUEUE  ', 'muted_dim'),
                @($StatusMark, 'warn'), @(' REVIEW WARN  ', 'muted_dim'), @($StatusMark, 'success'), @(' READY  ', 'muted_dim'), @($StatusMark, 'violet_light'), @(' REVIEW PENDING  ', 'muted_dim'), @($StatusMark, 'muted_dim'), @(' DRAFT', 'muted_dim')) $w
            Write-PanelLine @(, @(' ↑↓ choose · Enter open · Ctrl + click a #number · R refresh · Esc back', 'muted_dim')) $w
            # erase what a longer previous frame left below
            if ($Interactive -and $Vt) { Write-Host "$Esc[J" -NoNewline }
            else { for ($i = $script:PanelRow; $i -lt $lastHeight; $i++) { Write-PanelLine @(@('', 'muted')) $w } }
            $lastHeight = $script:PanelRow
            if (-not $Interactive) { Write-Host ''; return }
        }
    } finally {
        if ($Interactive) { [Console]::CursorVisible = $true }
    }
}
