//! GPU rendering module for garchomp compositor.

mod gpu;
mod pipeline;
mod renderer;
mod xlib;

pub use gpu::{GpuContext, GpuError};
pub use pipeline::{CompositePipeline, Uniforms, Vertex};
pub use renderer::Renderer;
pub use xlib::{XlibDisplay, XlibWindowHandle};
