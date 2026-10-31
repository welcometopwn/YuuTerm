# Local terminal in the actual native app: a capture build with an isolated
# profile opens a Windows PowerShell bookmark (C:\Windows) through ConPTY, runs
# typed input, checks the console size and exits with a code. Writes report.json
# and local-terminal.png into target/local-terminal-<time>.
# -Measure: holds one idle local session instead, samples CPU/memory of the
# whole process tree (app, WebView2, console host, shell) for 30 s, closes the
# window like a user and requires that no shell or console host outlives it.
param([switch]$SkipBuild, [switch]$Measure, [switch]$Flood, [int]$Seconds = 30)
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$folder = Join-Path $root ('target/local-terminal-' + (Get-Date -Format 'yyyyMMdd-HHmmss-fff'))
$names = @('YUUTERM_CAPTURE', 'YUUTERM_CAPTURE_SCOPE', 'YUUTERM_CAPTURE_REPORT', 'WEBVIEW2_USER_DATA_FOLDER', 'APPDATA', 'LOCALAPPDATA')
$saved = @{}; foreach ($name in $names) { $saved[$name] = [Environment]::GetEnvironmentVariable($name, 'Process') }
$app = $null
Push-Location $root
try {
    $env:PATH = "$env:USERPROFILE/.cargo/bin;$env:PATH"
    if (-not $SkipBuild) {
        & npm.cmd run build; if ($LASTEXITCODE) { throw 'Frontend build failed' }
        & cargo.exe build --release -p yuuterm-desktop --features capture --locked; if ($LASTEXITCODE) { throw 'Native capture build failed' }
    }
    New-Item -ItemType Directory -Path $folder | Out-Null
    $env:YUUTERM_CAPTURE_SCOPE = if ($Flood) { 'local-flood' } elseif ($Measure) { 'local-hold' } else { 'local-terminal' }
    $env:YUUTERM_CAPTURE = Join-Path $folder 'local-terminal.png'
    $env:YUUTERM_CAPTURE_REPORT = Join-Path $folder 'report.json'
    $env:WEBVIEW2_USER_DATA_FOLDER = Join-Path $folder 'WebView'
    $env:APPDATA = Join-Path $folder 'AppData'; $env:LOCALAPPDATA = Join-Path $folder 'LocalAppData'
    $app = Start-Process (Join-Path $root 'target/release/yuuterm-desktop.exe') -PassThru -RedirectStandardError (Join-Path $folder 'app.err')
    if ($Measure -or $Flood) {
        function Tree([int]$id) {
            $all = Get-CimInstance Win32_Process
            $ids = [Collections.Generic.HashSet[int]]::new(); [void]$ids.Add($id)
            do { $added = $false; foreach ($p in $all) { if ($ids.Contains([int]$p.ParentProcessId) -and $ids.Add([int]$p.ProcessId)) { $added = $true } } } while ($added)
            return @(Get-Process -Id @($ids) -ErrorAction SilentlyContinue)
        }
        $deadline = [DateTime]::UtcNow.AddSeconds(30)
        do { Start-Sleep -Milliseconds 300; $shells = @(Tree $app.Id | Where-Object Name -eq 'powershell') } until ($shells.Count -or [DateTime]::UtcNow -gt $deadline)
        if (-not $shells.Count) { throw 'The local shell did not start' }
        Start-Sleep 10
        $before = Tree $app.Id
        $watch = [Diagnostics.Stopwatch]::StartNew()
        $cpu0 = ($before | ForEach-Object { $_.TotalProcessorTime.TotalMilliseconds } | Measure-Object -Sum).Sum
        $start = @{}; foreach ($p in $before) { $start[$p.Id] = $p.TotalProcessorTime.TotalMilliseconds }
        Start-Sleep $Seconds
        $after = Tree $app.Id
        if (@($after | Where-Object { $before.Id -contains $_.Id }).Count -ne $before.Count -or $after.Count -ne $before.Count) { throw 'Process tree changed during the measurement' }
        $cpu1 = ($after | ForEach-Object { $_.TotalProcessorTime.TotalMilliseconds } | Measure-Object -Sum).Sum
        $result = [ordered]@{
            processes = ($after.Name | Sort-Object) -join ', '
            cpu_percent_of_one_core = [math]::Round(($cpu1 - $cpu0) / $watch.Elapsed.TotalMilliseconds * 100, 2)
            private_mb = [math]::Round((($after | Measure-Object PrivateMemorySize64 -Sum).Sum) / 1MB, 1)
            shell_and_console_private_mb = [math]::Round((($after | Where-Object Name -in 'powershell', 'conhost', 'OpenConsole' | Measure-Object PrivateMemorySize64 -Sum).Sum) / 1MB, 1)
            seconds = $Seconds
            per_process = ($after | ForEach-Object { '{0}={1:N2}' -f $_.Name, (($_.TotalProcessorTime.TotalMilliseconds - $start[$_.Id]) / $watch.Elapsed.TotalMilliseconds * 100) }) -join '; '
        }
        $children = @($after | Where-Object Id -ne $app.Id | Where-Object Name -in 'powershell', 'conhost', 'OpenConsole')
        [void]$app.CloseMainWindow()
        if (-not $app.WaitForExit(20000)) { throw 'The app did not close' }
        Start-Sleep 2
        $left = @($children | Where-Object { Get-Process -Id $_.Id -ErrorAction SilentlyContinue })
        if ($left.Count) { throw "Shell processes outlived the app: $($left.Name -join ', ')" }
        $result.shell_closed_with_app = $true
        [pscustomobject]$result | ConvertTo-Json | Set-Content (Join-Path $folder 'measure.json') -Encoding utf8
        return [pscustomobject]$result
    }
    if (-not $app.WaitForExit(90000)) { throw 'Local terminal test exceeded its 90-second deadline' }
    # No shell may outlive the app.
    Start-Sleep -Milliseconds 500
    $orphans = @(Get-CimInstance Win32_Process -Filter "ParentProcessId = $($app.Id)" -ErrorAction SilentlyContinue)
    $report = Get-Content $env:YUUTERM_CAPTURE_REPORT -Raw | ConvertFrom-Json
    if ($app.ExitCode -ne 0 -or -not $report.passed) { throw ('Local terminal checks failed: ' + ($report.checks -join ' | ')) }
    if ($orphans.Count) { throw "Shell processes outlived the app: $($orphans.Name -join ', ')" }
    [pscustomobject]@{ Passed = $true; Checks = $report.checks.Count; Folder = $folder }
}
finally {
    if ($app -and -not $app.HasExited) { $app.Kill(); $app.WaitForExit() }
    foreach ($name in $names) { [Environment]::SetEnvironmentVariable($name, $saved[$name], 'Process') }
    Pop-Location
}
