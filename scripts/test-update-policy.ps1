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
    function Show-Menu($Title, $Items) { $Items[1] }
    $forced = Read-ReleaseMaintenance $policy
    Assert-PolicyTest ($forced.cache_invalidation_before -eq '0.5.0' -and $forced.settings_reset_before -eq '0.5.0') 'Both forced choices must be recorded.'
    [IO.File]::WriteAllText($messageFile, "Fixture`nAurora-Update-Policy: $($forced | ConvertTo-Json -Compress)", [Text.UTF8Encoding]::new($false))
    git -C $taskRoot tag -a v0.5.0 $blob -F $messageFile
    $read = Get-UpdatePolicy $taskRoot v0.5.0 v0.5.0
    Assert-UpdatePolicy $read '0.5.0'
    Assert-PolicyTest ($read.cache_invalidation_before -eq '0.5.0' -and $read.settings_reset_before -eq '0.5.0') 'The workflow must read back exactly the choices of the tag.'
    function Show-Menu($Title, $Items) { return $null }
    Assert-PolicyTest ($null -eq (Read-ReleaseMaintenance $policy)) 'Cancelling must not create a policy.'
    $pending = [ordered]@{ schema = 1; version = '0.5.0'; cache_invalidation_before = $null; settings_reset_before = $null }
    $script:menuStep = 0
    function Show-Menu($Title, $Items) { if ($script:menuStep++ -eq 0) { $Items[1] } else { return $null } }
    Assert-PolicyTest ($null -eq (Read-ReleaseMaintenance $pending) -and -not $pending.cache_invalidation_before) 'Cancelling at the settings step must not apply the cache choice.'
    $invalid = @{ schema = 1; version = '0.5.0'; cache_invalidation_before = '0.6.0' }
    $rejected = $false
    try { Assert-UpdatePolicy $invalid '0.5.0' } catch { $rejected = $true }
    Assert-PolicyTest $rejected 'A threshold later than the release must be rejected.'
    git -C $taskRoot update-index --add -- update-policy.json
    $tree = git -C $taskRoot write-tree
    '{"cache_invalidation_before":"0.9.0"}' | Set-Content -LiteralPath (Join-Path $taskRoot 'update-policy.json') -Encoding UTF8
    $fromRef = Get-UpdatePolicy -RepoPath $taskRoot -Tag v0.5.0 -Ref $tree
    Assert-PolicyTest (-not $fromRef.cache_invalidation_before) 'Local changes must not change the policy of the published commit.'
    Write-Output 'Update policy: choices, inheritance, annotation and rejection verified.'
} finally {
    $taskResolved = [IO.Path]::GetFullPath($taskRoot)
    $taskTemp = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
    if ((Split-Path $taskResolved -Parent).TrimEnd('\') -ne $taskTemp.TrimEnd('\') -or (Split-Path $taskResolved -Leaf) -notlike 'aurora-policy-test-*') { throw 'Refused to clean up outside the fixture.' }
    Remove-Item -LiteralPath $taskResolved -Recurse -Force
}
