use single_instance::SingleInstance;

/// Guards against a second `ferric-qt` process running. If another instance
/// is already up, brings its window to the foreground (Windows) and returns
/// `false` — caller should exit immediately without creating a QGuiApplication
/// at all. Returns `true` if this is the only instance and startup should
/// proceed normally.
pub fn acquire_or_focus_existing() -> bool {
    let instance = match SingleInstance::new("ferric-qt") {
        Ok(i) => i,
        Err(e) => {
            tracing::warn!("single-instance check failed ({e}), proceeding anyway");
            return true;
        }
    };

    if instance.is_single() {
        // Leak the guard deliberately — it must live for the process lifetime
        // to hold the lock; there is no natural owner to keep it in cxx-qt's
        // QGuiApplication::exec() loop, and the OS reclaims it on exit either way.
        std::mem::forget(instance);
        return true;
    }

    tracing::info!("ferric-qt already running — focusing existing window");
    focus_existing_window();
    false
}

#[cfg(windows)]
fn focus_existing_window() {
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        FindWindowW, SetForegroundWindow, ShowWindow, SW_RESTORE,
    };

    let title: Vec<u16> = "Ferric\0".encode_utf16().collect();
    // SAFETY: FindWindowW with a null class name and a valid, nul-terminated
    // UTF-16 window title is a standard, documented Win32 call; the returned
    // HWND (if any) is only ever passed to other Win32 calls, never dereferenced.
    let hwnd: HWND = unsafe { FindWindowW(std::ptr::null(), title.as_ptr()) };
    if hwnd.is_null() {
        // Started hidden at login (--autostart): the window does not exist natively yet, so ask the
        // running copy, through the daemon, to show itself.
        if let Err(e) = crate::command_bus::request_show_ui() {
            tracing::warn!("could not find existing Ferric window to focus: {e}");
        }
        return;
    }
    // SAFETY: hwnd was just returned by FindWindowW and checked non-null.
    unsafe {
        ShowWindow(hwnd, SW_RESTORE);
        SetForegroundWindow(hwnd);
    }
}

#[cfg(not(windows))]
fn focus_existing_window() {
    tracing::warn!("focus-existing-window not implemented on this platform");
}
