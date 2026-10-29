<#
.SYNOPSIS
    Puts the media plugins in a folder ready to zip: the Linden Lab binaries
    built from the Legacy Viewer sources and the CEF / Dullahan (and on
    Windows VLC) files of the prebuilt packages, all in one llplugin folder.
    The "Media plugins" workflow runs it after the build.

    Windows: SLPlugin.exe, media_plugin_cef.dll, media_plugin_libvlc.dll, laid
    out like the llplugin folder of the Legacy Viewer (viewer_manifest.py,
    WindowsManifest).
    Linux (PowerShell 7 on the runner): SLPlugin, libmedia_plugin_cef.so and
    libmedia_plugin_gstreamer.so, the files of viewer_manifest.py
    (LinuxManifest), and the shared libraries of the prebuilt packages they
    link; video goes through the system's GStreamer, as in the Legacy Viewer.
    macOS (PowerShell 7 on the runner): SLPlugin.app as the Legacy Viewer's
    application holds it (viewer_manifest.py, DarwinManifest), the plugins,
    CEF's framework, the Dullahan helpers and LibVLC in its
    Contents/Frameworks, signed ad hoc.

.EXAMPLE
    ./scripts/package-media-plugins.ps1 -Source fs -Build fs/build-vc170-64 -Out Aurora-Media-Plugins-1-windows-x64 -Version 1
.EXAMPLE
    ./scripts/package-media-plugins.ps1 -Platform linux -Source fs -Build fs/build-linux-x86_64 -Out Aurora-Media-Plugins-2-linux-x64 -Version 2
.EXAMPLE
    ./scripts/package-media-plugins.ps1 -Platform macos -Source fs -Build fs/build-darwin-universal -Out Aurora-Media-Plugins-3-macos-universal -Version 3
#>
param(
    # checkout of the Legacy Viewer sources, at the commit that was built
    [Parameter(Mandatory)][string]$Source,
    # its CMake build folder (holds packages\ and the built targets)
    [Parameter(Mandatory)][string]$Build,
    [Parameter(Mandatory)][string]$Out,
    [Parameter(Mandatory)][string]$Version,
    [string]$Configuration = 'Release',
    [ValidateSet('windows', 'linux', 'macos')][string]$Platform = 'windows'
)
$ErrorActionPreference = 'Stop'

$packages = Join-Path $Build 'packages'
$bin = Join-Path $packages 'bin/release'
$resources = Join-Path $packages 'resources'
$plugin = Join-Path $Out 'llplugin'
New-Item -ItemType Directory -Force -Path $plugin | Out-Null

function Copy-Required([string]$From, [string[]]$Names, [string]$To) {
    foreach ($name in $Names) {
        $path = Join-Path $From $name
        if (-not (Test-Path -Path $path)) { throw "Missing from the build: $path" }
        if ($Platform -eq 'linux') {
            # the packages hold versioned libraries behind symbolic links: the
            # pack gets the files themselves (the viewer refuses links in it)
            foreach ($item in Get-Item -Path $path) {
                & cp -RL $item.FullName $To
                if ($LASTEXITCODE -ne 0) { throw "cp failed for $($item.FullName)" }
            }
        } else {
            Copy-Item -LiteralPath $path -Destination $To -Recurse
        }
    }
}

if ($Platform -eq 'windows') {
    # the launcher sits with the plugins: it finds the C++ runtime next to it
    Copy-Required (Join-Path $Build "llplugin\slplugin\$Configuration") 'SLPlugin.exe' $plugin
    Copy-Required (Join-Path $Build "media_plugins\cef\$Configuration") 'media_plugin_cef.dll' $plugin
    Copy-Required (Join-Path $Build "media_plugins\libvlc\$Configuration") 'media_plugin_libvlc.dll' $plugin

    # CEF runtime and the Dullahan host process
    Copy-Required $bin @(
        'chrome_elf.dll', 'd3dcompiler_47.dll', 'dxcompiler.dll', 'dxil.dll', 'libcef.dll', 'libEGL.dll',
        'libGLESv2.dll', 'v8_context_snapshot.bin', 'vk_swiftshader.dll', 'vk_swiftshader_icd.json',
        'vulkan-1.dll', 'dullahan_host.exe'
    ) $plugin
    Copy-Required $resources @('chrome_100_percent.pak', 'chrome_200_percent.pak', 'resources.pak', 'icudtl.dat', 'locales') $plugin

    # LibVLC and its modules
    Copy-Required $bin @('libvlc.dll', 'libvlccore.dll', 'plugins') $plugin

    # C++ runtime of the compiler that built the three binaries
    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
    $vs = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
    if (-not $vs) { throw 'Visual Studio with the C++ tools was not found.' }
    $crt = Get-ChildItem -Path (Join-Path $vs 'VC\Redist\MSVC') -Recurse -Directory -Filter 'Microsoft.VC*.CRT' |
        Where-Object { $_.FullName -match '\\x64\\' -and $_.FullName -notmatch 'onecore|debug_nonredist' } |
        Sort-Object FullName -Descending | Select-Object -First 1
    if (-not $crt) { throw 'The Visual C++ runtime to redistribute was not found.' }
    Copy-Required $crt.FullName @('msvcp140.dll', 'vcruntime140.dll', 'vcruntime140_1.dll') $plugin
} elseif ($Platform -eq 'macos') {
    function Assert-Native([string]$What) { if ($LASTEXITCODE -ne 0) { throw "$What failed" } }
    $lib = Join-Path $packages 'lib/release'
    $app = Join-Path $plugin 'SLPlugin.app'
    $frameworks = Join-Path $app 'Contents/Frameworks'
    # the launcher's bundle as Xcode built it, without its debug symbols
    & ditto (Join-Path $Build "llplugin/slplugin/$Configuration/SLPlugin.app") $app; Assert-Native 'ditto SLPlugin.app'
    New-Item -ItemType Directory -Force -Path (Join-Path $frameworks 'plugins') | Out-Null
    Copy-Required (Join-Path $Build "media_plugins/cef/$Configuration") 'media_plugin_cef.dylib' $frameworks
    Copy-Required (Join-Path $Build "media_plugins/libvlc/$Configuration") 'media_plugin_libvlc.dylib' $frameworks

    # CEF: Dullahan loads the framework and starts its helpers from the
    # main bundle's Contents/Frameworks, where the helpers find it too
    foreach ($bundle in 'Chromium Embedded Framework.framework', 'DullahanHelper.app', 'DullahanHelper (Alerts).app',
        'DullahanHelper (GPU).app', 'DullahanHelper (Renderer).app', 'DullahanHelper (Plugin).app') {
        $from = Join-Path $lib $bundle
        if (-not (Test-Path -LiteralPath $from)) { throw "Missing from the build: $from" }
        & ditto $from (Join-Path $frameworks $bundle); Assert-Native "ditto $bundle"
    }
    # LibVLC under the names its plugin and modules load (@rpath/libvlc.dylib,
    # @rpath/libvlccore.dylib): the files themselves, the pack holds no link
    foreach ($name in 'libvlc.dylib', 'libvlccore.dylib') {
        & cp -L (Join-Path $lib $name) (Join-Path $frameworks $name); Assert-Native "cp $name"
    }
    # its modules with their times: plugins.dat describes them by time, a
    # newer file would make VLC scan them all at every start (the plugin
    # sets VLC_PLUGIN_PATH to ./plugins, SLPlugin running in Frameworks)
    $modules = @(Get-ChildItem -LiteralPath (Join-Path $lib 'plugins') -Filter '*.dylib').FullName + (Join-Path $lib 'plugins/plugins.dat')
    & cp -p @modules (Join-Path $frameworks 'plugins'); Assert-Native 'cp of the VLC modules'

    # what the viewer expects of the pack: no link, arm64 in every binary,
    # and nothing loaded from outside the bundle but the system's libraries.
    # A few VLC modules are Intel's own optimisations (MMX, SSE), built for
    # x86_64 only and shipped by the Legacy Viewer too: VLC loads none of
    # them on Apple Silicon, they are listed only.
    $links = @(& find $app -type l)
    if ($links.Count) { throw "Links in the pack: $($links -join ', ')" }
    $modules = (Resolve-Path -LiteralPath (Join-Path $frameworks 'plugins')).Path
    $intelOnly = @()
    foreach ($file in Get-ChildItem -LiteralPath $app -Recurse -File) {
        $archs = & lipo -archs $file.FullName 2>$null
        if ($LASTEXITCODE -ne 0) { $global:LASTEXITCODE = 0; continue }   # not a Mach-O file
        if ($archs -notmatch '\barm64\b') {
            if (-not $file.FullName.StartsWith($modules)) { throw "$($file.FullName) has no arm64 code ($archs)" }
            $intelOnly += $file.Name
        }
        # (a universal binary lists its dependencies once per architecture,
        # each list after a header line ending with a colon; a library lists
        # its own install name too, which loads nothing: the VLC modules keep
        # the .libs/ name of their build)
        $own = @(& otool -D $file.FullName | Where-Object { -not $_.TrimEnd().EndsWith(':') } | ForEach-Object { $_.Trim() })
        foreach ($line in (& otool -L $file.FullName | Where-Object { -not $_.TrimEnd().EndsWith(':') })) {
            $dep = ($line.Trim() -split ' \(')[0]
            if ($dep -and $own -notcontains $dep -and $dep -notmatch '^(@rpath|@loader_path|@executable_path)/|^/usr/lib/|^/System/') {
                throw "$($file.Name) loads $dep, outside the bundle"
            }
        }
    }

    if ($intelOnly.Count) { Write-Host "VLC modules for Intel Macs only: $($intelOnly -join ', ')" }

    # an ad hoc signature, as the Legacy Viewer signs without its
    # certificate (viewer_manifest.py, package_finish): each library, then
    # the bundle with the hardened runtime and SLPlugin's entitlements
    # (JIT for CEF, libraries of other signers)
    $libraries = @(Get-ChildItem -LiteralPath $frameworks -Filter '*.dylib').FullName +
        @(Get-ChildItem -LiteralPath (Join-Path $frameworks 'plugins') -Filter '*.dylib').FullName +
        @(Get-ChildItem -LiteralPath (Join-Path $frameworks 'Chromium Embedded Framework.framework/Libraries') -Filter '*.dylib' -ErrorAction SilentlyContinue).FullName
    foreach ($library in $libraries) { & codesign --force --sign - $library; Assert-Native "codesign $library" }
    & codesign --force --deep --options runtime --entitlements (Join-Path $Source 'indra/newview/slplugin.entitlements') --sign - $app
    Assert-Native 'codesign SLPlugin.app'
    & codesign --verify --deep --strict $app; Assert-Native 'codesign --verify'
} else {
    $lib = Join-Path $packages 'lib/release'
    # the launcher and the two plugins; the gstreamer10 target is installed
    # under the name the MIME table uses (media_plugin_gstreamer)
    Copy-Required (Join-Path $Build 'llplugin/slplugin') 'SLPlugin' $plugin
    Copy-Required (Join-Path $Build 'media_plugins/cef') 'libmedia_plugin_cef.so' $plugin
    Copy-Required (Join-Path $Build 'media_plugins/gstreamer10') 'libmedia_plugin_gstreamer10.so' $plugin
    Move-Item -LiteralPath (Join-Path $plugin 'libmedia_plugin_gstreamer10.so') -Destination (Join-Path $plugin 'libmedia_plugin_gstreamer.so')

    # CEF runtime and the Dullahan host, all next to SLPlugin: Dullahan looks
    # for dullahan_host and chrome-sandbox next to the running program, CEF
    # for its resources next to libcef.so. Without a chrome-sandbox owned by
    # root with the setuid bit, CEF runs without its sandbox, as in the
    # Legacy Viewer.
    Copy-Required $lib @('libcef.so', 'libEGL*', 'libvulkan*', 'libvk_swiftshader*', 'libGLESv2*', 'v8_context_snapshot.bin', 'vk_swiftshader_icd.json') $plugin
    Copy-Required $bin @('chrome-sandbox', 'dullahan_host') $plugin
    Copy-Required $resources @('chrome_100_percent.pak', 'chrome_200_percent.pak', 'resources.pak', 'icudtl.dat', 'locales') $plugin

    # the shared libraries of the packages these binaries link, if any (the
    # pinned commit links them statically): the viewer starts SLPlugin with
    # LD_LIBRARY_PATH on this folder, as the Legacy Viewer's wrapper.sh does
    # with its lib folder, which dullahan_host needs for libcef.so
    $libFull = (Resolve-Path -LiteralPath $lib).Path
    $saved = $env:LD_LIBRARY_PATH
    $env:LD_LIBRARY_PATH = "$((Resolve-Path -LiteralPath $plugin).Path):$libFull"
    $missing = @{}
    do {
        $added = 0
        foreach ($file in Get-ChildItem -LiteralPath $plugin -File) {
            # libcef.so weighs over a gigabyte: read the magic number only
            $head = [byte[]]::new(4)
            $stream = [IO.File]::OpenRead($file.FullName)
            try { $read = $stream.Read($head, 0, 4) } finally { $stream.Dispose() }
            if ($read -ne 4 -or ($head -join ',') -ne '127,69,76,70') { continue }   # not an ELF file
            foreach ($line in (& ldd $file.FullName 2>$null)) {
                if ($line -match '^\s*(\S+) => not found') { $missing[$Matches[1]] = $file.Name; continue }
                if ($line -notmatch '^\s*(\S+) => (\S+)') { continue }
                $name = $Matches[1]; $path = $Matches[2]
                if (-not $path.StartsWith($libFull) -or (Test-Path -LiteralPath (Join-Path $plugin $name))) { continue }
                & cp -L $path (Join-Path $plugin $name)
                if ($LASTEXITCODE -ne 0) { throw "cp failed for $path" }
                Write-Host "  linked library: $name"
                $added++
            }
        }
    } while ($added)
    $env:LD_LIBRARY_PATH = $saved
    foreach ($name in $missing.Keys) {
        if (-not (Test-Path -LiteralPath (Join-Path $plugin $name))) {
            Write-Warning "$name (needed by $($missing[$name])) is neither in the packages nor on this runner: it must come from the user's system"
        }
    }
    & chmod 755 (Join-Path $plugin 'SLPlugin') (Join-Path $plugin 'dullahan_host') (Join-Path $plugin 'chrome-sandbox')
    if ($LASTEXITCODE -ne 0) { throw 'chmod failed' }
}

# licence texts of the packages, next to the binaries they cover
$licenses = Join-Path $Out 'LICENSES'
New-Item -ItemType Directory -Force -Path $licenses | Out-Null
Copy-Item -Path (Join-Path $packages 'LICENSES/*') -Destination $licenses -Recurse
Copy-Item -LiteralPath (Join-Path $Source 'LICENSE') -Destination (Join-Path $licenses 'firestorm-viewer-LGPL-2.1.txt')

# what was built, for the viewer (version check) and for the licence notice
function Get-PackageVersion([string]$Name) {
    $xml = [IO.File]::ReadAllText((Join-Path (Resolve-Path -LiteralPath $Source).Path 'autobuild.xml'))
    $match = [regex]::Match($xml, "<key>version</key>\s*<string>([^<]+)</string>\s*<key>name</key>\s*<string>$([regex]::Escape($Name))</string>")
    if (-not $match.Success) { throw "No version of $Name in autobuild.xml." }
    $match.Groups[1].Value
}
$built = [ordered]@{ dullahan = Get-PackageVersion 'dullahan' }
if ($Platform -ne 'linux') { $built['vlc-bin'] = Get-PackageVersion 'vlc-bin' }
$manifest = [ordered]@{
    schema   = 1
    version  = $Version
    source   = [ordered]@{
        repository = (git -C $Source remote get-url origin).Trim()
        commit     = (git -C $Source rev-parse HEAD).Trim()
    }
    packages = $built
}
$manifestPath = Join-Path (Resolve-Path -LiteralPath $Out).Path 'aurora-media-plugins.json'
# Windows PowerShell 5.1 does not support Set-Content -Encoding utf8NoBOM.
[IO.File]::WriteAllText($manifestPath, ($manifest | ConvertTo-Json), [Text.UTF8Encoding]::new($false))

$files = Get-ChildItem -LiteralPath $Out -Recurse -File
Write-Host ("{0}: {1} files, {2:N1} MB" -f $Out, $files.Count, (($files | Measure-Object Length -Sum).Sum / 1MB))
