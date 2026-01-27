//! Workspace tracking and transition animations.

use std::collections::HashMap;
use std::time::{Duration, Instant};
use x11rb::protocol::xproto::Window;

use super::animation::Easing;

/// Direction of workspace transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

    /// Get the offset multiplier for this direction.
    pub fn offset(&self) -> (f32, f32) {
        match self {
            Self::Left => (-1.0, 0.0),
            Self::Right => (1.0, 0.0),
            Self::Up => (0.0, -1.0),
            Self::Down => (0.0, 1.0),
        }
    }
}

/// Active workspace transition animation.
#[derive(Debug, Clone)]
pub struct WorkspaceTransition {
    pub from: usize,
    pub to: usize,
    pub direction: TransitionDirection,
    pub start_time: Instant,
    pub duration: Duration,
    pub easing: Easing,
}

impl WorkspaceTransition {
    /// Create a new workspace transition.
    pub fn new(from: usize, to: usize, direction: TransitionDirection) -> Self {
        Self {
            from,
            to,
            direction,
            start_time: Instant::now(),
            duration: Duration::from_millis(250),
            easing: Easing::EaseOut,
        }
    }

    /// Get the current progress (0.0 to 1.0).
    pub fn progress(&self) -> f32 {
        let elapsed = self.start_time.elapsed().as_secs_f32();
        let duration = self.duration.as_secs_f32();
        (elapsed / duration).clamp(0.0, 1.0)
    }

    /// Get the eased progress value.
    pub fn eased_progress(&self) -> f32 {
        self.easing.apply(self.progress())
    }

    /// Check if the transition is complete.
    pub fn is_complete(&self) -> bool {
        self.start_time.elapsed() >= self.duration
    }

    /// Calculate the offset for windows on the "from" workspace.
    /// They slide out in the direction of the transition.
    pub fn from_offset(&self, screen_width: f32, screen_height: f32) -> (f32, f32) {
        let t = self.eased_progress();
        let (dx, dy) = self.direction.offset();
        (dx * t * screen_width, dy * t * screen_height)
    }

    /// Calculate the offset for windows on the "to" workspace.
    /// They slide in from the opposite direction.
    pub fn to_offset(&self, screen_width: f32, screen_height: f32) -> (f32, f32) {
        let t = self.eased_progress();
        let (dx, dy) = self.direction.offset();
        // Start from opposite side, slide to center
        (dx * (1.0 - t) * -screen_width, dy * (1.0 - t) * -screen_height)
    }
}

/// Workspace state tracking.
#[derive(Debug)]
pub struct WorkspaceState {
    /// Current workspace index.
    pub current: usize,
    /// Windows by workspace.
    pub windows_by_workspace: HashMap<usize, Vec<Window>>,
    /// Active transition animation.
    pub transition: Option<WorkspaceTransition>,
}

impl WorkspaceState {
    /// Create new workspace state.
    pub fn new() -> Self {
        Self {
            current: 0,
            windows_by_workspace: HashMap::new(),
            transition: None,
        }
    }

    /// Set the current workspace.
    pub fn set_current(&mut self, workspace: usize) {
        self.current = workspace;
    }

    /// Start a workspace transition animation.
    pub fn start_transition(&mut self, from: usize, to: usize, direction: &str) {
        let dir = TransitionDirection::from_str(direction);
        self.transition = Some(WorkspaceTransition::new(from, to, dir));
        self.current = to;
        tracing::debug!(
            "Starting workspace transition: {} -> {} ({:?})",
            from,
            to,
            dir
        );
    }

    /// Update transition and return true if there's an active transition.
    pub fn update_transition(&mut self) -> bool {
        if let Some(ref transition) = self.transition {
            if transition.is_complete() {
                tracing::debug!("Workspace transition complete");
                self.transition = None;
                false
            } else {
                true
            }
        } else {
            false
        }
    }

    /// Assign a window to a workspace.
    pub fn assign_window(&mut self, window: Window, workspace: usize) {
        // Remove from all workspaces first
        for windows in self.windows_by_workspace.values_mut() {
            windows.retain(|&w| w != window);
        }
        // Add to new workspace
        self.windows_by_workspace
            .entry(workspace)
            .or_default()
            .push(window);
    }

    /// Remove a window from all workspaces.
    pub fn remove_window(&mut self, window: Window) {
        for windows in self.windows_by_workspace.values_mut() {
            windows.retain(|&w| w != window);
        }
    }

    /// Check if a window is on the current workspace.
    pub fn is_on_current_workspace(&self, window: Window) -> bool {
        self.windows_by_workspace
            .get(&self.current)
            .map(|windows| windows.contains(&window))
            .unwrap_or(false)
    }

    /// Check if a window is on a specific workspace.
    pub fn is_on_workspace(&self, window: Window, workspace: usize) -> bool {
        self.windows_by_workspace
            .get(&workspace)
            .map(|windows| windows.contains(&window))
            .unwrap_or(false)
    }

    /// Get the workspace for a window.
    pub fn get_window_workspace(&self, window: Window) -> Option<usize> {
        for (ws, windows) in &self.windows_by_workspace {
            if windows.contains(&window) {
                return Some(*ws);
            }
        }
        None
    }
}

impl Default for WorkspaceState {
    fn default() -> Self {
        Self::new()
    }
}
