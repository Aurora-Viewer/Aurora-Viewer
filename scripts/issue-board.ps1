<#
.SYNOPSIS
    Reads or moves an issue on the team's GitHub project (Aurora Team
    Planning), and lists the issues still to take, highest priority first.
    The project manager sets Priority (an issue field of the organisation,
    like the type) and the milestone; the agents only move the project's
    Status (AGENTS.md, section 15): In progress when they start; at the end
    Done, Waiting Review (the fix waits for the human's or the QA's
    validation) or Waiting for user response (a comment asked the reporter for
    something); On hold, with a comment, when something outside the work
    blocks it. New issues land in Triage (the board's own workflows), which
    the QA sorts to Todo: the agents never take an issue out of it on their
    own. An issue not on the project yet is added to it when its Status is
    set.

    Needs the project scope on the gh token, once per machine:
    gh auth refresh -s project

.EXAMPLE
    ./scripts/issue-board.ps1 -Issue 623                         # Status, Priority, milestone
    ./scripts/issue-board.ps1 -Issue 623 -Status 'In progress'
    ./scripts/issue-board.ps1 -Issue 623 -Status 'Waiting for user response'
    ./scripts/issue-board.ps1 -Todo                              # open Todo issues, by priority
    ./scripts/issue-board.ps1 -List 'On hold'                    # open issues with that Status, by priority
    ./scripts/issue-board.ps1 -List Triage                       # new issues the QA has not sorted yet
#>
param(
    [int]$Issue,
    [string]$Status,
    [switch]$Todo,
    [string]$List,
    [string]$Project = 'Aurora Team Planning'
)
. "$PSScriptRoot\common.ps1"
# gh writes UTF-8; the console decodes it with its OEM code page by default
try { [Console]::OutputEncoding = [Text.Encoding]::UTF8 } catch { }

if ($Todo) { $List = 'Todo' }
if (-not $List -and -not $Issue) { throw 'Give -Issue <n> (with -Status to move it), -Todo or -List <status>.' }
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

$fieldNames = @{ sf = 'Status' }
$value = '... on ProjectV2ItemFieldSingleSelectValue { name }'
# Priority is an issue field of the organisation, not a field of the project
$issueFields = 'issueFieldValues(first:20){nodes{... on IssueFieldSingleSelectValue{name field{... on IssueFieldSingleSelect{name options{name}}}}}}'

# The project and its Status field with the options.
$q = 'query($o:String!,$q:String!,$sf:String!){organization(login:$o){projectsV2(first:20,query:$q){nodes{id title ' +
    'status:field(name:$sf){... on ProjectV2SingleSelectField{id name options{id name}}}}}}}'
$projects = (Invoke-Graphql $q (@{ o = $owner; q = $Project } + $fieldNames)).organization.projectsV2.nodes
$board = @($projects | Where-Object { $_.title -eq $Project }) | Select-Object -First 1
if (-not $board) { throw "No project named '$Project' in $owner (found: $(($projects | ForEach-Object title) -join ', '))." }
if (-not $board.status) { throw "The project '$Project' has no single select field named Status." }

# Priority of an issue: its name and its rank among the options (the first,
# Urgent, is the most urgent); rank 99 without a priority.
function Get-Priority($Issue) {
    $v = @($Issue.issueFieldValues.nodes | Where-Object { $_.field -and $_.field.name -eq 'Priority' }) | Select-Object -First 1
    if (-not $v) { return [pscustomobject]@{ Name = $null; Rank = 99 } }
    $names = @($v.field.options | ForEach-Object name)
    [pscustomobject]@{ Name = $v.name; Rank = [array]::IndexOf($names, $v.name) }
}

# Status option by name, case and spaces ignored ("in progress", "waiting-review")
$key = { param($t) ($t -replace '[\s_-]', '').ToLowerInvariant() }
function Find-StatusOption([string]$Wanted) {
    $o = @($board.status.options | Where-Object { (& $key $_.name) -eq (& $key $Wanted) }) | Select-Object -First 1
    if (-not $o) { throw "No Status '$Wanted' on the project (choices: $(($board.status.options | ForEach-Object name) -join ', '))." }
    $o
}

if ($List) {
    # open issues of this repository with that Status
    $listed = (Find-StatusOption $List).name
    $rows = @()
    $after = $null
    do {
        $q = 'query($p:ID!,$sf:String!,$after:String){node(id:$p){... on ProjectV2{items(first:100,after:$after){' +
            'pageInfo{hasNextPage endCursor} nodes{status:fieldValueByName(name:$sf){' + $value + '} ' +
            'content{... on Issue{number title state repository{nameWithOwner} milestone{title} assignees(first:5){nodes{login}} ' + $issueFields + '}}}}}}}'
        $vars = @{ p = $board.id } + $fieldNames
        if ($after) { $vars.after = $after }
        $page = (Invoke-Graphql $q $vars).node.items
        foreach ($n in $page.nodes) {
            $c = $n.content
            if (-not $c -or -not $c.number -or $c.state -ne 'OPEN' -or $c.repository.nameWithOwner -ne "$owner/$name") { continue }
            if ($n.status.name -ne $listed) { continue }
            $p = Get-Priority $c
            $rows += [pscustomobject]@{
                Rank      = $p.Rank
                Issue     = "#$($c.number)"
                Priority  = if ($p.Name) { $p.Name } else { '-' }
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
    $q = 'query($o:String!,$r:String!,$n:Int!,$sf:String!){repository(owner:$o,name:$r){issue(number:$n){id title state milestone{title} ' + $issueFields + ' ' +
        'projectItems(first:20){nodes{id project{id} status:fieldValueByName(name:$sf){' + $value + '}}}}}}'
    $i = (Invoke-Graphql $q (@{ o = $owner; r = $name; n = $Issue } + $fieldNames)).repository.issue
    if (-not $i) { throw "Issue #$Issue not found in $owner/$name." }
    $item = @($i.projectItems.nodes | Where-Object { $_.project.id -eq $board.id }) | Select-Object -First 1
    [pscustomobject]@{ Issue = $i; Item = $item }
}

function Show-IssueItem($s) {
    $i = $s.Issue
    Write-Host "#$Issue $($i.title) ($($i.state.ToLowerInvariant()))"
    $p = (Get-Priority $i).Name
    $pr = if ($p) { $p } else { '- (set by the project manager)' }
    if (-not $s.Item) { Write-Host "  Not on the project '$Project'.   Priority: $pr"; return }
    $st = if ($s.Item.status) { $s.Item.status.name } else { '-' }
    $ms = if ($i.milestone) { $i.milestone.title } else { '-' }
    Write-Host "  Status: $st   Priority: $pr   Milestone: $ms"
}

$s = Get-IssueItem
if (-not $Status) { Show-IssueItem $s; exit 0 }

$option = Find-StatusOption $Status

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
    Write-Host '  The issue is still open: close it once resolved (AGENTS.md, section 15), or use Waiting Review / Waiting for user response.' -ForegroundColor Yellow
}
if ($option.name -eq 'On hold') {
    Write-Host '  On hold needs a comment on the issue saying why and what unblocks it (AGENTS.md, section 15).' -ForegroundColor Yellow
}
