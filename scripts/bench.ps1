# Measures the release targets from todo.md (Phase 8) on this machine:
# binary size, startup to first healthy response, idle memory and idle CPU.
#
#   cargo build --release -p tpt-home-cli
#   pwsh scripts/bench.ps1
#
# Startup includes the time for this script's own health probe, so it is a
# slight overestimate. Idle figures are for an empty server with no plugins.

param(
    [string]$Binary = (Join-Path $PSScriptRoot '..\target\release\tpt-home.exe'),
    [int]$Port = 18080,
    [int]$IdleSeconds = 10
)

$ErrorActionPreference = 'Stop'
$Binary = (Resolve-Path $Binary).Path

$work = Join-Path ([IO.Path]::GetTempPath()) ('tpt-bench-' + [guid]::NewGuid().ToString('N'))
$data = Join-Path $work 'data'
New-Item -ItemType Directory -Path $work | Out-Null

try {
    & $Binary --data-dir $data init --admin-password 'bench-password-1' | Out-Null

    $clock = [Diagnostics.Stopwatch]::StartNew()
    $proc = Start-Process -FilePath $Binary -PassThru -WindowStyle Hidden -ArgumentList @(
        '--data-dir', $data, 'run', '--host', '127.0.0.1', '--port', "$Port"
    )
    $ready = $false
    while (-not $ready) {
        try {
            $res = Invoke-WebRequest -Uri "http://127.0.0.1:$Port/healthz" -UseBasicParsing -TimeoutSec 2
            $ready = ($res.StatusCode -eq 200)
        } catch {
            $ready = $false
        }
        if ($clock.Elapsed.TotalSeconds -gt 30) { throw 'server did not become healthy within 30 s' }
        if (-not $ready) { Start-Sleep -Milliseconds 20 }
    }
    $startupMs = $clock.Elapsed.TotalMilliseconds

    # Let startup work (migrations, job timers) settle before measuring idle.
    Start-Sleep -Seconds 2
    $cpuStart = $proc.TotalProcessorTime
    Start-Sleep -Seconds $IdleSeconds
    $proc.Refresh()
    $cpuMs = ($proc.TotalProcessorTime - $cpuStart).TotalMilliseconds
    $idleCpuPct = 100 * $cpuMs / ($IdleSeconds * 1000)

    $result = [pscustomobject]@{
        BinaryMB     = [math]::Round((Get-Item $Binary).Length / 1MB, 2)
        StartupMs    = [math]::Round($startupMs, 0)
        IdleRamMB    = [math]::Round($proc.WorkingSet64 / 1MB, 1)
        PeakRamMB    = [math]::Round($proc.PeakWorkingSet64 / 1MB, 1)
        IdleCpuPct   = [math]::Round($idleCpuPct, 2)
        IdleSeconds  = $IdleSeconds
    }
    $result | Format-List
    $result
} finally {
    if ($proc -and -not $proc.HasExited) { Stop-Process -Id $proc.Id -Force }
    Start-Sleep -Milliseconds 200
    Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
}
