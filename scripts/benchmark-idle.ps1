param(
    [int]$DurationSeconds = 15,
    [int]$IntervalMs = 1000,
    [double]$MaxTotalRamMb = 350,
    [double]$MaxAvgCpuPercent = 3.0,
    [string]$OutputJson = "",
    [switch]$FailOnThreshold
)

$ErrorActionPreference = "Stop"

if ($DurationSeconds -lt 1) { throw "DurationSeconds must be at least 1." }
if ($IntervalMs -lt 100) { throw "IntervalMs must be at least 100." }

$fernProcs = @(Get-Process -Name "fern" -ErrorAction SilentlyContinue)
if ($fernProcs.Count -eq 0) {
    Write-Host "Fern (fern.exe) is not currently running." -ForegroundColor Yellow
    Write-Host "Launch Fern, let it settle in idle state, and re-run:" -ForegroundColor Gray
    Write-Host "  npm run benchmark:idle" -ForegroundColor Cyan
    if ($FailOnThreshold) {
        exit 2
    }
    exit 0
}

function Get-MonitorCount {
    try {
        Add-Type -AssemblyName System.Windows.Forms -ErrorAction Stop
        return [System.Windows.Forms.Screen]::AllScreens.Count
    } catch {
        return $null
    }
}

function Get-DescendantWebViewPids([int[]]$rootPids) {
    $allProcs = Get-CimInstance -ClassName Win32_Process -Property ProcessId, ParentProcessId, Name -ErrorAction SilentlyContinue
    if (-not $allProcs) { return @() }

    $childrenByParent = @{}
    foreach ($p in $allProcs) {
        $ppid = [int]$p.ParentProcessId
        if (-not $childrenByParent.ContainsKey($ppid)) {
            $childrenByParent[$ppid] = New-Object System.Collections.Generic.List[object]
        }
        $childrenByParent[$ppid].Add($p)
    }

    $visited = New-Object System.Collections.Generic.HashSet[int]
    $queue = New-Object System.Collections.Generic.Queue[int]
    foreach ($rpid in $rootPids) {
        if ($visited.Add($rpid)) {
            $queue.Enqueue($rpid)
        }
    }

    $wvPids = New-Object System.Collections.Generic.List[int]
    while ($queue.Count -gt 0) {
        $curr = $queue.Dequeue()
        if ($childrenByParent.ContainsKey($curr)) {
            foreach ($child in $childrenByParent[$curr]) {
                $cid = [int]$child.ProcessId
                if ($visited.Add($cid)) {
                    $queue.Enqueue($cid)
                    if ($child.Name -ieq "msedgewebview2.exe") {
                        $wvPids.Add($cid)
                    }
                }
            }
        }
    }
    return @($wvPids.ToArray())
}

function Get-Stats([double[]]$values) {
    if (-not $values -or $values.Count -eq 0) {
        return [ordered]@{ min = 0.0; avg = 0.0; max = 0.0 }
    }
    $measure = $values | Measure-Object -Minimum -Average -Maximum
    return [ordered]@{
        min = [math]::Round([double]$measure.Minimum, 2)
        avg = [math]::Round([double]$measure.Average, 2)
        max = [math]::Round([double]$measure.Maximum, 2)
    }
}

$cpuCores = [Environment]::ProcessorCount
$monitorCount = Get-MonitorCount
$sampleCount = [math]::Max(1, [int][math]::Ceiling(($DurationSeconds * 1000.0) / $IntervalMs))

$monitorDisplay = if ($null -ne $monitorCount) { $monitorCount } else { "unknown" }
Write-Host "Sampling Fern idle performance..." -ForegroundColor Cyan
Write-Host ("  Duration: {0}s | Interval: {1}ms | Samples: {2} | Monitors: {3} | Logical CPUs: {4}" -f `
    $DurationSeconds, $IntervalMs, $sampleCount, $monitorDisplay, $cpuCores) -ForegroundColor Gray

$fernWsSamples = New-Object System.Collections.Generic.List[double]
$fernPrivSamples = New-Object System.Collections.Generic.List[double]
$wvWsSamples = New-Object System.Collections.Generic.List[double]
$wvPrivSamples = New-Object System.Collections.Generic.List[double]
$totalWsSamples = New-Object System.Collections.Generic.List[double]
$totalPrivSamples = New-Object System.Collections.Generic.List[double]
$wvCountSamples = New-Object System.Collections.Generic.List[double]
$cpuSamples = New-Object System.Collections.Generic.List[double]

$prevCpuByPid = @{}
$prevTimestamp = [DateTime]::UtcNow

# Seed baseline CPU times
$seedFern = @(Get-Process -Name "fern" -ErrorAction SilentlyContinue)
$seedWvPids = Get-DescendantWebViewPids -rootPids ($seedFern | ForEach-Object { $_.Id })
$seedAll = @($seedFern)
if ($seedWvPids.Count -gt 0) {
    $seedAll += @(Get-Process -Id $seedWvPids -ErrorAction SilentlyContinue)
}
foreach ($proc in $seedAll) {
    try { $prevCpuByPid[$proc.Id] = $proc.TotalProcessorTime.TotalSeconds } catch {}
}

for ($i = 0; $i -lt $sampleCount; $i++) {
    Start-Sleep -Milliseconds $IntervalMs
    $now = [DateTime]::UtcNow
    $elapsedSec = ($now - $prevTimestamp).TotalSeconds
    if ($elapsedSec -le 0) { $elapsedSec = $IntervalMs / 1000.0 }

    $currentFern = @(Get-Process -Name "fern" -ErrorAction SilentlyContinue)
    if ($currentFern.Count -eq 0) {
        Write-Host "Fern exited during sampling." -ForegroundColor Yellow
        break
    }

    $fernPids = @($currentFern | ForEach-Object { $_.Id })
    $wvPids = Get-DescendantWebViewPids -rootPids $fernPids
    $currentWv = @()
    if ($wvPids.Count -gt 0) {
        $currentWv = @(Get-Process -Id $wvPids -ErrorAction SilentlyContinue)
    }

    $fernWsMb = 0.0
    $fernPrivMb = 0.0
    $wvWsMb = 0.0
    $wvPrivMb = 0.0
    $deltaCpuSec = 0.0
    $nextCpuByPid = @{}

    foreach ($p in $currentFern) {
        try {
            $fernWsMb += ($p.WorkingSet64 / 1MB)
            $fernPrivMb += ($p.PrivateMemorySize64 / 1MB)
            $cpuSec = $p.TotalProcessorTime.TotalSeconds
            $nextCpuByPid[$p.Id] = $cpuSec
            if ($prevCpuByPid.ContainsKey($p.Id)) {
                $d = $cpuSec - $prevCpuByPid[$p.Id]
                if ($d -gt 0) { $deltaCpuSec += $d }
            }
        } catch {}
    }

    foreach ($p in $currentWv) {
        try {
            $wvWsMb += ($p.WorkingSet64 / 1MB)
            $wvPrivMb += ($p.PrivateMemorySize64 / 1MB)
            $cpuSec = $p.TotalProcessorTime.TotalSeconds
            $nextCpuByPid[$p.Id] = $cpuSec
            if ($prevCpuByPid.ContainsKey($p.Id)) {
                $d = $cpuSec - $prevCpuByPid[$p.Id]
                if ($d -gt 0) { $deltaCpuSec += $d }
            }
        } catch {}
    }

    $cpuPct = ($deltaCpuSec / ($elapsedSec * $cpuCores)) * 100.0

    $fernWsSamples.Add($fernWsMb)
    $fernPrivSamples.Add($fernPrivMb)
    $wvWsSamples.Add($wvWsMb)
    $wvPrivSamples.Add($wvPrivMb)
    $totalWsSamples.Add($fernWsMb + $wvWsMb)
    $totalPrivSamples.Add($fernPrivMb + $wvPrivMb)
    $wvCountSamples.Add([double]$currentWv.Count)
    $cpuSamples.Add($cpuPct)

    $prevCpuByPid = $nextCpuByPid
    $prevTimestamp = $now
}

if ($totalWsSamples.Count -eq 0) {
    Write-Host "No samples collected." -ForegroundColor Yellow
    if ($FailOnThreshold) { exit 2 }
    exit 0
}

$fernWsStats = Get-Stats $fernWsSamples.ToArray()
$fernPrivStats = Get-Stats $fernPrivSamples.ToArray()
$wvWsStats = Get-Stats $wvWsSamples.ToArray()
$wvPrivStats = Get-Stats $wvPrivSamples.ToArray()
$totalWsStats = Get-Stats $totalWsSamples.ToArray()
$totalPrivStats = Get-Stats $totalPrivSamples.ToArray()
$wvCountStats = Get-Stats $wvCountSamples.ToArray()
$cpuStats = Get-Stats $cpuSamples.ToArray()

$ramPassed = $totalWsStats.avg -le $MaxTotalRamMb
$cpuPassed = $cpuStats.avg -le $MaxAvgCpuPercent
$overallPassed = $ramPassed -and $cpuPassed

$rows = @(
    [pscustomobject]@{ Metric = "fern.exe Working Set (MB)";      Min = $fernWsStats.min;   Avg = $fernWsStats.avg;   Max = $fernWsStats.max }
    [pscustomobject]@{ Metric = "fern.exe Private Mem (MB)";      Min = $fernPrivStats.min; Avg = $fernPrivStats.avg; Max = $fernPrivStats.max }
    [pscustomobject]@{ Metric = "WebView2 Working Set (MB)";      Min = $wvWsStats.min;     Avg = $wvWsStats.avg;     Max = $wvWsStats.max }
    [pscustomobject]@{ Metric = "WebView2 Private Mem (MB)";      Min = $wvPrivStats.min;   Avg = $wvPrivStats.avg;   Max = $wvPrivStats.max }
    [pscustomobject]@{ Metric = "Total Working Set (MB)";         Min = $totalWsStats.min;  Avg = $totalWsStats.avg;  Max = $totalWsStats.max }
    [pscustomobject]@{ Metric = "Total Private Mem (MB)";         Min = $totalPrivStats.min;Avg = $totalPrivStats.avg;Max = $totalPrivStats.max }
    [pscustomobject]@{ Metric = "WebView2 Child Processes";       Min = $wvCountStats.min;  Avg = $wvCountStats.avg;  Max = $wvCountStats.max }
    [pscustomobject]@{ Metric = "Total CPU (%)";                  Min = $cpuStats.min;      Avg = $cpuStats.avg;      Max = $cpuStats.max }
)

Write-Host ""
Write-Host "Fern Idle Benchmark Summary" -ForegroundColor Green
$rows | Format-Table -AutoSize

$ramStatus = if ($ramPassed) { "PASS" } else { "FAIL" }
$cpuStatus = if ($cpuPassed) { "PASS" } else { "FAIL" }
$statusColor = if ($overallPassed) { "Green" } else { "Red" }
Write-Host ("Thresholds: Avg Total Working Set <= {0} MB ({1}) | Avg CPU <= {2}% ({3})" -f `
    $MaxTotalRamMb, $ramStatus, $MaxAvgCpuPercent, $cpuStatus) `
    -ForegroundColor $statusColor

if ($OutputJson) {
    $report = [ordered]@{
        timestampUtc    = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")
        durationSeconds = $DurationSeconds
        intervalMs      = $IntervalMs
        samples         = $totalWsSamples.Count
        monitorCount    = $monitorCount
        logicalCpus     = $cpuCores
        thresholds      = [ordered]@{
            maxTotalRamMb    = $MaxTotalRamMb
            maxAvgCpuPercent = $MaxAvgCpuPercent
        }
        passed          = [ordered]@{
            ram     = $ramPassed
            cpu     = $cpuPassed
            overall = $overallPassed
        }
        metrics         = [ordered]@{
            fernWorkingSetMb    = $fernWsStats
            fernPrivateMb       = $fernPrivStats
            webviewWorkingSetMb = $wvWsStats
            webviewPrivateMb    = $wvPrivStats
            totalWorkingSetMb   = $totalWsStats
            totalPrivateMb      = $totalPrivStats
            webviewProcessCount = $wvCountStats
            cpuPercent          = $cpuStats
        }
    }

    $outDir = Split-Path -Parent $OutputJson
    if ($outDir -and -not (Test-Path $outDir)) {
        New-Item -ItemType Directory -Path $outDir -Force | Out-Null
    }
    [IO.File]::WriteAllText($OutputJson, ($report | ConvertTo-Json -Depth 6), (New-Object Text.UTF8Encoding $false))
    Write-Host "Wrote JSON report to $OutputJson" -ForegroundColor Cyan
}

if ($FailOnThreshold -and -not $overallPassed) {
    Write-Error ("Idle benchmark exceeded thresholds (Avg RAM: {0} MB / {1} MB, Avg CPU: {2}% / {3}%)." -f `
        $totalWsStats.avg, $MaxTotalRamMb, $cpuStats.avg, $MaxAvgCpuPercent)
    exit 1
}

exit 0
