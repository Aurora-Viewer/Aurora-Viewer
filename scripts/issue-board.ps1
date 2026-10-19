<#
.SYNOPSIS
    Reads or moves an issue on the team's GitHub project (Aurora Team
    Planning), and lists the issues still to take, highest priority first.
    The project manager sets Priority and the milestone; the agents only move
    Status: In progress when they start, Waiting Review or Done at the end
    (AGENTS.md, section 15). An issue not on the project yet is added to it
    when its Status is set.

    Needs the project scope on the gh token, once per machine:
    gh auth refresh -s project

.EXAMPLE
    ./scripts/issue-board.ps1 -Issue 623                         # Status, Priority, milestone
    ./scripts/issue-board.ps1 -Issue 623 -Status 'In progress'
    ./scripts/issue-board.ps1 -Todo                              # open Todo issues, by priority
#>
param(
    [int]$Issue,
    [string]$Status,
    [switch]$Todo,
    [string]$Project = 'Aurora Team Planning'
)
. "$PSScriptRoot\common.ps1"
# gh writes UTF-8; the console decodes it with its OEM code page by default
try { [Console]::OutputEncoding = [Text.Encoding]::UTF8 } catch { }

if (-not $Todo -and -not $Issue) { throw 'Give -Issue <n> (with -Status to move it) or -Todo.' }
if ($Status -and -not $Issue) { throw '-Status needs -Issue <n>.' }

# The GitHub repository, from this repository's origin (like merge-pr.ps1).
$origin = (git -C $PSScriptRoot remote get-url origin).Trim()
if ($origin -notmatch 'github\.com[:/](?<owner>[^/]+)/(?<name>[^/]+?)(\.git)?$') { throw "Unexpected origin remote: $origin" }
$owner = $Matches.owner; $name = $Matches.name

# Every string goes in a GraphQL variable, none in the query: Windows
# PowerShell 5.1 drops the double quotes of an argument given to a native
# command.
function Invoke-Graphql([string]$Query, [hashtable]$Vars) {
    $a = @('api', 'graphql', '-f', "query=$Query")
    foreach ($k in $Vars.Keys) {
        $flag = if ($Vars[$k] -is [int]) { '-F' } else { '-f' }
        $a += $flag, "$k=$($Vars[$k])"
    }
    $out = (& gh @a 2>&1 | ForEach-Object { "$_" }) -join "`n"
    if ($LASTEXITCODE -ne 0) {
        if ($out -match 'read:project|INSUFFICIENT_SCOPES') {
            throw 'The gh token cannot read the projects. The human runs once, in a terminal: gh auth refresh -s project'
        }
        throw "gh api graphql failed: $out"
    }
    ($out | ConvertFrom-Json).data
}

$fieldNames = @{ sf = 'Status'; pf = 'Priority' }
$single = '... on ProjectV2SingleSelectField { id name options { id name } }'
$value = '... on ProjectV2ItemFieldSingleSelectValue { name }'

# The project, its Status and Priority fields with their options (in the
# project's order: the first Priority option is the most urgent).
$q = 'query($o:String!,$q:String!,$sf:String!,$pf:String!){organization(login:$o){projectsV2(first:20,query:$q){nodes{id title ' +
    'status:field(name:$sf){' + $single + '} priority:field(name:$pf){' + $single + '}}}}}'
$projects = (Invoke-Graphql $q (@{ o = $owner; q = $Project } + $fieldNames)).organization.projectsV2.nodes
$board = @($projects | Where-Object { $_.title -eq $Project }) | Select-Object -First 1
if (-not $board) { throw "No project named '$Project' in $owner (found: $(($projects | ForEach-Object title) -join ', '))." }
if (-not $board.status) { throw "The project '$Project' has no single select field named Status." }
$rank = @{}
if ($board.priority) { for ($i = 0; $i -lt $board.priority.options.Count; $i++) { $rank[$board.priority.options[$i].name] = $i } }

if ($Todo) {
    # open issues of this repository whose Status is Todo
    $rows = @()
    $after = $null
    do {
        $q = 'query($p:ID!,$sf:String!,$pf:String!,$after:String){node(id:$p){... on ProjectV2{items(first:100,after:$after){' +
            'pageInfo{hasNextPage endCursor} nodes{status:fieldValueByName(name:$sf){' + $value + '} priority:fieldValueByName(name:$pf){' + $value + '} ' +
            'content{... on Issue{number title state repository{nameWithOwner} milestone{title} assignees(first:5){nodes{login}}}}}}}}}'
        $vars = @{ p = $board.id } + $fieldNames
        if ($after) { $vars.after = $after }
        $page = (Invoke-Graphql $q $vars).node.items
        foreach ($n in $page.nodes) {
            $c = $n.content
            if (-not $c -or -not $c.number -or $c.state -ne 'OPEN' -or $c.repository.nameWithOwner -ne "$owner/$name") { continue }
            if ($n.status.name -ne 'Todo') { continue }
            $p = $n.priority.name
            $rows += [pscustomobject]@{
                Rank      = if ($p -and $rank.ContainsKey($p)) { $rank[$p] } else { 99 }
                Issue     = "#$($c.number)"
                Priority  = if ($p) { $p } else { '-' }
                Milestone = if ($c.milestone) { $c.milestone.title } else { '-' }
                Assignees = (@($c.assignees.nodes | ForEach-Object login) -join ', ')
                Title     = $c.title
            }
        }
        $after = $page.pageInfo.endCursor
    } while ($page.pageInfo.hasNextPage)
    # highest priority first, then the release milestone before the rest
    $rows | Sort-Object Rank, @{ Expression = { $_.Milestone -eq '-' } }, @{ Expression = { [int]$_.Issue.TrimStart('#') } } |
        Format-Table Issue, Priority, Milestone, Assignees, Title -AutoSize | Out-String -Width 200 | Write-Host
    exit 0
}

function Get-IssueItem {
    $q = 'query($o:String!,$r:String!,$n:Int!,$sf:String!,$pf:String!){repository(owner:$o,name:$r){issue(number:$n){id title state milestone{title} ' +
        'projectItems(first:20){nodes{id project{id} status:fieldValueByName(name:$sf){' + $value + '} priority:fieldValueByName(name:$pf){' + $value + '}}}}}}'
    $i = (Invoke-Graphql $q (@{ o = $owner; r = $name; n = $Issue } + $fieldNames)).repository.issue
    if (-not $i) { throw "Issue #$Issue not found in $owner/$name." }
    $item = @($i.projectItems.nodes | Where-Object { $_.project.id -eq $board.id }) | Select-Object -First 1
    [pscustomobject]@{ Issue = $i; Item = $item }
}

function Show-IssueItem($s) {
    $i = $s.Issue
    Write-Host "#$Issue $($i.title) ($($i.state.ToLowerInvariant()))"
    if (-not $s.Item) { Write-Host "  Not on the project '$Project'."; return }
    $st = if ($s.Item.status) { $s.Item.status.name } else { '-' }
    $pr = if ($s.Item.priority) { $s.Item.priority.name } else { '- (set by the project manager)' }
    $ms = if ($i.milestone) { $i.milestone.title } else { '-' }
    Write-Host "  Status: $st   Priority: $pr   Milestone: $ms"
}

$s = Get-IssueItem
if (-not $Status) { Show-IssueItem $s; exit 0 }

# option by name, case and spaces ignored ("in progress", "waiting-review")
$key = { param($t) ($t -replace '[\s_-]', '').ToLowerInvariant() }
$option = @($board.status.options | Where-Object { (& $key $_.name) -eq (& $key $Status) }) | Select-Object -First 1
if (-not $option) { throw "No Status '$Status' on the project (choices: $(($board.status.options | ForEach-Object name) -join ', '))." }

if (-not $s.Item) {
    $q = 'mutation($p:ID!,$c:ID!){addProjectV2ItemById(input:{projectId:$p,contentId:$c}){item{id}}}'
    $added = (Invoke-Graphql $q @{ p = $board.id; c = $s.Issue.id }).addProjectV2ItemById.item
    Write-Host "#$Issue added to the project '$Project'."
    $s.Item = [pscustomobject]@{ id = $added.id }
}
$q = 'mutation($p:ID!,$i:ID!,$f:ID!,$v:String!){updateProjectV2ItemFieldValue(input:{projectId:$p,itemId:$i,fieldId:$f,value:{singleSelectOptionId:$v}}){projectV2Item{id}}}'
[void](Invoke-Graphql $q @{ p = $board.id; i = $s.Item.id; f = $board.status.id; v = $option.id })
Show-IssueItem (Get-IssueItem)
if ($option.name -eq 'Done' -and $s.Issue.state -eq 'OPEN') {
    Write-Host '  The issue is still open: close it once resolved (AGENTS.md, section 15), or use Waiting Review.' -ForegroundColor Yellow
}
