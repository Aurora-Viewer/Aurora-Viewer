<#
.SYNOPSIS
    Keeps the `triage` label in step with the Priority of open issues, run by
    the Issue triage workflow: an open issue without a Priority carries
    `triage`, an issue with one does not. The project manager sets the
    Priority of each incoming issue (AGENTS.md section 15), so the label
    lists what they still have to look at.

.DESCRIPTION
    The issues opened by the authors of $Exempt never get the label: they
    set their own Priority, or let the project manager do it from the board.
    Closed issues are left as they are. The Priority is read again through
    the API at each run, whatever the event said: the workflow can then run
    on any change of an issue field, and a run by hand repairs any issue.

.EXAMPLE
    ./scripts/issue-triage.ps1 -Issue 1167
    (the workflow; GH_TOKEN with the issues permission)

.EXAMPLE
    ./scripts/issue-triage.ps1 -All -DryRun
    Every open issue: prints what would change, changes nothing.
#>
param(
    [int]$Issue,
    [switch]$All,
    [switch]$DryRun,
    [string]$Repo = $env:GITHUB_REPOSITORY,
    # GitHub logins, compared without case
    [string[]]$Exempt = @('Odessa-Draekavik', 'Rohacan')
)
$ErrorActionPreference = 'Stop'
$Label = 'triage'

if (-not $Repo) { throw 'Give -Repo owner/name, or set GITHUB_REPOSITORY.' }
if (-not $Issue -and -not $All) { throw 'Give -Issue <n> or -All.' }
$owner, $name = $Repo -split '/', 2

# Every string goes in a GraphQL variable, none in the query: Windows
# PowerShell would mangle the quotes on the command line.
function Invoke-Graphql([string]$Query, [hashtable]$Vars) {
    $a = @('api', 'graphql', '-f', "query=$Query")
    foreach ($k in $Vars.Keys) {
        $flag = if ($Vars[$k] -is [int]) { '-F' } else { '-f' }
        $a += @($flag, "$k=$($Vars[$k])")
    }
    $out = & gh @a 2>&1 | Out-String
    if ($LASTEXITCODE -ne 0) { throw "gh api graphql failed: $out" }
    ($out | ConvertFrom-Json).data
}
function Invoke-Gh {
    $output = & gh @args
    if ($LASTEXITCODE -ne 0) { throw "gh $($args -join ' ') failed." }
    $output
}

# Priority is an issue field of the organisation (scripts/issue-board.ps1).
$fields = 'number state author{login} labels(first:50){nodes{name}} ' +
    'issueFieldValues(first:20){nodes{... on IssueFieldSingleSelectValue{name field{... on IssueFieldSingleSelect{name}}}}}'

function Sync-Issue($I) {
    if ($I.state -ne 'OPEN') { Write-Host "#$($I.number): $($I.state.ToLower()), left as it is."; return }
    $priority = @($I.issueFieldValues.nodes | Where-Object { $_.field -and $_.field.name -eq 'Priority' -and $_.name }) | Select-Object -First 1
    $labelled = @($I.labels.nodes | Where-Object { $_.name -eq $Label }).Count -gt 0
    $author = if ($I.author) { $I.author.login } else { '' }
    $exempt = $Exempt -contains $author
    $want = -not $priority -and -not $exempt
    $why = if ($priority) { "Priority $($priority.name)" } elseif ($exempt) { "opened by $author" } else { 'no Priority' }
    if ($want -eq $labelled) { Write-Host "#$($I.number): $why, nothing to change."; return }
    $verb = if ($want) { 'add' } else { 'remove' }
    if ($DryRun) { Write-Host "#$($I.number): $why, would $verb '$Label'."; return }
    if ($want) { Invoke-Gh api -X POST "repos/$Repo/issues/$($I.number)/labels" -f "labels[]=$Label" | Out-Null }
    else { Invoke-Gh api -X DELETE "repos/$Repo/issues/$($I.number)/labels/$Label" | Out-Null }
    Write-Host "#$($I.number): $why, $verb '$Label'."
}

if ($Issue) {
    $q = 'query($o:String!,$r:String!,$n:Int!){repository(owner:$o,name:$r){issue(number:$n){' + $fields + '}}}'
    $i = (Invoke-Graphql $q @{ o = $owner; r = $name; n = $Issue }).repository.issue
    if (-not $i) { throw "Issue #$Issue not found in $Repo." }
    Sync-Issue $i
    exit 0
}

$cursor = $null
do {
    $q = 'query($o:String!,$r:String!,$c:String){repository(owner:$o,name:$r){issues(first:50,states:OPEN,after:$c){pageInfo{hasNextPage endCursor} nodes{' + $fields + '}}}}'
    $vars = @{ o = $owner; r = $name }
    if ($cursor) { $vars.c = $cursor }
    $page = (Invoke-Graphql $q $vars).repository.issues
    foreach ($i in $page.nodes) { Sync-Issue $i }
    $cursor = $page.pageInfo.endCursor
} while ($page.pageInfo.hasNextPage)
