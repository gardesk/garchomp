//! Window animation proxy for Lua scripting.

use mlua::{UserData, UserDataMethods};
use std::sync::{Arc, Mutex};

/// Transform state for a window being animated.
#[derive(Debug, Clone, Default)]
pub struct WindowTransform {
    /// Scale factor (x, y).
    pub scale: (f32, f32),
    /// Offset from original position (x, y).
    pub offset: (f32, f32),
    /// Opacity (0.0 to 1.0).
    pub opacity: f32,
    /// Brightness multiplier.
    pub brightness: f32,
    /// Blur radius.
    pub blur_radius: f32,
    /// Corner radius.
    pub corner_radius: f32,
    /// Saturation (0.0 to 1.0).
    pub saturation: f32,
}

impl WindowTransform {
    /// Create a new transform with default values (identity).
    pub fn new() -> Self {
        Self {
            scale: (1.0, 1.0),
            offset: (0.0, 0.0),
            opacity: 1.0,
            brightness: 1.0,
            blur_radius: 0.0,
            corner_radius: 0.0,
            saturation: 1.0,
        }
    }

    /// Reset to identity transform.
    pub fn reset(&mut self) {
        *self = Self::new();
    }
}

/// Lua proxy for window animation state.
///
/// This is passed to Lua animation callbacks to allow them to modify
/// the window's visual properties during animation.
#[derive(Clone)]
pub struct WindowAnimationProxy {
    window_id: u32,
    transform: Arc<Mutex<WindowTransform>>,
}

impl WindowAnimationProxy {
    /// Create a new window animation proxy.
    pub fn new(window_id: u32, transform: Arc<Mutex<WindowTransform>>) -> Self {
        Self { window_id, transform }
    }

    /// Get the window ID.
    pub fn window_id(&self) -> u32 {
        self.window_id
    }
}

impl UserData for WindowAnimationProxy {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        // set_scale(sx, sy) - Scale the window
        methods.add_method_mut("set_scale", |_, this, (sx, sy): (f32, f32)| {
            let mut t = this.transform.lock().unwrap();
            t.scale = (sx.max(0.0), sy.max(0.0));
            Ok(())
        });

        // set_offset(x, y) - Offset window position
        methods.add_method_mut("set_offset", |_, this, (x, y): (f32, f32)| {
            let mut t = this.transform.lock().unwrap();
            t.offset = (x, y);
            Ok(())
        });

        // set_opacity(alpha) - Set window opacity
        methods.add_method_mut("set_opacity", |_, this, alpha: f32| {
            let mut t = this.transform.lock().unwrap();
            t.opacity = alpha.clamp(0.0, 1.0);
            Ok(())
        });

        // set_brightness(mult) - Set brightness multiplier
        methods.add_method_mut("set_brightness", |_, this, mult: f32| {
            let mut t = this.transform.lock().unwrap();
            t.brightness = mult.max(0.0);
            Ok(())
        });

        // set_blur_radius(r) - Set blur radius
        methods.add_method_mut("set_blur_radius", |_, this, r: f32| {
            let mut t = this.transform.lock().unwrap();
            t.blur_radius = r.max(0.0);
            Ok(())
        });

        // set_corner_radius(r) - Set corner radius
        methods.add_method_mut("set_corner_radius", |_, this, r: f32| {
            let mut t = this.transform.lock().unwrap();
            t.corner_radius = r.max(0.0);
            Ok(())
        });

        // set_saturation(sat) - Set saturation
        methods.add_method_mut("set_saturation", |_, this, sat: f32| {
            let mut t = this.transform.lock().unwrap();
            t.saturation = sat.clamp(0.0, 1.0);
            Ok(())
        });

        // get_scale() - Get current scale
        methods.add_method("get_scale", |_, this, ()| {
            let t = this.transform.lock().unwrap();
            Ok((t.scale.0, t.scale.1))
        });

        // get_offset() - Get current offset
        methods.add_method("get_offset", |_, this, ()| {
            let t = this.transform.lock().unwrap();
            Ok((t.offset.0, t.offset.1))
        });

        // get_opacity() - Get current opacity
        methods.add_method("get_opacity", |_, this, ()| {
            let t = this.transform.lock().unwrap();
            Ok(t.opacity)
        });

        // id() - Get window ID
        methods.add_method("id", |_, this, ()| {
            Ok(this.window_id)
        });
    }
}
