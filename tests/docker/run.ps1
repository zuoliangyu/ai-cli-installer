#requires -Version 7.2
[CmdletBinding()]
param(
    [ValidateSet('All', 'Build', 'Integration', 'Browser')]
    [string]$Stage = 'All',
    [ValidateSet('claude-code-native', 'claude-code-npm', 'codex-cli-native', 'codex-cli-npm')]
    [string]$InstallCase
)

$ErrorActionPreference = 'Stop'
$projectRoot = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$runId = [DateTime]::UtcNow.ToString('yyyyMMdd-HHmmss') + '-' + [Guid]::NewGuid().ToString('N').Substring(0, 6)
$artifacts = Join-Path $projectRoot "target/docker-validation/$runId"
$null = New-Item -ItemType Directory -Force -Path $artifacts
$image = 'ai-cli-installer-validation:local'
$browserImage = 'ai-cli-installer-validation-browser:local'
$serverName = "ai-cli-validation-server-$runId"
$integrationName = "ai-cli-validation-integration-$runId"
$browserName = "ai-cli-validation-browser-$runId"
$ownedContainers = [System.Collections.Generic.List[string]]::new()
$failures = [System.Collections.Generic.List[string]]::new()

function Invoke-DockerLogged {
    param([string]$LogName, [string[]]$DockerArguments)
    & docker @DockerArguments 2>&1 | Tee-Object -FilePath (Join-Path $artifacts $LogName)
    if ($LASTEXITCODE -ne 0) { throw "Docker failed ($LASTEXITCODE): $LogName" }
}

function Get-SourceFingerprint {
    $inputs = @('Cargo.toml', 'Cargo.lock', 'crates', 'src-tauri', 'src', 'public',
        'package.json', 'package-lock.json', 'index.html', 'svelte.config.js',
        'tsconfig.json', 'tsconfig.node.json', 'vite.config.ts', 'tests/docker/Dockerfile')
    # Match Docker COPY's on-disk inputs, including generated/ignored files.
    $files = foreach ($inputPath in $inputs) {
        $item = Get-Item -LiteralPath (Join-Path $projectRoot $inputPath)
        if ($item.PSIsContainer) { Get-ChildItem -LiteralPath $item.FullName -Recurse -Force -File }
        else { $item }
    }
    $manifest = ($files | Sort-Object FullName -Unique | ForEach-Object {
        $relativePath = [IO.Path]::GetRelativePath($projectRoot, $_.FullName).Replace('\', '/')
        "$relativePath $((Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash)"
    }) -join "`n"
    $hash = [System.Security.Cryptography.SHA256]::HashData([Text.Encoding]::UTF8.GetBytes($manifest))
    return [Convert]::ToHexString($hash).ToLowerInvariant()
}

Push-Location $projectRoot
try {
    $os = & docker info --format '{{.OSType}}'
    if ($LASTEXITCODE -ne 0 -or $os -ne 'linux') { throw 'Docker must be running Linux containers.' }
    $commit = & git rev-parse HEAD
    if ($LASTEXITCODE -ne 0) { throw 'Cannot determine repository commit.' }
    $sourceHash = Get-SourceFingerprint

    if ($Stage -in @('All', 'Build')) {
        Invoke-DockerLogged 'build.log' @('build', '--progress=plain', '-f', 'tests/docker/Dockerfile', '-t', $image,
            '--label', "org.ai-cli-installer.validation.source-sha256=$sourceHash", '.')
    }
    $imageJson = & docker image inspect $image
    if ($LASTEXITCODE -ne 0) { throw 'Build the validation image first (-Stage Build).' }
    $imageInfo = ($imageJson | ConvertFrom-Json)[0]
    if ($imageInfo.Config.Labels.'org.ai-cli-installer.validation.source-sha256' -ne $sourceHash) {
        throw 'The image does not match current build inputs. Run -Stage Build first.'
    }
    $dirty = [bool](& git status --porcelain)
    @{ workspaceCommit = $commit; workspaceDirty = $dirty; sourceSha256 = $sourceHash;
        imageId = $imageInfo.Id; stage = $Stage; startedUtc = [DateTime]::UtcNow.ToString('o') } |
        ConvertTo-Json | Set-Content -LiteralPath (Join-Path $artifacts 'run.json') -Encoding utf8
    if ($Stage -in @('All', 'Integration')) {
        $ownedContainers.Add($integrationName)
        try {
            Invoke-DockerLogged 'integration.log' @(
                'run', '--name', $integrationName, '--init', '--cpus=4', '--memory=4g',
                '--mount', "type=bind,source=$PSScriptRoot,target=/tests,readonly",
                '--mount', "type=bind,source=$artifacts,target=/artifacts",
                '-e', 'ARTIFACT_DIR=/artifacts', '-e', "TEST_SOURCE_SHA256=$sourceHash",
                '-e', "TEST_GROUP=$(if ($InstallCase) { 'install' } else { 'all' })", '-e', "INSTALL_CASE=$InstallCase",
                $imageInfo.Id, 'node', '/tests/integration.mjs'
            )
        } catch { $failures.Add($_.Exception.Message) }
    }
    if ($Stage -in @('All', 'Browser')) {
        try {
            Invoke-DockerLogged 'browser-build.log' @('build', '--progress=plain', '-f', 'tests/docker/Dockerfile.browser', '-t', $browserImage, 'tests/docker')
            $testToken = [Guid]::NewGuid().ToString('N') + [Guid]::NewGuid().ToString('N')
            $ownedContainers.Add($serverName)
            & docker run -d --name $serverName --init --cpus=4 --memory=4g `
                -e "INSTALLER_TOKEN=$testToken" -e INSTALLER_HOST=127.0.0.1 `
                -e INSTALLER_PORT=3210 -e SHELL=/bin/bash `
                $imageInfo.Id /app/target/debug/installer-web | Out-Null
            if ($LASTEXITCODE -ne 0) { throw 'Cannot start browser test server.' }
            $ownedContainers.Add($browserName)
            Invoke-DockerLogged 'browser.log' @(
                'run', '--name', $browserName, '--init', '--cpus=2', '--memory=2g', '--shm-size=512m',
                '--network', "container:$serverName",
                '--mount', "type=bind,source=$PSScriptRoot,target=/tests,readonly",
                '--mount', "type=bind,source=$artifacts,target=/artifacts",
                '-e', 'BASE_URL=http://127.0.0.1:3210', '-e', "INSTALLER_TOKEN=$testToken",
                '-e', 'ARTIFACT_DIR=/artifacts', $browserImage
            )
        } catch { $failures.Add($_.Exception.Message) }
        finally {
            if ($ownedContainers.Contains($serverName)) {
                & docker logs $serverName 2>&1 | ForEach-Object { "$_".Replace($testToken, '[test-token]') } |
                    Set-Content -LiteralPath (Join-Path $artifacts 'browser-server.log') -Encoding utf8
            }
        }
    }
    Write-Host "Artifacts: $artifacts"
    if ($failures.Count) { throw ($failures -join [Environment]::NewLine) }
} finally {
    foreach ($container in $ownedContainers) { & docker rm -f $container 2>&1 | Out-Null }
    Pop-Location
}
