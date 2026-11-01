param([switch]$SkipBuild)
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
Push-Location $root
try {
    if (-not $SkipBuild) {
        Write-Host '[Release] Building the application'
        & npm.cmd run desktop:build -- --ci -- --locked
        if ($LASTEXITCODE) { throw 'Application build failed' }
    }
    Write-Host '[Release] Collecting dependency notices'
    & node.exe scripts/collect-licenses.mjs
    if ($LASTEXITCODE) { throw 'Dependency notice collection failed' }
    Write-Host '[Release] Preparing the bundled X server and matching sources'
    & (Join-Path $PSScriptRoot 'Get-X11Runtime.ps1') -Sources
    if (-not $?) { throw 'X server runtime/source preparation failed' }
    Write-Host '[Release] Building the installer'
    & (Join-Path $PSScriptRoot 'Build-Installer.ps1') -SkipBuild
    Write-Host '[Release] Building the portable ZIP'
    & (Join-Path $PSScriptRoot 'Build-Portable.ps1')
    $version = (Get-Content crates/yuuterm-desktop/tauri.conf.json -Raw | ConvertFrom-Json).version
    Write-Host '[Release] Packaging matching X11 sources'
    Compress-Archive -Path target/x11-sources/* -DestinationPath "target/distribution/YuuTerm-$version-x11-sources.zip" -Force
    $assets = Get-ChildItem target/distribution -File | Where-Object Name -ne 'SHA256SUMS.txt' | Sort-Object Name
    $lines = $assets | ForEach-Object { '{0}  {1}' -f (Get-FileHash $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant(), $_.Name }
    [IO.File]::WriteAllLines((Join-Path $root 'target/distribution/SHA256SUMS.txt'), $lines)
    Write-Output 'Unsigned release assets prepared in target/distribution. Publish all assets together.'
} finally { Pop-Location }
