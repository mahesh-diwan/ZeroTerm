struct Uniforms {
    screen_size: vec2<f32>,
    cell_size: vec2<f32>,
    cursor_pos: vec2<f32>,
    cursor_visible: u32,
    cols: u32,
    rows: u32,
    _padding: u32,
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

    let pos = cell_origin + local * uniforms.cell_size;

    let x = pos.x / uniforms.screen_size.x * 2.0 - 1.0;
    let y = 1.0 - pos.y / uniforms.screen_size.y * 2.0;

    var output: VertexOutput;
    output.clip_position = vec4<f32>(x, y, 0.0, 1.0);
    output.color = data.fg;
    output.bg_color = data.bg;
    output.tex_coord = mix(data.glyph_uv_min, data.glyph_uv_max, local);
    output.cell_size = local;
    output.attrs = data.attrs;
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    var fg = input.color;
    var bg = input.bg_color;

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
