# The Nightly workflow's choices (docs/UPDATES.md, "Beta channel"), shared
# by scripts/nightly-tag.ps1, scripts/nightly-prune.ps1 and their fixture
# tests (scripts/test-nightly.ps1). Pure functions: no Git, no GitHub.

# The SemVer of a release tag, or $null: vMAJOR.MINOR.PATCH[-PRE], no build
# metadata. Pre is the list of its dot-separated identifiers.
function ConvertTo-ReleaseVersion([string]$Tag) {
    if ($Tag -notmatch '^v(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?$') { return $null }
    $pre = @()
    if ($Matches[4]) { $pre = $Matches[4].Split('.') }
    [pscustomobject]@{ Tag = $Tag; Major = [long]$Matches[1]; Minor = [long]$Matches[2]; Patch = [long]$Matches[3]; Pre = $pre }
}

# SemVer precedence (§11): -1, 0 or 1.
function Compare-ReleaseVersion($A, $B) {
    foreach ($part in 'Major', 'Minor', 'Patch') {
        if ($A.$part -ne $B.$part) { return [Math]::Sign($A.$part - $B.$part) }
    }
    $a = @($A.Pre); $b = @($B.Pre)
    if (-not $a.Count -and -not $b.Count) { return 0 }
    if (-not $a.Count) { return 1 }
    if (-not $b.Count) { return -1 }
    for ($i = 0; $i -lt [Math]::Min($a.Count, $b.Count); $i++) {
        $x = $a[$i]; $y = $b[$i]
        $xn = $x -match '^\d+$'; $yn = $y -match '^\d+$'
        if ($xn -and $yn) { $c = [Math]::Sign([decimal]$x - [decimal]$y) }
        elseif ($xn) { $c = -1 }
        elseif ($yn) { $c = 1 }
        else { $c = [Math]::Sign([string]::CompareOrdinal($x, $y)) }
        if ($c -ne 0) { return $c }
    }
    [Math]::Sign($a.Count - $b.Count)
}

function Test-BetaVersion($V) { @($V.Pre).Count -gt 0 -and $V.Pre[0] -ceq 'beta' }

# Whether the commit to publish is already a release or a beta (one of its
# tags is a version): then a run has nothing new and does nothing.
function Test-AlreadyReleased([string[]]$HeadTags) {
    foreach ($t in $HeadTags) { if (ConvertTo-ReleaseVersion $t) { return $true } }
    $false
}

# The newest stable version among the tags, or $null.
function Get-LatestStable([string[]]$Tags) {
    $best = $null
    foreach ($t in $Tags) {
        $v = ConvertTo-ReleaseVersion $t
        if ($v -and -not @($v.Pre).Count -and (-not $best -or (Compare-ReleaseVersion $v $best) -gt 0)) { $best = $v }
    }
    $best
}

<#
    The next beta's tag: the patch after the newest stable version, then
    `beta.<UTC date>.<n>` (n counts that day's betas of that number), so a
    beta ranks above every earlier beta and below the stable it leads to.
    v0.7.6 on 2026-10-25 gives v0.7.7-beta.20261025.1, then .2…
#>
function Get-NextBetaTag([string[]]$Tags, [datetime]$UtcDate) {
    $stable = Get-LatestStable $Tags
    if ($stable) { $core = "$($stable.Major).$($stable.Minor).$($stable.Patch + 1)" } else { $core = '0.0.1' }
    $day = $UtcDate.ToString('yyyyMMdd', [Globalization.CultureInfo]::InvariantCulture)
    $prefix = "v$core-beta.$day."
    $n = 0
    foreach ($t in $Tags) {
        if ($t.StartsWith($prefix) -and $t.Substring($prefix.Length) -match '^[1-9]\d*$') { $n = [Math]::Max($n, [int]$t.Substring($prefix.Length)) }
    }
    "$prefix$($n + 1)"
}

# The beta tags to delete, newest first kept: the $Keep newest betas stay.
# Stable releases and other pre-releases are never touched.
function Get-BetasToPrune([string[]]$Tags, [int]$Keep) {
    $betas = @($Tags | ForEach-Object { ConvertTo-ReleaseVersion $_ } | Where-Object { $_ -and (Test-BetaVersion $_) })
    # Sort-Object cannot take a comparer in Windows PowerShell 5.1: an
    # insertion sort, newest first (a few dozen tags at most).
    $sorted = @()
    foreach ($b in $betas) {
        $i = 0
        while ($i -lt $sorted.Count -and (Compare-ReleaseVersion $sorted[$i] $b) -gt 0) { $i++ }
        $before = @(); if ($i -gt 0) { $before = $sorted[0..($i - 1)] }
        $after = @(); if ($i -lt $sorted.Count) { $after = $sorted[$i..($sorted.Count - 1)] }
        $sorted = @($before) + @($b) + @($after)
    }
    if ($sorted.Count -le $Keep) { return @() }
    @($sorted[$Keep..($sorted.Count - 1)] | ForEach-Object { $_.Tag })
}

# The release policy line a beta carries: the newest stable tag's
# thresholds, so the choices the team made at that release still apply to
# beta testers, under the beta's own version (scripts/write-update-policy.ps1
# checks the line against its tag).
function Get-PolicyLine([string[]]$StableTagMessage, [string]$Version) {
    $line = @($StableTagMessage | Where-Object { $_ -like 'Aurora-Update-Policy: *' })
    if ($line.Count -gt 1) { throw 'Several policies in the tag.' }
    if ($line.Count -eq 0) { return $null }
    $policy = $line[0].Substring('Aurora-Update-Policy: '.Length) | ConvertFrom-Json
    $policy.version = $Version
    'Aurora-Update-Policy: ' + ($policy | ConvertTo-Json -Compress)
}
