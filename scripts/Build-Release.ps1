param([switch]$SkipBuild)
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
Push-Location $root
try {
    if (-not $SkipBuild) {
        & npm.cmd run desktop:build -- --ci -- --locked
        if ($LASTEXITCODE) { throw 'Application build failed' }
    }
    & node.exe scripts/collect-licenses.mjs
    if ($LASTEXITCODE) { throw 'Dependency notice collection failed' }
    & (Join-Path $PSScriptRoot 'Get-X11Runtime.ps1') -Sources
    if (-not $?) { throw 'X server runtime/source preparation failed' }
    & (Join-Path $PSScriptRoot 'Build-Installer.ps1') -SkipBuild
    & (Join-Path $PSScriptRoot 'Build-Portable.ps1')
    $version = (Get-Content crates/yuuterm-desktop/tauri.conf.json -Raw | ConvertFrom-Json).version
    Compress-Archive -Path target/x11-sources/* -DestinationPath "target/distribution/YuuTerm-$version-x11-sources.zip" -Force
    $assets = Get-ChildItem target/distribution -File | Where-Object Name -ne 'SHA256SUMS.txt' | Sort-Object Name
    $lines = $assets | ForEach-Object { '{0}  {1}' -f (Get-FileHash $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant(), $_.Name }
    [IO.File]::WriteAllLines((Join-Path $root 'target/distribution/SHA256SUMS.txt'), $lines)
    Write-Output 'Unsigned release assets prepared in target/distribution. Publish all assets together.'
} finally { Pop-Location }
