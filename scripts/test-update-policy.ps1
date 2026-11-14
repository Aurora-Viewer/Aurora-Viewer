# Local policy/tag fixtures; never publishes a release or changes the checkout.
$ErrorActionPreference = 'Stop'
. "$PSScriptRoot/tools/update-policy.ps1"
$taskRoot = Join-Path ([IO.Path]::GetTempPath()) "aurora-policy-test-$([Guid]::NewGuid().ToString('N'))"
New-Item -ItemType Directory -Path $taskRoot | Out-Null
function Assert-PolicyTest([bool]$Condition, [string]$Message) { if (-not $Condition) { throw $Message } }
try {
    git -C $taskRoot init --quiet
    if ($LASTEXITCODE -ne 0) { throw 'Git fixture unavailable.' }
    # annotated tags need an identity, and a PC may have no global one
    git -C $taskRoot config user.name fixture
    git -C $taskRoot config user.email fixture@example.invalid
    '{"minimum_supported_version":null,"cache_invalidation_before":null,"settings_reset_before":null}' | Set-Content -LiteralPath (Join-Path $taskRoot 'update-policy.json') -Encoding UTF8
    $blob = git -C $taskRoot hash-object -w update-policy.json
    $old = '{"schema":1,"version":"0.3.0","minimum_supported_version":null,"cache_invalidation_before":"0.3.0","settings_reset_before":null}'
    $messageFile = Join-Path $taskRoot 'tag-message.txt'
    [IO.File]::WriteAllText($messageFile, "Fixture`nAurora-Update-Policy: $old", [Text.UTF8Encoding]::new($false))
    git -C $taskRoot tag -a v0.3.0 $blob -F $messageFile
    if ($LASTEXITCODE -ne 0) { throw 'Tag fixture unavailable.' }
    $policy = Get-UpdatePolicy $taskRoot v0.5.0 v0.3.0
    Assert-UpdatePolicy $policy '0.5.0'
    Assert-PolicyTest ($policy.cache_invalidation_before -eq '0.3.0') 'The threshold must survive skipped releases.'
    function Show-Menu($Title, $Items) { $Items[0] }
    $kept = Read-ReleaseMaintenance $policy
    Assert-PolicyTest ($kept.cache_invalidation_before -eq '0.3.0' -and -not $kept.settings_reset_before) 'The default choices must keep the cache and migrate the settings.'
    Assert-PolicyTest (-not $kept.minimum_supported_version) 'Keeping the minimum must keep every update optional.'
    function Show-Menu($Title, $Items) { $Items[1] }
    $forced = Read-ReleaseMaintenance $policy
    Assert-PolicyTest ($forced.minimum_supported_version -eq '0.5.0' -and $forced.cache_invalidation_before -eq '0.5.0' -and $forced.settings_reset_before -eq '0.5.0') 'The required version and both forced choices must be recorded.'
    [IO.File]::WriteAllText($messageFile, "Fixture`nAurora-Update-Policy: $($forced | ConvertTo-Json -Compress)", [Text.UTF8Encoding]::new($false))
    git -C $taskRoot tag -a v0.5.0 $blob -F $messageFile
    $read = Get-UpdatePolicy $taskRoot v0.5.0 v0.5.0
    Assert-UpdatePolicy $read '0.5.0'
    Assert-PolicyTest ($read.minimum_supported_version -eq '0.5.0' -and $read.cache_invalidation_before -eq '0.5.0' -and $read.settings_reset_before -eq '0.5.0') 'The workflow must read back exactly the choices of the tag.'
    $next = Get-UpdatePolicy $taskRoot v0.6.0 v0.5.0
    function Show-Menu($Title, $Items) { $Items[0] }
    Assert-PolicyTest ((Read-ReleaseMaintenance $next).minimum_supported_version -eq '0.5.0') 'The next release must keep the minimum by default.'
    function Show-Menu($Title, $Items) { return $null }
    Assert-PolicyTest ($null -eq (Read-ReleaseMaintenance $policy)) 'Cancelling must not create a policy.'
    $pending = [ordered]@{ schema = 1; version = '0.5.0'; minimum_supported_version = $null; cache_invalidation_before = $null; settings_reset_before = $null }
    $script:menuStep = 0
    function Show-Menu($Title, $Items) { if ($script:menuStep++ -lt 2) { $Items[1] } else { return $null } }
    Assert-PolicyTest ($null -eq (Read-ReleaseMaintenance $pending) -and -not $pending.cache_invalidation_before -and -not $pending.minimum_supported_version) 'Cancelling at the settings step must not apply the earlier choices.'
    # a minimum typed by hand
    function Write-Color { }
    function Wait-Back { }
    $floored = [ordered]@{ schema = 1; version = '0.6.0'; minimum_supported_version = '0.4.0'; cache_invalidation_before = $null; settings_reset_before = $null }
    $script:menuStep = 0
    function Show-Menu($Title, $Items) { if ($script:menuStep++ -eq 0) { $Items | Where-Object { $_.Other } } else { $Items[0] } }
    function Read-Text($Prompt) { 'v0.5.2' }
    Assert-PolicyTest ((Read-ReleaseMaintenance $floored).minimum_supported_version -eq '0.5.2') 'A minimum typed by hand must be recorded.'
    foreach ($typed in '0.3.0', '0.7.0', '0.5', 'abc') {
        $floored.minimum_supported_version = '0.4.0'
        $script:menuStep = 0
        function Read-Text($Prompt) { $typed }
        Assert-PolicyTest ($null -eq (Read-ReleaseMaintenance $floored) -and $floored.minimum_supported_version -eq '0.4.0') "The typed minimum '$typed' must be refused."
    }
    # pre-release: 0.6.0-beta.1 comes before 0.6.0, the updater would refuse it
    $beta = [ordered]@{ schema = 1; version = '0.6.0-beta.1'; minimum_supported_version = '0.6.0'; cache_invalidation_before = $null; settings_reset_before = $null }
    $script:offered = $null
    function Show-Menu($Title, $Items) { if (-not $script:offered) { $script:offered = $Items }; $Items[0] }
    $kept = Read-ReleaseMaintenance $beta
    Assert-PolicyTest (-not ($script:offered | Where-Object { $_.Minimum -eq '0.6.0' })) 'A pre-release must not offer to require its own number.'
    Assert-PolicyTest (-not $kept.minimum_supported_version) 'A minimum the pre-release does not reach must be left out.'
    $rejected = $false
    try { Assert-UpdatePolicy ([ordered]@{ schema = 1; version = '0.6.0-beta.1'; minimum_supported_version = '0.6.0' }) '0.6.0-beta.1' } catch { $rejected = $true }
    Assert-PolicyTest $rejected 'A pre-release requiring its own number must be rejected.'
    Assert-PolicyTest ($null -ne (Get-ReleaseFloorError '0.6.0' $null '0.6.0-beta.1') -and $null -eq (Get-ReleaseFloorError '0.5.9' $null '0.6.0-beta.1')) 'A typed minimum must stay below a pre-release.'
    $invalid = @{ schema = 1; version = '0.5.0'; cache_invalidation_before = '0.6.0' }
    $rejected = $false
    try { Assert-UpdatePolicy $invalid '0.5.0' } catch { $rejected = $true }
    Assert-PolicyTest $rejected 'A threshold later than the release must be rejected.'
    # grace period (#3014): chosen after a new minimum, 12 h writes nothing
    Assert-PolicyTest ($forced.grace_hours -eq 24) 'The grace period chosen must be recorded.'
    function Show-Menu($Title, $Items) { if ($Title -like '*Minimum version') { $Items[1] } else { $Items[0] } }
    $usual = Read-ReleaseMaintenance (Get-UpdatePolicy $taskRoot v0.7.0 v0.5.0)
    Assert-PolicyTest ($usual.minimum_supported_version -eq '0.7.0' -and -not $usual.Contains('grace_hours')) 'The usual 12 h must leave the viewer default.'
    function Show-Menu($Title, $Items) { if ($Title -like '*Minimum version') { $Items[1] } elseif ($Title -like '*Grace period') { $Items | Where-Object { $_.Hours -eq 0 } } else { $Items[0] } }
    $urgent = Read-ReleaseMaintenance (Get-UpdatePolicy $taskRoot v0.7.0 v0.5.0)
    Assert-PolicyTest ($urgent.minimum_supported_version -eq '0.7.0' -and $urgent.grace_hours -eq 0) 'Requiring at once must record no grace period.'
    [IO.File]::WriteAllText($messageFile, "Fixture`nAurora-Update-Policy: $($urgent | ConvertTo-Json -Compress)", [Text.UTF8Encoding]::new($false))
    git -C $taskRoot tag -a v0.7.0 $blob -F $messageFile
    Assert-PolicyTest ((Get-UpdatePolicy $taskRoot v0.7.0 v0.7.0).grace_hours -eq 0) 'The workflow must read back the grace period of the tag.'
    $keptUrgent = Get-UpdatePolicy $taskRoot v0.7.1 v0.7.0
    Assert-PolicyTest ($keptUrgent.grace_hours -eq 0) 'A release keeping an urgent minimum must stay without grace.'
    function Show-Menu($Title, $Items) { $Items[0] }
    $keptAgain = Read-ReleaseMaintenance (Get-UpdatePolicy $taskRoot v0.7.1 v0.7.0)
    Assert-PolicyTest ($keptAgain.minimum_supported_version -eq '0.7.0' -and $keptAgain.grace_hours -eq 0) 'Keeping the minimum must keep its grace period, without asking again.'
    function Show-Menu($Title, $Items) { if ($Title -like '*Minimum version') { $Items | Where-Object { $_.Minimum -eq '0.7.1' } } else { $Items[0] } }
    $raised = Read-ReleaseMaintenance $keptUrgent
    Assert-PolicyTest ($raised.minimum_supported_version -eq '0.7.1' -and -not $raised.Contains('grace_hours')) 'A new minimum must not keep the previous grace period.'
    foreach ($bad in 100, -1, 'soon') {
        $rejected = $false
        try { Assert-UpdatePolicy ([ordered]@{ schema = 1; version = '0.5.0'; grace_hours = $bad }) '0.5.0' } catch { $rejected = $true }
        Assert-PolicyTest $rejected "The grace period '$bad' must be rejected."
    }
    git -C $taskRoot update-index --add -- update-policy.json
    $tree = git -C $taskRoot write-tree
    '{"cache_invalidation_before":"0.9.0"}' | Set-Content -LiteralPath (Join-Path $taskRoot 'update-policy.json') -Encoding UTF8
    $fromRef = Get-UpdatePolicy -RepoPath $taskRoot -Tag v0.5.0 -Ref $tree
    Assert-PolicyTest (-not $fromRef.cache_invalidation_before) 'Local changes must not change the policy of the published commit.'
    Write-Output 'Update policy: choices, minimum, inheritance, annotation and rejection verified.'
} finally {
    $taskResolved = [IO.Path]::GetFullPath($taskRoot)
    $taskTemp = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
    $taskSep = [IO.Path]::DirectorySeparatorChar
    if ((Split-Path $taskResolved -Parent).TrimEnd($taskSep) -ne $taskTemp.TrimEnd($taskSep) -or (Split-Path $taskResolved -Leaf) -notlike 'aurora-policy-test-*') { throw 'Refused to clean up outside the fixture.' }
    Remove-Item -LiteralPath $taskResolved -Recurse -Force
}
