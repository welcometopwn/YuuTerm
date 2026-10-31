param([string]$ApplicationPath)
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$folder = Join-Path $root ('target/installer-test-' + (Get-Date -Format 'yyyyMMdd-HHmmss-fff'))
$stage = Join-Path $folder 'cargo'
$install = Join-Path $folder 'installed'
$product = 'YuuTerm Installer Fixture'
$key = "HKCU:/Software/Microsoft/Windows/CurrentVersion/Uninstall/$product"
$manufacturerKey = "HKCU:/Software/welcometopwn/$product"
$checks = [Collections.Generic.List[string]]::new()
$packageRuns = [Collections.Generic.List[object]]::new()
$app = $null
$savedTarget = $env:CARGO_TARGET_DIR
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class InstallerInstanceWindows {
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int command);
    [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr h);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
}
'@
function Check([bool]$condition, [string]$name) {
    if (-not $condition) { throw "Installer check failed: $name" }
    $checks.Add($name)
}
function Run-Package([string]$path, [string[]]$arguments) {
    $p = Start-Process $path -ArgumentList $arguments -WindowStyle Hidden -PassThru
    if (-not $p.WaitForExit(60000)) { throw 'Package exceeded 60-second deadline; process left untouched' }
    $packageRuns.Add(@{package=[IO.Path]::GetFileName($path);exit_code=$p.ExitCode})
    return $p.ExitCode
}
Push-Location $root
try {
    foreach ($existing in @($key, $manufacturerKey,
        "HKLM:/Software/Microsoft/Windows/CurrentVersion/Uninstall/$product",
        (Join-Path ([Environment]::GetFolderPath('Programs')) "$product.lnk"),
        (Join-Path ([Environment]::GetFolderPath('Desktop')) "$product.lnk"))) {
        if (Test-Path -LiteralPath $existing) { throw 'Fixture identity already exists; refusing to modify it' }
    }
    New-Item -ItemType Directory -Path "$stage/release" | Out-Null
    $source = if ($ApplicationPath) { (Resolve-Path -LiteralPath $ApplicationPath).Path } else { Join-Path $root 'target/release/yuuterm-desktop.exe' }
    $sourceHash = (Get-FileHash $source).Hash
    Copy-Item $source "$stage/release/yuuterm-desktop.exe"
    [IO.File]::WriteAllText("$folder/fixture.conf.json", (@{productName=$product;identifier='io.github.welcometopwn.yuuterm.installerfixture'} | ConvertTo-Json))
    $env:CARGO_TARGET_DIR = $stage
    & npx.cmd tauri bundle --config crates/yuuterm-desktop/tauri.conf.json --config crates/yuuterm-desktop/tauri.installer.conf.json --config "$folder/fixture.conf.json" --bundles nsis --features custom-protocol --ci --no-binary-patching
    if ($LASTEXITCODE -ne 0) { throw 'Fixture bundle failed' }
    $packages = @(Get-ChildItem "$stage/release/bundle/nsis" -Filter '*setup.exe')
    if ($packages.Count -ne 1) { throw 'Expected exactly one fixture installer' }
    $setup = $packages[0].FullName
    Check ((Run-Package $setup @('/S', '/NS', "/D=$install")) -eq 0) 'silent current-user installation succeeds'
    $installedApp = Join-Path $install 'yuuterm-desktop.exe'
    Check ((Get-FileHash $installedApp).Hash -eq $sourceHash) 'installed application equals supplied release bytes'
    Check ((Get-ChildItem $install -Filter '*.dll').Count -ge 10) 'application-local Visual C++ runtime is installed'
    $x11 = Join-Path $install 'x11'
    $x11Manifest = Get-Content (Join-Path $x11 'manifest.json') -Raw | ConvertFrom-Json
    $x11Files = @($x11Manifest.files.PSObject.Properties)
    Check ($x11Files.Count -gt 300 -and @($x11Files | Where-Object { (Get-FileHash (Join-Path $x11 $_.Name) -Algorithm SHA256).Hash -ne $_.Value }).Count -eq 0) 'bundled X server is installed and matches its manifest'
    foreach ($notice in @('LICENSE', 'THIRD_PARTY_NOTICES.md', 'third-party/inventory.json', 'third-party/fonts/OFL.txt')) {
        Check (Test-Path -LiteralPath (Join-Path $install $notice)) "installed package contains $notice"
    }
    $entry = Get-ItemProperty -LiteralPath $key
    Check ($entry.DisplayName -eq $product -and $entry.Publisher -eq 'welcometopwn') 'current-user uninstall metadata is correct'
    Check (-not (Test-Path "HKLM:/Software/Microsoft/Windows/CurrentVersion/Uninstall/$product")) 'no machine-wide uninstall entry'
    $uninstaller = Join-Path $install 'uninstall.exe'
    Check (Test-Path $uninstaller) 'uninstaller exists in fixture directory'
    # No real bookmarks, editor staging or WebView profile is used by this process.
    $local = Join-Path $folder 'profile-local'
    $roaming = Join-Path $folder 'profile-roaming'
    $webview = Join-Path $folder 'webview'
    foreach ($dir in @($local,$roaming,$webview)) { New-Item -ItemType Directory $dir | Out-Null }
    $savedLocal = $env:LOCALAPPDATA
    $savedRoaming = $env:APPDATA
    $savedWebview = $env:WEBVIEW2_USER_DATA_FOLDER
    try {
        $env:LOCALAPPDATA = $local
        $env:APPDATA = $roaming
        $env:WEBVIEW2_USER_DATA_FOLDER = $webview
        $app = Start-Process $installedApp -PassThru
    } finally {
        $env:LOCALAPPDATA = $savedLocal
        $env:APPDATA = $savedRoaming
        $env:WEBVIEW2_USER_DATA_FOLDER = $savedWebview
    }
    $deadline = [DateTime]::UtcNow.AddSeconds(30)
    do {
        Start-Sleep -Milliseconds 200
        $app.Refresh()
    } while (-not $app.HasExited -and $app.MainWindowHandle -eq 0 -and [DateTime]::UtcNow -lt $deadline)
    Check (-not $app.HasExited -and $app.MainWindowHandle -ne 0 -and $app.Responding) 'actual installed application opens a responsive native window'
    $deadline = [DateTime]::UtcNow.AddSeconds(15)
    while ((Get-ChildItem $webview -Force).Count -eq 0 -and -not $app.HasExited -and [DateTime]::UtcNow -lt $deadline) { Start-Sleep -Milliseconds 100 }
    Check ((Get-ChildItem $webview -Force).Count -gt 0) 'installed application uses isolated WebView profile'
    $pidBefore = $app.Id
    $null = [InstallerInstanceWindows]::ShowWindow($app.MainWindowHandle, 6)
    Start-Sleep -Milliseconds 200
    try {
        $env:LOCALAPPDATA = $local
        $env:APPDATA = $roaming
        $env:WEBVIEW2_USER_DATA_FOLDER = $webview
        $second = Start-Process $installedApp -PassThru
        Check ($second.WaitForExit(12000) -and $second.ExitCode -eq 0) 'second installed launch exits successfully'
    } finally {
        $env:LOCALAPPDATA = $savedLocal
        $env:APPDATA = $savedRoaming
        $env:WEBVIEW2_USER_DATA_FOLDER = $savedWebview
        if ($second -and -not $second.HasExited) { $null = $second.CloseMainWindow(); $null = $second.WaitForExit(15000) }
    }
    $deadline = [DateTime]::UtcNow.AddSeconds(5)
    while ([InstallerInstanceWindows]::GetForegroundWindow() -ne $app.MainWindowHandle -and [DateTime]::UtcNow -lt $deadline) { Start-Sleep -Milliseconds 50 }
    Check (-not [InstallerInstanceWindows]::IsIconic($app.MainWindowHandle) -and [InstallerInstanceWindows]::GetForegroundWindow() -eq $app.MainWindowHandle) 'second installed launch restores and focuses original window'
    Check ((Run-Package $setup @('/S', '/NS', "/D=$install")) -eq 10) 'installation refuses a running application with exit code 10'
    Check (-not $app.HasExited -and $app.Id -eq $pidBefore) 'installation leaves original application process alive'
    Check ((Run-Package $uninstaller @('/S', "_?=$install")) -eq 10) 'uninstallation refuses a running application with exit code 10'
    Check (-not $app.HasExited -and (Test-Path $key) -and (Get-FileHash $installedApp).Hash -eq $sourceHash) 'blocked maintenance preserves process, registration and binary'
    Check ($app.CloseMainWindow()) 'fixture application accepts normal window close'
    Check ($app.WaitForExit(15000)) 'fixture application exits normally'
    $sentinels = @("$install/user-owned.txt", "$local/recovery.txt", "$roaming/preferences.txt")
    foreach ($path in $sentinels) { [IO.File]::WriteAllText($path, 'fixture-preserved-data') }
    Check ((Run-Package $setup @('/S', '/NS', "/D=$install")) -eq 0) 'same-version reinstallation succeeds after normal close'
    foreach ($path in $sentinels) { Check ([IO.File]::ReadAllText($path) -eq 'fixture-preserved-data') 'reinstallation preserves fixture-owned extra data' }
    Check ((Run-Package $uninstaller @('/S', "_?=$install")) -eq 0) 'silent uninstallation succeeds after normal close'
    # The NSIS uninstaller executes a temporary copy; allow its cleanup to settle.
    $deadline = [DateTime]::UtcNow.AddSeconds(10)
    while ((Test-Path $installedApp) -and [DateTime]::UtcNow -lt $deadline) { Start-Sleep -Milliseconds 100 }
    Check (-not (Test-Path $installedApp) -and -not (Test-Path $key)) 'uninstallation removes installed application and registration'
    Check ((Get-ChildItem $install -Filter '*.dll').Count -eq 0) 'uninstallation removes packaged runtime libraries'
    Check (-not (Test-Path (Join-Path $install 'x11/bin/XWin.exe'))) 'uninstallation removes the bundled X server'
    foreach ($path in $sentinels) { Check ([IO.File]::ReadAllText($path) -eq 'fixture-preserved-data') 'uninstallation preserves fixture-owned extra data' }
    Check ((Get-FileHash $source).Hash -eq $sourceHash) 'supplied release remains unchanged'
    @{passed=$true;application_sha256=$sourceHash;checks=$checks;package_runs=$packageRuns;qualification='Same exact release binary; isolated installer identity and profiles. Same-version reinstall only. No authenticated remote sessions or missing-runtime VM test.'} |
        ConvertTo-Json -Depth 5 | Set-Content "$folder/report.json" -Encoding utf8
    [pscustomobject]@{Passed=$true;Checks=$checks.Count;Folder=$folder}
} catch {
    if (Test-Path $folder) { @{passed=$false;checks=$checks;package_runs=$packageRuns;error=$_.Exception.Message} | ConvertTo-Json -Depth 5 | Set-Content "$folder/report.json" -Encoding utf8 }
    throw
} finally {
    # Close only the process created by this script, never a user's running app.
    if ($app -and -not $app.HasExited) { $null = $app.CloseMainWindow(); $null = $app.WaitForExit(15000) }
    if ((-not $app -or $app.HasExited) -and (Test-Path "$install/uninstall.exe")) {
        $null = Run-Package "$install/uninstall.exe" @('/S', "_?=$install")
    }
    # NSIS remembers the installation folder after uninstall. Remove only our
    # exact fixture value, after checking ownership and the absence of children.
    if ((Test-Path $manufacturerKey) -and -not (Test-Path $key)) {
        $remembered = Get-Item -LiteralPath $manufacturerKey
        if ($remembered.GetValue('') -eq $install -and $remembered.GetValueNames().Count -eq 1 -and $remembered.SubKeyCount -eq 0) {
            Remove-Item -LiteralPath $manufacturerKey
        }
    }
    # _?= keeps the uninstaller in place so its real exit code can be observed.
    # Once it has exited and registration/application are gone, remove that
    # exact generated executable; retain all sentinels and other fixture data.
    if ((-not $app -or $app.HasExited) -and -not (Test-Path $key) -and
        -not (Test-Path "$install/yuuterm-desktop.exe") -and (Test-Path "$install/uninstall.exe")) {
        Remove-Item -LiteralPath "$install/uninstall.exe"
    }
    foreach ($path in @("$stage/release/yuuterm-desktop.exe", $setup)) {
        if ($path -and (Test-Path -LiteralPath $path)) { Remove-Item -LiteralPath $path }
    }
    $env:CARGO_TARGET_DIR = $savedTarget
    Pop-Location
}
