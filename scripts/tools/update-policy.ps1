# Release policy is part of the annotated tag: the release can be configured
# without committing local choices to main. Thresholds carry across versions.
function Get-UpdatePolicy([string]$RepoPath, [string]$Tag, [string]$BaseTag, [string]$Ref) {
    $fields = @('minimum_supported_version', 'cache_invalidation_before', 'settings_reset_before')
    $policy = [ordered]@{ schema = 1; version = $Tag.TrimStart('v') }
    foreach ($field in $fields) { $policy[$field] = $null }
    if ($Ref) {
        $json = git -C $RepoPath show "${Ref}:update-policy.json"
        if ($LASTEXITCODE -ne 0) { throw 'Policy of the commit to publish not found.' }
        $defaults = ($json -join "`n") | ConvertFrom-Json
    } else {
        $defaults = Get-Content -Raw -LiteralPath (Join-Path $RepoPath 'update-policy.json') | ConvertFrom-Json
    }
    $previous = $null
    if ($BaseTag) {
        $message = git -C $RepoPath for-each-ref '--format=%(contents)' "refs/tags/$BaseTag"
        if ($LASTEXITCODE -ne 0) { throw 'Could not read the tag.' }
        $line = @($message | Where-Object { $_ -like 'Aurora-Update-Policy: *' })
        if ($line.Count -gt 1) { throw 'Several policies in the tag.' }
        if ($line.Count -eq 1) {
            $previous = $line[0].Substring('Aurora-Update-Policy: '.Length) | ConvertFrom-Json
            Assert-UpdatePolicy $previous $BaseTag.TrimStart('v')
        }
    }
    foreach ($field in $fields) {
        $values = @($defaults.$field, $previous.$field) | Where-Object { $_ }
        foreach ($value in $values) {
            if ($value -notmatch '^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$') { throw "Invalid threshold: $field." }
            if (-not $policy[$field] -or [version]$value -gt [version]$policy[$field]) { $policy[$field] = $value }
        }
    }
    $policy
}

function Assert-UpdatePolicy($Policy, [string]$Version) {
    if ($Policy.schema -ne 1 -or $Policy.version -cne $Version) { throw 'Policy does not match the tag.' }
    $core = ($Version -split '-')[0]
    foreach ($field in 'minimum_supported_version', 'cache_invalidation_before', 'settings_reset_before') {
        $value = $Policy.$field
        if ($value -and ($value -notmatch '^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$' -or [version]$value -gt [version]$core)) {
            throw "Invalid threshold for ${field}: $value."
        }
    }
    # 0.9.2-beta.1 comes before 0.9.2: the updater refuses a floor above the
    # version it offers (aurora-updater feed.rs, "invalid update floor")
    $minimum = $Policy.minimum_supported_version
    if ($minimum -and $Version -ne $core -and [version]$minimum -eq [version]$core) {
        throw "Invalid minimum for the pre-release ${Version}: $minimum."
    }
}

# A minimum typed by hand: X.Y.Z, never below the floor it inherits (the
# thresholds only go up), never above the release, and below a pre-release's
# own number. Returns the reason it is refused, or $null.
function Get-ReleaseFloorError([string]$Value, [string]$Floor, [string]$Version) {
    $core = ($Version -split '-')[0]
    if ($Value -notmatch '^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$') { return "'$Value' is not an X.Y.Z version." }
    if ($Floor -and [version]$Value -lt [version]$Floor) { return "The minimum cannot go below the current one ($Floor)." }
    if ([version]$Value -gt [version]$core) { return "The minimum cannot be above the release ($core)." }
    if ($Version -ne $core -and [version]$Value -eq [version]$core) { return "A pre-release cannot require its own number ($core): its users would be refused it." }
    $null
}

function Read-ReleaseMaintenance($Policy) {
    $core = ($Policy.version -split '-')[0]
    $pre = $Policy.version -ne $core
    $floor = $Policy.minimum_supported_version
    # as the nightly does (tools/nightly.ps1, Get-PolicyLine): a floor the
    # pre-release does not reach is left out rather than refused
    if ($pre -and $floor -and [version]$floor -ge [version]$core) { $floor = $null }
    $floorText = if ($floor) { "updates required below $floor" } else { 'every update optional' }
    $items = @(@{ Key = '1'; Label = 'Keep the current minimum'; Hint = "usual choice: $floorText"; Minimum = $floor })
    if (-not $pre -and (-not $floor -or [version]$floor -lt [version]$core)) {
        $items += @{ Key = '2'; Label = "Require v$core"; Hint = 'every earlier version must update: login blocked until then'; Minimum = $core }
    }
    $items += @{ Key = [string]($items.Count + 1); Label = 'Other minimum…'; Hint = 'an earlier X.Y.Z version'; Minimum = $null; Other = $true }
    $step = Show-Menu 'Step 2/4 · Minimum version' $items {
        Show-Logo 'New release'
        Write-Color "  Chosen version: v$($Policy.version)" teal
        Write-Color "  Minimum today: $(if ($floor) { $floor + ' (a version below it cannot log in before updating)' } else { 'none' })." muted
    }
    if (-not $step) { return }
    $minimum = $step.Minimum
    if ($step.Other) {
        $minimum = Read-Text "Minimum version (X.Y.Z$(if ($floor) { ", $floor or later" }), up to $core; empty: cancel):"
        if (-not $minimum) { return }
        $minimum = $minimum.TrimStart('v')
        $refused = Get-ReleaseFloorError $minimum $floor $Policy.version
        if ($refused) { Write-Color "  $refused" danger; Wait-Back; return }
    }
    $minimumText = if ($minimum) { "required below $minimum" } else { 'every update optional' }
    $cache = Show-Menu 'Step 3/4 · User cache' @(
        @{ Key = '1'; Label = 'Keep the cache'; Hint = 'usual choice, no data to download again'; Clear = $false }
        @{ Key = '2'; Label = "Force cache invalidation"; Hint = 'cache cleared at the first start of this version'; Clear = $true }
    ) {
        Show-Logo 'New release'
        Write-Color "  Chosen version: v$($Policy.version)" teal
        Write-Color "  Update: $minimumText." muted
    }
    if (-not $cache) { return }
    $cacheThreshold = if ($cache.Clear) { $core } else { $Policy.cache_invalidation_before }
    $settings = Show-Menu 'Step 4/4 · User settings' @(
        @{ Key = '1'; Label = 'Keep and migrate automatically'; Hint = 'migrations coded in the viewer, usual choice'; Reset = $false }
        @{ Key = '2'; Label = 'Reset the settings'; Hint = 'back to the default values; announced before installation'; Reset = $true }
    ) {
        Show-Logo 'New release'
        Write-Color "  Chosen version: v$($Policy.version)" teal
        Write-Color "  Update: $minimumText." muted
        Write-Color "  Cache: $(if ($cacheThreshold) { 'cleared for versions before ' + $cacheThreshold } else { 'kept' })." muted
    }
    if (-not $settings) { return }
    $Policy.minimum_supported_version = $minimum
    if ($cache.Clear) { $Policy.cache_invalidation_before = $core }
    if ($settings.Reset) { $Policy.settings_reset_before = $core }
    Assert-UpdatePolicy $Policy $Policy.version
    $Policy
}
