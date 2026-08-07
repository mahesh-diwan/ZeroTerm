struct Uniforms {
    screen_size: vec2<f32>,
    cell_size: vec2<f32>,
    viewport_origin: vec2<f32>,
    cols: u32,
    rows: u32,
    // 1 when the surface alpha mode is PreMultiplied: premultiply output RGB
    // by alpha so translucent pixels composite correctly (no glow).
    premultiply: u32,
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
    // Top-left of the glyph bitmap inside the cell, in cell pixels
    // (placement.left, baseline + placement.top).
    glyph_offset: vec2<f32>,
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
    @location(2) glyph_uv_min: vec2<f32>,
    // Local position within the cell quad (0..1); the cell is drawn
    // inflated by half a device pixel, so this spans cell +/- 0.5px.
    @location(3) cell_local: vec2<f32>,
    @location(4) attrs: u32,
    @location(5) glyph_uv_per_px: vec2<f32>,
    @location(6) glyph_size: vec2<f32>,
    @location(7) glyph_offset: vec2<f32>,
}

// uv delta per bitmap pixel, guarding against a zero-sized glyph.
fn safe_uv_per_px(delta: vec2<f32>, size: vec2<f32>) -> vec2<f32> {
    var r = vec2<f32>(0.0);
    if size.x != 0.0 { r.x = delta.x / size.x; }
    if size.y != 0.0 { r.y = delta.y / size.y; }
    return r;
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
    output.glyph_uv_min = data.glyph_uv_min;
    output.glyph_uv_per_px = safe_uv_per_px(data.glyph_uv_max - data.glyph_uv_min, data.glyph_size);
    output.glyph_size = data.glyph_size;
    output.glyph_offset = data.glyph_offset;
    output.cell_local = local;
    output.attrs = data.attrs;
    return output;
}

fn srgb_to_linear(c: vec4<f32>) -> vec4<f32> {
    let low = c.rgb / 12.92;
    let high = pow((c.rgb + 0.055) / 1.055, vec3<f32>(2.4));
    let cond = c.rgb <= vec3<f32>(0.04045);
    return vec4<f32>(select(high, low, cond), c.a);
}

// NOTE: real background blur (the desktop behind the window) is a compositor
// feature (KDE 'Blur', Hyprland 'windowrule=blur') and can't be done from a
// wgpu renderer. The old self-blur composite was removed; window.opacity < 1.0
// renders straight to a translucent surface and the compositor blurs behind it.

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

    // Glyphs are rasterized at their natural size (no stretching to the cell,
    // which used to make narrow glyphs like 'i' and '.' fat and blocky). The
    // bitmap rect sits at `glyph_offset` within the cell; pixels outside it
    // contribute zero alpha so the background (and window transparency) shows.
    let cell_px = input.cell_local * uniforms.cell_size;
    let glyph_px = cell_px - input.glyph_offset;
    var glyph_alpha = 0.0;
    if (glyph_px.x >= 0.0 && glyph_px.y >= 0.0
        && glyph_px.x < input.glyph_size.x && glyph_px.y < input.glyph_size.y) {
        let uv = input.glyph_uv_min + glyph_px * input.glyph_uv_per_px;
        glyph_alpha = textureSample(atlas_texture, atlas_sampler, uv).a;
    }

    color = mix(color, fg, glyph_alpha);

    if (input.attrs & 0x4u) != 0u {
        let local_y = input.cell_local.y;
        if local_y >= 0.85 && local_y <= 0.90 {
            color = fg;
        }
    }

    if (input.attrs & 0x8u) != 0u {
        let local_y = input.cell_local.y;
        if local_y >= 0.48 && local_y <= 0.52 {
            color = fg;
        }
    }

    if (input.attrs & 0x100u) != 0u {
        let local_x = input.cell_local.x;
        if local_x >= 0.0 && local_x <= 0.15 {
            color = fg;
        }
    }

    if (input.attrs & 0x200u) != 0u {
        color = mix(fg, bg, 0.3);
    }

    if (input.attrs & 0x400u) != 0u {
        let img_sample = textureSample(image_texture, atlas_sampler, input.cell_local);
        if img_sample.a > 0.0 {
            color = img_sample;
        }
    }

    if uniforms.premultiply != 0u {
        return vec4<f32>(color.rgb * color.a, color.a);
    }
    return color;
}
