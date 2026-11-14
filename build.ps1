# Builds Equid: tests, the engine as WASM, the web app and the AI trip server.
#
#   .\build.ps1             cargo test + wasm + web (web\dist) + server (release)
#   .\build.ps1 -NoTest     skip cargo test
# Data (data\princeton) is committed; regenerating it is in prep\README.md and prep-shade\README.md.
param([switch]$NoTest)
$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot

function Step($name, [scriptblock]$cmd) {
    Write-Host "== $name" -ForegroundColor Cyan
    & $cmd
    if ($LASTEXITCODE -ne 0) { throw "$name failed (exit $LASTEXITCODE)" }
}

if (-not $NoTest) { Step "cargo test" { cargo test --workspace } }
Push-Location web
try {
    if (-not (Test-Path node_modules)) { Step "npm install" { npm install } }
    Step "web build (wasm + vite -> web\dist)" { npm run build }
} finally { Pop-Location }
Step "server (release)" { cargo build --release -p server }
Write-Host "Done. Start it with .\run.ps1" -ForegroundColor Green
