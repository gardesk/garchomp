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
    /// Whether this window is currently focused.
    pub focused: bool,
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

/// Intermediate render target for multi-pass rendering.
struct IntermediateTarget {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    width: u32,
    height: u32,
}

impl IntermediateTarget {
    fn new(device: &wgpu::Device, width: u32, height: u32, format: wgpu::TextureFormat) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("intermediate_render_target"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        Self {
            texture,
            view,
            width,
            height,
        }
    }

    fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32, format: wgpu::TextureFormat) {
        if self.width != width || self.height != height {
            *self = Self::new(device, width, height, format);
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
    // Intermediate render target for multi-pass rendering (blur support)
    intermediate_texture: Option<IntermediateTarget>,
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
            intermediate_texture: None,
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
        // Invalidate intermediate texture so it gets recreated at new size
        self.intermediate_texture = None;
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

        // Check if any window needs blur
        let needs_blur = self.blur_config.enabled
            && windows.iter().any(|w| w.blur_behind);

        if needs_blur {
            self.render_windows_with_blur(windows)
        } else {
            self.render_windows_simple(windows)
        }
    }

    /// Simple render path when no blur is needed.
    fn render_windows_simple(&mut self, windows: &[WindowRenderInfo]) -> Result<(), GpuError> {
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

    /// Render path with blur support for transparent windows.
    fn render_windows_with_blur(&mut self, windows: &[WindowRenderInfo]) -> Result<(), GpuError> {
        let (vw, vh) = self.gpu.dimensions();
        let format = self.gpu.format();

        // Ensure intermediate texture exists and is correct size
        if self.intermediate_texture.is_none() {
            self.intermediate_texture = Some(IntermediateTarget::new(
                &self.gpu.device,
                vw,
                vh,
                format,
            ));
        } else if let Some(ref mut it) = self.intermediate_texture {
            it.resize(&self.gpu.device, vw, vh, format);
        }

        // Get frame surface
        let frame = self.gpu.begin_frame()?;
        let surface_view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self.gpu.create_encoder();

        // Get intermediate target view (we need to reborrow after encoder creation)
        let intermediate_view = &self.intermediate_texture.as_ref().unwrap().view;

        // Phase 1: Render shadows and non-blur windows to intermediate
        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("composite_phase1"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: intermediate_view,
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

            // Render shadows for all windows
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

            // Render windows that don't need blur
            for win in windows {
                if !win.blur_behind {
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
        }
        // Render pass ends here, intermediate texture now contains background

        // Phase 2: Apply blur and render blur windows
        // Run blur on the intermediate texture
        let blur_iterations = self.blur_config.iterations;
        let blur_strength = self.blur_config.strength;

        let blurred_view = self.blur_pipeline.blur(
            &self.gpu.device,
            &self.gpu.queue,
            &mut encoder,
            intermediate_view,
            vw,
            vh,
            blur_iterations,
            blur_strength,
        );

        // Create bind group for blurred texture (if available) before render pass
        let blur_bind_group = blurred_view.map(|view| {
            self.pipeline.create_bind_group(&self.gpu.device, view)
        });

        // Phase 3: Final composition to surface
        // We need to composite: background + blurred regions + blur windows
        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("composite_final"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &surface_view,
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

            // Re-render shadows
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

            // Render all windows, using blurred background for blur_behind windows
            for win in windows {
                if win.blur_behind {
                    // First draw blurred background in window region
                    if let Some(ref bg) = blur_bind_group {
                        // Draw blurred background in window region
                        self.pipeline.update_uniforms(
                            &self.gpu.queue,
                            win.x as f32,
                            win.y as f32,
                            win.width as f32,
                            win.height as f32,
                            vw as f32,
                            vh as f32,
                            1.0, // Opaque blur background
                            win.corner_radius,
                        );
                        self.pipeline.render(&mut render_pass, bg);
                    }

                    // Then draw the transparent window on top
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
                } else {
                    // Non-blur window, render normally
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
