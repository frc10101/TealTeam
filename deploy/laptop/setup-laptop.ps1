# Get a Windows laptop (the lead's or the admin's) ready for the event LAN (SK1).
# The scouts' Chromebooks cannot run this: see docs/SETUP_GUIDE.md.
#
#   powershell -ExecutionPolicy Bypass -File deploy\laptop\setup-laptop.ps1 [-Url http://tealteam.local] [-PiIp 10.101.0.1] [-DryRun]
#   ... -Undo        put Wi-Fi, sleep and the hosts entry back
#
# Run in a PowerShell opened as Administrator (Wi-Fi, power and hosts need it).
# Needs no internet. UNTESTED: written for Windows 10/11, never run.
param([string]$Url = "http://tealteam.local", [string]$PiIp = "10.101.0.1", [switch]$DryRun, [switch]$Undo)
$ErrorActionPreference = "Stop"
function Say($m) { Write-Host "==> $m" }
function Run($desc, [scriptblock]$b) { if ($DryRun) { Write-Host "    would: $desc" } else { & $b } }
$Url = $Url.TrimEnd("/")
$hosts = "$env:SystemRoot\System32\drivers\etc\hosts"

if ($Undo) {
    Say "putting things back"
    Run "Wi-Fi on" { Get-NetAdapter -Physical | Where-Object { $_.PhysicalMediaType -like "*802.11*" } | Enable-NetAdapter -Confirm:$false }
    Run "sleep 30 min on AC" { powercfg /change standby-timeout-ac 30 }
    Run "remove the tealteam hosts line" { (Get-Content $hosts) | Where-Object { $_ -notmatch "# tealteam$" } | Set-Content $hosts }
    exit 0
}

Say "wired link"
$wired = Get-NetAdapter -Physical | Where-Object { $_.Status -eq "Up" -and $_.PhysicalMediaType -notlike "*802.11*" }
if ($wired) { Write-Host "    up: $($wired[0].Name)" } else { Write-Host "    NO wired link: plug the cable into the switch" }

Say "Wi-Fi off (event data is wired only)"
Run "disable Wi-Fi adapters" { Get-NetAdapter -Physical | Where-Object { $_.PhysicalMediaType -like "*802.11*" } | Disable-NetAdapter -Confirm:$false }

Say "name tealteam.local"
$name = ([uri]$Url).Host
try { Resolve-DnsName $name -ErrorAction Stop | Out-Null; Write-Host "    resolves" }
catch {
    Write-Host "    does not resolve (no Bonjour): adding $PiIp $name to the hosts file"
    Run "append the hosts line" { Add-Content $hosts "$PiIp $name # tealteam" }
    $Url = "http://$PiIp"
}

Say "the server at $Url"
try { Write-Host "    answers: $((Invoke-WebRequest -UseBasicParsing "$Url/health" -TimeoutSec 5).Content)" }
catch { Write-Host "    does not answer. Check the cable and the Pi; try http://$PiIp" }

Say "shortcut on the desktop"
Run "write TealTeam.url" { Set-Content "$([Environment]::GetFolderPath('Desktop'))\TealTeam.url" "[InternetShortcut]`r`nURL=$Url/" }

Say "stay awake on AC (undo with -Undo)"
Run "never sleep on AC" { powercfg /change standby-timeout-ac 0; powercfg /change monitor-timeout-ac 0 }

Write-Host @"

Still by hand, in the browser:
  [ ] open $Url/ and sign in (the sign-in sheet has your password)
  [ ] clear old site data for tealteam from earlier events
  [ ] bookmark $Url/ and $Url/submission
"@
