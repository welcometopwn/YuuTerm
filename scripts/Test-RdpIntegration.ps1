param([switch]$SkipBuild)
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
Push-Location $root
$process = $null
$savedPath = $env:PATH
$env:PATH = "$env:USERPROFILE/.cargo/bin;$env:PATH"
$names = @('YUUTERM_RDP_PROBE', 'YUUTERM_RDP_PROBE_REPORT', 'YUUTERM_RDP_PROBE_CAPTURE')
$saved = @{}
foreach ($name in $names) { $saved[$name] = [Environment]::GetEnvironmentVariable($name, 'Process') }
try {
    if (-not $SkipBuild) {
        & npm.cmd run desktop:build -- --debug --features rdp-probe --ci -- --locked
        if ($LASTEXITCODE -ne 0) { throw 'RDP developer build failed' }
    }
    $folder = Join-Path $root ('target/rdp-events-' + (Get-Date -Format 'yyyyMMdd-HHmmss-fff'))
    New-Item -ItemType Directory -Path $folder | Out-Null
    $env:YUUTERM_RDP_PROBE = '1'
    $env:YUUTERM_RDP_PROBE_REPORT = Join-Path $folder 'report.json'
    $env:YUUTERM_RDP_PROBE_CAPTURE = Join-Path $folder 'native.png'
    $process = Start-Process -FilePath (Join-Path $root 'target/debug/yuuterm-desktop.exe') -WindowStyle Hidden -PassThru `
        -RedirectStandardOutput (Join-Path $folder 'stdout.log') -RedirectStandardError (Join-Path $folder 'stderr.log')
    if (-not $process.WaitForExit(30000)) { throw 'RDP developer probe exceeded its 30-second deadline' }
    if (-not (Test-Path -LiteralPath $env:YUUTERM_RDP_PROBE_REPORT)) { throw 'RDP probe produced no report (check the developer feature build)' }
    $report = Get-Content -LiteralPath $env:YUUTERM_RDP_PROBE_REPORT -Raw | ConvertFrom-Json
    if ($process.ExitCode -ne 0 -or -not $report.passed) { throw ('RDP probe failed: ' + $report.failure) }
    if (-not (Test-Path -LiteralPath $env:YUUTERM_RDP_PROBE_CAPTURE)) { throw 'Native window capture missing' }
    [pscustomobject]@{ Passed = $report.passed; Checks = $report.checks.Count; Folder = $folder }
}
finally {
    # Only this helper's process is eligible for cleanup. Never stop a user app.
    if ($process -and -not $process.HasExited) { $process.Kill(); $process.WaitForExit() }
    foreach ($name in $names) { [Environment]::SetEnvironmentVariable($name, $saved[$name], 'Process') }
    $env:PATH = $savedPath
    Pop-Location
}
