# Nightly beta fixtures (scripts/tools/nightly.ps1): versions, the next beta
# tag, pruning and the policy line. No Git, no GitHub; the Nightly workflow
# runs them on the pull requests that change these files.
$ErrorActionPreference = 'Stop'
. "$PSScriptRoot/tools/nightly.ps1"
function Assert-Nightly([bool]$Condition, [string]$Message) { if (-not $Condition) { throw $Message } }

# Reading: SemVer tags only, build metadata refused.
Assert-Nightly ($null -eq (ConvertTo-ReleaseVersion 'assets-1')) 'A non-version tag must be refused.'
Assert-Nightly ($null -eq (ConvertTo-ReleaseVersion 'v0.7.6+3.gabc')) 'Build metadata must be refused.'
Assert-Nightly ($null -eq (ConvertTo-ReleaseVersion 'v01.2.3')) 'Leading zeros must be refused.'
$beta = ConvertTo-ReleaseVersion 'v0.7.7-beta.20261025.2'
Assert-Nightly ($beta.Patch -eq 7 -and $beta.Pre.Count -eq 3 -and (Test-BetaVersion $beta)) 'A beta must be read.'
Assert-Nightly (-not (Test-BetaVersion (ConvertTo-ReleaseVersion 'v0.7.7-rc.1'))) 'An rc is not a beta.'

# Precedence, as the updater (semver crate) orders them.
$order = 'v0.7.6', 'v0.7.7-beta.20261025.1', 'v0.7.7-beta.20261025.2', 'v0.7.7-beta.20261025.10', 'v0.7.7-beta.20261026.1', 'v0.7.7'
for ($i = 1; $i -lt $order.Count; $i++) {
    $c = Compare-ReleaseVersion (ConvertTo-ReleaseVersion $order[$i]) (ConvertTo-ReleaseVersion $order[$i - 1])
    Assert-Nightly ($c -eq 1) "$($order[$i]) must rank above $($order[$i - 1])."
}

# The next beta: the patch after the newest stable, the day, a count.
$day = [datetime]::new(2026, 10, 25, 3, 0, 0, [DateTimeKind]::Utc)
Assert-Nightly ((Get-NextBetaTag @('v0.7.5', 'v0.7.6', 'v0.6.9', 'assets-1') $day) -eq 'v0.7.7-beta.20261025.1') 'First beta of the day.'
Assert-Nightly ((Get-NextBetaTag @('v0.7.6', 'v0.7.7-beta.20261025.1', 'v0.7.7-beta.20261025.9') $day) -eq 'v0.7.7-beta.20261025.10') 'Count past 9.'
Assert-Nightly ((Get-NextBetaTag @('v0.7.6', 'v0.7.7-beta.20261024.3') $day) -eq 'v0.7.7-beta.20261025.1') 'A new day starts again at 1.'
Assert-Nightly ((Get-NextBetaTag @('v0.7.6', 'v0.7.7', 'v0.7.7-beta.20261025.1') $day) -eq 'v0.7.8-beta.20261025.1') 'After a stable, the next patch.'
Assert-Nightly ((Get-NextBetaTag @('v0.8.0-beta.1') $day) -eq 'v0.0.1-beta.20261025.1') 'Without a stable release.'

# Two runs the same UTC day (11:00 and 23:00): .1 then .2.
$noon = [datetime]::new(2026, 10, 25, 11, 0, 0, [DateTimeKind]::Utc)
$first = Get-NextBetaTag @('v0.7.6') $noon
$second = Get-NextBetaTag @('v0.7.6', $first) $noon.AddHours(12)
Assert-Nightly ($first -eq 'v0.7.7-beta.20261025.1' -and $second -eq 'v0.7.7-beta.20261025.2') "Same day: $first then $second."
# Nothing new since the last beta (or release): the run does nothing.
Assert-Nightly (Test-AlreadyReleased @($first)) 'A commit with a beta tag is already released.'
Assert-Nightly (Test-AlreadyReleased @('v0.7.6')) 'A commit with a stable tag is already released.'
Assert-Nightly (-not (Test-AlreadyReleased @())) 'A commit without a tag is new.'
Assert-Nightly (-not (Test-AlreadyReleased @('vnext-notes'))) 'A tag that is not a version does not count.'

# Pruning: the newest betas stay, stables and other pre-releases are kept.
$tags = @('v0.7.6', 'v0.7.7-rc.1') + (1..12 | ForEach-Object { "v0.7.7-beta.20261025.$_" })
$pruned = @(Get-BetasToPrune $tags 10)
Assert-Nightly ($pruned.Count -eq 2 -and $pruned -contains 'v0.7.7-beta.20261025.1' -and $pruned -contains 'v0.7.7-beta.20261025.2') "Unexpected pruning: $($pruned -join ', ')"
Assert-Nightly (@(Get-BetasToPrune @('v0.7.6', 'v0.7.7-beta.20261025.1') 10).Count -eq 0) 'Nothing to prune under the limit.'

# The policy line: the newest stable tag's thresholds, the beta's version,
# accepted by the release's own check (tools/update-policy.ps1).
. "$PSScriptRoot/tools/update-policy.ps1"
$line = Get-PolicyLine @('Aurora Viewer 0.7.6', '', 'Aurora-Update-Policy: {"schema":1,"version":"0.7.6","minimum_supported_version":null,"cache_invalidation_before":"0.7.1","settings_reset_before":"0.7.3"}') '0.7.7-beta.20261025.1'
$policy = $line.Substring('Aurora-Update-Policy: '.Length) | ConvertFrom-Json
Assert-Nightly ($policy.version -eq '0.7.7-beta.20261025.1' -and $policy.cache_invalidation_before -eq '0.7.1' -and $policy.settings_reset_before -eq '0.7.3') "Unexpected policy line: $line"
Assert-UpdatePolicy $policy '0.7.7-beta.20261025.1'
Assert-Nightly ($null -eq (Get-PolicyLine @('Aurora Viewer 0.7.6') '0.7.7-beta.20261025.1')) 'No policy line.'

Write-Host 'Nightly tests passed.'
