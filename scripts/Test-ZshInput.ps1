# External zsh input repro. Uses an already trusted OpenSSH alias and agent;
# no remote shell configuration edits. Private host/prompt capture stays in target.
param([Parameter(Mandatory=$true)][string]$SshAlias)
$ErrorActionPreference='Stop'
$root=Split-Path $PSScriptRoot -Parent
$folder=Join-Path $root ('target/zsh-input-'+(Get-Date -Format 'yyyyMMdd-HHmmss-fff'))
New-Item -ItemType Directory -Path $folder | Out-Null
$options=@{}
& ssh.exe -G $SshAlias | ForEach-Object {
    if($_ -match '^(hostname|user|port) (.+)$') {$options[$Matches[1]]=$Matches[2]}
}
if($LASTEXITCODE){throw 'Cannot resolve SSH alias'}
$known=& ssh-keygen.exe -F $options.hostname | Where-Object {$_ -match '^[^#].* ssh-ed25519 '}
if(-not $known){throw 'An existing trusted Ed25519 host key is required'}
$parts=($known | Select-Object -First 1) -split '\s+'
$hosts=@{};$hosts["[$($options.hostname.ToLowerInvariant())]:$($options.port)"]=$parts[1]+' '+$parts[2]
[IO.File]::WriteAllText((Join-Path $folder 'host-keys.json'),(@{version=1;hosts=$hosts}|ConvertTo-Json))
$profile=@{id=1;name='External zsh input qualification';protocol='Ssh';host=$options.hostname;port=[int]$options.port;username=$options.user;statistics_enabled=$false;ssh=@{auth='WindowsAgent';x11_forward=$false};logging=@{enabled=$false;file=''}}
[IO.File]::WriteAllText((Join-Path $folder 'profile.json'),($profile|ConvertTo-Json -Depth 8))
$names=@('APPDATA','LOCALAPPDATA','WEBVIEW2_USER_DATA_FOLDER','YUUTERM_CAPTURE_SCOPE','YUUTERM_CAPTURE_PROFILE','YUUTERM_CAPTURE','YUUTERM_CAPTURE_REPORT','YUUTERM_ZSH_HOST','YUUTERM_ZSH_USER')
$saved=@{};foreach($name in $names){$saved[$name]=[Environment]::GetEnvironmentVariable($name,'Process')}
$app=$null
try {
    $env:APPDATA=Join-Path $folder 'Roaming';$env:LOCALAPPDATA=Join-Path $folder 'Local'
    $env:WEBVIEW2_USER_DATA_FOLDER=Join-Path $folder 'WebView'
    $env:YUUTERM_CAPTURE_SCOPE='zsh-input';$env:YUUTERM_CAPTURE_PROFILE=Join-Path $folder 'profile.json'
    $env:YUUTERM_CAPTURE=Join-Path $folder 'screen.png';$env:YUUTERM_CAPTURE_REPORT=Join-Path $folder 'report.json'
    $env:YUUTERM_ZSH_HOST=$options.hostname;$env:YUUTERM_ZSH_USER=$options.user
    $app=Start-Process (Join-Path $root 'target/debug/yuuterm-desktop.exe') -WindowStyle Hidden -PassThru -RedirectStandardError (Join-Path $folder 'app.err')
    $null=$app.Handle
    if(-not $app.WaitForExit(45000)){throw 'zsh input capture deadline exceeded'}
    $report=Get-Content $env:YUUTERM_CAPTURE_REPORT -Raw | ConvertFrom-Json
    $report | ConvertTo-Json -Depth 4
    if(-not $report.passed){throw 'zsh input regression reproduced; private capture retained in target'}
} finally {
    if($app -and -not $app.HasExited){$app.Kill();$null=$app.WaitForExit(5000)}
    foreach($name in $names){[Environment]::SetEnvironmentVariable($name,$saved[$name],'Process')}
}
