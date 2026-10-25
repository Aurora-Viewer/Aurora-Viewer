# Crash triage fixtures (scripts/tools/crash-triage.ps1): a report as the
# viewer writes it and a small Breakpad symbol file. No GitHub call; the
# Crash triage workflow runs it on the pull requests that change the triage.
$ErrorActionPreference = 'Stop'
. "$PSScriptRoot/tools/crash-triage.ps1"
function Assert-Triage([bool]$Condition, [string]$Message) { if (-not $Condition) { throw $Message } }

$id = 'FEDCBA9876543210FEDCBA98765432101'
function New-Report([string]$Crash, [string[]]$Stack, [string]$Extra = '') {
    @"
$CrashMarker
Sent from the crash window of Aurora Viewer.

| Field | Value |
|---|---|
| Version | 0.8.0 |
| Commit | abcdef0123 |
| Crash | $Crash |
| System | Windows 11 Pro (26200, x86_64) |
| GPU | NVIDIA GeForce RTX 4070 (Vulkan, driver NVIDIA 581.57) |
$Extra
### Stack

``````text
$($Stack -join "`n")
``````

### Modules

``````text
aurora_viewer id=$id code=6700000Ac5e0000
kernelbase id=0123456789ABCDEF0123456789ABCDEF1 code=6712ABCD3f0000
``````
"@
}

# FUNC 1000: aurora_render::draw, with aurora_render::cull inlined at
# 1010-1020 (called from line 40); FUNC 2000: aurora_viewer::run;
# 3000: std::panicking frame; PUBLIC 5000: a function without FUNC record.
$sym = @'
MODULE windows x86_64 FEDCBA9876543210FEDCBA98765432101 aurora_viewer.pdb
INFO CODE_ID 6700000Ac5e0000 aurora-viewer.exe
FILE 0 D:\a\Aurora-Viewer\Aurora-Viewer\crates\aurora-render\src\draw.rs
FILE 1 D:\a\Aurora-Viewer\Aurora-Viewer\crates\aurora-render\src\cull.rs
FILE 2 /rustc/0123abcd/library\std\src\panicking.rs
FILE 3 C:\Users\runneradmin\.cargo\registry\src\index.crates.io-1949cf8c6b5b557f\wgpu-core-27.0.3\src\lib.rs
INLINE_ORIGIN 0 aurora_render::cull<aurora_render::Mesh>
FUNC 1000 40 0 aurora_render::draw(aurora_render::Frame*)
INLINE 0 40 0 0 1010 10
1000 10 30 0
1010 10 7 1
1020 20 44 0
FUNC m 2000 30 0 aurora_viewer::run::closure$3
2000 30 120 0
FUNC 3000 10 0 std::panicking::panic_with_hook
3000 10 823 2
FUNC 4000 10 0 wgpu_core::submit
4000 10 12 3
PUBLIC 5000 0 __scrt_common_main_seh
PUBLIC 4ff0 0 earlier_public
'@
$work = Join-Path ([IO.Path]::GetTempPath()) "aurora-triage-test-$([Guid]::NewGuid().ToString('N'))"
New-Item -ItemType Directory $work | Out-Null
try {
    $symFile = Join-Path $work 'aurora_viewer.sym'
    [IO.File]::WriteAllText($symFile, $sym)

    # Reading: validated values only, the marker is required.
    $native = New-Report 'Native exception 0xC0000005' @('00 aurora_viewer+0x1015', '01 aurora_viewer+0x2011', '02 ?', '03 kernelbase+0x2dda7', '04 aurora_viewer+0x5003')
    $r = Read-CrashReport $native
    Assert-Triage ($r.Version -eq '0.8.0' -and $r.Kind -eq 'Native exception 0xC0000005') 'Version and kind must be read.'
    Assert-Triage ($r.Frames.Count -eq 5 -and $r.Frames[0].Offset -eq 0x1015 -and $r.Frames[2].Text -eq '?') 'Every stack line must be read.'
    Assert-Triage ($r.Modules[$OwnModule].Id -eq $id) 'The own module id must be read.'
    Assert-Triage ($null -eq (Read-CrashReport ($native.Replace($CrashMarker, '')))) 'Without the marker, an issue is not a crash report.'
    $hostile = Read-CrashReport (New-Report 'Rust panic' @('00 aurora_viewer+0x1000', '01 evil`name+0x10', '02 aurora_viewer+0xZZ', '03 x+0x1 ; rm') '| System | <img src=x> |')
    Assert-Triage ($hostile.Frames.Count -eq 1 -and $hostile.System -eq 'Windows 11 Pro (26200, x86_64)') "Malformed lines and values must be dropped ($($hostile.Frames.Count) frames, system $($hostile.System))."

    Assert-Triage ((Get-ReleaseTag '0.8.0') -eq 'v0.8.0' -and (Get-ReleaseTag '0.8.0-beta.1') -eq 'v0.8.0-beta.1') 'Released versions must name their tag.'
    Assert-Triage ($null -eq (Get-ReleaseTag '0.7.3+13.gabcdef')) 'A development build has no release.'

    foreach ($case in @(
            @('D:\a\Aurora-Viewer\Aurora-Viewer\crates\aurora-net\src\udp.rs', 'crates/aurora-net/src/udp.rs'),
            @('C:\Users\runneradmin\.cargo\registry\src\index.crates.io-1949cf8c6b5b557f\wgpu-core-27.0.3\src\lib.rs', 'wgpu-core-27.0.3/src/lib.rs'),
            @('/rustc/0123abcd/library\core\src\option.rs', 'library/core/src/option.rs'),
            @('D:\a\_work\1\s\src\vctools\crt\vcstartup\src\startup\exe_common.inl', 'exe_common.inl'))) {
        Assert-Triage ((Format-SourcePath $case[0]) -eq $case[1]) "Path $($case[0]) must become $($case[1])."
    }

    # Resolution: the fault is exact, return addresses look up the call;
    # the inlined function comes first, at its own line, then its caller
    # at the call line.
    $frames = Get-SymbolizedFrames $r $symFile
    $text = @($frames | ForEach-Object { '{0:00} {1}{2} {3}' -f $_.Index, $_.Function, $(if ($_.Inlined) { ' (inlined)' } else { '' }), $_.Location })
    $expected = @(
        '00 aurora_render::cull<aurora_render::Mesh> (inlined) crates/aurora-render/src/cull.rs:7',
        '00 aurora_render::draw(aurora_render::Frame*) crates/aurora-render/src/draw.rs:40',
        '01 aurora_viewer::run::closure$3 crates/aurora-render/src/draw.rs:120',
        '02 ? ',
        '03 kernelbase+0x2dda7 ',
        '04 __scrt_common_main_seh ')
    Assert-Triage (($text -join "`n") -eq ($expected -join "`n")) "Unexpected frames:`n$($text -join "`n")"

    # Signature: the first meaningful frames by stable name, independent of
    # offsets, generics and closure numbers; the machinery is skipped.
    $sig = Get-CrashSignature $r $frames
    Assert-Triage ((@($sig.Frames) -join '|') -eq 'aurora_render::cull|aurora_render::draw|aurora_viewer::run::closure') "Unexpected signature frames: $(@($sig.Frames) -join '|')"
    $moved = Read-CrashReport (New-Report 'Native exception 0xC0000005' @('00 aurora_viewer+0x1016', '01 aurora_viewer+0x2021'))
    Assert-Triage ((Get-CrashSignature $moved (Get-SymbolizedFrames $moved $symFile)).Id -eq $sig.Id) 'Other offsets in the same functions must give the same signature.'
    $other = Read-CrashReport (New-Report 'Native exception 0xC0000094' @('00 aurora_viewer+0x1015', '01 aurora_viewer+0x2011'))
    Assert-Triage ((Get-CrashSignature $other (Get-SymbolizedFrames $other $symFile)).Id -ne $sig.Id) 'Another exception code is another crash.'
    $panic = Read-CrashReport (New-Report 'Rust panic' @('00 aurora_viewer+0x3005', '01 aurora_viewer+0x4005', '02 aurora_viewer+0x2005') "`nLocation: ``crates/aurora-viewer/src/app.rs:12:5```n")
    $panicSig = Get-CrashSignature $panic (Get-SymbolizedFrames $panic $symFile)
    Assert-Triage ((@($panicSig.Frames) -join '|') -eq 'wgpu_core::submit|aurora_viewer::run::closure') 'The panic machinery must be skipped.'
    Assert-Triage ($panic.Location -eq 'crates/aurora-viewer/src/app.rs:12:5') 'The panic location must be read.'
    Assert-Triage ((Get-TriageTitle $panic $panicSig) -eq 'Crash: Rust panic in wgpu_core::submit') 'Unexpected panic title.'
    Assert-Triage ((Get-TriageTitle $r $sig) -eq 'Crash: native exception 0xC0000005 in aurora_render::cull') 'Unexpected native title.'
    # A frozen window: the main thread's stack, its own kind of signature.
    $freeze = Read-CrashReport (New-Report 'Freeze' @('00 kernelbase+0x2dda7', '01 aurora_viewer+0x1015', '02 aurora_viewer+0x2011'))
    Assert-Triage ($freeze.Kind -eq 'Freeze') 'A freeze report must be read.'
    $freezeSig = Get-CrashSignature $freeze (Get-SymbolizedFrames $freeze $symFile)
    Assert-Triage ((@($freezeSig.Frames) -join '|') -eq 'aurora_render::cull|aurora_render::draw|aurora_viewer::run::closure') "Unexpected freeze signature frames: $(@($freezeSig.Frames) -join '|')"
    Assert-Triage ($freezeSig.Id -ne $sig.Id) 'A freeze is not the crash at the same place.'
    Assert-Triage ((Get-TriageTitle $freeze $freezeSig) -eq 'Crash: freeze in aurora_render::cull') 'Unexpected freeze title.'
    # The GPU kept a frame: the render thread's stack, another kind again.
    $stall = Read-CrashReport (New-Report 'GPU not responding' @('00 kernelbase+0x2dda7', '01 aurora_viewer+0x1015', '02 aurora_viewer+0x2011'))
    Assert-Triage ($stall.Kind -eq 'GPU not responding') 'A GPU stall report must be read.'
    $stallSig = Get-CrashSignature $stall (Get-SymbolizedFrames $stall $symFile)
    Assert-Triage ($stallSig.Id -ne $freezeSig.Id) 'A GPU stall is not a freeze at the same place.'
    Assert-Triage ((Get-TriageTitle $stall $stallSig) -eq 'Crash: GPU stall in aurora_render::cull') 'Unexpected GPU stall title.'
    # Out of memory: the allocation's own frames are skipped like the panic machinery.
    $oom = Read-CrashReport (New-Report 'Out of memory (70368744177664 bytes)' @('00 aurora_viewer+0x3005', '01 aurora_viewer+0x2011'))
    Assert-Triage ($oom.Kind -eq 'Out of memory') 'An out of memory report must be read.'
    $oomSig = Get-CrashSignature $oom (Get-SymbolizedFrames $oom $symFile)
    Assert-Triage ((@($oomSig.Frames) -join '|') -eq 'aurora_viewer::run::closure') "Unexpected out of memory signature frames: $(@($oomSig.Frames) -join '|')"
    Assert-Triage ((Get-TriageTitle $oom $oomSig) -eq 'Crash: out of memory in aurora_viewer::run::closure') 'Unexpected out of memory title.'
    Assert-Triage ($null -eq (Read-CrashReport (New-Report 'Out of memory (lots bytes)' @('00 aurora_viewer+0x3005')))) 'A malformed size is not a known kind.'

    # Without symbols: offsets of this exact build.
    $bare = Get-SymbolizedFrames $r $null
    $bareSig = Get-CrashSignature $r $bare
    Assert-Triage ((@($bareSig.Frames)[0]) -eq "unsymbolized $id" -and $bareSig.Id -ne $sig.Id) 'Without symbols the signature must name the build.'
    Assert-Triage ($null -eq (Get-TriageTitle $r $bareSig)) 'Without symbols the title stays as the viewer wrote it.'

    # The section: added once, replaced on a new run, the report kept.
    $section = Format-TriageSection $frames 'Aurora-Viewer-v0.8.0-windows-x64-symbols.zip' $sig
    $body = Merge-TriageSection $native $section
    Assert-Triage ($body.StartsWith($native.TrimEnd()) -and $body.Contains("<!-- crash-signature: $($sig.Id) -->")) 'The section must follow the report.'
    $again = Merge-TriageSection $body (Format-TriageSection $bare $null $bareSig)
    Assert-Triage (([regex]::Matches($again, [regex]::Escape($TriageStart))).Count -eq 1 -and -not $again.Contains($sig.Id)) 'A new run must replace the section.'
    Assert-Triage ((Read-CrashReport $body).Frames.Count -eq 5) 'The added section must not be read as the report.'

    # Duplicates: the lowest other open issue with the same signature.
    $issues = @(
        @{ Number = 30; Body = $body },
        @{ Number = 12; Body = $body },
        @{ Number = 40; Body = $body },
        @{ Number = 5; Body = $native },
        @{ Number = 6; Body = $null })
    Assert-Triage ((Find-DuplicateIssue $issues $sig.Id 40) -eq 12) 'The first open issue must be the reference.'
    Assert-Triage ($null -eq (Find-DuplicateIssue $issues $sig.Id 12 | Where-Object { $_ -lt 12 })) 'The reference itself has no earlier duplicate.'
    Assert-Triage ($null -eq (Find-DuplicateIssue $issues 'ffffffffffffffff' 40)) 'Another signature is a new crash.'
    Write-Host 'Crash triage tests passed.'
}
finally {
    Remove-Item -Recurse -Force $work
}
