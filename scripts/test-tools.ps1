# Isolated Windows console and reload regression tests. No GitHub writes,
# installation, viewer launch or changes to the user's repository/preferences.
param([switch]$ConsoleFixture, [string]$OutputFile)
$ErrorActionPreference = 'Stop'

function Assert-ToolsTest([bool]$Condition, [string]$Message) {
    if (-not $Condition) { throw $Message }
}

if ($ConsoleFixture) {
    try {
        . "$PSScriptRoot/tools/ui.ps1"
        $ErrorActionPreference = 'Stop'
        Assert-ToolsTest $Interactive 'The fixture requires a real console.'
        $raw = $Host.UI.RawUI
        function Set-TestSize([int]$Width, [int]$Height) {
            Clear-Host
            $raw.BufferSize = [Management.Automation.Host.Size]::new([Math]::Max($Width, $raw.BufferSize.Width), [Math]::Max($Height, $raw.BufferSize.Height))
            $raw.WindowSize = [Management.Automation.Host.Size]::new($Width, $Height)
            $raw.BufferSize = [Management.Automation.Host.Size]::new($Width, $Height)
            Clear-Host
        }
        function Get-TestScreen {
            $cells = $raw.GetBufferContents([Management.Automation.Host.Rectangle]::new(0, 0, $raw.WindowSize.Width - 1, $raw.WindowSize.Height - 1))
            for ($y = 0; $y -lt $cells.GetLength(0); $y++) {
                $line = [Text.StringBuilder]::new()
                for ($x = 0; $x -lt $cells.GetLength(1); $x++) { $cell = $cells[$y, $x]; [void]$line.Append($cell.Character) }
                $line.ToString()
            }
        }
        $items = @(1..30 | ForEach-Object { @{ Key = [string]$_; Label = "Option-$_"; Hint = 'A longer explanation of this option' } })
        foreach ($vtMode in $false, $true) {
            $Vt = $vtMode
            foreach ($dimensions in @(@(120, 30), @(80, 18), @(40, 12), @(20, 6))) {
                "Console $($dimensions -join 'x'), VT=$vtMode" | Add-Content -LiteralPath $OutputFile
                Set-TestSize $dimensions[0] $dimensions[1]
                [Console]::CursorVisible = $true
                $script:keyIndex = 0
                $expected = @(1, 2, 1, 30, 1, 1)
                $keys = @('DownArrow', 'UpArrow', 'End', 'Home', 'Resize', 'Enter')
                $KeySource = {
                    "Key $script:keyIndex" | Add-Content -LiteralPath $OutputFile
                    $screen = @(Get-TestScreen)
                    $selected = @($screen | Where-Object { $_.Contains([string][char]0x203A) })
                    Assert-ToolsTest ($selected.Count -eq 1) "Selection must occupy exactly one row: $($dimensions -join 'x'), key $script:keyIndex."
                    Assert-ToolsTest ($selected[0] -match "Option-$($expected[$script:keyIndex])(?:\D|$)") 'Highlight and selected item must agree.'
                    Assert-ToolsTest (-not [Console]::CursorVisible) 'The cursor must stay hidden while navigating.'
                    $key = $keys[$script:keyIndex++]
                    if ($key -eq 'Resize') { Set-TestSize 40 12 }
                    [pscustomobject]@{ Key = $key; KeyChar = [char]0 }
                }
                $pick = Show-Menu 'Choose an option' $items { Show-Logo 'Test'; 1..25 | ForEach-Object { Write-Color 'Long preceding transcript' muted } }
                Assert-ToolsTest ($pick.Key -eq '1') 'Home and Enter must choose the first item.'
                Assert-ToolsTest ([Console]::CursorVisible) 'Menu exit must restore the previous cursor state.'
            }
        }
        Set-TestSize 80 18
        $KeySource = { [pscustomobject]@{ Key = 'Escape'; KeyChar = [char]0 } }
        Assert-ToolsTest ($null -eq (Show-Menu 'Cancel' $items)) 'Escape must cancel.'
        $KeySource = { throw 'Injected keyboard failure' }
        $failed = $false
        try { Show-Menu 'Failure' $items } catch { $failed = $true }
        Assert-ToolsTest ($failed -and [Console]::CursorVisible) 'Failure must also restore the cursor.'
        $KeySource = { [pscustomobject]@{ Key = 'D'; KeyChar = 'd' } }
        $letter = Show-Menu 'Shortcut' @(@{ Key = 'd'; Label = 'Direct'; Hint = '' })
        Assert-ToolsTest ($letter.Key -eq 'd') 'Letter shortcuts must still work.'
        . "$PSScriptRoot/tools/update-policy.ps1"
        Set-TestSize 100 20
        $script:keyIndex = 0
        $KeySource = {
            $screen = @(Get-TestScreen) -join "`n"
            Assert-ToolsTest ($screen.Contains('v0.5.0')) 'Each release step must show the chosen version.'
            if ($script:keyIndex -eq 2) { Assert-ToolsTest ($screen.Contains('3/3') -and $screen.Contains('0.5.0.')) 'The settings step must show the prior cache choice.' }
            $key = if ($script:keyIndex++ -eq 0) { 'DownArrow' } else { 'Enter' }
            [pscustomobject]@{ Key = $key; KeyChar = [char]0 }
        }
        $policy = Read-ReleaseMaintenance ([ordered]@{ schema = 1; version = '0.5.0'; cache_invalidation_before = $null; settings_reset_before = $null })
        Assert-ToolsTest ($policy.cache_invalidation_before -eq '0.5.0' -and -not $policy.settings_reset_before) 'Release steps must preserve the selected maintenance options.'
        # Exercise the whole release flow against synthetic read-only data.
        . "$PSScriptRoot/tools/releases.ps1"
        function Invoke-GhJson { @{ status = 'completed'; conclusion = 'success' } }
        function Get-ReleaseContext {
            @{ Last = @{ Tag = 'v0.5.0'; V = @{ Major = 0; Minor = 5; Patch = 0 } }; Prs = @(@{ number = 1; title = 'Fixture'; labels = @(@{ name = 'feature' }) }) }
        }
        function Get-UpdatePolicy { [ordered]@{ schema = 1; version = '0.6.0'; cache_invalidation_before = $null; settings_reset_before = $null } }
        function git { $global:LASTEXITCODE = 0 }
        $Paths = @{ Repo = 'fixture only' }
        $DryRun = $true
        $script:keyIndex = 0
        $KeySource = {
            $screen = @(Get-TestScreen) -join "`n"
            $stage = switch ($script:keyIndex) { 0 { '1/3' }; 1 { '2/3' }; 2 { '3/3' }; default { 'Confirmation' } }
            Assert-ToolsTest ($screen.Contains($stage)) 'Release options and confirmation must have separate screens.'
            Assert-ToolsTest ($screen.Contains('v0.6.0')) 'The chosen version must reach the confirmation.'
            $script:keyIndex++
            [pscustomobject]@{ Key = 'Enter'; KeyChar = [char]0 }
        }
        New-AuroraRelease @{ Slug = 'fixture only' }
        Assert-ToolsTest ($script:keyIndex -eq 5) 'The dry run must reach confirmation and return without publishing.'
        # The footer may receive more status text than a narrow window fits.
        Set-TestSize 30 6
        $FooterStatus = { @(('x' * 200), 'success') }
        $raw.CursorPosition = [Management.Automation.Host.Coordinates]::new(0, 1)
        Write-Footer 'Long keyboard hints'
        Assert-ToolsTest ($raw.CursorPosition.Y -eq 1) 'A long footer must not wrap or move the cursor.'
        'Console: navigation, paging, scrolling, resize, cancellation, cursor and footer passed.' | Set-Content -LiteralPath $OutputFile -Encoding UTF8
        exit 0
    } catch {
        "$_`n$($_.ScriptStackTrace)" | Set-Content -LiteralPath $OutputFile -Encoding UTF8
        exit 1
    }
}

$scratch = Join-Path (Split-Path $PSScriptRoot -Parent) '.scratch'
$fixture = Join-Path $scratch "tools-test-$([Guid]::NewGuid().ToString('N'))"
New-Item -ItemType Directory -Path $fixture -Force | Out-Null
try {
    $output = Join-Path $fixture 'console.txt'
    $engine = (Get-Process -Id $PID).Path
    $process = Start-Process -FilePath $engine -WindowStyle Hidden -ArgumentList @(
        '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', "`"$PSCommandPath`"", '-ConsoleFixture', '-OutputFile', "`"$output`"") -PassThru
    if (-not $process.WaitForExit(60000)) { $process.Kill(); throw "Console test timed out: $(Get-Content -Raw -LiteralPath $output)" }
    Assert-ToolsTest ($process.ExitCode -eq 0) "Console failure: $(Get-Content -Raw -LiteralPath $output)"
    Get-Content -LiteralPath $output

    # Load the entry-point helpers without executing installation or its menu.
    $tokens = $null; $errors = $null
    $ast = [Management.Automation.Language.Parser]::ParseFile((Join-Path $PSScriptRoot 'tools/aurora-tools.ps1'), [ref]$tokens, [ref]$errors)
    Assert-ToolsTest (-not $errors) 'The tools entry point must parse.'
    foreach ($name in 'Get-ToolsFingerprint', 'Invoke-ToolsRepair', 'Restart-AuroraTools') {
        $definition = $ast.Find({ param($node) $node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq $name }, $true)
        . ([scriptblock]::Create($definition.Extent.Text))
    }
    function Write-Color { }
    $repo = Join-Path $fixture 'repository with spaces'
    $modules = Join-Path $repo 'scripts/tools'
    New-Item -ItemType Directory -Path $modules -Force | Out-Null
    $module = Join-Path $modules 'ui.ps1'
    'old script' | Set-Content -LiteralPath $module
    $p = @{ Repo = $repo }
    $ToolsFingerprint = Get-ToolsFingerprint $repo
    $ToolsRestartPath = $null
    function Invoke-Repair { }
    Invoke-ToolsRepair $p
    Assert-ToolsTest (-not $ToolsRestartPath) 'Unchanged scripts must not restart.'
    function Invoke-Repair { 'new script' | Set-Content -LiteralPath $module }
    Invoke-ToolsRepair $p
    Assert-ToolsTest ($ToolsRestartPath -eq (Join-Path $modules 'aurora-tools.ps1')) 'Changed scripts must request a reload from the updated repository.'
    $ToolsFingerprint = Get-ToolsFingerprint $repo
    'new module' | Set-Content -LiteralPath (Join-Path $modules 'added.ps1')
    Assert-ToolsTest ($ToolsFingerprint -ne (Get-ToolsFingerprint $repo)) 'New modules must be detected.'
    $ToolsFingerprint = Get-ToolsFingerprint $repo
    Remove-Item -LiteralPath (Join-Path $modules 'added.ps1')
    Assert-ToolsTest ($ToolsFingerprint -ne (Get-ToolsFingerprint $repo)) 'Removed modules must be detected.'
    $ToolsFingerprint = Get-ToolsFingerprint $repo
    $ToolsRestartPath = $null
    function Invoke-Repair { 'updated before failure' | Set-Content -LiteralPath $module; throw 'Later repair failed' }
    $failed = $false
    try { Invoke-ToolsRepair $p } catch { $failed = $true }
    Assert-ToolsTest ($failed -and $ToolsRestartPath) 'A partial repair failure must still request reload.'
    $child = Join-Path $modules 'aurora-tools.ps1'
    # The replacement is a harmless fixture that records its fresh process.
    @'
param([string]$Action, [switch]$DryRun)
@{ Action = $Action; DryRun = [bool]$DryRun; Process = $PID } | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $PSScriptRoot 'reloaded.json')
'@ | Set-Content -LiteralPath $child -Encoding UTF8
    $DryRun = $true
    Restart-AuroraTools $child
    Assert-ToolsTest ($LASTEXITCODE -eq 0) 'The replacement process must succeed.'
    $result = Get-Content -Raw -LiteralPath (Join-Path $modules 'reloaded.json') | ConvertFrom-Json
    Assert-ToolsTest ($result.Process -ne $PID -and $result.Action -eq 'menu' -and $result.DryRun) 'Reload must use a fresh process, reopen the menu and preserve DryRun.'
    Write-Output 'Reload: unchanged, changed, added/removed modules, partial failure and fresh process passed.'
} finally {
    $resolved = [IO.Path]::GetFullPath($fixture)
    if ((Split-Path $resolved -Parent) -ne [IO.Path]::GetFullPath($scratch) -or (Split-Path $resolved -Leaf) -notlike 'tools-test-*') { throw 'Cleanup outside fixture refused.' }
    Remove-Item -LiteralPath $resolved -Recurse -Force
}
