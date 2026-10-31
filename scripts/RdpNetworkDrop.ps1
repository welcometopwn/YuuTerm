# Runs ON the RDP test host (copied there by Test-RdpProduction.ps1 -Scope
# rdp-network, started as a SYSTEM scheduled task so it survives the outage).
# Waits, takes the network adapter down, then always brings it back.
param([int]$DelaySeconds = 45, [int]$DownSeconds = 60, [string]$Adapter = 'Ethernet')
Start-Sleep -Seconds $DelaySeconds
try {
    Disable-NetAdapter -Name $Adapter -Confirm:$false
    Start-Sleep -Seconds $DownSeconds
}
finally {
    Enable-NetAdapter -Name $Adapter -Confirm:$false
}
