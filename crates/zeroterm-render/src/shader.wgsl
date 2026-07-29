struct Uniforms {
    screen_size: vec2<f32>,
    cell_size: vec2<f32>,
    cursor_pos: vec2<f32>,
    cursor_visible: u32,
    _padding: vec2<u32>,
}

@group(0) @binding(0)
var<uniform> uniforms: Uniforms;

@group(1) @binding(0)
var atlas_texture: texture_2d<f32>;
@group(1) @binding(1)
var atlas_sampler: sampler;

struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) tex_coord: vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) bg_color: vec4<f32>,
    @location(4) cell_size: vec2<f32>,
    @location(5) attrs: u32,
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
fn vs_main(input: VertexInput) -> VertexOutput {
    var output: VertexOutput;

    let x = input.position.x / uniforms.screen_size.x * 2.0 - 1.0;
    let y = 1.0 - input.position.y / uniforms.screen_size.y * 2.0;
    output.clip_position = vec4<f32>(x, y, 0.0, 1.0);

    output.color = input.color;
    output.bg_color = input.bg_color;
    output.tex_coord = input.tex_coord;
    output.cell_size = input.cell_size;
    output.attrs = input.attrs;

    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    var fg = input.color;
    var bg = input.bg_color;

    // Reverse: swap fg and bg
    if (input.attrs & 0x40u) != 0u {
        let temp = fg;
        fg = bg;
        bg = temp;
    }

    // Start with background
    var color = bg;

    // Bold — slightly brighter
    if (input.attrs & 0x1u) != 0u {
        fg.r = min(fg.r * 1.2, 1.0);
        fg.g = min(fg.g * 1.2, 1.0);
        fg.b = min(fg.b * 1.2, 1.0);
    }

    // Dim — darker
    if (input.attrs & 0x10u) != 0u {
        fg.r = fg.r * 0.7;
        fg.g = fg.g * 0.7;
        fg.b = fg.b * 0.7;
    }

    // Invisible
    if (input.attrs & 0x80u) != 0u {
        fg.a = 0.0;
    }

    // Sample glyph alpha from atlas
    let glyph_alpha = textureSample(atlas_texture, atlas_sampler, input.tex_coord).a;

    // Mix glyph over background
    color = mix(color, fg, glyph_alpha);

    // Underline — use cell-local y (cell_size attribute holds local position)
    if (input.attrs & 0x4u) != 0u {
        let local_y = input.cell_size.y;
        if local_y >= 0.85 && local_y <= 0.90 {
            color = fg;
        }
    }

    // Strikethrough
    if (input.attrs & 0x8u) != 0u {
        let local_y = input.cell_size.y;
        if local_y >= 0.48 && local_y <= 0.52 {
            color = fg;
        }
    }

    // Bar cursor — vertical line at cell-local x
    if (input.attrs & 0x100u) != 0u {
        let local_x = input.cell_size.x;
        if local_x >= 0.0 && local_x <= 0.15 {
            color = fg;
        }
    }

    return color;
}
