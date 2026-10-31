param(
    [Parameter(Mandatory=$true)][string]$Summary,
    [ValidateRange(0, 60000)][double]$ReadyBudgetMs = 0
)
$ErrorActionPreference = 'Stop'
$data = Get-Content -LiteralPath $Summary -Raw | ConvertFrom-Json
if (-not $data.passed -or @($data.runs).Count -lt 3) { throw 'Need at least three completed instrumented runs' }
$rows = @($data.runs | ForEach-Object {
    $timings = $_.idle_report.backend_startup
    $first = ($timings | Measure-Object start_observed_ms -Minimum).Minimum
    $listing = ($timings | Measure-Object fixture_listing_observed_ms -Maximum).Maximum
    [pscustomobject]@{
        repeat = $_.repeat
        sessions = $_.connections
        ready_ms = [Math]::Round($_.launch_to_ready_observed_ms, 1)
        first_call_ms = [Math]::Round($first, 1)
        call_spread_ms = [Math]::Round(($timings | Measure-Object start_observed_ms -Maximum).Maximum - $first, 1)
        calls_to_last_listing_ms = [Math]::Round($listing - $first, 1)
        listing_to_idle_report_ms = [Math]::Round($_.idle_report.backend_elapsed_ms - $listing, 1)
        idle_cpu_core_percent = [Math]::Round($_.client_cpu_one_logical_core_percent, 2)
        private_peak_mib = [Math]::Round(($_.process_samples | Measure-Object private_bytes -Maximum).Maximum / 1MB, 1)
        processes_max = ($_.process_samples | Measure-Object processes -Maximum).Maximum
        backend_to_navigation_estimate_ms = [Math]::Round($_.idle_report.backend_elapsed_ms - $_.idle_report.frontend_ready_ms, 1)
    }
})
$rows | Format-Table -AutoSize
$data.runs | ForEach-Object {
    $stages = @($_.idle_report.backend_startup | ForEach-Object { $_.stages } | Where-Object { $_ })
    if ($stages.Count) {
        [pscustomobject]@{
            repeat = $_.repeat
            agent_connect_max_ms = ($stages | Measure-Object agent_connect_ms -Maximum).Maximum
            agent_authentication_max_ms = ($stages | Measure-Object agent_authentication_ms -Maximum).Maximum
            handshake_max_ms = ($stages | Measure-Object handshake_ms -Maximum).Maximum
        } | Format-List
    }
}
foreach ($count in @($rows.sessions | Sort-Object -Unique)) {
    $values = @($rows | Where-Object sessions -eq $count | ForEach-Object ready_ms | Sort-Object)
    $median = ($values[[int][Math]::Floor(($values.Count - 1) / 2)] + $values[[int][Math]::Floor($values.Count / 2)]) / 2
    "Sessions: $count; median launch-to-ready: $median ms; budget: $ReadyBudgetMs ms"
    if ($ReadyBudgetMs -gt 0 -and $median -gt $ReadyBudgetMs) { throw 'Startup readiness exceeds the requested regression budget' }
}
