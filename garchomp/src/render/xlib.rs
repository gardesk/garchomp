//! Xlib display and window handle wrappers for raw_window_handle integration.

use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawDisplayHandle,
    RawWindowHandle, WindowHandle, XlibDisplayHandle, XlibWindowHandle as RawXlibWindowHandle,
};
use std::ptr::NonNull;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum XlibError {
    #[error("failed to open X11 display")]
    OpenDisplay,
}

/// Wrapper around Xlib display connection.
///
/// Maintains the raw display pointer needed for wgpu surface creation.
pub struct XlibDisplay {
    display: *mut std::ffi::c_void,
    screen: i32,
}

impl XlibDisplay {
    /// Open connection to the default X11 display.
    ///
    /// # Safety
    /// The display pointer must remain valid for the lifetime of this struct.
    pub unsafe fn from_raw(display: *mut std::ffi::c_void, screen: i32) -> Self {
        Self { display, screen }
    }

    /// Get the raw display pointer.
    pub fn display_ptr(&self) -> *mut std::ffi::c_void {
        self.display
    }

    /// Get the screen number.
    pub fn screen(&self) -> i32 {
        self.screen
    }
}

// XlibDisplay doesn't own the display - x11rb does
// So we don't implement Drop to close it

unsafe impl Send for XlibDisplay {}
unsafe impl Sync for XlibDisplay {}

/// Window handle wrapper for raw_window_handle integration.
pub struct XlibWindowHandle {
    window: u64,
    display: *mut std::ffi::c_void,
    screen: i32,
}

impl XlibWindowHandle {
    /// Create a new window handle.
    pub fn new(window: u32, display: *mut std::ffi::c_void, screen: i32) -> Self {
        Self {
            window: window as u64,
            display,
            screen,
        }
    }
}

impl HasWindowHandle for XlibWindowHandle {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let handle = RawXlibWindowHandle::new(self.window);
        let raw = RawWindowHandle::Xlib(handle);
        // SAFETY: The window handle is valid for the lifetime of this struct
        Ok(unsafe { WindowHandle::borrow_raw(raw) })
    }
}

impl HasDisplayHandle for XlibWindowHandle {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        let display_ptr = NonNull::new(self.display);
        let handle = XlibDisplayHandle::new(display_ptr, self.screen);
        let raw = RawDisplayHandle::Xlib(handle);
        // SAFETY: The display handle is valid for the lifetime of this struct
        Ok(unsafe { DisplayHandle::borrow_raw(raw) })
    }
}

unsafe impl Send for XlibWindowHandle {}
unsafe impl Sync for XlibWindowHandle {}
