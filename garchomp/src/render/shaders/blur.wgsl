// garchomp blur shader
// Supports multiple blur techniques: dual kawase, box, and gaussian

struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) tex_coords: vec2<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) tex_coords: vec2<f32>,
}

struct BlurUniforms {
    // Texel size (1.0 / texture_size)
    texel_size: vec2<f32>,
    // Blur offset/radius multiplier
    offset: f32,
    // Blur technique + pass type packed:
    // Low 2 bits: technique (0=kawase, 1=box, 2=gaussian)
    // Bit 2: pass type for kawase (0=down, 1=up)
    // Bit 3: direction for separable (0=horizontal, 1=vertical)
    mode: u32,
}

const TECHNIQUE_KAWASE: u32 = 0u;
const TECHNIQUE_BOX: u32 = 1u;
const TECHNIQUE_GAUSSIAN: u32 = 2u;

@group(0) @binding(0)
var<uniform> uniforms: BlurUniforms;

@group(0) @binding(1)
var t_source: texture_2d<f32>;

@group(0) @binding(2)
var s_source: sampler;

// Simple passthrough vertex shader for fullscreen quad
@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;

    // Convert from 0-1 to NDC (-1 to 1)
    out.clip_position = vec4<f32>(
        in.position.x * 2.0 - 1.0,
        1.0 - in.position.y * 2.0,
        0.0,
        1.0
    );
    out.tex_coords = in.tex_coords;

    return out;
}

// ============================================
// Dual Kawase Blur
// Efficient multi-pass blur, good for large radii
// ============================================

fn kawase_downsample(uv: vec2<f32>) -> vec4<f32> {
    let offset = uniforms.texel_size * uniforms.offset;

    // Center sample (weight 4)
    var color = textureSample(t_source, s_source, uv) * 4.0;

    // Four corner samples (weight 1 each)
    color += textureSample(t_source, s_source, uv + vec2<f32>(-offset.x, -offset.y));
    color += textureSample(t_source, s_source, uv + vec2<f32>(offset.x, -offset.y));
    color += textureSample(t_source, s_source, uv + vec2<f32>(-offset.x, offset.y));
    color += textureSample(t_source, s_source, uv + vec2<f32>(offset.x, offset.y));

    return color / 8.0;
}

fn kawase_upsample(uv: vec2<f32>) -> vec4<f32> {
    let offset = uniforms.texel_size * uniforms.offset;
    let half_offset = offset * 0.5;

    var color = vec4<f32>(0.0);

    // Diagonal samples (weight 1 each)
    color += textureSample(t_source, s_source, uv + vec2<f32>(-offset.x, -offset.y));
    color += textureSample(t_source, s_source, uv + vec2<f32>(offset.x, -offset.y));
    color += textureSample(t_source, s_source, uv + vec2<f32>(-offset.x, offset.y));
    color += textureSample(t_source, s_source, uv + vec2<f32>(offset.x, offset.y));

    // Cross samples (weight 2 each)
    color += textureSample(t_source, s_source, uv + vec2<f32>(0.0, -half_offset.y)) * 2.0;
    color += textureSample(t_source, s_source, uv + vec2<f32>(0.0, half_offset.y)) * 2.0;
    color += textureSample(t_source, s_source, uv + vec2<f32>(-half_offset.x, 0.0)) * 2.0;
    color += textureSample(t_source, s_source, uv + vec2<f32>(half_offset.x, 0.0)) * 2.0;

    return color / 12.0;
}

// ============================================
// Box Blur
// Simple, fast, but can look blocky
// ============================================

fn box_blur(uv: vec2<f32>, horizontal: bool) -> vec4<f32> {
    let radius = i32(uniforms.offset);
    var color = vec4<f32>(0.0);
    var count = 0.0;

    for (var i = -radius; i <= radius; i++) {
        let offset = select(
            vec2<f32>(0.0, f32(i) * uniforms.texel_size.y),
            vec2<f32>(f32(i) * uniforms.texel_size.x, 0.0),
            horizontal
        );
        color += textureSample(t_source, s_source, uv + offset);
        count += 1.0;
    }

    return color / count;
}

// ============================================
// Gaussian Blur (9-tap separable)
// High quality, classic blur look
// ============================================

// Gaussian kernel weights for 9-tap filter (sigma ~2.0)
const GAUSSIAN_WEIGHTS: array<f32, 5> = array<f32, 5>(
    0.227027,  // center
    0.1945946, // 1 away
    0.1216216, // 2 away
    0.054054,  // 3 away
    0.016216   // 4 away
);

fn gaussian_blur(uv: vec2<f32>, horizontal: bool) -> vec4<f32> {
    let dir = select(
        vec2<f32>(0.0, uniforms.texel_size.y),
        vec2<f32>(uniforms.texel_size.x, 0.0),
        horizontal
    ) * uniforms.offset;

    // Center sample
    var color = textureSample(t_source, s_source, uv) * GAUSSIAN_WEIGHTS[0];

    // Symmetric samples
    for (var i = 1; i < 5; i++) {
        let offset = dir * f32(i);
        color += textureSample(t_source, s_source, uv + offset) * GAUSSIAN_WEIGHTS[i];
        color += textureSample(t_source, s_source, uv - offset) * GAUSSIAN_WEIGHTS[i];
    }

    return color;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let technique = uniforms.mode & 3u;
    let is_upsample = (uniforms.mode & 4u) != 0u;
    let is_vertical = (uniforms.mode & 8u) != 0u;

    switch technique {
        case TECHNIQUE_KAWASE: {
            if is_upsample {
                return kawase_upsample(in.tex_coords);
            } else {
                return kawase_downsample(in.tex_coords);
            }
        }
        case TECHNIQUE_BOX: {
            return box_blur(in.tex_coords, !is_vertical);
        }
        case TECHNIQUE_GAUSSIAN: {
            return gaussian_blur(in.tex_coords, !is_vertical);
        }
        default: {
            return textureSample(t_source, s_source, in.tex_coords);
        }
    }
}
