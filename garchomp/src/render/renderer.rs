//! High-level renderer that manages the GPU context and rendering passes.

use super::{BlurPipeline, BlurTechnique, CompositePipeline, GpuContext, GpuError, ShadowConfig, ShadowPipeline, TextureManager};
use super::hdr::{HdrConfig, HdrRenderTarget, TonemapPipeline};
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
    /// Scale factor from Lua animation (x, y). Default: (1.0, 1.0).
    pub scale: (f32, f32),
    /// Position offset from Lua animation (x, y). Default: (0.0, 0.0).
    pub offset: (f32, f32),
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
    pub hdr_config: HdrConfig,
    pub texture_manager: TextureManager,
    clear_color: Color,
    // Test texture for validating the pipeline
    test_texture: Option<WindowRenderData>,
    // Test uniform buffer for test texture
    test_uniform_buffer: wgpu::Buffer,
    // Bind groups for windows (keyed by window ID) with uniform buffer and actual texture dimensions
    window_bind_groups: HashMap<u32, (wgpu::BindGroup, wgpu::Buffer, u32, u32)>,
    // Shadow bind groups for windows (keyed by window ID) with uniform buffer
    shadow_bind_groups: HashMap<u32, (wgpu::BindGroup, wgpu::Buffer)>,
    // Intermediate render target for multi-pass rendering (blur support)
    intermediate_texture: Option<IntermediateTarget>,
    // HDR render target and tonemapping (optional)
    hdr_target: Option<HdrRenderTarget>,
    tonemap_pipeline: Option<TonemapPipeline>,
    // HDR-compatible pipelines (created when HDR is enabled)
    hdr_pipeline: Option<CompositePipeline>,
    hdr_shadow_pipeline: Option<ShadowPipeline>,
    // Sampler for tonemapping
    linear_sampler: wgpu::Sampler,
    // Root pixmap (wallpaper) - bind group, uniform buffer, width, height
    root_pixmap_data: Option<(wgpu::BindGroup, wgpu::Buffer, u32, u32)>,
    // Current root pixmap ID
    root_pixmap_id: Option<u32>,
}

impl Renderer {
    /// Create a new renderer for the given overlay window.
    pub async fn new(window: u32, width: u32, height: u32, vsync: super::VSync) -> Result<Self, GpuError> {
        let gpu = GpuContext::new(window, width, height, vsync).await?;

        // Create the composite pipeline
        let pipeline = CompositePipeline::new(&gpu.device, gpu.format());

        // Create shadow pipeline
        let shadow_pipeline = ShadowPipeline::new(&gpu.device, gpu.format());

        // Create blur pipeline
        let blur_pipeline = BlurPipeline::new(&gpu.device, gpu.format());

        // Create texture manager using the same display connection
        let texture_manager = TextureManager::new(gpu.display_ptr())?;

        // Create test uniform buffer
        let test_uniform_buffer = pipeline.create_uniform_buffer(&gpu.device);

        // Create a test texture (checkerboard pattern) to validate rendering
        let test_texture = Self::create_test_texture(&gpu, &pipeline, &test_uniform_buffer);

        // Create linear sampler for tonemapping
        let linear_sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("linear_sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        Ok(Self {
            gpu,
            pipeline,
            shadow_pipeline,
            blur_pipeline,
            shadow_config: ShadowConfig::default(),
            blur_config: BlurConfig::default(),
            hdr_config: HdrConfig::default(),
            texture_manager,
            clear_color: Color {
                r: 0.1,
                g: 0.1,
                b: 0.15,
                a: 1.0,
            },
            test_texture: Some(test_texture),
            test_uniform_buffer,
            window_bind_groups: HashMap::new(),
            shadow_bind_groups: HashMap::new(),
            intermediate_texture: None,
            hdr_target: None,
            tonemap_pipeline: None,
            hdr_pipeline: None,
            hdr_shadow_pipeline: None,
            linear_sampler,
            root_pixmap_data: None,
            root_pixmap_id: None,
        })
    }

    /// Create a test checkerboard texture.
    fn create_test_texture(gpu: &GpuContext, pipeline: &CompositePipeline, uniform_buffer: &wgpu::Buffer) -> WindowRenderData {
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
        let bind_group = pipeline.create_bind_group(&gpu.device, &texture_view, uniform_buffer);

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

    /// Set VSync mode.
    pub fn set_vsync(&mut self, vsync: super::VSync) {
        self.gpu.set_vsync(vsync);
    }

    /// Set the clear color (background).
    pub fn set_clear_color(&mut self, r: f64, g: f64, b: f64, a: f64) {
        self.clear_color = Color { r, g, b, a };
    }

    /// Set the root pixmap (wallpaper background).
    pub fn set_root_pixmap(&mut self, pixmap: Option<u32>) {
        if pixmap == self.root_pixmap_id {
            return; // No change
        }

        self.root_pixmap_id = pixmap;

        if let Some(pix) = pixmap {
            // Get screen dimensions for the root pixmap
            let (width, height) = self.gpu.dimensions();

            // Try to convert the pixmap to a texture
            match self.texture_manager.update_texture(
                &self.gpu.device,
                &self.gpu.queue,
                0xFFFFFFFF, // Special ID for root pixmap
                pix as u64,
                width,
                height,
            ) {
                Ok(tex) => {
                    let uniform_buffer = self.pipeline.create_uniform_buffer(&self.gpu.device);
                    let bind_group = self.pipeline.create_bind_group(&self.gpu.device, &tex.view, &uniform_buffer);
                    self.root_pixmap_data = Some((bind_group, uniform_buffer, tex.width, tex.height));
                    tracing::info!("Root pixmap texture created: {}x{}", tex.width, tex.height);
                }
                Err(e) => {
                    tracing::warn!("Failed to create root pixmap texture: {}", e);
                    self.root_pixmap_data = None;
                }
            }
        } else {
            self.root_pixmap_data = None;
            self.texture_manager.remove_texture(0xFFFFFFFF);
        }
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
                    &self.test_uniform_buffer,
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
        tracing::trace!("render_windows called with {} windows", windows.len());

        // Update textures for all windows that have pixmaps
        for win in windows {
            tracing::trace!(
                "Processing window {:#x}: pixmap={:#x} {}x{} at ({},{})",
                win.id, win.pixmap, win.width, win.height, win.x, win.y
            );
            if win.pixmap != 0 && win.width > 0 && win.height > 0 {
                match self.texture_manager.update_texture(
                    &self.gpu.device,
                    &self.gpu.queue,
                    win.id,
                    win.pixmap,
                    win.width as u32,
                    win.height as u32,
                ) {
                    Ok(tex) => {
                        tracing::trace!("Texture updated for window {:#x} (actual size {}x{})", win.id, tex.width, tex.height);
                        // Create or update bind group for this window with its own uniform buffer
                        let uniform_buffer = self.pipeline.create_uniform_buffer(&self.gpu.device);
                        let bind_group = self.pipeline.create_bind_group(&self.gpu.device, &tex.view, &uniform_buffer);
                        self.window_bind_groups.insert(win.id, (bind_group, uniform_buffer, tex.width, tex.height));

                        // Create shadow bind group for windows that need shadows
                        if win.shadow_enabled && !self.shadow_bind_groups.contains_key(&win.id) {
                            let shadow_buffer = self.shadow_pipeline.create_uniform_buffer(&self.gpu.device);
                            let shadow_bind = self.shadow_pipeline.create_bind_group(&self.gpu.device, &shadow_buffer);
                            self.shadow_bind_groups.insert(win.id, (shadow_bind, shadow_buffer));
                        }
                    }
                    Err(e) => {
                        tracing::warn!("Failed to update texture for window {:#x}: {}", win.id, e);
                    }
                }
            } else {
                tracing::trace!("Skipping window {:#x}: no valid pixmap/size", win.id);
            }
        }

        // Check if HDR rendering is enabled
        if self.is_hdr_enabled() {
            return self.render_windows_hdr(windows);
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

            // Render root pixmap (wallpaper) as background
            if let Some((ref bind_group, ref uniform_buffer, tex_w, tex_h)) = self.root_pixmap_data {
                self.pipeline.update_uniforms(
                    &self.gpu.queue,
                    uniform_buffer,
                    0.0,
                    0.0,
                    tex_w as f32,
                    tex_h as f32,
                    vw as f32,
                    vh as f32,
                    1.0, // Fully opaque
                    0.0, // No corner radius
                );
                self.pipeline.render(&mut render_pass, bind_group);
            }

            // First pass: render shadows for all windows (back to front)
            for win in windows {
                if win.shadow_enabled {
                    if let Some((shadow_bind, shadow_buffer)) = self.shadow_bind_groups.get(&win.id) {
                        // Use actual texture dimensions for shadow sizing
                        let (base_w, base_h) = if let Some((_, _, tex_w, tex_h)) = self.window_bind_groups.get(&win.id) {
                            (*tex_w as f32, *tex_h as f32)
                        } else {
                            (win.width as f32, win.height as f32)
                        };
                        // Apply Lua animation transforms
                        let x = win.x as f32 + win.offset.0;
                        let y = win.y as f32 + win.offset.1;
                        let w = base_w * win.scale.0;
                        let h = base_h * win.scale.1;

                        self.shadow_pipeline.update_uniforms(
                            &self.gpu.queue,
                            shadow_buffer,
                            x,
                            y,
                            w,
                            h,
                            vw as f32,
                            vh as f32,
                            win.corner_radius,
                            &self.shadow_config,
                        );
                        self.shadow_pipeline.render(&mut render_pass, shadow_bind);
                    }
                }
            }

            // Second pass: render windows (back to front)
            for win in windows.iter() {
                if let Some((bind_group, uniform_buffer, tex_w, tex_h)) = self.window_bind_groups.get(&win.id) {
                    // Apply Lua animation transforms: offset position and scale size
                    let x = win.x as f32 + win.offset.0;
                    let y = win.y as f32 + win.offset.1;
                    let w = *tex_w as f32 * win.scale.0;
                    let h = *tex_h as f32 * win.scale.1;

                    tracing::debug!(
                        "Rendering window {:#x} at ({},{}) size {}x{} opacity={} scale=({:.2},{:.2})",
                        win.id, x, y, w, h, win.opacity, win.scale.0, win.scale.1
                    );
                    self.pipeline.update_uniforms(
                        &self.gpu.queue,
                        uniform_buffer,
                        x,
                        y,
                        w,
                        h,
                        vw as f32,
                        vh as f32,
                        win.opacity,
                        win.corner_radius,
                    );

                    self.pipeline.render(&mut render_pass, bind_group);
                } else {
                    tracing::warn!("No bind group for window {:#x}", win.id);
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

            // Render root pixmap (wallpaper) as background
            if let Some((ref bind_group, ref uniform_buffer, tex_w, tex_h)) = self.root_pixmap_data {
                self.pipeline.update_uniforms(
                    &self.gpu.queue,
                    uniform_buffer,
                    0.0,
                    0.0,
                    tex_w as f32,
                    tex_h as f32,
                    vw as f32,
                    vh as f32,
                    1.0,
                    0.0,
                );
                self.pipeline.render(&mut render_pass, bind_group);
            }

            // Render shadows for all windows
            for win in windows {
                if win.shadow_enabled {
                    if let Some((shadow_bind, shadow_buffer)) = self.shadow_bind_groups.get(&win.id) {
                        let (w, h) = if let Some((_, _, tex_w, tex_h)) = self.window_bind_groups.get(&win.id) {
                            (*tex_w as f32, *tex_h as f32)
                        } else {
                            (win.width as f32, win.height as f32)
                        };
                        self.shadow_pipeline.update_uniforms(
                            &self.gpu.queue,
                            shadow_buffer,
                            win.x as f32,
                            win.y as f32,
                            w,
                            h,
                            vw as f32,
                            vh as f32,
                            win.corner_radius,
                            &self.shadow_config,
                        );
                        self.shadow_pipeline.render(&mut render_pass, shadow_bind);
                    }
                }
            }

            // Render windows that don't need blur
            for win in windows {
                if !win.blur_behind {
                    if let Some((bind_group, uniform_buffer, tex_w, tex_h)) = self.window_bind_groups.get(&win.id) {
                        self.pipeline.update_uniforms(
                            &self.gpu.queue,
                            uniform_buffer,
                            win.x as f32,
                            win.y as f32,
                            *tex_w as f32,
                            *tex_h as f32,
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

        // Create uniform buffer for blur background rendering
        let blur_uniform_buffer = self.pipeline.create_uniform_buffer(&self.gpu.device);

        // Create bind group for blurred texture (if available) before render pass
        let blur_bind_group = blurred_view.map(|view| {
            self.pipeline.create_bind_group(&self.gpu.device, view, &blur_uniform_buffer)
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
                    if let Some((shadow_bind, shadow_buffer)) = self.shadow_bind_groups.get(&win.id) {
                        let (w, h) = if let Some((_, _, tex_w, tex_h)) = self.window_bind_groups.get(&win.id) {
                            (*tex_w as f32, *tex_h as f32)
                        } else {
                            (win.width as f32, win.height as f32)
                        };
                        self.shadow_pipeline.update_uniforms(
                            &self.gpu.queue,
                            shadow_buffer,
                            win.x as f32,
                            win.y as f32,
                            w,
                            h,
                            vw as f32,
                            vh as f32,
                            win.corner_radius,
                            &self.shadow_config,
                        );
                        self.shadow_pipeline.render(&mut render_pass, shadow_bind);
                    }
                }
            }

            // Render all windows, using blurred background for blur_behind windows
            for win in windows {
                let (tex_w, tex_h) = if let Some((_, _, tw, th)) = self.window_bind_groups.get(&win.id) {
                    (*tw as f32, *th as f32)
                } else {
                    (win.width as f32, win.height as f32)
                };

                if win.blur_behind {
                    // First draw blurred background in window region
                    if let Some(ref bg) = blur_bind_group {
                        // Draw blurred background in window region
                        self.pipeline.update_uniforms(
                            &self.gpu.queue,
                            &blur_uniform_buffer,
                            win.x as f32,
                            win.y as f32,
                            tex_w,
                            tex_h,
                            vw as f32,
                            vh as f32,
                            1.0, // Opaque blur background
                            win.corner_radius,
                        );
                        self.pipeline.render(&mut render_pass, bg);
                    }

                    // Then draw the transparent window on top
                    if let Some((bind_group, uniform_buffer, _, _)) = self.window_bind_groups.get(&win.id) {
                        self.pipeline.update_uniforms(
                            &self.gpu.queue,
                            uniform_buffer,
                            win.x as f32,
                            win.y as f32,
                            tex_w,
                            tex_h,
                            vw as f32,
                            vh as f32,
                            win.opacity,
                            win.corner_radius,
                        );
                        self.pipeline.render(&mut render_pass, bind_group);
                    }
                } else {
                    // Non-blur window, render normally
                    if let Some((bind_group, uniform_buffer, _, _)) = self.window_bind_groups.get(&win.id) {
                        self.pipeline.update_uniforms(
                            &self.gpu.queue,
                            uniform_buffer,
                            win.x as f32,
                            win.y as f32,
                            tex_w,
                            tex_h,
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
        self.shadow_bind_groups.remove(&window_id);
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

    /// Enable HDR rendering with the given configuration.
    pub fn enable_hdr(&mut self, config: HdrConfig) {
        if !config.enabled {
            self.disable_hdr();
            return;
        }

        let (width, height) = self.gpu.dimensions();
        let hdr_format = wgpu::TextureFormat::Rgba16Float;

        // Create HDR render target
        self.hdr_target = Some(HdrRenderTarget::new(&self.gpu.device, width, height));

        // Create HDR-compatible pipelines (render to Rgba16Float)
        self.hdr_pipeline = Some(CompositePipeline::new(&self.gpu.device, hdr_format));
        self.hdr_shadow_pipeline = Some(ShadowPipeline::new(&self.gpu.device, hdr_format));

        // Create tonemapping pipeline (renders HDR to SDR surface)
        self.tonemap_pipeline = Some(TonemapPipeline::new(
            &self.gpu.device,
            self.gpu.format(),
            &config,
        ));

        self.hdr_config = config;

        tracing::info!(
            "HDR rendering enabled: peak={}nits, paper_white={}nits, operator={:?}",
            self.hdr_config.peak_luminance,
            self.hdr_config.paper_white,
            self.hdr_config.tonemap_operator
        );
    }

    /// Disable HDR rendering.
    pub fn disable_hdr(&mut self) {
        self.hdr_target = None;
        self.tonemap_pipeline = None;
        self.hdr_pipeline = None;
        self.hdr_shadow_pipeline = None;
        self.hdr_config.enabled = false;
        tracing::info!("HDR rendering disabled");
    }

    /// Update HDR configuration.
    pub fn update_hdr_config(&mut self, config: HdrConfig) {
        self.hdr_config = config.clone();
        if let Some(ref pipeline) = self.tonemap_pipeline {
            pipeline.update_uniforms(&self.gpu.queue, &config);
        }
    }

    /// Check if HDR rendering is enabled.
    pub fn is_hdr_enabled(&self) -> bool {
        self.hdr_config.enabled && self.hdr_target.is_some() && self.tonemap_pipeline.is_some()
    }

    /// Update shadow configuration.
    pub fn update_shadow_config(&mut self, config: ShadowConfig) {
        self.shadow_config = config;
    }

    /// Update blur configuration.
    pub fn update_blur_config(&mut self, enabled: bool, strength: u32) {
        self.blur_config.enabled = enabled;
        self.blur_config.strength = strength as f32;
    }

    /// Render windows with HDR pipeline (HDR compositing + tonemapping).
    fn render_windows_hdr(&mut self, windows: &[WindowRenderInfo]) -> Result<(), GpuError> {
        let (vw, vh) = self.gpu.dimensions();

        // Ensure HDR target is correct size
        if let Some(ref mut hdr_target) = self.hdr_target {
            hdr_target.resize(&self.gpu.device, vw, vh);
        } else {
            return self.render_windows_simple(windows);
        }

        // Ensure HDR pipelines exist
        let hdr_pipeline = match &self.hdr_pipeline {
            Some(p) => p,
            None => return self.render_windows_simple(windows),
        };
        let hdr_shadow_pipeline = match &self.hdr_shadow_pipeline {
            Some(p) => p,
            None => return self.render_windows_simple(windows),
        };

        // Create bind groups for HDR pipeline (must use HDR pipeline's bind group layout)
        // Store both bind group and uniform buffer per window
        let mut hdr_bind_groups: HashMap<u32, (wgpu::BindGroup, wgpu::Buffer)> = HashMap::new();
        let mut hdr_shadow_bind_groups: HashMap<u32, (wgpu::BindGroup, wgpu::Buffer)> = HashMap::new();
        for (id, (_, _, _, _)) in &self.window_bind_groups {
            if let Some(tex) = self.texture_manager.get_texture(*id) {
                let uniform_buffer = hdr_pipeline.create_uniform_buffer(&self.gpu.device);
                let bind_group = hdr_pipeline.create_bind_group(&self.gpu.device, &tex.view, &uniform_buffer);
                hdr_bind_groups.insert(*id, (bind_group, uniform_buffer));
            }
            // Create HDR shadow bind group for this window
            let shadow_buffer = hdr_shadow_pipeline.create_uniform_buffer(&self.gpu.device);
            let shadow_bind = hdr_shadow_pipeline.create_bind_group(&self.gpu.device, &shadow_buffer);
            hdr_shadow_bind_groups.insert(*id, (shadow_bind, shadow_buffer));
        }

        // Create HDR bind group for root pixmap if available
        let hdr_root_pixmap = if let Some(tex) = self.texture_manager.get_texture(0xFFFFFFFF) {
            let uniform_buffer = hdr_pipeline.create_uniform_buffer(&self.gpu.device);
            let bind_group = hdr_pipeline.create_bind_group(&self.gpu.device, &tex.view, &uniform_buffer);
            Some((bind_group, uniform_buffer, tex.width, tex.height))
        } else {
            None
        };

        let frame = self.gpu.begin_frame()?;
        let surface_view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self.gpu.create_encoder();

        // Get HDR target view
        let hdr_view = &self.hdr_target.as_ref().unwrap().view;

        // Phase 1: Render to HDR target (Rgba16Float) using HDR pipelines
        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("hdr_composite_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: hdr_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(Color {
                            r: self.clear_color.r,
                            g: self.clear_color.g,
                            b: self.clear_color.b,
                            a: self.clear_color.a,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            // Render root pixmap (wallpaper) as background
            if let Some((ref bind_group, ref uniform_buffer, tex_w, tex_h)) = hdr_root_pixmap {
                hdr_pipeline.update_uniforms(
                    &self.gpu.queue,
                    uniform_buffer,
                    0.0,
                    0.0,
                    tex_w as f32,
                    tex_h as f32,
                    vw as f32,
                    vh as f32,
                    1.0,
                    0.0,
                );
                hdr_pipeline.render(&mut render_pass, bind_group);
            }

            // Render shadows using HDR shadow pipeline
            for win in windows {
                if win.shadow_enabled {
                    if let Some((shadow_bind, shadow_buffer)) = hdr_shadow_bind_groups.get(&win.id) {
                        let (w, h) = if let Some((_, _, tex_w, tex_h)) = self.window_bind_groups.get(&win.id) {
                            (*tex_w as f32, *tex_h as f32)
                        } else {
                            (win.width as f32, win.height as f32)
                        };
                        hdr_shadow_pipeline.update_uniforms(
                            &self.gpu.queue,
                            shadow_buffer,
                            win.x as f32,
                            win.y as f32,
                            w,
                            h,
                            vw as f32,
                            vh as f32,
                            win.corner_radius,
                            &self.shadow_config,
                        );
                        hdr_shadow_pipeline.render(&mut render_pass, shadow_bind);
                    }
                }
            }

            // Render windows using HDR composite pipeline
            for win in windows {
                if let Some((bind_group, uniform_buffer)) = hdr_bind_groups.get(&win.id) {
                    let (tex_w, tex_h) = if let Some((_, _, tw, th)) = self.window_bind_groups.get(&win.id) {
                        (*tw, *th)
                    } else {
                        (win.width as u32, win.height as u32)
                    };
                    hdr_pipeline.update_uniforms(
                        &self.gpu.queue,
                        uniform_buffer,
                        win.x as f32,
                        win.y as f32,
                        tex_w as f32,
                        tex_h as f32,
                        vw as f32,
                        vh as f32,
                        win.opacity,
                        win.corner_radius,
                    );
                    hdr_pipeline.render(&mut render_pass, bind_group);
                }
            }
        }

        // Phase 2: Tonemap HDR to SDR on surface
        if let Some(ref tonemap) = self.tonemap_pipeline {
            let tonemap_bind_group = tonemap.create_bind_group(
                &self.gpu.device,
                hdr_view,
                &self.linear_sampler,
            );

            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("tonemap_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &surface_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            render_pass.set_pipeline(&tonemap.pipeline);
            render_pass.set_bind_group(0, &tonemap_bind_group, &[]);
            render_pass.draw(0..3, 0..1); // Fullscreen triangle
        }

        self.gpu.end_frame(encoder, frame);
        Ok(())
    }
}
