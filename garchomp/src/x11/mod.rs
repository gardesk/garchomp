//! X11 protocol layer for garchomp compositor.

mod atoms;
mod composite;
mod connection;

pub use atoms::Atoms;
pub use composite::CompositeExt;
pub use connection::{Connection, ConnectionError};
