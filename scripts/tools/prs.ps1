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
        number title url isDraft
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
      nodes { number title url mergedAt author { login } }
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
# Redrawn in place every second (timers move), adapted to the window width so
# it can stay open in a corner of the screen.

# Text cut to fit a column of $Width characters (with one space after).
function Format-Cell([string]$Text, [int]$Width) {
    if ($Text.Length -ge $Width) { $Text = $Text.Substring(0, $Width - 2) + '…' }
    $Text.PadRight($Width)
}

function Write-PanelLine([object[]]$Segments, [int]$Width, [switch]$Selected) {
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
    if ($Selected) { Write-Color $pad ink -Background raised } else { Write-Host $pad }
}

function Show-PrPanel {
    $sel = 0
    $lastDraw = [DateTime]::MinValue
    $lastHeight = 0
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
                $rows = @(Get-PrRows)
                switch ($k.Key) {
                    'UpArrow' { if ($rows) { $sel = ($sel - 1 + $rows.Count) % $rows.Count } }
                    'DownArrow' { if ($rows) { $sel = ($sel + 1) % $rows.Count } }
                    { $_ -in 'Escape', 'Backspace' } { return }
                    { $_ -in 'Enter', 'O' } { if ($rows) { Start-Process $rows[[Math]::Min($sel, $rows.Count - 1)].Url } }
                    'R' { $GitHub.Force = $true }
                }
                $lastDraw = [DateTime]::MinValue
            }
            if (((Get-Date) - $lastDraw).TotalMilliseconds -lt 1000) { Start-Sleep -Milliseconds 50; continue }
            $lastDraw = Get-Date
            Update-PrNotifications

            $w = if ($Interactive) { $Host.UI.RawUI.WindowSize.Width - 1 } else { 110 }
            $compact = $w -lt 96
            $rows = @(Get-PrRows)
            if ($sel -ge $rows.Count) { $sel = [Math]::Max(0, $rows.Count - 1) }
            if ($Interactive) { [Console]::SetCursorPosition(0, 0) }
            $lines = 0

            $mainState = if ($GitHub.Data) { $GitHub.Data.data.repository.defaultBranchRef.target.statusCheckRollup.state }
            # switch skips a null value entirely, default included: test it first
            $main = if (-not $mainState) { '…', 'muted' } else { switch ($mainState) { 'SUCCESS' { 'verte', 'success' } { $_ -in 'FAILURE', 'ERROR' } { 'rouge', 'danger' } { $_ -in 'PENDING', 'EXPECTED' } { 'en cours', 'violet_light' } default { '?', 'muted' } } }
            $fresh = if ($GitHub.Updated) { Format-Ago $GitHub.Updated } else { 'chargement…' }
            Write-PanelLine @(@(' Aurora · PR', 'violet_light'), @('   main : ', 'muted'), @($main[0], $main[1]), @("   à jour $fresh", 'muted_dim'), @("   $(Get-Date -Format 'HH:mm:ss')", 'muted_dim')) $w; $lines++
            if ($GitHub.Error) { Write-PanelLine @(@(" GitHub : $($GitHub.Error)", 'warn')) $w; $lines++ }
            Write-PanelLine @(, @((' ' + ('─' * ($w - 2))), 'muted_dim')) $w; $lines++

            if (-not $rows) {
                Write-PanelLine @(@($(if ($GitHub.Data) { ' Aucune PR ouverte.' } else { ' Chargement…' }), 'muted')) $w; $lines++
            }
            for ($i = 0; $i -lt $rows.Count; $i++) {
                $r = $rows[$i]
                $ci = $r.Ci
                if ($r.CiRunning -and $r.CiSince) { $ci = "en cours $(Format-Duration ((Get-Date) - $r.CiSince.ToLocalTime()))" }
                $queue = $r.Queue
                if ($r.QueueSince) { $queue = "$queue $(Format-Duration ((Get-Date) - $r.QueueSince.ToLocalTime()))" }
                $num = ("#$($r.Number)").PadRight(6)
                if ($compact) {
                    $titleW = [Math]::Max(10, $w - 9 - 22 - 2)
                    $title = if ($r.Title.Length -gt $titleW) { $r.Title.Substring(0, $titleW - 1) + '…' } else { $r.Title }
                    Write-PanelLine @(@(" $StatusMark ", $r.Status), @($num, 'violet_light'), @($title.PadRight($titleW), 'ink'), @(" $ci", $r.CiColor)) $w -Selected:($i -eq $sel); $lines++
                    $second = @(, @('         ', 'muted'))
                    if ($queue) { $second += , @("$queue  ", $r.QueueColor) }
                    $second += , @($r.Verdict, $r.VerdictColor)
                    $second += , @("  $($r.Author)$(if ($r.Draft) { ', brouillon' })", 'muted_dim')
                    Write-PanelLine $second $w; $lines++
                } else {
                    $titleW = [Math]::Max(20, $w - 9 - 16 - 22 - 22 - 12)
                    $title = if ($r.Title.Length -gt $titleW) { $r.Title.Substring(0, $titleW - 1) + '…' } else { $r.Title }
                    $author = if ($r.Author.Length -gt 15) { $r.Author.Substring(0, 14) + '…' } else { $r.Author }
                    Write-PanelLine @(@(" $StatusMark ", $r.Status), @($num, 'violet_light'), @($title.PadRight($titleW + 1), 'ink'), @($author.PadRight(16), 'muted_dim'),
                        @((Format-Cell "CI $ci" 22), $r.CiColor), @((Format-Cell $queue 22), $r.QueueColor), @($r.Verdict, $r.VerdictColor)) $w -Selected:($i -eq $sel)
                    $lines++
                }
            }

            $merged = if ($GitHub.Data) { @($GitHub.Data.data.repository.merged.nodes | Select-Object -First $(if ($compact) { 3 } else { 5 })) } else { @() }
            if ($merged) {
                Write-PanelLine @(@('', 'muted')) $w; $lines++
                Write-PanelLine @(@(' Fusionnées récemment', 'muted')) $w; $lines++
                foreach ($m in $merged) {
                    $when = Format-Ago ([datetime]$m.mergedAt).ToLocalTime()
                    $titleW = [Math]::Max(10, $w - 9 - $when.Length - 3)
                    $title = if ($m.title.Length -gt $titleW) { $m.title.Substring(0, $titleW - 1) + '…' } else { $m.title }
                    Write-PanelLine @(@(" $StatusMark ", 'teal'), @(("#$($m.number)").PadRight(6), 'teal'), @($title.PadRight($titleW + 1), 'muted'), @($when, 'muted_dim')) $w; $lines++
                }
            }
            Write-PanelLine @(, @((' ' + ('─' * ($w - 2))), 'muted_dim')) $w; $lines++
            Write-PanelLine @(@(" $StatusMark", 'danger'), @(' rouge  ', 'muted_dim'), @($StatusMark, 'amber'), @(' CI en cours  ', 'muted_dim'), @($StatusMark, 'teal'), @(' en file  ', 'muted_dim'),
                @($StatusMark, 'warn'), @(' à trancher  ', 'muted_dim'), @($StatusMark, 'success'), @(' prête  ', 'muted_dim'), @($StatusMark, 'violet_light'), @(' à relire  ', 'muted_dim'), @($StatusMark, 'muted_dim'), @(' brouillon', 'muted_dim')) $w; $lines++
            Write-PanelLine @(@(' ↑↓ choisir · Entrée ouvrir · R rafraîchir · Échap retour', 'muted_dim')) $w; $lines++
            # erase what a longer previous frame left below
            for ($i = $lines; $i -lt $lastHeight; $i++) { Write-Host (' ' * $w) }
            $lastHeight = $lines
            if (-not $Interactive) { return }
        }
    } finally {
        if ($Interactive) { [Console]::CursorVisible = $true }
    }
}
