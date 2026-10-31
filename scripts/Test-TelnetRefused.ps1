[CmdletBinding()]
param([switch]$SkipBuild)
$ErrorActionPreference='Stop'
$root=Split-Path $PSScriptRoot -Parent
$folder=Join-Path $root ('target/telnet-refused-'+(Get-Date -Format 'yyyyMMdd-HHmmss-fff'))
$names=@('APPDATA','LOCALAPPDATA','WEBVIEW2_USER_DATA_FOLDER','YUUTERM_CAPTURE_PROFILE','YUUTERM_CAPTURE_SCOPE','YUUTERM_CAPTURE','YUUTERM_CAPTURE_REPORT')
$saved=@{};foreach($name in $names){$saved[$name]=[Environment]::GetEnvironmentVariable($name,'Process')}
$app=$null;$socket=$null
Push-Location $root
try {
 if(-not $SkipBuild){
  & npm.cmd run build;if($LASTEXITCODE){throw 'Frontend build failed'}
  & cargo.exe build --release -p yuuterm-desktop --all-features --locked;if($LASTEXITCODE){throw 'Native build failed'}
 }
 New-Item -ItemType Directory -Path $folder | Out-Null
 # Bound but not listening: reserve a loopback-only port which refuses connects.
 $socket=[Net.Sockets.Socket]::new([Net.Sockets.AddressFamily]::InterNetwork,[Net.Sockets.SocketType]::Stream,[Net.Sockets.ProtocolType]::Tcp)
 $socket.ExclusiveAddressUse=$true
 $socket.Bind([Net.IPEndPoint]::new([Net.IPAddress]::Loopback,0))
 $profile=@{id=1;name='Refused TELNET fixture';protocol='Telnet';host='127.0.0.1';port=$socket.LocalEndPoint.Port;username='fixture';font_size=$null}
 [IO.File]::WriteAllText((Join-Path $folder 'profile.json'),($profile | ConvertTo-Json),[Text.UTF8Encoding]::new($false))
 $env:APPDATA=Join-Path $folder 'Roaming';$env:LOCALAPPDATA=Join-Path $folder 'Local'
 $env:WEBVIEW2_USER_DATA_FOLDER=Join-Path $folder 'WebView'
 $env:YUUTERM_CAPTURE_PROFILE=Join-Path $folder 'profile.json';$env:YUUTERM_CAPTURE_SCOPE='telnet-refused'
 $env:YUUTERM_CAPTURE=Join-Path $folder 'screen.png';$env:YUUTERM_CAPTURE_REPORT=Join-Path $folder 'report.json'
 Copy-Item (Join-Path $root 'target/release/yuuterm-desktop.exe') (Join-Path $folder 'app.exe')
 $app=Start-Process (Join-Path $folder 'app.exe') -WindowStyle Hidden -PassThru -RedirectStandardError (Join-Path $folder 'app.err')
 $null=$app.Handle
 $deadline=[DateTime]::UtcNow.AddSeconds(60)
 while(-not $app.HasExited -and [DateTime]::UtcNow -lt $deadline){Start-Sleep -Milliseconds 100}
 if(-not $app.HasExited){throw 'Refused TELNET native deadline exceeded'}
 $report=Get-Content -LiteralPath $env:YUUTERM_CAPTURE_REPORT -Raw | ConvertFrom-Json
 if($app.ExitCode -ne 0 -or -not $report.passed -or -not(Test-Path -LiteralPath $env:YUUTERM_CAPTURE)){throw ('Native refused-connect checks failed: '+($report.checks -join ' | '))}
 [pscustomobject]@{Passed=$true;Checks=$report.checks.Count;Folder=$folder}
}finally{
 if($app -and -not $app.HasExited){$app.Kill();$app.WaitForExit()}
 if($socket){$socket.Dispose()}
 foreach($name in $names){[Environment]::SetEnvironmentVariable($name,$saved[$name],'Process')}
 $copy=Join-Path $folder 'app.exe';if(Test-Path -LiteralPath $copy){Remove-Item -LiteralPath $copy -Force}
 Pop-Location
}
