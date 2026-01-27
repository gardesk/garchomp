//! High-level renderer that manages the GPU context and rendering passes.

use super::{BlurPipeline, BlurTechnique, CompositePipeline, GpuContext, GpuError, ShadowConfig, ShadowPipeline, TextureManager};
use std::collections::HashMap;
use wgpu::Color;

/// Window render data - texture and bind group for a window.
pub struct WindowRenderData {
    pub texture: wgpu::Texture,
    pub texture_view: wgpu::TextureView,
    pub bind_group: wgpu::BindGroup,
    pub width: u32,
    pub height: u32,
}

/// Information about a window to render.
#[derive(Clone, Debug)]
pub struct WindowRenderInfo {
    pub id: u32,
    pub pixmap: u64,
    pub x: i16,
    pub y: i16,
    pub width: u16,
    pub height: u16,
    pub opacity: f32,
    pub corner_radius: f32,
    pub shadow_enabled: bool,
    pub blur_behind: bool,
}

/// Blur configuration.
#[derive(Debug, Clone, Copy)]
pub struct BlurConfig {
    pub enabled: bool,
    pub technique: BlurTechnique,
    pub iterations: u32,
    pub strength: f32,
}

impl Default for BlurConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            technique: BlurTechnique::DualKawase,
            iterations: 4,
            strength: 1.0,
        }
    }
}

/// The main renderer for the compositor.
pub struct Renderer {
    pub gpu: GpuContext,
    pub pipeline: CompositePipeline,
    pub shadow_pipeline: ShadowPipeline,
    pub blur_pipeline: BlurPipeline,
    pub shadow_config: ShadowConfig,
    pub blur_config: BlurConfig,
    pub texture_manager: TextureManager,
    clear_color: Color,
    // Test texture for validating the pipeline
    test_texture: Option<WindowRenderData>,
    // Bind groups for windows (keyed by window ID)
    window_bind_groups: HashMap<u32, wgpu::BindGroup>,
}

impl Renderer {
    /// Create a new renderer for the given overlay window.
    pub async fn new(window: u32, width: u32, height: u32) -> Result<Self, GpuError> {
        let gpu = GpuContext::new(window, width, height).await?;

        // Create the composite pipeline
        let pipeline = CompositePipeline::new(&gpu.device, gpu.format());

        // Create shadow pipeline
        let shadow_pipeline = ShadowPipeline::new(&gpu.device, gpu.format());

        // Create blur pipeline
        let blur_pipeline = BlurPipeline::new(&gpu.device, gpu.format());

        // Create texture manager using the same display connection
        let texture_manager = TextureManager::new(gpu.display_ptr())?;

        // Create a test texture (checkerboard pattern) to validate rendering
        let test_texture = Self::create_test_texture(&gpu, &pipeline);

        Ok(Self {
            gpu,
            pipeline,
            shadow_pipeline,
            blur_pipeline,
            shadow_config: ShadowConfig::default(),
            blur_config: BlurConfig::default(),
            texture_manager,
            clear_color: Color {
                r: 0.1,
                g: 0.1,
                b: 0.15,
                a: 1.0,
            },
            test_texture: Some(test_texture),
            window_bind_groups: HashMap::new(),
        })
    }

    /// Create a test checkerboard texture.
    fn create_test_texture(gpu: &GpuContext, pipeline: &CompositePipeline) -> WindowRenderData {
        let width = 256u32;
        let height = 256u32;
        let cell_size = 32u32;

        // Generate checkerboard pattern
        let mut pixels = vec![0u8; (width * height * 4) as usize];
        for y in 0..height {
            for x in 0..width {
                let idx = ((y * width + x) * 4) as usize;
                let cell_x = x / cell_size;
                let cell_y = y / cell_size;
                let is_dark = (cell_x + cell_y) % 2 == 0;

                if is_dark {
                    pixels[idx] = 60;      // R
                    pixels[idx + 1] = 60;  // G
                    pixels[idx + 2] = 80;  // B
                    pixels[idx + 3] = 255; // A
                } else {
                    pixels[idx] = 100;     // R
                    pixels[idx + 1] = 100; // G
                    pixels[idx + 2] = 140; // B
                    pixels[idx + 3] = 255; // A
                }
            }
        }

        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("test_texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        gpu.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );

        let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = pipeline.create_bind_group(&gpu.device, &texture_view);

        WindowRenderData {
            texture,
            texture_view,
            bind_group,
            width,
            height,
        }
    }

    /// Resize the render surface.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.gpu.resize(width, height);
    }

    /// Set the clear color (background).
    pub fn set_clear_color(&mut self, r: f64, g: f64, b: f64, a: f64) {
        self.clear_color = Color { r, g, b, a };
    }

    /// Render a frame with the test texture to validate the pipeline.
    pub fn render_test(&mut self) -> Result<(), GpuError> {
        let frame = self.gpu.begin_frame()?;
        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self.gpu.create_encoder();

        // Render pass
        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("composite_pass"),
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

            // Render test texture if available
            if let Some(test) = &self.test_texture {
                let (vw, vh) = self.gpu.dimensions();
                // Center the test texture
                let x = (vw as f32 - test.width as f32) / 2.0;
                let y = (vh as f32 - test.height as f32) / 2.0;

                self.pipeline.update_uniforms(
                    &self.gpu.queue,
                    x,
                    y,
                    test.width as f32,
                    test.height as f32,
                    vw as f32,
                    vh as f32,
                    1.0,
                    20.0, // Test with rounded corners
                );

                self.pipeline.render(&mut render_pass, &test.bind_group);
            }
        }

        self.gpu.end_frame(encoder, frame);
        Ok(())
    }

    /// Render a frame with the given windows.
    pub fn render_windows(&mut self, windows: &[WindowRenderInfo]) -> Result<(), GpuError> {
        // Update textures for all windows that have pixmaps
        for win in windows {
            if win.pixmap != 0 && win.width > 0 && win.height > 0 {
                if let Ok(tex) = self.texture_manager.update_texture(
                    &self.gpu.device,
                    &self.gpu.queue,
                    win.id,
                    win.pixmap,
                    win.width as u32,
                    win.height as u32,
                ) {
                    // Create or update bind group for this window
                    let bind_group = self.pipeline.create_bind_group(&self.gpu.device, &tex.view);
                    self.window_bind_groups.insert(win.id, bind_group);
                }
            }
        }

        let frame = self.gpu.begin_frame()?;
        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self.gpu.create_encoder();

        // Render pass
        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("composite_pass"),
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

            let (vw, vh) = self.gpu.dimensions();

            // First pass: render shadows for all windows (back to front)
            for win in windows {
                if win.shadow_enabled {
                    self.shadow_pipeline.update_uniforms(
                        &self.gpu.queue,
                        win.x as f32,
                        win.y as f32,
                        win.width as f32,
                        win.height as f32,
                        vw as f32,
                        vh as f32,
                        win.corner_radius,
                        &self.shadow_config,
                    );
                    self.shadow_pipeline.render(&mut render_pass);
                }
            }

            // Second pass: render windows (back to front)
            for win in windows {
                if let Some(bind_group) = self.window_bind_groups.get(&win.id) {
                    self.pipeline.update_uniforms(
                        &self.gpu.queue,
                        win.x as f32,
                        win.y as f32,
                        win.width as f32,
                        win.height as f32,
                        vw as f32,
                        vh as f32,
                        win.opacity,
                        win.corner_radius,
                    );

                    self.pipeline.render(&mut render_pass, bind_group);
                }
            }
        }

        self.gpu.end_frame(encoder, frame);
        Ok(())
    }

    /// Render a frame - currently renders the test pattern.
    pub fn render(&mut self) -> Result<(), GpuError> {
        // For now, just render the test pattern
        self.render_test()
    }

    /// Remove texture and bind group for a window.
    pub fn remove_window(&mut self, window_id: u32) {
        self.texture_manager.remove_texture(window_id);
        self.window_bind_groups.remove(&window_id);
    }

    /// Poll the GPU device and sync display.
    pub fn poll_and_sync(&self) {
        self.gpu.poll_and_sync();
    }

    /// Get current surface dimensions.
    pub fn dimensions(&self) -> (u32, u32) {
        self.gpu.dimensions()
    }

    /// Get the GPU device.
    pub fn device(&self) -> &wgpu::Device {
        &self.gpu.device
    }

    /// Get the GPU queue.
    pub fn queue(&self) -> &wgpu::Queue {
        &self.gpu.queue
    }
}
