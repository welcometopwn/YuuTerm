# Policy tests for the read-only Windows inventory, using synthetic process/socket
# records. This does not establish compatibility with an installed X server.
$ErrorActionPreference = 'Stop'
$inspect = [scriptblock]::Create([IO.File]::ReadAllText((Join-Path $PSScriptRoot 'Inspect-X11.ps1')))
$oldDisplay = $env:DISPLAY
$oldAuthority = $env:XAUTHORITY
function Get-AppxPackage { @() }
function Test-Path { param($LiteralPath) $script:installed }
function Get-CimInstance { $script:process }
function Get-NetTCPConnection { $script:listeners }
try {
    $env:DISPLAY = 'localhost:0'
    $env:XAUTHORITY = 'C:/synthetic/authority'
    $script:installed = $false
    $script:process = @()
    $script:listeners = @()
    $checks = 0
    function Check([bool]$condition) { if (-not $condition) { throw "X11 policy check $script:checks failed" }; $script:checks++ }
    $result = (& $inspect | ConvertFrom-Json)
    Check ($result.installed.Count -eq 0 -and $result.candidates.Count -eq 0)
    $script:installed = $true
    $script:process = @{Name='vcxsrv.exe'; ProcessId=123; CommandLine='vcxsrv.exe :0 -noclipboard -auth "C:/synthetic/cookie file"'}
    $script:listeners = @(@{LocalPort=6000;LocalAddress='127.0.0.1'})
    $result = (& $inspect | ConvertFrom-Json)
    Check ($result.installed -contains 'VcXsrv' -and $result.candidates.Count -eq 1)
    Check ($result.candidates[0].authority -eq 'C:/synthetic/cookie file')
    foreach ($address in @('0.0.0.0','::','192.0.2.1')) {
        $script:listeners = @(@{LocalPort=6000;LocalAddress='127.0.0.1'},@{LocalPort=6000;LocalAddress=$address})
        Check ((& $inspect | ConvertFrom-Json).candidates.Count -eq 0)
    }
    $script:listeners = @(@{LocalPort=6000;LocalAddress='::1'})
    Check ((& $inspect | ConvertFrom-Json).candidates[0].ipv6 -eq $true)
    foreach ($arguments in @('-noclipboard -ac','-clipboard','', '-noclipboard -clipboard','-auth "C:/ -noclipboard fake"','-auth -noclipboard','-NOCLIPBOARD')) {
        $script:process.CommandLine = 'vcxsrv.exe :0 '+$arguments
        Check ((& $inspect | ConvertFrom-Json).candidates.Count -eq 0)
    }
    $script:process.Name = 'X410.exe'
    $script:process.CommandLine = 'X410.exe -noclipboard'
    Check ((& $inspect | ConvertFrom-Json).candidates.Count -eq 0)
    $env:DISPLAY = '192.0.2.1:0'
    $refused = $false
    try { & $inspect | Out-Null } catch { $refused = $true }
    Check $refused
    @{passed=$true;checks=$checks;qualification='Synthetic Windows process/listener policy records only'} | ConvertTo-Json
} finally { $env:DISPLAY=$oldDisplay; $env:XAUTHORITY=$oldAuthority }
