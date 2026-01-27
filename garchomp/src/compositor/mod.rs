//! Core compositor state and event handling.

mod window;

pub use window::{TrackedWindow, WindowType};

use crate::render::{GpuError, Renderer, WindowRenderInfo};
use crate::x11::{CompositeExt, Connection};
use std::collections::HashMap;
use thiserror::Error;
use x11rb::connection::Connection as _;
use x11rb::protocol::damage::{ConnectionExt as DamageConnectionExt, ReportLevel};
use x11rb::protocol::xproto::{
    AtomEnum, ChangeWindowAttributesAux, ConfigureNotifyEvent, ConnectionExt, CreateNotifyEvent,
    DestroyNotifyEvent, EventMask, MapNotifyEvent, PropertyNotifyEvent,
    UnmapNotifyEvent, Window,
};
use x11rb::protocol::Event;

#[derive(Error, Debug)]
pub enum CompositorError {
    #[error("X11 connection error: {0}")]
    Connection(#[from] crate::x11::ConnectionError),

    #[error("X11 error: {0}")]
    X11(#[from] x11rb::errors::ConnectionError),

    #[error("X11 reply error: {0}")]
    Reply(#[from] x11rb::errors::ReplyError),

    #[error("GPU error: {0}")]
    Gpu(#[from] GpuError),
}

pub type Result<T> = std::result::Result<T, CompositorError>;

/// The main compositor state.
pub struct Compositor {
    pub conn: Connection,
    pub overlay: Window,
    pub renderer: Renderer,
    pub windows: HashMap<Window, TrackedWindow>,
    pub running: bool,
    needs_redraw: bool,
}

impl Compositor {
    /// Create a new compositor instance.
    pub async fn new() -> Result<Self> {
        let conn = Connection::new()?;

        // Redirect all windows for compositing
        conn.redirect_subwindows()?;

        // Get the overlay window
        let overlay = conn.get_overlay_window()?;
        conn.configure_overlay(overlay)?;

        // Get screen dimensions for GPU surface
        let screen = conn.screen();
        let width = screen.width_in_pixels as u32;
        let height = screen.height_in_pixels as u32;

        // Initialize GPU renderer
        tracing::info!("Initializing GPU renderer for {}x{} surface", width, height);
        let renderer = Renderer::new(overlay, width, height).await?;
        tracing::info!("GPU renderer initialized");

        // Subscribe to events on root window
        let event_mask = EventMask::SUBSTRUCTURE_NOTIFY
            | EventMask::STRUCTURE_NOTIFY
            | EventMask::PROPERTY_CHANGE;

        conn.conn.change_window_attributes(
            conn.root(),
            &ChangeWindowAttributesAux::new().event_mask(event_mask),
        )?;

        conn.flush()?;

        let mut compositor = Self {
            conn,
            overlay,
            renderer,
            windows: HashMap::new(),
            running: true,
            needs_redraw: true,
        };

        // Scan existing windows
        compositor.scan_windows()?;

        Ok(compositor)
    }

    /// Scan for existing windows on startup.
    fn scan_windows(&mut self) -> Result<()> {
        let tree = self.conn.conn.query_tree(self.conn.root())?.reply()?;

        // Collect windows to track (avoid borrow issues)
        let mut to_track = Vec::new();
        for window in tree.children {
            if window == self.overlay {
                continue;
            }

            if let Ok(attrs) = self.conn.conn.get_window_attributes(window) {
                if let Ok(attrs) = attrs.reply() {
                    if attrs.map_state != x11rb::protocol::xproto::MapState::UNMAPPED {
                        to_track.push(window);
                    }
                }
            }
        }

        for window in to_track {
            self.track_window(window)?;
        }

        tracing::info!("Scanned {} existing windows", self.windows.len());
        Ok(())
    }

    /// Start tracking a window.
    fn track_window(&mut self, window: Window) -> Result<()> {
        if self.windows.contains_key(&window) || window == self.overlay {
            return Ok(());
        }

        // Get window geometry
        let geom = self.conn.conn.get_geometry(window)?.reply()?;
        let attrs = self.conn.conn.get_window_attributes(window)?.reply()?;

        // Get window pixmap for compositing
        let pixmap = self.conn.name_window_pixmap(window).ok();

        // Create damage tracking for this window
        let damage = self.conn.generate_id()?;
        self.conn
            .conn
            .damage_create(damage, window, ReportLevel::NON_EMPTY)?;

        // Subscribe to property changes on the window
        self.conn.conn.change_window_attributes(
            window,
            &ChangeWindowAttributesAux::new().event_mask(EventMask::PROPERTY_CHANGE),
        )?;

        let tracked = TrackedWindow {
            id: window,
            pixmap,
            damage,
            x: geom.x,
            y: geom.y,
            width: geom.width,
            height: geom.height,
            border_width: geom.border_width,
            mapped: true,
            override_redirect: attrs.override_redirect,
            window_type: WindowType::Normal,
            opacity: 1.0,
            damaged: true,
        };

        tracing::debug!(
            "Tracking window {:#x}: {}x{}+{}+{} override_redirect={}",
            window,
            geom.width,
            geom.height,
            geom.x,
            geom.y,
            attrs.override_redirect
        );

        self.windows.insert(window, tracked);
        Ok(())
    }

    /// Stop tracking a window.
    pub fn untrack_window(&mut self, window: Window) {
        if let Some(tracked) = self.windows.remove(&window) {
            // Destroy damage object
            let _ = self.conn.conn.damage_destroy(tracked.damage);

            // Free pixmap
            if let Some(pixmap) = tracked.pixmap {
                let _ = self.conn.conn.free_pixmap(pixmap);
            }

            // Remove texture from renderer
            self.renderer.remove_window(window);

            tracing::debug!("Untracked window {:#x}", window);
        }
    }

    /// Run the compositor event loop.
    pub fn run(&mut self) -> Result<()> {
        tracing::info!("Starting compositor event loop");

        while self.running {
            let event = self.conn.conn.wait_for_event()?;
            self.handle_event(event)?;
        }

        Ok(())
    }

    /// Handle a single X11 event.
    pub fn handle_event(&mut self, event: Event) -> Result<()> {
        match event {
            Event::CreateNotify(e) => self.handle_create(e)?,
            Event::DestroyNotify(e) => self.handle_destroy(e),
            Event::MapNotify(e) => self.handle_map(e)?,
            Event::UnmapNotify(e) => self.handle_unmap(e),
            Event::ConfigureNotify(e) => self.handle_configure(e)?,
            Event::PropertyNotify(e) => self.handle_property(e)?,
            Event::Error(e) => {
                tracing::warn!("X11 error: {:?}", e);
            }
            Event::Unknown(raw) => {
                // Check for damage events by response type (first byte)
                if !raw.is_empty() {
                    let response_type = raw[0] & 0x7f; // Mask out the "sent" flag
                    if response_type == self.conn.damage_event_base {
                        if raw.len() >= 8 {
                            let drawable = u32::from_ne_bytes([raw[4], raw[5], raw[6], raw[7]]);
                            if let Some(tracked) = self.windows.get_mut(&drawable) {
                                tracked.damaged = true;
                                self.needs_redraw = true;
                                let _ = self.conn.conn.damage_subtract(tracked.damage, 0u32, 0u32);
                            }
                        }
                    }
                }
            }
            _ => {}
        }

        Ok(())
    }

    fn handle_create(&mut self, event: CreateNotifyEvent) -> Result<()> {
        tracing::trace!("CreateNotify: window {:#x}", event.window);
        Ok(())
    }

    fn handle_destroy(&mut self, event: DestroyNotifyEvent) {
        tracing::trace!("DestroyNotify: window {:#x}", event.window);
        self.untrack_window(event.window);
    }

    fn handle_map(&mut self, event: MapNotifyEvent) -> Result<()> {
        tracing::trace!("MapNotify: window {:#x}", event.window);

        if let Some(tracked) = self.windows.get_mut(&event.window) {
            tracked.mapped = true;
            tracked.damaged = true;

            // Get fresh pixmap
            if let Some(old_pixmap) = tracked.pixmap.take() {
                let _ = self.conn.conn.free_pixmap(old_pixmap);
            }
            tracked.pixmap = self.conn.name_window_pixmap(event.window).ok();
        } else {
            self.track_window(event.window)?;
        }

        self.needs_redraw = true;
        Ok(())
    }

    fn handle_unmap(&mut self, event: UnmapNotifyEvent) {
        tracing::trace!("UnmapNotify: window {:#x}", event.window);

        if let Some(tracked) = self.windows.get_mut(&event.window) {
            tracked.mapped = false;
            self.needs_redraw = true;
        }
    }

    fn handle_configure(&mut self, event: ConfigureNotifyEvent) -> Result<()> {
        // Check if this is a root window configure (screen resize)
        if event.window == self.conn.root() {
            tracing::info!("Screen resized to {}x{}", event.width, event.height);
            self.renderer.resize(event.width as u32, event.height as u32);
            self.needs_redraw = true;
            return Ok(());
        }

        if let Some(tracked) = self.windows.get_mut(&event.window) {
            let size_changed =
                tracked.width != event.width || tracked.height != event.height;

            tracked.x = event.x;
            tracked.y = event.y;
            tracked.width = event.width;
            tracked.height = event.height;
            tracked.border_width = event.border_width;
            tracked.damaged = true;

            if size_changed {
                // Need new pixmap for resized window
                if let Some(old_pixmap) = tracked.pixmap.take() {
                    let _ = self.conn.conn.free_pixmap(old_pixmap);
                }
                if tracked.mapped {
                    tracked.pixmap = self.conn.name_window_pixmap(event.window).ok();
                }
            }

            self.needs_redraw = true;
        }

        Ok(())
    }

    fn handle_property(&mut self, event: PropertyNotifyEvent) -> Result<()> {
        // Check for opacity changes
        if event.atom == self.conn.atoms._NET_WM_WINDOW_OPACITY {
            let opacity = self.get_window_opacity(event.window);
            if let Some(tracked) = self.windows.get_mut(&event.window) {
                tracked.opacity = opacity;
                tracked.damaged = true;
                self.needs_redraw = true;
            }
        }

        Ok(())
    }

    fn get_window_opacity(&self, window: Window) -> f32 {
        let cookie = match self.conn.conn.get_property(
            false,
            window,
            self.conn.atoms._NET_WM_WINDOW_OPACITY,
            AtomEnum::CARDINAL,
            0,
            1,
        ) {
            Ok(c) => c,
            Err(_) => return 1.0,
        };

        match cookie.reply() {
            Ok(reply) => {
                if let Some(value) = reply.value32().and_then(|mut v| v.next()) {
                    value as f32 / u32::MAX as f32
                } else {
                    1.0
                }
            }
            Err(_) => 1.0,
        }
    }

    /// Render a frame if needed.
    pub fn render(&mut self) -> Result<()> {
        if !self.needs_redraw {
            return Ok(());
        }

        // Collect window info for rendering
        // Sort by stacking order (TODO: implement proper stacking)
        let windows: Vec<WindowRenderInfo> = self
            .windows
            .values()
            .filter(|w| w.mapped && w.pixmap.is_some())
            .map(|w| WindowRenderInfo {
                id: w.id,
                pixmap: w.pixmap.unwrap() as u64,
                x: w.x,
                y: w.y,
                width: w.width,
                height: w.height,
                opacity: w.opacity,
            })
            .collect();

        // Render the windows
        self.renderer.render_windows(&windows)?;
        self.needs_redraw = false;

        // Clear damage flags on windows
        for window in self.windows.values_mut() {
            window.damaged = false;
        }

        Ok(())
    }

    /// Mark the compositor as needing a redraw.
    pub fn request_redraw(&mut self) {
        self.needs_redraw = true;
    }

    /// Check if a redraw is needed.
    pub fn needs_redraw(&self) -> bool {
        self.needs_redraw
    }

    /// Resize the renderer surface.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.renderer.resize(width, height);
        self.needs_redraw = true;
    }

    /// Shutdown the compositor cleanly.
    pub fn shutdown(&mut self) -> Result<()> {
        tracing::info!("Shutting down compositor");

        // Clean up tracked windows
        let windows: Vec<Window> = self.windows.keys().copied().collect();
        for window in windows {
            self.untrack_window(window);
        }

        // Release overlay
        self.conn.release_overlay_window()?;
        self.conn.flush()?;

        Ok(())
    }
}

impl Drop for Compositor {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}
