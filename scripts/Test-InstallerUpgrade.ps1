# Different-version upgrade qualification. Packages an older and a newer release
# under the isolated installer fixture identity (versions 0.0.9 and the current
# one), installs the old one, upgrades in place, refuses a silent downgrade and
# checks that user data survives. Never touches the real YuuTerm installation.
param(
    [Parameter(Mandatory)][string]$OldApplicationPath,
    [string]$NewApplicationPath
)
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$folder = Join-Path $root ('target/installer-upgrade-' + (Get-Date -Format 'yyyyMMdd-HHmmss-fff'))
$install = Join-Path $folder 'installed'
$product = 'YuuTerm Installer Fixture'
$key = "HKCU:/Software/Microsoft/Windows/CurrentVersion/Uninstall/$product"
$manufacturerKey = "HKCU:/Software/welcometopwn/$product"
$checks = [Collections.Generic.List[string]]::new()
$runs = [Collections.Generic.List[object]]::new()
$app = $null
$setups = @{}
$savedTarget = $env:CARGO_TARGET_DIR
function Check([bool]$condition, [string]$name) {
    if (-not $condition) { throw "Upgrade check failed: $name" }
    $checks.Add($name)
}
function Run-Package([string]$path, [string[]]$arguments) {
    $p = Start-Process $path -ArgumentList $arguments -WindowStyle Hidden -PassThru
    if (-not $p.WaitForExit(60000)) { throw 'Package exceeded 60-second deadline; process left untouched' }
    $runs.Add(@{package = [IO.Path]::GetFileName($path); arguments = ($arguments -join ' '); exit_code = $p.ExitCode })
    return $p.ExitCode
}
function Build-Package([string]$name, [string]$application, [string]$version) {
    $stage = Join-Path $folder "cargo-$name"
    New-Item -ItemType Directory -Path "$stage/release" | Out-Null
    Copy-Item $application "$stage/release/yuuterm-desktop.exe"
    $config = Join-Path $folder "$name.conf.json"
    @{productName = $product; identifier = 'io.github.welcometopwn.yuuterm.installerfixture'; version = $version } |
        ConvertTo-Json | Set-Content $config -Encoding utf8
    $env:CARGO_TARGET_DIR = $stage
    & npx.cmd tauri bundle --config crates/yuuterm-desktop/tauri.conf.json --config crates/yuuterm-desktop/tauri.installer.conf.json --config $config --bundles nsis --features custom-protocol --ci --no-binary-patching | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "$name fixture bundle failed" }
    $packages = @(Get-ChildItem "$stage/release/bundle/nsis" -Filter '*setup.exe')
    if ($packages.Count -ne 1) { throw "Expected exactly one $name installer" }
    Remove-Item -LiteralPath "$stage/release/yuuterm-desktop.exe"
    return $packages[0].FullName
}
Push-Location $root
try {
    foreach ($existing in @($key, $manufacturerKey, "HKLM:/Software/Microsoft/Windows/CurrentVersion/Uninstall/$product")) {
        if (Test-Path -LiteralPath $existing) { throw 'Fixture identity already exists; refusing to modify it' }
    }
    New-Item -ItemType Directory -Path $folder | Out-Null
    $old = (Resolve-Path -LiteralPath $OldApplicationPath).Path
    $new = if ($NewApplicationPath) { (Resolve-Path -LiteralPath $NewApplicationPath).Path } else { Join-Path $root 'target/release/yuuterm-desktop.exe' }
    $oldHash = (Get-FileHash $old).Hash; $newHash = (Get-FileHash $new).Hash
    if ($oldHash -eq $newHash) { throw 'Old and new application must differ' }
    $version = (Get-Content crates/yuuterm-desktop/tauri.conf.json -Raw | ConvertFrom-Json).version
    $setups.old = Build-Package 'old' $old '0.0.9'
    $setups.new = Build-Package 'new' $new $version

    Check ((Run-Package $setups.old @('/S', '/NS', "/D=$install")) -eq 0) 'older version 0.0.9 installs silently for the current user'
    $installedApp = Join-Path $install 'yuuterm-desktop.exe'
    Check ((Get-FileHash $installedApp).Hash -eq $oldHash -and (Get-ItemProperty -LiteralPath $key).DisplayVersion -eq '0.0.9') 'older binary and version are registered'

    $local = Join-Path $folder 'profile-local'; $roaming = Join-Path $folder 'profile-roaming'
    foreach ($dir in @($local, $roaming)) { New-Item -ItemType Directory $dir | Out-Null }
    $sentinels = @("$install/user-owned.txt", "$local/settings-sentinel.txt", "$roaming/preferences-sentinel.txt")
    foreach ($path in $sentinels) { [IO.File]::WriteAllText($path, 'fixture-preserved-data') }

    Check ((Run-Package $setups.new @('/S', '/NS', "/D=$install")) -eq 0) "upgrade to $version succeeds in place"
    Check ((Get-FileHash $installedApp).Hash -eq $newHash) 'upgrade replaces the application with the newer bytes'
    Check ((Get-ItemProperty -LiteralPath $key).DisplayVersion -eq $version) "registration reports version $version"
    Check (@(Get-ChildItem "HKCU:/Software/Microsoft/Windows/CurrentVersion/Uninstall" | Where-Object { $_.PSChildName -like "$product*" }).Count -eq 1) 'upgrade leaves exactly one uninstall registration'
    foreach ($path in $sentinels) { Check ([IO.File]::ReadAllText($path) -eq 'fixture-preserved-data') 'upgrade preserves user-owned data' }

    $downgrade = Run-Package $setups.old @('/S', '/NS', "/D=$install")
    Check ($downgrade -ne 0) "silent downgrade to 0.0.9 is refused (exit code $downgrade)"
    Check ((Get-FileHash $installedApp).Hash -eq $newHash -and (Get-ItemProperty -LiteralPath $key).DisplayVersion -eq $version) 'refused downgrade leaves the newer installation intact'

    $webview = Join-Path $folder 'webview'; New-Item -ItemType Directory $webview | Out-Null
    $app = Start-Process $installedApp -Environment @{LOCALAPPDATA = $local; APPDATA = $roaming; WEBVIEW2_USER_DATA_FOLDER = $webview } -PassThru
    $deadline = [DateTime]::UtcNow.AddSeconds(30)
    do { Start-Sleep -Milliseconds 200; $app.Refresh() } while (-not $app.HasExited -and $app.MainWindowHandle -eq 0 -and [DateTime]::UtcNow -lt $deadline)
    Check (-not $app.HasExited -and $app.MainWindowHandle -ne 0 -and $app.Responding) 'upgraded application opens a responsive native window'
    Check ($app.CloseMainWindow() -and $app.WaitForExit(15000)) 'upgraded application exits normally'

    Check ((Run-Package (Join-Path $install 'uninstall.exe') @('/S', "_?=$install")) -eq 0) 'upgraded installation uninstalls silently'
    $deadline = [DateTime]::UtcNow.AddSeconds(10)
    while ((Test-Path $installedApp) -and [DateTime]::UtcNow -lt $deadline) { Start-Sleep -Milliseconds 100 }
    Check (-not (Test-Path $installedApp) -and -not (Test-Path $key)) 'uninstall removes application and registration'
    foreach ($path in $sentinels) { Check ([IO.File]::ReadAllText($path) -eq 'fixture-preserved-data') 'uninstall preserves user-owned data' }
    @{passed = $true; old_sha256 = $oldHash; new_sha256 = $newHash; new_version = $version; checks = $checks; package_runs = $runs } |
        ConvertTo-Json -Depth 5 | Set-Content "$folder/report.json" -Encoding utf8
    [pscustomobject]@{ Passed = $true; Checks = $checks.Count; Folder = $folder }
} catch {
    if (Test-Path $folder) { @{passed = $false; checks = $checks; package_runs = $runs; error = $_.Exception.Message } | ConvertTo-Json -Depth 5 | Set-Content "$folder/report.json" -Encoding utf8 }
    throw
} finally {
    if ($app -and -not $app.HasExited) { $null = $app.CloseMainWindow(); $null = $app.WaitForExit(15000) }
    if ((-not $app -or $app.HasExited) -and (Test-Path "$install/uninstall.exe") -and (Test-Path $key)) {
        $null = Run-Package "$install/uninstall.exe" @('/S', "_?=$install")
    }
    if ((Test-Path $manufacturerKey) -and -not (Test-Path $key)) {
        $remembered = Get-Item -LiteralPath $manufacturerKey
        if ($remembered.GetValue('') -eq $install -and $remembered.GetValueNames().Count -eq 1 -and $remembered.SubKeyCount -eq 0) {
            Remove-Item -LiteralPath $manufacturerKey
        }
    }
    if (-not (Test-Path $key) -and -not (Test-Path "$install/yuuterm-desktop.exe") -and (Test-Path "$install/uninstall.exe")) {
        Remove-Item -LiteralPath "$install/uninstall.exe"
    }
    # Keep only the report: generated packages and staging are disposable.
    foreach ($name in 'old', 'new') {
        $stage = Join-Path $folder "cargo-$name"
        if (Test-Path -LiteralPath $stage) { Remove-Item -LiteralPath $stage -Recurse -Force }
    }
    $env:CARGO_TARGET_DIR = $savedTarget
    Pop-Location
}
