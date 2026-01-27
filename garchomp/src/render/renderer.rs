//! High-level renderer that manages the GPU context and rendering passes.

use super::{GpuContext, GpuError};
use wgpu::Color;

/// The main renderer for the compositor.
pub struct Renderer {
    pub gpu: GpuContext,
    clear_color: Color,
}

impl Renderer {
    /// Create a new renderer for the given overlay window.
    pub async fn new(window: u32, width: u32, height: u32) -> Result<Self, GpuError> {
        let gpu = GpuContext::new(window, width, height).await?;

        Ok(Self {
            gpu,
            clear_color: Color {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 1.0,
            },
        })
    }

    /// Resize the render surface.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.gpu.resize(width, height);
    }

    /// Set the clear color (background).
    pub fn set_clear_color(&mut self, r: f64, g: f64, b: f64, a: f64) {
        self.clear_color = Color { r, g, b, a };
    }

    /// Render a frame - currently just clears to the background color.
    pub fn render(&self) -> Result<(), GpuError> {
        let frame = self.gpu.begin_frame()?;
        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self.gpu.create_encoder();

        // Clear pass - just clear to background color for now
        {
            let _render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("clear_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(self.clear_color),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            // Render pass ends here when _render_pass is dropped
        }

        self.gpu.end_frame(encoder, frame);
        Ok(())
    }

    /// Poll the GPU device and sync display.
    pub fn poll_and_sync(&self) {
        self.gpu.poll_and_sync();
    }

    /// Get current surface dimensions.
    pub fn dimensions(&self) -> (u32, u32) {
        self.gpu.dimensions()
    }
}
