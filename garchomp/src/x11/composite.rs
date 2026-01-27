//! X11 Composite extension operations.

use super::connection::{Connection, Result};
use x11rb::protocol::composite::{ConnectionExt as _, Redirect};
use x11rb::protocol::xfixes::ConnectionExt as XfixesConnectionExt;
use x11rb::protocol::xproto::{
    AtomEnum, ConfigureWindowAux, ConnectionExt as _, Pixmap, StackMode, Window,
};

/// Extension trait for Composite operations on Connection.
pub trait CompositeExt {
    /// Redirect all subwindows of root for compositing.
    fn redirect_subwindows(&self) -> Result<()>;

    /// Get the composite overlay window.
    fn get_overlay_window(&self) -> Result<Window>;

    /// Release the overlay window.
    fn release_overlay_window(&self) -> Result<()>;

    /// Get a pixmap for a window's contents.
    fn name_window_pixmap(&self, window: Window) -> Result<Pixmap>;

    /// Unredirect a window (for fullscreen bypass).
    fn unredirect_window(&self, window: Window) -> Result<()>;

    /// Re-redirect a window.
    fn redirect_window(&self, window: Window) -> Result<()>;

    /// Configure the overlay window to cover the screen.
    fn configure_overlay(&self, overlay: Window) -> Result<()>;

    /// Check if a window wants to bypass the compositor.
    fn wants_bypass(&self, window: Window) -> Result<bool>;
}

impl CompositeExt for Connection {
    fn redirect_subwindows(&self) -> Result<()> {
        self.conn
            .composite_redirect_subwindows(self.root(), Redirect::AUTOMATIC)?;
        tracing::debug!("Redirected subwindows for compositing");
        Ok(())
    }

    fn get_overlay_window(&self) -> Result<Window> {
        let reply = self.conn.composite_get_overlay_window(self.root())?.reply()?;
        tracing::debug!("Got overlay window: {:#x}", reply.overlay_win);
        Ok(reply.overlay_win)
    }

    fn release_overlay_window(&self) -> Result<()> {
        self.conn.composite_release_overlay_window(self.root())?;
        tracing::debug!("Released overlay window");
        Ok(())
    }

    fn name_window_pixmap(&self, window: Window) -> Result<Pixmap> {
        let pixmap = self.generate_id()?;
        self.conn.composite_name_window_pixmap(window, pixmap)?;
        Ok(pixmap)
    }

    fn unredirect_window(&self, window: Window) -> Result<()> {
        self.conn
            .composite_unredirect_window(window, Redirect::AUTOMATIC)?;
        tracing::debug!("Unredirected window {:#x}", window);
        Ok(())
    }

    fn redirect_window(&self, window: Window) -> Result<()> {
        self.conn
            .composite_redirect_window(window, Redirect::AUTOMATIC)?;
        tracing::debug!("Redirected window {:#x}", window);
        Ok(())
    }

    fn configure_overlay(&self, overlay: Window) -> Result<()> {
        let screen = self.screen();

        // Make overlay cover the entire screen
        let aux = ConfigureWindowAux::new()
            .x(0)
            .y(0)
            .width(screen.width_in_pixels as u32)
            .height(screen.height_in_pixels as u32)
            .stack_mode(StackMode::ABOVE);

        self.conn.configure_window(overlay, &aux)?;

        // Allow input to pass through to windows below
        let region = self.generate_id()?;
        self.conn.xfixes_create_region(region, &[])?;
        self.conn.xfixes_set_window_shape_region(overlay, x11rb::protocol::shape::SK::INPUT, 0, 0, region)?;
        self.conn.xfixes_destroy_region(region)?;

        tracing::debug!(
            "Configured overlay: {}x{}",
            screen.width_in_pixels,
            screen.height_in_pixels
        );
        Ok(())
    }

    fn wants_bypass(&self, window: Window) -> Result<bool> {
        let reply = self.conn.get_property(
            false,
            window,
            self.atoms._NET_WM_BYPASS_COMPOSITOR,
            AtomEnum::CARDINAL,
            0,
            1,
        )?.reply()?;

        if let Some(value) = reply.value32().and_then(|mut v| v.next()) {
            // 1 = bypass requested, 2 = bypass when fullscreen
            Ok(value >= 1)
        } else {
            Ok(false)
        }
    }
}
