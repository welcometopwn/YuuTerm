param(
    [ValidateSet('all', 'session', 'files', 'file-navigation', 'editor', 'recovery', 'filemenus', 'history', 'logging', 'transfer', 'pollwait', 'icons', 'qol', 'clipboard')]
    [string]$Scope = 'logging',
    [switch]$SkipBuild,
    [switch]$MobaImport,
    [string]$BinaryPath
)
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$folder = Join-Path $root ('target/' + $Scope + '-' + (Get-Date -Format 'yyyyMMdd-HHmmss-fff'))
$app = $null
$fixture = $null
$savedPath = $env:PATH
$names = @('YUUTERM_FIXTURE_LINKS', 'YUUTERM_FIXTURE_NAVIGATION', 'YUUTERM_CAPTURE_SCOPE', 'YUUTERM_CAPTURE_PROFILE', 'YUUTERM_CAPTURE', 'YUUTERM_CAPTURE_REPORT', 'YUUTERM_CAPTURE_MOBA_IMPORT', 'APPDATA', 'LOCALAPPDATA', 'WEBVIEW2_USER_DATA_FOLDER')
$saved = @{}
foreach ($name in $names) { $saved[$name] = [Environment]::GetEnvironmentVariable($name, 'Process') }
Push-Location $root
try {
    $env:PATH = "$env:USERPROFILE/.cargo/bin;$env:PATH"
    if (-not $SkipBuild) {
        & npm.cmd run desktop:build -- --debug --features capture --ci -- --locked
        if ($LASTEXITCODE -ne 0) { throw 'Native capture build failed' }
        & cargo.exe build -p yuuterm-ssh --example ui_fixture --locked
        if ($LASTEXITCODE -ne 0) { throw 'SSH fixture build failed' }
    }
    New-Item -ItemType Directory -Path $folder | Out-Null
    if ($MobaImport) {
        if ($Scope -ne 'transfer') { throw 'MobaImport requires the transfer scope' }
        Copy-Item (Join-Path $root 'crates/yuuterm-core/tests/fixtures/MobaXterm.ini') (Join-Path $folder 'moba-import.ini')
        $env:YUUTERM_CAPTURE_MOBA_IMPORT = '1'
    } else { Remove-Item Env:YUUTERM_CAPTURE_MOBA_IMPORT -ErrorAction SilentlyContinue }
    if(-not $BinaryPath){$BinaryPath=Join-Path $root 'target/debug/yuuterm-desktop.exe'}
    Copy-Item -LiteralPath $BinaryPath (Join-Path $folder 'app.exe')
    Copy-Item target/debug/examples/ui_fixture.exe (Join-Path $folder 'fixture.exe')
    $env:YUUTERM_FIXTURE_LINKS = '1'
    if($Scope -eq 'file-navigation'){$env:YUUTERM_FIXTURE_NAVIGATION='1'}else{Remove-Item Env:YUUTERM_FIXTURE_NAVIGATION -ErrorAction SilentlyContinue}
    $fixture = Start-Process (Join-Path $folder 'fixture.exe') -WindowStyle Hidden -PassThru `
        -ArgumentList ('"' + $folder + '"') -RedirectStandardError (Join-Path $folder 'fixture.err')
    $deadline = [DateTime]::UtcNow.AddSeconds(10)
    while (-not (Test-Path (Join-Path $folder 'profile.json')) -and [DateTime]::UtcNow -lt $deadline) {
        if ($fixture.HasExited) { throw 'Fixture exited before generating its profile; check the Windows SSH agent' }
        Start-Sleep -Milliseconds 100
    }
    if (-not (Test-Path (Join-Path $folder 'profile.json'))) { throw 'Fixture profile deadline exceeded' }
    if ($Scope -eq 'logging') {
        $custom = Join-Path $folder 'custom/custom.log'
        New-Item -ItemType Directory -Path (Split-Path $custom -Parent) | Out-Null
        [IO.File]::WriteAllText($custom, "existing-custom-prefix`r`n", [Text.UTF8Encoding]::new($false))
        $profile = Get-Content (Join-Path $folder 'profile.json') -Raw | ConvertFrom-Json
        $profile | Add-Member -NotePropertyName logging -NotePropertyValue @{enabled=$false;file=$custom} -Force
        $profile | ConvertTo-Json -Depth 10 | Set-Content (Join-Path $folder 'profile.json') -Encoding utf8
    }
    $env:YUUTERM_CAPTURE_SCOPE = $Scope
    $env:YUUTERM_CAPTURE_PROFILE = Join-Path $folder 'profile.json'
    $env:YUUTERM_CAPTURE = Join-Path $folder 'screen.png'
    $env:YUUTERM_CAPTURE_REPORT = Join-Path $folder 'report.json'
    # Own data folder: a running YuuTerm owns the user's folder (single instance)
    # and would otherwise receive this launch instead of the fixture.
    $env:APPDATA = Join-Path $folder 'AppData'; $env:LOCALAPPDATA = Join-Path $folder 'LocalAppData'
    $env:WEBVIEW2_USER_DATA_FOLDER = Join-Path $folder 'WebView'
    $app = Start-Process (Join-Path $folder 'app.exe') -WindowStyle Hidden -PassThru -RedirectStandardError (Join-Path $folder 'app.err')
    # Windows PowerShell 5 needs the process handle cached before exit to retain ExitCode.
    $null = $app.Handle
    $deadline = [DateTime]::UtcNow.AddSeconds(60)
    while (-not $app.HasExited -and [DateTime]::UtcNow -lt $deadline) {
        if (Test-Path $env:YUUTERM_CAPTURE_REPORT) {
            try { $report = Get-Content $env:YUUTERM_CAPTURE_REPORT -Raw | ConvertFrom-Json } catch { $report = $null }
            if ($report -and -not $report.passed) { throw 'Native fixture checks failed; report retained' }
        }
        Start-Sleep -Milliseconds 100
    }
    if (-not $app.HasExited) { throw 'Native capture exceeded its 60-second deadline' }
    $app.WaitForExit()
    if ($app.ExitCode -ne 0) { throw ('Native capture exited with code ' + $app.ExitCode) }
    $report = Get-Content $env:YUUTERM_CAPTURE_REPORT -Raw | ConvertFrom-Json
    if (-not $report.passed) { throw ('Native fixture checks failed: ' + $report.checks[-1]) }
    if (-not (Test-Path $env:YUUTERM_CAPTURE)) { throw 'Native screenshot missing' }
    if ($Scope -eq 'logging') {
        $defaultDir = Join-Path $folder 'Documents/YuuTerm/SSH fixture 1'
        $defaults = @(Get-ChildItem $defaultDir -Filter '*.log')
        if ($defaults.Count -ne 2) { throw 'Default reconnect did not create two timestamped session logs' }
        $defaultText = ($defaults | ForEach-Object { [IO.File]::ReadAllText($_.FullName) }) -join [Environment]::NewLine
        if (-not $defaultText.Contains('default-auto-one') -or -not $defaultText.Contains('default-auto-two') -or -not $defaultText.Contains('Ready')) { throw 'Default logs missed banner or connection markers' }
        $customBytes = [IO.File]::ReadAllBytes((Join-Path $folder 'custom/custom.log'))
        if ($customBytes.Length -gt 16384) { throw 'Unexpected fixture log size' }
        $customText = [Text.Encoding]::UTF8.GetString($customBytes)
        if (-not $customText.StartsWith("existing-custom-prefix`r`n") -or -not $customText.Contains('custom-auto-one') -or -not $customText.Contains('custom-auto-two') -or -not $customText.Contains('custom-auto-three')) { throw 'Custom log append or reconnect markers missing' }
        if (($defaultText + $customText).Contains('logging-disabled-marker')) { throw 'Disabled output was recorded' }
        $records = Get-Content (Join-Path $folder 'log-shutdown.json') -Raw | ConvertFrom-Json -NoEnumerate
        if ($records.Count -ne 1 -or -not $records[0][1].finished -or $records[0][1].active -or $records[0][1].error) { throw 'Native close did not finish automatic logging' }
        @{passed=$true;default_session_files=$defaults.Count;default_banner_and_reconnect_markers=$true;
          existing_custom_prefix_preserved=$true;custom_reconnect_markers=3;disabled_output_absent=$true;graceful_close_finished=$true} |
            ConvertTo-Json | Set-Content (Join-Path $folder 'log-content-checks.json') -Encoding utf8
    }
    if ($Scope -eq 'qol') {
        # The whole-tree batch must hold real files but no link entries.
        $tree = Get-ChildItem (Join-Path $folder 'downloads') -Directory -Filter 'YuuTerm-download-*' |
            ForEach-Object { Join-Path $_.FullName 'fixture' } | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
        if (-not $tree) { throw 'Directory download did not create the fixture tree' }
        if (-not (Test-Path -LiteralPath (Join-Path $tree 'read me.txt'))) { throw 'Directory download missed a regular file' }
        foreach ($link in 'broken-link', 'cycle-link', 'dir-link', 'file-link') {
            if (Test-Path -LiteralPath (Join-Path $tree $link)) { throw "Directory download followed or copied $link" }
        }
    }
    [pscustomobject]@{ Passed = $true; Checks = $report.checks.Count; Folder = $folder }
}
finally {
    foreach ($process in @($app, $fixture)) {
        if ($process -and -not $process.HasExited) { $process.Kill(); $process.WaitForExit() }
    }
    # Exact generated files only; reports, profiles and staged/recovery work stay.
    foreach ($name in @('app.exe', 'fixture.exe')) {
        $copy = Join-Path $folder $name
        if (Test-Path -LiteralPath $copy) { Remove-Item -LiteralPath $copy }
    }
    foreach ($name in $names) { [Environment]::SetEnvironmentVariable($name, $saved[$name], 'Process') }
    $env:PATH = $savedPath
    Pop-Location
}
