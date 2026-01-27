//! GPU rendering module for garchomp compositor.

mod gpu;
mod renderer;
mod xlib;

pub use gpu::{GpuContext, GpuError};
pub use renderer::Renderer;
pub use xlib::{XlibDisplay, XlibWindowHandle};
