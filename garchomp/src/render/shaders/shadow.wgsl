// garchomp shadow shader
// Renders soft shadows behind windows with rounded corner support

struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) tex_coords: vec2<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) local_pos: vec2<f32>,  // Position within shadow quad (0-1)
}

struct ShadowUniforms {
    // Transform: position (xy) and scale (zw) - includes spread
    transform: vec4<f32>,
    // Viewport size
    viewport: vec2<f32>,
    // Shadow color (RGB)
    color: vec3<f32>,
    // Shadow opacity
    opacity: f32,
    // Window size (without shadow spread)
    window_size: vec2<f32>,
    // Shadow spread (how much larger the shadow is than the window)
    spread: f32,
    // Shadow blur radius (controls edge softness)
    blur_radius: f32,
    // Corner radius
    corner_radius: f32,
    // Shadow offset from window
    offset: vec2<f32>,
    // Padding for alignment
    _padding: f32,
}

@group(0) @binding(0)
var<uniform> uniforms: ShadowUniforms;

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;

    // Apply scale and position (shadow quad is larger than window)
    let scaled = in.position * uniforms.transform.zw;
    let positioned = scaled + uniforms.transform.xy;

    // Convert to NDC
    let ndc_x = (positioned.x / uniforms.viewport.x) * 2.0 - 1.0;
    let ndc_y = 1.0 - (positioned.y / uniforms.viewport.y) * 2.0;

    out.clip_position = vec4<f32>(ndc_x, ndc_y, 0.0, 1.0);
    out.local_pos = in.position;

    return out;
}

// Signed distance function for a rounded rectangle
fn sdf_rounded_rect(p: vec2<f32>, size: vec2<f32>, radius: f32) -> f32 {
    let r = min(radius, min(size.x, size.y) * 0.5);
    let q = abs(p) - size + vec2<f32>(r, r);
    return length(max(q, vec2<f32>(0.0, 0.0))) + min(max(q.x, q.y), 0.0) - r;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Total shadow quad size (window + 2*spread)
    let shadow_size = uniforms.window_size + vec2<f32>(uniforms.spread * 2.0);

    // Convert local position to shadow-centered coordinates
    // Account for offset
    let shadow_center = vec2<f32>(0.5) - uniforms.offset / shadow_size;
    let pixel_pos = (in.local_pos - shadow_center) * shadow_size;

    // Calculate distance to window shape (with rounded corners)
    let half_window = uniforms.window_size * 0.5;
    let dist = sdf_rounded_rect(pixel_pos, half_window, uniforms.corner_radius);

    // Softer shadow falloff using smoothstep for gradual transition
    // blur_radius controls the fade distance
    let blur = max(uniforms.blur_radius, 1.0);

    // Only render shadow outside the window
    if dist < 0.0 {
        return vec4<f32>(0.0);
    }

    // INVARIANT: Fade must complete within spread distance to avoid hard edges.
    // The shadow quad extends `spread` pixels beyond the window on each side.
    // If fade_distance > spread, the shadow would be cut off abruptly at the quad edge.
    // Using spread directly ensures a smooth fade to zero at the quad boundary.
    let fade_distance = uniforms.spread;

    // Normalize distance to 0-1 range within the spread
    let t = clamp(dist / fade_distance, 0.0, 1.0);

    // Apply a soft falloff curve (quintic smoothstep for very smooth fade)
    let t2 = t * t;
    let t3 = t2 * t;
    let shadow_alpha = 1.0 - (6.0 * t3 * t2 - 15.0 * t2 * t2 + 10.0 * t3);

    // Apply shadow opacity
    let final_alpha = shadow_alpha * uniforms.opacity;

    return vec4<f32>(uniforms.color, final_alpha);
}
