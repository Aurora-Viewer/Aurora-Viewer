<# Publishes the tag metadata in the release package, without changing the repository. #>
param([Parameter(Mandatory)][string]$Tag, [string]$Out = 'aurora-update-policy.json')
$ErrorActionPreference = 'Stop'
. "$PSScriptRoot/tools/update-policy.ps1"
$taskRepo = (git rev-parse --show-toplevel).Trim()
$taskPolicy = Get-UpdatePolicy -RepoPath $taskRepo -Tag $Tag -BaseTag $Tag
Assert-UpdatePolicy $taskPolicy $Tag.TrimStart('v')
$taskJson = $taskPolicy | ConvertTo-Json -Depth 4
[IO.File]::WriteAllText([IO.Path]::GetFullPath($Out), $taskJson, [Text.UTF8Encoding]::new($false))
