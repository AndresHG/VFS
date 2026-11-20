<#
.SYNOPSIS
  VFS-8 AC-1: installs the NSIS installer silently, per user, exactly as a user gets it, and
  records what it installed; or uninstalls it and records what is left.

  windows-install.ps1 -Installer <path\VFS_x_x64-setup.exe> -Out <dir>   # install + verify
  windows-install.ps1 -Uninstall -Out <dir>                              # uninstall + verify

  Writes <Out>\install.json or <Out>\uninstall.json. Exits non-zero when a check fails, after
  writing the JSON. Prints the installed app's exe path on the last line of an install.
#>
param(
  [string] $Installer = '',
  [switch] $Uninstall,
  [Parameter(Mandatory = $true)] [string] $Out
)
$ErrorActionPreference = 'Stop'
New-Item -ItemType Directory -Force -Path $Out | Out-Null
$uninstallKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\VFS'
$machineKey = 'HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\VFS'
$startMenu = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs'
$installDir = Join-Path $env:LOCALAPPDATA 'VFS'

function Shortcuts {
  @(Get-ChildItem -Path $startMenu -Recurse -Filter '*.lnk' -ErrorAction SilentlyContinue |
    Where-Object { $_.BaseName -like '*VFS*' } | ForEach-Object { $_.FullName })
}

function WebView2Version {
  foreach ($k in @('HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}',
                   'HKCU:\Software\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}')) {
    $v = (Get-ItemProperty -Path $k -ErrorAction SilentlyContinue).pv
    if ($v -and $v -ne '0.0.0.0') { return $v }
  }
  return $null
}

$checks = [ordered]@{}
$fail = @()
function Check([string] $name, [bool] $ok, $detail) {
  $script:checks[$name] = [ordered]@{ ok = $ok; detail = $detail }
  if (-not $ok) { $script:fail += $name }
  Write-Host ('{0,-28} {1}  {2}' -f $name, ($(if ($ok) { 'ok' } else { 'FAIL' })), ($detail | ConvertTo-Json -Compress -Depth 4))
}

if (-not $Uninstall) {
  $installerItem = Get-Item -LiteralPath $Installer
  # The NSIS manifest of a per-user installer asks for no elevation (asInvoker). Read it from
  # the binary instead of trusting the runner's UAC state: the runner user is an administrator.
  $bytes = [IO.File]::ReadAllBytes($installerItem.FullName)
  $text = [Text.Encoding]::ASCII.GetString($bytes)
  $level = [regex]::Match($text, 'requestedExecutionLevel\s+level="([^"]+)"').Groups[1].Value
  Check 'installer_asks_no_elevation' ($level -eq 'asInvoker') @{ requestedExecutionLevel = $level }
  $wv2Before = WebView2Version
  $t = [Diagnostics.Stopwatch]::StartNew()
  $p = Start-Process -FilePath $installerItem.FullName -ArgumentList '/S' -PassThru -Wait
  $secs = [math]::Round($t.Elapsed.TotalSeconds, 1)
  Check 'installer_exit_code' ($p.ExitCode -eq 0) @{ exit = $p.ExitCode; seconds = $secs; size_mb = [math]::Round($installerItem.Length / 1MB, 1) }
  $exe = Get-ChildItem -Path $installDir -Filter '*.exe' -ErrorAction SilentlyContinue |
    Where-Object { $_.Name -notlike 'uninstall*' } | Select-Object -First 1
  Check 'app_under_localappdata' ($null -ne $exe) @{ dir = $installDir; files = @(Get-ChildItem -Path $installDir -Recurse -File -ErrorAction SilentlyContinue | ForEach-Object { '{0} ({1} KB)' -f $_.FullName.Substring($installDir.Length + 1), [math]::Round($_.Length / 1KB) }) }
  Check 'uninstaller_present' (Test-Path (Join-Path $installDir 'uninstall.exe')) @{}
  $reg = Get-ItemProperty -Path $uninstallKey -ErrorAction SilentlyContinue
  Check 'uninstall_entry_hkcu' ($null -ne $reg) @{ DisplayName = $reg.DisplayName; DisplayVersion = $reg.DisplayVersion; UninstallString = $reg.UninstallString; Publisher = $reg.Publisher }
  Check 'no_uninstall_entry_hklm' (-not (Test-Path $machineKey)) @{}
  $lnk = Shortcuts
  Check 'start_menu_shortcut' ($lnk.Count -gt 0) @{ shortcuts = $lnk }
  $wv2 = WebView2Version
  Check 'webview2_present' ($null -ne $wv2) @{ before = $wv2Before; after = $wv2; note = 'GitHub runners ship WebView2, so the embedded bootstrapper is not exercised here' }
  $result = [ordered]@{ checks = $checks; failed = $fail; exe = $exe.FullName; webview2 = $wv2; os = (Get-CimInstance Win32_OperatingSystem | Select-Object Caption, Version, BuildNumber) }
  $result | ConvertTo-Json -Depth 6 | Set-Content -Path (Join-Path $Out 'install.json') -Encoding UTF8
  if ($fail.Count) { Write-Host "failed: $($fail -join ', ')"; exit 1 }
  Write-Output $exe.FullName
} else {
  $un = Join-Path $installDir 'uninstall.exe'
  $p = Start-Process -FilePath $un -ArgumentList '/S' -PassThru -Wait
  # NSIS copies the uninstaller to a temp folder and returns at once; wait for it to finish.
  $deadline = (Get-Date).AddSeconds(60)
  while ((Test-Path (Join-Path $installDir '*.exe')) -and (Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 500 }
  Check 'uninstaller_exit_code' ($p.ExitCode -eq 0) @{ exit = $p.ExitCode }
  Check 'app_files_removed' (-not (Test-Path (Join-Path $installDir '*.exe'))) @{ left = @(Get-ChildItem -Path $installDir -Recurse -ErrorAction SilentlyContinue | ForEach-Object FullName) }
  Check 'uninstall_entry_removed' (-not (Test-Path $uninstallKey)) @{}
  Check 'start_menu_shortcut_removed' ((Shortcuts).Count -eq 0) @{ left = Shortcuts }
  $data = Join-Path $env:LOCALAPPDATA 'app.vfs.desktop'
  $checks['user_data_after_uninstall'] = [ordered]@{ ok = $true; detail = @{ path = $data; exists = (Test-Path $data); note = 'reported, not judged: whether uninstall should delete the index is a product decision' } }
  [ordered]@{ checks = $checks; failed = $fail } | ConvertTo-Json -Depth 6 | Set-Content -Path (Join-Path $Out 'uninstall.json') -Encoding UTF8
  if ($fail.Count) { Write-Host "failed: $($fail -join ', ')"; exit 1 }
}
