[CmdletBinding()]
param([switch]$SkipBuild, [ValidateSet('plain','vnc')][string]$Authentication='plain')
$ErrorActionPreference='Stop'
$taskRoot=Split-Path $PSScriptRoot -Parent
$taskFolder=Join-Path $taskRoot ('target/vnc-saved-'+$Authentication+'-'+(Get-Date -Format 'yyyyMMdd-HHmmss-fff'))
$taskNames=@('YUUTERM_CAPTURE_PROFILE','YUUTERM_CAPTURE_SCOPE','YUUTERM_CAPTURE','YUUTERM_CAPTURE_REPORT','WEBVIEW2_USER_DATA_FOLDER','APPDATA','LOCALAPPDATA','YUUTERM_VNC_FIXTURE_AUTH')
$taskPrevious=@{}
foreach($taskName in $taskNames){$taskPrevious[$taskName]=[Environment]::GetEnvironmentVariable($taskName,'Process')}
$taskApp=$null; $taskFixture=$null
Push-Location $taskRoot
try {
    if(-not $SkipBuild){
        & npm.cmd run build
        if($LASTEXITCODE){throw 'Frontend build failed'}
        & cargo.exe build --locked -p yuuterm-desktop --features capture
        if($LASTEXITCODE){throw 'Capture build failed'}
        & cargo.exe build --locked -p yuuterm-ssh --example vnc_fixture
        if($LASTEXITCODE){throw 'VNC fixture build failed'}
    }
    New-Item -ItemType Directory -Path $taskFolder | Out-Null
    $env:YUUTERM_VNC_FIXTURE_AUTH=$Authentication
    $taskFixture=Start-Process (Join-Path $taskRoot 'target/debug/examples/vnc_fixture.exe') -ArgumentList ('"'+$taskFolder+'"') -WindowStyle Hidden -PassThru -RedirectStandardError (Join-Path $taskFolder 'fixture.err')
    $taskDeadline=[DateTime]::UtcNow.AddSeconds(10)
    while(-not(Test-Path (Join-Path $taskFolder 'profile.json')) -and [DateTime]::UtcNow -lt $taskDeadline){Start-Sleep -Milliseconds 100}
    if(-not(Test-Path (Join-Path $taskFolder 'profile.json'))){throw 'VNC fixture profile missing'}
    $env:YUUTERM_CAPTURE_PROFILE=Join-Path $taskFolder 'profile.json'
    $env:YUUTERM_CAPTURE_SCOPE='vnc-saved'
    $env:YUUTERM_CAPTURE=Join-Path $taskFolder 'screen.png'
    $env:YUUTERM_CAPTURE_REPORT=Join-Path $taskFolder 'report.json'
    $env:WEBVIEW2_USER_DATA_FOLDER=Join-Path $taskFolder 'WebView'
    $env:APPDATA=Join-Path $taskFolder 'AppData'
    $env:LOCALAPPDATA=Join-Path $taskFolder 'LocalAppData'
    # The capture-only bootstrap seeds unique entries with the public fixture
    # password and deletes them on close. No authentication dialog is automated.
    $taskApp=Start-Process (Join-Path $taskRoot 'target/debug/yuuterm-desktop.exe') -WindowStyle Hidden -PassThru -RedirectStandardError (Join-Path $taskFolder 'app.err')
    $null=$taskApp.Handle
    $taskDeadline=[DateTime]::UtcNow.AddSeconds(90)
    while(-not $taskApp.HasExited -and [DateTime]::UtcNow -lt $taskDeadline){
        if(Test-Path $env:YUUTERM_CAPTURE_REPORT){
            $taskReport=Get-Content $env:YUUTERM_CAPTURE_REPORT -Raw | ConvertFrom-Json
            if(-not $taskReport.passed){throw 'Native saved-VNC checks failed; report retained'}
        }
        Start-Sleep -Milliseconds 100
    }
    if(-not $taskApp.HasExited){throw 'Native saved-VNC deadline exceeded'}
    $taskReport=Get-Content $env:YUUTERM_CAPTURE_REPORT -Raw | ConvertFrom-Json
    if(-not $taskReport.passed -or -not(Test-Path $env:YUUTERM_CAPTURE)){throw 'Native saved-VNC report/capture missing'}
    [pscustomobject]@{Passed=$true;Checks=$taskReport.checks.Count;Folder=$taskFolder;Authentication=$Authentication}
} finally {
    if($taskApp -and -not $taskApp.HasExited){
        $null=$taskApp.CloseMainWindow()
        if(-not $taskApp.WaitForExit(5000)){$taskApp.Kill();$taskApp.WaitForExit()}
    }
    if($taskFixture -and -not $taskFixture.HasExited){$taskFixture.Kill();$taskFixture.WaitForExit()}
    foreach($taskName in $taskNames){[Environment]::SetEnvironmentVariable($taskName,$taskPrevious[$taskName],'Process')}
    Pop-Location
}
