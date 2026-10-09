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
        number title url isDraft additions deletions
        author { login }
        mergeQueueEntry { position state enqueuedAt }
        autoMergeRequest { enabledAt }
        commits(last: 1) { nodes { commit { statusCheckRollup {
          state
          contexts(first: 20) { nodes {
            ... on CheckRun { name status conclusion startedAt }
            ... on StatusContext { context state }
          } }
        } } } }
        comments(last: 10) { nodes { body } }
      }
    }
    merged: pullRequests(states: MERGED, first: 5, orderBy: {field: UPDATED_AT, direction: DESC}) {
      nodes { number title url mergedAt additions deletions author { login } }
    }
  }
}
'@

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
    if ($LASTEXITCODE -ne 0) { $GitHub.Error = 'pas connecté à GitHub'; return }
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

# Short age for a column: '04 min', '29 min', '02 h', '03 j'.
function Format-Age([datetime]$When) {
    $t = (Get-Date) - $When
    if ($t.TotalMinutes -lt 1) { '{0:00} s' -f [int][Math]::Floor($t.TotalSeconds) }
    elseif ($t.TotalHours -lt 1) { '{0:00} min' -f [int][Math]::Floor($t.TotalMinutes) }
    elseif ($t.TotalDays -lt 1) { '{0:00} h' -f [int][Math]::Floor($t.TotalHours) }
    else { '{0:00} j' -f [int][Math]::Floor($t.TotalDays) }
}

function Format-Ago([datetime]$When) {
    $t = (Get-Date) - $When
    if ($t.TotalMinutes -lt 1) { "il y a $([int]$t.TotalSeconds) s" }
    elseif ($t.TotalHours -lt 1) { "il y a $([int]$t.TotalMinutes) min" }
    elseif ($t.TotalDays -lt 1) { "il y a $([int]$t.TotalHours) h" }
    else { "il y a $([int]$t.TotalDays) j" }
}

# What a pull request looks like on screen.
function Get-PrRows {
    $repo = if ($GitHub.Data) { $GitHub.Data.data.repository }
    if (-not $repo) { return @() }
    foreach ($p in $repo.open.nodes) {
        $rollup = $p.commits.nodes[0].commit.statusCheckRollup
        $ci = 'aucune'; $ciColor = 'muted_dim'; $ciRunning = $false; $since = $null
        if ($rollup) {
            switch ($rollup.state) {
                'SUCCESS' { $ci = 'verte'; $ciColor = 'success' }
                { $_ -in 'FAILURE', 'ERROR' } {
                    $failed = @($rollup.contexts.nodes | Where-Object { $_.conclusion -in 'FAILURE', 'TIMED_OUT', 'CANCELLED' -or $_.state -in 'FAILURE', 'ERROR' } | ForEach-Object { if ($_.name) { $_.name } else { $_.context } })
                    $ci = 'rouge'; $ciColor = 'danger'
                    if ($failed) { $ci = "rouge ($($failed[0]))" }
                }
                default {
                    $ciRunning = $true; $ciColor = 'violet_light'
                    $starts = @($rollup.contexts.nodes | Where-Object { $_.status -in 'IN_PROGRESS', 'QUEUED' -and $_.startedAt } | ForEach-Object { [datetime]$_.startedAt })
                    if ($starts) { $since = ($starts | Sort-Object | Select-Object -First 1) }
                    $ci = 'en cours'
                }
            }
        }
        $queue = ''; $queueColor = 'muted_dim'; $queueSince = $null
        if ($p.mergeQueueEntry) {
            $queueSince = [datetime]$p.mergeQueueEntry.enqueuedAt
            $queue = switch ($p.mergeQueueEntry.state) {
                'AWAITING_CHECKS' { "file n°$($p.mergeQueueEntry.position), tests" }
                'MERGEABLE' { "file n°$($p.mergeQueueEntry.position), fusion" }
                'UNMERGEABLE' { "file : bloquée" }
                default { "file n°$($p.mergeQueueEntry.position)" }
            }
            $queueColor = if ($p.mergeQueueEntry.state -eq 'UNMERGEABLE') { 'danger' } else { 'teal' }
        } elseif ($p.autoMergeRequest) {
            $queue = 'fusion demandée'; $queueColor = 'teal'
        }
        $review = $p.comments.nodes | Where-Object { $_.body -match '^\s*## Relecture' } | Select-Object -Last 1
        $verdict = 'pas relue'; $verdictColor = 'muted_dim'
        if ($review) {
            if ($review.body -match 'Verdict : ✅') { $verdict = 'relue : OK'; $verdictColor = 'success' }
            elseif ($review.body -match 'Verdict : ⚠') { $verdict = 'à trancher'; $verdictColor = 'warn' }
            elseif ($review.body -match 'Verdict : ❌') { $verdict = 'bloquée'; $verdictColor = 'danger' }
            else { $verdict = 'relue'; $verdictColor = 'muted' }
        }
        # one colour for the whole PR (the square before its number), worst first
        $status = if ($p.isDraft) { 'muted_dim' }
            elseif ($ci -like 'rouge*' -or $queue -eq 'file : bloquée' -or $verdict -eq 'bloquée') { 'danger' }
            elseif ($queue -like 'file*' -or $queue -eq 'fusion demandée') { 'teal' }
            elseif ($ciRunning) { 'amber' }
            elseif ($verdict -eq 'à trancher') { 'warn' }
            elseif ($ci -eq 'verte' -and $verdict -eq 'relue : OK') { 'success' }
            else { 'violet_light' }
        [pscustomobject]@{
            Number = $p.number; Title = $p.title; Url = $p.url; Author = $p.author.login; Draft = $p.isDraft
            Additions = [int]$p.additions; Deletions = [int]$p.deletions
            Ci = $ci; CiColor = $ciColor; CiRunning = $ciRunning; CiSince = $since
            Queue = $queue; QueueColor = $queueColor; QueueSince = $queueSince
            Verdict = $verdict; VerdictColor = $verdictColor; Status = $status
        }
    }
}

# The square drawn before a count or a PR number, in the status colour.
$StatusMark = '■'

# Footer, as coloured segments: "PR ■ 3 ouvertes ■ 1 CI ■ 1 en file ■ 1 prête
# ■ 1 rouge", or why there is nothing.
function Get-PrSummary {
    if (-not $GitHub.Data) {
        if ($GitHub.Error) { return , @("GitHub : $($GitHub.Error)", 'warn') }
        return , @('GitHub…', 'muted')
    }
    $rows = @(Get-PrRows)
    if (-not $rows) { return , @('PR : aucune ouverte', 'muted') }
    $counts = @(
        @($rows.Count, 'ouverte', 'violet_light'),
        @(@($rows | Where-Object CiRunning).Count, 'CI', 'amber'),
        @(@($rows | Where-Object { $_.Queue -like 'file*' }).Count, 'en file', 'teal'),
        @(@($rows | Where-Object { $_.Status -eq 'success' }).Count, 'prête', 'success'),
        @(@($rows | Where-Object { $_.Ci -like 'rouge*' }).Count, 'rouge', 'danger')
    )
    $segments = @(, @('PR', 'muted'))
    foreach ($c in $counts) {
        if (-not $c[0]) { continue }
        $label = if ($c[1] -ne 'CI' -and $c[1] -ne 'en file' -and $c[0] -gt 1) { "$($c[1])s" } else { $c[1] }
        $segments += , @("  $StatusMark ", $c[2])
        $segments += , @("$($c[0]) $label", 'ink')
    }
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
                if ($m) { Send-Toast "PR #$n fusionnée" $m.title }
                continue
            }
            $was = $before[$n]; $is = $now[$n]
            if ($is.Ci -like 'rouge*' -and $was.Ci -notlike 'rouge*') { Send-Toast "CI rouge sur la PR #$n" $is.Title }
            if ($was.Queue -like 'file*' -and $is.Queue -notlike 'file*') { Send-Toast "PR #$n sortie de la file de fusion" $is.Title }
        }
        foreach ($n in $now.Keys) {
            if (-not $before.ContainsKey($n)) { Send-Toast "Nouvelle PR #$n ($($now[$n].Author))" $now[$n].Title }
        }
        if ($mainState -in 'FAILURE', 'ERROR' -and $script:PrSeen.Main -notin 'FAILURE', 'ERROR') {
            Send-Toast 'main est rouge' 'La CI de main a échoué : vérifie la dernière fusion.'
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
            if ($Selected) { Write-Color $text $s[1] -Background raised -NoNewline }
            else { Write-Color $text $s[1] -NoNewline }
        }
        $used += $text.Length
    }
    $pad = ' ' * [Math]::Max(0, $Width - $used)
    if ($Selected) { Write-Color $pad ink -Background raised -NoNewline } else { Write-Host $pad -NoNewline }
}

# The PRs in the panel's order: the merge queue (by position, auto-merge
# requests after), then the others as GitHub sorts them (last updated first).
function Get-PanelRows {
    $rows = @(Get-PrRows)
    foreach ($r in $rows) { $r | Add-Member -NotePropertyName InQueue -NotePropertyValue ($r.Queue -like 'file*' -or $r.Queue -eq 'fusion demandée') -Force }
    $position = { if ($_.Queue -match 'n°(\d+)') { [int]$Matches[1] } elseif ($_.Queue -like 'file*') { 0 } else { 1000 } }
    @($rows | Where-Object InQueue | Sort-Object $position) + @($rows | Where-Object { -not $_.InQueue })
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
            $main = if (-not $mainState) { '…', 'muted' } else { switch ($mainState) { 'SUCCESS' { 'verte', 'success' } { $_ -in 'FAILURE', 'ERROR' } { 'rouge', 'danger' } { $_ -in 'PENDING', 'EXPECTED' } { 'en cours', 'violet_light' } default { '?', 'muted' } } }
            $fresh = if ($GitHub.Updated) { Format-Ago $GitHub.Updated } else { 'chargement…' }
            Write-PanelLine @(@(' Aurora · PR', 'violet_light'), @('   main : ', 'muted'), @($main[0], $main[1]), @("   à jour $fresh", 'muted_dim'), @("   $(Get-Date -Format 'HH:mm:ss')", 'muted_dim')) $w
            if ($GitHub.Error) { Write-PanelLine @(@(" GitHub : $($GitHub.Error)", 'warn')) $w }
            Write-PanelLine @(, @((' ' + ('─' * ($w - 2))), 'muted_dim')) $w

            # The body, as lines to write (S: segments, Pr: index of the PR in
            # $rows on its first line, -1 elsewhere), in three sections one
            # above the other: the titles keep the whole width.
            $body = [Collections.Generic.List[object]]::new()
            $line = { param($Segments, $Pr = -1) $body.Add([pscustomobject]@{ S = $Segments; Pr = $Pr }) }
            $heading = { param($Text, $Count) & $line @(@(" $Text", 'violet_light'), @($(if ($null -ne $Count) { " ($Count)" } else { '' }), 'muted_dim')) }
            # the texts of this frame (durations move), then column widths fitted
            # to them: the title gets all the room the other columns leave
            $cells = foreach ($r in $rows) {
                $ci = $r.Ci
                if ($r.CiRunning -and $r.CiSince) { $ci = "en cours $(Format-Duration ((Get-Date) - $r.CiSince.ToLocalTime()))" }
                $queue = $r.Queue
                if ($r.QueueSince) { $queue = "$queue $(Format-Duration ((Get-Date) - $r.QueueSince.ToLocalTime()))" }
                [pscustomobject]@{ Row = $r; Ci = "CI $ci"; Queue = $queue; Author = "$($r.Author)$(if ($r.Draft) { ', brouillon' })" }
            }
            $merged = if ($GitHub.Data) { @($GitHub.Data.data.repository.merged.nodes | Select-Object -First $(if ($compact) { 3 } else { 5 })) } else { @() }
            $whens = @($merged | ForEach-Object { Format-Age ([datetime]$_.mergedAt).ToLocalTime() })
            $fit = { param($texts, $min, $max) [Math]::Min($max, [Math]::Max($min, (@($texts | ForEach-Object { ([string]$_).Length }) + 0 | Measure-Object -Maximum).Maximum + 2)) }
            $ciW = & $fit ($cells | ForEach-Object Ci) 8 26
            $queueW = & $fit ($cells | Where-Object { $_.Row.InQueue } | ForEach-Object Queue) 8 26
            $authorW = & $fit ($cells | ForEach-Object Author) 6 22
            $verdictW = & $fit ($rows | ForEach-Object Verdict) 6 13
            $whenW = & $fit $whens 6 16
            $mergedAuthorW = & $fit ($merged | ForEach-Object { $_.author.login }) 6 18
            # lines added and removed, '+120 -8', one column for all three sections
            $addL = (@($rows | ForEach-Object { "+$($_.Additions)".Length }) + @($merged | ForEach-Object { "+$($_.additions)".Length }) + 2 | Measure-Object -Maximum).Maximum
            $delL = (@($rows | ForEach-Object { "-$($_.Deletions)".Length }) + @($merged | ForEach-Object { "-$($_.deletions)".Length }) + 2 | Measure-Object -Maximum).Maximum
            $diffW = $addL + $delL + 3
            $diff = { param($Add, $Del) @((("+$Add").PadLeft($addL) + ' '), 'success'), @((("-$Del").PadRight($delL) + '  '), 'danger') }
            $queued = @(for ($i = 0; $i -lt $rows.Count; $i++) { if ($rows[$i].InQueue) { $i } })
            $open = @(for ($i = 0; $i -lt $rows.Count; $i++) { if (-not $rows[$i].InQueue) { $i } })
            # lists of indexes: test their .Count, @(0) alone is false

            if (-not $rows) {
                & $line @(, @($(if ($GitHub.Data) { ' Aucune PR ouverte.' } else { ' Chargement…' }), 'muted'))
            }
            if ($queued.Count) {
                & $heading 'FILE DE FUSION' $queued.Count
                foreach ($i in $queued) {
                    $c = $cells[$i]; $r = $c.Row
                    $start = @(@(" $StatusMark ", $r.Status), @(("#$($r.Number)").PadRight(6), 'violet_light'))
                    if ($compact) {
                        & $line ($start + @(, @((Format-Cell $r.Title ($w - 9)), 'ink'))) $i
                        & $line (@(, @('         ', 'muted')) + (& $diff $r.Additions $r.Deletions) + @(@("$($c.Queue)  ", $r.QueueColor), @($c.Author, 'muted_dim')))
                    } else {
                        $titleW = [Math]::Max(20, $w - 9 - $authorW - $diffW - $queueW)
                        & $line ($start + @(@((Format-Cell $r.Title $titleW), 'ink'), @((Format-Cell $c.Author $authorW), 'muted_dim')) + (& $diff $r.Additions $r.Deletions) + @(, @($c.Queue, $r.QueueColor))) $i
                    }
                }
            }
            if ($open.Count) {
                if ($queued.Count) { & $line @(, @('', 'muted')) }
                & $heading 'PR OUVERTES' $open.Count
                foreach ($i in $open) {
                    $c = $cells[$i]; $r = $c.Row
                    $start = @(@(" $StatusMark ", $r.Status), @(("#$($r.Number)").PadRight(6), 'violet_light'))
                    if ($compact) {
                        $titleW = [Math]::Max(10, $w - 9 - $ciW)
                        & $line ($start + @(@((Format-Cell $r.Title $titleW), 'ink'), @($c.Ci, $r.CiColor))) $i
                        & $line (@(, @('         ', 'muted')) + (& $diff $r.Additions $r.Deletions) + @(@($r.Verdict, $r.VerdictColor), @("  $($c.Author)", 'muted_dim')))
                    } else {
                        $titleW = [Math]::Max(20, $w - 9 - $authorW - $diffW - $ciW - $verdictW)
                        & $line ($start + @(@((Format-Cell $r.Title $titleW), 'ink'), @((Format-Cell $c.Author $authorW), 'muted_dim')) + (& $diff $r.Additions $r.Deletions) + @(
                                @((Format-Cell $c.Ci $ciW), $r.CiColor), @($r.Verdict, $r.VerdictColor))) $i
                    }
                }
            }
            $prLines = $body.Count
            if ($merged) {
                & $line @(, @('', 'muted'))
                & $heading 'FUSIONNÉES RÉCEMMENT'
                for ($j = 0; $j -lt $merged.Count; $j++) {
                    $m = $merged[$j]
                    $titleW = [Math]::Max(10, $w - 9 - $mergedAuthorW - $diffW - $whenW)
                    & $line (@(@(" $StatusMark ", 'teal'), @(("#$($m.number)").PadRight(6), 'teal'), @((Format-Cell $m.title $titleW), 'muted'),
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
                $firstOfSection = $sel -eq $(if ($rows[$sel].InQueue) { $queued[0] } else { $open[0] })
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
                Write-PanelLine @(, @(" ↕ $hidden autre(s) PR, ↑↓ pour les voir", 'muted_dim')) $w
            }
            Write-PanelLine @(, @((' ' + ('─' * ($w - 2))), 'muted_dim')) $w
            Write-PanelLine @(@(" $StatusMark", 'danger'), @(' rouge  ', 'muted_dim'), @($StatusMark, 'amber'), @(' CI en cours  ', 'muted_dim'), @($StatusMark, 'teal'), @(' en file  ', 'muted_dim'),
                @($StatusMark, 'warn'), @(' à trancher  ', 'muted_dim'), @($StatusMark, 'success'), @(' prête  ', 'muted_dim'), @($StatusMark, 'violet_light'), @(' à relire  ', 'muted_dim'), @($StatusMark, 'muted_dim'), @(' brouillon', 'muted_dim')) $w
            Write-PanelLine @(, @(' ↑↓ choisir · Entrée ouvrir · R rafraîchir · Échap retour', 'muted_dim')) $w
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
