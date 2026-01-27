//! GPU rendering module for garchomp compositor.

mod gpu;
mod xlib;

pub use gpu::{GpuContext, GpuError};
pub use xlib::{XlibDisplay, XlibWindowHandle};
