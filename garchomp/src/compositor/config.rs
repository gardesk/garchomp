//! Effects configuration for the compositor.

/// Configuration for all compositor visual effects.
#[derive(Debug, Clone)]
pub struct EffectsConfig {
    // Blur
    /// Enable blur behind transparent windows.
    pub blur_enabled: bool,
    /// Blur strength (0-20), affects number of iterations.
    pub blur_strength: u32,

    // Shadows
    /// Enable window shadows.
    pub shadow_enabled: bool,
    /// Shadow blur radius in pixels.
    pub shadow_radius: f32,
    /// Shadow opacity (0.0-1.0).
    pub shadow_opacity: f32,
    /// Shadow offset (x, y) in pixels.
    pub shadow_offset: (f32, f32),
    /// Shadow color (RGB, 0.0-1.0).
    pub shadow_color: [f32; 3],

    // Corners
    /// Default corner radius for windows.
    pub corner_radius: f32,

    // Opacity
    /// Opacity multiplier for focused windows (0.0-1.0).
    pub opacity_focused: f32,
    /// Opacity multiplier for unfocused windows (0.0-1.0).
    pub opacity_unfocused: f32,

    // Fading
    /// Enable fade animations.
    pub fade_enabled: bool,
    /// Fade-in duration in seconds.
    pub fade_in_duration: f32,
    /// Fade-out duration in seconds.
    pub fade_out_duration: f32,
}

impl Default for EffectsConfig {
    fn default() -> Self {
        Self {
            // Blur (disabled by default - expensive)
            blur_enabled: false,
            blur_strength: 5,

            // Shadows (enabled, matching picom defaults)
            shadow_enabled: true,
            shadow_radius: 12.0,
            shadow_opacity: 0.5,
            shadow_offset: (0.0, 5.0),
            shadow_color: [0.0, 0.0, 0.0],

            // Corners
            corner_radius: 12.0,

            // Opacity (no dimming by default)
            opacity_focused: 1.0,
            opacity_unfocused: 1.0,

            // Fading (enabled with reasonable defaults)
            fade_enabled: true,
            fade_in_duration: 0.1,
            fade_out_duration: 0.1,
        }
    }
}

impl EffectsConfig {
    /// Calculate effective opacity for a window based on focus state.
    pub fn effective_opacity(&self, base_opacity: f32, focused: bool) -> f32 {
        let multiplier = if focused {
            self.opacity_focused
        } else {
            self.opacity_unfocused
        };
        base_opacity * multiplier
    }

    /// Check if shadows should be rendered.
    pub fn shadows_active(&self) -> bool {
        self.shadow_enabled && self.shadow_opacity > 0.0
    }

    /// Check if blur should be rendered.
    pub fn blur_active(&self) -> bool {
        self.blur_enabled && self.blur_strength > 0
    }
}
