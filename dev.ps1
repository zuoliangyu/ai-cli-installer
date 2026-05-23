#requires -Version 7.0
[CmdletBinding()]
param(
    [switch]$Web,
    [switch]$NoInstall
)

$ErrorActionPreference = 'Stop'
Set-Location -LiteralPath $PSScriptRoot

if (-not $NoInstall -and -not (Test-Path 'node_modules')) {
    Write-Host '[dev] installing npm dependencies...' -ForegroundColor Cyan
    npm install
    if ($LASTEXITCODE -ne 0) { throw 'npm install failed' }
}

if ($Web) {
    Write-Host '[dev] starting installer-web (browser mode)' -ForegroundColor Cyan
    npm run web
} else {
    Write-Host '[dev] starting tauri dev (desktop mode)' -ForegroundColor Cyan
    npm run tauri -- dev
}

if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
