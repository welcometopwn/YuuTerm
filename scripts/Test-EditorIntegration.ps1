param([string]$Editor = "$env:LOCALAPPDATA/Programs/Microsoft VS Code/Code.exe")
$ErrorActionPreference='Stop'
$repoRoot=Split-Path -Parent $PSScriptRoot
if(!(Test-Path -LiteralPath $Editor)){throw 'Installed VS Code executable not found'}
$runDir=Join-Path $repoRoot ('target/editor-'+(Get-Date -Format yyyyMMdd-HHmmss))
New-Item -ItemType Directory $runDir | Out-Null
$owned=[Collections.Generic.List[Diagnostics.Process]]::new()
try {
    Copy-Item "$repoRoot/target/debug/examples/ui_fixture.exe" "$runDir/fixture.exe"
    Copy-Item "$repoRoot/target/debug/yuuterm-desktop.exe" "$runDir/app.exe"
    $fixture=Start-Process "$runDir/fixture.exe" -ArgumentList $runDir -WindowStyle Hidden -PassThru -RedirectStandardError "$runDir/fixture.err"
    $owned.Add($fixture)
    for($n=0;$n -lt 100 -and !(Test-Path "$runDir/profile.json");$n++){Start-Sleep -Milliseconds 100}
    if(!(Test-Path "$runDir/profile.json")){throw 'Fixture did not create its profile'}
    $env:YUUTERM_CAPTURE_SCOPE='editor'
    $env:YUUTERM_CAPTURE_PROFILE="$runDir/profile.json"
    $env:YUUTERM_CAPTURE="$runDir/screen.png"
    $env:YUUTERM_CAPTURE_REPORT="$runDir/report.json"
    try {$app=Start-Process "$runDir/app.exe" -WindowStyle Hidden -PassThru -RedirectStandardError "$runDir/app.err"}
    finally {Remove-Item Env:YUUTERM_CAPTURE_SCOPE,Env:YUUTERM_CAPTURE_PROFILE,Env:YUUTERM_CAPTURE,Env:YUUTERM_CAPTURE_REPORT}
    $owned.Add($app)
    for($n=0;$n -lt 100 -and !(Test-Path "$runDir/editor-request.json");$n++){Start-Sleep -Milliseconds 100}
    if(!(Test-Path "$runDir/editor-request.json")){throw 'Native app did not request an editor test'}
    $env:YUUTERM_EDITOR_FIXTURE=$runDir
    $extension=Join-Path $repoRoot 'scripts/vscode-fixture'
    $arguments=@('--new-window','--skip-welcome','--skip-release-notes','--disable-extensions',"--user-data-dir=$runDir/editor-profile","--extensions-dir=$runDir/extensions","--extensionDevelopmentPath=$extension","--extensionTestsPath=$extension/tests.cjs")
    try {$code=Start-Process -FilePath $Editor -ArgumentList $arguments -WindowStyle Hidden -PassThru -RedirectStandardOutput "$runDir/editor.out" -RedirectStandardError "$runDir/editor.err"}
    finally {Remove-Item Env:YUUTERM_EDITOR_FIXTURE}
    $owned.Add($code)
    $deadline=[DateTime]::UtcNow.AddSeconds(120)
    $report=$null
    while([DateTime]::UtcNow -lt $deadline) {
        if(Test-Path "$runDir/report.json") {
            try {$report=Get-Content "$runDir/report.json" -Raw|ConvertFrom-Json} catch {$report=$null}
            if($null -ne $report){break}
        }
        if($app.HasExited){throw 'Native test exited without a report'}
        Start-Sleep -Milliseconds 150
    }
    if($null -eq $report){throw 'Editor test timed out; inspect retained reports'}
    if(!$report.passed){throw 'Editor test failed; inspect retained report.json'}
    if(!$app.WaitForExit(10000) -or !(Test-Path "$runDir/screen.png")){throw 'Native screenshot did not finish'}
    $null=$code.WaitForExit(5000)
    "Passed: installed-editor qualification. Evidence: $runDir"
} finally {
    foreach($child in $owned) {
        if(!$child.HasExited){$child.Kill();$null=$child.WaitForExit(5000)}
    }
    foreach($name in @('app.exe','fixture.exe')) {
        $file=[IO.Path]::GetFullPath((Join-Path $runDir $name))
        if(!$file.StartsWith([IO.Path]::GetFullPath($runDir)+[IO.Path]::DirectorySeparatorChar,[StringComparison]::OrdinalIgnoreCase)){throw 'Cleanup path escaped its test directory'}
        if(Test-Path -LiteralPath $file){Remove-Item -LiteralPath $file}
    }
}
