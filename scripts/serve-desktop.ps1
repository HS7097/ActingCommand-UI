param(
    [int]$Port = 5177
)

$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent $PSScriptRoot
$UiDir = Join-Path $Root "apps\desktop"

if (-not (Test-Path $UiDir)) {
    throw "Desktop UI directory not found: $UiDir"
}

$UiRoot = [System.IO.Path]::GetFullPath($UiDir)
$Listener = [System.Net.HttpListener]::new()
$Prefix = "http://127.0.0.1:$Port/"
$Listener.Prefixes.Add($Prefix)
$Listener.Start()

Write-Host "GachaPilot desktop UI: $Prefix"
Write-Host "Serving: $UiRoot"

$Mime = @{
    ".html" = "text/html; charset=utf-8"
    ".css" = "text/css; charset=utf-8"
    ".js" = "application/javascript; charset=utf-8"
    ".json" = "application/json; charset=utf-8"
    ".png" = "image/png"
    ".jpg" = "image/jpeg"
    ".jpeg" = "image/jpeg"
    ".webp" = "image/webp"
    ".svg" = "image/svg+xml"
    ".ico" = "image/x-icon"
}

try {
    while ($Listener.IsListening) {
        $Context = $Listener.GetContext()
        $RequestPath = [Uri]::UnescapeDataString($Context.Request.Url.AbsolutePath.TrimStart("/"))
        if ([string]::IsNullOrWhiteSpace($RequestPath)) {
            $RequestPath = "index.html"
        }

        $LocalPath = [System.IO.Path]::GetFullPath((Join-Path $UiRoot $RequestPath))
        if (-not $LocalPath.StartsWith($UiRoot, [System.StringComparison]::OrdinalIgnoreCase)) {
            $Context.Response.StatusCode = 403
            $Context.Response.Close()
            continue
        }

        if (-not (Test-Path $LocalPath -PathType Leaf)) {
            $Context.Response.StatusCode = 404
            $Bytes = [System.Text.Encoding]::UTF8.GetBytes("Not found")
            $Context.Response.OutputStream.Write($Bytes, 0, $Bytes.Length)
            $Context.Response.Close()
            continue
        }

        $Extension = [System.IO.Path]::GetExtension($LocalPath).ToLowerInvariant()
        $Context.Response.ContentType = if ($Mime.ContainsKey($Extension)) { $Mime[$Extension] } else { "application/octet-stream" }
        $Context.Response.Headers.Set("Cache-Control", "no-store")
        $Bytes = [System.IO.File]::ReadAllBytes($LocalPath)
        $Context.Response.ContentLength64 = $Bytes.Length
        $Context.Response.OutputStream.Write($Bytes, 0, $Bytes.Length)
        $Context.Response.Close()
    }
}
finally {
    if ($Listener.IsListening) {
        $Listener.Stop()
    }
    $Listener.Close()
}
