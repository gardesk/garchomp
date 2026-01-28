//! Lua configuration system for garchomp compositor.

use crate::render::{HdrConfig, TonemapOperator};
use mlua::{Function, Lua, RegistryKey, Result as LuaResult, Table, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use super::window_proxy::WindowTransform;

/// Animation trigger events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AnimationTrigger {
    WindowOpen,
    WindowClose,
    WindowMinimize,
    WindowUnminimize,
    FocusIn,
    FocusOut,
    WorkspaceSwitch,
}

impl AnimationTrigger {
    /// Parse trigger name from string.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "window_open" => Some(Self::WindowOpen),
            "window_close" => Some(Self::WindowClose),
            "window_minimize" => Some(Self::WindowMinimize),
            "window_unminimize" => Some(Self::WindowUnminimize),
            "focus_in" | "focus_change" => Some(Self::FocusIn),
            "focus_out" => Some(Self::FocusOut),
            "workspace_switch" => Some(Self::WorkspaceSwitch),
            _ => None,
        }
    }
}

/// Window matching criteria for rules.
#[derive(Debug, Clone, Default)]
pub struct WindowMatch {
    pub class: Option<String>,
    pub instance: Option<String>,
    pub title: Option<String>,
    pub window_type: Option<String>,
    pub fullscreen: Option<bool>,
}

impl WindowMatch {
    /// Check if this match applies to a window with the given properties.
    pub fn matches(
        &self,
        class: Option<&str>,
        instance: Option<&str>,
        title: Option<&str>,
        wtype: Option<&str>,
        fullscreen: bool,
    ) -> bool {
        if let Some(ref c) = self.class {
            if class != Some(c.as_str()) {
                return false;
            }
        }
        if let Some(ref i) = self.instance {
            if instance != Some(i.as_str()) {
                return false;
            }
        }
        if let Some(ref t) = self.title {
            if let Some(win_title) = title {
                if !win_title.contains(t.as_str()) {
                    return false;
                }
            } else {
                return false;
            }
        }
        if let Some(ref wt) = self.window_type {
            if wtype != Some(wt.as_str()) {
                return false;
            }
        }
        if let Some(fs) = self.fullscreen {
            if fullscreen != fs {
                return false;
            }
        }
        true
    }
}

/// Window rule configuration.
pub struct WindowRule {
    pub matcher: WindowMatch,
    pub blur_behind: Option<bool>,
    pub shadow: Option<bool>,
    pub opacity: Option<f32>,
    pub corner_radius: Option<f32>,
    /// Animation override for window_open trigger.
    pub open_animation: Option<RuleAnimation>,
    /// Animation override for window_close trigger.
    pub close_animation: Option<RuleAnimation>,
}

/// Animation configuration from Lua (without callback - used for simple config).
#[derive(Debug, Clone)]
pub struct AnimationConfig {
    pub duration: f32,
    pub curve: String,
}

/// Animation config with callback for per-window rule overrides.
pub struct RuleAnimation {
    pub duration: f32,
    pub curve: String,
    pub callback_key: RegistryKey,
}

/// Registered animation callback.
pub struct RegisteredAnimation {
    pub duration: f32,
    pub curve: String,
    pub callback_key: RegistryKey,
}

/// Lua configuration state.
pub struct LuaConfig {
    lua: Lua,
    /// Animation callbacks by trigger.
    animations: HashMap<AnimationTrigger, RegisteredAnimation>,
    /// Window rules.
    rules: Vec<WindowRule>,
    /// Config file path.
    config_path: PathBuf,
}

impl LuaConfig {
    /// Create a new Lua configuration.
    pub fn new() -> LuaResult<Self> {
        let lua = Lua::new();

        // Create garchomp global table
        let garchomp = lua.create_table()?;
        lua.globals().set("garchomp", garchomp)?;

        // Determine which config path we'll use (set properly during load())
        let config_path = if garchomp_config_path().exists() {
            garchomp_config_path()
        } else {
            gar_config_path()
        };

        Ok(Self {
            lua,
            animations: HashMap::new(),
            rules: Vec::new(),
            config_path,
        })
    }

    /// Load configuration with fallback chain:
    /// 1. Try ~/.config/garchomp/init.lua (dedicated garchomp config)
    /// 2. Try garchomp = { ... } table from ~/.config/gar/init.lua
    pub fn load(&mut self) -> LuaResult<()> {
        // Clear existing config
        self.animations.clear();
        self.rules.clear();

        // Set up the garchomp API
        self.setup_api()?;

        // Try dedicated garchomp config first
        let garchomp_config = garchomp_config_path();
        if garchomp_config.exists() {
            tracing::info!("Loading dedicated config from {:?}", garchomp_config);
            self.config_path = garchomp_config.clone();
            return self.load_dedicated_config(&garchomp_config);
        }

        // Fall back to garchomp section in gar's init.lua
        let gar_config = gar_config_path();
        if gar_config.exists() {
            tracing::info!("Loading garchomp section from {:?}", gar_config);
            self.config_path = gar_config.clone();
            return self.load_from_gar_config(&gar_config);
        }

        tracing::info!("No config file found, using defaults");
        Ok(())
    }

    /// Load configuration from a specific file (dedicated garchomp config).
    pub fn load_from(&mut self, path: &PathBuf) -> LuaResult<()> {
        self.animations.clear();
        self.rules.clear();
        self.setup_api()?;
        self.load_dedicated_config(path)
    }

    /// Load a dedicated garchomp config file.
    fn load_dedicated_config(&mut self, path: &PathBuf) -> LuaResult<()> {
        if !path.exists() {
            tracing::info!("Config file not found: {:?}", path);
            return Ok(());
        }

        let config_content = std::fs::read_to_string(path)
            .map_err(|e| mlua::Error::external(e))?;

        self.lua.load(&config_content).set_name(path.to_string_lossy()).exec()?;

        // Extract registered animations and rules
        self.extract_registrations()?;

        tracing::info!(
            "Loaded {} animations, {} rules from dedicated config",
            self.animations.len(),
            self.rules.len()
        );

        Ok(())
    }

    /// Load garchomp settings from gar's init.lua.
    /// Looks for a `garchomp = { ... }` table in the file.
    fn load_from_gar_config(&mut self, path: &PathBuf) -> LuaResult<()> {
        if !path.exists() {
            return Ok(());
        }

        // Create a sandboxed environment that provides the gar global
        // so gar's config doesn't error, but we only read the garchomp table.
        // Use a metatable to make any access to gar return a no-op function.
        let gar_stub = self.lua.create_table()?;

        // Create a metatable that returns a no-op function for any missing key
        let mt = self.lua.create_table()?;
        let noop = self.lua.create_function(|_, _: mlua::Variadic<Value>| Ok(()))?;
        let noop_clone = noop.clone();
        mt.set("__index", self.lua.create_function(move |_, (_t, _k): (Value, Value)| {
            Ok(noop_clone.clone())
        })?)?;
        gar_stub.set_metatable(Some(mt));

        // Allow gar.bar to be a table (for gar.bar = { ... })
        let bar_table = self.lua.create_table()?;
        gar_stub.set("bar", bar_table)?;

        self.lua.globals().set("gar", gar_stub)?;

        let config_content = std::fs::read_to_string(path)
            .map_err(|e| mlua::Error::external(e))?;

        // Execute the config - this may set garchomp = { ... }
        if let Err(e) = self.lua.load(&config_content).set_name(path.to_string_lossy()).exec() {
            tracing::warn!("Error loading gar config (continuing): {}", e);
            // Don't fail - we might still have gotten some settings
        }

        // Check if a garchomp table was defined
        let globals = self.lua.globals();
        if let Ok(garchomp_table) = globals.get::<Table>("garchomp") {
            // Copy settings from the user's garchomp table to our internal one
            self.import_settings_from_table(&garchomp_table)?;
        }

        // Extract any animations/rules that were registered
        self.extract_registrations()?;

        tracing::info!(
            "Loaded {} animations, {} rules from gar config",
            self.animations.len(),
            self.rules.len()
        );

        Ok(())
    }

    /// Import settings from a user-defined garchomp table.
    fn import_settings_from_table(&mut self, source: &Table) -> LuaResult<()> {
        let garchomp: Table = self.lua.globals().get("garchomp")?;

        // List of settings to import
        let settings = [
            "blur_enabled", "blur_strength", "blur_iterations",
            "shadow_enabled", "shadow_radius", "shadow_offset_x", "shadow_offset_y", "shadow_opacity",
            "shadow_color_r", "shadow_color_g", "shadow_color_b",
            "corner_radius", "opacity", "opacity_focused", "opacity_unfocused",
            "fade_enabled", "fade_in_duration", "fade_out_duration",
            "hdr_enabled", "hdr_peak_luminance", "hdr_paper_white", "hdr_tonemap",
        ];

        for key in settings {
            if let Ok(value) = source.get::<Value>(key) {
                if value != Value::Nil {
                    garchomp.set(key, value)?;
                }
            }
        }

        Ok(())
    }

    /// Set up the garchomp Lua API.
    fn setup_api(&self) -> LuaResult<()> {
        let garchomp: Table = self.lua.globals().get("garchomp")?;

        // Create temporary storage tables for registrations
        let animations_table = self.lua.create_table()?;
        let rules_table = self.lua.create_table()?;

        garchomp.set("_animations", animations_table)?;
        garchomp.set("_rules", rules_table)?;

        // garchomp.animate(trigger, config)
        let animate_fn = self.lua.create_function(|lua, (trigger, config): (String, Table)| {
            let garchomp: Table = lua.globals().get("garchomp")?;
            let animations: Table = garchomp.get("_animations")?;

            // Store the animation config
            animations.set(trigger, config)?;

            Ok(())
        })?;
        garchomp.set("animate", animate_fn)?;

        // garchomp.rule(match, config)
        let rule_fn = self.lua.create_function(|lua, (matcher, config): (Table, Table)| {
            let garchomp: Table = lua.globals().get("garchomp")?;
            let rules: Table = garchomp.get("_rules")?;

            // Create a rule entry combining matcher and config
            let rule = lua.create_table()?;
            rule.set("match", matcher)?;
            rule.set("config", config)?;

            // Append to rules array
            let len = rules.len()? + 1;
            rules.set(len, rule)?;

            Ok(())
        })?;
        garchomp.set("rule", rule_fn)?;

        // garchomp.set(key, value) - for global settings
        let set_fn = self.lua.create_function(|lua, (key, value): (String, Value)| {
            let garchomp: Table = lua.globals().get("garchomp")?;
            garchomp.set(key, value)?;
            Ok(())
        })?;
        garchomp.set("set", set_fn)?;

        Ok(())
    }

    /// Extract registered animations and rules from Lua state.
    fn extract_registrations(&mut self) -> LuaResult<()> {
        let garchomp: Table = self.lua.globals().get("garchomp")?;

        // Extract animations
        let animations: Table = garchomp.get("_animations")?;
        for pair in animations.pairs::<String, Table>() {
            let (trigger_name, config) = pair?;

            if let Some(trigger) = AnimationTrigger::from_name(&trigger_name) {
                let duration: f32 = config.get("duration").unwrap_or(0.2);
                let curve: String = config.get("curve").unwrap_or_else(|_| "ease-out".to_string());

                // Store the animate callback in registry
                if let Ok(callback) = config.get::<Function>("animate") {
                    let key = self.lua.create_registry_value(callback)?;
                    self.animations.insert(trigger, RegisteredAnimation {
                        duration,
                        curve,
                        callback_key: key,
                    });
                }
            }
        }

        // Extract rules
        let rules: Table = garchomp.get("_rules")?;
        for pair in rules.pairs::<i64, Table>() {
            let (_, rule_table) = pair?;

            let matcher_table: Table = rule_table.get("match")?;
            let config_table: Table = rule_table.get("config")?;

            let matcher = WindowMatch {
                class: matcher_table.get("class").ok(),
                instance: matcher_table.get("instance").ok(),
                title: matcher_table.get("title").ok(),
                // Support both "type" and "window_type" for consistency
                window_type: matcher_table.get("window_type").ok()
                    .or_else(|| matcher_table.get("type").ok()),
                fullscreen: matcher_table.get("fullscreen").ok(),
            };

            // Parse animation overrides
            let open_animation = self.parse_rule_animation(&config_table, "open_animation")?;
            let close_animation = self.parse_rule_animation(&config_table, "close_animation")?;

            let rule = WindowRule {
                matcher,
                blur_behind: config_table.get("blur_behind").ok(),
                shadow: config_table.get("shadow").ok(),
                opacity: config_table.get("opacity").ok(),
                corner_radius: config_table.get("corner_radius").ok(),
                open_animation,
                close_animation,
            };

            self.rules.push(rule);
        }

        Ok(())
    }

    /// Get animation config for a trigger.
    pub fn get_animation(&self, trigger: AnimationTrigger) -> Option<&RegisteredAnimation> {
        self.animations.get(&trigger)
    }

    /// Find matching rules for a window.
    pub fn find_rules(
        &self,
        class: Option<&str>,
        instance: Option<&str>,
        title: Option<&str>,
        wtype: Option<&str>,
        fullscreen: bool,
    ) -> Vec<&WindowRule> {
        self.rules
            .iter()
            .filter(|r| r.matcher.matches(class, instance, title, wtype, fullscreen))
            .collect()
    }

    /// Call an animation callback.
    pub fn call_animation(
        &self,
        trigger: AnimationTrigger,
        t: f32,
        transform: &Arc<Mutex<WindowTransform>>,
        window_id: u32,
    ) -> LuaResult<()> {
        if let Some(anim) = self.animations.get(&trigger) {
            let callback: Function = self.lua.registry_value(&anim.callback_key)?;

            // Create window proxy for Lua
            let proxy = super::WindowAnimationProxy::new(window_id, Arc::clone(transform));

            callback.call::<()>((t, proxy))?;
        }

        Ok(())
    }

    /// Parse a rule animation from a config table.
    fn parse_rule_animation(&self, config_table: &Table, key: &str) -> LuaResult<Option<RuleAnimation>> {
        let anim_table: Option<Table> = config_table.get(key).ok();

        if let Some(anim) = anim_table {
            let duration: f32 = anim.get("duration").unwrap_or(0.2);
            let curve: String = anim.get("curve").unwrap_or_else(|_| "ease-out".to_string());

            // Get the animate callback
            let callback: Option<Function> = anim.get("animate").ok();

            if let Some(cb) = callback {
                // Store callback in registry
                let callback_key = self.lua.create_registry_value(cb)?;

                return Ok(Some(RuleAnimation {
                    duration,
                    curve,
                    callback_key,
                }));
            }
        }

        Ok(None)
    }

    /// Call a rule animation callback.
    pub fn call_rule_animation(
        &self,
        animation: &RuleAnimation,
        t: f32,
        transform: &Arc<Mutex<WindowTransform>>,
        window_id: u32,
    ) -> LuaResult<()> {
        let callback: Function = self.lua.registry_value(&animation.callback_key)?;
        let proxy = super::WindowAnimationProxy::new(window_id, Arc::clone(transform));
        callback.call::<()>((t, proxy))?;
        Ok(())
    }

    /// Get a global setting value.
    pub fn get_setting<T: mlua::FromLua>(&self, key: &str) -> Option<T> {
        let garchomp: Table = self.lua.globals().get("garchomp").ok()?;
        garchomp.get(key).ok()
    }

    /// Get the config file path.
    pub fn config_path(&self) -> &PathBuf {
        &self.config_path
    }

    /// Get HDR configuration from Lua settings.
    ///
    /// Reads the following settings:
    /// - hdr_enabled: bool (default: false)
    /// - hdr_peak_luminance: f32 (default: 1000.0 nits)
    /// - hdr_paper_white: f32 (default: 203.0 nits)
    /// - hdr_tonemap: string ("aces", "reinhard", "hable", "none")
    pub fn get_hdr_config(&self) -> HdrConfig {
        let enabled = self.get_setting::<bool>("hdr_enabled").unwrap_or(false);
        let peak_luminance = self.get_setting::<f32>("hdr_peak_luminance").unwrap_or(1000.0);
        let paper_white = self.get_setting::<f32>("hdr_paper_white").unwrap_or(203.0);
        let tonemap_name = self.get_setting::<String>("hdr_tonemap").unwrap_or_else(|| "aces".to_string());

        HdrConfig {
            enabled,
            peak_luminance,
            paper_white,
            tonemap_operator: TonemapOperator::from_name(&tonemap_name),
            display_hdr_capable: false, // Set by compositor based on visual detection
        }
    }

    /// Get blur configuration from Lua settings.
    ///
    /// Reads the following settings:
    /// - blur_enabled: bool (default: true)
    /// - blur_strength: f32 (default: 5.0, range 0-20)
    /// - blur_iterations: u32 (default: 4)
    pub fn get_blur_config(&self) -> (bool, f32, u32) {
        let enabled = self.get_setting::<bool>("blur_enabled").unwrap_or(true);
        let strength = self.get_setting::<f32>("blur_strength").unwrap_or(5.0);
        let iterations = self.get_setting::<u32>("blur_iterations").unwrap_or(4);
        (enabled, strength, iterations)
    }

    /// Get shadow configuration from Lua settings.
    ///
    /// Reads the following settings:
    /// - shadow_enabled: bool (default: true)
    /// - shadow_radius: f32 (default: 12.0)
    /// - shadow_offset_x: f32 (default: 5.0)
    /// - shadow_offset_y: f32 (default: 5.0)
    /// - shadow_opacity: f32 (default: 0.6)
    pub fn get_shadow_config(&self) -> (bool, f32, f32, f32, f32) {
        let enabled = self.get_setting::<bool>("shadow_enabled").unwrap_or(true);
        let radius = self.get_setting::<f32>("shadow_radius").unwrap_or(12.0);
        let offset_x = self.get_setting::<f32>("shadow_offset_x").unwrap_or(5.0);
        let offset_y = self.get_setting::<f32>("shadow_offset_y").unwrap_or(5.0);
        let opacity = self.get_setting::<f32>("shadow_opacity").unwrap_or(0.6);
        (enabled, radius, offset_x, offset_y, opacity)
    }

    /// Get corner radius setting.
    pub fn get_corner_radius(&self) -> f32 {
        self.get_setting::<f32>("corner_radius").unwrap_or(12.0)
    }

    /// Get default window opacity.
    pub fn get_default_opacity(&self) -> f32 {
        self.get_setting::<f32>("opacity").unwrap_or(1.0)
    }
}

/// Get the XDG config directory.
fn xdg_config_dir() -> PathBuf {
    std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            PathBuf::from(home).join(".config")
        })
}

/// Get the dedicated garchomp config path (~/.config/garchomp/init.lua).
fn garchomp_config_path() -> PathBuf {
    xdg_config_dir().join("garchomp").join("init.lua")
}

/// Get the gar config path (~/.config/gar/init.lua).
fn gar_config_path() -> PathBuf {
    xdg_config_dir().join("gar").join("init.lua")
}
