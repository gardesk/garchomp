// garchomp composite shader
// Renders a textured quad with position, opacity, and basic transforms

struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) tex_coords: vec2<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) tex_coords: vec2<f32>,
}

struct Uniforms {
    // Transform: position (xy) and scale (zw)
    transform: vec4<f32>,
    // Viewport size for NDC conversion
    viewport: vec2<f32>,
    // Opacity
    opacity: f32,
    // Padding for alignment
    _padding: f32,
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

    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let color = textureSample(t_diffuse, s_diffuse, in.tex_coords);
    return vec4<f32>(color.rgb, color.a * uniforms.opacity);
}
