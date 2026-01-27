//! Lua configuration system for garchomp compositor.

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
}

impl WindowMatch {
    /// Check if this match applies to a window with the given properties.
    pub fn matches(&self, class: Option<&str>, instance: Option<&str>, title: Option<&str>, wtype: Option<&str>) -> bool {
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
        true
    }
}

/// Window rule configuration.
#[derive(Debug, Clone)]
pub struct WindowRule {
    pub matcher: WindowMatch,
    pub blur_behind: Option<bool>,
    pub shadow: Option<bool>,
    pub opacity: Option<f32>,
    pub corner_radius: Option<f32>,
    /// Animation override for window_open trigger.
    pub open_animation: Option<AnimationConfig>,
    /// Animation override for window_close trigger.
    pub close_animation: Option<AnimationConfig>,
}

/// Animation configuration from Lua.
#[derive(Debug, Clone)]
pub struct AnimationConfig {
    pub duration: f32,
    pub curve: String,
    // Note: callback_key is stored in RegisteredAnimation, not here
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

        let config_path = dirs_config_path();

        Ok(Self {
            lua,
            animations: HashMap::new(),
            rules: Vec::new(),
            config_path,
        })
    }

    /// Load configuration from the default config file.
    pub fn load(&mut self) -> LuaResult<()> {
        self.load_from(&self.config_path.clone())
    }

    /// Load configuration from a specific file.
    pub fn load_from(&mut self, path: &PathBuf) -> LuaResult<()> {
        if !path.exists() {
            tracing::info!("Config file not found: {:?}", path);
            return Ok(());
        }

        tracing::info!("Loading config from {:?}", path);

        // Clear existing config
        self.animations.clear();
        self.rules.clear();

        // Set up the garchomp API before loading config
        self.setup_api()?;

        // Load and execute config file
        let config_content = std::fs::read_to_string(path)
            .map_err(|e| mlua::Error::external(e))?;

        self.lua.load(&config_content).set_name(path.to_string_lossy()).exec()?;

        // Extract registered animations and rules
        self.extract_registrations()?;

        tracing::info!(
            "Loaded {} animations, {} rules",
            self.animations.len(),
            self.rules.len()
        );

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
                window_type: matcher_table.get("type").ok(),
            };

            let rule = WindowRule {
                matcher,
                blur_behind: config_table.get("blur_behind").ok(),
                shadow: config_table.get("shadow").ok(),
                opacity: config_table.get("opacity").ok(),
                corner_radius: config_table.get("corner_radius").ok(),
                open_animation: None, // TODO: parse animation overrides
                close_animation: None,
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
    pub fn find_rules(&self, class: Option<&str>, instance: Option<&str>, title: Option<&str>, wtype: Option<&str>) -> Vec<&WindowRule> {
        self.rules
            .iter()
            .filter(|r| r.matcher.matches(class, instance, title, wtype))
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

    /// Get a global setting value.
    pub fn get_setting<T: mlua::FromLua>(&self, key: &str) -> Option<T> {
        let garchomp: Table = self.lua.globals().get("garchomp").ok()?;
        garchomp.get(key).ok()
    }

    /// Get the config file path.
    pub fn config_path(&self) -> &PathBuf {
        &self.config_path
    }
}

/// Get the default config file path.
fn dirs_config_path() -> PathBuf {
    let config_dir = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            PathBuf::from(home).join(".config")
        });

    config_dir.join("gar").join("init.lua")
}
