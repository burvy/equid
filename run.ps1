# Starts Equid locally: the AI trip server (port 8787) in the background, then the web dev server.
#
#   .\run.ps1               Princeton data, web at http://localhost:5173 (opens the browser)
#   .\run.ps1 -NoAgent      web only (the AI panel won't answer)
#   .\run.ps1 -Data fixtures   the server on the tiny test graph instead of Princeton
# Keys are read by the server itself from Z:\App Data\Downloads\claude.env / vexcel.env
# (or ANTHROPIC_API_KEY / VEXCEL_API_KEY). No Claude key -> offline mock model.
param([switch]$NoAgent, [string]$Data = "princeton")
$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot

$server = $null
if (-not $NoAgent) {
    $env:EQUID_DATA = "data/$Data"
    $server = Start-Process cargo -ArgumentList "run", "--release", "-p", "server" -NoNewWindow -PassThru
    Write-Host "AI trip server starting on http://127.0.0.1:8787 (data/$Data)" -ForegroundColor Cyan
}
try {
    Set-Location web
    if (-not (Test-Path node_modules)) { npm install }
    Start-Job { Start-Sleep 8; Start-Process "http://localhost:5173" } | Out-Null
    npm run dev
} finally {
    if ($server -and -not $server.HasExited) { Stop-Process -Id $server.Id -Force }
}
