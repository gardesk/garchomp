// garchomp composite shader
// Renders a textured quad with position, opacity, rounded corners, and basic transforms

struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) tex_coords: vec2<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) tex_coords: vec2<f32>,
    @location(1) local_pos: vec2<f32>,  // Position within the window (0-1)
}

struct Uniforms {
    // Transform: position (xy) and scale (zw)
    transform: vec4<f32>,
    // Viewport size for NDC conversion
    viewport: vec2<f32>,
    // Opacity
    opacity: f32,
    // Corner radius in pixels
    corner_radius: f32,
    // Window size in pixels (for corner calculation)
    window_size: vec2<f32>,
    // UV offset and scale for screen-space sampling (blur background)
    // Default: offset=(0,0), scale=(1,1) for normal texture sampling
    // For blur: offset=(win_x/vw, win_y/vh), scale=(win_w/vw, win_h/vh)
    uv_offset: vec2<f32>,
    uv_scale: vec2<f32>,
}

@group(0) @binding(0)
var<uniform> uniforms: Uniforms;

@group(0) @binding(1)
var t_diffuse: texture_2d<f32>;

@group(0) @binding(2)
var s_diffuse: sampler;

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;

    // Apply scale and position
    let scaled = in.position * uniforms.transform.zw;
    let positioned = scaled + uniforms.transform.xy;

    // Convert to NDC (-1 to 1)
    // Screen coords: (0,0) top-left, (width, height) bottom-right
    // NDC: (-1,-1) bottom-left, (1,1) top-right
    let ndc_x = (positioned.x / uniforms.viewport.x) * 2.0 - 1.0;
    let ndc_y = 1.0 - (positioned.y / uniforms.viewport.y) * 2.0;

    out.clip_position = vec4<f32>(ndc_x, ndc_y, 0.0, 1.0);
    out.tex_coords = in.tex_coords;
    out.local_pos = in.position;  // 0-1 position within window

    return out;
}

// Signed distance function for a rounded rectangle
// Returns negative inside, positive outside
fn sdf_rounded_rect(p: vec2<f32>, size: vec2<f32>, radius: f32) -> f32 {
    // Handle the case where radius is larger than half the smallest dimension
    let r = min(radius, min(size.x, size.y) * 0.5);

    // Move to corner-relative space
    let q = abs(p) - size + vec2<f32>(r, r);

    // Distance to rounded corner
    return length(max(q, vec2<f32>(0.0, 0.0))) + min(max(q.x, q.y), 0.0) - r;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Apply UV transform for screen-space sampling (blur backgrounds)
    let transformed_uv = in.tex_coords * uniforms.uv_scale + uniforms.uv_offset;
    let color = textureSample(t_diffuse, s_diffuse, transformed_uv);

    // Use texture alpha, defaulting to opaque for depth-24 windows
    var base_alpha = color.a;
    if base_alpha < 0.01 {
        base_alpha = 1.0;
    }

    var alpha = base_alpha * uniforms.opacity;

    if uniforms.corner_radius > 0.0 {
        let pixel_pos = (in.local_pos - 0.5) * uniforms.window_size;
        let half_size = uniforms.window_size * 0.5;
        let dist = sdf_rounded_rect(pixel_pos, half_size, uniforms.corner_radius);
        alpha *= 1.0 - smoothstep(-1.0, 1.0, dist);
    }

    return vec4<f32>(color.rgb, alpha);

    // Normal rendering code (disabled for debugging):
    /*
    var base_alpha = color.a;
    if base_alpha < 0.01 {
        base_alpha = 1.0;
    }

    var alpha = base_alpha * uniforms.opacity;

    if uniforms.corner_radius > 0.0 {
        let pixel_pos = (in.local_pos - 0.5) * uniforms.window_size;
        let half_size = uniforms.window_size * 0.5;
        let dist = sdf_rounded_rect(pixel_pos, half_size, uniforms.corner_radius);
        alpha *= 1.0 - smoothstep(-1.0, 1.0, dist);
    }

    return vec4<f32>(color.rgb, alpha);
    */
}
