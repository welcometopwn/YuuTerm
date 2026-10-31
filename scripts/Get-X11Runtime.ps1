# Bundled X server: assembles the Cygwin/X runtime YuuTerm starts on demand for X11
# forwarding (docs/x11-forwarding.md). Downloads the pinned Cygwin packages listed in
# scripts/x11-runtime.json, verifies each SHA-512 from the signed setup index, extracts
# only the files XWin needs and writes target/x11-runtime/x11 with license texts,
# source references and a SHA-256 manifest. -Release also copies it next to
# target/release/yuuterm-desktop.exe; -Sources downloads the matching source archives
# to target/x11-sources for publication with a release.
param([switch]$Release, [switch]$Sources)
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$pin = Get-Content (Join-Path $PSScriptRoot 'x11-runtime.json') -Raw | ConvertFrom-Json
$cache = Join-Path $root 'target/x11-cache'
$out = Join-Path $root 'target/x11-runtime/x11'
$work = Join-Path $root ('target/x11-work-' + (Get-Date -Format 'yyyyMMdd-HHmmss-fff'))
$tar = Join-Path $env:SystemRoot 'System32/tar.exe'

function Get-Pinned($item, $directory) {
    New-Item -ItemType Directory -Force $directory | Out-Null
    $file = Join-Path $directory (Split-Path $item.path -Leaf)
    if (-not (Test-Path $file) -or (Get-FileHash $file -Algorithm SHA512).Hash -ne $item.sha512) {
        Invoke-WebRequest -Uri "$($pin.mirror)/$($item.path)" -OutFile $file -UseBasicParsing
    }
    if ((Get-FileHash $file -Algorithm SHA512).Hash -ne $item.sha512) {
        Remove-Item -LiteralPath $file
        throw "SHA-512 mismatch for $($item.path)"
    }
    $file
}

try {
    $extracted = Join-Path $work 'root'
    New-Item -ItemType Directory -Force $extracted | Out-Null
    $links = @()
    foreach ($package in $pin.packages) {
        $archive = Get-Pinned $package.install $cache
        # Windows tar cannot create the archives' symlinks; they are resolved below.
        & $tar -xf $archive -C $extracted 2>$null
        $links += & $tar -tvf $archive | Where-Object { $_ -match ' -> ' } | ForEach-Object {
            if ($_ -match '\s(\S+) -> (\S+)$') { [pscustomobject]@{ Name = $Matches[1]; Target = $Matches[2] } }
        }
    }
    foreach ($link in $links) {
        $target = if ($link.Target.StartsWith('/')) { $link.Target.TrimStart('/') } else { "$(Split-Path $link.Name -Parent)/$($link.Target)" }
        $source = Join-Path $extracted $target
        $destination = Join-Path $extracted $link.Name
        if (-not (Test-Path $destination) -and (Test-Path $source -PathType Leaf)) { Copy-Item $source $destination }
    }

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
        foreach ($package in $pin.packages) { $null = Get-Pinned $package.source (Join-Path $root 'target/x11-sources') }
    }
    [pscustomobject]@{
        Runtime = $out; Files = $files.Count
        SizeMB = [math]::Round(((Get-ChildItem $out -Recurse -File | Measure-Object Length -Sum).Sum / 1MB), 1)
        ManifestSHA256 = (Get-FileHash (Join-Path $out 'manifest.json')).Hash
    }
} finally {
    if (Test-Path $work) { Remove-Item -LiteralPath $work -Recurse -Force }
}
