//! IPC protocol types for garchomp compositor.

use serde::{Deserialize, Serialize};

/// Request sent to garchomp.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    /// Reload configuration.
    Reload,
    /// Toggle an effect.
    SetEffect { effect: String, enabled: bool },
    /// Set blur strength (0-20).
    SetBlurStrength { strength: u32 },
    /// Get window information.
    GetWindowInfo { window: u32 },
    /// List all managed windows.
    ListWindows,
    /// Ping for health check.
    Ping,
}

/// Response from garchomp.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Response {
    /// Success with no data.
    Ok,
    /// Error response.
    Error { message: String },
    /// Pong response to ping.
    Pong,
    /// Window information.
    WindowInfo(WindowInfo),
    /// List of windows.
    WindowList { windows: Vec<WindowInfo> },
}

/// Information about a managed window.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowInfo {
    pub id: u32,
    pub x: i16,
    pub y: i16,
    pub width: u16,
    pub height: u16,
    pub mapped: bool,
    pub override_redirect: bool,
}

/// Events sent from gar WM to garchomp.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GarEvent {
    /// Workspace changed.
    WorkspaceChanged {
        from: usize,
        to: usize,
        direction: String,
    },
    /// Focus changed.
    FocusChanged { old: Option<u32>, new: Option<u32> },
    /// Window entered fullscreen.
    WindowFullscreen { window: u32, fullscreen: bool },
}
