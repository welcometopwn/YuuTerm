param([string]$ApplicationPath, [switch]$SkipInstaller)
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$work = Join-Path $root ('target/single-instance-' + (Get-Date -Format 'yyyyMMdd-HHmmss-fff'))
$checks = [Collections.Generic.List[string]]::new()
$owned = [Collections.Generic.List[Diagnostics.Process]]::new()
$junctions = [Collections.Generic.List[string]]::new()
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class InstanceWindows {
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int command);
    [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr h);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint process);
}
'@
function Check([bool]$ok, [string]$name) {
    if (-not $ok) { throw "Single-instance check failed: $name" }
    $checks.Add($name)
}
function Launch([string]$exe, [string]$local) {
    $saved = @{}
    try {
        foreach ($name in @('LOCALAPPDATA','APPDATA','WEBVIEW2_USER_DATA_FOLDER')) {
            $saved[$name] = [Environment]::GetEnvironmentVariable($name)
        }
        $env:LOCALAPPDATA = $local
        $env:APPDATA = Join-Path $local 'roaming'
        # Let portable mode choose WebView2 beside its EXE. Explicitly isolate
        # the regular WebView too (Windows known folders can ignore env overrides).
        $env:WEBVIEW2_USER_DATA_FOLDER = if (Test-Path (Join-Path (Split-Path $exe -Parent) 'yuuterm.portable')) { $null } else { Join-Path $local 'WebView2' }
        $process = Start-Process $exe -PassThru
        $owned.Add($process)
        return $process
    } finally {
        foreach ($name in $saved.Keys) { [Environment]::SetEnvironmentVariable($name, $saved[$name]) }
    }
}
function Wait-Window([Diagnostics.Process]$process) {
    $deadline = [DateTime]::UtcNow.AddSeconds(30)
    do {
        Start-Sleep -Milliseconds 100
        $process.Refresh()
    } while (-not $process.HasExited -and $process.MainWindowHandle -eq 0 -and [DateTime]::UtcNow -lt $deadline)
    Check (-not $process.HasExited -and $process.MainWindowHandle -ne 0 -and $process.Responding) 'real release process opens a responsive native window'
}
function Duplicate([string]$exe, [string]$local, [Diagnostics.Process]$primary) {
    $null = [InstanceWindows]::ShowWindow($primary.MainWindowHandle, 6)
    Start-Sleep -Milliseconds 200
    Check ([InstanceWindows]::IsIconic($primary.MainWindowHandle)) 'primary window is minimized before duplicate launch'
    $second = Launch $exe $local
    Check ($second.WaitForExit(12000) -and $second.ExitCode -eq 0) 'second launch exits successfully without another backend'
    $deadline = [DateTime]::UtcNow.AddSeconds(5)
    while (([InstanceWindows]::IsIconic($primary.MainWindowHandle) -or [InstanceWindows]::GetForegroundWindow() -ne $primary.MainWindowHandle) -and [DateTime]::UtcNow -lt $deadline) { Start-Sleep -Milliseconds 50 }
    Check (-not $primary.HasExited -and -not [InstanceWindows]::IsIconic($primary.MainWindowHandle)) 'second launch restores the original window'
    $foreground = [InstanceWindows]::GetForegroundWindow()
    $foregroundPid = [uint32]0
    $null = [InstanceWindows]::GetWindowThreadProcessId($foreground, [ref]$foregroundPid)
    $foregroundName = if ($foregroundPid) { (Get-Process -Id $foregroundPid).ProcessName } else { 'none' }
    if ($foreground -ne $primary.MainWindowHandle) { throw "Foreground mismatch: expected=$($primary.MainWindowHandle), actual=$foreground, process=$foregroundName" }
    Check $true 'second launch focuses the original native window'
}
function Close-Owned([Diagnostics.Process]$process) {
    Check ($process.CloseMainWindow() -and $process.WaitForExit(15000)) 'primary exits by normal window close'
}
Push-Location $root
try {
    New-Item -ItemType Directory -Path "$work/release" | Out-Null
    $source = if ($ApplicationPath) { (Resolve-Path -LiteralPath $ApplicationPath).Path } else { Join-Path $root 'target/release/yuuterm-desktop.exe' }
    $hash = (Get-FileHash $source).Hash
    $regular = Join-Path $work 'release/yuuterm-desktop.exe'
    Copy-Item $source $regular
    $local = Join-Path $work 'local'
    # Exercise the startup race, not just an already initialized window.
    $a = Launch $regular $local
    $b = Launch $regular $local
    $deadline = [DateTime]::UtcNow.AddSeconds(30)
    do { Start-Sleep -Milliseconds 100; $a.Refresh(); $b.Refresh() } while (-not $a.HasExited -and -not $b.HasExited -and [DateTime]::UtcNow -lt $deadline)
    Check ($a.HasExited -xor $b.HasExited) 'simultaneous startup admits exactly one process'
    $primary = if ($a.HasExited) { $b } else { $a }
    $secondary = if ($a.HasExited) { $a } else { $b }
    Check ($secondary.ExitCode -eq 0) 'startup-race secondary exits successfully'
    Wait-Window $primary
    Duplicate $regular $local $primary
    $settings = Join-Path $local 'YuuTerm/settings.json'
    [IO.File]::WriteAllText((Join-Path $local 'YuuTerm/sentinel.txt'), 'unchanged')
    # A copied executable still resolves the same settings folder.
    New-Item -ItemType Directory "$work/copy" | Out-Null
    Copy-Item $regular "$work/copy/yuuterm-desktop.exe"
    Duplicate "$work/copy/yuuterm-desktop.exe" $local $primary
    Check ([IO.File]::ReadAllText((Join-Path $local 'YuuTerm/sentinel.txt')) -eq 'unchanged') 'duplicate launches preserve existing data'
    $alias = Join-Path $work 'local-alias'
    $null = New-Item -ItemType Junction -Path $alias -Target $local
    $junctions.Add($alias)
    Duplicate $regular $alias $primary
    Check (-not $primary.HasExited) 'directory junction aliases cannot bypass ownership'
    # Build and extract a real portable ZIP from the same release bytes.
    New-Item -ItemType Directory "$work/package/YuuTerm" | Out-Null
    Copy-Item $source "$work/package/YuuTerm/yuuterm-desktop.exe"
    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
    $vs = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
    $crt = Get-ChildItem (Join-Path $vs 'VC/Redist/MSVC') -Directory | Where-Object Name -Match '^\d+\.\d+\.\d+$' | Sort-Object { [version]$_.Name } -Descending | ForEach-Object { Join-Path $_.FullName 'x64/Microsoft.VC143.CRT' } | Where-Object { Test-Path $_ } | Select-Object -First 1
    if (-not $crt) { throw 'Visual C++ runtime redistributable is missing' }
    Copy-Item "$crt/*.dll" "$work/package/YuuTerm"
    [IO.File]::WriteAllText("$work/package/YuuTerm/yuuterm.portable", 'Fixture portable marker')
    Compress-Archive "$work/package/YuuTerm" "$work/portable.zip"
    Expand-Archive "$work/portable.zip" "$work/extracted"
    $portable = "$work/extracted/YuuTerm/yuuterm-desktop.exe"
    $portableProcess = Launch $portable $local
    Wait-Window $portableProcess
    Check (-not $primary.HasExited) 'portable and regular data folders can run side by side'
    Duplicate $portable $local $portableProcess
    Check (Test-Path "$work/extracted/YuuTerm/data/instance.lock") 'portable lease lives beside executable'
    Check (Test-Path "$work/extracted/YuuTerm/data/WebView2") 'portable WebView uses its own data folder'
    Expand-Archive "$work/portable.zip" "$work/extracted-two"
    $portableTwo = Launch "$work/extracted-two/YuuTerm/yuuterm-desktop.exe" $local
    Wait-Window $portableTwo
    Check (-not $portableProcess.HasExited -and -not $primary.HasExited) 'two portable folders and regular mode can run concurrently'
    Close-Owned $portableTwo
    Close-Owned $portableProcess
    # Crash releases OS ownership; stale record never blocks the next launch.
    $primary.Kill()
    Check ($primary.WaitForExit(15000)) 'fixture-only forced termination completes'
    $recovered = Launch $regular $local
    Wait-Window $recovered
    Duplicate $regular $local $recovered
    Close-Owned $recovered
    if (-not $SkipInstaller) {
        & powershell.exe -NoProfile -ExecutionPolicy Bypass -File "$PSScriptRoot/Test-Installer.ps1" -ApplicationPath $source
        Check ($LASTEXITCODE -eq 0) 'installed package two-launch and maintenance checks pass'
    }
    Check ((Get-FileHash $source).Hash -eq $hash) 'source release executable remains unchanged'
    $qualification = 'Exact release bytes in isolated regular layout and extracted portable ZIP. No real profiles or remote sessions. Foreground activation verified on current interactive Windows desktop.'
    if (-not $SkipInstaller) { $qualification += ' Current-user installer fixture also tested.' }
    @{passed=$true;application_sha256=$hash;checks=$checks;installer_tested=(-not $SkipInstaller);qualification=$qualification} | ConvertTo-Json -Depth 5 | Set-Content "$work/report.json" -Encoding UTF8
    [pscustomobject]@{Passed=$true;Checks=$checks.Count;Folder=$work}
} catch {
    @{passed=$false;checks=$checks;error=$_.Exception.Message} | ConvertTo-Json -Depth 5 | Set-Content "$work/report.json" -Encoding UTF8
    throw
} finally {
    foreach ($process in $owned) {
        if (-not $process.HasExited) {
            $null = $process.CloseMainWindow()
            if (-not $process.WaitForExit(15000)) { $process.Kill(); $null = $process.WaitForExit(15000) }
        }
    }
    foreach ($junction in $junctions) {
        if ((Get-Item -LiteralPath $junction -Force).Attributes -band [IO.FileAttributes]::ReparsePoint) {
            # Remove just the junction, never recursively traverse its target.
            [IO.Directory]::Delete($junction)
        }
    }
    Pop-Location
}
