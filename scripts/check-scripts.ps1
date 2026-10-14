<#
.SYNOPSIS
    Parses every PowerShell script of scripts/ with the parser of Windows
    PowerShell 5.1, the one aurora-tools.cmd and the humans run. A syntax
    error there stops every action of the Aurora tools, and PowerShell 7
    does not always see it (an unescaped apostrophe in a string, for example).
#>
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
if ($PSVersionTable.PSEdition -ne 'Desktop') {
    # under PowerShell 7: run this check again in Windows PowerShell
    & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $PSCommandPath
    exit $LASTEXITCODE
}
$failed = 0
foreach ($f in Get-ChildItem -LiteralPath (Join-Path $root 'scripts') -Recurse -Filter *.ps1) {
    $tokens = $null; $errors = $null
    [void][System.Management.Automation.Language.Parser]::ParseFile($f.FullName, [ref]$tokens, [ref]$errors)
    foreach ($e in $errors) {
        Write-Host ("{0}:{1}:{2}: {3}" -f $f.FullName.Substring($root.Length + 1), $e.Extent.StartLineNumber, $e.Extent.StartColumnNumber, $e.Message) -ForegroundColor Red
        $failed++
    }
}
if ($failed) { Write-Host "$failed syntax error(s) for Windows PowerShell 5.1." -ForegroundColor Red; exit 1 }
Write-Host 'Every script of scripts/ parses in Windows PowerShell 5.1.'
