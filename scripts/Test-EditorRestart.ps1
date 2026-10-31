param([switch]$SkipBuild)
$ErrorActionPreference='Stop'
$root=[IO.Path]::GetFullPath((Split-Path $PSScriptRoot -Parent))
$folder=Join-Path $root ('target/editor-restart-'+(Get-Date -Format 'yyyyMMdd-HHmmss-fff'))
$names=@('LOCALAPPDATA','APPDATA','WEBVIEW2_USER_DATA_FOLDER','YUUTERM_FIXTURE_LINKS','YUUTERM_CAPTURE_SCOPE','YUUTERM_CAPTURE_PROFILE','YUUTERM_CAPTURE','YUUTERM_CAPTURE_REPORT','TEMP','TMP')
$saved=@{};foreach($name in $names){$saved[$name]=[Environment]::GetEnvironmentVariable($name,'Process')}
$app=$null;$fixture=$null
Push-Location $root
try {
    New-Item -ItemType Directory -Force target/test-temp | Out-Null
    $env:TEMP=Join-Path $root 'target/test-temp';$env:TMP=$env:TEMP
    if(-not $SkipBuild){
        & npm.cmd run desktop:build -- --debug --features capture --ci -- --locked
        if($LASTEXITCODE -ne 0){throw 'Native build failed'}
        & cargo.exe build -p yuuterm-ssh --example ui_fixture --locked
        if($LASTEXITCODE -ne 0){throw 'Loopback fixture build failed'}
    }
    New-Item -ItemType Directory $folder | Out-Null
    Copy-Item target/debug/yuuterm-desktop.exe (Join-Path $folder 'app.exe')
    Copy-Item target/debug/examples/ui_fixture.exe (Join-Path $folder 'fixture.exe')
    $env:LOCALAPPDATA=Join-Path $folder 'LocalAppData';$env:APPDATA=Join-Path $folder 'AppData'
    $env:WEBVIEW2_USER_DATA_FOLDER=Join-Path $folder 'WebView2'
    $env:YUUTERM_FIXTURE_LINKS='1'
    $env:YUUTERM_CAPTURE_PROFILE=Join-Path $folder 'profile.json'
    $fixture=Start-Process (Join-Path $folder 'fixture.exe') -WindowStyle Hidden -PassThru -ArgumentList ('"'+$folder+'"') -RedirectStandardError (Join-Path $folder 'fixture.err')
    $deadline=[DateTime]::UtcNow.AddSeconds(15)
    while(-not(Test-Path $env:YUUTERM_CAPTURE_PROFILE) -and [DateTime]::UtcNow -lt $deadline){if($fixture.HasExited){throw 'Loopback fixture failed'};Start-Sleep -Milliseconds 100}
    if(-not(Test-Path $env:YUUTERM_CAPTURE_PROFILE)){throw 'Fixture startup deadline'}
    $runs=@()
    foreach($phase in @('prepare','restore')){
        if($phase -eq 'restore'){
            $records=@(Get-ChildItem (Join-Path $folder 'staging') -Directory | ForEach-Object {
                $file=Join-Path $_.FullName 'recovery.json';$record=Get-Content -LiteralPath $file -Raw | ConvertFrom-Json
                [pscustomobject]@{remote=$record.remote;local=(Join-Path $_.FullName $record.filename);record=$file}
            })
            $legacy=$records | Where-Object remote -eq '/fixture/restart-legacy.txt'
            @{remote=$legacy.remote;local=$legacy.local;endpoint='fixture@127.0.0.1 (legacy fixture)'} | ConvertTo-Json | Set-Content -LiteralPath $legacy.record -Encoding utf8
            # PowerShell 5 writes a BOM; JSON produced by the old Rust version did not.
            $text=[IO.File]::ReadAllText($legacy.record);[IO.File]::WriteAllText($legacy.record,$text,[Text.UTF8Encoding]::new($false))
        }
        $env:YUUTERM_CAPTURE_SCOPE='restart-'+$phase
        $env:YUUTERM_CAPTURE=Join-Path $folder ($phase+'.png')
        $env:YUUTERM_CAPTURE_REPORT=Join-Path $folder ($phase+'-report.json')
        $app=Start-Process (Join-Path $folder 'app.exe') -WindowStyle Hidden -PassThru -RedirectStandardError (Join-Path $folder ($phase+'.err'))
        $null=$app.Handle;$pidValue=$app.Id
        $deadline=[DateTime]::UtcNow.AddSeconds(90)
        while(-not $app.HasExited -and [DateTime]::UtcNow -lt $deadline){Start-Sleep -Milliseconds 100}
        if(-not $app.HasExited){throw "Native $phase deadline exceeded"}
        $app.WaitForExit()
        $report=Get-Content $env:YUUTERM_CAPTURE_REPORT -Raw | ConvertFrom-Json
        if($app.ExitCode -ne 0 -or -not $report.passed -or -not(Test-Path $env:YUUTERM_CAPTURE)){throw "Native $phase checks failed; report retained"}
        $runs+=@{phase=$phase;pid=$pidValue;exit_code=$app.ExitCode;checks=$report.checks.Count}
        Start-Sleep -Milliseconds 500
    }
    if($runs[0].pid -eq $runs[1].pid){throw 'App process identity was reused'}
    foreach($name in @('keep','legacy')){
        $record=$records | Where-Object remote -eq ('/fixture/restart-'+$name+'.txt')
        if(-not(Test-Path -LiteralPath $record.local) -or (Test-Path -LiteralPath $record.record) -or [IO.File]::ReadAllText($record.local) -ne ('pending restart-'+$name+'.txt')){throw 'Keep local copy did not retain exact bytes/dismiss metadata'}
    }
    $discard=$records | Where-Object remote -eq '/fixture/restart-discard.txt'
    if((Test-Path -LiteralPath $discard.local) -or (Test-Path -LiteralPath $discard.record)){throw 'Discard left selected file/record behind'}
    @{passed=$true;distinct_app_processes=$true;first_exit_before_second_start=$true;same_loopback_server_across_restart=$true;runs=$runs;kept_files_exact=$true;discarded_file_and_record_absent=$true;executable_sha256=(Get-FileHash (Join-Path $folder 'app.exe') -Algorithm SHA256).Hash} | ConvertTo-Json -Depth 6 | Set-Content (Join-Path $folder 'restart-processes.json') -Encoding utf8
    [pscustomobject]@{Passed=$true;Folder=$folder;PrepareChecks=$runs[0].checks;RestoreChecks=$runs[1].checks}
} finally {
    foreach($process in @($app,$fixture)){if($process -and -not $process.HasExited){$process.Kill();$process.WaitForExit()}}
    foreach($name in @('app.exe','fixture.exe')){ $file=Join-Path $folder $name;if(Test-Path -LiteralPath $file){Remove-Item -LiteralPath $file} }
    foreach($name in $names){[Environment]::SetEnvironmentVariable($name,$saved[$name],'Process')}
    Pop-Location
}
