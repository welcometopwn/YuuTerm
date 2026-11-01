# Assemble the pinned Cygwin/X runtime with licenses and a file manifest.
# -Sources also fetches the corresponding sources required for redistribution.
param([switch]$Release, [switch]$Sources)
$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot 'PinnedDownload.psm1') -Force
Import-Module (Join-Path $PSScriptRoot 'ArchiveTools.psm1') -Force
$root = Split-Path $PSScriptRoot -Parent
$pin = Get-Content (Join-Path $PSScriptRoot 'x11-runtime.json') -Raw | ConvertFrom-Json
$cache = Join-Path $root 'target/x11-cache'
$out = Join-Path $root 'target/x11-runtime/x11'
$work = Join-Path $root ('target/x11-work-' + (Get-Date -Format 'yyyyMMdd-HHmmss-fff'))

function Get-Pinned($item, $directory) {
    Get-PinnedFile -Mirror $pin.mirror -Item $item -Directory $directory
}

try {
    $extracted = Join-Path $work 'root'
    New-Item -ItemType Directory -Force $extracted | Out-Null
    $links = @()
    foreach ($package in $pin.packages) {
        Write-Host "[X11] Runtime package: $($package.name) $($package.version)"
        $archive = Get-Pinned $package.install $cache
        $links += @(Expand-PinnedArchive -Archive $archive -Directory $extracted)
    }
    Write-Host "[X11] Resolving $($links.Count) archive links as regular files"
    foreach ($link in $links) {
        $target = if ($link.HardLink -or $link.Target.StartsWith('/')) { $link.Target.TrimStart('/') } else { "$(Split-Path $link.Name -Parent)/$($link.Target)" }
        $source = Join-Path $extracted $target
        $destination = Join-Path $extracted $link.Name
        $prefix = [IO.Path]::GetFullPath($extracted) + [IO.Path]::DirectorySeparatorChar
        foreach ($path in @($source, $destination)) {
            if (-not [IO.Path]::GetFullPath($path).StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)) {
                throw 'Archive link escapes the extraction directory'
            }
        }
        if (-not (Test-Path $destination) -and (Test-Path $source -PathType Leaf)) { Copy-Item $source $destination }
    }

    Write-Host '[X11] Assembling runtime, licenses and file manifest'

    if (Test-Path $out) { Remove-Item -LiteralPath $out -Recurse -Force }
    $bin = New-Item -ItemType Directory -Force (Join-Path $out 'bin')
    foreach ($name in $pin.bin) { Copy-Item (Join-Path $extracted "usr/bin/$name") $bin }
    # Request names for X protocol error messages (/usr/lib maps to /lib in Cygwin).
    New-Item -ItemType Directory -Force (Join-Path $out 'lib/xorg') | Out-Null
    Copy-Item (Join-Path $extracted 'usr/lib/xorg/protocol.txt') (Join-Path $out 'lib/xorg')
    New-Item -ItemType Directory -Force (Join-Path $out 'usr/share/X11') | Out-Null
    Copy-Item (Join-Path $extracted 'usr/share/xkeyboard-config-2') (Join-Path $out 'usr/share/X11/xkb') -Recurse
    if (-not (Test-Path (Join-Path $out 'usr/share/X11/xkb/rules/base') -PathType Leaf)) { throw 'XKB rules missing' }

    $licenses = Join-Path $out 'licenses'
    Get-ChildItem (Join-Path $extracted 'usr/share/doc') -Directory | ForEach-Object {
        $texts = Get-ChildItem $_.FullName -File | Where-Object Name -Match '^(COPYING|LICENSE|CYGWIN_LICENSE|COPYRIGHT)'
        if ($texts) {
            $target = New-Item -ItemType Directory -Force (Join-Path $licenses $_.Name)
            $texts | Copy-Item -Destination $target
        }
    }
    $notice = @(
        'YuuTerm bundled X server: Cygwin/X and its runtime dependencies, unmodified Cygwin binaries.'
        "Index signed by Cygwin key $($pin.setup_index_signed_by); checked $($pin.checked)."
        'Each package is redistributed under its own license; texts shipped by the packages are in licenses/.'
        'Corresponding source archives (published with each YuuTerm release, also at the Cygwin mirror):'
        ''
    ) + ($pin.packages | ForEach-Object { "$($_.name) $($_.version)`r`n  $($pin.mirror)/$($_.source.path)`r`n  sha512 $($_.source.sha512)" })
    [IO.File]::WriteAllText((Join-Path $out 'SOURCES.txt'), ($notice -join "`r`n") + "`r`n")

    # The manifest identifies this runtime; YuuTerm copies it to its data folder
    # when the manifest differs and verifies every listed hash while copying.
    $files = [ordered]@{}
    Get-ChildItem $out -Recurse -File | Sort-Object FullName | ForEach-Object {
        $relative = [IO.Path]::GetRelativePath($out, $_.FullName).Replace('\', '/')
        $files[$relative] = (Get-FileHash $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
    }
    $server = $pin.packages | Where-Object name -EQ 'xorg-server'
    $manifest = [ordered]@{ server = "Cygwin/X $($server.version)"; files = $files }
    [IO.File]::WriteAllText((Join-Path $out 'manifest.json'), ($manifest | ConvertTo-Json -Depth 3))

    if ($Release) {
        $installed = Join-Path $root 'target/release/x11'
        if (Test-Path $installed) { Remove-Item -LiteralPath $installed -Recurse -Force }
        Copy-Item $out $installed -Recurse
    }
    if ($Sources) {
        Write-Host '[X11] Preparing matching source archives'
        foreach ($package in $pin.packages) {
            Write-Host "[X11] Source package: $($package.name) $($package.version)"
            $null = Get-Pinned $package.source (Join-Path $root 'target/x11-sources')
        }
    }
    Write-Host "[X11] Complete: $($files.Count) runtime files"
    [pscustomobject]@{
        Runtime = $out; Files = $files.Count
        SizeMB = [math]::Round(((Get-ChildItem $out -Recurse -File | Measure-Object Length -Sum).Sum / 1MB), 1)
        ManifestSHA256 = (Get-FileHash (Join-Path $out 'manifest.json')).Hash
    }
} finally {
    if (Test-Path $work) { Remove-Item -LiteralPath $work -Recurse -Force }
}
