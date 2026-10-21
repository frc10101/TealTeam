# Windows laptop for an event (F12). OPTIONAL: the team's Chromebooks use the
# ChromeOS checklist in docs/SETUP_GUIDE.md instead.
#
#   powershell -ExecutionPolicy Bypass -File deploy\laptop\setup-laptop.ps1 [-PiIp 10.101.0.1] [-DryRun] [-Undo]
#
# Needs an administrator PowerShell (Wi-Fi, hosts file, power). Wi-Fi off,
# a hosts entry if tealteam.local does not resolve, a desktop shortcut, no
# sleep on AC; -Undo reverses them. No internet needed.
#
# UNTESTED: never run on Windows.
param([string]$PiIp = "10.101.0.1", [switch]$DryRun, [switch]$Undo)
$ErrorActionPreference = "Stop"
function Run($what, [scriptblock]$do) { if ($DryRun) { "    would: $what" } else { & $do } }
$hosts = "$env:SystemRoot\System32\drivers\etc\hosts"
$link = Join-Path ([Environment]::GetFolderPath("Desktop")) "TealTeam.url"

if ($Undo) {
    Get-NetAdapter -Physical | Where-Object { $_.Name -match "Wi-?Fi" } | ForEach-Object { Run "enable $($_.Name)" { Enable-NetAdapter -Name $_.Name -Confirm:$false } }
    Run "remove shortcut" { Remove-Item $link -ErrorAction SilentlyContinue }
    Run "sleep after 30 min on AC" { powercfg /change standby-timeout-ac 30 }
    return
}
"==> Wi-Fi off"
Get-NetAdapter -Physical | Where-Object { $_.Name -match "Wi-?Fi" -and $_.Status -eq "Up" } | ForEach-Object { Run "disable $($_.Name)" { Disable-NetAdapter -Name $_.Name -Confirm:$false } }
"==> finding the server"
$url = "http://tealteam.local"
try { [void][Net.Dns]::GetHostAddresses("tealteam.local") ; "    tealteam.local resolves" }
catch {
    "    does not resolve: adding $PiIp to the hosts file"
    Run "hosts entry" { if (-not (Select-String -Path $hosts -Pattern "tealteam.local" -Quiet)) { Add-Content $hosts "`n$PiIp tealteam.local" } }
}
if (-not $DryRun) { "    server answers: " + (Invoke-WebRequest -UseBasicParsing "$url/health" -TimeoutSec 5).Content }
"==> shortcut and sleep"
Run "shortcut" { Set-Content $link "[InternetShortcut]`nURL=$url/" }
Run "no sleep on AC" { powercfg /change standby-timeout-ac 0 }
"`nBrowser steps this cannot do: sign in with the sign-in sheet; clear old site data for $url; open the scouting page once."
