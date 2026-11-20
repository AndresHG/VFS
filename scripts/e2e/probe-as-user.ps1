<#
.SYNOPSIS
  VFS-5 AC-7: rehearses docs/research/tools/vfs-probe.ps1 on a Windows runner. The runner user
  is an administrator, so the probe runs as a standard user created here (a scheduled task with
  that user's credentials, "run with lowest privileges"), and once more as the administrator
  for the USN step only, so the two can be compared.

  probe-as-user.ps1 -Probe <vfs-probe.ps1> -DataRoot <folder> -Out <dir>
#>
param(
  [Parameter(Mandatory = $true)] [string] $Probe,
  [Parameter(Mandatory = $true)] [string] $DataRoot,
  [Parameter(Mandatory = $true)] [string] $Out,
  [int] $Minutes = 1
)
$ErrorActionPreference = 'Stop'
$user = 'vfsprobe'
$work = 'C:\vfs-probe-rehearsal'
New-Item -ItemType Directory -Force -Path $work, $Out | Out-Null
Copy-Item $Probe (Join-Path $work 'vfs-probe.ps1')

# A random password that meets the default complexity policy; it never leaves this job.
$chars = [char[]]'abcdefghijkmnopqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789'
$pw = (-join (1..20 | ForEach-Object { $chars | Get-Random })) + 'a#7Z'
net user $user $pw /add /y | Out-Null
$groups = (net user $user) -join ' '
Write-Host "created $user; groups: $(([regex]::Match($groups, 'Local Group Memberships\s+(.*?)Global')).Groups[1].Value.Trim())"

# A scheduled task with a password needs "Log on as a batch job", which only administrators
# hold by default on Windows Server. Grant it to this user alone.
$sid = (New-Object Security.Principal.NTAccount($user)).Translate([Security.Principal.SecurityIdentifier]).Value
secedit /export /cfg (Join-Path $work 'sec.inf') /areas USER_RIGHTS | Out-Null
$inf = Get-Content (Join-Path $work 'sec.inf')
if ($inf -match '^SeBatchLogonRight') {
  $inf = $inf -replace '^(SeBatchLogonRight\s*=\s*.*)$', "`$1,*$sid"
} else {
  $inf = $inf -replace '^\[Privilege Rights\]$', "[Privilege Rights]`r`nSeBatchLogonRight = *$sid"
}
$inf | Set-Content (Join-Path $work 'sec2.inf') -Encoding Unicode
secedit /configure /db (Join-Path $work 'sec.sdb') /cfg (Join-Path $work 'sec2.inf') /areas USER_RIGHTS | Out-Null

$userOut = Join-Path $work 'out'
New-Item -ItemType Directory -Force -Path $userOut | Out-Null
icacls $userOut /grant "${user}:(OI)(CI)M" | Out-Null
$scratch = Join-Path $work 'scratch'
New-Item -ItemType Directory -Force -Path $scratch | Out-Null
icacls $scratch /grant "${user}:(OI)(CI)M" | Out-Null

$arg = "-NoProfile -ExecutionPolicy Bypass -File `"$work\vfs-probe.ps1`" -DataRoot `"$DataRoot`" -Step all " +
       "-Minutes $Minutes -WalkMinutes 2 -OutDir `"$userOut`" -ScratchParent `"$scratch`""
$action = New-ScheduledTaskAction -Execute 'powershell.exe' -Argument $arg
Register-ScheduledTask -TaskName 'vfs-probe' -Action $action -User $user -Password $pw -RunLevel Limited | Out-Null
$t = [Diagnostics.Stopwatch]::StartNew()
Start-ScheduledTask -TaskName 'vfs-probe'
Start-Sleep -Seconds 5
while ((Get-ScheduledTask -TaskName 'vfs-probe').State -eq 'Running' -and $t.Elapsed.TotalMinutes -lt 40) { Start-Sleep -Seconds 5 }
$info = Get-ScheduledTaskInfo -TaskName 'vfs-probe'
Write-Host ("standard-user probe finished in {0:N0} s, LastTaskResult 0x{1:X}" -f $t.Elapsed.TotalSeconds, $info.LastTaskResult)
Copy-Item -Recurse -Force (Join-Path $userOut '*') (Join-Path $Out 'standard-user')

# The USN calls and the counter sampling with the administrator's token, for comparison.
foreach ($step in @('usn', 'perf')) {
  & powershell.exe -NoProfile -ExecutionPolicy Bypass -File (Join-Path $work 'vfs-probe.ps1') `
    -DataRoot $DataRoot -Step $step -Minutes 1 -OutDir (Join-Path $Out 'administrator')
}

Unregister-ScheduledTask -TaskName 'vfs-probe' -Confirm:$false
net user $user /delete | Out-Null
if ($info.LastTaskResult -ne 0) { throw ("standard-user probe exited 0x{0:X}" -f $info.LastTaskResult) }
