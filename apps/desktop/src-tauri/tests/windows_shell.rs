//! End-to-end checks against the real Windows shell, run by the Windows CI job: "Mostrar en
//! carpeta" must leave the item selected in an Explorer window (VFS-1 AC-13, issue 107), and
//! the first-launch splash must appear and go away (issue 108). Explorer's selection is read
//! back through the Shell's own automation object (`Shell.Application`), the same view a user
//! sees.
#![cfg(windows)]

use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use vfs_core::api::RevealOutcome;
use vfs_desktop_lib::reveal;
use vfs_desktop_lib::splash::{Splash, SPLASH_CLASS, SPLASH_DELAY};

/// Polls every open Explorer window for up to 20 s and prints the paths selected in it, one
/// per line, as soon as one of them ends with `$env:VFS_NAME`; then prints one `window:` line
/// per Shell window and the Explorer processes (context for a failure), and closes the
/// windows that show `$env:VFS_DIR_NAME`.
const SELECTED_ITEMS_PS: &str = r#"
$deadline = (Get-Date).AddSeconds(20)
$shell = New-Object -ComObject Shell.Application
do {
  $sel = @()
  foreach ($w in @($shell.Windows())) {
    try { foreach ($i in @($w.Document.SelectedItems())) { $sel += $i.Path } } catch {}
  }
  if (@($sel | Where-Object { $_.EndsWith('\' + $env:VFS_NAME) }).Count -gt 0) { break }
  Start-Sleep -Milliseconds 300
} while ((Get-Date) -lt $deadline)
$sel | ForEach-Object { $_ }
foreach ($w in @($shell.Windows())) {
  try { Write-Output ("window: " + $w.FullName + " | " + $w.LocationURL + " | " + $w.Document.Folder.Self.Path) } catch { Write-Output ("window: " + $w.FullName + " (no folder view)") }
}
Get-Process explorer -ErrorAction SilentlyContinue | ForEach-Object { Write-Output ("explorer pid " + $_.Id + " session " + $_.SessionId) }
Write-Output ("this session " + (Get-Process -Id $PID).SessionId)
foreach ($w in @($shell.Windows())) {
  try { if ($w.Document.Folder.Self.Name -eq $env:VFS_DIR_NAME) { $w.Quit() } } catch {}
}
"#;

/// Paths selected in Explorer windows once `name` is among them (or after the timeout).
fn selected_in_explorer(name: &str, dir: &Path) -> Vec<String> {
    let dir_name = dir.file_name().unwrap().to_string_lossy().into_owned();
    let out = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            SELECTED_ITEMS_PS,
        ])
        .env("VFS_NAME", name)
        .env("VFS_DIR_NAME", dir_name)
        .output()
        .expect("run powershell");
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::to_owned)
        .collect()
}

/// VFS-1 AC-13 (issue 107): revealing a file and a folder opens their parent in Explorer with
/// that very item selected.
#[test]
fn reveal_selects_the_item_in_explorer() {
    let root = tempfile::Builder::new()
        .prefix("vfs-reveal-")
        .tempdir()
        .unwrap();
    let dir = root.path().join("Año 2019");
    std::fs::create_dir(&dir).unwrap();
    let file = dir.join("Notaría, copia.pdf");
    std::fs::write(&file, b"x").unwrap();
    let sub = dir.join("Fotos boda");
    std::fs::create_dir(&sub).unwrap();

    for item in [&file, &sub] {
        let name = item.file_name().unwrap().to_string_lossy().into_owned();
        assert_eq!(reveal::reveal(item).unwrap(), RevealOutcome::Revealed);
        let selected = selected_in_explorer(&name, &dir);
        assert!(
            selected.iter().any(|p| p.ends_with(&format!(r"\{name}"))),
            "{name} not selected in Explorer; selected: {selected:?}"
        );
    }
}

fn splash_window() -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::FindWindowW;
    let class: Vec<u16> = SPLASH_CLASS.encode_utf16().chain([0]).collect();
    // SAFETY: `class` is NUL-terminated and outlives the call.
    !unsafe { FindWindowW(class.as_ptr(), std::ptr::null()) }.is_null()
}

fn wait_for(want: bool, within: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < within {
        if splash_window() == want {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

/// Issue 108: the splash window appears shortly after start and is gone once closed; closed
/// before its delay, it never appears.
#[test]
fn splash_appears_and_closes() {
    let splash = Splash::show();
    assert!(
        wait_for(true, SPLASH_DELAY + Duration::from_secs(5)),
        "splash never appeared"
    );
    splash.close();
    assert!(wait_for(false, Duration::from_secs(5)), "splash still open");

    let early = Splash::show();
    early.close();
    std::thread::sleep(SPLASH_DELAY * 3);
    assert!(
        !splash_window(),
        "a splash closed before its delay appeared"
    );
}
