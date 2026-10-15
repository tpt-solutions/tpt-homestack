# Load test for a running tpt-homestack server. Starts the release binary on a
# fresh data directory, logs in, sends authenticated GET requests with a given
# concurrency, and reports throughput, latency percentiles, status counts and
# memory.
#
#   pwsh scripts/load.ps1                       # 2000 requests, 16 at a time
#   pwsh scripts/load.ps1 -Requests 20000 -Concurrency 32 -RotateClients
#
# The server rate-limits each client address to 240 requests a minute. From a
# single machine every request has the same address, so most of a plain run
# comes back 429. Use -RotateClients to send a different X-Forwarded-For per
# request with trust_proxy on; that measures the server, not the limiter.
#
# Soak: -DurationSeconds runs requests until the time is up instead of a fixed
# count. Add -SampleFile to write server and client memory every
# -SampleSeconds while the run goes on, so growth over time is visible.

param(
    [string]$Binary = (Join-Path $PSScriptRoot '..\target\release\tpt-home.exe'),
    [int]$Port = 18081,
    [int]$Requests = 2000,
    [int]$Concurrency = 16,
    [int]$DurationSeconds = 0,
    [switch]$RotateClients,
    [string]$SampleFile = '',
    [int]$SampleSeconds = 30
)

$ErrorActionPreference = 'Stop'
$Binary = (Resolve-Path $Binary).Path
$work = Join-Path ([IO.Path]::GetTempPath()) ('tpt-load-' + [guid]::NewGuid().ToString('N'))
$data = Join-Path $work 'data'
New-Item -ItemType Directory -Path $work | Out-Null
$password = 'load-test-password-1'
$sampler = $null

try {
    & $Binary --data-dir $data init --admin-password $password | Out-Null
    $env:TPT_HOME_TRUST_PROXY = if ($RotateClients) { 'true' } else { 'false' }
    $proc = Start-Process -FilePath $Binary -PassThru -WindowStyle Hidden -ArgumentList @(
        '--data-dir', $data, 'run', '--host', '127.0.0.1', '--port', "$Port"
    )
    $env:TPT_HOME_TRUST_PROXY = $null

    $base = "http://127.0.0.1:$Port"
    $ready = $false
    for ($i = 0; $i -lt 300 -and -not $ready; $i++) {
        try { $ready = (Invoke-WebRequest "$base/healthz" -UseBasicParsing -TimeoutSec 2).StatusCode -eq 200 } catch { Start-Sleep -Milliseconds 100 }
    }
    if (-not $ready) { throw 'server did not become healthy' }

    # One shared client with keep-alive. A new TCP connection per request
    # exhausts Windows' ephemeral ports (about 15k TIME_WAIT sockets after a
    # few runs), so requests would start failing on the client side.
    $client = [System.Net.Http.HttpClient]::new()
    $client.Timeout = [TimeSpan]::FromSeconds(10)

    $body = [System.Net.Http.StringContent]::new(
        (@{ username = 'admin'; password = $password } | ConvertTo-Json), [Text.Encoding]::UTF8, 'application/json')
    $login = $client.PostAsync("$base/api/auth/login", $body).GetAwaiter().GetResult()
    if (-not $login.IsSuccessStatusCode) { throw "login failed: $($login.StatusCode)" }
    $setCookie = $login.Headers.GetValues('Set-Cookie') | Select-Object -First 1
    $cookie = $setCookie.Split(';')[0]
    $login.Dispose()

    # A few notes so the list call does real work.
    for ($n = 0; $n -lt 50; $n++) {
        $note = [System.Net.Http.StringContent]::new(
            (@{ title = "note $n"; body = ('lorem ipsum ' * 20); tags = @('load') } | ConvertTo-Json),
            [Text.Encoding]::UTF8, 'application/json')
        $req = [System.Net.Http.HttpRequestMessage]::new([System.Net.Http.HttpMethod]::Post, "$base/api/notes")
        $req.Headers.TryAddWithoutValidation('Cookie', $cookie) | Out-Null
        $req.Content = $note
        $resp = $client.SendAsync($req).GetAwaiter().GetResult()
        $resp.Dispose()
        $req.Dispose()
    }

    if ($SampleFile) {
        # A separate process writes memory samples, so the client's own work
        # does not share a process with the sampling.
        $samplerScript = Join-Path $work 'sample.ps1'
        @'
param($ServerPid, $ClientPid, $LogFile, $Seconds)
$start = Get-Date
while (Get-Process -Id $ServerPid -ErrorAction SilentlyContinue) {
    $s = Get-Process -Id $ServerPid -ErrorAction SilentlyContinue
    $c = Get-Process -Id $ClientPid -ErrorAction SilentlyContinue
    if (-not $s -or -not $c) { break }
    $line = "{0,6:N0}s  server RAM {1,6:N1} MB  client private {2,6:N1} MB" -f `
        ((Get-Date) - $start).TotalSeconds, ($s.WorkingSet64 / 1MB), ($c.PrivateMemorySize64 / 1MB)
    Add-Content -Path $LogFile -Value $line
    Start-Sleep -Seconds $Seconds
}
'@ | Set-Content -Path $samplerScript
        $sampler = Start-Process -FilePath pwsh -PassThru -WindowStyle Hidden -ArgumentList @(
            '-NoProfile', '-File', $samplerScript, '-ServerPid', "$($proc.Id)", '-ClientPid', "$PID",
            '-LogFile', $SampleFile, '-Seconds', "$SampleSeconds")
    }

    $memBefore = [math]::Round($proc.WorkingSet64 / 1MB, 1)
    $deadline = if ($DurationSeconds -gt 0) { (Get-Date).AddSeconds($DurationSeconds) } else { $null }
    # Each worker sends its share of the requests, or runs until the deadline.
    $perWorker = if ($deadline) { [int]::MaxValue } else { [math]::Ceiling($Requests / $Concurrency) }

    $run = [Diagnostics.Stopwatch]::StartNew()
    # One parallel call for the whole run. Starting a new parallel call per
    # batch leaked client memory, so the workers loop inside a single call.
    $workers = 1..$Concurrency | ForEach-Object -ThrottleLimit $Concurrency -Parallel {
        $http = $using:client
        $end = $using:deadline
        $quota = $using:perWorker
        $latencies = [System.Collections.Generic.List[double]]::new()
        $failures = @{}
        $slow = [System.Collections.Generic.List[string]]::new()
        $sent = 0
        while ($sent -lt $quota -and (-not $end -or (Get-Date) -lt $end)) {
            $sent++
            $startedAt = [DateTime]::UtcNow
            $t = [Diagnostics.Stopwatch]::StartNew()
            $req = [System.Net.Http.HttpRequestMessage]::new([System.Net.Http.HttpMethod]::Get, "$($using:base)/api/notes")
            $req.Headers.TryAddWithoutValidation('Cookie', $using:cookie) | Out-Null
            if ($using:RotateClients) {
                # About 62,000 distinct addresses, so no single one reaches the
                # 240-a-minute limit even at several thousand requests a second.
                $n = $sent + $_ * 7919
                $req.Headers.TryAddWithoutValidation('X-Forwarded-For', "10.$($n % 250).$([math]::Floor($n / 250) % 250).1") | Out-Null
            }
            $code = 0
            try {
                $resp = $http.SendAsync($req).GetAwaiter().GetResult()
                $code = [int]$resp.StatusCode
                # Read the body so the connection can be reused.
                $resp.Content.ReadAsByteArrayAsync().GetAwaiter().GetResult() | Out-Null
                $resp.Dispose()
            } catch {
                $code = 0
            } finally {
                $req.Dispose()
            }
            $ms = $t.Elapsed.TotalMilliseconds
            if ($code -eq 200) { $latencies.Add($ms) }
            else { $failures[$code] = 1 + [int]$failures[$code] }
            # Slow requests are kept with their start time, so a stall can be
            # matched across workers: one worker alone points at the client or
            # the network, all of them at once points at the server.
            if ($ms -gt 1000) { $slow.Add(("{0:o} {1:N0} ms (status {2})" -f $startedAt, $ms, $code)) }
        }
        [pscustomobject]@{ Latencies = $latencies.ToArray(); Failures = $failures; Slow = $slow.ToArray() }
    }
    $run.Stop()

    $all = [System.Collections.Generic.List[double]]::new()
    $failures = @{}
    $slowRequests = [System.Collections.Generic.List[string]]::new()
    foreach ($w in $workers) {
        $all.AddRange([double[]]$w.Latencies)
        foreach ($k in $w.Failures.Keys) { $failures[$k] = [int]$failures[$k] + [int]$w.Failures[$k] }
        $slowRequests.AddRange([string[]]$w.Slow)
    }
    if ($slowRequests.Count) {
        Write-Host "requests over 1 s ($($slowRequests.Count)):"
        $slowRequests | Sort-Object | ForEach-Object { Write-Host "  $_" }
    }
    $sorted = $all.ToArray()
    [array]::Sort($sorted)
    $failed = 0
    foreach ($v in $failures.Values) { $failed += $v }
    $total = $sorted.Length + $failed
    function Pct([double[]]$xs, [double]$p) {
        if ($xs.Count -eq 0) { return 0 }
        $xs[[math]::Min($xs.Count - 1, [math]::Floor($xs.Count * $p))]
    }
    $statusText = @("200=$($sorted.Length)") + @($failures.GetEnumerator() | Sort-Object Name | ForEach-Object { "$($_.Name)=$($_.Value)" })
    $proc.Refresh()
    $memAfter = [math]::Round($proc.WorkingSet64 / 1MB, 1)

    [pscustomobject]@{
        Requests      = $total
        Seconds       = [math]::Round($run.Elapsed.TotalSeconds, 1)
        RequestsPerS  = [math]::Round($total / $run.Elapsed.TotalSeconds, 0)
        Statuses      = ($statusText -join ' ')
        P50Ms         = [math]::Round((Pct $sorted 0.50), 1)
        P95Ms         = [math]::Round((Pct $sorted 0.95), 1)
        P99Ms         = [math]::Round((Pct $sorted 0.99), 1)
        MaxMs         = if ($sorted.Length) { [math]::Round($sorted[-1], 1) } else { 0 }
        RamBeforeMB   = $memBefore
        RamAfterMB    = $memAfter
        RotateClients = [bool]$RotateClients
        Concurrency   = $Concurrency
    }
} finally {
    if ($sampler -and -not $sampler.HasExited) { Stop-Process -Id $sampler.Id -Force }
    if ($client) { $client.Dispose() }
    if ($proc -and -not $proc.HasExited) { Stop-Process -Id $proc.Id -Force }
    Start-Sleep -Milliseconds 200
    Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
}
