# Upgrade check with real binaries: a database written by an older tpt-home is
# migrated by the new one and must keep its data. Use it before a release, with
# the previous release binary as -OldBinary.
#
#   pwsh scripts/upgrade-check.ps1 -OldBinary .\old\tpt-home.exe
#   pwsh scripts/upgrade-check.ps1 -OldBinary .\old\tpt-home.exe -NewBinary .\target\release\tpt-home.exe
#
# Steps: init and write a note with the old binary; stop it; `migrate --dry-run`
# and `migrate` with the new binary; start the new server; check the note is
# still served. Exit code 0 on success.

param(
    [Parameter(Mandatory)][string]$OldBinary,
    [string]$NewBinary = (Join-Path $PSScriptRoot '..\target\release\tpt-home.exe'),
    [int]$Port = 18110,
    [string]$Password = 'upgrade-check-pass-1'
)

$ErrorActionPreference = 'Stop'
$OldBinary = (Resolve-Path $OldBinary).Path
$NewBinary = (Resolve-Path $NewBinary).Path
$work = Join-Path ([IO.Path]::GetTempPath()) ('tpt-upgrade-' + [guid]::NewGuid().ToString('N'))
$data = Join-Path $work 'data'
$marker = 'upgrade check ' + [guid]::NewGuid().ToString('N').Substring(0, 8)
$base = "http://127.0.0.1:$Port"
New-Item -ItemType Directory -Path $work | Out-Null

function Wait-Healthy {
    for ($i = 0; $i -lt 100; $i++) {
        try {
            $res = Invoke-WebRequest -Uri "$base/healthz" -UseBasicParsing -TimeoutSec 2
            if ($res.StatusCode -eq 200) { return }
        } catch { }
        Start-Sleep -Milliseconds 200
    }
    throw 'server did not become healthy'
}

function Start-Server([string]$binary) {
    Start-Process -FilePath $binary -PassThru -WindowStyle Hidden -ArgumentList @(
        '--data-dir', $data, 'run', '--host', '127.0.0.1', '--port', "$Port"
    )
}

function Stop-Server($proc) {
    if ($proc -and -not $proc.HasExited) {
        Stop-Process -Id $proc.Id -Force
        $proc.WaitForExit()
    }
    Start-Sleep -Milliseconds 500
}

$server = $null
try {
    Write-Host "1. old binary: init and write a note"
    & $OldBinary --data-dir $data init --admin-password $Password | Out-Null
    $server = Start-Server $OldBinary
    Wait-Healthy
    $session = $null
    Invoke-WebRequest -Uri "$base/api/auth/login" -Method Post -ContentType 'application/json' `
        -Body (@{ username = 'admin'; password = $Password } | ConvertTo-Json) `
        -SessionVariable session -UseBasicParsing | Out-Null
    Invoke-WebRequest -Uri "$base/api/notes" -Method Post -ContentType 'application/json' `
        -Body (@{ title = $marker; body = 'written by the old binary' } | ConvertTo-Json) `
        -WebSession $session -UseBasicParsing | Out-Null
    Stop-Server $server
    $server = $null

    Write-Host "2. new binary: dry run, then migrate"
    & $NewBinary --data-dir $data migrate --dry-run
    & $NewBinary --data-dir $data migrate

    Write-Host "3. new binary: serve and read the note back"
    $server = Start-Server $NewBinary
    Wait-Healthy
    $session = $null
    Invoke-WebRequest -Uri "$base/api/auth/login" -Method Post -ContentType 'application/json' `
        -Body (@{ username = 'admin'; password = $Password } | ConvertTo-Json) `
        -SessionVariable session -UseBasicParsing | Out-Null
    $notes = Invoke-WebRequest -Uri "$base/api/notes" -WebSession $session -UseBasicParsing
    if ($notes.Content -notlike "*$marker*") {
        throw "the note written by the old binary is missing after the upgrade"
    }
    Write-Host "PASS: the note from the old binary survives the upgrade"
    $code = 0
} catch {
    Write-Host "FAIL: $($_.Exception.Message)"
    $code = 1
} finally {
    Stop-Server $server
    Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
}
exit $code
