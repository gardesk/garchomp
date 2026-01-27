//! Effects configuration for the compositor.

use crate::config::LuaConfig;

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

            // Fading (disabled for testing)
            fade_enabled: false,
            fade_in_duration: 0.1,
            fade_out_duration: 0.1,
        }
    }
}

impl EffectsConfig {
    /// Load effects configuration from Lua config.
    ///
    /// Reads settings from the loaded Lua config file and applies them.
    /// Falls back to defaults for any unset values.
    pub fn from_lua_config(lua: &LuaConfig) -> Self {
        let (blur_enabled, blur_strength, _blur_iterations) = lua.get_blur_config();
        let (shadow_enabled, shadow_radius, shadow_offset_x, shadow_offset_y, shadow_opacity) =
            lua.get_shadow_config();
        let corner_radius = lua.get_corner_radius();
        let default_opacity = lua.get_default_opacity();

        // Read additional settings with defaults
        let opacity_focused: f32 = lua.get_setting("opacity_focused").unwrap_or(1.0);
        let opacity_unfocused: f32 = lua.get_setting("opacity_unfocused").unwrap_or(1.0);
        let fade_enabled: bool = lua.get_setting("fade_enabled").unwrap_or(false);
        let fade_in_duration: f32 = lua.get_setting("fade_in_duration").unwrap_or(0.1);
        let fade_out_duration: f32 = lua.get_setting("fade_out_duration").unwrap_or(0.1);
        let shadow_color_r: f32 = lua.get_setting("shadow_color_r").unwrap_or(0.0);
        let shadow_color_g: f32 = lua.get_setting("shadow_color_g").unwrap_or(0.0);
        let shadow_color_b: f32 = lua.get_setting("shadow_color_b").unwrap_or(0.0);

        Self {
            blur_enabled,
            blur_strength: blur_strength as u32,
            shadow_enabled,
            shadow_radius,
            shadow_opacity,
            shadow_offset: (shadow_offset_x, shadow_offset_y),
            shadow_color: [shadow_color_r, shadow_color_g, shadow_color_b],
            corner_radius,
            opacity_focused: opacity_focused * default_opacity,
            opacity_unfocused: opacity_unfocused * default_opacity,
            fade_enabled,
            fade_in_duration,
            fade_out_duration,
        }
    }

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
