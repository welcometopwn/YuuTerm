[CmdletBinding()]
param([switch]$SkipBuild,[ValidateSet('raw','tight','zrle')][string]$Encoding='raw',[ValidateSet('plain','vnc')][string]$Authentication='plain')
$ErrorActionPreference='Stop'
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
$securityCondition=New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::NameProperty,'Windows Security')
$root=Split-Path $PSScriptRoot -Parent
$folder=Join-Path $root ('target/vnc-native-'+(Get-Date -Format 'yyyyMMdd-HHmmss-fff'))
$names=@('YUUTERM_CAPTURE_PROFILE','YUUTERM_CAPTURE_SCOPE','YUUTERM_CAPTURE','YUUTERM_CAPTURE_REPORT','WEBVIEW2_USER_DATA_FOLDER','APPDATA','LOCALAPPDATA','YUUTERM_VNC_FIXTURE_ENCODING','YUUTERM_VNC_FIXTURE_AUTH')
$saved=@{};foreach($name in $names){$saved[$name]=[Environment]::GetEnvironmentVariable($name,'Process')}
$app=$null;$fixture=$null
Push-Location $root
try {
    if(-not $SkipBuild){
        & npm.cmd run build;if($LASTEXITCODE){throw 'Frontend build failed'}
        & cargo.exe build -p yuuterm-desktop --features capture --locked;if($LASTEXITCODE){throw 'Native build failed'}
        & cargo.exe build -p yuuterm-ssh --example vnc_fixture --locked;if($LASTEXITCODE){throw 'Fixture build failed'}
    }
    New-Item -ItemType Directory -Path $folder | Out-Null
    $env:YUUTERM_VNC_FIXTURE_ENCODING=$Encoding
    $env:YUUTERM_VNC_FIXTURE_AUTH=$Authentication
    $fixture=Start-Process (Join-Path $root 'target/debug/examples/vnc_fixture.exe') -ArgumentList ('"'+$folder+'"') -WindowStyle Hidden -PassThru -RedirectStandardError (Join-Path $folder 'fixture.err')
    $deadline=[DateTime]::UtcNow.AddSeconds(10)
    while(-not(Test-Path (Join-Path $folder 'profile.json')) -and [DateTime]::UtcNow -lt $deadline){Start-Sleep -Milliseconds 100}
    if(-not(Test-Path (Join-Path $folder 'profile.json'))){throw 'Fixture profile missing'}
    $profile=Get-Content (Join-Path $folder 'profile.json') -Raw | ConvertFrom-Json
    $expectedEndpoint='Sign in to 127.0.0.1:'+$profile.port
    $env:YUUTERM_CAPTURE_PROFILE=Join-Path $folder 'profile.json';$env:YUUTERM_CAPTURE_SCOPE='vnc'
    $env:YUUTERM_CAPTURE=Join-Path $folder 'screen.png';$env:YUUTERM_CAPTURE_REPORT=Join-Path $folder 'report.json'
    $env:WEBVIEW2_USER_DATA_FOLDER=Join-Path $folder 'WebView'
    $env:APPDATA=Join-Path $folder 'AppData';$env:LOCALAPPDATA=Join-Path $folder 'LocalAppData'
    $app=Start-Process (Join-Path $root 'target/debug/yuuterm-desktop.exe') -WindowStyle Hidden -PassThru -RedirectStandardError (Join-Path $folder 'app.err')
    $deadline=[DateTime]::UtcNow.AddSeconds(120)
    $answered=0
    $submitted=[Collections.Generic.HashSet[string]]::new()
    while(-not $app.HasExited -and [DateTime]::UtcNow -lt $deadline){
        # CredUI is hosted out of process, without a usable HWND owner chain.
        # Match the exact VNC caption and this run's ephemeral loopback endpoint
        # in the system broker. Windows can reuse an existing broker HWND.
        $windows=[System.Windows.Automation.AutomationElement]::RootElement.FindAll(
            [System.Windows.Automation.TreeScope]::Children,$securityCondition)
        foreach($dialog in $windows){
            $broker=Get-Process -Id $dialog.Current.ProcessId -ErrorAction SilentlyContinue
            if(-not $broker -or $broker.Path -ne (Join-Path $env:WINDIR 'System32/CredentialUIBroker.exe')){continue}
            $items=$dialog.FindAll([System.Windows.Automation.TreeScope]::Descendants,[System.Windows.Automation.Condition]::TrueCondition)
            $labels=@($items | ForEach-Object {$_.Current.Name})
            $matchesEndpoint=@($labels | Where-Object {$_-eq $expectedEndpoint})
            Write-Verbose ("Broker candidate: endpoint matches="+$matchesEndpoint.Count+" caption="+($labels -contains 'YuuTerm VNC sign-in'))
            if($labels -notcontains 'YuuTerm VNC sign-in' -or $matchesEndpoint.Count -ne 1){continue}
            $identity=$dialog.GetRuntimeId() -join ','
            if($submitted.Contains($identity)){continue}
            # Leave Remember me unchecked: these fixtures test one-shot sign-in.
            $edits=@($items | Where-Object {$_.Current.ControlType -eq [System.Windows.Automation.ControlType]::Edit})
            $passwords=@($edits | Where-Object {$_.Current.IsPassword})
            Write-Verbose ("Native fields="+$edits.Count+" password fields="+$passwords.Count+" runtime="+$identity)
            if($passwords.Count -ne 1){continue}
            foreach($edit in $edits){
                $value=if($edit.Current.IsPassword){'password'}elseif($Authentication -eq 'plain'){'fixture'}else{''}
                $edit.GetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern).SetValue($value)
            }
            $ok=@($items | Where-Object {$_.Current.ControlType -eq [System.Windows.Automation.ControlType]::Button -and $_.Current.Name -match '^(OK|Sign in|Connect)$'})
            if($ok.Count -ne 1){throw 'Native VNC submit button not found'}
            $ok[0].GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke()
            $null=$submitted.Add($identity)
            $answered++
            Write-Verbose ("Submitted native prompt "+$answered)
        }
        Start-Sleep -Milliseconds 100
    }
    if(-not $app.HasExited){throw 'Native VNC capture deadline exceeded'}
    $report=Get-Content (Join-Path $folder 'report.json') -Raw | ConvertFrom-Json
    if(-not $report.passed -or -not(Test-Path (Join-Path $folder 'screen.png'))){throw 'Native VNC validation failed; reports retained'}
    if($answered -ne 4){throw "Expected four native VNC prompts, answered $answered"}
    @{prompts=$answered;remember_checkbox=$true;passwords_saved=$false;renderer_secret_input=$false;authentication=$Authentication;empty_username=($Authentication -eq 'vnc')} | ConvertTo-Json | Set-Content (Join-Path $folder 'native-prompts.json') -Encoding utf8
    $deadline=[DateTime]::UtcNow.AddSeconds(2)
    while(-not(Test-Path (Join-Path $folder 'clipboard-result.json')) -and [DateTime]::UtcNow -lt $deadline){Start-Sleep -Milliseconds 50}
    if(-not(Test-Path (Join-Path $folder 'clipboard-result.json'))){throw 'Loopback peer did not receive the explicitly reviewed clipboard payload'}
    [pscustomobject]@{Passed=$true;Checks=$report.checks.Count;Folder=$folder;Encoding=$Encoding}
} finally {
    foreach($process in @($app,$fixture)){if($process -and -not $process.HasExited){$process.Kill();$process.WaitForExit()}}
    foreach($name in $names){[Environment]::SetEnvironmentVariable($name,$saved[$name],'Process')}
    Pop-Location
}
