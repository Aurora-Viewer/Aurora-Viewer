<# Package server for test-update.ps1; listens on loopback only. #>
param([Parameter(Mandatory)][string]$Root, [Parameter(Mandatory)][int]$Port)
$ErrorActionPreference = 'Stop'
$taskFeedRoot = (Resolve-Path -LiteralPath (Join-Path $Root 'feed')).Path
$listener = [System.Net.HttpListener]::new()
$listener.Prefixes.Add("http://127.0.0.1:$Port/")
$listener.Start()
try {
    Set-Content -LiteralPath (Join-Path $Root 'server-ready') -Value $Port
    while ($listener.IsListening) {
        $request = $listener.GetContext()
        $relative = [Uri]::UnescapeDataString($request.Request.Url.AbsolutePath.TrimStart('/'))
        $path = [IO.Path]::GetFullPath((Join-Path $taskFeedRoot $relative))
        if (!$path.StartsWith($taskFeedRoot + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase) -or !(Test-Path -LiteralPath $path -PathType Leaf)) {
            $request.Response.StatusCode = 404
            $request.Response.Close()
            continue
        }
        $input = [IO.File]::OpenRead($path)
        try {
            $request.Response.ContentLength64 = $input.Length
            $buffer = [byte[]]::new(256KB)
            while (($read = $input.Read($buffer, 0, $buffer.Length)) -gt 0) {
                $request.Response.OutputStream.Write($buffer, 0, $read)
                if ($relative.EndsWith('.zip')) { Start-Sleep -Milliseconds 35 }
            }
        } catch [System.Net.HttpListenerException] {
            # The user may cancel an in-flight download.
        } catch [System.IO.IOException] {
            # The cancelled client's socket may already be closed.
        } finally { $input.Dispose(); $request.Response.Close() }
    }
} finally { $listener.Close() }
