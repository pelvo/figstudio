//! Bringing figstudio's window forward on Windows when a later launch hands it files
//! (`handoff.rs`). That launch passed this process the right to take the foreground
//! (`AllowSetForegroundWindow`). egui's `Focus` goes through winit, which first sends an
//! Alt key press with `SendInput`, and that input can cancel the right, so the window
//! would only flash in the taskbar. Here the window asks for the foreground itself, with
//! no input in between.

use raw_window_handle::{HasWindowHandle, RawWindowHandle};

/// figstudio's top-level window.
#[derive(Debug)]
pub(crate) struct Window(isize);

impl Window {
    /// The window eframe created for this app, if it is a Win32 one.
    pub(crate) fn of(cc: &eframe::CreationContext<'_>) -> Option<Self> {
        match cc.window_handle().ok()?.as_raw() {
            RawWindowHandle::Win32(handle) => Some(Self(handle.hwnd.get())),
            _ => None,
        }
    }

    /// Asks for the foreground (call on the window's thread, after files were handed over).
    /// A minimized window is restored first: Windows does not give the foreground to one,
    /// and egui's restore comes only after this frame, too late for the right the launch
    /// passed on.
    pub(crate) fn come_forward(&self) {
        use windows::Win32::Foundation::HWND;
        use windows::Win32::UI::WindowsAndMessaging::{
            IsIconic, SW_RESTORE, SetForegroundWindow, ShowWindow,
        };
        let hwnd = HWND(self.0 as *mut core::ffi::c_void);
        #[expect(
            unsafe_code,
            reason = "taking the foreground is a Win32 call (approved by the owner)"
        )]
        // SAFETY: `hwnd` is the live top-level window eframe created for this process (it
        // lives as long as the app, which owns this), and this runs on its UI thread.
        // `IsIconic`, `ShowWindow` and `SetForegroundWindow` take the handle (and a show
        // command) by value and touch no memory of ours; a refusal is only a `false`
        // result.
        let taken = unsafe {
            if IsIconic(hwnd).as_bool() {
                let _ = ShowWindow(hwnd, SW_RESTORE);
            }
            SetForegroundWindow(hwnd)
        };
        if !taken.as_bool() {
            tracing::warn!("single instance: Windows kept the foreground elsewhere");
        }
    }
}
