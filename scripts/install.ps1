# SPDX-License-Identifier: MIT OR Apache-2.0
# Copyright (c) 2026 TPT Solutions
#
# tpt-homestack installer helper for Windows.
# Registers the binary as a Scheduled Task that starts at logon (the
# simplest reliable auto-start without a service wrapper), and prepares the
# data directory under %LOCALAPPDATA%\tpt-home.
#
# Usage:
#   .\scripts\install.ps1                      # installs from target\release
#   .\scripts\install.ps1 -Uninstall

param(
    [switch]$Uninstall,
    [string]$Binary = "target\release\tpt-home.exe",
    [UInt16]$Port = 8080
)

$ErrorActionPreference = "Stop"
$TaskName = "tpt-homestack"
$DataDir = Join-Path $env:LOCALAPPDATA "tpt-home"

if ($Uninstall) {
    schtasks /Delete /TN $TaskName /F 2>$null
    Write-Host "uninstalled (data left in $DataDir)"
    exit 0
}

if (-not (Test-Path $Binary)) {
    Write-Error "binary not found at $Binary (run: cargo build --release)"
}

New-Item -ItemType Directory -Force -Path $DataDir | Out-Null
$bin = (Resolve-Path $Binary).Path
$exe = Join-Path $env:LOCALAPPDATA "tpt-home\tpt-home.exe"
Copy-Item $bin $exe -Force

$schema = @"
tpt-homestack Windows install
Data directory: $DataDir
Next steps:
  1. & "$exe" --data-dir "$DataDir" init
  2. schtasks /Run /TN $TaskName
  3. open http://127.0.0.1:$Port
"@

$action = New-ScheduledTaskAction -Execute $exe -Argument "run --data-dir `"$DataDir`" --port $Port"
$trigger = New-ScheduledTaskTrigger -AtLogOn
$settings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -RestartCount 3 -RestartInterval (New-TimeSpan -Minutes 1)
Register-ScheduledTask -TaskName $TaskName -Action $action -Trigger $trigger -Settings $settings -Force | Out-Null

Write-Host $schema
