param([string]$ApplicationPath)
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
Push-Location $root
try {
    $source = if ($ApplicationPath) { (Resolve-Path -LiteralPath $ApplicationPath).Path } else { Join-Path $root 'target/release/yuuterm-desktop.exe' }
    $version = (Get-Content crates/yuuterm-desktop/tauri.conf.json -Raw | ConvertFrom-Json).version
    $package = Join-Path $root "target/portable-staging/YuuTerm-$version"
    if (Test-Path $package) { throw 'Portable staging exists; preserve or move it before rebuilding' }
    New-Item -ItemType Directory -Force $package | Out-Null
    Copy-Item $source (Join-Path $package 'yuuterm-desktop.exe')
    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
    $vs = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
    $crt = Get-ChildItem (Join-Path $vs 'VC/Redist/MSVC') -Directory |
        Where-Object Name -Match '^\d+\.\d+\.\d+$' | Sort-Object { [version]$_.Name } -Descending |
        ForEach-Object { Join-Path $_.FullName 'x64/Microsoft.VC143.CRT' } | Where-Object { Test-Path $_ } | Select-Object -First 1
    if (-not $crt) { throw 'Visual C++ x64 runtime redistributable missing' }
    Copy-Item (Join-Path $crt '*.dll') $package
    Copy-Item target/x11-runtime/x11 (Join-Path $package 'x11') -Recurse
    Copy-Item target/third-party (Join-Path $package 'third-party') -Recurse
    Copy-Item LICENSE,THIRD_PARTY_NOTICES.md $package
    Set-Content (Join-Path $package 'yuuterm.portable') 'Portable mode: local settings in data/. Passwords remain in Windows Credential Manager.'
    New-Item -ItemType Directory -Force target/distribution | Out-Null
    Compress-Archive -LiteralPath $package -DestinationPath "target/distribution/YuuTerm-$version-portable.zip" -Force
} finally { Pop-Location }
