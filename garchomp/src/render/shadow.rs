//! Shadow rendering pipeline.

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use super::pipeline::Vertex;

/// Shadow configuration.
#[derive(Debug, Clone, Copy)]
pub struct ShadowConfig {
    /// Shadow color (RGB, 0-1)
    pub color: [f32; 3],
    /// Shadow opacity (0-1)
    pub opacity: f32,
    /// Shadow spread (pixels added around window)
    pub spread: f32,
    /// Shadow blur radius (controls edge softness)
    pub blur_radius: f32,
    /// Shadow offset from window center (x, y pixels)
    pub offset: [f32; 2],
}

impl Default for ShadowConfig {
    fn default() -> Self {
        Self {
            color: [0.0, 0.0, 0.0], // Black
            opacity: 0.5,
            spread: 15.0,
            blur_radius: 12.0,
            offset: [0.0, 5.0], // Slight downward offset (light from above)
        }
    }
}

/// Uniform data for shadow rendering.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct ShadowUniforms {
    /// Transform: position (xy) and scale (zw)
    pub transform: [f32; 4],
    /// Viewport size
    pub viewport: [f32; 2],
    /// Shadow color
    pub color: [f32; 3],
    /// Shadow opacity
    pub opacity: f32,
    /// Window size
    pub window_size: [f32; 2],
    /// Shadow spread
    pub spread: f32,
    /// Shadow blur radius
    pub blur_radius: f32,
    /// Corner radius
    pub corner_radius: f32,
    /// Shadow offset
    pub offset: [f32; 2],
    /// Padding
    pub _padding: f32,
}

/// Shadow rendering pipeline.
pub struct ShadowPipeline {
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
}

impl ShadowPipeline {
    /// Create a new shadow pipeline.
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        // Load shader
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("shadow_shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/shadow.wgsl").into()),
        });

        // Create bind group layout
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("shadow_bind_group_layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        // Create pipeline layout
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("shadow_pipeline_layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        // Create pipeline with alpha blending
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("shadow_pipeline"),
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
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
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
            label: Some("shadow_uniform_buffer"),
            contents: bytemuck::cast_slice(&[ShadowUniforms {
                transform: [0.0, 0.0, 100.0, 100.0],
                viewport: [1920.0, 1080.0],
                color: [0.0, 0.0, 0.0],
                opacity: 0.5,
                window_size: [100.0, 100.0],
                spread: 15.0,
                blur_radius: 12.0,
                corner_radius: 0.0,
                offset: [0.0, 5.0],
                _padding: 0.0,
            }]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        // Create bind group
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("shadow_bind_group"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        // Create vertex buffer (unit quad)
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
            label: Some("shadow_vertex_buffer"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("shadow_index_buffer"),
            contents: bytemuck::cast_slice(&indices),
            usage: wgpu::BufferUsages::INDEX,
        });

        Self {
            pipeline,
            bind_group_layout,
            uniform_buffer,
            bind_group,
            vertex_buffer,
            index_buffer,
        }
    }

    /// Update uniforms for a window shadow.
    pub fn update_uniforms(
        &self,
        queue: &wgpu::Queue,
        window_x: f32,
        window_y: f32,
        window_width: f32,
        window_height: f32,
        viewport_width: f32,
        viewport_height: f32,
        corner_radius: f32,
        config: &ShadowConfig,
    ) {
        // Shadow quad is larger than window by 2*spread on each side
        let shadow_width = window_width + config.spread * 2.0;
        let shadow_height = window_height + config.spread * 2.0;

        // Position shadow quad (offset from window position, accounting for spread)
        let shadow_x = window_x - config.spread + config.offset[0];
        let shadow_y = window_y - config.spread + config.offset[1];

        let uniforms = ShadowUniforms {
            transform: [shadow_x, shadow_y, shadow_width, shadow_height],
            viewport: [viewport_width, viewport_height],
            color: config.color,
            opacity: config.opacity,
            window_size: [window_width, window_height],
            spread: config.spread,
            blur_radius: config.blur_radius,
            corner_radius,
            offset: [
                config.offset[0] / shadow_width,
                config.offset[1] / shadow_height,
            ],
            _padding: 0.0,
        };

        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::cast_slice(&[uniforms]));
    }

    /// Render a shadow.
    pub fn render<'a>(&'a self, render_pass: &mut wgpu::RenderPass<'a>) {
        render_pass.set_pipeline(&self.pipeline);
        render_pass.set_bind_group(0, &self.bind_group, &[]);
        render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        render_pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
        render_pass.draw_indexed(0..6, 0, 0..1);
    }
}
