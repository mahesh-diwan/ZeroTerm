struct Uniforms {
    screen_size: vec2<f32>,
    cell_size: vec2<f32>,
    viewport_origin: vec2<f32>,
    cols: u32,
    rows: u32,
}

@group(0) @binding(0)
var<uniform> uniforms: Uniforms;

@group(0) @binding(1)
var<storage, read> cell_data: array<CellData>;

@group(1) @binding(0)
var atlas_texture: texture_2d<f32>;
@group(1) @binding(1)
var atlas_sampler: sampler;

@group(1) @binding(2)
var image_texture: texture_2d<f32>;

struct CellData {
    glyph_uv_min: vec2<f32>,
    glyph_uv_max: vec2<f32>,
    glyph_size: vec2<f32>,
    _pad0: vec2<f32>,
    fg: vec4<f32>,
    bg: vec4<f32>,
    attrs: u32,
    _pad1: array<u32, 3>,
}

struct VertexInput {
    @location(0) position: vec2<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) bg_color: vec4<f32>,
    @location(2) tex_coord: vec2<f32>,
    @location(3) cell_size: vec2<f32>,
    @location(4) attrs: u32,
}

@vertex
fn vs_main(input: VertexInput, @builtin(instance_index) ii: u32) -> VertexOutput {
    let col = ii % uniforms.cols;
    let row = ii / uniforms.cols;
    let data = cell_data[ii];

    let cell_origin = vec2<f32>(f32(col), f32(row)) * uniforms.cell_size;
    let local = input.position;

    let pos = uniforms.viewport_origin + cell_origin + local * uniforms.cell_size;

    let x = pos.x / uniforms.screen_size.x * 2.0 - 1.0;
    let y = 1.0 - pos.y / uniforms.screen_size.y * 2.0;

    // Inflate the quad by half a device pixel so adjacent cells overlap
    // at fractional scaling (no hairline seams from rasterizer coverage).
    var clip_xy = vec2<f32>(x, y);
    clip_xy += (local * 2.0 - 1.0) / uniforms.screen_size;

    var output: VertexOutput;
    output.clip_position = vec4<f32>(clip_xy, 0.0, 1.0);
    output.color = data.fg;
    output.bg_color = data.bg;
    output.tex_coord = mix(data.glyph_uv_min, data.glyph_uv_max, clamp(local, vec2<f32>(0.0), vec2<f32>(1.0)));
    output.cell_size = local;
    output.attrs = data.attrs;
    return output;
}

fn srgb_to_linear(c: vec4<f32>) -> vec4<f32> {
    let low = c.rgb / 12.92;
    let high = pow((c.rgb + 0.055) / 1.055, vec3<f32>(2.4));
    let cond = c.rgb <= vec3<f32>(0.04045);
    return vec4<f32>(select(high, low, cond), c.a);
}

// --- Blur / transparency composite (optional post-process path) ---
// Only used when window.blur=true AND window.opacity<1.0. The scene is
// rendered into an offscreen texture; this pass blurs it and blits to the
// surface. Blurring what is BEHIND the window is a compositor feature, not
// something a wgpu renderer can do portably — this blurs the terminal's own
// framebuffer and preserves its alpha so a compositor that honors surface
// alpha can show the desktop through the translucent background.

struct BlurParams {
    header: vec4<u32>,
    kernel: array<vec4<f32>, 16>,
}

@group(0) @binding(0)
var<uniform> blur_params: BlurParams;

@group(0) @binding(1)
var scene_texture: texture_2d<f32>;

@group(0) @binding(2)
var scene_sampler: sampler;

struct BlurVertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

// Full-screen triangle from vertex_index; no vertex buffer needed.
@vertex
fn vs_blur(@builtin(vertex_index) vi: u32) -> BlurVertexOutput {
    var out: BlurVertexOutput;
    if vi == 0u {
        out.clip_position = vec4<f32>(-1.0, -1.0, 0.0, 1.0);
    } else if vi == 1u {
        out.clip_position = vec4<f32>(3.0, -1.0, 0.0, 1.0);
    } else {
        out.clip_position = vec4<f32>(-1.0, 3.0, 0.0, 1.0);
    }
    // Texture origin is top-left (v=0 at top) while NDC +y is up: flip v.
    out.uv = vec2<f32>((out.clip_position.x + 1.0) * 0.5, (1.0 - out.clip_position.y) * 0.5);
    return out;
}

// Single-pass separable Gaussian: taps x taps samples, outer-product weights.
// Each kernel vec4 packs (weight, x_off_norm, y_off_norm, unused); x uses the
// i-th row offset, y the j-th column offset.
// ponytail: kernel size quantized to {3,5,7,9,11}; no ping-pong passes, no
// mipmap pyramid, no compute shader. Upgrade to two-pass separable only if
// the 11-tap (121 samples) case ever shows up in a profiler.
@fragment
fn fs_blur(input: BlurVertexOutput) -> @location(0) vec4<f32> {
    var color = vec3<f32>(0.0);
    var alpha = 0.0f;
    for (var i = 0u; i < blur_params.header.x; i = i + 1u) {
        for (var j = 0u; j < blur_params.header.x; j = j + 1u) {
            let w = blur_params.kernel[i].x * blur_params.kernel[j].x;
            let uv = input.uv + vec2<f32>(blur_params.kernel[i].y, blur_params.kernel[j].z);
            let c = textureSampleLevel(scene_texture, scene_sampler, uv, 0.0);
            color += c.rgb * w;
            alpha += c.a * w;
        }
    }
    return vec4<f32>(color, alpha);
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    var fg = srgb_to_linear(input.color);
    var bg = srgb_to_linear(input.bg_color);

    if (input.attrs & 0x40u) != 0u {
        let temp = fg;
        fg = bg;
        bg = temp;
    }

    var color = bg;

    if (input.attrs & 0x1u) != 0u {
        fg.r = min(fg.r * 1.2, 1.0);
        fg.g = min(fg.g * 1.2, 1.0);
        fg.b = min(fg.b * 1.2, 1.0);
    }

    if (input.attrs & 0x10u) != 0u {
        fg.r = fg.r * 0.7;
        fg.g = fg.g * 0.7;
        fg.b = fg.b * 0.7;
    }

    if (input.attrs & 0x80u) != 0u {
        fg.a = 0.0;
    }

    if (input.attrs & 0x800u) != 0u {
        bg = mix(bg, vec4<f32>(1.0, 1.0, 1.0, 1.0), 0.07);
    }

    let glyph_alpha = textureSample(atlas_texture, atlas_sampler, input.tex_coord).a;

    color = mix(color, fg, glyph_alpha);

    if (input.attrs & 0x4u) != 0u {
        let local_y = input.cell_size.y;
        if local_y >= 0.85 && local_y <= 0.90 {
            color = fg;
        }
    }

    if (input.attrs & 0x8u) != 0u {
        let local_y = input.cell_size.y;
        if local_y >= 0.48 && local_y <= 0.52 {
            color = fg;
        }
    }

    if (input.attrs & 0x100u) != 0u {
        let local_x = input.cell_size.x;
        if local_x >= 0.0 && local_x <= 0.15 {
            color = fg;
        }
    }

    if (input.attrs & 0x200u) != 0u {
        color = mix(fg, bg, 0.3);
    }

    if (input.attrs & 0x400u) != 0u {
        let img_sample = textureSample(image_texture, atlas_sampler, input.cell_size);
        if img_sample.a > 0.0 {
            color = img_sample;
        }
    }

    return color;
}
