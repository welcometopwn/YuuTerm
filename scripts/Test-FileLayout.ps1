# Two fully exited app processes share one isolated persistent WebView profile.
param([switch]$SkipBuild)
$ErrorActionPreference='Stop'
$root=Split-Path $PSScriptRoot -Parent
$folder=Join-Path $root ('target/file-layout-'+(Get-Date -Format 'yyyyMMdd-HHmmss-fff'))
$names=@('APPDATA','LOCALAPPDATA','WEBVIEW2_USER_DATA_FOLDER','YUUTERM_CAPTURE_SCOPE','YUUTERM_CAPTURE_PROFILE','YUUTERM_CAPTURE','YUUTERM_CAPTURE_REPORT','YUUTERM_FIXTURE_LINKS')
$saved=@{};foreach($name in $names){$saved[$name]=[Environment]::GetEnvironmentVariable($name,'Process')}
$app=$null;$fixture=$null
Push-Location $root
try {
  if(-not $SkipBuild){
    & npm.cmd run build;if($LASTEXITCODE){throw 'Frontend build failed'}
    & cargo.exe build -p yuuterm-desktop --features capture --locked;if($LASTEXITCODE){throw 'Native build failed'}
    & cargo.exe build -p yuuterm-ssh --example ui_fixture --locked;if($LASTEXITCODE){throw 'Loopback fixture build failed'}
  }
  New-Item -ItemType Directory -Path $folder | Out-Null
  Copy-Item target/debug/yuuterm-desktop.exe (Join-Path $folder 'app.exe')
  Copy-Item target/debug/examples/ui_fixture.exe (Join-Path $folder 'fixture.exe')
  $env:APPDATA=Join-Path $folder 'Roaming';$env:LOCALAPPDATA=Join-Path $folder 'Local'
  $env:WEBVIEW2_USER_DATA_FOLDER=Join-Path $folder 'WebView'
  $env:YUUTERM_FIXTURE_LINKS='1';$env:YUUTERM_CAPTURE_PROFILE=Join-Path $folder 'profile.json'
  $fixture=Start-Process (Join-Path $folder 'fixture.exe') -ArgumentList ('"'+$folder+'"') -WindowStyle Hidden -PassThru -RedirectStandardError (Join-Path $folder 'fixture.err')
  $deadline=[DateTime]::UtcNow.AddSeconds(10)
  while(-not(Test-Path $env:YUUTERM_CAPTURE_PROFILE) -and [DateTime]::UtcNow -lt $deadline){if($fixture.HasExited){throw 'Loopback fixture failed'};Start-Sleep -Milliseconds 100}
  if(-not(Test-Path $env:YUUTERM_CAPTURE_PROFILE)){throw 'Loopback fixture deadline'}
  $runs=@()
  foreach($phase in @('prepare','restore')){
    $env:YUUTERM_CAPTURE_SCOPE='layout-'+$phase
    $env:YUUTERM_CAPTURE=Join-Path $folder ($phase+'.png');$env:YUUTERM_CAPTURE_REPORT=Join-Path $folder ($phase+'-report.json')
    $app=Start-Process (Join-Path $folder 'app.exe') -WindowStyle Hidden -PassThru -RedirectStandardError (Join-Path $folder ($phase+'.err'))
    $null=$app.Handle
    if(-not $app.WaitForExit(60000)){throw "Layout $phase deadline exceeded"}
    $report=Get-Content $env:YUUTERM_CAPTURE_REPORT -Raw | ConvertFrom-Json
    if($app.ExitCode -ne 0 -or -not $report.passed -or -not(Test-Path $env:YUUTERM_CAPTURE)){throw "Layout $phase failed; report retained"}
    $runs+=@{phase=$phase;pid=$app.Id;checks=$report.checks.Count;exit_code=$app.ExitCode}
  }
  if($runs[0].pid -eq $runs[1].pid){throw 'Process identity unexpectedly reused'}
  @{passed=$true;distinct_app_processes=$true;first_exited_before_second=$true;same_persistent_webview_profile=$true;same_real_loopback_ssh_sftp_server=$true;runs=$runs;executable_sha256=(Get-FileHash (Join-Path $folder 'app.exe')).Hash} | ConvertTo-Json -Depth 5 | Set-Content (Join-Path $folder 'restart-report.json')
  [pscustomobject]@{Passed=$true;Folder=$folder;PrepareChecks=$runs[0].checks;RestoreChecks=$runs[1].checks}
} finally {
  foreach($process in @($app,$fixture)){if($process -and -not $process.HasExited){$process.Kill();$null=$process.WaitForExit(5000)}}
  foreach($name in @('app.exe','fixture.exe')){$copy=Join-Path $folder $name;if(Test-Path -LiteralPath $copy){Remove-Item -LiteralPath $copy}}
  foreach($name in $names){[Environment]::SetEnvironmentVariable($name,$saved[$name],'Process')}
  Pop-Location
}
