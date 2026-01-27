//! Multi-technique blur implementation.
//! Supports Dual Kawase, Box, and Gaussian blur algorithms.

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use super::pipeline::Vertex;

/// Available blur techniques.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BlurTechnique {
    /// Dual Kawase - efficient multi-pass blur, best for large radii
    #[default]
    DualKawase,
    /// Box blur - simple and fast, can look blocky
    Box,
    /// Gaussian - high quality classic blur
    Gaussian,
}

impl BlurTechnique {
    /// Get the mode bits for the shader.
    fn mode_bits(self) -> u32 {
        match self {
            BlurTechnique::DualKawase => 0,
            BlurTechnique::Box => 1,
            BlurTechnique::Gaussian => 2,
        }
    }
}

/// Uniform data for blur passes.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct BlurUniforms {
    /// Texel size (1.0 / texture_size)
    pub texel_size: [f32; 2],
    /// Blur offset/radius multiplier
    pub offset: f32,
    /// Mode bits: technique (0-1) | upsample (2) | vertical (3)
    pub mode: u32,
}

/// Intermediate texture for blur passes.
pub struct BlurTexture {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub width: u32,
    pub height: u32,
}

impl BlurTexture {
    pub fn new(device: &wgpu::Device, width: u32, height: u32, format: wgpu::TextureFormat) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("blur_texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
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
}

/// Multi-technique blur pipeline.
pub struct BlurPipeline {
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    uniform_buffer: wgpu::Buffer,
    sampler: wgpu::Sampler,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    // Intermediate textures for blur passes
    blur_textures: Vec<BlurTexture>,
    format: wgpu::TextureFormat,
    // Current blur technique
    technique: BlurTechnique,
}

impl BlurPipeline {
    /// Create a new blur pipeline.
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        // Load shader
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("blur_shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/blur.wgsl").into()),
        });

        // Create bind group layout
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("blur_bind_group_layout"),
            entries: &[
                // Uniforms
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // Source texture
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                // Sampler
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        // Create pipeline layout
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("blur_pipeline_layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        // Create pipeline
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("blur_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[Vertex::desc()],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        // Create uniform buffer
        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("blur_uniform_buffer"),
            contents: bytemuck::cast_slice(&[BlurUniforms {
                texel_size: [1.0 / 1920.0, 1.0 / 1080.0],
                offset: 1.0,
                mode: 0,
            }]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        // Create sampler (linear filtering for smooth blur)
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("blur_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        // Fullscreen quad vertices
        let vertices = [
            Vertex {
                position: [0.0, 0.0],
                tex_coords: [0.0, 0.0],
            },
            Vertex {
                position: [1.0, 0.0],
                tex_coords: [1.0, 0.0],
            },
            Vertex {
                position: [0.0, 1.0],
                tex_coords: [0.0, 1.0],
            },
            Vertex {
                position: [1.0, 1.0],
                tex_coords: [1.0, 1.0],
            },
        ];
        let indices: [u16; 6] = [0, 2, 1, 1, 2, 3];

        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("blur_vertex_buffer"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("blur_index_buffer"),
            contents: bytemuck::cast_slice(&indices),
            usage: wgpu::BufferUsages::INDEX,
        });

        Self {
            pipeline,
            bind_group_layout,
            uniform_buffer,
            sampler,
            vertex_buffer,
            index_buffer,
            blur_textures: Vec::new(),
            format,
            technique: BlurTechnique::default(),
        }
    }

    /// Set the blur technique.
    pub fn set_technique(&mut self, technique: BlurTechnique) {
        self.technique = technique;
    }

    /// Get the current blur technique.
    pub fn technique(&self) -> BlurTechnique {
        self.technique
    }

    /// Ensure blur textures are allocated for the given dimensions and iteration count.
    pub fn ensure_textures(&mut self, device: &wgpu::Device, width: u32, height: u32, iterations: u32) {
        // We need iterations*2 textures (downsample + upsample chain)
        // Each downsample halves the resolution
        let needed = iterations as usize;

        // Check if we need to recreate textures
        let needs_recreate = self.blur_textures.is_empty()
            || self.blur_textures.len() < needed
            || self.blur_textures[0].width != width / 2
            || self.blur_textures[0].height != height / 2;

        if needs_recreate {
            self.blur_textures.clear();

            let mut w = width / 2;
            let mut h = height / 2;

            for _ in 0..needed {
                // Minimum size of 1x1
                w = w.max(1);
                h = h.max(1);

                self.blur_textures.push(BlurTexture::new(device, w, h, self.format));

                w /= 2;
                h /= 2;
            }
        }
    }

    /// Create a bind group for a blur pass.
    fn create_bind_group(&self, device: &wgpu::Device, source_view: &wgpu::TextureView) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("blur_bind_group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(source_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        })
    }

    /// Run blur passes on a source texture.
    /// Returns the view of the final blurred texture.
    pub fn blur(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        source_view: &wgpu::TextureView,
        source_width: u32,
        source_height: u32,
        iterations: u32,
        strength: f32,
    ) -> Option<&wgpu::TextureView> {
        if iterations == 0 || strength <= 0.0 {
            return None;
        }

        match self.technique {
            BlurTechnique::DualKawase => {
                self.blur_kawase(device, queue, encoder, source_view, source_width, source_height, iterations)
            }
            BlurTechnique::Box | BlurTechnique::Gaussian => {
                self.blur_separable(device, queue, encoder, source_view, source_width, source_height, iterations, strength)
            }
        }
    }

    /// Dual Kawase blur (downsample/upsample chain).
    fn blur_kawase(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        source_view: &wgpu::TextureView,
        source_width: u32,
        source_height: u32,
        iterations: u32,
    ) -> Option<&wgpu::TextureView> {
        self.ensure_textures(device, source_width, source_height, iterations);

        if self.blur_textures.is_empty() {
            return None;
        }

        let iterations = iterations.min(self.blur_textures.len() as u32);
        let technique_bits = self.technique.mode_bits();

        // Downsample passes
        let mut current_source = source_view;
        let mut current_width = source_width;
        let mut current_height = source_height;

        for i in 0..iterations as usize {
            let target = &self.blur_textures[i];

            let uniforms = BlurUniforms {
                texel_size: [1.0 / current_width as f32, 1.0 / current_height as f32],
                offset: 1.0,
                mode: technique_bits, // Downsample (bit 2 = 0)
            };
            queue.write_buffer(&self.uniform_buffer, 0, bytemuck::cast_slice(&[uniforms]));

            let bind_group = self.create_bind_group(device, current_source);

            {
                let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("blur_downsample_pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &target.view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });

                render_pass.set_pipeline(&self.pipeline);
                render_pass.set_bind_group(0, &bind_group, &[]);
                render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                render_pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
                render_pass.draw_indexed(0..6, 0, 0..1);
            }

            current_source = &self.blur_textures[i].view;
            current_width = target.width;
            current_height = target.height;
        }

        // Upsample passes (reverse order)
        for i in (1..iterations as usize).rev() {
            let source_tex = &self.blur_textures[i];
            let target = &self.blur_textures[i - 1];

            let uniforms = BlurUniforms {
                texel_size: [1.0 / source_tex.width as f32, 1.0 / source_tex.height as f32],
                offset: 0.5,
                mode: technique_bits | 4, // Upsample (bit 2 = 1)
            };
            queue.write_buffer(&self.uniform_buffer, 0, bytemuck::cast_slice(&[uniforms]));

            let bind_group = self.create_bind_group(device, &source_tex.view);

            {
                let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("blur_upsample_pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &target.view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });

                render_pass.set_pipeline(&self.pipeline);
                render_pass.set_bind_group(0, &bind_group, &[]);
                render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                render_pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
                render_pass.draw_indexed(0..6, 0, 0..1);
            }
        }

        Some(&self.blur_textures[0].view)
    }

    /// Separable blur (box or gaussian) - horizontal then vertical pass.
    fn blur_separable(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        source_view: &wgpu::TextureView,
        source_width: u32,
        source_height: u32,
        iterations: u32,
        strength: f32,
    ) -> Option<&wgpu::TextureView> {
        // For separable blur, we need 2 textures at full resolution for ping-pong
        self.ensure_textures(device, source_width * 2, source_height * 2, 2);

        if self.blur_textures.len() < 2 {
            return None;
        }

        let technique_bits = self.technique.mode_bits();

        // Initial horizontal pass from source
        let uniforms = BlurUniforms {
            texel_size: [1.0 / source_width as f32, 1.0 / source_height as f32],
            offset: strength,
            mode: technique_bits, // Horizontal (bit 3 = 0)
        };
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::cast_slice(&[uniforms]));

        let bind_group = self.create_bind_group(device, source_view);
        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("blur_horizontal_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.blur_textures[0].view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            render_pass.set_pipeline(&self.pipeline);
            render_pass.set_bind_group(0, &bind_group, &[]);
            render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
            render_pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
            render_pass.draw_indexed(0..6, 0, 0..1);
        }

        // Subsequent passes (ping-pong between textures)
        for i in 0..iterations {
            let is_vertical = i % 2 == 0; // First pass after initial is vertical
            let source_idx = i as usize % 2;
            let target_idx = (i as usize + 1) % 2;

            let uniforms = BlurUniforms {
                texel_size: [1.0 / source_width as f32, 1.0 / source_height as f32],
                offset: strength,
                mode: technique_bits | if is_vertical { 8 } else { 0 },
            };
            queue.write_buffer(&self.uniform_buffer, 0, bytemuck::cast_slice(&[uniforms]));

            let bind_group = self.create_bind_group(device, &self.blur_textures[source_idx].view);
            {
                let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("blur_separable_pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &self.blur_textures[target_idx].view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });

                render_pass.set_pipeline(&self.pipeline);
                render_pass.set_bind_group(0, &bind_group, &[]);
                render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                render_pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
                render_pass.draw_indexed(0..6, 0, 0..1);
            }
        }

        // Return the last written texture
        let final_idx = iterations as usize % 2;
        Some(&self.blur_textures[final_idx].view)
    }
}
