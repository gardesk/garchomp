//! HDR rendering support for garchomp compositor.
//!
//! Provides high dynamic range compositing with:
//! - Rgba16Float intermediate render targets
//! - ACES and Reinhard tonemapping
//! - SDR to HDR inverse tonemapping
//! - Colorspace handling (sRGB, scRGB-linear, BT.2020)

use wgpu::util::DeviceExt;

/// Colorspace values for _GARCHOMP_COLORSPACE atom.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u32)]
pub enum Colorspace {
    /// Standard sRGB (SDR content).
    #[default]
    Srgb = 0,
    /// scRGB linear (extended range, HDR).
    ScrgbLinear = 1,
    /// BT.2020 with PQ transfer (HDR10).
    Bt2020Pq = 2,
}

impl From<u32> for Colorspace {
    fn from(value: u32) -> Self {
        match value {
            1 => Self::ScrgbLinear,
            2 => Self::Bt2020Pq,
            _ => Self::Srgb,
        }
    }
}

/// Tonemapping operator selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TonemapOperator {
    /// No tonemapping (passthrough).
    None,
    /// Reinhard simple tonemapping.
    Reinhard,
    /// ACES filmic tonemapping (default, cinematic look).
    #[default]
    Aces,
    /// Uncharted 2 / Hable tonemapping.
    Hable,
}

impl TonemapOperator {
    pub fn from_name(name: &str) -> Self {
        match name.to_lowercase().as_str() {
            "none" | "passthrough" => Self::None,
            "reinhard" => Self::Reinhard,
            "aces" | "filmic" => Self::Aces,
            "hable" | "uncharted2" => Self::Hable,
            _ => Self::default(),
        }
    }
}

/// HDR configuration for the compositor.
#[derive(Debug, Clone)]
pub struct HdrConfig {
    /// Enable HDR compositing.
    pub enabled: bool,
    /// Output peak luminance in nits (for tonemapping).
    pub peak_luminance: f32,
    /// Paper white luminance for SDR content.
    pub paper_white: f32,
    /// Tonemapping operator to use.
    pub tonemap_operator: TonemapOperator,
    /// Whether the display supports HDR.
    pub display_hdr_capable: bool,
}

impl Default for HdrConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            peak_luminance: 1000.0, // Standard HDR1000 display
            paper_white: 203.0,     // ITU-R BT.2408 reference
            tonemap_operator: TonemapOperator::Aces,
            display_hdr_capable: false,
        }
    }
}

/// HDR render target for intermediate compositing.
pub struct HdrRenderTarget {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub format: wgpu::TextureFormat,
    pub width: u32,
    pub height: u32,
}

impl HdrRenderTarget {
    /// Create a new HDR render target with Rgba16Float format.
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let format = wgpu::TextureFormat::Rgba16Float;

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("hdr_render_target"),
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
            format,
            width,
            height,
        }
    }

    /// Resize the render target.
    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        if width != self.width || height != self.height {
            *self = Self::new(device, width, height);
        }
    }
}

/// Uniform buffer for tonemapping shader.
#[repr(C)]
#[derive(Debug, Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct TonemapUniforms {
    /// Peak luminance in nits.
    pub peak_luminance: f32,
    /// Paper white luminance in nits.
    pub paper_white: f32,
    /// Tonemapping operator (0=none, 1=reinhard, 2=aces, 3=hable).
    pub operator: u32,
    /// Padding for alignment.
    pub _padding: u32,
}

/// Tonemapping pipeline for HDR to SDR conversion.
pub struct TonemapPipeline {
    pub pipeline: wgpu::RenderPipeline,
    pub bind_group_layout: wgpu::BindGroupLayout,
    pub uniform_buffer: wgpu::Buffer,
}

impl TonemapPipeline {
    /// Create the tonemapping pipeline.
    pub fn new(
        device: &wgpu::Device,
        output_format: wgpu::TextureFormat,
        config: &HdrConfig,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("tonemap_shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("tonemap.wgsl").into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("tonemap_bind_group_layout"),
            entries: &[
                // HDR texture input
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
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
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                // Uniform buffer
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("tonemap_pipeline_layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("tonemap_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: output_format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let uniforms = TonemapUniforms {
            peak_luminance: config.peak_luminance,
            paper_white: config.paper_white,
            operator: config.tonemap_operator as u32,
            _padding: 0,
        };

        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("tonemap_uniform_buffer"),
            contents: bytemuck::cast_slice(&[uniforms]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        Self {
            pipeline,
            bind_group_layout,
            uniform_buffer,
        }
    }

    /// Create a bind group for the tonemapping pass.
    pub fn create_bind_group(
        &self,
        device: &wgpu::Device,
        hdr_view: &wgpu::TextureView,
        sampler: &wgpu::Sampler,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("tonemap_bind_group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(hdr_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.uniform_buffer.as_entire_binding(),
                },
            ],
        })
    }

    /// Update the tonemapping uniforms.
    pub fn update_uniforms(&self, queue: &wgpu::Queue, config: &HdrConfig) {
        let uniforms = TonemapUniforms {
            peak_luminance: config.peak_luminance,
            paper_white: config.paper_white,
            operator: config.tonemap_operator as u32,
            _padding: 0,
        };
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::cast_slice(&[uniforms]));
    }
}

/// Check if a wgpu texture format supports HDR.
pub fn is_hdr_format(format: wgpu::TextureFormat) -> bool {
    matches!(
        format,
        wgpu::TextureFormat::Rgba16Float
            | wgpu::TextureFormat::Rgb10a2Unorm
            | wgpu::TextureFormat::Rgba32Float
            | wgpu::TextureFormat::Rg11b10Ufloat
    )
}

/// Get the best HDR format supported by the surface.
pub fn select_hdr_format(surface_caps: &wgpu::SurfaceCapabilities) -> Option<wgpu::TextureFormat> {
    // Priority: Rgba16Float > Rgb10a2Unorm > Rg11b10Ufloat
    for format in &[
        wgpu::TextureFormat::Rgba16Float,
        wgpu::TextureFormat::Rgb10a2Unorm,
        wgpu::TextureFormat::Rg11b10Ufloat,
    ] {
        if surface_caps.formats.contains(format) {
            return Some(*format);
        }
    }
    None
}
