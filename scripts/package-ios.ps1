<#
.SYNOPSIS
    Builds "Aurora Viewer.app" for iPadOS and iOS from the built viewer, and the
    .ipa that holds it: the executable, Info.plist, the icons, the
    translated texts of Info.plist and the files of package.ps1 (licences,
    notices). Runs on a Mac (PowerShell 7, the Xcode command line tools).

    Without -Identity the app is signed ad hoc: an .ipa to sign again with
    one's own Apple ID before it goes on a device (docs/IPAD.md). With
    -Identity and -Provision it is signed for the devices of that profile,
    under the profile's own application identifier.
.EXAMPLE
    $env:IPHONEOS_DEPLOYMENT_TARGET = '26.0'
    cargo build -p aurora-viewer --release --target aarch64-apple-ios
    ./scripts/package-ios.ps1 -Exe target/aarch64-apple-ios/release/aurora-viewer -Out dist
.EXAMPLE
    ./scripts/package-ios.ps1 -Exe target/aarch64-apple-ios/release/aurora-viewer -Out dist `
        -Identity "Apple Development: Jane Doe (ABCDE12345)" -Provision ~/Downloads/Aurora.mobileprovision
#>
param(
    [Parameter(Mandatory)][string]$Exe,
    # folder that receives "Aurora Viewer.app" and the .ipa
    [Parameter(Mandatory)][string]$Out,
    # a signing identity of the keychain (security find-identity -v -p codesigning)
    [string]$Identity = '',
    # the provisioning profile that goes with it (.mobileprovision)
    [string]$Provision = '',
    # the app's identifier, when not Info.plist's: a personal team signs
    # under an identifier of its own. With -Provision, the profile's is taken.
    [string]$BundleId = '',
    # repository whose files go in the bundle (default: this script's)
    [string]$Source = (Split-Path $PSScriptRoot -Parent)
)
$ErrorActionPreference = 'Stop'
if (-not $IsMacOS) { throw 'package-ios.ps1 runs on a Mac: it needs sips, codesign and ditto.' }
if ($Identity -and -not $Provision) { throw '-Identity needs -Provision: a device only runs an app whose profile names it.' }
if ($Provision -and -not $Identity) { throw '-Provision needs -Identity.' }
if ($BundleId -and $BundleId -notmatch '^[A-Za-z0-9][A-Za-z0-9.-]*$') { throw "Invalid -BundleId: $BundleId" }
# (PowerShell hands "~" to the tools as it is: the paths are made whole here)
if (-not (Test-Path -LiteralPath $Exe)) { throw "No executable at $Exe." }
$Exe = (Resolve-Path -LiteralPath $Exe).Path
if ($Provision) {
    if (-not (Test-Path $Provision)) { throw "No provisioning profile at $Provision." }
    $Provision = (Resolve-Path $Provision).Path
}

function Invoke-Tool([string]$Tool, [string[]]$Arguments) {
    & $Tool @Arguments
    if ($LASTEXITCODE -ne 0) { throw "$Tool failed ($LASTEXITCODE)." }
}

# An iPadOS executable does not run here, so it cannot be asked what it is:
# its load commands say the system it was built for, and Git says its
# version, by the rule of aurora-version's build script.
$plist = Get-Content -LiteralPath (Join-Path $Source 'packaging/ios/Info.plist') -Raw
if ($plist -notmatch '<key>MinimumOSVersion</key>\s*<string>([\d.]+)</string>') { throw 'packaging/ios/Info.plist has no MinimumOSVersion.' }
$minimum = [version]$Matches[1]
$built = (& vtool -show-build $Exe | Out-String)
if ($LASTEXITCODE -ne 0) { throw "vtool could not read $Exe." }
if ($built -notmatch 'platform\s+IOS\b') { throw "$Exe is not an iPadOS executable (cargo build --target aarch64-apple-ios)." }
if ($built -notmatch 'minos\s+([\d.]+)') { throw "$Exe does not say its minimum system." }
if ([version]$Matches[1] -gt $minimum) {
    throw "$Exe asks for iPadOS $($Matches[1]), Info.plist for $minimum`: build with IPHONEOS_DEPLOYMENT_TARGET=$minimum."
}
if ($env:AURORA_VERSION_TAG) {
    $version = $env:AURORA_VERSION_TAG -replace '^v', ''
} else {
    $described = (& git -C $Source describe --tags --match 'v[0-9]*.[0-9]*.[0-9]*' --long --abbrev=9 | Out-String).Trim()
    if ($LASTEXITCODE -ne 0 -or $described -notmatch '^v(.+)-(\d+)-g([0-9a-f]+)$') { throw "git describe gives no version: $described" }
    $version = if ($Matches[2] -eq '0') { $Matches[1] } else { "$($Matches[1])+$($Matches[2]).g$($Matches[3])" }
}

New-Item -ItemType Directory -Force -Path $Out | Out-Null
$app = Join-Path $Out 'Aurora Viewer.app'
if (Test-Path -LiteralPath $app) { Remove-Item -LiteralPath $app -Recurse -Force }
# (the file's name carries no "+": some sideloading tools trip on it)
$kind = if ($Identity) { 'signed' } else { 'unsigned' }
$ipa = Join-Path $Out "Aurora-Viewer-$($version -replace '\+', '-')-ios-$kind.ipa"

$work = Join-Path ([IO.Path]::GetTempPath()) "aurora-ios-$PID"
New-Item -ItemType Directory -Force -Path (Join-Path $work 'Payload') | Out-Null
$done = $false
try {
    # the files every package carries; an iOS bundle is flat, the executable
    # at its root under the name Info.plist gives
    & (Join-Path $PSScriptRoot 'package.ps1') -Exe $Exe -Out $app -Source $Source -Version $version
    $packed = Join-Path $app (Split-Path $Exe -Leaf)
    if ((Split-Path $Exe -Leaf) -ne 'aurora-viewer') { Move-Item -LiteralPath $packed -Destination (Join-Path $app 'aurora-viewer') }

    $entitlements = Join-Path $work 'entitlements.plist'
    if ($Identity) {
        # the profile inside the app, its entitlements for the signature and
        # its application identifier (TEAM.identifier) for Info.plist
        Copy-Item -LiteralPath $Provision -Destination (Join-Path $app 'embedded.mobileprovision')
        $decoded = Join-Path $work 'profile.plist'
        Invoke-Tool 'security' @('cms', '-D', '-i', $Provision, '-o', $decoded)
        Invoke-Tool 'plutil' @('-extract', 'Entitlements', 'xml1', '-o', $entitlements, $decoded)
        $granted = (& plutil -extract 'application-identifier' raw -o - $entitlements | Out-String).Trim()
        if ($LASTEXITCODE -ne 0 -or $granted -notmatch '^([A-Z0-9]+)\.(.+)$') { throw 'The profile names no application identifier.' }
        $team = $Matches[1]
        if ($Matches[2] -match '\*') {
            # a wildcard profile (TEAM.*) leaves the identifier to -BundleId
            # or Info.plist; the signature must name the app's own, whole
            $signedAs = if ($BundleId) { $BundleId } else { 'org.aurora-viewer.AuroraViewer' }
            Invoke-Tool 'plutil' @('-replace', 'application-identifier', '-string', "$team.$signedAs", $entitlements)
        } elseif (-not $BundleId) {
            $BundleId = $Matches[2]
        } elseif ($BundleId -ne $Matches[2]) {
            throw "The profile is for $($Matches[2]), not for $BundleId."
        }
    }

    # MAJOR.MINOR.PATCH of the build, the only form CFBundleShortVersionString takes
    if ($version -notmatch '^(\d+\.\d+\.\d+)') { throw "Invalid package version: $version" }
    $plist = $plist.Replace('@VERSION@', $Matches[1])
    if ($BundleId) {
        $shipped = '<string>org.aurora-viewer.AuroraViewer</string>'
        if (-not $plist.Contains($shipped)) { throw 'packaging/ios/Info.plist no longer has the identifier this script replaces.' }
        $plist = $plist.Replace($shipped, "<string>$BundleId</string>")
    }
    Set-Content -LiteralPath (Join-Path $app 'Info.plist') -Value $plist -Encoding utf8NoBOM -NoNewline
    Set-Content -LiteralPath (Join-Path $app 'PkgInfo') -Value 'APPL????' -Encoding ascii -NoNewline

    # the icons Info.plist names, from the logo on its background (no
    # transparency: the system draws the rounded corners itself)
    $logo = Join-Path $Source 'assets/branding/aurora-logo-bg.png'
    foreach ($icon in @(
            @{ Name = 'AppIcon60x60@2x.png'; Size = 120 },
            @{ Name = 'AppIcon60x60@3x.png'; Size = 180 },
            @{ Name = 'AppIcon76x76@2x~ipad.png'; Size = 152 },
            @{ Name = 'AppIcon83.5x83.5@2x~ipad.png'; Size = 167 }
        )) {
        Invoke-Tool 'sips' @('-z', "$($icon.Size)", "$($icon.Size)", $logo, '--out', (Join-Path $app $icon.Name)) | Out-Null
    }
    Get-ChildItem -LiteralPath (Join-Path $Source 'packaging/ios') -Directory -Filter '*.lproj' | ForEach-Object {
        Copy-Item -LiteralPath $_.FullName -Destination $app -Recurse
    }

    if ($Identity) {
        Invoke-Tool 'codesign' @('--force', '--sign', $Identity, '--entitlements', $entitlements, '--timestamp=none', $app)
    } else {
        Invoke-Tool 'codesign' @('--force', '--sign', '-', $app)
    }

    # an .ipa is a zip whose root is Payload/
    $payload = Join-Path $work 'Payload'
    Invoke-Tool 'ditto' @($app, (Join-Path $payload 'Aurora Viewer.app'))
    if (Test-Path -LiteralPath $ipa) { Remove-Item -LiteralPath $ipa -Force }
    Invoke-Tool 'ditto' @('-c', '-k', '--sequesterRsrc', '--keepParent', $payload, $ipa)
    $done = $true
} finally {
    Remove-Item -LiteralPath $work -Recurse -Force
    # no half-made app is left behind to be installed by mistake
    if (-not $done -and (Test-Path -LiteralPath $app)) { Remove-Item -LiteralPath $app -Recurse -Force }
}
"${app}: Aurora Viewer $version ($kind)"
"$ipa"
