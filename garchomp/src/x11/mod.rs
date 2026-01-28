//! X11 protocol layer for garchomp compositor.

mod atoms;
mod composite;
mod connection;
mod visual;

pub use atoms::Atoms;
pub use composite::CompositeExt;
pub use connection::{Connection, ConnectionError, MonitorInfo};
pub use visual::{VisualConfig, create_colormap, find_best_visual, get_visual_depth, has_deepcolor_support};
