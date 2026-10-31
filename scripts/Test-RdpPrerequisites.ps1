param([Parameter(Mandatory)][string]$OutputPath)
$ErrorActionPreference = 'Stop'
if (-not [Environment]::Is64BitProcess) { throw 'Run this probe with 64-bit PowerShell.' }
if ([Threading.Thread]::CurrentThread.ApartmentState -ne 'STA') {
    throw 'Run with pwsh -STA -NoProfile -File scripts/Test-RdpPrerequisites.ps1 -OutputPath <path>.'
}

# Read-only local capability inventory. Never sets a server, credentials,
# redirections, certificate policy or Connect; never enables Remote Desktop.
$dll = Join-Path ([Environment]::GetFolderPath('System')) 'mstscax.dll'
$file = Get-Item -LiteralPath $dll
$signature = Get-AuthenticodeSignature -LiteralPath $dll
$classes = @()
$registry = [Microsoft.Win32.Registry]::ClassesRoot.OpenSubKey('CLSID')
try {
    foreach ($id in $registry.GetSubKeyNames()) {
        $key = $registry.OpenSubKey($id)
        try {
            $name = [string]$key.GetValue('')
            if ($name -notmatch '^Microsoft RDP Client Control(?: \(redistributable\))? - version (\d+)$') { continue }
            $version = [int]$Matches[1]
            $server = $key.OpenSubKey('InprocServer32')
            if (-not $server) { continue }
            try { $path = [Environment]::ExpandEnvironmentVariables([string]$server.GetValue('')) }
            finally { $server.Dispose() }
            if (-not [string]::Equals($path.Trim('"'), $dll, [StringComparison]::OrdinalIgnoreCase)) { continue }
            $classes += [pscustomobject]@{ clsid = $id; name = $name; version = $version }
        } finally { $key.Dispose() }
    }
} finally { $registry.Dispose() }

$probes = @()
foreach ($entry in ($classes | Sort-Object version -Descending | Select-Object -First 4)) {
    $instance = $null
    $instantiated = $false
    try {
        $type = [Type]::GetTypeFromCLSID([Guid]$entry.clsid, $true)
        $instance = [Activator]::CreateInstance($type)
        $instantiated = $true
        $controlVersion = $instance.GetType().InvokeMember('Version', [Reflection.BindingFlags]::GetProperty, $null, $instance, $null)
        $probes += [pscustomobject]@{ clsid = $entry.clsid; class = $entry.name; instantiated = $true; control_version = [string]$controlVersion }
    } catch {
        # Do not dump COM exception payloads or ambient process configuration.
        $failure = $_.Exception
        while ($failure.InnerException) { $failure = $failure.InnerException }
        $probes += [pscustomobject]@{ clsid = $entry.clsid; class = $entry.name; instantiated = $instantiated; failed_stage = $(if ($instantiated) { 'Version property' } else { 'COM activation' }); hresult = $failure.HResult.ToString('X8') }
    } finally {
        if ($instance -and [Runtime.InteropServices.Marshal]::IsComObject($instance)) {
            [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($instance)
        }
    }
}
$result = [ordered]@{
    schema = 1
    scope = 'Read-only local RDP control prerequisites; no network connection or embedded surface test'
    dll = 'System32/mstscax.dll'
    dll_version = $file.VersionInfo.FileVersion
    signature_status = [string]$signature.Status
    registered_classes = $classes
    activation_probes = $probes
    not_tested = @('RDP connection/NLA/certificate validation', 'Tauri composition and modal occlusion', 'keyboard/pointer routing', 'resize/mixed DPI', 'clipboard/audio')
}
$fullPath = [IO.Path]::GetFullPath($OutputPath)
[IO.File]::WriteAllText($fullPath, ($result | ConvertTo-Json -Depth 6) + "`n", [Text.UTF8Encoding]::new($false))
Write-Output ('RDP prerequisite report written; ' + ($probes | Where-Object instantiated).Count + ' local control activations succeeded. No connection attempted.')
