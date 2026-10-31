# Embedded by the X11 discovery worker. Read-only; never starts an X server.
$ErrorActionPreference = 'Stop'
function Read-X11Arguments([string]$commandLine) {
    # Deliberately accept only simple Windows argv forms. Embedded/escaped quote
    # forms are refused rather than confusing a path with an executable option.
    if ($commandLine -match '\\"' -or $commandLine -notmatch '^\s*(?:"[^"\r\n]*"|[^\s"]+)(?:\s+(?:"[^"\r\n]*"|[^\s"]+))*\s*$') { return @() }
    @([regex]::Matches($commandLine, '"[^"]*"|[^\s"]+') | ForEach-Object { $_.Value.Trim('"') })
}
$known = @()
foreach ($base in @($env:ProgramFiles, ${env:ProgramFiles(x86)}, "$env:LOCALAPPDATA/Programs")) {
    if ($base -and (Test-Path -LiteralPath (Join-Path $base 'VcXsrv/vcxsrv.exe'))) { $known += 'VcXsrv' }
}
if (Get-AppxPackage -Name '*X410*' -ErrorAction SilentlyContinue | Select-Object -First 1) { $known += 'X410' }
$display = $env:DISPLAY
if (-not $display) { $display = 'localhost:0' }
if ($display -notmatch '^(?:localhost|127\.0\.0\.1)?:([0-9]{1,3})(?:\.0)?$') { throw 'DISPLAY must select a local display 0 through 999' }
$number = [int]$Matches[1]
$port = 6000 + $number
$candidates = @()
foreach ($process in @(Get-CimInstance Win32_Process -Filter "Name='vcxsrv.exe' OR Name='X410.exe'")) {
    $name = if ($process.Name -ieq 'vcxsrv.exe') { 'VcXsrv' } else { 'X410' }
    $known += $name
    $arguments = @(Read-X11Arguments $process.CommandLine)
    $authority = $env:XAUTHORITY
    $options = @()
    for ($i=1; $i -lt $arguments.Count; $i++) {
        if ($arguments[$i] -ceq '-auth') {
            if (++$i -ge $arguments.Count) { $options = @(); break }
            $authority = $arguments[$i]
        } else { $options += $arguments[$i] }
    }
    # Remote clipboard reads stay off. X410 detection alone cannot establish its
    # clipboard/authentication policy, so only this reviewed VcXsrv mode is usable.
    if ($name -ne 'VcXsrv' -or $options -cnotcontains '-noclipboard' -or
        $options -ccontains '-clipboard' -or $options -ccontains '-ac') { continue }
    # A listener owned by this process must exist, with no wildcard/LAN bind.
    $listeners = @(Get-NetTCPConnection -State Listen -OwningProcess $process.ProcessId -ErrorAction SilentlyContinue)
    $selected = @($listeners | Where-Object { $_.LocalPort -eq $port -and $_.LocalAddress -in @('127.0.0.1','::1') })
    $external = @($listeners | Where-Object {
        $address = [Net.IPAddress]::Parse($_.LocalAddress)
        -not [Net.IPAddress]::IsLoopback($address)
    })
    if ($selected.Count -eq 0 -or $external.Count -ne 0) { continue }
    if (-not $authority) { $authority = Join-Path $env:USERPROFILE '.Xauthority' }
    $ipv4 = @($selected | Where-Object LocalAddress -eq '127.0.0.1').Count -gt 0
    $candidates += @{ server=$name; port=$port; ipv6=(-not $ipv4); authority=$authority; display=$number }
}
@{ installed=@($known | Select-Object -Unique); candidates=$candidates; hostname=$env:COMPUTERNAME } | ConvertTo-Json -Depth 4 -Compress
