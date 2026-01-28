//! X11 connection wrapper with extension support.

use super::Atoms;
use std::os::unix::io::{AsRawFd, RawFd};
use thiserror::Error;
use x11rb::connection::{Connection as X11Connection, RequestConnection};
use x11rb::protocol::composite::ConnectionExt as CompositeConnectionExt;
use x11rb::protocol::damage::ConnectionExt as DamageConnectionExt;
use x11rb::protocol::randr::ConnectionExt as RandrConnectionExt;
use x11rb::protocol::xfixes::ConnectionExt as XfixesConnectionExt;
use x11rb::protocol::xproto::{ConnectionExt as XprotoConnectionExt, Screen, Window};
use x11rb::rust_connection::RustConnection;

/// Information about a monitor/output.
#[derive(Debug, Clone)]
pub struct MonitorInfo {
    /// Monitor name (e.g., "DP-1", "HDMI-0").
    pub name: String,
    /// X position in pixels.
    pub x: i16,
    /// Y position in pixels.
    pub y: i16,
    /// Width in pixels.
    pub width: u16,
    /// Height in pixels.
    pub height: u16,
    /// Whether this is the primary monitor.
    pub primary: bool,
}

#[derive(Error, Debug)]
pub enum ConnectionError {
    #[error("failed to connect to X11 display")]
    Connect(#[from] x11rb::errors::ConnectError),

    #[error("X11 connection error: {0}")]
    Connection(#[from] x11rb::errors::ConnectionError),

    #[error("X11 reply error: {0}")]
    Reply(#[from] x11rb::errors::ReplyError),

    #[error("extension '{name}' not available (required version: {required})")]
    ExtensionMissing { name: &'static str, required: &'static str },

    #[error("failed to intern atoms")]
    Atoms(#[from] x11rb::errors::ReplyOrIdError),
}

pub type Result<T> = std::result::Result<T, ConnectionError>;

/// X11 connection with compositor extensions.
pub struct Connection {
    pub conn: RustConnection,
    pub screen_num: usize,
    pub atoms: Atoms,
    pub composite_opcode: u8,
    pub damage_opcode: u8,
    pub damage_event_base: u8,
}

impl Connection {
    /// Connect to the X11 display and initialize extensions.
    pub fn new() -> Result<Self> {
        let (conn, screen_num) = x11rb::connect(None)?;

        // Query Composite extension
        let composite = conn
            .composite_query_version(0, 4)?
            .reply()?;
        tracing::info!(
            "Composite extension: {}.{}",
            composite.major_version,
            composite.minor_version
        );
        if composite.major_version == 0 && composite.minor_version < 4 {
            return Err(ConnectionError::ExtensionMissing {
                name: "Composite",
                required: "0.4",
            });
        }

        // Query Damage extension
        let damage = conn
            .damage_query_version(1, 1)?
            .reply()?;
        tracing::info!(
            "Damage extension: {}.{}",
            damage.major_version,
            damage.minor_version
        );

        // Query XFixes extension
        let xfixes = conn
            .xfixes_query_version(5, 0)?
            .reply()?;
        tracing::info!(
            "XFixes extension: {}.{}",
            xfixes.major_version,
            xfixes.minor_version
        );

        // Get extension opcodes for event handling
        let composite_ext = conn
            .extension_information(x11rb::protocol::composite::X11_EXTENSION_NAME)?
            .ok_or(ConnectionError::ExtensionMissing {
                name: "Composite",
                required: "0.4",
            })?;

        let damage_ext = conn
            .extension_information(x11rb::protocol::damage::X11_EXTENSION_NAME)?
            .ok_or(ConnectionError::ExtensionMissing {
                name: "Damage",
                required: "1.1",
            })?;

        // Intern atoms
        let atoms = Atoms::new(&conn)?.reply()?;

        Ok(Self {
            conn,
            screen_num,
            atoms,
            composite_opcode: composite_ext.major_opcode,
            damage_opcode: damage_ext.major_opcode,
            damage_event_base: damage_ext.first_event,
        })
    }

    /// Get the default screen.
    pub fn screen(&self) -> &Screen {
        &self.conn.setup().roots[self.screen_num]
    }

    /// Get the root window.
    pub fn root(&self) -> Window {
        self.screen().root
    }

    /// Flush pending requests to the server.
    pub fn flush(&self) -> Result<()> {
        self.conn.flush()?;
        Ok(())
    }

    /// Generate a new X11 ID.
    pub fn generate_id(&self) -> Result<u32> {
        Ok(self.conn.generate_id()?)
    }

    /// Get the raw file descriptor for polling.
    pub fn as_raw_fd(&self) -> RawFd {
        self.conn.stream().as_raw_fd()
    }

    /// Get the window stacking order from the WM.
    /// Returns windows from bottom to top (render order).
    pub fn get_stacking_order(&self) -> Result<Vec<u32>> {
        use x11rb::protocol::xproto::AtomEnum;

        let reply = self.conn.get_property(
            false,
            self.root(),
            self.atoms._NET_CLIENT_LIST_STACKING,
            AtomEnum::WINDOW,
            0,
            u32::MAX,
        )?.reply()?;

        if let Some(windows) = reply.value32() {
            Ok(windows.collect())
        } else {
            // Fallback: query tree for basic stacking
            let tree = self.conn.query_tree(self.root())?.reply()?;
            Ok(tree.children)
        }
    }

    /// Get the window type atom for a window.
    /// Returns the first type atom, or None if not set.
    pub fn get_window_type(&self, window: Window) -> Option<u32> {
        use x11rb::protocol::xproto::AtomEnum;

        let reply = self.conn.get_property(
            false,
            window,
            self.atoms._NET_WM_WINDOW_TYPE,
            AtomEnum::ATOM,
            0,
            32,
        ).ok()?.reply().ok()?;

        reply.value32().and_then(|mut v| v.next())
    }

    /// Get the currently active (focused) window.
    pub fn get_active_window(&self) -> Option<Window> {
        use x11rb::protocol::xproto::AtomEnum;

        let reply = self.conn.get_property(
            false,
            self.root(),
            self.atoms._NET_ACTIVE_WINDOW,
            AtomEnum::WINDOW,
            0,
            1,
        ).ok()?.reply().ok()?;

        reply.value32().and_then(|mut v| v.next())
    }

    /// Get the root window background pixmap (set by wallpaper daemons).
    /// Checks _XROOTPMAP_ID first, then falls back to ESETROOT_PMAP_ID.
    pub fn get_root_pixmap(&self) -> Option<u32> {
        use x11rb::protocol::xproto::AtomEnum;

        // Try _XROOTPMAP_ID first (more common)
        let reply = self.conn.get_property(
            false,
            self.root(),
            self.atoms._XROOTPMAP_ID,
            AtomEnum::PIXMAP,
            0,
            1,
        ).ok()?.reply().ok()?;

        if let Some(pixmap) = reply.value32().and_then(|mut v| v.next()) {
            if pixmap != 0 {
                return Some(pixmap);
            }
        }

        // Fallback to ESETROOT_PMAP_ID
        let reply = self.conn.get_property(
            false,
            self.root(),
            self.atoms.ESETROOT_PMAP_ID,
            AtomEnum::PIXMAP,
            0,
            1,
        ).ok()?.reply().ok()?;

        reply.value32().and_then(|mut v| v.next()).filter(|&p| p != 0)
    }

    /// Query all monitors using RandR.
    /// Returns a list of active monitors with their geometry.
    pub fn get_monitors(&self) -> Result<Vec<MonitorInfo>> {
        // Query RandR version first
        let randr_version = self.conn.randr_query_version(1, 5)?.reply()?;
        tracing::debug!(
            "RandR version: {}.{}",
            randr_version.major_version,
            randr_version.minor_version
        );

        // Get screen resources
        let resources = self.conn.randr_get_screen_resources(self.root())?.reply()?;

        // Get primary output
        let primary = self.conn.randr_get_output_primary(self.root())?.reply()?;
        let primary_output = primary.output;

        let mut monitors = Vec::new();

        // Iterate through all CRTCs to find active outputs
        for crtc in &resources.crtcs {
            let crtc_info = match self.conn.randr_get_crtc_info(*crtc, 0)?.reply() {
                Ok(info) => info,
                Err(_) => continue,
            };

            // Skip disabled CRTCs
            if crtc_info.width == 0 || crtc_info.height == 0 {
                continue;
            }

            // Get output name from first connected output
            for output in &crtc_info.outputs {
                let output_info = match self.conn.randr_get_output_info(*output, 0)?.reply() {
                    Ok(info) => info,
                    Err(_) => continue,
                };

                // Convert name bytes to string
                let name = String::from_utf8_lossy(&output_info.name).to_string();

                monitors.push(MonitorInfo {
                    name,
                    x: crtc_info.x,
                    y: crtc_info.y,
                    width: crtc_info.width,
                    height: crtc_info.height,
                    primary: *output == primary_output,
                });

                // Only take first output per CRTC
                break;
            }
        }

        // Sort by position (left to right, top to bottom)
        monitors.sort_by(|a, b| {
            if a.y != b.y {
                a.y.cmp(&b.y)
            } else {
                a.x.cmp(&b.x)
            }
        });

        tracing::info!("Found {} monitors", monitors.len());
        for (i, m) in monitors.iter().enumerate() {
            tracing::info!(
                "  Monitor {}: {} {}x{}+{}+{} {}",
                i, m.name, m.width, m.height, m.x, m.y,
                if m.primary { "(primary)" } else { "" }
            );
        }

        Ok(monitors)
    }

    /// Find which monitor a point is on.
    pub fn point_to_monitor(&self, x: i32, y: i32, monitors: &[MonitorInfo]) -> Option<usize> {
        for (i, m) in monitors.iter().enumerate() {
            let mx = m.x as i32;
            let my = m.y as i32;
            let mw = m.width as i32;
            let mh = m.height as i32;

            if x >= mx && x < mx + mw && y >= my && y < my + mh {
                return Some(i);
            }
        }
        None
    }

    /// Find which monitor a window is primarily on (by center point).
    pub fn window_to_monitor(&self, x: i16, y: i16, width: u16, height: u16, monitors: &[MonitorInfo]) -> Option<usize> {
        let center_x = x as i32 + (width as i32 / 2);
        let center_y = y as i32 + (height as i32 / 2);
        self.point_to_monitor(center_x, center_y, monitors)
    }
}
