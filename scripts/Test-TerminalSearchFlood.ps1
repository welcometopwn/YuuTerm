# Instrumented native release, shared xterm renderer over real loopback TELNET.
# No SSH agent, user settings, external host or physical keyboard is involved.
[CmdletBinding()]
param([switch]$DebugBuild, [switch]$SkipBuild)
$ErrorActionPreference='Stop'
$root=Split-Path $PSScriptRoot -Parent
$folder=Join-Path $root ('target/search-flood-'+(Get-Date -Format 'yyyyMMdd-HHmmss-fff'))
$app=$null;$fixture=$null
$names=@('APPDATA','LOCALAPPDATA','WEBVIEW2_USER_DATA_FOLDER','YUUTERM_CAPTURE_PROFILE','YUUTERM_CAPTURE_SCOPE','YUUTERM_CAPTURE','YUUTERM_CAPTURE_REPORT')
$saved=@{};foreach($name in $names){$saved[$name]=[Environment]::GetEnvironmentVariable($name,'Process')}
$release=Join-Path $root 'target/release/yuuterm-desktop.exe'
$originalHash=if(Test-Path -LiteralPath $release){(Get-FileHash -LiteralPath $release).Hash}else{$null}
$previous=@{};$cpu=0.0;$peakPrivate=0L;$peakWorking=0L;$unreadable=0
$seen=[Collections.Generic.HashSet[int]]::new()
$samples=[Collections.Generic.List[object]]::new()
Push-Location $root
try {
    New-Item -ItemType Directory -Path $folder | Out-Null
    if(-not $SkipBuild){
        & npm.cmd run build;if($LASTEXITCODE){throw 'Frontend build failed'}
    }
    if($DebugBuild){
        if(-not $SkipBuild){
            & cargo.exe build -p yuuterm-desktop --features capture --locked;if($LASTEXITCODE){throw 'Debug capture build failed'}
            & cargo.exe build -p yuuterm-ssh --example telnet_fixture --locked;if($LASTEXITCODE){throw 'Debug loopback fixture build failed'}
        }
        Copy-Item (Join-Path $root 'target/debug/yuuterm-desktop.exe') (Join-Path $folder 'app.exe')
        Copy-Item (Join-Path $root 'target/debug/examples/telnet_fixture.exe') (Join-Path $folder 'fixture.exe')
    }else{
        # Link to an isolated executable: never replace the canonical release.
        & cargo.exe rustc --release -p yuuterm-desktop --features capture --locked -- ('--emit=link='+(Join-Path $folder 'app.exe'))
        if($LASTEXITCODE){throw 'Instrumented release build failed'}
        & cargo.exe build --release -p yuuterm-ssh --example telnet_fixture --locked
        if($LASTEXITCODE){throw 'Release loopback fixture build failed'}
        Copy-Item (Join-Path $root 'target/release/examples/telnet_fixture.exe') (Join-Path $folder 'fixture.exe')
    }
    $fixture=Start-Process (Join-Path $folder 'fixture.exe') -ArgumentList ('"'+$folder+'" --search-flood') -WindowStyle Hidden -PassThru -RedirectStandardError (Join-Path $folder 'fixture.err')
    $deadline=[DateTime]::UtcNow.AddSeconds(10)
    while(-not(Test-Path (Join-Path $folder 'profile.json')) -and [DateTime]::UtcNow -lt $deadline){Start-Sleep -Milliseconds 50}
    if(-not(Test-Path (Join-Path $folder 'profile.json'))){throw 'Loopback fixture profile deadline exceeded'}
    $env:APPDATA=Join-Path $folder 'Roaming';$env:LOCALAPPDATA=Join-Path $folder 'Local'
    $env:WEBVIEW2_USER_DATA_FOLDER=Join-Path $folder 'WebView'
    $env:YUUTERM_CAPTURE_PROFILE=Join-Path $folder 'profile.json';$env:YUUTERM_CAPTURE_SCOPE='search-flood'
    $env:YUUTERM_CAPTURE=Join-Path $folder 'screen.png';$env:YUUTERM_CAPTURE_REPORT=Join-Path $folder 'report.json'
    $app=Start-Process (Join-Path $folder 'app.exe') -WindowStyle Hidden -PassThru -RedirectStandardError (Join-Path $folder 'app.err')
    $null=$app.Handle
    $watch=[Diagnostics.Stopwatch]::StartNew()
    while(-not $app.HasExited -and $watch.Elapsed.TotalSeconds -lt 90){
        $tree=[Collections.Generic.HashSet[int]]::new();$null=$tree.Add($app.Id)
        $processes=@(Get-CimInstance Win32_Process -Property ProcessId,ParentProcessId)
        do {
            $added=$false
            foreach($p in $processes){if($tree.Contains([int]$p.ParentProcessId) -and $tree.Add([int]$p.ProcessId)){$added=$true}}
        }while($added)
        $private=0L;$working=0L
        foreach($id in $tree){
            $p=Get-Process -Id $id -ErrorAction SilentlyContinue
            if(-not $p){continue}
            try {
                $null=$seen.Add($id)
                $seconds=$p.TotalProcessorTime.TotalSeconds
                $prior=if($previous.ContainsKey($id)){$previous[$id]}else{0.0}
                $cpu+=[Math]::Max(0.0, $seconds - $prior)
                $previous[$id]=$seconds
                $private+=$p.PrivateMemorySize64;$working+=$p.WorkingSet64
            }catch{$unreadable++}finally{$p.Dispose()}
        }
        $peakPrivate=[Math]::Max($peakPrivate,$private);$peakWorking=[Math]::Max($peakWorking,$working)
        $samples.Add(@{elapsed_ms=$watch.ElapsedMilliseconds;sampled_cpu_seconds=$cpu;private_bytes=$private;working_bytes=$working;members=$tree.Count})
        if(Test-Path $env:YUUTERM_CAPTURE_REPORT){
            $report=Get-Content $env:YUUTERM_CAPTURE_REPORT -Raw | ConvertFrom-Json
            if(-not $report.passed){throw ('Native flood checks failed: '+$report.checks[-1])}
        }
        Start-Sleep -Milliseconds 500
    }
    if(-not $app.HasExited){throw 'Native search-flood deadline exceeded'}
    $app.WaitForExit()
    $report=Get-Content $env:YUUTERM_CAPTURE_REPORT -Raw | ConvertFrom-Json
    if(-not $report.passed){throw ('Native flood checks failed: '+$report.checks[-1])}
    if($app.ExitCode -ne 0 -or -not $report.passed -or -not(Test-Path $env:YUUTERM_CAPTURE)){throw 'Native report, exit or capture failed'}
    if($cpu -le 0 -or $seen.Count -lt 2){throw 'CPU/helper sampling failed; no performance result accepted'}
    $measurement=@{
        passed=$true;build=$(if($DebugBuild){'debug/capture'}else{'release/capture'});connections=4
        checks=$report.checks.Count;sampled_client_tree_cpu_seconds=$cpu;peak_client_tree_private_bytes=$peakPrivate
        peak_client_tree_working_bytes=$peakWorking;observed_client_processes=$seen.Count;unreadable_process_samples=$unreadable
        samples=$samples.ToArray();fixture_cpu_seconds=$fixture.TotalProcessorTime.TotalSeconds
        limits='Includes app/WebView helpers; sampled CPU can miss exited helpers. Server reported separately. Host monitoring excluded. Paced synthetic loopback output, synthetic DOM input, synchronous search timers; not physical input-to-paint, idle or maximum throughput. No before/after speedup claim.'
    }
    $measurement | ConvertTo-Json -Depth 6 | Set-Content (Join-Path $folder 'measurement.json') -Encoding utf8
    [pscustomobject]@{Passed=$true;Checks=$report.checks.Count;Build=$measurement.build;Folder=$folder;ClientProcesses=$seen.Count}
}finally{
    foreach($process in @($app,$fixture)){if($process -and -not $process.HasExited){$process.Kill();$null=$process.WaitForExit(5000)}}
    foreach($name in $names){[Environment]::SetEnvironmentVariable($name,$saved[$name],'Process')}
    # Only these own generated executables/PDBs are disposable. Reports remain.
    foreach($name in @('app.exe','app.pdb','fixture.exe','fixture.pdb')){
        $path=Join-Path $folder $name
        if(Test-Path -LiteralPath $path){Remove-Item -LiteralPath $path -Force}
    }
    if($originalHash -and (Get-FileHash -LiteralPath $release).Hash -ne $originalHash){throw 'Canonical release changed during the instrumented fixture'}
    Pop-Location
}
