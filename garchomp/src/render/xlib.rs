//! Xlib display and window handle wrappers for raw_window_handle integration.

use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawDisplayHandle,
    RawWindowHandle, WindowHandle, XlibDisplayHandle, XlibWindowHandle as RawXlibWindowHandle,
};
use std::ptr::NonNull;
use thiserror::Error;
use x11_dl::xlib::Xlib;

#[derive(Error, Debug)]
pub enum XlibError {
    #[error("failed to load Xlib")]
    LoadXlib,
    #[error("failed to open X11 display")]
    OpenDisplay,
}

/// Wrapper around Xlib display connection.
///
/// This opens a separate Xlib connection for GPU surface creation,
/// independent of the x11rb connection used for compositor protocol.
pub struct XlibDisplay {
    xlib: Xlib,
    display: *mut x11_dl::xlib::Display,
}

impl XlibDisplay {
    /// Open connection to the default X11 display.
    pub fn open() -> Result<Self, XlibError> {
        let xlib = Xlib::open().map_err(|_| XlibError::LoadXlib)?;

        // SAFETY: XOpenDisplay with NULL opens the default display
        let display = unsafe { (xlib.XOpenDisplay)(std::ptr::null()) };

        if display.is_null() {
            return Err(XlibError::OpenDisplay);
        }

        Ok(Self { xlib, display })
    }

    /// Get the raw display pointer.
    pub fn display_ptr(&self) -> *mut std::ffi::c_void {
        self.display as *mut std::ffi::c_void
    }

    /// Get the default screen number.
    pub fn default_screen(&self) -> i32 {
        // SAFETY: display is valid, XDefaultScreen returns screen number
        unsafe { (self.xlib.XDefaultScreen)(self.display) }
    }

    /// Flush the display (send all pending requests).
    pub fn flush(&self) {
        // SAFETY: display is valid
        unsafe { (self.xlib.XFlush)(self.display) };
    }

    /// Sync the display (flush and wait for all requests to complete).
    pub fn sync(&self) {
        // SAFETY: display is valid
        unsafe { (self.xlib.XSync)(self.display, 0) };
    }
}

impl Drop for XlibDisplay {
    fn drop(&mut self) {
        // SAFETY: display is valid and we own it
        unsafe { (self.xlib.XCloseDisplay)(self.display) };
    }
}

// SAFETY: The Xlib display is thread-safe when properly synchronized
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
