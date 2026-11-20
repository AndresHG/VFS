<#
.SYNOPSIS
  VFS-5: downloads the Edge WebDriver whose version matches the installed WebView2 runtime (a
  mismatch makes the session hang, per the Tauri docs), checks its Microsoft signature, and
  exports its path as MSEDGEDRIVER for journey.py / measure.py, which start `tauri-driver`
  themselves: the Windows runner ends a step's background processes when the step ends.
#>
$ErrorActionPreference = 'Stop'
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
"MSEDGEDRIVER=$driver" >> $env:GITHUB_ENV
