param([switch]$SkipBuild)
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
Push-Location $root
try {
    if (-not $SkipBuild) {
        & npm.cmd run desktop:build -- --ci -- --locked
        if ($LASTEXITCODE) { throw 'Application build failed' }
    }
    if (-not (Test-Path target/third-party/inventory.json)) { throw 'Run scripts/Build-Release.ps1 to collect third-party notices first' }
    & npx.cmd tauri bundle --config crates/yuuterm-desktop/tauri.conf.json --config crates/yuuterm-desktop/tauri.installer.conf.json --bundles nsis --features custom-protocol --ci --no-binary-patching
    if ($LASTEXITCODE) { throw 'Installer bundle failed' }
    $version = (Get-Content crates/yuuterm-desktop/tauri.conf.json -Raw | ConvertFrom-Json).version
    New-Item -ItemType Directory -Force target/distribution | Out-Null
    Copy-Item "target/release/bundle/nsis/YuuTerm_${version}_x64-setup.exe" "target/distribution/YuuTerm-$version-x64-setup.exe" -Force
} finally { Pop-Location }
