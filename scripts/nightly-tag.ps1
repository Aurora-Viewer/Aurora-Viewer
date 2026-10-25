<#
.SYNOPSIS
    Run by the Nightly workflow on the commit to publish: when main has
    changed since the last release or beta, creates and pushes the next beta
    tag (scripts/tools/nightly.ps1) and writes `tag=<tag>` to the step's
    output; otherwise writes nothing and the build is skipped.

.EXAMPLE
    ./scripts/nightly-tag.ps1 -DryRun
    Prints the tag the next nightly would create here, creates nothing.
#>
param([switch]$DryRun)
$ErrorActionPreference = 'Stop'
. "$PSScriptRoot/tools/nightly.ps1"

$repo = (git rev-parse --show-toplevel).Trim()
$tags = @(git -C $repo tag --list 'v*')
$here = @(git -C $repo tag --points-at HEAD --list 'v*')
if (Test-AlreadyReleased $here) {
    Write-Host "HEAD is already released as $($here -join ', '): no nightly."
    exit 0
}
$tag = Get-NextBetaTag $tags ([datetime]::UtcNow)
$message = @("Aurora Viewer $($tag.TrimStart('v')) (nightly beta)")
$stable = Get-LatestStable $tags
if ($stable) {
    $policy = Get-PolicyLine @(git -C $repo for-each-ref '--format=%(contents)' "refs/tags/$($stable.Tag)") $tag.TrimStart('v')
    if ($policy) { $message += '', $policy }
}
Write-Host "Next beta: $tag"
if ($DryRun) { $message | ForEach-Object { Write-Host "  $_" }; exit 0 }

# Through a file: a native argument would lose the policy's JSON quotes in
# Windows PowerShell.
$file = [IO.Path]::GetTempFileName()
try {
    [IO.File]::WriteAllText($file, ($message -join "`n") + "`n", [Text.UTF8Encoding]::new($false))
    git -C $repo -c user.name='github-actions[bot]' -c user.email='41898282+github-actions[bot]@users.noreply.github.com' tag -a $tag -F $file
    if ($LASTEXITCODE -ne 0) { throw 'git tag failed.' }
}
finally { Remove-Item -LiteralPath $file -ErrorAction SilentlyContinue }
git -C $repo push origin "refs/tags/$tag"
if ($LASTEXITCODE -ne 0) { throw 'git push failed.' }
if ($env:GITHUB_OUTPUT) { "tag=$tag" >> $env:GITHUB_OUTPUT }
