# Crash report triage, shared by scripts/crash-triage.ps1 (the Crash triage
# workflow) and scripts/test-crash-triage.ps1. Pure functions, no GitHub
# call. An issue body is written by anyone: every value read from it is
# validated against a fixed shape again, like the viewer did (crash/summary.rs).

$CrashMarker = '<!-- aurora-crash v1 -->'
$TriageStart = '<!-- crash-triage:start -->'
$TriageEnd = '<!-- crash-triage:end -->'
# The viewer's own module, the one whose symbols each release publishes.
$OwnModule = 'aurora_viewer'

function Read-CrashReport([string]$Body) {
    if (-not $Body -or -not $Body.Contains($CrashMarker)) { return $null }
    # A body already triaged keeps its report above the added section.
    $start = $Body.IndexOf($TriageStart)
    if ($start -ge 0) { $Body = $Body.Substring(0, $start) }
    $report = [ordered]@{ Version = $null; Kind = $null; Location = $null; System = $null; Gpu = $null; Frames = @(); Modules = @{} }
    $section = ''
    $inFence = $false
    foreach ($raw in ($Body -split "`r?`n")) {
        $line = $raw.TrimEnd()
        if ($line -match '^### (.+)$') { $section = $Matches[1]; $inFence = $false; continue }
        if ($line -eq '```text') { $inFence = $true; continue }
        if ($line -eq '```') { $inFence = $false; continue }
        if ($line -match '^\| (Version|Crash|System|GPU) \| (.+) \|$') {
            $value = $Matches[2]
            switch ($Matches[1]) {
                'Version' { if ($value -match '^\d+\.\d+\.\d+([-+][0-9A-Za-z.+-]*)?$') { $report.Version = $value } }
                'Crash' {
                    if ($value -eq 'Rust panic') { $report.Kind = 'Rust panic' }
                    elseif ($value -eq 'Freeze') { $report.Kind = 'Freeze' }
                    elseif ($value -eq 'GPU not responding') { $report.Kind = 'GPU not responding' }
                    elseif ($value -match '^Out of memory \(\d{1,20} bytes\)$') { $report.Kind = 'Out of memory' }
                    elseif ($value -match '^Native exception (0x[0-9A-F]{8})$') { $report.Kind = "Native exception $($Matches[1])" }
                }
                'System' { if ($value -match '^[A-Za-z0-9 ._()\-,+/]{1,120}$') { $report.System = $value } }
                'GPU' { if ($value -match '^[A-Za-z0-9 ._()\-,+/]{1,160}$') { $report.Gpu = $value } }
            }
            continue
        }
        if ($line -match '^Location: `([A-Za-z0-9_./-]{1,200}:\d+:\d+)`$') { $report.Location = $Matches[1]; continue }
        if (-not $inFence) { continue }
        if ($section -eq 'Stack') {
            if ($line -match '^\d\d \?$') { $report.Frames += , @{ Module = $null; Offset = $null; Text = '?' } }
            elseif ($line -match '^\d\d ([A-Za-z0-9_.-]{1,64})\+0x([0-9a-f]{1,16})$') {
                $report.Frames += , @{ Module = $Matches[1]; Offset = [Convert]::ToInt64($Matches[2], 16); Text = "$($Matches[1])+0x$($Matches[2])" }
            }
        }
        elseif ($section -eq 'Modules' -and $line -match '^([A-Za-z0-9_.-]{1,64})( id=([0-9A-F]{1,48}))?( code=([0-9A-Fa-f]{1,48}))?$') {
            $report.Modules[$Matches[1]] = @{ Id = $Matches[3]; Code = $Matches[5] }
        }
    }
    if (-not $report.Version -or -not $report.Kind) { return $null }
    $report
}

# Only tagged releases have published symbols; a development build
# (0.7.3+13.gabcdef) has none.
function Get-ReleaseTag([string]$Version) {
    if ($Version -match '^\d+\.\d+\.\d+(-[0-9A-Za-z.]+)?$') { return "v$Version" }
    $null
}

# Source paths of the symbol file as paths inside the sources: the build
# folders of CI or of a PC name nothing useful.
function Format-SourcePath([string]$Path) {
    if (-not $Path) { return '' }
    $p = $Path.Replace('\', '/')
    $i = $p.IndexOf('/registry/src/')
    if ($i -ge 0) {
        $rest = $p.Substring($i + '/registry/src/'.Length)
        $slash = $rest.IndexOf('/')
        if ($slash -ge 0) { return $rest.Substring($slash + 1) }
    }
    if ($p.Contains('/rustc/')) {
        $i = $p.IndexOf('/library/')
        if ($i -ge 0) { return $p.Substring($i + 1) }
    }
    $i = $p.IndexOf('/crates/')
    if ($i -ge 0) { return $p.Substring($i + 1) }
    if ($p.StartsWith('crates/')) { return $p }
    $p.Substring($p.LastIndexOf('/') + 1)
}

<#
    Resolves addresses (offsets in the module) with a Breakpad .sym file,
    read once as a stream. Returns address -> frames, innermost first: the
    inlined functions (INLINE records) then the function that contains them.
    Each frame: Function, File, Line.
#>
function Resolve-BreakpadAddresses([string]$Path, [long[]]$Addresses) {
    $files = @{}
    $origins = @{}
    $found = @{}
    $publics = @{}
    $current = $null
    $finish = {
        param($func)
        foreach ($a in $func.Matched) {
            $row = $null
            foreach ($l in $func.Lines) { if ($a -ge $l.Start -and $a -lt $l.End) { $row = $l; break } }
            $chain = @($func.Inlines | Where-Object { $_.Ranges | Where-Object { $a -ge $_[0] -and $a -lt $_[1] } } | Sort-Object Depth)
            $frames = @()
            $file = $null; $line = $null
            if ($row) { $file = $row.File; $line = $row.Line }
            for ($k = $chain.Count - 1; $k -ge 0; $k--) {
                $frames += , @{ Function = $chain[$k].Origin; File = $file; Line = $line }
                $file = $chain[$k].CallFile; $line = $chain[$k].CallLine
            }
            $frames += , @{ Function = $func.Name; File = $file; Line = $line }
            $found[$a] = $frames
        }
    }
    foreach ($line in [IO.File]::ReadLines($Path)) {
        if ($line.StartsWith('FILE ')) { $p = $line.Split(' ', 3); $files[$p[1]] = $p[2]; continue }
        if ($line.StartsWith('INLINE_ORIGIN ')) { $p = $line.Split(' ', 3); $origins[$p[1]] = $p[2]; continue }
        if ($line.StartsWith('FUNC ')) {
            if ($current) { & $finish $current }
            $rest = $line.Substring(5)
            if ($rest.StartsWith('m ')) { $rest = $rest.Substring(2) }
            $p = $rest.Split(' ', 4)
            $start = [Convert]::ToInt64($p[0], 16)
            $end = $start + [Convert]::ToInt64($p[1], 16)
            # A plain loop: a pipeline per FUNC record is far too slow here.
            $matched = @()
            foreach ($a in $Addresses) { if ($a -ge $start -and $a -lt $end) { $matched += $a } }
            $current = $null
            if ($matched.Count) { $current = @{ Name = $p[3]; Matched = $matched; Lines = @(); Inlines = @() } }
            continue
        }
        if ($line.StartsWith('PUBLIC ')) {
            if ($current) { & $finish $current; $current = $null }
            $rest = $line.Substring(7)
            if ($rest.StartsWith('m ')) { $rest = $rest.Substring(2) }
            $p = $rest.Split(' ', 3)
            $start = [Convert]::ToInt64($p[0], 16)
            # The nearest symbol below each address, for code without FUNC.
            foreach ($a in $Addresses) {
                if ($start -le $a -and (-not $publics.ContainsKey($a) -or $publics[$a].Start -lt $start)) { $publics[$a] = @{ Start = $start; Name = $p[2] } }
            }
            continue
        }
        if (-not $current) { continue }
        if ($line.StartsWith('INLINE ')) {
            $p = $line.Substring(7).Split(' ')
            $ranges = @()
            for ($i = 4; $i + 1 -lt $p.Count; $i += 2) {
                $s = [Convert]::ToInt64($p[$i], 16)
                $ranges += , @($s, ($s + [Convert]::ToInt64($p[$i + 1], 16)))
            }
            $current.Inlines += , @{ Depth = [int]$p[0]; CallLine = [int]$p[1]; CallFile = $p[2]; Origin = $p[3]; Ranges = $ranges }
            continue
        }
        if ($line -match '^([0-9a-f]+) ([0-9a-f]+) (\d+) (\d+)$') {
            $s = [Convert]::ToInt64($Matches[1], 16)
            $current.Lines += , @{ Start = $s; End = $s + [Convert]::ToInt64($Matches[2], 16); Line = [int]$Matches[3]; File = $Matches[4] }
        }
    }
    if ($current) { & $finish $current }
    $result = @{}
    foreach ($a in $Addresses) {
        if ($found.ContainsKey($a)) {
            $result[$a] = @($found[$a] | ForEach-Object {
                    $name = $_.Function
                    if ($origins.ContainsKey($name)) { $name = $origins[$name] }
                    $file = $null
                    if ($null -ne $_.File -and $files.ContainsKey($_.File)) { $file = Format-SourcePath $files[$_.File] }
                    @{ Function = $name; File = $file; Line = $_.Line }
                })
        }
        elseif ($publics.ContainsKey($a)) {
            $result[$a] = @(, @{ Function = $publics[$a].Name; File = $null; Line = $null })
        }
    }
    $result
}

<#
    The report's frames with the viewer's own ones resolved. A return
    address follows its call: look up the byte before it, except for the
    faulting instruction itself (first frame of a native exception).
    Returns a list of @{ Index; Function; Location; Inlined; Own }.
#>
function Get-SymbolizedFrames($Report, [string]$SymbolFile) {
    $addresses = @{}
    for ($i = 0; $i -lt $Report.Frames.Count; $i++) {
        $f = $Report.Frames[$i]
        if ($f.Module -ne $OwnModule) { continue }
        $exact = ($i -eq 0 -and $Report.Kind -like 'Native exception*')
        $addresses[$i] = if ($exact) { $f.Offset } else { $f.Offset - 1 }
    }
    $resolved = @{}
    if ($SymbolFile -and $addresses.Count) { $resolved = Resolve-BreakpadAddresses $SymbolFile ([long[]]@($addresses.Values | Sort-Object -Unique)) }
    $out = @()
    for ($i = 0; $i -lt $Report.Frames.Count; $i++) {
        $f = $Report.Frames[$i]
        $frames = $null
        if ($addresses.ContainsKey($i) -and $resolved.ContainsKey($addresses[$i])) { $frames = $resolved[$addresses[$i]] }
        if (-not $frames) {
            $out += , @{ Index = $i; Function = $f.Text; Location = ''; Inlined = $false; Own = $false; Resolved = $false }
            continue
        }
        for ($k = 0; $k -lt $frames.Count; $k++) {
            $location = ''
            if ($frames[$k].File) { $location = "$($frames[$k].File):$($frames[$k].Line)" }
            $out += , @{ Index = $i; Function = $frames[$k].Function.Replace('`', "'"); Location = $location; Inlined = ($k -lt $frames.Count - 1); Own = $true; Resolved = $true }
        }
    }
    $out
}

# Generic arguments and closure numbering change with unrelated edits.
function Get-StableName([string]$Name) {
    $s = New-Object System.Text.StringBuilder
    $depth = 0
    foreach ($c in $Name.ToCharArray()) {
        if ($c -eq '<') { $depth++; continue }
        if ($c -eq '>' -and $depth -gt 0) { $depth--; continue }
        if ($depth -eq 0) { [void]$s.Append($c) }
    }
    ($s.ToString() -replace '\(.*\)$', '' -replace '::closure\$\d+', '::closure').Trim()
}

# Frames of the panic and exception machinery, of the crash handler itself
# and of the process start: the same in every crash.
$IgnoredFrame = '^(std|core|alloc|panic_unwind|backtrace|backtrace_rs)::|^__rust|^rust_(begin_unwind|panic)|^aurora_viewer::(crash|logging|allocator)::|^(mimalloc|libmimalloc_sys)::|^_?mi_|^_*scrt_|^invoke_main$|^main$|^(kernelbase|ntdll|kernel32|ucrtbase|vcruntime140)(\+0x[0-9a-f]+)?$|^\?$'

<#
    The crash's signature: its kind and its first three meaningful frames,
    by function name (stable across versions while the code stays). Without
    symbols, the module offsets of this exact build.
#>
function Get-CrashSignature($Report, $Frames) {
    $kind = 'panic'
    if ($Report.Kind -like 'Native exception*') { $kind = $Report.Kind.Substring('Native exception '.Length) }
    elseif ($Report.Kind -eq 'Freeze') { $kind = 'freeze' }
    elseif ($Report.Kind -eq 'GPU not responding') { $kind = 'gpu stall' }
    elseif ($Report.Kind -eq 'Out of memory') { $kind = 'oom' }
    $resolved = @($Frames | Where-Object { $_.Resolved })
    $keys = @()
    if ($resolved.Count) {
        foreach ($f in $Frames) {
            $name = $f.Function
            if (-not $f.Resolved) { $name = $name -replace '\+0x[0-9a-f]+$', '' }
            $name = Get-StableName $name
            if ($name -match $IgnoredFrame) { continue }
            $keys += $name
            if ($keys.Count -ge 3) { break }
        }
    }
    else {
        $id = ''
        if ($Report.Modules.ContainsKey($OwnModule)) { $id = $Report.Modules[$OwnModule].Id }
        $keys = @("unsymbolized $id") + @($Report.Frames | Where-Object { $_.Module -eq $OwnModule } | Select-Object -First 3 | ForEach-Object { $_.Text })
    }
    $text = "$kind|$($keys -join '|')"
    $sha = [Security.Cryptography.SHA256]::Create()
    $hash = $sha.ComputeHash([Text.Encoding]::UTF8.GetBytes($text))
    @{ Id = (-join ($hash[0..7] | ForEach-Object { $_.ToString('x2') })); Frames = $keys }
}

function Get-TriageTitle($Report, $Signature) {
    $kind = 'Rust panic'
    if ($Report.Kind -like 'Native exception*') { $kind = 'native exception ' + $Report.Kind.Substring('Native exception '.Length) }
    elseif ($Report.Kind -eq 'Freeze') { $kind = 'freeze' }
    elseif ($Report.Kind -eq 'GPU not responding') { $kind = 'GPU stall' }
    elseif ($Report.Kind -eq 'Out of memory') { $kind = 'out of memory' }
    # Without symbols, the viewer's title (kind and fault) says as much.
    $keys = @($Signature.Frames)
    if (-not $keys.Count -or $keys[0] -like 'unsymbolized*') { return $null }
    $name = $keys[0]
    if ($name.Length -gt 100) { $name = '...' + $name.Substring($name.Length - 97) }
    "Crash: $kind in $name"
}

function Format-TriageSection($Frames, [string]$SymbolsName, $Signature) {
    $lines = @($TriageStart, '### Symbolized stack', '')
    if ($SymbolsName) { $lines += "Symbols: ``$SymbolsName`` (debug id checked)." }
    else { $lines += 'No published symbols for this build (a development build, or a release made before they were published): the stack stays as module offsets.' }
    $lines += ''
    $lines += '```text'
    foreach ($f in $Frames) {
        $text = '{0:00} {1}' -f $f.Index, $f.Function
        if ($f.Inlined) { $text += ' (inlined)' }
        if ($f.Location) { $text += "  $($f.Location)" }
        $lines += $text
    }
    $lines += '```'
    $lines += ''
    $lines += "Signature ``$($Signature.Id)``: $(@($Signature.Frames) -join ' > ')"
    $lines += "<!-- crash-signature: $($Signature.Id) -->"
    $lines += $TriageEnd
    $lines -join "`n"
}

# Replaces an earlier triage section (re-run, new symbols) instead of adding one.
function Merge-TriageSection([string]$Body, [string]$Section) {
    $start = $Body.IndexOf($TriageStart)
    $end = $Body.IndexOf($TriageEnd)
    if ($start -ge 0 -and $end -gt $start) {
        return $Body.Substring(0, $start) + $Section + $Body.Substring($end + $TriageEnd.Length)
    }
    $Body.TrimEnd() + "`n`n" + $Section + "`n"
}

# The first open issue (lowest number) with the same signature, other than this one.
function Find-DuplicateIssue($Issues, [string]$SignatureId, [int]$Self) {
    $marker = "<!-- crash-signature: $SignatureId -->"
    $same = @($Issues | Where-Object { $_.Number -ne $Self -and $_.Body -and $_.Body.Contains($marker) } | Sort-Object Number)
    if ($same.Count) { return [int]$same[0].Number }
    $null
}
