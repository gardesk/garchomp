//! IPC protocol types for garchomp compositor.

use serde::{Deserialize, Serialize};

/// IPC protocol version for compatibility checks.
pub const PROTOCOL_VERSION: u32 = 1;

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
    /// Version negotiation.
    Version { version: u32 },
    /// Get compositor status.
    Status,
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
    /// Version response.
    Version { version: u32, name: String },
    /// Compositor status.
    Status(CompositorStatus),
    /// Window information.
    WindowInfo(WindowInfo),
    /// List of windows.
    WindowList { windows: Vec<WindowInfo> },
}

/// Compositor status information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompositorStatus {
    pub version: u32,
    pub window_count: usize,
    pub current_workspace: usize,
    pub effects_enabled: EffectsStatus,
    pub connected_to_gar: bool,
    #[serde(default)]
    pub monitors: Vec<MonitorStatus>,
}

/// Information about a monitor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitorStatus {
    pub name: String,
    pub x: i16,
    pub y: i16,
    pub width: u16,
    pub height: u16,
    pub primary: bool,
}

/// Status of compositor effects.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EffectsStatus {
    pub blur: bool,
    pub shadows: bool,
    pub animations: bool,
    pub blur_strength: u32,
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
    #[serde(default)]
    pub workspace: Option<usize>,
    #[serde(default)]
    pub focused: bool,
    #[serde(default)]
    pub fullscreen: bool,
    #[serde(default)]
    pub class: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
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
    /// Window moved.
    WindowMoved { window: u32, x: i32, y: i32 },
    /// Window resized.
    WindowResized { window: u32, width: u32, height: u32 },
    /// Window workspace changed.
    WindowWorkspace { window: u32, workspace: usize },
    /// Initial sync request (gar sending full state).
    Sync { windows: Vec<WindowInfo>, current_workspace: usize },
}

/// Direction of workspace transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransitionDirection {
    Left,
    Right,
    Up,
    Down,
}

impl TransitionDirection {
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "left" => Self::Left,
            "right" => Self::Right,
            "up" => Self::Up,
            "down" => Self::Down,
            _ => Self::Right,
        }
    }
}
