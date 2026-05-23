#requires -Version 7.0
[CmdletBinding()]
param(
    [ValidateSet('all', 'msi', 'nsis', 'app')]
    [string]$Target = 'all',
    [switch]$Clean,
    [switch]$Open,
    [switch]$NoInstall
)

$ErrorActionPreference = 'Stop'
Set-Location -LiteralPath $PSScriptRoot

if ($Clean) {
    Write-Host '[build] cleaning previous artifacts...' -ForegroundColor Yellow
    if (Test-Path 'dist')                  { Remove-Item -Recurse -Force 'dist' }
    if (Test-Path 'src-tauri/target/release/bundle') {
        Remove-Item -Recurse -Force 'src-tauri/target/release/bundle'
    }
}

if (-not $NoInstall -and -not (Test-Path 'node_modules')) {
    Write-Host '[build] installing npm dependencies...' -ForegroundColor Cyan
    npm install
    if ($LASTEXITCODE -ne 0) { throw 'npm install failed' }
}

$tauriArgs = @('run', 'tauri', '--', 'build')
if ($Target -eq 'app') {
    $tauriArgs += @('--no-bundle')
} elseif ($Target -ne 'all') {
    $tauriArgs += @('--bundles', $Target)
}

Write-Host "[build] tauri build (target=$Target)" -ForegroundColor Cyan
& npm @tauriArgs
if ($LASTEXITCODE -ne 0) { throw "tauri build failed (exit $LASTEXITCODE)" }

$bundleDir = Join-Path $PSScriptRoot 'target/release/bundle'
if (Test-Path $bundleDir) {
    Write-Host ''
    Write-Host '[build] artifacts:' -ForegroundColor Green
    Get-ChildItem -Recurse -File -Path $bundleDir |
        Where-Object { $_.Extension -in '.msi', '.exe' -and $_.Name -notmatch 'uninstall' } |
        ForEach-Object {
            $size = [math]::Round($_.Length / 1MB, 2)
            Write-Host ("  {0}  ({1} MB)" -f $_.FullName, $size)
        }
    if ($Open) { Invoke-Item $bundleDir }
}
