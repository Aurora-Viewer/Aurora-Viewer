<#
.SYNOPSIS
    Tests the agents' shell hook (.claude/settings.json, which has Claude
    Code run .claude/hooks/shell-rules.sh before every Bash or PowerShell
    tool call; AGENTS.md section 5, "Tools"). Each command of the table
    below is sent as the JSON Claude Code sends, through the command line of
    the settings file itself, and must be refused (exit code 2, one line on
    stderr that names the command to use) or let through (exit code 0).

    Only the hook runs, never the commands of the table. Skipped where there
    is no bash (the hook then never runs either).
#>
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$onWindows = ($PSVersionTable.PSEdition -eq 'Desktop') -or $IsWindows

# Git Bash on Windows, the one Claude Code runs its hooks with: bash.exe is
# not in PATH there (and the one of System32 starts WSL).
function Find-Bash {
    if (-not $onWindows) {
        $bash = Get-Command bash -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
        if ($bash) { return $bash.Source }
        return $null
    }
    $git = Get-Command git -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
    if (-not $git) { return $null }
    $dir = Split-Path $git.Source -Parent
    foreach ($i in 1..3) {
        $candidate = Join-Path $dir 'bin\bash.exe'
        if (Test-Path -LiteralPath $candidate) { return $candidate }
        $dir = Split-Path $dir -Parent
        if (-not $dir) { break }
    }
    $null
}

# A JSON string as JavaScript writes it (Claude Code): only the quote, the
# backslash and the control characters are escaped.
function ConvertTo-JsonText([string]$Text) {
    # the backslash first: the other escapes add their own
    '"' + $Text.Replace('\', '\\').Replace('"', '\"').Replace("`n", '\n').Replace("`r", '\r').Replace("`t", '\t') + '"'
}

function New-HookInput([string]$Tool, [string]$Command) {
    '{"session_id":"test","transcript_path":"C:\\Users\\someone\\.claude\\projects\\aurora\\test.jsonl",' +
    '"cwd":"C:\\Aurora-Viewer\\work\\some-task","permission_mode":"default","hook_event_name":"PreToolUse",' +
    '"tool_name":"' + $Tool + '","tool_input":{"command":' + (ConvertTo-JsonText $Command) +
    ',"description":"What the command does"},"tool_use_id":"toolu_test"}'
}

# The hook decodes its input by pieces of $pieceSize characters (its
# piece_size). A script that holds $Text once per character of its JSON
# text, each time placed so that a piece ends before that character; a
# filler no rule looks at, between two ";", moves it there.
function New-PieceScript([string]$Tool, [string]$Text) {
    $json = ConvertTo-JsonText $Text
    $length = $json.Length - 2
    $empty = New-HookInput $Tool ''
    $at = $empty.IndexOf('"command":"') + '"command":"'.Length
    $b = New-Object Text.StringBuilder
    foreach ($cut in 1..($length - 1)) {
        $end = [int][Math]::Ceiling(($at + 2 + $cut) / $pieceSize) * $pieceSize
        $fill = $end - $cut - $at
        [void]$b.Append(';').Append('x', $fill - 2).Append(';').Append($Text)
        $at += $fill + $length
    }
    $b.Append(';').ToString()
}

# Runs the hook's command line on $Json, as Claude Code does: in bash, with
# the project folder in CLAUDE_PROJECT_DIR. $PathDir is its whole PATH.
# Returns @{ Code; Message; Ms }.
function Invoke-Hook([string]$Json, [string]$PathDir, [string]$Project) {
    $psi = New-Object Diagnostics.ProcessStartInfo
    $psi.FileName = $bash
    if ($PSVersionTable.PSEdition -eq 'Desktop') {
        $psi.Arguments = '-c "' + ($hookCommand -replace '"', '\"') + '"'
    } else {
        [void]$psi.ArgumentList.Add('-c')
        [void]$psi.ArgumentList.Add($hookCommand)
    }
    $psi.UseShellExecute = $false
    $psi.CreateNoWindow = $true
    $psi.RedirectStandardInput = $true
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $psi.EnvironmentVariables['PATH'] = $PathDir
    $psi.EnvironmentVariables['CLAUDE_PROJECT_DIR'] = $Project
    $watch = [Diagnostics.Stopwatch]::StartNew()
    $p = [Diagnostics.Process]::Start($psi)
    $bytes = [Text.Encoding]::UTF8.GetBytes($Json)
    # A hook that cannot start (the "missing" and "broken" cases) never reads
    # its input: if its shell has already ended, the write fails with
    # "Broken pipe". Its exit code is what the case checks.
    try {
        $p.StandardInput.BaseStream.Write($bytes, 0, $bytes.Length)
        $p.StandardInput.Close()
    } catch [System.IO.IOException] {
    }
    $out = $p.StandardOutput.ReadToEnd()
    $err = $p.StandardError.ReadToEnd()
    $p.WaitForExit()
    @{ Code = $p.ExitCode; Message = ($out + $err).Trim(); Ms = $watch.Elapsed.TotalMilliseconds }
}

$bash = Find-Bash
if (-not $bash) {
    Write-Host 'No bash here: the shell hook of the agents is not tested (and cannot run).' -ForegroundColor Yellow
    exit 0
}

# The settings file, as Claude Code reads it: one command hook before the
# two shell tools.
$settings = Get-Content -Raw -LiteralPath (Join-Path $root '.claude/settings.json') | ConvertFrom-Json
$entry = @($settings.hooks.PreToolUse)[0]
$handler = @($entry.hooks)[0]
$hookCommand = [string]$handler.command
if ($entry.matcher -ne 'Bash|PowerShell' -or $handler.type -ne 'command' -or $handler.shell -ne 'bash' -or -not $hookCommand -or -not ($handler.timeout -gt 0)) {
    Write-Host '.claude/settings.json: expected one PreToolUse command hook, in bash, on "Bash|PowerShell", with its timeout.' -ForegroundColor Red
    exit 1
}
# The seconds Claude Code gives the hook: past them it lets the call through
# unchecked, after making the agent wait that long.
$timeout = [int]$handler.timeout
$hookText = Get-Content -Raw -LiteralPath (Join-Path $root '.claude/hooks/shell-rules.sh')
if ($hookText -notmatch '(?m)^piece_size=(\d+)\r?$') {
    Write-Host '.claude/hooks/shell-rules.sh: expected its "piece_size=<number>" line.' -ForegroundColor Red
    exit 1
}
$pieceSize = [int]$Matches[1]

# What a refusal must say, per rule.
$search = 'ripgrep'
$tests = 'cargo test --workspace'
$toolchain = 'rust-toolchain.toml'
$wrapper = 'rustc wrapper'
$pass = $null

# Tool, command, expected refusal ($null: let through). NoRg: ripgrep missing.
$cases = @(
    # --- recursive text search, with ripgrep installed
    @{ Tool = 'Bash'; Command = 'grep -rn "fn main" crates/'; Expect = $search }
    @{ Tool = 'Bash'; Command = 'grep -R foo .'; Expect = $search }
    @{ Tool = 'Bash'; Command = 'cd crates && grep --include=*.rs -rI "foo" .'; Expect = $search }
    @{ Tool = 'Bash'; Command = 'grep foo -r src | head -20'; Expect = $search }
    @{ Tool = 'Bash'; Command = "egrep -r 'a|b' src"; Expect = $search }
    @{ Tool = 'Bash'; Command = 'rgrep foo'; Expect = $search }
    @{ Tool = 'Bash'; Command = 'LC_ALL=C grep --recursive foo .'; Expect = $search }
    @{ Tool = 'Bash'; Command = 'files=$(grep -rl foo crates)'; Expect = $search }
    @{ Tool = 'Bash'; Command = "cd docs`ngrep -rn foo ."; Expect = $search }
    @{ Tool = 'PowerShell'; Command = 'findstr /s /i "sccache" *.ps1'; Expect = $search }
    @{ Tool = 'PowerShell'; Command = 'findstr /spin "foo" *.rs'; Expect = $search }
    @{ Tool = 'PowerShell'; Command = 'Get-ChildItem -Recurse -Filter *.rs | Select-String "foo"'; Expect = $search }
    @{ Tool = 'PowerShell'; Command = "gci crates -r -Include *.rs | sls 'foo' -List"; Expect = $search }
    @{ Tool = 'PowerShell'; Command = 'Select-String -Path (Get-ChildItem -Recurse *.md) -Pattern foo'; Expect = $search }
    @{ Tool = 'PowerShell'; Command = 'Set-Location docs; ls -Recurse | Select-String foo | Select-Object -First 5'; Expect = $search }
    # --- the same without ripgrep: nothing to use instead, nothing refused
    @{ Tool = 'Bash'; Command = 'grep -rn foo .'; NoRg = $true; Expect = $pass }
    @{ Tool = 'PowerShell'; Command = 'findstr /s foo *.rs'; NoRg = $true; Expect = $pass }
    @{ Tool = 'PowerShell'; Command = 'Get-ChildItem -Recurse | Select-String foo'; NoRg = $true; Expect = $pass }
    # --- searches and texts that are fine
    @{ Tool = 'Bash'; Command = 'rg -n "grep -r" docs'; Expect = $pass }
    @{ Tool = 'Bash'; Command = 'grep -n foo Cargo.toml'; Expect = $pass }
    @{ Tool = 'Bash'; Command = 'git grep -n foo'; Expect = $pass }
    @{ Tool = 'Bash'; Command = 'cargo build 2>&1 | grep -E "^error" -A 5'; Expect = $pass }
    @{ Tool = 'Bash'; Command = 'git commit -m "Refuse grep -r and cargo test -p in the hook"'; Expect = $pass }
    @{ Tool = 'Bash'; Command = 'git status'; Expect = $pass }
    @{ Tool = 'PowerShell'; Command = 'findstr /i "foo" notes.txt'; Expect = $pass }
    @{ Tool = 'PowerShell'; Command = 'Get-ChildItem -Recurse -Filter target | Measure-Object'; Expect = $pass }
    @{ Tool = 'PowerShell'; Command = 'Get-ChildItem scripts -Filter *.ps1 | Select-String foo'; Expect = $pass }
    @{ Tool = 'PowerShell'; Command = "gh pr list --json title --jq '.[].title'"; Expect = $pass }
    # a Windows path whose \n is two characters, not a new line
    @{ Tool = 'PowerShell'; Command = 'Get-Content .\ngrep -r foo'; Expect = $pass }
    # --- tests: the whole workspace
    @{ Tool = 'Bash'; Command = 'cargo test -p aurora-render staging_round_trip -- --ignored'; Expect = $tests }
    @{ Tool = 'PowerShell'; Command = 'Set-Location C:\Aurora-Viewer\work\some-task; cargo test --package aurora-i18n'; Expect = $tests }
    @{ Tool = 'Bash'; Command = 'cargo test -paurora-net'; Expect = $tests }
    @{ Tool = 'PowerShell'; Command = "`$env:RUST_BACKTRACE = '1'; cargo t -p aurora-viewer chat 2>&1 | Select-Object -Last 20"; Expect = $tests }
    @{ Tool = 'PowerShell'; Command = 'Measure-Command { cargo test -p aurora-prim }'; Expect = $tests }
    @{ Tool = 'PowerShell'; Command = '$out = cargo.exe test --locked -p aurora-net'; Expect = $tests }
    @{ Tool = 'Bash'; Command = 'cargo test --workspace render_thread'; Expect = $pass }
    @{ Tool = 'Bash'; Command = 'cargo test --workspace --locked'; Expect = $pass }
    @{ Tool = 'Bash'; Command = 'cargo test --profile ci --workspace chat -- --test-threads 4'; Expect = $pass }
    @{ Tool = 'Bash'; Command = 'cargo test --release --manifest-path fuzz/Cargo.toml --lib'; Expect = $pass }
    @{ Tool = 'PowerShell'; Command = 'cargo run -p aurora-viewer -- --title "Test some-task"'; Expect = $pass }
    @{ Tool = 'PowerShell'; Command = 'cargo clippy --workspace --all-targets --locked -- -D warnings'; Expect = $pass }
    @{ Tool = 'Bash'; Command = 'cargo build -p aurora-updater'; Expect = $pass }
    # --- one toolchain
    @{ Tool = 'Bash'; Command = 'cargo +nightly build'; Expect = $toolchain }
    @{ Tool = 'PowerShell'; Command = 'cargo +1.99.0 clippy --workspace'; Expect = $toolchain }
    @{ Tool = 'Bash'; Command = 'cd crates/aurora-net && cargo +stable test --workspace'; Expect = $toolchain }
    @{ Tool = 'Bash'; Command = 'cd fuzz && cargo +nightly fuzz run -O mesh --features libfuzzer'; Expect = $pass }
    # --- the compiler wrapper
    @{ Tool = 'Bash'; Command = 'RUSTC_WRAPPER= cargo build'; Expect = $wrapper }
    @{ Tool = 'Bash'; Command = 'export RUSTC_WRAPPER=sccache'; Expect = $wrapper }
    @{ Tool = 'PowerShell'; Command = "`$env:RUSTC_WRAPPER = ''; cargo check"; Expect = $wrapper }
    @{ Tool = 'PowerShell'; Command = "`$env:CARGO_BUILD_RUSTC_WRAPPER='C:\tools\other.exe'"; Expect = $wrapper }
    @{ Tool = 'PowerShell'; Command = "cargo build --config `"build.rustc-wrapper=''`""; Expect = $wrapper }
    @{ Tool = 'Bash'; Command = "cargo build --config 'build.rustc-wrapper=`"`"'"; Expect = $wrapper }
    @{ Tool = 'Bash'; Command = 'rg -n "RUSTC_WRAPPER=" scripts'; Expect = $pass }
    @{ Tool = 'PowerShell'; Command = 'Write-Output $env:RUSTC_WRAPPER'; Expect = $pass }
    @{ Tool = 'PowerShell'; Command = 'cargo build --config net.git-fetch-with-cli=true'; Expect = $pass }
    # --- quotes and backslashes of the JSON text
    @{ Tool = 'PowerShell'; Command = 'rg "a\"b" C:\Users\someone\notes; grep -r foo .'; Expect = $search }
    # --- input that is not a shell tool call
    @{ Json = ''; Expect = $pass }
    @{ Json = 'not JSON, but it names cargo test -p and grep -r'; Expect = $pass }
    @{ Json = '{"tool_name":"Read","tool_input":{"file_path":"notes-on-cargo.md"}}'; Expect = $pass }
    # the same text with spaces, as another JSON writer would print it
    @{ Json = '{ "tool_name": "Bash", "tool_input": { "command": "grep -r foo ." } }'; Expect = $search }
    # --- long input (a file written through a here-document): read to its end,
    # and in the time Claude Code gives the hook, on a slow PC too
    @{ Tool = 'Bash'; Command = ("echo line`n" * 20000) + 'cargo test -p aurora-net'; Expect = $tests; Long = 'a script of 20,000 lines' }
    # The hook decodes its input by pieces: in these two, a piece ends on
    # every character of the text in turn, inside each escape. A quote or a
    # backslash decoded wrong ends the script before its last line, which
    # then goes through...
    @{ Tool = 'PowerShell'; Command = (New-PieceScript 'PowerShell' 'echo "C:\some\dir\" \\ "x"') + 'cargo test -p aurora-net'; Expect = $tests; Long = 'quotes and backslashes cut by the pieces' }
    # ... a line break that is lost gives this grep the "-r" of the next
    # line, and one that is made up (the \n of ".\nrgrep" is a backslash
    # and an n) starts a line with "rgrep": both would be refused.
    @{ Tool = 'Bash'; Command = New-PieceScript 'Bash' "grep x .\nrgrep `n -r y"; Expect = $pass; Long = 'line breaks cut by the pieces' }
    # a long text before the command
    @{ Json = '{"tool_name":"Bash","tool_input":{"description":"' + ('What the command does. ' * 10000) + '","command":"cargo test -p aurora-net"}}'; Expect = $tests; Long = 'a long field before the command' }
    # --- a hook that cannot run lets everything through, it never refuses
    # everything: its file is missing, or an older bash stops on its syntax
    @{ Tool = 'Bash'; Command = 'cargo test -p aurora-net'; Project = 'missing'; Expect = $pass }
    @{ Tool = 'Bash'; Command = 'cargo test -p aurora-net'; Project = 'broken'; Expect = $pass }
)

$temp = Join-Path ([IO.Path]::GetTempPath()) "aurora-hook-test-$([Guid]::NewGuid().ToString('N'))"
$withRg = Join-Path $temp 'with-rg'
$withoutRg = Join-Path $temp 'without-rg'
$projects = @{ missing = (Join-Path $temp 'missing'); broken = (Join-Path $temp 'broken') }
New-Item -ItemType Directory -Path $withRg, $withoutRg, $projects.missing, (Join-Path $projects.broken '.claude/hooks') -Force | Out-Null
$failed = 0
$total = 0.0
$slowest = 0.0
$slowestLong = 0.0
$slowestLongName = ''
try {
    # a stand-in the hook only looks for, never runs
    if ($onWindows) {
        [IO.File]::WriteAllBytes((Join-Path $withRg 'rg.exe'), [byte[]]@())
    } else {
        Set-Content -LiteralPath (Join-Path $withRg 'rg') -Value '#!/bin/sh'
        & chmod +x (Join-Path $withRg 'rg')
        # The hook's command line starts `bash` from PATH, and PATH is only
        # the stand-in folder: without bash in it the hook cannot start
        # off Windows (Git's bash.exe finds it there). The hook needs
        # nothing else: builtins only.
        foreach ($d in $withRg, $withoutRg) {
            New-Item -ItemType SymbolicLink -Path (Join-Path $d 'bash') -Target $bash | Out-Null
        }
    }
    [IO.File]::WriteAllText((Join-Path $projects.broken '.claude/hooks/shell-rules.sh'), "if then (`n")
    foreach ($c in $cases) {
        $json = if ($c.ContainsKey('Json')) { $c.Json } else { New-HookInput $c.Tool $c.Command }
        $shown = if ($c.Long) { "long input: $($c.Long)" } elseif ($c.ContainsKey('Json')) { "input: $($c.Json)" } else { "$($c.Tool): $($c.Command)" }
        $project = if ($c.Project) { $projects[$c.Project] } else { $root }
        $r = Invoke-Hook $json $(if ($c.NoRg) { $withoutRg } else { $withRg }) $project
        if ($c.Long) {
            if ($r.Ms -gt $slowestLong) { $slowestLong = $r.Ms; $slowestLongName = $c.Long }
        } else {
            $total += $r.Ms
            if ($r.Ms -gt $slowest) { $slowest = $r.Ms }
        }
        $problem = $null
        if ($c.Expect) {
            if ($r.Code -ne 2) { $problem = "expected a refusal (exit code 2), got exit code $($r.Code): $($r.Message)" }
            elseif ($r.Message -notlike "*$($c.Expect)*") { $problem = "refused, but the message does not name '$($c.Expect)': $($r.Message)" }
            elseif (($r.Message -split "`n").Count -ne 1) { $problem = "the refusal must be one line: $($r.Message)" }
        } elseif ($r.Code -ne 0) {
            $problem = "expected to pass, got exit code $($r.Code): $($r.Message)"
        } elseif ($r.Message -and -not $c.Project) {
            # bash's own complaint is expected from a hook that cannot run
            $problem = "expected to pass in silence, got: $($r.Message)"
        }
        if (-not $problem -and $c.Long -and $r.Ms -gt 1000 * $timeout) { $problem = "took $([int]$r.Ms) ms (the hook is given $timeout s)" }
        if ($problem) {
            Write-Host "  $shown`n    $problem" -ForegroundColor Red
            $failed++
        }
    }
} finally {
    Remove-Item -LiteralPath $temp -Recurse -Force -ErrorAction SilentlyContinue
}
if ($failed) { Write-Host "Shell hook of the agents: $failed of $($cases.Count) commands went the wrong way." -ForegroundColor Red; exit 1 }
$count = @($cases | Where-Object { -not $_.Long }).Count
Write-Host ("Shell hook of the agents: {0} commands refused or let through as expected ({1:N0} ms per call, slowest {2:N0} ms; long input: {3:N0} ms at most, on {4}, of the {5} s the hook is given)." -f $cases.Count, ($total / $count), $slowest, $slowestLong, $slowestLongName, $timeout)
