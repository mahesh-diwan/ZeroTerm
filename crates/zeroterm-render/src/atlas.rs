//! GlyphAtlas: font loading, glyph rasterization (swash) and atlas-texture
//! management. Carved out of renderer.rs so the text-pipeline machinery is
//! navigable on its own; the Renderer only needs `new`, `get_or_insert_glyph`,
//! `cell_metrics`, and the `view`/`sampler` bind resources.

use std::collections::HashMap;
use swash::scale::{Render, ScaleContext, Source};
use swash::FontRef;

use crate::renderer::{Result, RendererError};

const ATLAS_SIZE: u32 = 1024;

/// A rasterized glyph plus everything the shader needs to place it. Returned
/// (not a raw tuple) so the placement contract travels with the type: `offset`
/// is the bitmap's top-left within its cell, in **cell pixels, y-down** — the
/// shader samples `cell_px - offset` and rejects anything outside the bitmap
/// rect, so a wrong offset silently makes text invisible.
#[derive(Clone, Copy)]
pub(crate) struct GlyphInfo {
    pub(crate) x: u32,
    pub(crate) y: u32,
    pub(crate) width: u32,
    pub(crate) height: u32,
    /// Bitmap top-left within its cell, in cell pixels (y-down). swash
    /// rasterizes y-up (Origin::BottomLeft), so this is computed once here as
    /// `(placement.left, baseline - placement.top)`.
    pub(crate) offset_x: f32,
    pub(crate) offset_y: f32,
}

impl GlyphInfo {
    /// Normalized atlas rect for this glyph (u0, v0, u1, v1).
    pub(crate) fn uv(&self) -> (f32, f32, f32, f32) {
        info_to_uv(self)
    }
}

/// Pure placement math shared by the atlas and its tests: swash measures from
/// the pen origin with y-up (Origin::BottomLeft), where `top` is the ink's top
/// edge ABOVE the baseline. The cell is y-down, so the bitmap's top sits
/// `baseline - top` pixels below the cell top. The old `baseline + top` put
/// every bitmap below the cell bottom — invisible text.
fn cell_offset(baseline: f32, left: f32, top: f32) -> (f32, f32) {
    (left, baseline - top)
}

/// Normalized atlas rect for a packed glyph.
fn info_to_uv(info: &GlyphInfo) -> (f32, f32, f32, f32) {
    let u0 = info.x as f32 / ATLAS_SIZE as f32;
    let v0 = info.y as f32 / ATLAS_SIZE as f32;
    let u1 = (info.x + info.width) as f32 / ATLAS_SIZE as f32;
    let v1 = (info.y + info.height) as f32 / ATLAS_SIZE as f32;
    (u0, v0, u1, v1)
}

pub(crate) struct GlyphAtlas {
    texture: wgpu::Texture,
    pub(crate) view: wgpu::TextureView,
    pub(crate) sampler: wgpu::Sampler,
    glyph_cache: HashMap<u32, GlyphInfo>,
    font_data: Vec<u8>,
    scale_context: ScaleContext,
    cursor_x: u32,
    cursor_y: u32,
    row_height: u32,
    font_size: f32,
    /// Baseline position within a cell (ascent, in px) — glyph bitmaps are
    /// anchored to it via `baseline - placement.top` (swash Origin::BottomLeft:
    /// placement.top is the ink's top edge above the baseline, so in the cell's
    /// y-down space the ink top sits `baseline - top` px below the cell top).
    baseline: f32,
    cell_width: f32,
    cell_height: f32,
    font_path: Option<String>,
}

/// Measure the exact cell size a Renderer will use for a font size, without a
/// GPU. Mirrors the metrics block in GlyphAtlas::new (ascent/descent/leading
/// for the row height, 'W' ink width for the column width) so the app can
/// spawn the PTY at the final size *before* the renderer arrives. Spawning at
/// the right size kills the startup resize storm: every PTY resize delivers
/// SIGWINCH, and bash reprints its prompt on each — three startup resizes
/// stacked three prompts.
/// Font metrics derived from a font blob at a size: the cell (row) height
/// comes from ascent + descent + leading, the cell width from the 'W' ink,
/// and `baseline` (ascent) anchors glyph bitmaps inside their cell. Shared by
/// `estimate_cell_size`, `GlyphAtlas::new`, and `set_font` so the three metric
/// computations can never drift. Returns `None` when the blob is not a
/// parseable font (callers then fall back to size-derived estimates).
struct FontMetrics {
    cell_width: f32,
    cell_height: f32,
    baseline: f32,
    ascent: f32,
    descent: f32,
    leading: f32,
}

fn compute_metrics(
    font_data: &[u8],
    font_size: f32,
    scale_ctx: &mut ScaleContext,
) -> Option<FontMetrics> {
    let font = FontRef::from_index(font_data, 0)?;
    let metrics = font.metrics(&[]);
    let scale = font_size / metrics.units_per_em as f32;
    let ascent = metrics.ascent * scale;
    let descent = metrics.descent * scale;
    let leading = metrics.leading * scale;
    let cell_height = (ascent + descent + leading).ceil();
    let mut scaler = scale_ctx.builder(font).size(font_size).build();
    let charmap = font.charmap();
    let mut cell_width = font_size * 0.5;
    if let Some(img) = Render::new(&[Source::Outline])
        .format(swash::zeno::Format::Alpha)
        .render(&mut scaler, charmap.map(0x57u32))
    {
        cell_width = img.placement.width as f32;
    }
    Some(FontMetrics {
        cell_width,
        cell_height,
        baseline: ascent,
        ascent,
        descent,
        leading,
    })
}

pub fn estimate_cell_size(font_size: f32, font_path: Option<&str>) -> (f32, f32) {
    if let Ok(data) = load_font_bytes(font_path) {
        let mut scale_ctx = ScaleContext::new();
        if let Some(m) = compute_metrics(&data, font_size, &mut scale_ctx) {
            return (m.cell_width, m.cell_height);
        }
    }
    (font_size * 0.5, font_size * 1.2)
}

impl GlyphAtlas {
    pub(crate) fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        font_size: f32,
        font_path: Option<String>,
    ) -> Result<Self> {
        let font_data = load_font_bytes(font_path.as_deref())?;

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Glyph Atlas"),
            size: wgpu::Extent3d {
                width: ATLAS_SIZE,
                height: ATLAS_SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        // ponytail: no full-atlas clear. Uninitialized texels are only sampled
        // at 0.5-texel Linear bleed into *written* glyph neighbors, never far
        // from a glyph, so skipping the 4MB write_texture saves ~30ms of boot.
        let mut atlas = Self {
            texture,
            view,
            sampler,
            glyph_cache: HashMap::new(),
            font_data,
            font_path,
            scale_context: ScaleContext::new(),
            cursor_x: 0,
            cursor_y: 0,
            row_height: 0,
            font_size,
            baseline: 0.0,
            cell_width: font_size * 0.5,
            cell_height: font_size * 1.2,
        };

        // Compute cell metrics from font (shared with estimate_cell_size /
        // set_font so the three computations can never drift). new() cannot
        // fail here: the bytes came from a successful load, so a parse miss
        // keeps the size-derived estimates.
        if let Some(m) = compute_metrics(
            &atlas.font_data,
            atlas.font_size,
            &mut atlas.scale_context,
        ) {
            atlas.cell_width = m.cell_width;
            atlas.cell_height = m.cell_height;
            atlas.baseline = m.baseline;
            log::info!(
                "Cell metrics: {:.2}x{:.2} (ascent {:.2}, descent {:.2}, leading {:.2}, 'W' ink {}x{})",
                m.cell_width,
                m.cell_height,
                m.ascent,
                m.descent,
                m.leading,
                m.cell_width as u32,
                m.cell_height as u32
            );
        }

        // Pre-pack ASCII printable characters
        for ch in 32u8..=126 {
            atlas.get_or_insert_glyph(ch as char, device, queue);
        }

        Ok(atlas)
    }

    pub(crate) fn get_or_insert_glyph(
        &mut self,
        ch: char,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> GlyphInfo {
        let key = ch as u32;
        if let Some(info) = self.glyph_cache.get(&key) {
            return *info;
        }

        let font = match FontRef::from_index(&self.font_data, 0) {
            Some(f) => f,
            None => {
                // Zero-size glyph: no ink, bg shows through (the shader's
                // coverage test rejects everything outside the bitmap rect).
                return GlyphInfo {
                    x: 0,
                    y: 0,
                    width: 0,
                    height: 0,
                    offset_x: 0.0,
                    offset_y: 0.0,
                };
            }
        };

        let charmap = font.charmap();
        let glyph_id = charmap.map(key);

        let mut scaler = self
            .scale_context
            .builder(font)
            .size(self.font_size)
            .build();

        let image = Render::new(&[Source::Outline])
            .format(swash::zeno::Format::Alpha)
            .render(&mut scaler, glyph_id);

        match image {
            Some(img) if img.placement.width > 0 && img.placement.height > 0 => {
                let info = self.pack_glyph(&img, device, queue);
                self.glyph_cache.insert(key, info);
                info
            }
            _ => GlyphInfo {
                x: 0,
                y: 0,
                width: 0,
                height: 0,
                offset_x: 0.0,
                offset_y: 0.0,
            },
        }
    }

    fn pack_glyph(
        &mut self,
        image: &swash::scale::image::Image,
        _device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> GlyphInfo {
        let w = image.placement.width;
        let h = image.placement.height;
        // Cell-space offsets, y-down (see `cell_offset`).
        let (offset_x, offset_y) = cell_offset(
            self.baseline,
            image.placement.left as f32,
            image.placement.top as f32,
        );

        // Advance to next row if needed
        if self.cursor_x + w > ATLAS_SIZE {
            self.cursor_x = 0;
            self.cursor_y += self.row_height + 1;
            self.row_height = 0;
        }

        // Check if we've run out of space
        if self.cursor_y + h > ATLAS_SIZE {
            log::warn!("Glyph atlas full, clearing");
            self.clear_atlas(queue);
        }

        let info = GlyphInfo {
            x: self.cursor_x,
            y: self.cursor_y,
            width: w,
            height: h,
            offset_x,
            offset_y,
        };

        // Convert alpha mask to RGBA (white RGB, alpha from mask)
        let mut rgba = Vec::with_capacity((w * h * 4) as usize);
        for &alpha in &image.data {
            rgba.push(255);
            rgba.push(255);
            rgba.push(255);
            rgba.push(alpha);
        }

        queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: self.cursor_x,
                    y: self.cursor_y,
                    z: 0,
                },
                aspect: wgpu::TextureAspect::All,
            },
            &rgba,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(w * 4),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );

        self.cursor_x += w + 1;
        self.row_height = self.row_height.max(h);

        info
    }

    pub(crate) fn cell_metrics(&self) -> (f32, f32) {
        (self.cell_width, self.cell_height)
    }

    fn clear_atlas(&mut self, queue: &wgpu::Queue) {
        let clear = vec![0u8; (ATLAS_SIZE * ATLAS_SIZE * 4) as usize];
        queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &clear,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(ATLAS_SIZE * 4),
                rows_per_image: Some(ATLAS_SIZE),
            },
            wgpu::Extent3d {
                width: ATLAS_SIZE,
                height: ATLAS_SIZE,
                depth_or_array_layers: 1,
            },
        );
        self.cursor_x = 0;
        self.cursor_y = 0;
        self.row_height = 0;
        self.glyph_cache.clear();
    }

    fn repack_ascii(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        self.clear_atlas(queue);
        for ch in 32u8..=126 {
            self.get_or_insert_glyph(ch as char, device, queue);
        }
    }

    /// Change the rasterization size (and/or font file) at runtime: recomputes
    /// the cell metrics from the (possibly new) font, clears every cached
    /// glyph, and repacks the ASCII set at the new size. A failed font load
    /// keeps the previous font/size in place.
    pub(crate) fn set_font(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        font_path: Option<String>,
        font_size: f32,
    ) -> Result<()> {
        let data = load_font_bytes(font_path.as_deref())?;
        // Compute the new metrics BEFORE committing any state: a blob that
        // loads but cannot be parsed keeps the previous font, size, and
        // metrics — and the caller's font_path/size bookkeeping stays in sync
        // with what actually rendered.
        let metrics = compute_metrics(&data, font_size, &mut self.scale_context).ok_or_else(
            || {
                RendererError::FontInvalid(
                    font_path
                        .clone()
                        .unwrap_or_else(|| "<default font>".to_string()),
                )
            },
        )?;
        self.font_path = font_path;
        self.font_data = data;
        self.font_size = font_size;
        self.cell_width = metrics.cell_width;
        self.cell_height = metrics.cell_height;
        self.baseline = metrics.baseline;
        // Every cached bitmap was rasterized at the old size; drop them all
        // (non-ASCII glyphs too — repack_ascii only refreshes the ASCII set).
        self.glyph_cache.clear();
        self.repack_ascii(device, queue);
        Ok(())
    }
}

fn load_font_bytes(font_path: Option<&str>) -> Result<Vec<u8>> {
    if let Some(path) = font_path {
        return std::fs::read(path).map_err(|p| RendererError::FontNotFound(p.to_string()));
    }
    let paths = [
        "/usr/share/fonts/TTF/JetBrainsMonoNerdFont-Regular.ttf",
        "/usr/share/fonts/TTF/JetBrainsMonoNLNerdFont-Regular.ttf",
        "/usr/share/fonts/truetype/JetBrainsMono/JetBrainsMonoNerdFont-Regular.ttf",
        "/usr/share/fonts/TTF/MesloLGMNerdFontMono-Regular.ttf",
        "/usr/share/fonts/liberation/LiberationMono-Regular.ttf",
        "/usr/share/fonts/truetype/liberation/LiberationMono-Regular.ttf",
        "/usr/share/fonts/GeistMonoVF.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
    ];
    for path in &paths {
        if let Ok(data) = std::fs::read(path) {
            log::info!("Loaded font: {}", path);
            return Ok(data);
        }
    }
    log::info!("No system font found, using embedded DejaVu Sans Mono fallback");
    Ok(include_bytes!("../DejaVuSansMono.ttf").to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use swash::scale::{Render, ScaleContext, Source};
    use swash::FontRef;

    const FONT: &[u8] = include_bytes!("../DejaVuSansMono.ttf");

    /// Recomputes the atlas's cell metrics exactly like `GlyphAtlas::new` does
    /// (ascent + descent + leading, rounded up) so the test shares the atlas's
    /// ground truth without needing a GPU.
    fn cell_metrics(font_size: f32) -> (f32, f32) {
        let font = FontRef::from_index(FONT, 0).unwrap();
        let metrics = font.metrics(&[]);
        let scale = font_size / metrics.units_per_em as f32;
        let ascent = metrics.ascent * scale;
        let descent = metrics.descent * scale;
        let leading = metrics.leading * scale;
        ((ascent + descent + leading).ceil(), ascent)
    }

    #[test]
    fn estimate_cell_size_matches_atlas_metrics() {
        // Regression: the app spawns the PTY at a size computed from
        // estimate_cell_size (no GPU). If it drifts from the real atlas
        // metrics, the spawn estimate disagrees with the renderer -> resize
        // storm -> bash reprints its prompt. cell_metrics() below recomputes
        // the atlas's ground truth without a GPU.
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/DejaVuSansMono.ttf");
        let (cell_height, _baseline) = cell_metrics(14.0);
        let (w, h) = estimate_cell_size(14.0, Some(path));
        assert!(w > 0.0 && h > 0.0, "metrics must be positive: {w}x{h}");
        assert!(
            (h - cell_height).abs() < 0.01,
            "estimate height {h} must match atlas {cell_height}"
        );
        assert!(h > w, "monospace cell: height {h} > width {w}");
        // No font available -> nonzero fallback with a sane aspect.
        let (w2, h2) = estimate_cell_size(14.0, None);
        assert!(w2 > 0.0 && h2 > 0.0 && h2 > w2);
    }

    #[test]
    fn glyph_bitmaps_fit_inside_their_cell() {
        // Regression for the invisible-text bug: pack_glyph computed
        // offset_y = baseline + placement.top, which put every bitmap below
        // the cell bottom (baseline 14.5 + top 11.8 = 26.3px in a 19px cell);
        // the shader coverage test then rejected all ink and text vanished.
        let font_size = 14.0;
        let (cell_height, baseline) = cell_metrics(font_size);
        let font = FontRef::from_index(FONT, 0).unwrap();
        let charmap = font.charmap();
        let mut ctx = ScaleContext::new();
        let mut scaler = ctx.builder(font).size(font_size).build();
        for ch in ['[', 'g', '.', 'i', 'W', 'A', 'j', '×'] {
            let img = Render::new(&[Source::Outline])
                .format(swash::zeno::Format::Alpha)
                .render(&mut scaler, charmap.map(ch as u32))
                .unwrap_or_else(|| panic!("'{ch}' rasterizes"));
            let (_, off_y) =
                cell_offset(baseline, img.placement.left as f32, img.placement.top as f32);
            let bottom = off_y + img.placement.height as f32;
            assert!(
                off_y >= 0.0,
                "'{ch}' ink top {off_y:.1} is above the cell (baseline {baseline:.1}, top {})",
                img.placement.top
            );
            assert!(
                bottom <= cell_height,
                "'{ch}' ink bottom {bottom:.1} exceeds cell {cell_height:.1} \
                 (baseline {baseline:.1}, top {}, height {})",
                img.placement.top,
                img.placement.height
            );
        }
    }

    #[test]
    fn cell_offset_flips_swash_y_up_to_cell_y_down() {
        // swash Origin::BottomLeft: a glyph whose ink top sits above the
        // baseline (placement.top > 0) must land BELOW the cell top in the
        // y-down cell space.
        let (_, baseline) = cell_metrics(14.0);
        let (ox, oy) = cell_offset(baseline, -1.0, 11.8);
        assert_eq!(ox, -1.0);
        assert!((oy - (baseline - 11.8)).abs() < 1e-4);
    }
}
