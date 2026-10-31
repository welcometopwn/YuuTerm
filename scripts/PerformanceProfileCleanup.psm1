function Clear-OwnedPerformanceWebView {
    param([Parameter(Mandatory)][string]$Root, [Parameter(Mandatory)][string]$Run)
    $workspace = Split-Path $PSScriptRoot -Parent
    $target = [IO.Path]::GetFullPath((Join-Path $workspace 'target'))
    $rootItem = Get-Item -LiteralPath $Root -ErrorAction Stop
    $runItem = Get-Item -LiteralPath $Run -ErrorAction Stop
    if ($rootItem.Parent.FullName -ne $target -or $rootItem.Name -notmatch '^performance-\d{8}-\d{6}-\d{3}$' -or
        $runItem.Parent.FullName -ne $rootItem.FullName -or $runItem.Name -notmatch '^run-[1-5]-(1|10|20)$' -or
        ($rootItem.Attributes -band [IO.FileAttributes]::ReparsePoint) -or ($runItem.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
        throw 'Refusing cache cleanup outside an owned performance run'
    }
    $cache = Join-Path $runItem.FullName 'webview'
    if (-not (Test-Path -LiteralPath $cache)) { return $true }
    $cacheItem = Get-Item -LiteralPath $cache -ErrorAction Stop
    $full = [IO.Path]::GetFullPath($cacheItem.FullName)
    $expected = [IO.Path]::GetFullPath((Join-Path $runItem.FullName 'webview'))
    if ($full -ne $expected -or -not $full.StartsWith($rootItem.FullName + [IO.Path]::DirectorySeparatorChar,[StringComparison]::OrdinalIgnoreCase) -or
        ($cacheItem.Attributes -band [IO.FileAttributes]::ReparsePoint) -or
        @(Get-ChildItem -LiteralPath $full -Recurse -Force -Attributes ReparsePoint -ErrorAction Stop).Count -gt 0) {
        throw 'Refusing cache cleanup through a reparse point or unexpected path'
    }
    # One native PowerShell operation against the verified exact cache directory.
    # Never touch run roots, reports, profile metadata, recovery files or Cargo.
    try { Remove-Item -LiteralPath $full -Recurse -Force -ErrorAction Stop }
    catch { return $false } # A late WebView helper may still have files locked.
    return -not (Test-Path -LiteralPath $full)
}
Export-ModuleMember -Function Clear-OwnedPerformanceWebView
