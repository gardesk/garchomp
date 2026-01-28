//! Core compositor state and event handling.

mod animation;
mod config;
mod window;
mod workspace;

pub use animation::{Animation, Easing, WindowAnimations};
use crate::config::AnimationTrigger;
pub use config::EffectsConfig;
pub use window::{LuaAnimation, TrackedWindow, WindowRuleOverrides, WindowType};
pub use workspace::{TransitionDirection, WorkspaceState, WorkspaceTransition};

use crate::config::LuaConfig;
use crate::ipc::GarConnection;
use crate::render::{GpuError, HdrConfig, Renderer, WindowRenderInfo};
use crate::x11::{CompositeExt, Connection};
use garchomp_ipc::GarEvent;
use std::collections::HashMap;
use thiserror::Error;
use x11rb::connection::Connection as _;
use x11rb::protocol::damage::{ConnectionExt as DamageConnectionExt, NotifyEvent as DamageNotifyEvent, ReportLevel};
use x11rb::protocol::xproto::{
    AtomEnum, ChangeWindowAttributesAux, ConfigureNotifyEvent, ConnectionExt, CreateNotifyEvent,
    DestroyNotifyEvent, EventMask, MapNotifyEvent, PropertyNotifyEvent,
    UnmapNotifyEvent, Window,
};
use x11rb::protocol::Event;

#[derive(Error, Debug)]
pub enum CompositorError {
    #[error("X11 connection error: {0}")]
    Connection(#[from] crate::x11::ConnectionError),

    #[error("X11 error: {0}")]
    X11(#[from] x11rb::errors::ConnectionError),

    #[error("X11 reply error: {0}")]
    Reply(#[from] x11rb::errors::ReplyError),

    #[error("GPU error: {0}")]
    Gpu(#[from] GpuError),
}

pub type Result<T> = std::result::Result<T, CompositorError>;

/// The main compositor state.
pub struct Compositor {
    pub conn: Connection,
    pub overlay: Window,
    pub renderer: Renderer,
    pub windows: HashMap<Window, TrackedWindow>,
    pub running: bool,
    needs_redraw: bool,
    /// Currently focused window (from _NET_ACTIVE_WINDOW).
    active_window: Option<Window>,
    /// Effects configuration.
    pub effects: EffectsConfig,
    /// Workspace state tracking.
    pub workspaces: WorkspaceState,
    /// Connection to gar window manager.
    pub gar: GarConnection,
    /// Lua configuration (for reloading and animation callbacks).
    lua_config: Option<LuaConfig>,
    /// Root window background pixmap (wallpaper).
    root_pixmap: Option<u32>,
}

impl Compositor {
    /// Create a new compositor instance.
    pub async fn new() -> Result<Self> {
        let conn = Connection::new()?;

        // Redirect all windows for compositing
        conn.redirect_subwindows()?;

        // Get the overlay window
        let overlay = conn.get_overlay_window()?;
        conn.configure_overlay(overlay)?;

        // Get screen dimensions for GPU surface
        let screen = conn.screen();
        let width = screen.width_in_pixels as u32;
        let height = screen.height_in_pixels as u32;

        // Initialize GPU renderer
        tracing::info!("Initializing GPU renderer for {}x{} surface", width, height);
        let renderer = Renderer::new(overlay, width, height).await?;
        tracing::info!("GPU renderer initialized");

        // Subscribe to events on root window
        let event_mask = EventMask::SUBSTRUCTURE_NOTIFY
            | EventMask::STRUCTURE_NOTIFY
            | EventMask::PROPERTY_CHANGE;

        conn.conn.change_window_attributes(
            conn.root(),
            &ChangeWindowAttributesAux::new().event_mask(event_mask),
        )?;

        conn.flush()?;

        // Try to connect to gar
        let mut gar = GarConnection::new();
        gar.connect();

        // Load Lua configuration
        let (lua_config, effects) = match LuaConfig::new() {
            Ok(mut lua) => {
                if let Err(e) = lua.load() {
                    tracing::warn!("Failed to load Lua config: {}", e);
                    (Some(lua), EffectsConfig::default())
                } else {
                    let effects = EffectsConfig::from_lua_config(&lua);
                    tracing::info!(
                        "Loaded config: blur={}, shadows={}, corners={:.1}px, fade={}",
                        effects.blur_enabled,
                        effects.shadow_enabled,
                        effects.corner_radius,
                        effects.fade_enabled
                    );
                    (Some(lua), effects)
                }
            }
            Err(e) => {
                tracing::warn!("Failed to create Lua config: {}", e);
                (None, EffectsConfig::default())
            }
        };

        // Get initial root pixmap (wallpaper)
        let root_pixmap = conn.get_root_pixmap();
        if let Some(pix) = root_pixmap {
            tracing::info!("Found root pixmap: {:#x}", pix);
        }

        let mut compositor = Self {
            conn,
            overlay,
            renderer,
            windows: HashMap::new(),
            running: true,
            needs_redraw: true,
            active_window: None,
            effects,
            workspaces: WorkspaceState::new(),
            gar,
            lua_config,
            root_pixmap,
        };

        // Get initial active window
        compositor.active_window = compositor.conn.get_active_window();

        // Apply HDR config if enabled
        if let Some(ref lua) = compositor.lua_config {
            let hdr_config = lua.get_hdr_config();
            if hdr_config.enabled {
                tracing::info!("Enabling HDR from config");
                compositor.renderer.enable_hdr(hdr_config);
            }
        }

        // Set initial root pixmap in renderer
        compositor.renderer.set_root_pixmap(compositor.root_pixmap);

        // Scan existing windows
        compositor.scan_windows()?;

        // Sync current workspace from X11
        let current_desktop = compositor.get_current_desktop();
        compositor.workspaces.set_current(current_desktop);
        tracing::info!("Current workspace: {}", current_desktop);

        Ok(compositor)
    }

    /// Scan for existing windows on startup.
    fn scan_windows(&mut self) -> Result<()> {
        let tree = self.conn.conn.query_tree(self.conn.root())?.reply()?;

        // Collect windows to track (avoid borrow issues)
        let mut to_track = Vec::new();
        for window in tree.children {
            if window == self.overlay {
                continue;
            }

            if let Ok(attrs) = self.conn.conn.get_window_attributes(window) {
                if let Ok(attrs) = attrs.reply() {
                    if attrs.map_state != x11rb::protocol::xproto::MapState::UNMAPPED {
                        to_track.push(window);
                    }
                }
            }
        }

        for window in to_track {
            self.track_window(window)?;
        }

        tracing::info!("Scanned {} existing windows", self.windows.len());
        Ok(())
    }

    /// Start tracking a window.
    fn track_window(&mut self, window: Window) -> Result<()> {
        if self.windows.contains_key(&window) || window == self.overlay {
            return Ok(());
        }

        // Get window geometry
        let geom = self.conn.conn.get_geometry(window)?.reply()?;
        let attrs = self.conn.conn.get_window_attributes(window)?.reply()?;

        // Get window pixmap for compositing
        let pixmap = self.conn.name_window_pixmap(window).ok();

        // Create damage tracking for this window
        let damage = self.conn.generate_id()?;
        self.conn
            .conn
            .damage_create(damage, window, ReportLevel::NON_EMPTY)?;

        // Subscribe to property changes on the window
        self.conn.conn.change_window_attributes(
            window,
            &ChangeWindowAttributesAux::new().event_mask(EventMask::PROPERTY_CHANGE),
        )?;

        // Detect window type from _NET_WM_WINDOW_TYPE
        let window_type = self.get_window_type_enum(window);

        // Get initial opacity
        let opacity = self.get_window_opacity(window);

        // Check for fullscreen state
        let fullscreen = self.is_window_fullscreen(window);

        // Check for compositor bypass request (_NET_WM_BYPASS_COMPOSITOR)
        let bypass_compositor = self.conn.wants_bypass(window).unwrap_or(false);
        if bypass_compositor || fullscreen {
            tracing::debug!("Unredirecting window {:#x} (fullscreen={}, bypass={})",
                window, fullscreen, bypass_compositor);
            if let Err(e) = self.conn.unredirect_window(window) {
                tracing::warn!("Failed to unredirect window: {}", e);
            }
        }

        // Get window class and title for rule matching
        let (wm_class, wm_instance) = self.get_window_class(window);
        let wm_name = self.get_window_name(window);
        let window_type_str = window_type_to_string(window_type);

        // Find matching rules and build overrides
        let rule_overrides = if let Some(ref lua_config) = self.lua_config {
            let rules = lua_config.find_rules(
                wm_class.as_deref(),
                wm_instance.as_deref(),
                wm_name.as_deref(),
                Some(&window_type_str),
                fullscreen,
            );
            build_rule_overrides(&rules)
        } else {
            WindowRuleOverrides::default()
        };

        // Create animation state, starting fade-in if enabled
        let mut animations = WindowAnimations::new();
        if self.effects.fade_enabled {
            animations.start_fade_in(std::time::Duration::from_secs_f32(
                self.effects.fade_in_duration,
            ));
        }

        let tracked = TrackedWindow {
            id: window,
            pixmap,
            damage,
            x: geom.x,
            y: geom.y,
            width: geom.width,
            height: geom.height,
            border_width: geom.border_width,
            mapped: true,
            override_redirect: attrs.override_redirect,
            window_type,
            opacity,
            corner_radius: self.effects.corner_radius,
            damaged: true,
            animations,
            rule_overrides,
            fullscreen,
            lua_animation: None,
            wm_class,
            wm_instance,
            wm_name,
        };

        tracing::debug!(
            "Tracking window {:#x}: {}x{}+{}+{} override_redirect={}",
            window,
            geom.width,
            geom.height,
            geom.x,
            geom.y,
            attrs.override_redirect
        );

        self.windows.insert(window, tracked);

        // Assign window to its workspace from _NET_WM_DESKTOP
        // Only for non-override-redirect windows (managed windows)
        if !attrs.override_redirect {
            if let Some(desktop) = self.get_window_desktop(window) {
                self.workspaces.assign_window(window, desktop);
                tracing::debug!("Window {:#x} assigned to workspace {}", window, desktop);
            }
        }

        // Start window_open animation if configured
        self.start_lua_animation(window, crate::config::AnimationTrigger::WindowOpen);

        Ok(())
    }

    /// Start a Lua animation for a window if one is configured.
    fn start_lua_animation(&mut self, window: Window, trigger: AnimationTrigger) {
        if self.lua_config.is_none() {
            return;
        }

        // Get window properties for rule matching
        let (wm_class, wm_instance) = self.get_window_class(window);
        let (window_type, fullscreen) = if let Some(tracked) = self.windows.get(&window) {
            (Some(window_type_to_string(tracked.window_type)), tracked.fullscreen)
        } else {
            (None, false)
        };

        let lua_config = self.lua_config.as_ref().unwrap();

        // Check for rule-specific animation override first
        let rules = lua_config.find_rules(
            wm_class.as_deref(),
            wm_instance.as_deref(),
            None,
            window_type.as_deref(),
            fullscreen,
        );

        // Look for animation override in matched rules
        let rule_animation = rules.iter().find_map(|r| {
            match trigger {
                AnimationTrigger::WindowOpen => r.open_animation.as_ref(),
                AnimationTrigger::WindowClose => r.close_animation.as_ref(),
                _ => None,
            }
        });

        // Use rule animation if found, otherwise fall back to global
        if let Some(rule_anim) = rule_animation {
            let duration = std::time::Duration::from_secs_f32(rule_anim.duration);
            let easing = Easing::from_name(&rule_anim.curve);

            if let Some(tracked) = self.windows.get_mut(&window) {
                tracked.start_lua_animation(trigger, duration, easing);
                tracing::debug!(
                    "Started {:?} rule animation for window {:#x} ({}s)",
                    trigger, window, rule_anim.duration
                );
            }
        } else if let Some(anim) = lua_config.get_animation(trigger) {
            let duration = std::time::Duration::from_secs_f32(anim.duration);
            let easing = Easing::from_name(&anim.curve);

            if let Some(tracked) = self.windows.get_mut(&window) {
                tracked.start_lua_animation(trigger, duration, easing);
                tracing::debug!(
                    "Started {:?} global animation for window {:#x} ({}s)",
                    trigger, window, anim.duration
                );
            }
        }
    }

    /// Update all active Lua animations by calling their callbacks.
    fn update_lua_animations(&mut self) {
        // Collect window IDs with active Lua animations
        let lua_animated: Vec<(Window, AnimationTrigger, f32, std::sync::Arc<std::sync::Mutex<crate::config::WindowTransform>>)> = self
            .windows
            .iter()
            .filter_map(|(id, w)| {
                w.lua_animation.as_ref().filter(|a| !a.is_complete()).map(|a| {
                    (*id, a.trigger, a.eased_progress(), a.transform_handle())
                })
            })
            .collect();

        // Call Lua callbacks for each animation
        if let Some(ref lua_config) = self.lua_config {
            for (window_id, trigger, t, transform) in lua_animated {
                if let Err(e) = lua_config.call_animation(trigger, t, &transform, window_id) {
                    tracing::warn!("Lua animation callback error: {}", e);
                }
            }
        }
    }

    /// Stop tracking a window.
    pub fn untrack_window(&mut self, window: Window) {
        if let Some(tracked) = self.windows.remove(&window) {
            // Destroy damage object
            let _ = self.conn.conn.damage_destroy(tracked.damage);

            // Free pixmap
            if let Some(pixmap) = tracked.pixmap {
                let _ = self.conn.conn.free_pixmap(pixmap);
            }

            // Remove texture from renderer
            self.renderer.remove_window(window);

            tracing::debug!("Untracked window {:#x}", window);
        }
    }

    /// Run the compositor event loop.
    pub fn run(&mut self) -> Result<()> {
        tracing::info!("Starting compositor event loop");

        while self.running {
            let event = self.conn.conn.wait_for_event()?;
            self.handle_event(event)?;
        }

        Ok(())
    }

    /// Handle a single X11 event.
    pub fn handle_event(&mut self, event: Event) -> Result<()> {
        match event {
            Event::CreateNotify(e) => self.handle_create(e)?,
            Event::DestroyNotify(e) => self.handle_destroy(e),
            Event::MapNotify(e) => self.handle_map(e)?,
            Event::UnmapNotify(e) => self.handle_unmap(e),
            Event::ConfigureNotify(e) => self.handle_configure(e)?,
            Event::PropertyNotify(e) => self.handle_property(e)?,
            Event::DamageNotify(e) => self.handle_damage(e),
            Event::Error(e) => {
                tracing::warn!("X11 error: {:?}", e);
            }
            _ => {}
        }

        Ok(())
    }

    fn handle_create(&mut self, event: CreateNotifyEvent) -> Result<()> {
        tracing::trace!("CreateNotify: window {:#x}", event.window);
        Ok(())
    }

    fn handle_destroy(&mut self, event: DestroyNotifyEvent) {
        tracing::trace!("DestroyNotify: window {:#x}", event.window);
        self.untrack_window(event.window);
    }

    fn handle_map(&mut self, event: MapNotifyEvent) -> Result<()> {
        tracing::trace!("MapNotify: window {:#x}", event.window);

        if let Some(tracked) = self.windows.get_mut(&event.window) {
            tracked.mapped = true;
            tracked.damaged = true;

            // Get fresh pixmap
            if let Some(old_pixmap) = tracked.pixmap.take() {
                let _ = self.conn.conn.free_pixmap(old_pixmap);
            }
            tracked.pixmap = self.conn.name_window_pixmap(event.window).ok();

            // Start fade-in animation
            if self.effects.fade_enabled {
                tracked.animations.start_fade_in(std::time::Duration::from_secs_f32(
                    self.effects.fade_in_duration,
                ));
            }
        } else {
            self.track_window(event.window)?;
        }

        self.needs_redraw = true;
        Ok(())
    }

    fn handle_unmap(&mut self, event: UnmapNotifyEvent) {
        tracing::trace!("UnmapNotify: window {:#x}", event.window);

        if let Some(tracked) = self.windows.get_mut(&event.window) {
            // Start fade-out animation if enabled, otherwise hide immediately
            if self.effects.fade_enabled {
                tracked.animations.start_fade_out(std::time::Duration::from_secs_f32(
                    self.effects.fade_out_duration,
                ));
                // Window stays "mapped" for rendering during fade-out
                // It will be marked unmapped once animation completes
            } else {
                tracked.mapped = false;
            }
            self.needs_redraw = true;
        }
    }

    fn handle_configure(&mut self, event: ConfigureNotifyEvent) -> Result<()> {
        // Check if this is a root window configure (screen resize)
        if event.window == self.conn.root() {
            tracing::info!("Screen resized to {}x{}", event.width, event.height);
            self.renderer.resize(event.width as u32, event.height as u32);
            self.needs_redraw = true;
            return Ok(());
        }

        if let Some(tracked) = self.windows.get_mut(&event.window) {
            let size_changed =
                tracked.width != event.width || tracked.height != event.height;

            tracked.x = event.x;
            tracked.y = event.y;
            tracked.width = event.width;
            tracked.height = event.height;
            tracked.border_width = event.border_width;
            tracked.damaged = true;

            if size_changed {
                // Need new pixmap for resized window
                if let Some(old_pixmap) = tracked.pixmap.take() {
                    let _ = self.conn.conn.free_pixmap(old_pixmap);
                }
                if tracked.mapped {
                    tracked.pixmap = self.conn.name_window_pixmap(event.window).ok();
                }
            }

            self.needs_redraw = true;
        }

        Ok(())
    }

    fn handle_property(&mut self, event: PropertyNotifyEvent) -> Result<()> {
        // Check for opacity changes
        if event.atom == self.conn.atoms._NET_WM_WINDOW_OPACITY {
            let opacity = self.get_window_opacity(event.window);
            if let Some(tracked) = self.windows.get_mut(&event.window) {
                tracked.opacity = opacity;
                tracked.damaged = true;
                self.needs_redraw = true;
            }
        }

        // Check for active window changes (on root window)
        if event.window == self.conn.root() && event.atom == self.conn.atoms._NET_ACTIVE_WINDOW {
            let new_active = self.conn.get_active_window();
            if new_active != self.active_window {
                tracing::debug!("Active window changed: {:?} -> {:?}", self.active_window, new_active);
                self.active_window = new_active;
                self.needs_redraw = true;
            }
        }

        // Check for window type changes
        if event.atom == self.conn.atoms._NET_WM_WINDOW_TYPE {
            let window_type = self.get_window_type_enum(event.window);
            if let Some(tracked) = self.windows.get_mut(&event.window) {
                tracked.window_type = window_type;
                tracked.damaged = true;
                self.needs_redraw = true;
            }
        }

        // Check for current desktop change (on root window)
        if event.window == self.conn.root() && event.atom == self.conn.atoms._NET_CURRENT_DESKTOP {
            let new_desktop = self.get_current_desktop();
            if new_desktop != self.workspaces.current {
                tracing::info!("Workspace changed: {} -> {}", self.workspaces.current, new_desktop);
                self.workspaces.set_current(new_desktop);
                self.needs_redraw = true;
            }
        }

        // Check for window desktop/workspace change
        if event.atom == self.conn.atoms._NET_WM_DESKTOP {
            if let Some(desktop) = self.get_window_desktop(event.window) {
                self.workspaces.assign_window(event.window, desktop);
                tracing::debug!("Window {:#x} moved to workspace {}", event.window, desktop);
                self.needs_redraw = true;
            }
        }

        // Check for root pixmap change (wallpaper update)
        if event.window == self.conn.root()
            && (event.atom == self.conn.atoms._XROOTPMAP_ID
                || event.atom == self.conn.atoms.ESETROOT_PMAP_ID)
        {
            let new_pixmap = self.conn.get_root_pixmap();
            if new_pixmap != self.root_pixmap {
                tracing::info!("Root pixmap changed: {:?} -> {:?}", self.root_pixmap, new_pixmap);
                self.root_pixmap = new_pixmap;
                // Tell renderer about new background
                self.renderer.set_root_pixmap(new_pixmap);
                self.needs_redraw = true;
            }
        }

        Ok(())
    }

    fn handle_damage(&mut self, event: DamageNotifyEvent) {
        let drawable = event.drawable;

        if let Some(tracked) = self.windows.get_mut(&drawable) {
            tracked.damaged = true;
            self.needs_redraw = true;
            // Subtract damage region to clear it
            let _ = self.conn.conn.damage_subtract(tracked.damage, 0u32, 0u32);
        }
    }

    fn get_window_opacity(&self, window: Window) -> f32 {
        let cookie = match self.conn.conn.get_property(
            false,
            window,
            self.conn.atoms._NET_WM_WINDOW_OPACITY,
            AtomEnum::CARDINAL,
            0,
            1,
        ) {
            Ok(c) => c,
            Err(_) => return 1.0,
        };

        match cookie.reply() {
            Ok(reply) => {
                if let Some(value) = reply.value32().and_then(|mut v| v.next()) {
                    value as f32 / u32::MAX as f32
                } else {
                    1.0
                }
            }
            Err(_) => 1.0,
        }
    }

    /// Convert _NET_WM_WINDOW_TYPE atom to WindowType enum.
    fn get_window_type_enum(&self, window: Window) -> WindowType {
        let atoms = &self.conn.atoms;

        match self.conn.get_window_type(window) {
            Some(type_atom) if type_atom == atoms._NET_WM_WINDOW_TYPE_DESKTOP => WindowType::Desktop,
            Some(type_atom) if type_atom == atoms._NET_WM_WINDOW_TYPE_DOCK => WindowType::Dock,
            Some(type_atom) if type_atom == atoms._NET_WM_WINDOW_TYPE_TOOLBAR => WindowType::Toolbar,
            Some(type_atom) if type_atom == atoms._NET_WM_WINDOW_TYPE_MENU => WindowType::Menu,
            Some(type_atom) if type_atom == atoms._NET_WM_WINDOW_TYPE_UTILITY => WindowType::Utility,
            Some(type_atom) if type_atom == atoms._NET_WM_WINDOW_TYPE_SPLASH => WindowType::Splash,
            Some(type_atom) if type_atom == atoms._NET_WM_WINDOW_TYPE_DIALOG => WindowType::Dialog,
            Some(type_atom) if type_atom == atoms._NET_WM_WINDOW_TYPE_DROPDOWN_MENU => WindowType::DropdownMenu,
            Some(type_atom) if type_atom == atoms._NET_WM_WINDOW_TYPE_POPUP_MENU => WindowType::PopupMenu,
            Some(type_atom) if type_atom == atoms._NET_WM_WINDOW_TYPE_TOOLTIP => WindowType::Tooltip,
            Some(type_atom) if type_atom == atoms._NET_WM_WINDOW_TYPE_NOTIFICATION => WindowType::Notification,
            Some(type_atom) if type_atom == atoms._NET_WM_WINDOW_TYPE_COMBO => WindowType::Combo,
            Some(type_atom) if type_atom == atoms._NET_WM_WINDOW_TYPE_DND => WindowType::Dnd,
            _ => WindowType::Normal,
        }
    }

    /// Check if a window is currently focused.
    pub fn is_window_focused(&self, window: Window) -> bool {
        self.active_window == Some(window)
    }

    /// Check if a window is fullscreen.
    fn is_window_fullscreen(&self, window: Window) -> bool {
        let cookie = match self.conn.conn.get_property(
            false,
            window,
            self.conn.atoms._NET_WM_STATE,
            AtomEnum::ATOM,
            0,
            64,
        ) {
            Ok(c) => c,
            Err(_) => return false,
        };

        match cookie.reply() {
            Ok(reply) => {
                if let Some(atoms) = reply.value32() {
                    for atom in atoms {
                        if atom == self.conn.atoms._NET_WM_STATE_FULLSCREEN {
                            return true;
                        }
                    }
                }
                false
            }
            Err(_) => false,
        }
    }

    /// Get the workspace/desktop for a window from _NET_WM_DESKTOP.
    fn get_window_desktop(&self, window: Window) -> Option<usize> {
        let cookie = match self.conn.conn.get_property(
            false,
            window,
            self.conn.atoms._NET_WM_DESKTOP,
            AtomEnum::CARDINAL,
            0,
            1,
        ) {
            Ok(c) => c,
            Err(_) => return None,
        };

        match cookie.reply() {
            Ok(reply) => {
                reply.value32().and_then(|mut v| v.next()).map(|d| d as usize)
            }
            Err(_) => None,
        }
    }

    /// Get the current desktop from _NET_CURRENT_DESKTOP.
    fn get_current_desktop(&self) -> usize {
        let cookie = match self.conn.conn.get_property(
            false,
            self.conn.root(),
            self.conn.atoms._NET_CURRENT_DESKTOP,
            AtomEnum::CARDINAL,
            0,
            1,
        ) {
            Ok(c) => c,
            Err(_) => return 0,
        };

        match cookie.reply() {
            Ok(reply) => {
                reply.value32().and_then(|mut v| v.next()).map(|d| d as usize).unwrap_or(0)
            }
            Err(_) => 0,
        }
    }

    /// Get WM_CLASS (class, instance) for a window.
    fn get_window_class(&self, window: Window) -> (Option<String>, Option<String>) {
        let cookie = match self.conn.conn.get_property(
            false,
            window,
            AtomEnum::WM_CLASS,
            AtomEnum::STRING,
            0,
            256,
        ) {
            Ok(c) => c,
            Err(_) => return (None, None),
        };

        match cookie.reply() {
            Ok(reply) => {
                let value = reply.value;
                if value.is_empty() {
                    return (None, None);
                }
                // WM_CLASS format: "instance\0class\0"
                let parts: Vec<&[u8]> = value.split(|&b| b == 0).collect();
                let instance = parts.first()
                    .filter(|p| !p.is_empty())
                    .and_then(|p| std::str::from_utf8(p).ok())
                    .map(String::from);
                let class = parts.get(1)
                    .filter(|p| !p.is_empty())
                    .and_then(|p| std::str::from_utf8(p).ok())
                    .map(String::from);
                (class, instance)
            }
            Err(_) => (None, None),
        }
    }

    /// Get window title from _NET_WM_NAME (preferred) or WM_NAME (fallback).
    fn get_window_name(&self, window: Window) -> Option<String> {
        // Try _NET_WM_NAME first (UTF-8)
        if let Ok(cookie) = self.conn.conn.get_property(
            false,
            window,
            self.conn.atoms._NET_WM_NAME,
            self.conn.atoms.UTF8_STRING,
            0,
            1024,
        ) {
            if let Ok(reply) = cookie.reply() {
                if !reply.value.is_empty() {
                    if let Ok(name) = std::str::from_utf8(&reply.value) {
                        return Some(name.trim_end_matches('\0').to_string());
                    }
                }
            }
        }

        // Fall back to WM_NAME (Latin-1, but often ASCII)
        if let Ok(cookie) = self.conn.conn.get_property(
            false,
            window,
            AtomEnum::WM_NAME,
            AtomEnum::STRING,
            0,
            1024,
        ) {
            if let Ok(reply) = cookie.reply() {
                if !reply.value.is_empty() {
                    if let Ok(name) = std::str::from_utf8(&reply.value) {
                        return Some(name.trim_end_matches('\0').to_string());
                    }
                }
            }
        }

        None
    }

    /// Render a frame if needed.
    pub fn render(&mut self) -> Result<()> {
        if !self.needs_redraw {
            return Ok(());
        }

        // Update animations and check for completions
        let mut has_active_animations = false;
        let mut windows_to_unmap = Vec::new();

        for (id, w) in self.windows.iter_mut() {
            w.animations.cleanup_completed();
            w.cleanup_lua_animation();
            if w.animations.has_active_animations() {
                has_active_animations = true;
            }
            if w.has_lua_animation() {
                has_active_animations = true;
            }
            // Mark windows that finished fade-out as unmapped
            if w.animations.fade_out_complete() {
                windows_to_unmap.push(*id);
            }
        }

        // Call Lua animation callbacks for active animations
        self.update_lua_animations();

        // Unmap windows that finished fading out
        for id in windows_to_unmap {
            if let Some(w) = self.windows.get_mut(&id) {
                w.mapped = false;
                w.animations.opacity = None;
            }
        }

        // Update workspace transitions
        if self.workspaces.update_transition() {
            has_active_animations = true;
        }

        // Get proper stacking order from WM (bottom to top)
        let stacking_order = self.conn.get_stacking_order().unwrap_or_default();

        // Collect window info for rendering in stacking order
        let mut windows: Vec<WindowRenderInfo> = Vec::new();
        for window_id in stacking_order {
            if let Some(w) = self.windows.get(&window_id) {
                if w.mapped && w.pixmap.is_some() {
                    // Check workspace visibility - only render windows on current workspace,
                    // windows in transition, or windows not assigned to any workspace (docks, etc.)
                    let window_ws = self.workspaces.get_window_workspace(w.id);
                    let should_render = match window_ws {
                        None => true, // Not tracked by workspace (override-redirect, docks)
                        Some(ws) => {
                            ws == self.workspaces.current ||
                            self.workspaces.transition.as_ref().map_or(false, |t| {
                                ws == t.from || ws == t.to
                            })
                        }
                    };

                    if !should_render {
                        continue;
                    }

                    let focused = self.is_window_focused(w.id);
                    // Get Lua animation transform
                    let lua_transform = w.lua_transform();
                    // Get effective opacity: rule override takes priority over focus-based
                    let base_opacity = if w.rule_overrides.opacity.is_some() || w.fullscreen {
                        w.effective_opacity()
                    } else {
                        self.effects.effective_opacity(w.opacity, focused)
                    };
                    // Apply animation opacity multiplier and Lua transform opacity
                    let opacity = base_opacity * w.animations.opacity_multiplier() * lua_transform.opacity;
                    // Get effective corner radius (rule override > fullscreen > window type)
                    let corner_radius = if lua_transform.corner_radius > 0.0 {
                        lua_transform.corner_radius
                    } else {
                        w.effective_corner_radius()
                    };

                    // Calculate workspace transition offset
                    let screen = self.conn.screen();
                    let screen_width = screen.width_in_pixels as f32;
                    let screen_height = screen.height_in_pixels as f32;

                    let transition_offset = if let Some(ref transition) = self.workspaces.transition {
                        if let Some(ws) = window_ws {
                            if ws == transition.from {
                                // Window on "from" workspace: slide out
                                transition.from_offset(screen_width, screen_height)
                            } else if ws == transition.to {
                                // Window on "to" workspace: slide in
                                transition.to_offset(screen_width, screen_height)
                            } else {
                                (0.0, 0.0)
                            }
                        } else {
                            (0.0, 0.0) // Untracked windows don't move
                        }
                    } else {
                        (0.0, 0.0)
                    };

                    // Combine Lua transform offset with transition offset
                    let final_offset = (
                        lua_transform.offset.0 + transition_offset.0,
                        lua_transform.offset.1 + transition_offset.1,
                    );

                    windows.push(WindowRenderInfo {
                        id: w.id,
                        pixmap: w.pixmap.unwrap() as u64,
                        x: w.x,
                        y: w.y,
                        width: w.width,
                        height: w.height,
                        opacity,
                        corner_radius,
                        shadow_enabled: w.should_have_shadow() && self.effects.shadows_active(),
                        blur_behind: w.should_have_blur() && self.effects.blur_active(),
                        focused,
                        scale: lua_transform.scale,
                        offset: final_offset,
                    });
                }
            }
        }

        // Also include any windows we're tracking that aren't in stacking list
        // (e.g., override-redirect windows not managed by WM)
        for w in self.windows.values() {
            if w.mapped && w.pixmap.is_some() && !windows.iter().any(|wi| wi.id == w.id) {
                // Check workspace visibility - only render windows on current workspace,
                // windows in transition, or windows not assigned to any workspace (docks, etc.)
                let window_ws = self.workspaces.get_window_workspace(w.id);
                let should_render = match window_ws {
                    None => true, // Not tracked by workspace (override-redirect, docks)
                    Some(ws) => {
                        ws == self.workspaces.current ||
                        self.workspaces.transition.as_ref().map_or(false, |t| {
                            ws == t.from || ws == t.to
                        })
                    }
                };

                if !should_render {
                    continue;
                }

                let focused = self.is_window_focused(w.id);
                // Get Lua animation transform
                let lua_transform = w.lua_transform();
                // Get effective opacity: rule override takes priority over focus-based
                let base_opacity = if w.rule_overrides.opacity.is_some() || w.fullscreen {
                    w.effective_opacity()
                } else {
                    self.effects.effective_opacity(w.opacity, focused)
                };
                let opacity = base_opacity * w.animations.opacity_multiplier() * lua_transform.opacity;
                // Get effective corner radius (rule override > fullscreen > window type)
                let corner_radius = if lua_transform.corner_radius > 0.0 {
                    lua_transform.corner_radius
                } else {
                    w.effective_corner_radius()
                };

                // Calculate workspace transition offset
                let screen = self.conn.screen();
                let screen_width = screen.width_in_pixels as f32;
                let screen_height = screen.height_in_pixels as f32;

                let transition_offset = if let Some(ref transition) = self.workspaces.transition {
                    if let Some(ws) = window_ws {
                        if ws == transition.from {
                            transition.from_offset(screen_width, screen_height)
                        } else if ws == transition.to {
                            transition.to_offset(screen_width, screen_height)
                        } else {
                            (0.0, 0.0)
                        }
                    } else {
                        (0.0, 0.0)
                    }
                } else {
                    (0.0, 0.0)
                };

                let final_offset = (
                    lua_transform.offset.0 + transition_offset.0,
                    lua_transform.offset.1 + transition_offset.1,
                );

                windows.push(WindowRenderInfo {
                    id: w.id,
                    pixmap: w.pixmap.unwrap() as u64,
                    x: w.x,
                    y: w.y,
                    width: w.width,
                    height: w.height,
                    opacity,
                    corner_radius,
                    shadow_enabled: w.should_have_shadow() && self.effects.shadows_active(),
                    blur_behind: w.should_have_blur() && self.effects.blur_active(),
                    focused,
                    scale: lua_transform.scale,
                    offset: final_offset,
                });
            }
        }

        // Keep redrawing while animations are active
        if has_active_animations {
            self.needs_redraw = true;
        }

        // Render the windows
        self.renderer.render_windows(&windows)?;
        self.needs_redraw = false;

        // Clear damage flags on windows
        for window in self.windows.values_mut() {
            window.damaged = false;
        }

        Ok(())
    }

    /// Mark the compositor as needing a redraw.
    pub fn request_redraw(&mut self) {
        self.needs_redraw = true;
    }

    /// Check if a redraw is needed.
    pub fn needs_redraw(&self) -> bool {
        self.needs_redraw
    }

    /// Resize the renderer surface.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.renderer.resize(width, height);
        self.needs_redraw = true;
    }

    /// Handle an event from gar window manager.
    pub fn handle_gar_event(&mut self, event: GarEvent) {
        match event {
            GarEvent::WorkspaceChanged { from, to, direction } => {
                self.handle_workspace_change(from, to, &direction);
            }
            GarEvent::FocusChanged { old, new } => {
                self.handle_focus_change(old, new);
            }
            GarEvent::WindowFullscreen { window, fullscreen } => {
                self.handle_fullscreen(window, fullscreen);
            }
            GarEvent::WindowMoved { window, x, y } => {
                if let Some(tracked) = self.windows.get_mut(&window) {
                    tracked.x = x as i16;
                    tracked.y = y as i16;
                    tracked.damaged = true;
                    self.needs_redraw = true;
                }
            }
            GarEvent::WindowResized { window, width, height } => {
                if let Some(tracked) = self.windows.get_mut(&window) {
                    tracked.width = width as u16;
                    tracked.height = height as u16;
                    tracked.damaged = true;
                    self.needs_redraw = true;
                }
            }
            GarEvent::WindowWorkspace { window, workspace } => {
                self.workspaces.assign_window(window, workspace);
                self.needs_redraw = true;
            }
            GarEvent::Sync { windows, current_workspace } => {
                tracing::info!("Received sync from gar: {} windows, workspace {}",
                    windows.len(), current_workspace);
                self.workspaces.set_current(current_workspace);
                // TODO: sync window workspace assignments
                self.needs_redraw = true;
            }
        }
    }

    /// Handle workspace change event from gar.
    fn handle_workspace_change(&mut self, from: usize, to: usize, direction: &str) {
        tracing::debug!("Workspace change: {} -> {} ({})", from, to, direction);
        self.workspaces.start_transition(from, to, direction);
        self.needs_redraw = true;
    }

    /// Handle focus change event from gar.
    fn handle_focus_change(&mut self, old: Option<u32>, new: Option<u32>) {
        tracing::debug!("Focus change: {:?} -> {:?}", old, new);

        // Update active window
        self.active_window = new;

        // Trigger unfocus animation on old window
        if let Some(old_id) = old {
            if let Some(tracked) = self.windows.get_mut(&old_id) {
                if self.effects.fade_enabled {
                    // Start a subtle dim animation for unfocused window
                    tracked.animations.opacity = Some(Animation::new(
                        1.0,
                        self.effects.opacity_unfocused,
                        std::time::Duration::from_millis(150),
                        Easing::EaseOut,
                    ));
                }
            }
        }

        // Trigger focus animation on new window
        if let Some(new_id) = new {
            if let Some(tracked) = self.windows.get_mut(&new_id) {
                if self.effects.fade_enabled {
                    // Brighten focused window
                    tracked.animations.opacity = Some(Animation::new(
                        self.effects.opacity_unfocused,
                        1.0,
                        std::time::Duration::from_millis(150),
                        Easing::EaseOut,
                    ));
                }
            }
        }

        self.needs_redraw = true;
    }

    /// Handle fullscreen state change.
    fn handle_fullscreen(&mut self, window: Window, fullscreen: bool) {
        tracing::debug!("Fullscreen change: {} = {}", window, fullscreen);

        if fullscreen {
            // Unredirect window for direct rendering (bypass compositor)
            if let Err(e) = self.conn.unredirect_window(window) {
                tracing::warn!("Failed to unredirect fullscreen window: {}", e);
            }
        } else {
            // Redirect window back to compositor
            if let Err(e) = self.conn.redirect_window(window) {
                tracing::warn!("Failed to redirect window: {}", e);
            }
        }

        if let Some(tracked) = self.windows.get_mut(&window) {
            tracked.damaged = true;
        }
        self.needs_redraw = true;
    }

    /// Poll for gar events.
    pub fn poll_gar(&mut self) {
        // Try to reconnect if disconnected
        self.gar.try_reconnect();

        // Process any pending events
        while let Some(event) = self.gar.poll() {
            self.handle_gar_event(event);
        }
    }

    /// Check if connected to gar.
    pub fn is_connected_to_gar(&self) -> bool {
        self.gar.is_connected()
    }

    /// Sync workspace state by re-reading _NET_WM_DESKTOP for all windows.
    /// Called on gar reconnect to ensure we have accurate workspace assignments.
    pub fn sync_workspaces_from_gar(&mut self) {
        tracing::info!("Syncing workspace state from X11 properties");

        // Query current workspace from _NET_CURRENT_DESKTOP
        if let Ok(cookie) = self.conn.conn.get_property(
            false,
            self.conn.root(),
            self.conn.atoms._NET_CURRENT_DESKTOP,
            AtomEnum::CARDINAL,
            0,
            1,
        ) {
            if let Ok(reply) = cookie.reply() {
                if let Some(workspace) = reply.value32().and_then(|mut v| v.next()) {
                    let ws = workspace as usize;
                    if ws != self.workspaces.current {
                        tracing::debug!("Synced current workspace: {} -> {}", self.workspaces.current, ws);
                        self.workspaces.set_current(ws);
                    }
                }
            }
        }

        // Re-read _NET_WM_DESKTOP for all tracked windows
        let window_ids: Vec<Window> = self.windows.keys().copied().collect();
        for window_id in window_ids {
            if let Ok(cookie) = self.conn.conn.get_property(
                false,
                window_id,
                self.conn.atoms._NET_WM_DESKTOP,
                AtomEnum::CARDINAL,
                0,
                1,
            ) {
                if let Ok(reply) = cookie.reply() {
                    if let Some(workspace) = reply.value32().and_then(|mut v| v.next()) {
                        // 0xFFFFFFFF means "all workspaces" (sticky)
                        if workspace != 0xFFFFFFFF {
                            self.workspaces.assign_window(window_id, workspace as usize);
                        }
                    }
                }
            }
        }

        self.needs_redraw = true;
        tracing::info!("Workspace sync complete, current: {}", self.workspaces.current);
    }

    /// Get the path to the current config file.
    pub fn config_path(&self) -> Option<std::path::PathBuf> {
        self.lua_config.as_ref().map(|lua| lua.config_path().clone())
    }

    /// Reload configuration from Lua file.
    ///
    /// Returns Ok(()) if successful, or a string error message if failed.
    pub fn reload_config(&mut self) -> std::result::Result<(), String> {
        let lua_config = match &mut self.lua_config {
            Some(lua) => lua,
            None => {
                // Try to create a new LuaConfig if we don't have one
                match LuaConfig::new() {
                    Ok(lua) => {
                        self.lua_config = Some(lua);
                        self.lua_config.as_mut().unwrap()
                    }
                    Err(e) => return Err(format!("Failed to create Lua config: {}", e)),
                }
            }
        };

        // Reload the config file
        if let Err(e) = lua_config.load() {
            return Err(format!("Failed to load config: {}", e));
        }

        // Apply new effects configuration
        self.effects = EffectsConfig::from_lua_config(lua_config);

        tracing::info!(
            "Reloaded config: blur={}, shadows={}, corners={:.1}px, fade={}",
            self.effects.blur_enabled,
            self.effects.shadow_enabled,
            self.effects.corner_radius,
            self.effects.fade_enabled
        );

        // Update renderer's shadow config
        use crate::render::ShadowConfig;
        self.renderer.update_shadow_config(ShadowConfig {
            color: self.effects.shadow_color,
            opacity: self.effects.shadow_opacity,
            spread: self.effects.shadow_radius, // spread controls shadow size
            blur_radius: self.effects.shadow_radius,
            offset: [self.effects.shadow_offset.0, self.effects.shadow_offset.1],
        });

        // Update renderer's blur config
        self.renderer.update_blur_config(
            self.effects.blur_enabled,
            self.effects.blur_strength,
        );

        // Check for HDR config changes
        let hdr_config = lua_config.get_hdr_config();
        if hdr_config.enabled {
            self.renderer.enable_hdr(hdr_config);
        } else {
            self.renderer.disable_hdr();
        }

        // Request redraw with new settings
        self.needs_redraw = true;

        Ok(())
    }

    /// Shutdown the compositor cleanly.
    pub fn shutdown(&mut self) -> Result<()> {
        tracing::info!("Shutting down compositor");

        // Clean up tracked windows
        let windows: Vec<Window> = self.windows.keys().copied().collect();
        for window in windows {
            self.untrack_window(window);
        }

        // Release overlay
        self.conn.release_overlay_window()?;
        self.conn.flush()?;

        Ok(())
    }
}

impl Drop for Compositor {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

/// Convert WindowType enum to string for rule matching.
fn window_type_to_string(wtype: WindowType) -> String {
    match wtype {
        WindowType::Normal => "normal".to_string(),
        WindowType::Desktop => "desktop".to_string(),
        WindowType::Dock => "dock".to_string(),
        WindowType::Toolbar => "toolbar".to_string(),
        WindowType::Menu => "menu".to_string(),
        WindowType::Utility => "utility".to_string(),
        WindowType::Splash => "splash".to_string(),
        WindowType::Dialog => "dialog".to_string(),
        WindowType::DropdownMenu => "dropdown_menu".to_string(),
        WindowType::PopupMenu => "popup_menu".to_string(),
        WindowType::Tooltip => "tooltip".to_string(),
        WindowType::Notification => "notification".to_string(),
        WindowType::Combo => "combo".to_string(),
        WindowType::Dnd => "dnd".to_string(),
    }
}

/// Build WindowRuleOverrides from a list of matching rules.
/// Later rules override earlier ones.
fn build_rule_overrides(rules: &[&crate::config::WindowRule]) -> WindowRuleOverrides {
    let mut overrides = WindowRuleOverrides::default();
    for rule in rules {
        if let Some(shadow) = rule.shadow {
            overrides.shadow = Some(shadow);
        }
        if let Some(blur) = rule.blur_behind {
            overrides.blur_behind = Some(blur);
        }
        if let Some(opacity) = rule.opacity {
            overrides.opacity = Some(opacity);
        }
        if let Some(corner_radius) = rule.corner_radius {
            overrides.corner_radius = Some(corner_radius);
        }
    }
    overrides
}
