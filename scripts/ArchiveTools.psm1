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
        [string]$TarPath = "$env:SystemRoot/System32/tar.exe",
        [ValidateRange(1, 300)][int]$TimeoutSeconds = 60
    )
    $name = Split-Path $Archive -Leaf
    Write-Host "[X11] Listing: $name"
    $listing = Invoke-ArchiveProcess -FilePath $TarPath -ArgumentList @('-tvf', $Archive) -TimeoutSeconds $TimeoutSeconds
    $links = @($listing -split '\r?\n' | ForEach-Object {
        if ($_ -match '\s(?<name>\S+) (?<kind>->|link to) (?<target>\S+)$') {
            [pscustomobject]@{Name=$Matches.name;Target=$Matches.target;HardLink=($Matches.kind -eq 'link to')}
        }
    })
    $arguments = @('-xf', $Archive, '-C', $Directory, '--no-same-owner', '--no-same-permissions')
    foreach ($link in $links) { $arguments += @('--exclude', $link.Name) }
    Write-Host "[X11] Extracting regular files: $name ($($links.Count) links deferred)"
    $null = Invoke-ArchiveProcess -FilePath $TarPath -ArgumentList $arguments -TimeoutSeconds $TimeoutSeconds
    Write-Host "[X11] Extracted: $name"
    return $links
}

Export-ModuleMember -Function Invoke-ArchiveProcess, Expand-PinnedArchive
