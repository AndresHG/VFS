<#
.SYNOPSIS
  VFS-8 AC-6: the slow-disk runs on a Windows runner. The indexed tree and the index live on a
  separate virtual disk (a VHDX mounted as V:); the cache is made cold by dismounting and
  remounting it; "busy" adds a competing workload on the same disk (DiskSpd at a capped rate,
  plus contention.py emulating a sync client and an on-access scanner).

  slow-disk.ps1 -Action run -Exe <installed exe of the webdriver build> -Cli <vfs.exe> -Out <dir>
  slow-disk.ps1 -Action cycle [-Busy]   # used between runs: stop load, remount, restart load

  Nothing here throttles I/O or adds seek latency: no tool does that on a hosted runner
  without a custom driver (see specs/vfs-8-windows-e2e/spec.md, decision 3).
#>
param(
  [ValidateSet('run', 'cycle')] [string] $Action = 'run',
  [string] $Exe = '',
  [string] $Cli = '',
  [string] $Out = '',
  [switch] $Busy,
  [int] $Files = 100000,
  [int] $BenchFiles = 2000000,
  # DiskSpd throughput cap in bytes per millisecond per thread (-g); 8192 is about 8 MB/s.
  [int] $DiskSpdRate = 8192
)
$ErrorActionPreference = 'Stop'
# The two disk conditions; the label names measure.py's output files.
$Quiet = 'quiet'
$Loaded = 'busy'
$Conditions = @($Quiet, $Loaded)
$here = $PSScriptRoot
$work = 'C:\vfs-e2e'
$vhd = Join-Path $work 'data.vhdx'
$state = Join-Path $work 'state'
$py = (Get-Command python).Source

function Log([string] $m) { Write-Host ('{0:HH:mm:ss} {1}' -f (Get-Date), $m) }

function Mount-Data {
  Mount-DiskImage -ImagePath $vhd -StorageType VHDX | Out-Null
  $part = Get-DiskImage -ImagePath $vhd | Get-Disk | Get-Partition | Where-Object Type -eq 'Basic'
  if ($part.DriveLetter -ne 'V') { $part | Set-Partition -NewDriveLetter V }
}

function Dismount-Data { Dismount-DiskImage -ImagePath $vhd | Out-Null }

function Start-Load {
  $ds = Join-Path $work 'diskspd\amd64\diskspd.exe'
  # Random 64 KiB I/O, 30 % writes, one thread, two outstanding, capped rate, no OS caching.
  $dsArgs = @('-c4G', '-d7200', '-r', '-w30', '-b64K', '-t1', '-o2', "-g$DiskSpdRate", '-Sh', 'V:\load.dat')
  $p = Start-Process -FilePath $ds -ArgumentList $dsArgs -PassThru `
    -RedirectStandardOutput (Join-Path $state 'diskspd.out.txt')
  Remove-Item -Force -ErrorAction SilentlyContinue (Join-Path $state 'stop')
  $c = Start-Process -FilePath $py -PassThru -WindowStyle Hidden -ArgumentList @(
    (Join-Path $here 'contention.py'), 'V:\Archivo', '--stop-file', (Join-Path $state 'stop'),
    '--report', (Join-Path $state ('contention-{0:HHmmss}.json' -f (Get-Date))))
  Set-Content -Path (Join-Path $state 'load.pids') -Value "$($p.Id) $($c.Id)"
  Start-Sleep -Seconds 5
  Log "load started (diskspd $($p.Id), contention $($c.Id))"
}

function Stop-Load {
  $f = Join-Path $state 'load.pids'
  if (-not (Test-Path $f)) { return }
  $ids = (Get-Content $f).Split(' ')
  Set-Content -Path (Join-Path $state 'stop') -Value 'stop'
  Stop-Process -Id ([int]$ids[0]) -Force -ErrorAction SilentlyContinue
  Wait-Process -Id ([int]$ids[1]) -Timeout 30 -ErrorAction SilentlyContinue
  Stop-Process -Id ([int]$ids[1]) -Force -ErrorAction SilentlyContinue
  Remove-Item $f
  Start-Sleep -Seconds 2
  Log 'load stopped'
}

function Cycle([bool] $busy) {
  Stop-Load
  Dismount-Data
  Mount-Data
  Log 'V: remounted (cold cache)'
  if ($busy) { Start-Load }
}

if ($Action -eq 'cycle') { Cycle $Busy.IsPresent; exit 0 }

New-Item -ItemType Directory -Force -Path $work, $state, $Out | Out-Null
$hw = [ordered]@{
  cpu = (Get-CimInstance Win32_Processor | Select-Object -First 1).Name
  cores = (Get-CimInstance Win32_Processor | Measure-Object NumberOfLogicalProcessors -Sum).Sum
  ram_gb = [math]::Round((Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory / 1GB, 1)
  os = (Get-CimInstance Win32_OperatingSystem).Caption + ' ' + (Get-CimInstance Win32_OperatingSystem).Version
  host_disk = (Get-PhysicalDisk | Select-Object FriendlyName, MediaType, BusType, @{n = 'GB'; e = { [math]::Round($_.Size / 1GB) } })
  vhdx_on = $vhd
}
$hw | ConvertTo-Json -Depth 4 | Set-Content (Join-Path $Out 'hardware.json') -Encoding UTF8

# DiskSpd: Microsoft's release, accepted only with a valid Microsoft signature.
Invoke-WebRequest -Uri 'https://github.com/microsoft/diskspd/releases/download/v2.3/DiskSpd.ZIP' -OutFile (Join-Path $work 'diskspd.zip')
Expand-Archive -Path (Join-Path $work 'diskspd.zip') -DestinationPath (Join-Path $work 'diskspd') -Force
$sig = Get-AuthenticodeSignature (Join-Path $work 'diskspd\amd64\diskspd.exe')
if ($sig.Status -ne 'Valid' -or $sig.SignerCertificate.Subject -notmatch 'Microsoft') { throw "diskspd signature: $($sig.Status)" }

# The virtual disk: 64 GB, dynamically expanding, NTFS, mounted as V:.
Set-Content -Path (Join-Path $work 'create.txt') -Encoding ASCII -Value @"
create vdisk file="$vhd" maximum=65536 type=expandable
select vdisk file="$vhd"
attach vdisk
create partition primary
format fs=ntfs quick label=VFSDATA
assign letter=V
"@
diskpart /s (Join-Path $work 'create.txt') | Out-Host
Log 'generating the tree on V:'
& $py (Join-Path $here 'gen_tree.py') 'V:\Archivo' --files $Files --manifest 'V:\manifest.json'
if ($LASTEXITCODE) { throw 'gen_tree failed' }
Copy-Item 'V:\manifest.json' (Join-Path $Out 'manifest.json')

# The index follows the tree onto V: through a junction; the app itself is unchanged.
New-Item -ItemType Directory -Force -Path 'V:\appdata' | Out-Null
$appData = Join-Path $env:LOCALAPPDATA 'app.vfs.desktop'
if (Test-Path $appData) { Remove-Item -Recurse -Force $appData }
New-Item -ItemType Junction -Path $appData -Target 'V:\appdata' | Out-Null

$failed = @()
foreach ($label in $Conditions) {
  $busy = $label -eq $Loaded
  Get-ChildItem 'V:\appdata' -Force | Remove-Item -Recurse -Force
  Cycle $busy
  $between = "powershell -NoProfile -ExecutionPolicy Bypass -File `"$PSCommandPath`" -Action cycle" + $(if ($busy) { ' -Busy' } else { '' })
  Log "measure $label"
  & $py (Join-Path $here 'measure.py') --app $Exe --manifest 'V:\manifest.json' --label $label --out $Out --between $between
  if ($LASTEXITCODE) { $failed += "measure-$label" }
  Copy-Item -Recurse -Force 'V:\appdata\logs' (Join-Path $Out "app-logs-$label") -ErrorAction SilentlyContinue
  Get-ChildItem 'V:\appdata' -Filter 'index.db*' | Select-Object Name, Length |
    ConvertTo-Json | Set-Content (Join-Path $Out "index-files-$label.json")
  Stop-Load
}

# The same first scan by the CLI: foreground priority, no app, no watcher, for comparison.
Cycle $false
New-Item -ItemType Directory -Force -Path 'V:\cli' | Out-Null
Log 'CLI first scan (foreground, cold V:)'
& $Cli index --db 'V:\cli\index.db' 'V:\Archivo' | Set-Content (Join-Path $Out 'cli-index-quiet.txt') -Encoding UTF8
Remove-Item -Recurse -Force 'V:\cli'

Log 'generating the 2,000,000-file synthetic index on V:'
New-Item -ItemType Directory -Force -Path 'V:\bench' | Out-Null
$t = [Diagnostics.Stopwatch]::StartNew()
& $Cli gen-db --files $BenchFiles --out 'V:\bench\bench.db' | Out-Host
Log ("gen-db {0:N0} s" -f $t.Elapsed.TotalSeconds)
foreach ($label in $Conditions) {
  Cycle ($label -eq $Loaded)
  Log "bench $label (cold V:)"
  # A non-zero exit is the AC-9 verdict (budgets for an SSD), recorded in the JSON, not a
  # failure of this run.
  & $Cli bench --db 'V:\bench\bench.db' --json | Set-Content (Join-Path $Out "bench-$label.json") -Encoding UTF8
  Stop-Load
}
Copy-Item (Join-Path $state 'contention-*.json'), (Join-Path $state 'diskspd.out.txt') $Out -ErrorAction SilentlyContinue
if ($failed.Count) { throw "failed: $($failed -join ', ')" }
