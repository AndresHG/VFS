//! First-launch splash (issue 108). On a fresh Windows machine WebView2 needs tens of seconds
//! before it paints the first page. The main window therefore starts hidden (`tauri.conf.json`,
//! shown once its page has loaded) and a small native window saying "Preparando VFS…" (in the OS language) shows at
//! once instead; [`Splash::close`] removes it when the main window is shown. It is plain
//! Win32 (no web view, no extra dependency) on its own thread with its own message loop, so
//! it paints while the main thread waits on WebView2. On macOS the page
//! paints in under a second and there is no splash.

/// Window class of the splash, by which tests find it.
pub const SPLASH_CLASS: &str = "VfsSplash";

/// Splash size in logical pixels (96 DPI).
pub const SPLASH_SIZE: (i32, i32) = (360, 120);

/// Delay before the splash appears, so a second launch, which only focuses the running
/// window and exits, never flashes it.
pub const SPLASH_DELAY: std::time::Duration = std::time::Duration::from_millis(300);

/// Top-left corner of a `size` window centred in the work area `(left, top, right, bottom)`.
/// A window larger than the area is pinned to its top-left corner.
pub fn centered(area: (i32, i32, i32, i32), size: (i32, i32)) -> (i32, i32) {
    let (left, top, right, bottom) = area;
    (
        left + ((right - left - size.0) / 2).max(0),
        top + ((bottom - top - size.1) / 2).max(0),
    )
}

#[cfg(windows)]
pub use imp::Splash;

/// No splash where the web view paints at once.
#[cfg(not(windows))]
#[derive(Default)]
pub struct Splash;

#[cfg(not(windows))]
impl Splash {
    pub fn show() -> Self {
        Splash
    }

    pub fn close(&self) {}
}

#[cfg(windows)]
mod imp {
    use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
    use std::sync::Arc;

    use windows_sys::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
    use windows_sys::Win32::Graphics::Gdi::{
        BeginPaint, CreateFontW, CreateSolidBrush, DeleteObject, DrawTextW, EndPaint, FillRect,
        SelectObject, SetBkMode, SetTextColor, CLEARTYPE_QUALITY, DEFAULT_CHARSET, DT_CENTER,
        DT_SINGLELINE, DT_VCENTER, FW_NORMAL, PAINTSTRUCT, TRANSPARENT,
    };
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::HiDpi::GetDpiForSystem;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DispatchMessageW, GetClientRect, GetMessageW, LoadCursorW,
        LoadIconW, PostMessageW, PostQuitMessage, RegisterClassW, SystemParametersInfoW,
        TranslateMessage, IDC_ARROW, MSG, SPI_GETWORKAREA, WM_CLOSE, WM_DESTROY, WM_PAINT,
        WNDCLASSW, WS_BORDER, WS_EX_APPWINDOW, WS_POPUP, WS_VISIBLE,
    };

    use super::{centered, SPLASH_CLASS, SPLASH_DELAY, SPLASH_SIZE};

    /// Papel's `--paper` and `--ink` (`ui/styles.css`), as COLORREF (0x00BBGGRR).
    const PAPER: COLORREF = 0x00f3_f8fa;
    const INK: COLORREF = 0x001b_1e1f;
    /// Icon resource id tauri-build gives the app icon (`IDI_APPLICATION`).
    const APP_ICON_ID: usize = 32512;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain([0]).collect()
    }

    /// The splash window, shown by [`Splash::show`] and removed by [`Splash::close`]; held as
    /// Tauri managed state.
    pub struct Splash {
        hwnd: Arc<AtomicIsize>,
        closed: Arc<AtomicBool>,
    }

    impl Splash {
        /// Starts the splash thread; the window appears after [`SPLASH_DELAY`] unless
        /// [`Splash::close`] came first.
        pub fn show() -> Self {
            let splash = Splash {
                hwnd: Arc::default(),
                closed: Arc::default(),
            };
            let (hwnd, closed) = (splash.hwnd.clone(), splash.closed.clone());
            let spawned = std::thread::Builder::new()
                .name("splash".into())
                .spawn(move || {
                    std::thread::sleep(SPLASH_DELAY);
                    if !closed.load(Ordering::SeqCst) {
                        run(&hwnd, &closed);
                    }
                });
            if let Err(e) = spawned {
                log::warn!("splash thread: {e}");
            }
            splash
        }

        /// Removes the splash (or stops it from appearing). Safe to call more than once.
        pub fn close(&self) {
            self.closed.store(true, Ordering::SeqCst);
            let hwnd = self.hwnd.load(Ordering::SeqCst);
            if hwnd != 0 {
                // SAFETY: posting to a window that may already be gone is harmless.
                unsafe { PostMessageW(hwnd as HWND, WM_CLOSE, 0, 0) };
            }
        }
    }

    /// Creates the window and pumps its messages until it is destroyed.
    fn run(slot: &AtomicIsize, closed: &AtomicBool) {
        let class = wide(SPLASH_CLASS);
        let title = wide("VFS");
        // SAFETY: plain Win32 calls on this thread; every pointer passed is to a live local.
        unsafe {
            let instance = GetModuleHandleW(std::ptr::null());
            let wc = WNDCLASSW {
                style: 0,
                lpfnWndProc: Some(wndproc),
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: instance,
                hIcon: LoadIconW(instance, APP_ICON_ID as *const u16),
                hCursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW),
                hbrBackground: std::ptr::null_mut(),
                lpszMenuName: std::ptr::null(),
                lpszClassName: class.as_ptr(),
            };
            RegisterClassW(&wc);
            let dpi = GetDpiForSystem() as i32;
            let size = (SPLASH_SIZE.0 * dpi / 96, SPLASH_SIZE.1 * dpi / 96);
            let mut area: RECT = std::mem::zeroed();
            SystemParametersInfoW(SPI_GETWORKAREA, 0, (&mut area as *mut RECT).cast(), 0);
            let (x, y) = centered((area.left, area.top, area.right, area.bottom), size);
            let hwnd = CreateWindowExW(
                WS_EX_APPWINDOW,
                class.as_ptr(),
                title.as_ptr(),
                WS_POPUP | WS_BORDER | WS_VISIBLE,
                x,
                y,
                size.0,
                size.1,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                instance,
                std::ptr::null(),
            );
            if hwnd.is_null() {
                log::warn!("splash window not created");
                return;
            }
            slot.store(hwnd as isize, Ordering::SeqCst);
            // `close` may have run between the check in `show` and the store above.
            if closed.load(Ordering::SeqCst) {
                PostMessageW(hwnd, WM_CLOSE, 0, 0);
            }
            let mut msg: MSG = std::mem::zeroed();
            while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }

    unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        match msg {
            WM_PAINT => {
                paint(hwnd);
                0
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                0
            }
            // WM_CLOSE falls through: DefWindowProcW destroys the window.
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }

    /// Paper background with the text centred in Segoe UI at 20 px (logical).
    unsafe fn paint(hwnd: HWND) {
        let mut ps: PAINTSTRUCT = std::mem::zeroed();
        let hdc = BeginPaint(hwnd, &mut ps);
        let mut rect: RECT = std::mem::zeroed();
        GetClientRect(hwnd, &mut rect);
        let brush = CreateSolidBrush(PAPER);
        FillRect(hdc, &rect, brush);
        DeleteObject(brush);
        let face = wide("Segoe UI");
        let font = CreateFontW(
            -(20 * GetDpiForSystem() as i32 / 96),
            0,
            0,
            0,
            FW_NORMAL as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            0,
            0,
            CLEARTYPE_QUALITY as u32,
            0,
            face.as_ptr(),
        );
        let old = SelectObject(hdc, font);
        SetBkMode(hdc, TRANSPARENT as i32);
        SetTextColor(hdc, INK);
        // VFS-22 R5: drawn before the UI runs, so in the OS language.
        let splash = crate::i18n::Lang::system().strings().splash;
        let mut text: Vec<u16> = splash.encode_utf16().collect();
        DrawTextW(
            hdc,
            text.as_mut_ptr(),
            text.len() as i32,
            &mut rect,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE,
        );
        SelectObject(hdc, old);
        DeleteObject(font);
        EndPaint(hwnd, &ps);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Issue 108: the splash sits in the middle of the work area, and a window larger than
    /// the area is pinned to its corner rather than pushed off screen.
    #[test]
    fn splash_is_centred_in_the_work_area() {
        assert_eq!(centered((0, 0, 1024, 728), (360, 120)), (332, 304));
        assert_eq!(centered((100, 50, 1124, 778), (360, 120)), (432, 354));
        assert_eq!(centered((0, 0, 300, 100), (360, 120)), (0, 0));
    }
}
