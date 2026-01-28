//! Lua configuration and window rules for garchomp.

mod lua;
mod watcher;
mod window_proxy;

pub use lua::{AnimationTrigger, LuaConfig, RuleAnimation, WindowRule};
pub use watcher::{ConfigEvent, ConfigWatcher};
pub use window_proxy::{WindowAnimationProxy, WindowTransform};
