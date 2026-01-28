//! Window tracking types.

use super::animation::{Easing, WindowAnimations};
use crate::config::WindowTransform;
use crate::config::AnimationTrigger;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
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

/// Rule overrides from Lua config.
#[derive(Debug, Clone, Default)]
pub struct WindowRuleOverrides {
    pub shadow: Option<bool>,
    pub blur_behind: Option<bool>,
    pub opacity: Option<f32>,
    pub corner_radius: Option<f32>,
}

/// Active Lua animation state.
pub struct LuaAnimation {
    /// Animation trigger type.
    pub trigger: AnimationTrigger,
    /// Start time of the animation.
    pub start_time: Instant,
    /// Duration of the animation.
    pub duration: Duration,
    /// Easing function.
    pub easing: Easing,
    /// Transform state shared with Lua callback.
    pub transform: Arc<Mutex<WindowTransform>>,
}

impl LuaAnimation {
    /// Create a new Lua animation.
    pub fn new(trigger: AnimationTrigger, duration: Duration, easing: Easing) -> Self {
        Self {
            trigger,
            start_time: Instant::now(),
            duration,
            easing,
            transform: Arc::new(Mutex::new(WindowTransform::new())),
        }
    }

    /// Get the raw progress (0.0 to 1.0).
    pub fn progress(&self) -> f32 {
        let elapsed = self.start_time.elapsed().as_secs_f32();
        let duration = self.duration.as_secs_f32();
        if duration <= 0.0 {
            1.0
        } else {
            (elapsed / duration).clamp(0.0, 1.0)
        }
    }

    /// Get the eased progress.
    pub fn eased_progress(&self) -> f32 {
        self.easing.apply(self.progress())
    }

    /// Check if the animation is complete.
    pub fn is_complete(&self) -> bool {
        self.start_time.elapsed() >= self.duration
    }

    /// Get a clone of the transform Arc for passing to Lua.
    pub fn transform_handle(&self) -> Arc<Mutex<WindowTransform>> {
        Arc::clone(&self.transform)
    }

    /// Get the current transform values.
    pub fn get_transform(&self) -> WindowTransform {
        self.transform.lock().unwrap().clone()
    }
}

impl std::fmt::Debug for LuaAnimation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LuaAnimation")
            .field("trigger", &self.trigger)
            .field("duration", &self.duration)
            .field("progress", &self.progress())
            .finish()
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
    /// Corner radius in pixels.
    pub corner_radius: f32,
    /// Whether the window has been damaged since last render.
    pub damaged: bool,
    /// Animation state for this window.
    pub animations: WindowAnimations,
    /// Rule overrides from Lua config.
    pub rule_overrides: WindowRuleOverrides,
    /// Whether this window is fullscreen.
    pub fullscreen: bool,
    /// Active Lua animation (if any).
    pub lua_animation: Option<LuaAnimation>,
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
        // Rule override takes priority
        if let Some(shadow) = self.rule_overrides.shadow {
            return shadow;
        }
        // Fullscreen windows never have shadows
        if self.fullscreen {
            return false;
        }
        match self.window_type {
            WindowType::Desktop | WindowType::Dock => false,
            WindowType::Tooltip | WindowType::Menu => false,
            _ => !self.override_redirect,
        }
    }

    /// Check if this window should have blur behind.
    pub fn should_have_blur(&self) -> bool {
        // Rule override takes priority
        if let Some(blur) = self.rule_overrides.blur_behind {
            return blur;
        }
        // Fullscreen windows never have blur
        if self.fullscreen {
            return false;
        }
        self.should_have_effects() && self.opacity < 1.0
    }

    /// Check if this window should have rounded corners.
    pub fn should_have_corners(&self) -> bool {
        // Rule override takes priority (0 radius = no corners)
        if let Some(radius) = self.rule_overrides.corner_radius {
            return radius > 0.0;
        }
        // Fullscreen windows never have rounded corners
        if self.fullscreen {
            return false;
        }
        match self.window_type {
            WindowType::Desktop | WindowType::Dock => false,
            WindowType::Tooltip | WindowType::Menu => false,
            _ => true,
        }
    }

    /// Get the effective corner radius for this window.
    pub fn effective_corner_radius(&self) -> f32 {
        // Rule override takes priority
        if let Some(radius) = self.rule_overrides.corner_radius {
            return radius;
        }
        // Fullscreen windows have no corners
        if self.fullscreen {
            return 0.0;
        }
        // Check window type
        if !self.should_have_corners() {
            return 0.0;
        }
        self.corner_radius
    }

    /// Get the effective opacity for this window.
    pub fn effective_opacity(&self) -> f32 {
        // Rule override takes priority
        if let Some(opacity) = self.rule_overrides.opacity {
            return opacity;
        }
        // Fullscreen windows are always opaque
        if self.fullscreen {
            return 1.0;
        }
        self.opacity
    }

    /// Start a Lua animation for this window.
    pub fn start_lua_animation(&mut self, trigger: AnimationTrigger, duration: Duration, easing: Easing) {
        self.lua_animation = Some(LuaAnimation::new(trigger, duration, easing));
    }

    /// Check if there's an active Lua animation.
    pub fn has_lua_animation(&self) -> bool {
        self.lua_animation.as_ref().map(|a| !a.is_complete()).unwrap_or(false)
    }

    /// Get the Lua animation transform for rendering.
    /// Returns identity transform if no animation is active.
    pub fn lua_transform(&self) -> WindowTransform {
        self.lua_animation
            .as_ref()
            .filter(|a| !a.is_complete())
            .map(|a| a.get_transform())
            .unwrap_or_else(WindowTransform::new)
    }

    /// Clear completed Lua animation.
    pub fn cleanup_lua_animation(&mut self) {
        if let Some(ref anim) = self.lua_animation {
            if anim.is_complete() {
                self.lua_animation = None;
            }
        }
    }
}
