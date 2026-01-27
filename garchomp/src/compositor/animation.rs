//! Animation state and helpers for window effects.

use std::time::{Duration, Instant};

/// Easing function type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Easing {
    Linear,
    EaseIn,
    EaseOut,
    EaseInOut,
}

impl Easing {
    /// Apply the easing function to a normalized time value (0.0 to 1.0).
    pub fn apply(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Easing::Linear => t,
            Easing::EaseIn => t * t,
            Easing::EaseOut => 1.0 - (1.0 - t) * (1.0 - t),
            Easing::EaseInOut => {
                if t < 0.5 {
                    2.0 * t * t
                } else {
                    1.0 - (-2.0 * t + 2.0).powi(2) / 2.0
                }
            }
        }
    }
}

/// Animation state for a single property.
#[derive(Debug, Clone)]
pub struct Animation {
    /// Starting value.
    pub from: f32,
    /// Target value.
    pub to: f32,
    /// When the animation started.
    pub start_time: Instant,
    /// Animation duration.
    pub duration: Duration,
    /// Easing function.
    pub easing: Easing,
}

impl Animation {
    /// Create a new animation.
    pub fn new(from: f32, to: f32, duration: Duration, easing: Easing) -> Self {
        Self {
            from,
            to,
            start_time: Instant::now(),
            duration,
            easing,
        }
    }

    /// Create a fade-in animation (0 to 1).
    pub fn fade_in(duration: Duration) -> Self {
        Self::new(0.0, 1.0, duration, Easing::EaseOut)
    }

    /// Create a fade-out animation (1 to 0).
    pub fn fade_out(duration: Duration) -> Self {
        Self::new(1.0, 0.0, duration, Easing::EaseIn)
    }

    /// Get the current animated value.
    pub fn value(&self) -> f32 {
        let elapsed = self.start_time.elapsed().as_secs_f32();
        let duration = self.duration.as_secs_f32();

        if duration <= 0.0 {
            return self.to;
        }

        let t = (elapsed / duration).clamp(0.0, 1.0);
        let eased_t = self.easing.apply(t);

        self.from + (self.to - self.from) * eased_t
    }

    /// Check if the animation has completed.
    pub fn is_complete(&self) -> bool {
        self.start_time.elapsed() >= self.duration
    }

    /// Get the target value.
    pub fn target(&self) -> f32 {
        self.to
    }
}

/// Window animation state tracking multiple animated properties.
#[derive(Debug, Default)]
pub struct WindowAnimations {
    /// Opacity/fade animation.
    pub opacity: Option<Animation>,
    /// Scale animation (for zoom effects).
    pub scale: Option<Animation>,
    /// X offset animation (for slide effects).
    pub offset_x: Option<Animation>,
    /// Y offset animation (for slide effects).
    pub offset_y: Option<Animation>,
}

impl WindowAnimations {
    /// Create empty animation state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Start a fade-in animation.
    pub fn start_fade_in(&mut self, duration: Duration) {
        self.opacity = Some(Animation::fade_in(duration));
    }

    /// Start a fade-out animation.
    pub fn start_fade_out(&mut self, duration: Duration) {
        self.opacity = Some(Animation::fade_out(duration));
    }

    /// Get current opacity multiplier (1.0 if no animation).
    pub fn opacity_multiplier(&self) -> f32 {
        self.opacity.as_ref().map(|a| a.value()).unwrap_or(1.0)
    }

    /// Check if any animations are active.
    pub fn has_active_animations(&self) -> bool {
        self.opacity.as_ref().map(|a| !a.is_complete()).unwrap_or(false)
            || self.scale.as_ref().map(|a| !a.is_complete()).unwrap_or(false)
            || self.offset_x.as_ref().map(|a| !a.is_complete()).unwrap_or(false)
            || self.offset_y.as_ref().map(|a| !a.is_complete()).unwrap_or(false)
    }

    /// Clean up completed animations.
    pub fn cleanup_completed(&mut self) {
        if self.opacity.as_ref().map(|a| a.is_complete()).unwrap_or(false) {
            self.opacity = None;
        }
        if self.scale.as_ref().map(|a| a.is_complete()).unwrap_or(false) {
            self.scale = None;
        }
        if self.offset_x.as_ref().map(|a| a.is_complete()).unwrap_or(false) {
            self.offset_x = None;
        }
        if self.offset_y.as_ref().map(|a| a.is_complete()).unwrap_or(false) {
            self.offset_y = None;
        }
    }

    /// Check if a fade-out animation has completed (window ready to untrack).
    pub fn fade_out_complete(&self) -> bool {
        self.opacity
            .as_ref()
            .map(|a| a.is_complete() && a.target() == 0.0)
            .unwrap_or(false)
    }
}
