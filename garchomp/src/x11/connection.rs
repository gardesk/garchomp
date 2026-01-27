//! X11 connection wrapper with extension support.

use super::Atoms;
use std::os::unix::io::{AsRawFd, RawFd};
use thiserror::Error;
use x11rb::connection::{Connection as X11Connection, RequestConnection};
use x11rb::protocol::composite::ConnectionExt as CompositeConnectionExt;
use x11rb::protocol::damage::ConnectionExt as DamageConnectionExt;
use x11rb::protocol::xfixes::ConnectionExt as XfixesConnectionExt;
use x11rb::protocol::xproto::{ConnectionExt as XprotoConnectionExt, Screen, Window};
use x11rb::rust_connection::RustConnection;

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
}
