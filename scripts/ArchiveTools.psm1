function Invoke-ArchiveProcess {
    param(
        [Parameter(Mandatory)][string]$FilePath,
        [Parameter(Mandatory)][string[]]$ArgumentList,
        [ValidateRange(1, 300)][int]$TimeoutSeconds = 60
    )
    $start = [Diagnostics.ProcessStartInfo]::new()
    $start.FileName = $FilePath
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.RedirectStandardInput = $true
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    foreach ($argument in $ArgumentList) { $start.ArgumentList.Add($argument) }
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $start
    try {
        $null = $process.Start()
        $process.StandardInput.Close()
        $stdout = $process.StandardOutput.ReadToEndAsync()
        $stderr = $process.StandardError.ReadToEndAsync()
        if (-not $process.WaitForExit($TimeoutSeconds * 1000)) {
            $process.Kill($true)
            $null = $process.WaitForExit(5000)
            throw "Archive process timed out after $TimeoutSeconds seconds"
        }
        $output = $stdout.GetAwaiter().GetResult()
        $errors = $stderr.GetAwaiter().GetResult()
        if ($process.ExitCode -ne 0) {
            throw "Archive process failed (exit $($process.ExitCode)): $($errors.Trim())"
        }
        return $output
    } finally { $process.Dispose() }
}

function Expand-PinnedArchive {
    param(
        [Parameter(Mandatory)][string]$Archive,
        [Parameter(Mandatory)][string]$Directory,
        [string]$PythonPath = (Get-Command python.exe -ErrorAction Stop).Source,
        [ValidateRange(1, 300)][int]$TimeoutSeconds = 60
    )
    $name = Split-Path $Archive -Leaf
    Write-Host "[X11] Extracting regular files: $name"
    $arguments = @((Join-Path $PSScriptRoot 'extract-pinned-archive.py'), $Archive, $Directory)
    $output = Invoke-ArchiveProcess -FilePath $PythonPath -ArgumentList $arguments -TimeoutSeconds $TimeoutSeconds
    $links = @($output | ConvertFrom-Json)
    Write-Host "[X11] Extracted: $name"
    return $links
}

Export-ModuleMember -Function Invoke-ArchiveProcess, Expand-PinnedArchive
