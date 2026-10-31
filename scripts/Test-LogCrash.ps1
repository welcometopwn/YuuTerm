[CmdletBinding()]
param([switch]$AllFeatures)
$ErrorActionPreference='Stop'
$root=Split-Path $PSScriptRoot -Parent
Push-Location $root
try {
 $arguments=@('test','-p','yuuterm-desktop','--locked')
 if($AllFeatures){$arguments+='--all-features'}
 $arguments+=@('output_log::crash_tests::killed_process_preserves_checkpoints_and_records_older_than_sync_interval','--','--exact','--nocapture')
 & cargo.exe @arguments
 if($LASTEXITCODE){throw 'Forced process-kill regression failed'}
 Get-Content -LiteralPath (Join-Path $root 'target/log-crash-tests/process-kill.json') -Raw
}finally{Pop-Location}
