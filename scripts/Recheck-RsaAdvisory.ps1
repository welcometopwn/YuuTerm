# Public upstream evidence only; no script/source evaluation and no private data.
[CmdletBinding()]
param()
$ErrorActionPreference='Stop'
$root=Split-Path $PSScriptRoot -Parent
$folder=Join-Path $root ('target/rsa-rereview-'+(Get-Date -Format 'yyyyMMdd-HHmmss-fff'))
$headers=@{'User-Agent'='YuuTerm dependency re-review'}
$utf8=[Text.UTF8Encoding]::new($false)
New-Item -ItemType Directory -Path $folder | Out-Null
$registry=Invoke-RestMethod 'https://crates.io/api/v1/crates/rsa' -Headers $headers -TimeoutSec 30
$advisoryUrl='https://raw.githubusercontent.com/RustSec/advisory-db/main/crates/rsa/RUSTSEC-2023-0071.md'
$advisory=(Invoke-WebRequest $advisoryUrl -Headers $headers -UseBasicParsing -TimeoutSec 30).Content
[IO.File]::WriteAllText((Join-Path $folder 'advisory.md'),$advisory,$utf8)
$ranges=[regex]::Match($advisory,'(?m)^patched\s*=\s*(?<value>\[[^\r\n]*\])').Groups['value'].Value
if(-not $ranges){throw 'Cannot identify upstream patched range'}
$versions=@($registry.versions | Where-Object {-not $_.yanked} | Sort-Object created_at -Descending | Select-Object -First 8 | ForEach-Object {
 [ordered]@{version=$_.num;published=$_.created_at;checksum=$_.checksum}
})
$latest=$versions[0]
if($latest.version -notmatch '^\d+\.\d+\.\d+(-[a-zA-Z0-9.]+)?$'){throw 'Unexpected registry version'}
$archive=Join-Path $folder 'rsa-latest.crate'
Invoke-WebRequest ("https://static.crates.io/crates/rsa/rsa-"+$latest.version+'.crate') -Headers $headers -UseBasicParsing -OutFile $archive -TimeoutSec 30
$archiveHash=(Get-FileHash -LiteralPath $archive).Hash.ToLowerInvariant()
if($archiveHash -ne $latest.checksum){throw 'Published archive checksum mismatch'}
# Extract only named files to stdout as data; never execute downloaded source.
$sources=@{}
$tar=Join-Path $env:SystemRoot 'System32/tar.exe'
foreach($name in @('README.md','src/pkcs1v15/signing_key.rs','src/pkcs1v15.rs','Cargo.toml')){
 $lines=& $tar -xOf $archive ("rsa-"+$latest.version+'/'+$name)
 if($LASTEXITCODE){throw 'Cannot inspect published source file'}
 $text=$lines -join "`n"
 $destination=Join-Path $folder ($name.Replace('/','-'))
 [IO.File]::WriteAllText($destination,$text,$utf8)
 $sources[$name]=[ordered]@{sha256=(Get-FileHash -LiteralPath $destination).Hash.ToLowerInvariant()}
}
$summary=[ordered]@{
 checked_utc=[DateTime]::UtcNow.ToString('o')
 registry_url='https://crates.io/api/v1/crates/rsa'
 advisory_url=$advisoryUrl
 advisory_sha256=(Get-FileHash -LiteralPath (Join-Path $folder 'advisory.md')).Hash.ToLowerInvariant()
 patched_ranges=$ranges
 recent_non_yanked_versions=$versions
 inspected_version=$latest.version
 archive_sha256=$archiveHash
 source_files=$sources
}
[IO.File]::WriteAllText((Join-Path $folder 'summary.json'),($summary | ConvertTo-Json -Depth 10),$utf8)
$summary | ConvertTo-Json -Depth 10
Write-Output "Reports: $folder"
