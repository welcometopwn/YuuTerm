# Synthetic UI events over real loopback TCP; no timing/focus qualification.
[CmdletBinding()]
param([ValidateSet('highlight-wrap','highlight-import','log-rotation','serial-import','telnet-line','bookmark-groups','transfer','session-actions')][string]$Scope='highlight-wrap',[switch]$SkipBuild,[string]$BinaryPath)
$ErrorActionPreference='Stop'
$root=Split-Path $PSScriptRoot -Parent
$folder=Join-Path $root ('target/terminal-features-'+$Scope+'-'+(Get-Date -Format 'yyyyMMdd-HHmmss-fff'))
$app=$null;$fixture=$null;$udp=$null
$names=@('APPDATA','LOCALAPPDATA','WEBVIEW2_USER_DATA_FOLDER','YUUTERM_CAPTURE_PROFILE','YUUTERM_CAPTURE_SCOPE','YUUTERM_CAPTURE','YUUTERM_CAPTURE_REPORT','YUUTERM_CAPTURE_MOBA_IMPORT','YUUTERM_WOL_TEST_PORT')
$saved=@{};foreach($name in $names){$saved[$name]=[Environment]::GetEnvironmentVariable($name,'Process')}
Push-Location $root
try {
 if(-not $SkipBuild){
  & npm.cmd run build;if($LASTEXITCODE){throw 'Frontend build failed'}
  & cargo.exe build --release -p yuuterm-desktop --all-features --locked;if($LASTEXITCODE){throw 'Native fixture build failed'}
  & cargo.exe build --release -p yuuterm-ssh --example telnet_fixture --locked;if($LASTEXITCODE){throw 'Loopback peer build failed'}
 }
 New-Item -ItemType Directory -Path $folder | Out-Null
 if($Scope -eq 'session-actions'){
  $udp=[Net.Sockets.UdpClient]::new([Net.IPEndPoint]::new([Net.IPAddress]::Loopback,0))
  $env:YUUTERM_WOL_TEST_PORT=[string]$udp.Client.LocalEndPoint.Port
 }
 if($Scope -eq 'transfer'){Copy-Item (Join-Path $root 'crates/yuuterm-core/tests/fixtures/MobaXterm.ini') (Join-Path $folder 'moba-import.ini');$env:YUUTERM_CAPTURE_MOBA_IMPORT='1'}
 if($Scope -eq 'bookmark-groups'){Copy-Item (Join-Path $root 'crates/yuuterm-core/tests/fixtures/bookmark-groups.mxtsessions') (Join-Path $folder 'moba-import.ini');$env:YUUTERM_CAPTURE_MOBA_IMPORT='1'}
 if($Scope -eq 'serial-import'){Copy-Item (Join-Path $root 'crates/yuuterm-core/tests/fixtures/serial-variants.mxtsessions') (Join-Path $folder 'moba-import.ini');$env:YUUTERM_CAPTURE_MOBA_IMPORT='1'}
 if($Scope -eq 'highlight-import'){Copy-Item (Join-Path $root 'crates/yuuterm-core/tests/fixtures/highlight-synthetic.msyn') (Join-Path $folder 'highlight-synthetic.msyn')}
 if(-not $BinaryPath){$BinaryPath=Join-Path $root 'target/release/yuuterm-desktop.exe'}
 Copy-Item -LiteralPath $BinaryPath (Join-Path $folder 'app.exe')
 Copy-Item (Join-Path $root 'target/release/examples/telnet_fixture.exe') (Join-Path $folder 'fixture.exe')
 $fixture=Start-Process (Join-Path $folder 'fixture.exe') -ArgumentList ('"'+$folder+'"') -WindowStyle Hidden -PassThru -RedirectStandardError (Join-Path $folder 'fixture.err')
 $deadline=[DateTime]::UtcNow.AddSeconds(10)
 while(-not(Test-Path (Join-Path $folder 'profile.json')) -and [DateTime]::UtcNow -lt $deadline){Start-Sleep -Milliseconds 50}
 if(-not(Test-Path (Join-Path $folder 'profile.json'))){throw 'Loopback fixture startup failed'}
 if($Scope -eq 'log-rotation'){
  $profilePath=Join-Path $folder 'profile.json'
  $profile=Get-Content -LiteralPath $profilePath -Raw | ConvertFrom-Json
  $profile.logging.file=Join-Path $folder 'rotation.log'
  [IO.File]::WriteAllText($profilePath,($profile | ConvertTo-Json -Depth 20),[Text.UTF8Encoding]::new($false))
 }
 $env:APPDATA=Join-Path $folder 'Roaming';$env:LOCALAPPDATA=Join-Path $folder 'Local'
 $env:WEBVIEW2_USER_DATA_FOLDER=Join-Path $folder 'WebView'
 $env:YUUTERM_CAPTURE_PROFILE=Join-Path $folder 'profile.json';$env:YUUTERM_CAPTURE_SCOPE=$Scope
 $env:YUUTERM_CAPTURE=Join-Path $folder 'screen.png';$env:YUUTERM_CAPTURE_REPORT=Join-Path $folder 'report.json'
 $app=Start-Process (Join-Path $folder 'app.exe') -WindowStyle Hidden -PassThru -RedirectStandardError (Join-Path $folder 'app.err')
 $null=$app.Handle
 $deadline=[DateTime]::UtcNow.AddSeconds(90)
 while(-not $app.HasExited -and [DateTime]::UtcNow -lt $deadline){Start-Sleep -Milliseconds 100}
 if(-not $app.HasExited){throw 'Native terminal fixture deadline exceeded'}
 $report=Get-Content $env:YUUTERM_CAPTURE_REPORT -Raw | ConvertFrom-Json
 if($app.ExitCode -ne 0 -or -not $report.passed -or -not(Test-Path $env:YUUTERM_CAPTURE)){throw ('Native terminal checks failed: '+$report.checks[-1])}
 if($Scope -eq 'session-actions'){
  $udp.Client.ReceiveTimeout=2000;$remote=[Net.IPEndPoint]::new([Net.IPAddress]::Any,0)
  $packet=$udp.Receive([ref]$remote)
  $expected=[byte[]](@(255)*6+(@(2,17,34,51,68,85)*16))
  if($packet.Length -ne 102 -or [Convert]::ToBase64String($packet) -ne [Convert]::ToBase64String($expected) -or $udp.Available -ne 0){throw 'WoL bytes or MAC deduplication failed'}
  @{passed=$true;packets=1;bytes=102;mac_deduplicated=$true;scope='Actual UDP on loopback; no physical wake test or LAN broadcast'}|ConvertTo-Json|Set-Content (Join-Path $folder 'wol-packet.json')
 }
 if($Scope -eq 'log-rotation'){
  $logs=@(Get-ChildItem -LiteralPath (Join-Path $folder 'rotation.log.yuuterm-rotated') -Filter '*.log')+@(Get-Item -LiteralPath (Join-Path $folder 'rotation.log'))
  if($logs.Count -ne 2 -or @($logs | Where-Object {$_.Length -gt 1MB -or $_.Length -eq 0}).Count){throw 'Retention count or file size failed'}
  $active=[IO.File]::ReadAllText((Join-Path $folder 'rotation.log'))
  if(-not $active.Contains('rotation-reconnect-marker')){throw 'Reconnect marker missing from active file'}
  $disk=[pscustomobject]@{Passed=$true;Files=$logs.Count;MaxBytes=1MB;Lengths=@($logs | ForEach-Object {$_.Length});ReconnectMarker=$true;Scope='synthetic loopback TELNET; no focus/performance claim'}
  [IO.File]::WriteAllText((Join-Path $folder 'disk-checks.json'),($disk | ConvertTo-Json),[Text.UTF8Encoding]::new($false))
 }
 [pscustomobject]@{Passed=$true;Scope=$Scope;Checks=$report.checks.Count;Folder=$folder;LockedDesktop=$true}
}finally{
 if($udp){$udp.Dispose()}
 foreach($process in @($app,$fixture)){if($process -and -not $process.HasExited){$process.Kill();$null=$process.WaitForExit(5000)}}
 foreach($name in $names){[Environment]::SetEnvironmentVariable($name,$saved[$name],'Process')}
 foreach($name in @('app.exe','fixture.exe')){ $path=Join-Path $folder $name;if(Test-Path -LiteralPath $path){Remove-Item -LiteralPath $path -Force} }
 Pop-Location
}
