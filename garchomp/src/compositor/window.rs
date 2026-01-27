//! Window tracking types.

use x11rb::protocol::xproto::{Pixmap, Window};

/// Type of window for compositor effects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowType {
    Normal,
    Desktop,
    Dock,
    Toolbar,
    Menu,
    Utility,
    Splash,
    Dialog,
    DropdownMenu,
    PopupMenu,
    Tooltip,
    Notification,
    Combo,
    Dnd,
}

impl Default for WindowType {
    fn default() -> Self {
        Self::Normal
    }
}

/// A window being tracked by the compositor.
#[derive(Debug)]
pub struct TrackedWindow {
    /// X11 window ID.
    pub id: Window,
    /// Pixmap containing window contents.
    pub pixmap: Option<Pixmap>,
    /// Damage object for tracking changes.
    pub damage: u32,
    /// X position.
    pub x: i16,
    /// Y position.
    pub y: i16,
    /// Width.
    pub width: u16,
    /// Height.
    pub height: u16,
    /// Border width.
    pub border_width: u16,
    /// Whether the window is mapped (visible).
    pub mapped: bool,
    /// Whether this is an override-redirect window (unmanaged).
    pub override_redirect: bool,
    /// Window type for effect decisions.
    pub window_type: WindowType,
    /// Window opacity (0.0 - 1.0).
    pub opacity: f32,
    /// Whether the window has been damaged since last render.
    pub damaged: bool,
}

impl TrackedWindow {
    /// Check if this window should have effects applied.
    pub fn should_have_effects(&self) -> bool {
        match self.window_type {
            WindowType::Desktop | WindowType::Dock => false,
            WindowType::Tooltip | WindowType::PopupMenu | WindowType::DropdownMenu => false,
            _ => true,
        }
    }

    /// Check if this window should have shadows.
    pub fn should_have_shadow(&self) -> bool {
        match self.window_type {
            WindowType::Desktop | WindowType::Dock => false,
            WindowType::Tooltip | WindowType::Menu => false,
            _ => !self.override_redirect,
        }
    }

    /// Check if this window should have blur behind.
    pub fn should_have_blur(&self) -> bool {
        self.should_have_effects() && self.opacity < 1.0
    }

    /// Check if this window should have rounded corners.
    pub fn should_have_corners(&self) -> bool {
        match self.window_type {
            WindowType::Desktop | WindowType::Dock => false,
            WindowType::Tooltip | WindowType::Menu => false,
            _ => true,
        }
    }
}
