param(
    [Parameter(Mandatory=$true)] [string]$AuroraExe,
    [int]$Steps = 10,
    [switch]$Scripted
)
$ErrorActionPreference = 'Stop'
$Root = Split-Path -Parent $MyInvocation.MyCommand.Path
$Agent = Join-Path $Root 'agent'
$AuroraExe = (Resolve-Path $AuroraExe).Path
if ($Steps -lt 0) { throw 'Steps must be >= 0.' }

$cfg = Get-Content (Join-Path $Agent 'config.json') -Raw | ConvertFrom-Json
$bytes = New-Object byte[] 32
$rng = [Security.Cryptography.RandomNumberGenerator]::Create()
try { $rng.GetBytes($bytes) } finally { $rng.Dispose() }
# Ephemeral secret shared by only the launched viewer and agent process.
$env:AURORA_AI_TOKEN = [Convert]::ToBase64String($bytes)
$env:AURORA_AI_CHAT_READ = '1'
$env:AURORA_AI_ALLOW_CHAT = if ($cfg.allow_chat) { '1' } else { '0' }

$python = Join-Path $Agent '.venv\Scripts\python.exe'
if (!(Test-Path $python)) {
    py -3 -m venv (Join-Path $Agent '.venv')
}
& $python -m pip install -r (Join-Path $Agent 'requirements.txt')
if ($LASTEXITCODE -ne 0) { throw 'Python dependency installation failed.' }

# Resume an explicitly new session. Old PAUSED/STOP flags are not retained.
$Data = Join-Path $Agent 'data'
New-Item -ItemType Directory -Force -Path $Data | Out-Null
Remove-Item -Force -ErrorAction SilentlyContinue (Join-Path $Data 'STOP')
Remove-Item -Force -ErrorAction SilentlyContinue (Join-Path $Data 'PAUSED')

Write-Host 'Launching the AI-enabled build of Aurora (you will log in manually).'
$viewer = Start-Process -FilePath $AuroraExe -PassThru
$env:AURORA_AI_VIEWER_PID = [string]$viewer.Id
Write-Host 'Aurora PID:' $viewer.Id
Write-Host 'Python agent actions:' $(if ($cfg.allow_walking) { 'walk enabled' } else { 'walk disabled' }), $(if ($cfg.allow_chat) { 'chat enabled' } else { 'chat disabled' })
Write-Host 'Ctrl+C stops the agent; Aurora can remain open.'
$argsList = @('aurora_agent.py', '--steps', [string]$Steps)
if ($Scripted) { $argsList += '--scripted' }
Push-Location $Agent
try {
    & $python @argsList
} finally {
    Pop-Location
    Remove-Item Env:AURORA_AI_TOKEN -ErrorAction SilentlyContinue
}
