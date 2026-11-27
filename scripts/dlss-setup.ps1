<#
.SYNOPSIS
Installs NVIDIA DLSS for the viewer's DLSS option on Windows.

.DESCRIPTION
For a viewer built with the `dlss` feature (docs/DLSS.md). NVIDIA's
files are not ours to ship: this fetches NVIDIA's Streamline release from
NVIDIA's own repository for you, shows you NVIDIA's DLSS licence, and
installs its files only once you accept it.

Run it with: powershell -ExecutionPolicy Bypass -File dlss-setup.ps1
  (no argument)  download NVIDIA Streamline and install
  -Zip <file>    install from the Streamline zip you downloaded
  -Remove        remove the installed files

The files go to $env:AURORA_DLSS_DIR, else %LOCALAPPDATA%\aurora-dlss: the
folder the viewer looks in at start. From the release's bin\x64, each
signed by NVIDIA:
  sl.interposer.dll, sl.common.dll, sl.dlss.dll   NVIDIA Streamline
  nvngx_dlss.dll                                  the DLSS model
  sl.dlss_d.dll, nvngx_dlssd.dll                  the same, for Ray Reconstruction
  NVIDIA-DLSS-LICENSE.txt     NVIDIA's DLSS licence you accepted
  STREAMLINE-LICENSE.txt      Streamline's own (MIT)

Close the viewer first: while it runs it holds the DLLs open.
#>
[CmdletBinding()]
param(
    [string]$Zip = '',
    [switch]$Remove,
    [switch]$Help
)

$ErrorActionPreference = 'Stop'
# NVIDIA's Streamline release, by version and the SHA-256 GitHub gives
# for it: another version is another review.
$Release = 'v2.14.16'
$ZipName = "streamline-sdk-$Release.zip"
$Url = "https://github.com/NVIDIA-RTX/Streamline/releases/download/$Release/$ZipName"
$Sha256 = '32041E51DB0CAB709A271B72A352281B690C2ABBD669A827809401A5DD23956B'

$Dlls = @('sl.interposer.dll', 'sl.common.dll', 'sl.dlss.dll', 'nvngx_dlss.dll', 'sl.dlss_d.dll', 'nvngx_dlssd.dll')
# zip entry -> installed name: the licences under their own names, so a
# LICENSE.txt of the user's in a custom AURORA_DLSS_DIR is left alone
$Licences = [ordered]@{
    'bin/x64/nvngx_dlss.license.txt' = 'NVIDIA-DLSS-LICENSE.txt'
    'license.txt'                    = 'STREAMLINE-LICENSE.txt'
}
# what an earlier version of this script installed in Streamline's place
$Old = @('nvsdk_ngx.dll')

function Fail([string]$Message) {
    Write-Host "dlss-setup: $Message" -ForegroundColor Red
    exit 1
}

# A DLL the running viewer holds open can be neither replaced nor deleted.
function Busy([string]$File, $Err) {
    Fail "cannot write $File (${Err}): close the viewer, which holds the DLSS files open while it runs, and run this again"
}

# A path made absolute as the cmdlets below read it: against PowerShell's
# own location, not the process's folder ([IO.Path]::GetFullPath's, which
# Set-Location does not move), so a relative AURORA_DLSS_DIR is checked
# where the files would go. Nothing need exist yet.
function Get-FullPath([string]$Path) {
    try { return [IO.Path]::GetFullPath($ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($Path)) }
    catch { Fail "not a path here: $Path ($($_.Exception.Message))" }
}

# AURORA_DLSS_DIR inside an unpacked Streamline release, at any depth:
# removing would delete the release's own DLLs. A junction or a symbolic
# link as the folder, or anywhere above it, is refused too: its name says
# nothing of where it leads (Get-FullPath keeps it). Other reparse points
# (a OneDrive folder) have no LinkType: plain folders here.
function Test-RefusedFolder([string]$Folder) {
    $full = (Get-FullPath $Folder).TrimEnd('\', '/')
    for ($up = $full; $up; $up = Split-Path -Parent $up) {
        $item = Get-Item -LiteralPath $up -Force -ErrorAction SilentlyContinue
        if ($item -and $item.LinkType -in @('Junction', 'SymbolicLink')) { return $true }
        if (Test-Path -LiteralPath (Join-Path $up 'include\sl.h')) { return $true }
    }
    return $false
}

if ($Help) {
    Get-Help $PSCommandPath -Detailed
    exit 0
}

$dest = $env:AURORA_DLSS_DIR
if (-not $dest) {
    if (-not $env:LOCALAPPDATA) { Fail 'LOCALAPPDATA is not set: set AURORA_DLSS_DIR to the folder to install in' }
    $dest = Join-Path $env:LOCALAPPDATA 'aurora-dlss'
}
# (resolved once: the checks and every file operation below use this path)
$dest = Get-FullPath $dest
# What a refused folder is called: the default one, under a junctioned or
# linked profile, was not chosen by the user, who sets AURORA_DLSS_DIR.
$destName = if ($env:AURORA_DLSS_DIR) { "AURORA_DLSS_DIR ($dest)" } else { "the default folder ($dest)" }
$destFix = if ($env:AURORA_DLSS_DIR) { 'choose another folder' } else { 'set AURORA_DLSS_DIR to another folder' }
$refused = "$destName is inside NVIDIA's Streamline release, or is or lies under a junction or a symbolic link"

function Remove-Installed([string[]]$Names) {
    foreach ($name in $Names) {
        $file = Join-Path $dest $name
        if (Test-Path -LiteralPath $file) {
            try { Remove-Item -LiteralPath $file -Force } catch { Busy $file $_ }
        }
    }
}

if ($Remove) {
    if (Test-RefusedFolder $dest) { Fail "${refused}: nothing removed, $destFix" }
    Remove-Installed ($Dlls + @($Licences.Values) + $Old)
    if ((Test-Path -LiteralPath $dest) -and -not (Get-ChildItem -LiteralPath $dest -Force)) {
        Remove-Item -LiteralPath $dest -Force
    }
    Write-Host "Removed NVIDIA DLSS from $dest. The viewer offers FSR alone from its next start."
    exit 0
}

if (-not [Environment]::Is64BitOperatingSystem) { Fail 'DLSS needs 64-bit Windows' }
if (Test-RefusedFolder $dest) { Fail "${refused}: nothing installed, $destFix" }
if ($Zip) {
    try { $Zip = (Resolve-Path -LiteralPath $Zip).ProviderPath } catch { Fail "no file $Zip" }
}

$work = Join-Path ([IO.Path]::GetTempPath()) ('aurora-dlss-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $work | Out-Null
try {
    if (-not $Zip) {
        Write-Host "Downloading NVIDIA Streamline $Release from $Url (about 280 MB)..."
        $Zip = Join-Path $work $ZipName
        # (without the progress bar, Windows PowerShell 5 downloads many times faster)
        $ProgressPreference = 'SilentlyContinue'
        try { Invoke-WebRequest -Uri $Url -OutFile $Zip -UseBasicParsing }
        catch { Fail "the download failed ($($_.Exception.Message)): download $Url yourself and pass -Zip <file>" }
    }
    $hash = (Get-FileHash -LiteralPath $Zip -Algorithm SHA256).Hash
    if ($hash -ne $Sha256) { Fail "$Zip is not NVIDIA's $ZipName (SHA-256 $hash, expected $Sha256)" }

    # Only the files above come out of the zip, by their exact entry names.
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $out = Join-Path $work 'files'
    New-Item -ItemType Directory -Path $out | Out-Null
    $archive = [IO.Compression.ZipFile]::OpenRead($Zip)
    try {
        $wanted = @($Dlls | ForEach-Object { "bin/x64/$_" }) + @($Licences.Keys)
        foreach ($name in $wanted) {
            $entry = $archive.GetEntry($name)
            if (-not $entry) { Fail "no $name in $Zip" }
            [IO.Compression.ZipFileExtensions]::ExtractToFile($entry, (Join-Path $out ($name -replace '^.*/', '')))
        }
    }
    finally { $archive.Dispose() }

    foreach ($dll in $Dlls) {
        $sig = Get-AuthenticodeSignature -LiteralPath (Join-Path $out $dll)
        if ($sig.Status -ne 'Valid' -or $sig.SignerCertificate.Subject -notmatch 'O="?NVIDIA Corporation') {
            Fail "$dll from the zip is not signed by NVIDIA ($($sig.Status)): nothing installed"
        }
    }

    $licence = Join-Path $out 'nvngx_dlss.license.txt'
    Write-Host ''
    Write-Host "NVIDIA DLSS is under NVIDIA's licence, not the viewer's. Read it:"
    Write-Host ''
    # Paging needs a console; a host without it (the ISE, a remote session)
    # throws, and gets the text whole.
    try { Get-Content -LiteralPath $licence | Out-Host -Paging }
    catch { Get-Content -LiteralPath $licence | Out-Host }
    Write-Host ''
    $answer = Read-Host "Do you accept NVIDIA's licence for DLSS? Type yes to install"
    if ($answer -cne 'yes') { Fail 'not accepted: nothing installed' }

    New-Item -ItemType Directory -Path $dest -Force | Out-Null
    $pairs = @($Dlls | ForEach-Object { , @($_, $_) }) +
        @($Licences.GetEnumerator() | ForEach-Object { , @(($_.Key -replace '^.*/', ''), $_.Value) })
    foreach ($pair in $pairs) {
        $target = Join-Path $dest $pair[1]
        try { Copy-Item -LiteralPath (Join-Path $out $pair[0]) -Destination $target -Force } catch { Busy $target $_ }
    }
    Remove-Installed $Old

    Write-Host ''
    Write-Host "Installed NVIDIA Streamline $Release with DLSS in $dest."
    Write-Host 'Restart the viewer, then choose NVIDIA DLSS under Preferences > Graphics > Anti-aliasing.'
    Write-Host 'NVIDIA graphics cards only.'
}
finally {
    Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue
}
