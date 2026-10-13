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
}

function Read-ReleaseMaintenance($Policy) {
    $cache = Show-Menu 'Step 2/3 · User cache' @(
        @{ Key = '1'; Label = 'Keep the cache'; Hint = 'usual choice, no data to download again'; Clear = $false }
        @{ Key = '2'; Label = "Force cache invalidation"; Hint = 'cache cleared at the first start of this version'; Clear = $true }
    ) { Show-Logo 'New release'; Write-Color "  Chosen version: v$($Policy.version)" teal }
    if (-not $cache) { return }
    $core = ($Policy.version -split '-')[0]
    $cacheThreshold = if ($cache.Clear) { $core } else { $Policy.cache_invalidation_before }
    $settings = Show-Menu 'Step 3/3 · User settings' @(
        @{ Key = '1'; Label = 'Keep and migrate automatically'; Hint = 'migrations coded in the viewer, usual choice'; Reset = $false }
        @{ Key = '2'; Label = 'Reset the settings'; Hint = 'back to the default values; announced before installation'; Reset = $true }
    ) {
        Show-Logo 'New release'
        Write-Color "  Chosen version: v$($Policy.version)" teal
        Write-Color "  Cache: $(if ($cacheThreshold) { 'cleared for versions before ' + $cacheThreshold } else { 'kept' })." muted
    }
    if (-not $settings) { return }
    if ($cache.Clear) { $Policy.cache_invalidation_before = $core }
    if ($settings.Reset) { $Policy.settings_reset_before = $core }
    Assert-UpdatePolicy $Policy $Policy.version
    $Policy
}
