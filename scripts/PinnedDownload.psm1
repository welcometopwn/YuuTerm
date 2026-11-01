function Get-PinnedFile {
    param(
        [Parameter(Mandatory)][string]$Mirror,
        [Parameter(Mandatory)][object]$Item,
        [Parameter(Mandatory)][string]$Directory,
        [ValidateRange(1, 1800)][int]$TimeoutSeconds = 180,
        [ValidateRange(1, 120)][int]$ConnectTimeoutSeconds = 20,
        [ValidateRange(0, 5)][int]$Retries = 2
    )
    $ErrorActionPreference = 'Stop'
    New-Item -ItemType Directory -Force $Directory | Out-Null
    $file = Join-Path $Directory (Split-Path $Item.path -Leaf)
    $name = Split-Path $file -Leaf
    if ((Test-Path -LiteralPath $file -PathType Leaf) -and (Get-FileHash -LiteralPath $file -Algorithm SHA512).Hash -eq $Item.sha512) {
        Write-Host "[X11] Cache hit: $name"
        return $file
    }
    $partial = "$file.partial"
    $curl = (Get-Command curl.exe -ErrorAction Stop).Source
    $watch = [Diagnostics.Stopwatch]::StartNew()
    $retryWindow = ($Retries + 1) * $TimeoutSeconds + $Retries * 2
    Write-Host "[X11] Download: $name (timeout ${TimeoutSeconds}s per attempt, $Retries retries)"
    try {
        & $curl --fail --location --silent --show-error --connect-timeout $ConnectTimeoutSeconds --max-time $TimeoutSeconds --retry $Retries --retry-all-errors --retry-delay 2 --retry-max-time $retryWindow --output $partial "$($Mirror.TrimEnd('/'))/$($Item.path)"
        if ($LASTEXITCODE -ne 0) { throw "Download failed for $name (curl exit $LASTEXITCODE)" }
        if ((Get-FileHash -LiteralPath $partial -Algorithm SHA512).Hash -ne $Item.sha512) {
            throw "SHA-512 mismatch for $($Item.path)"
        }
        Move-Item -LiteralPath $partial -Destination $file -Force
        $seconds = $watch.Elapsed.TotalSeconds.ToString('F1', [Globalization.CultureInfo]::InvariantCulture)
        Write-Host "[X11] Verified: $name ($seconds seconds)"
        return $file
    } finally {
        if (Test-Path -LiteralPath $partial -PathType Leaf) { Remove-Item -LiteralPath $partial -Force }
    }
}
Export-ModuleMember -Function Get-PinnedFile
