<#
.SYNOPSIS
  VFS-5: starts `tauri-driver` on port 4444 with the Edge WebDriver whose version matches the
  installed WebView2 runtime (a mismatch makes the session hang, per the Tauri docs).
  Expects `tauri-driver` on PATH (cargo install tauri-driver --locked). Logs to -Out.
#>
param([Parameter(Mandatory = $true)] [string] $Out)
$ErrorActionPreference = 'Stop'
New-Item -ItemType Directory -Force -Path $Out | Out-Null
$key = 'HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}'
$wv2 = (Get-ItemProperty -Path $key).pv
Write-Host "WebView2 runtime $wv2"
$dir = Join-Path $env:RUNNER_TEMP 'msedgedriver'
New-Item -ItemType Directory -Force -Path $dir | Out-Null
$zip = Join-Path $dir 'edgedriver.zip'
Invoke-WebRequest -Uri "https://msedgedriver.microsoft.com/$wv2/edgedriver_win64.zip" -OutFile $zip
Expand-Archive -Path $zip -DestinationPath $dir -Force
$driver = Join-Path $dir 'msedgedriver.exe'
$sig = Get-AuthenticodeSignature -FilePath $driver
if ($sig.Status -ne 'Valid' -or $sig.SignerCertificate.Subject -notmatch 'Microsoft') {
  throw "msedgedriver.exe signature: $($sig.Status) $($sig.SignerCertificate.Subject)"
}
Write-Host "msedgedriver $((Get-Item $driver).VersionInfo.FileVersion)"
$td = (Get-Command tauri-driver).Source
Start-Process -FilePath $td -ArgumentList @('--port', '4444', '--native-driver', $driver) `
  -RedirectStandardOutput (Join-Path $Out 'tauri-driver.out.log') `
  -RedirectStandardError (Join-Path $Out 'tauri-driver.err.log')
$deadline = (Get-Date).AddSeconds(30)
while ((Get-Date) -lt $deadline) {
  try { Invoke-RestMethod -Uri 'http://127.0.0.1:4444/status' -TimeoutSec 2 | Out-Null; Write-Host 'tauri-driver ready'; exit 0 } catch { Start-Sleep -Milliseconds 500 }
}
throw 'tauri-driver did not answer on :4444'
