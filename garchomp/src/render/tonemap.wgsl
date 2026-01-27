// Tonemapping shader for HDR to SDR conversion.
//
// Supports multiple tonemapping operators:
// - 0: None (passthrough)
// - 1: Reinhard
// - 2: ACES filmic
// - 3: Hable (Uncharted 2)

struct TonemapUniforms {
    peak_luminance: f32,
    paper_white: f32,
    tonemap_op: u32,
    _padding: u32,
}

@group(0) @binding(0) var hdr_texture: texture_2d<f32>;
@group(0) @binding(1) var hdr_sampler: sampler;
@group(0) @binding(2) var<uniform> uniforms: TonemapUniforms;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

// Fullscreen triangle vertex shader
@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    var out: VertexOutput;

    // Generate fullscreen triangle
    let x = f32(i32(vertex_index & 1u) * 4 - 1);
    let y = f32(i32(vertex_index >> 1u) * 4 - 1);

    out.position = vec4<f32>(x, y, 0.0, 1.0);
    out.uv = vec2<f32>((x + 1.0) * 0.5, (1.0 - y) * 0.5);

    return out;
}

// sRGB to linear conversion
fn srgb_to_linear(srgb: vec3<f32>) -> vec3<f32> {
    let cutoff = srgb <= vec3<f32>(0.04045);
    let linear_low = srgb / 12.92;
    let linear_high = pow((srgb + 0.055) / 1.055, vec3<f32>(2.4));
    return select(linear_high, linear_low, cutoff);
}

// Linear to sRGB conversion
fn linear_to_srgb(linear: vec3<f32>) -> vec3<f32> {
    let cutoff = linear <= vec3<f32>(0.0031308);
    let srgb_low = linear * 12.92;
    let srgb_high = 1.055 * pow(linear, vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(srgb_high, srgb_low, cutoff);
}

// Reinhard simple tonemapping
fn tonemap_reinhard(hdr: vec3<f32>) -> vec3<f32> {
    return hdr / (hdr + vec3<f32>(1.0));
}

// Reinhard extended tonemapping with white point
fn tonemap_reinhard_extended(hdr: vec3<f32>, white_point: f32) -> vec3<f32> {
    let numerator = hdr * (1.0 + hdr / (white_point * white_point));
    return numerator / (1.0 + hdr);
}

// ACES filmic tonemapping
// Based on the fitted curve from Krzysztof Narkowicz
fn tonemap_aces(hdr: vec3<f32>) -> vec3<f32> {
    let a = 2.51;
    let b = 0.03;
    let c = 2.43;
    let d = 0.59;
    let e = 0.14;

    let x = hdr * 0.6; // Exposure bias
    return clamp((x * (a * x + b)) / (x * (c * x + d) + e), vec3<f32>(0.0), vec3<f32>(1.0));
}

// Hable (Uncharted 2) tonemapping curve
fn hable_curve(x: vec3<f32>) -> vec3<f32> {
    let A = 0.15; // Shoulder strength
    let B = 0.50; // Linear strength
    let C = 0.10; // Linear angle
    let D = 0.20; // Toe strength
    let E = 0.02; // Toe numerator
    let F = 0.30; // Toe denominator

    return ((x * (A * x + C * B) + D * E) / (x * (A * x + B) + D * F)) - E / F;
}

fn tonemap_hable(hdr: vec3<f32>) -> vec3<f32> {
    let white_point = 11.2;
    let exposure_bias = 2.0;

    let curr = hable_curve(hdr * exposure_bias);
    let white_scale = vec3<f32>(1.0) / hable_curve(vec3<f32>(white_point));

    return curr * white_scale;
}

// Luminance calculation (Rec. 709)
fn luminance(color: vec3<f32>) -> f32 {
    return dot(color, vec3<f32>(0.2126, 0.7152, 0.0722));
}

// Inverse tonemapping for SDR to HDR expansion
fn inverse_tonemap_reinhard(sdr: vec3<f32>) -> vec3<f32> {
    // Clamp to avoid division by zero
    let clamped = clamp(sdr, vec3<f32>(0.0), vec3<f32>(0.99));
    return clamped / (vec3<f32>(1.0) - clamped);
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let hdr_color = textureSample(hdr_texture, hdr_sampler, in.uv);

    // Extract RGB and alpha
    var color = hdr_color.rgb;
    let alpha = hdr_color.a;

    // Detect if content is SDR (all values in 0-1 range) or HDR (values > 1.0)
    let max_val = max(max(color.r, color.g), color.b);
    let is_hdr = max_val > 1.0;

    // For SDR content: ALWAYS pass through unchanged (no darkening, no color shift)
    // For HDR content: apply tonemapping to compress into displayable range
    if !is_hdr {
        // SDR passthrough - output exactly what we received
        return vec4<f32>(color, alpha);
    }

    // HDR content - apply tonemapping based on selected operator
    // First normalize HDR values relative to display capabilities
    let normalized = color * (uniforms.paper_white / uniforms.peak_luminance);

    var tonemapped: vec3<f32>;
    switch uniforms.tonemap_op {
        case 0u: {
            // None - just clamp HDR to SDR range
            tonemapped = clamp(normalized, vec3<f32>(0.0), vec3<f32>(1.0));
        }
        case 1u: {
            // Reinhard
            tonemapped = tonemap_reinhard(normalized);
        }
        case 2u: {
            // ACES filmic
            tonemapped = tonemap_aces(normalized);
        }
        case 3u: {
            // Hable (Uncharted 2)
            tonemapped = tonemap_hable(normalized);
        }
        default: {
            tonemapped = tonemap_aces(normalized);
        }
    }

    return vec4<f32>(tonemapped, alpha);
}

// Fragment shader for SDR to HDR expansion (inverse tonemapping)
@fragment
fn fs_inverse_tonemap(in: VertexOutput) -> @location(0) vec4<f32> {
    let sdr_color = textureSample(hdr_texture, hdr_sampler, in.uv);

    // Convert from sRGB to linear
    let linear = srgb_to_linear(sdr_color.rgb);

    // Apply inverse tonemapping to expand to HDR range
    let hdr = inverse_tonemap_reinhard(linear);

    // Scale by paper white to get nits
    let scaled = hdr * (uniforms.paper_white / 80.0); // 80 nits = SDR reference

    return vec4<f32>(scaled, sdr_color.a);
}
