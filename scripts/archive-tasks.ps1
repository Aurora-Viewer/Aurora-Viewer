<#
.SYNOPSIS
    Moves the finished tasks of TASKS.md to docs/TASKS_ARCHIVE.md, so that
    TASKS.md only holds open work and stays quick to read.

.DESCRIPTION
    A task is a top-level line "- <status> ..." with the indented lines that
    follow it. Every task whose status is the green check (done) leaves
    TASKS.md and is added to the archive under the same "## " heading, at the
    top of that section. Tasks to do, in progress or waiting for a grid check
    stay where they are, and so does a finished task that still lists an
    open point in its indented lines. Without -Apply, nothing is written:
    the script only says what it would move.

    Run it in a docs task after a release (AGENTS.md, section 6), then open
    the PR.

.EXAMPLE
    ./scripts/archive-tasks.ps1           # what would move, nothing written
    ./scripts/archive-tasks.ps1 -Apply    # move the finished tasks
#>
param([switch]$Apply)
$ErrorActionPreference = 'Stop'

$repo = Split-Path $PSScriptRoot -Parent
$tasksPath = Join-Path $repo 'TASKS.md'
$archivePath = Join-Path $repo 'docs\TASKS_ARCHIVE.md'
$utf8 = New-Object Text.UTF8Encoding $false
$done = '- ' + [char]0x2705 + ' '
# an indented point still to do, in progress or waiting for a grid check
$openPoint = '^\s+- (' + [char]0x2B1C + '|' + [char]::ConvertFromUtf32(0x1F527) + '|' + [char]::ConvertFromUtf32(0x1F7E1) + ')'
$held = 0
# tasks listed before the first heading of TASKS.md
$firstSection = '## General'

$text = [IO.File]::ReadAllText($tasksPath, $utf8)
$eol = if ($text.Contains("`r`n")) { "`r`n" } else { "`n" }
$lines = $text -replace "`r`n", "`n" -split "`n"

# Split TASKS.md: what stays, and the finished tasks by section.
$kept = New-Object Collections.Generic.List[string]
$moved = [ordered]@{}
$section = $firstSection
$i = 0
while ($i -lt $lines.Count) {
    $line = $lines[$i]
    if ($line.StartsWith('## ')) { $section = $line }
    if ($line.StartsWith($done)) {
        # the task and its indented lines, which an empty line may separate
        $end = $i + 1
        $last = $i
        while ($end -lt $lines.Count -and ($lines[$end] -eq '' -or $lines[$end] -match '^\s+\S')) {
            if ($lines[$end] -ne '') { $last = $end }
            $end++
        }
        $block = @($lines[$i..$last] | Where-Object { $_ -ne '' })
        # a finished task that still lists open points stays in TASKS.md
        if (-not ($block | Where-Object { $_ -match $openPoint })) {
            if (-not $moved.Contains($section)) { $moved[$section] = New-Object Collections.Generic.List[string] }
            $moved[$section].AddRange([string[]]$block)
            $i = $last + 1
            continue
        }
        $held++
    }
    # no run of empty lines where tasks were removed
    if ($line -ne '' -or $kept.Count -eq 0 -or $kept[$kept.Count - 1] -ne '') { $kept.Add($line) }
    $i++
}

$count = 0
foreach ($s in $moved.Keys) {
    $n = @($moved[$s] | Where-Object { $_.StartsWith($done) }).Count
    $count += $n
    Write-Host ('{0,4} finished task(s)  {1}' -f $n, $s)
}
if ($held) { Write-Host "$held finished task(s) kept: they still list open points." }
if ($count -eq 0) { Write-Host 'No finished task to archive.'; return }
if (-not $Apply) { Write-Host "$count task(s) would move to docs\TASKS_ARCHIVE.md. Run again with -Apply."; return }

# Archive: the new tasks go to the top of their section; sections keep the
# order in which they first appeared.
$archive = [ordered]@{}
$header = New-Object Collections.Generic.List[string]
if (Test-Path -LiteralPath $archivePath) {
    $old = [IO.File]::ReadAllText($archivePath, $utf8) -replace "`r`n", "`n" -split "`n"
    $current = $null
    foreach ($line in $old) {
        if ($line.StartsWith('## ')) { $current = $line; $archive[$current] = New-Object Collections.Generic.List[string]; continue }
        if ($null -eq $current) { $header.Add($line) } elseif ($line -ne '') { $archive[$current].Add($line) }
    }
    while ($header.Count -gt 0 -and $header[$header.Count - 1] -eq '') { $header.RemoveAt($header.Count - 1) }
} else {
    $header.AddRange([string[]]@(
        '# Aurora Viewer - archived tasks'
        ''
        'Finished tasks moved out of [TASKS.md](../TASKS.md) by'
        '`scripts/archive-tasks.ps1`, under the same headings, most recently archived'
        'first. Search this file when you need the history of a feature; open work'
        'is in TASKS.md.'
    ))
}
foreach ($s in $moved.Keys) {
    if (-not $archive.Contains($s)) { $archive[$s] = New-Object Collections.Generic.List[string] }
    $archive[$s].InsertRange(0, $moved[$s])
}
$out = New-Object Collections.Generic.List[string]
$out.AddRange($header)
foreach ($s in $archive.Keys) {
    $out.Add('')
    $out.Add($s)
    $out.Add('')
    $out.AddRange($archive[$s])
}
$out.Add('')

[IO.File]::WriteAllText($archivePath, ($out -join $eol), $utf8)
[IO.File]::WriteAllText($tasksPath, ($kept -join $eol), $utf8)
Write-Host "$count task(s) moved to docs\TASKS_ARCHIVE.md." -ForegroundColor Green
